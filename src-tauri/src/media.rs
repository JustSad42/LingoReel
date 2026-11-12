//! Turning a video into study material: pull subtitles and audio out of the file,
//! listen to the audio with Whisper, and line everything up.
use crate::align::{self, Heard};
use crate::components;
use crate::dict;
use crate::subs::{self, Cue};
use crate::util::{cmd, decode_text, lang2, Res};
use crate::AppState;
use anyhow::{anyhow, bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncBufReadExt, BufReader};

const TEXT_CODECS: &[&str] = &["subrip", "srt", "ass", "ssa", "mov_text", "tx3g", "webvtt", "vtt", "text"];
const VIDEO_EXT: &[&str] = &["mkv", "mp4", "m4v", "avi", "mov", "webm", "ts", "wmv"];
const PREPARED_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SubTrack {
    pub id: String,
    pub lang: String,
    pub codec: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub forced: bool,
    /// Stream index inside the video file, for embedded tracks.
    #[serde(default)]
    pub index: Option<i64>,
    /// Separate subtitle served over HTTP (Plex sidecar).
    #[serde(default)]
    pub url: Option<String>,
    /// Separate subtitle file on disk.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AudioTrack {
    pub index: i64,
    pub lang: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MediaReq {
    pub key: String,
    pub title: String,
    /// File path or URL playable by mpv and FFmpeg.
    pub url: String,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub subs: Vec<SubTrack>,
    #[serde(default)]
    pub audio: Vec<AudioTrack>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct PrepareOpts {
    pub target_lang: String,
    pub native_lang: String,
    /// "auto", "whisper" (transcribe the audio) or a track id.
    pub target_sub: String,
    /// "auto", "none" or a track id.
    pub native_sub: String,
    /// Listen to the audio to time each word.
    pub align: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Prepared {
    pub version: u32,
    pub key: String,
    pub title: String,
    pub lang: String,
    pub cues: Vec<Cue>,
    /// True when word timings come from the audio rather than being estimated.
    pub aligned: bool,
    pub target_source: String,
    pub native_source: String,
    pub audio_index: Option<i64>,
    /// Seconds the subtitle file was off by.
    pub offset: f64,
    pub note: String,
}

fn prepared_path(state: &AppState, key: &str) -> PathBuf {
    let safe: String = key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    state.dir.join("media").join(format!("{safe}.json"))
}

pub fn load_prepared(state: &AppState, key: &str) -> Option<Prepared> {
    let text = std::fs::read_to_string(prepared_path(state, key)).ok()?;
    serde_json::from_str::<Prepared>(&text).ok().filter(|p| p.version == PREPARED_VERSION)
}

#[tauri::command]
pub fn prepared_get(state: State<AppState>, key: String) -> Res<Option<Prepared>> {
    Ok(load_prepared(&state, &key))
}

#[tauri::command]
pub fn prepared_delete(state: State<AppState>, key: String) -> Res<()> {
    let _ = std::fs::remove_file(prepared_path(&state, &key));
    Ok(())
}

fn is_text(t: &SubTrack) -> bool {
    TEXT_CODECS.contains(&t.codec.to_lowercase().as_str())
}

fn pick_sub<'a>(tracks: &'a [SubTrack], lang: &str, choice: &str) -> Option<&'a SubTrack> {
    if choice != "auto" {
        return tracks.iter().find(|t| t.id == choice);
    }
    let mut candidates: Vec<&SubTrack> =
        tracks.iter().filter(|t| is_text(t) && lang2(&t.lang) == lang).collect();
    // Full dialogue tracks first: not forced, not signs/songs only.
    candidates.sort_by_key(|t| {
        let title = t.title.to_lowercase();
        (t.forced || title.contains("forc") || title.contains("sign") || title.contains("song")) as u8
    });
    candidates.first().copied()
}

fn emit(app: &AppHandle, key: &str, stage: &str, pct: f64, note: &str) {
    let _ = app.emit("prepare-progress", json!({"key": key, "stage": stage, "pct": pct, "note": note}));
}

/// Read an external subtitle (file or URL) into cues.
async fn read_external(state: &AppState, track: &SubTrack, tmp: &Path) -> anyhow::Result<Vec<Cue>> {
    let bytes = if let Some(url) = &track.url {
        state.http.get(url).send().await?.error_for_status()?.bytes().await?.to_vec()
    } else if let Some(p) = &track.path {
        tokio::fs::read(p).await?
    } else {
        bail!("subtitle track has no source")
    };
    let codec = track.codec.to_lowercase();
    if matches!(codec.as_str(), "srt" | "subrip" | "vtt" | "webvtt") {
        return Ok(subs::parse_srt(&decode_text(&bytes)));
    }
    // ASS and friends: let FFmpeg convert.
    let src = tmp.join(format!("ext.{}", if codec == "ssa" { "ssa" } else { "ass" }));
    tokio::fs::write(&src, decode_text(&bytes)).await?;
    let out = tmp.join("ext.srt");
    let ffmpeg = components::require(state, "ffmpeg")?;
    let res = cmd(ffmpeg).args(["-y", "-v", "error", "-nostdin", "-i"]).arg(&src).arg(&out).output().await?;
    if !res.status.success() {
        bail!("could not convert subtitle: {}", String::from_utf8_lossy(&res.stderr));
    }
    Ok(subs::parse_srt(&decode_text(&tokio::fs::read(&out).await?)))
}

/// Run Whisper over a 16 kHz mono WAV and return every word with its time.
pub async fn whisper_words(
    state: &AppState,
    wav: &Path,
    lang: &str,
    out_base: &Path,
    mut progress: impl FnMut(f64),
) -> anyhow::Result<Vec<Heard>> {
    let exe = components::require(state, "whisper")?;
    let model = components::require(state, "whisper-model")?;
    let mut child = cmd(&exe)
        .current_dir(exe.parent().unwrap())
        .arg("-m").arg(&model)
        .arg("-f").arg(wav)
        // DTW gives a precise time for every token; it needs flash attention off.
        .args(["-l", lang, "-mc", "0", "-sns", "-pp", "-dtw", components::whisper_dtw_preset(), "-nfa", "-ojf", "-of"])
        .arg(out_base)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting whisper")?;
    let stderr = child.stderr.take().unwrap();
    let mut lines = BufReader::new(stderr).lines();
    let mut tail: Vec<String> = vec![];
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(p) = line.split("progress =").nth(1) {
            if let Ok(v) = p.trim().trim_end_matches('%').trim().parse::<f64>() {
                progress(v);
            }
        } else {
            tail.push(line);
            if tail.len() > 12 {
                tail.remove(0);
            }
        }
    }
    let status = child.wait().await?;
    let json_path = PathBuf::from(format!("{}.json", out_base.display()));
    if !status.success() || !json_path.exists() {
        bail!("speech recognition failed: {}", tail.join(" | "));
    }
    let raw = tokio::fs::read(&json_path).await?;
    let v: Value = serde_json::from_str(&String::from_utf8_lossy(&raw))?;
    Ok(words_from_whisper_json(&v))
}

/// Turn whisper.cpp's full JSON into words. Each token carries `t_dtw`, the moment
/// (in 1/100 s) the token finishes being spoken; a word starts where the previous one ended.
pub fn words_from_whisper_json(v: &Value) -> Vec<Heard> {
    let mut out: Vec<Heard> = vec![];
    for seg in v["transcription"].as_array().cloned().unwrap_or_default() {
        let seg_text = seg["text"].as_str().unwrap_or("").trim().to_string();
        if seg_text.is_empty() || seg_text.starts_with(['[', '(', '*', '♪']) {
            continue;
        }
        // Group tokens into words: a token starting with a space begins a new word.
        let mut words: Vec<(String, f64)> = vec![];
        for tok in seg["tokens"].as_array().cloned().unwrap_or_default() {
            let text = tok["text"].as_str().unwrap_or("");
            if text.starts_with("[_") || text.is_empty() {
                continue;
            }
            let dtw = tok["t_dtw"].as_f64().unwrap_or(-1.0);
            let end = if dtw >= 0.0 { dtw / 100.0 } else { tok["offsets"]["to"].as_f64().unwrap_or(0.0) / 1000.0 };
            let spoken = text.chars().any(|c| c.is_alphanumeric() || c == '\u{fffd}');
            if text.starts_with(' ') || words.is_empty() {
                words.push((text.trim().to_string(), if spoken { end } else { -1.0 }));
            } else if let Some(last) = words.last_mut() {
                last.0.push_str(text);
                if spoken {
                    last.1 = last.1.max(end);
                }
            }
        }
        // Tokens can split an accented letter in half; the segment text is always intact,
        // so take the spelling from there when the word counts agree.
        let clean: Vec<&str> = seg_text.split_whitespace().collect();
        let use_clean = clean.len() == words.len();
        let seg_start = seg["offsets"]["from"].as_f64().unwrap_or(0.0) / 1000.0;
        let mut prev_end = out.last().map(|h: &Heard| h.e).unwrap_or(0.0).max(seg_start - 0.5);
        for (i, (text, end)) in words.into_iter().enumerate() {
            let text = if use_clean { clean[i].to_string() } else { text };
            if end < 0.0 || !text.chars().any(|c| c.is_alphanumeric()) {
                continue;
            }
            let letters = text.chars().filter(|c| c.is_alphanumeric()).count() as f64;
            let longest = 0.065 * letters + 0.08;
            let start = prev_end.max(end - longest).min(end - 0.03);
            out.push(Heard { text, s: start.max(0.0), e: end });
            prev_end = end;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whisper_json_becomes_timed_words() {
        let v = json!({"transcription": [{
            "text": " Où est l'élève ?",
            "offsets": {"from": 1000, "to": 3000},
            "tokens": [
                {"text": "[_BEG_]", "t_dtw": -1, "offsets": {"from": 1000, "to": 1000}},
                {"text": " O", "t_dtw": 120, "offsets": {"from": 1000, "to": 1100}},
                {"text": "\u{fffd}", "t_dtw": 130, "offsets": {"from": 1100, "to": 1200}},
                {"text": " est", "t_dtw": 160, "offsets": {"from": 1200, "to": 1500}},
                {"text": " l", "t_dtw": 170, "offsets": {"from": 1500, "to": 1600}},
                {"text": "'", "t_dtw": 172, "offsets": {"from": 1600, "to": 1600}},
                {"text": "élève", "t_dtw": 230, "offsets": {"from": 1600, "to": 2300}},
                {"text": " ?", "t_dtw": 290, "offsets": {"from": 2300, "to": 2900}}
            ]
        }]});
        let w = words_from_whisper_json(&v);
        let texts: Vec<&str> = w.iter().map(|h| h.text.as_str()).collect();
        assert_eq!(texts, ["Où", "est", "l'élève"]);
        assert!((w[0].e - 1.30).abs() < 1e-9);
        // "est" cannot plausibly last longer than 0.275 s, so it starts at 1.325, not at 1.30.
        assert!((w[1].s - 1.325).abs() < 1e-9 && (w[1].e - 1.60).abs() < 1e-9);
        assert!((w[2].e - 2.30).abs() < 1e-9);
    }
}


async fn prepare(app: &AppHandle, state: &AppState, req: &MediaReq, opts: &PrepareOpts) -> anyhow::Result<Prepared> {
    let key = req.key.as_str();
    let tmp = state.dir.join("tmp").join(format!("prep-{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    tokio::fs::create_dir_all(&tmp).await?;

    let can_listen = components::locate(state, "whisper").is_some()
        && components::locate(state, "whisper-model").is_some();
    let target = if opts.target_sub == "whisper" { None } else { pick_sub(&req.subs, &opts.target_lang, &opts.target_sub) };
    let native = if opts.native_sub == "none" { None } else { pick_sub(&req.subs, &opts.native_lang, &opts.native_sub) };
    if target.is_none() && !can_listen {
        bail!(
            "This video has no {} text subtitles. Install the speech recognition components in Setup to transcribe the audio instead.",
            opts.target_lang
        );
    }
    let listen = can_listen && (opts.align || target.is_none());
    let audio = req
        .audio
        .iter()
        .find(|a| lang2(&a.lang) == opts.target_lang)
        .or_else(|| req.audio.first());
    let mut note = String::new();
    if let Some(a) = audio {
        let l = lang2(&a.lang);
        if !l.is_empty() && l != "und" && l != opts.target_lang {
            note = format!("No {} audio track found; the spoken language is '{}'.", opts.target_lang, a.lang);
        }
    }

    // One FFmpeg pass pulls embedded subtitles and the audio out of the file.
    let ffmpeg = components::require(state, "ffmpeg")?;
    let (t_srt, n_srt, wav) = (tmp.join("target.srt"), tmp.join("native.srt"), tmp.join("audio.wav"));
    let mut c = cmd(&ffmpeg);
    c.args(["-y", "-v", "error", "-nostdin", "-progress", "pipe:1", "-i"]).arg(&req.url);
    let mut outputs = 0;
    if let Some(i) = target.and_then(|t| t.index) {
        c.args(["-map", &format!("0:{i}"), "-c:s", "srt"]).arg(&t_srt);
        outputs += 1;
    }
    if let Some(i) = native.and_then(|t| t.index) {
        c.args(["-map", &format!("0:{i}"), "-c:s", "srt"]).arg(&n_srt);
        outputs += 1;
    }
    if listen {
        let map = audio.map(|a| format!("0:{}", a.index)).unwrap_or_else(|| "0:a:0".into());
        c.args(["-map", &map, "-vn", "-sn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"]).arg(&wav);
        outputs += 1;
    }
    if outputs > 0 {
        emit(app, key, "extract", 0.0, "Reading subtitles and audio from the file");
        let mut child = c.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().context("starting ffmpeg")?;
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut stderr = child.stderr.take().unwrap();
        let err_task = tokio::spawn(async move {
            let mut s = String::new();
            let _ = tokio::io::AsyncReadExt::read_to_string(&mut stderr, &mut s).await;
            s
        });
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.parse::<f64>().ok()) {
                if req.duration > 0.0 {
                    let pct = (us / 1e6 / req.duration * 100.0).clamp(0.0, 100.0);
                    emit(app, key, "extract", pct, "Reading subtitles and audio from the file");
                }
            }
        }
        let status = child.wait().await?;
        let err = err_task.await.unwrap_or_default();
        if !status.success() {
            bail!("FFmpeg could not read the video: {}", err.lines().last().unwrap_or("unknown error"));
        }
    }

    let read_track = |track: Option<&SubTrack>, embedded: PathBuf| {
        let track = track.cloned();
        let tmp = tmp.clone();
        async move {
            match track {
                None => Ok::<Vec<Cue>, anyhow::Error>(vec![]),
                Some(t) if t.index.is_some() => {
                    Ok(subs::parse_srt(&decode_text(&tokio::fs::read(&embedded).await?)))
                }
                Some(t) => read_external(state, &t, &tmp).await,
            }
        }
    };
    let mut cues = read_track(target, t_srt).await.context("reading target-language subtitles")?;
    let native_cues = read_track(native, n_srt).await.unwrap_or_default();

    let mut heard: Vec<Heard> = vec![];
    if listen {
        emit(app, key, "listen", 0.0, "Listening to the audio");
        let app2 = app.clone();
        let key2 = key.to_string();
        heard = whisper_words(state, &wav, &opts.target_lang, &tmp.join("whisper"), move |p| {
            emit(&app2, &key2, "listen", p, "Listening to the audio")
        })
        .await?;
    }

    emit(app, key, "align", 0.0, "Matching words to speech");
    let mut target_source = target.map(|t| if t.index.is_some() { "embedded" } else { "external" }).unwrap_or("speech");
    if cues.is_empty() {
        if heard.is_empty() {
            bail!("No subtitles could be read and no speech was recognised.");
        }
        cues = align::cues_from_heard(&heard);
        target_source = "speech";
    }
    let lang = opts.target_lang.clone();
    let has_dict = components::locate(state, &format!("dict-{lang}")).is_some();
    let mut prepared = Prepared {
        version: PREPARED_VERSION,
        key: key.to_string(),
        title: req.title.clone(),
        lang: lang.clone(),
        target_source: target_source.into(),
        native_source: if native_cues.is_empty() { "none".into() } else { "subtitles".into() },
        audio_index: audio.map(|a| a.index),
        note,
        ..Default::default()
    };
    // Tokenising and aligning are CPU work on a few thousand lines; do them off the async runtime.
    let tokenise = |cues: &mut Vec<Cue>| -> anyhow::Result<()> {
        if has_dict {
            dict::with_dict(state, &lang, |d| {
                for c in cues.iter_mut() {
                    c.tokens = subs::tokenize(&lang, &c.text, Some(d));
                }
            })?;
        } else {
            for c in cues.iter_mut() {
                c.tokens = subs::tokenize(&lang, &c.text, None);
            }
        }
        Ok(())
    };
    tokenise(&mut cues)?;
    if heard.is_empty() {
        cues.iter_mut().for_each(subs::spread_tokens);
    } else {
        let original = cues.clone();
        let stats = align::align(&mut cues, &heard);
        let rate = stats.matched as f64 / stats.total.max(1) as f64;
        if rate < 0.3 && target_source != "speech" {
            // The audio does not say what the subtitles say (different language or a dub
            // with its own script). Keep the file's own timing.
            cues = original;
            cues.iter_mut().for_each(subs::spread_tokens);
            prepared.note = format!(
                "Only {:.0}% of subtitle words were heard in the audio, so word timing is estimated. {}",
                rate * 100.0,
                prepared.note
            );
        } else {
            prepared.aligned = true;
            prepared.offset = stats.offset;
        }
    }
    subs::pair_native(&mut cues, &native_cues);
    prepared.cues = cues;

    let path = prepared_path(state, key);
    tokio::fs::create_dir_all(path.parent().unwrap()).await?;
    tokio::fs::write(&path, serde_json::to_vec(&prepared)?).await?;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    emit(app, key, "done", 100.0, "");
    Ok(prepared)
}

#[tauri::command]
pub async fn prepare_media(app: AppHandle, state: State<'_, AppState>, req: MediaReq, opts: PrepareOpts) -> Res<Prepared> {
    // One at a time: Whisper wants the whole GPU.
    let _busy = state.prepare_lock.lock().await;
    Ok(prepare(&app, &state, &req, &opts).await?)
}

#[derive(Serialize)]
pub struct UnknownWord {
    pub lemma: String,
    pub count: u32,
    pub rank: Option<i64>,
    pub gloss: String,
    pub example: String,
}

#[derive(Serialize, Default)]
pub struct MediaStats {
    pub words: u32,
    pub known: u32,
    pub learning: u32,
    pub unique: u32,
    pub unique_known: u32,
    pub lines: u32,
    pub aligned: bool,
    pub unknown: Vec<UnknownWord>,
}

fn statuses(state: &AppState, lang: &str) -> anyhow::Result<HashMap<String, i64>> {
    let db = state.db.lock().unwrap();
    let mut st = db.prepare("SELECT lemma, status FROM words WHERE lang=?1")?;
    let rows = st.query_map([lang], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    Ok(rows.flatten().collect())
}

fn compute_stats(state: &AppState, p: &Prepared, status: &HashMap<String, i64>, detail: bool) -> MediaStats {
    let mut counts: HashMap<&str, (u32, &str)> = HashMap::new();
    for cue in &p.cues {
        for t in cue.tokens.iter().filter(|t| t.w) {
            if let Some(l) = &t.lemma {
                let e = counts.entry(l.as_str()).or_insert((0, cue.text.as_str()));
                e.0 += 1;
            }
        }
    }
    let mut s = MediaStats { lines: p.cues.len() as u32, aligned: p.aligned, unique: counts.len() as u32, ..Default::default() };
    let mut unknown: Vec<(&str, u32, &str)> = vec![];
    for (lemma, (count, example)) in &counts {
        s.words += count;
        match status.get(*lemma).copied().unwrap_or(0) {
            2 | 3 => {
                s.known += count;
                s.unique_known += 1;
            }
            1 => {
                s.learning += count;
                unknown.push((lemma, *count, example));
            }
            _ => unknown.push((lemma, *count, example)),
        }
    }
    if detail {
        unknown.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        unknown.truncate(400);
        let _ = dict::with_dict(state, &p.lang, |d| {
            for (lemma, count, example) in &unknown {
                let look = d.lookup(lemma);
                s.unknown.push(UnknownWord {
                    lemma: lemma.to_string(),
                    count: *count,
                    rank: look.rank,
                    gloss: look.entries.first().and_then(|e| e.glosses.first().cloned()).unwrap_or_default(),
                    example: example.to_string(),
                });
            }
        });
    }
    s
}

/// Difficulty and the pre-study word list for one prepared video.
#[tauri::command]
pub fn media_stats(state: State<AppState>, key: String) -> Res<Option<MediaStats>> {
    let Some(p) = load_prepared(&state, &key) else { return Ok(None) };
    let status = statuses(&state, &p.lang)?;
    Ok(Some(compute_stats(&state, &p, &status, true)))
}

/// Light-weight difficulty numbers for many videos at once (library badges).
#[tauri::command]
pub fn media_summaries(state: State<AppState>, keys: Vec<String>) -> Res<HashMap<String, MediaStats>> {
    let mut cache: HashMap<String, HashMap<String, i64>> = HashMap::new();
    let mut out = HashMap::new();
    for key in keys {
        if let Some(p) = load_prepared(&state, &key) {
            if !cache.contains_key(&p.lang) {
                cache.insert(p.lang.clone(), statuses(&state, &p.lang)?);
            }
            out.insert(key, compute_stats(&state, &p, &cache[&p.lang], false));
        }
    }
    Ok(out)
}

#[derive(Serialize)]
pub struct FileEntry {
    pub path: String,
    pub name: String,
    pub folder: String,
    pub key: String,
}

pub fn file_key(path: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.to_lowercase().hash(&mut h);
    format!("file-{:016x}", h.finish())
}

fn walk(dir: &Path, root: &Path, out: &mut Vec<FileEntry>, depth: u32) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth < 6 {
                walk(&p, root, out, depth + 1);
            }
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| VIDEO_EXT.contains(&x.to_lowercase().as_str()))
            .unwrap_or(false)
        {
            let path = p.to_string_lossy().to_string();
            out.push(FileEntry {
                key: file_key(&path),
                name: p.file_stem().unwrap_or_default().to_string_lossy().to_string(),
                folder: p.parent().and_then(|d| d.strip_prefix(root).ok()).map(|d| d.to_string_lossy().to_string()).unwrap_or_default(),
                path,
            });
        }
    }
}

/// List video files under a folder (for libraries that are plain folders rather than Plex).
#[tauri::command]
pub async fn scan_folder(path: String) -> Res<Vec<FileEntry>> {
    let files = tokio::task::spawn_blocking(move || {
        let root = PathBuf::from(&path);
        let mut out = vec![];
        walk(&root, &root, &mut out, 0);
        out.sort_by(|a, b| (a.folder.to_lowercase(), a.name.to_lowercase()).cmp(&(b.folder.to_lowercase(), b.name.to_lowercase())));
        out
    })
    .await?;
    Ok(files)
}

/// Inspect a local video file: its audio tracks, embedded subtitles and sidecar subtitle files.
#[tauri::command]
pub async fn probe_file(state: State<'_, AppState>, path: String) -> Res<MediaReq> {
    let ffprobe = components::require(&state, "ffprobe")?;
    let out = cmd(ffprobe)
        .args(["-v", "error", "-print_format", "json", "-show_streams", "-show_format"])
        .arg(&path)
        .output()
        .await?;
    if !out.status.success() {
        return Err(anyhow!("could not read {path}: {}", String::from_utf8_lossy(&out.stderr)).into());
    }
    let v: Value = serde_json::from_slice(&out.stdout)?;
    let p = Path::new(&path);
    let stem = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let mut req = MediaReq {
        key: file_key(&path),
        title: stem.clone(),
        url: path.clone(),
        duration: v["format"]["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0),
        ..Default::default()
    };
    for s in v["streams"].as_array().cloned().unwrap_or_default() {
        let index = s["index"].as_i64().unwrap_or(0);
        let lang = s["tags"]["language"].as_str().unwrap_or("").to_string();
        let title = s["tags"]["title"].as_str().unwrap_or("").to_string();
        match s["codec_type"].as_str() {
            Some("audio") => req.audio.push(AudioTrack { index, lang, title }),
            Some("subtitle") => req.subs.push(SubTrack {
                id: format!("e{index}"),
                lang,
                codec: s["codec_name"].as_str().unwrap_or("").to_string(),
                title,
                forced: s["disposition"]["forced"].as_i64() == Some(1),
                index: Some(index),
                ..Default::default()
            }),
            _ => {}
        }
    }
    // Sidecars: Movie.fr.srt, Movie.french.ass, Movie.srt
    if let Some(dir) = p.parent() {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let sp = e.path();
                let name = sp.file_name().unwrap_or_default().to_string_lossy().to_string();
                let ext = sp.extension().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
                if !["srt", "ass", "ssa", "vtt"].contains(&ext.as_str()) || !name.starts_with(&stem) {
                    continue;
                }
                let middle = name[stem.len()..name.len() - ext.len() - 1].trim_matches('.').to_string();
                let lang = middle.split('.').map(lang2).find(|l| l.len() == 2).unwrap_or_default();
                req.subs.push(SubTrack {
                    id: format!("f{}", req.subs.len()),
                    lang,
                    codec: ext,
                    title: name,
                    forced: middle.to_lowercase().contains("forced"),
                    path: Some(sp.to_string_lossy().to_string()),
                    ..Default::default()
                });
            }
        }
    }
    Ok(req)
}

/// If Plex's path for a file maps to a folder this PC can read, use the file directly.
#[tauri::command]
pub fn map_path(file: String, from: String, to: String) -> Option<String> {
    if from.is_empty() || to.is_empty() || !file.starts_with(&from) {
        return None;
    }
    let rest = file[from.len()..].trim_start_matches(['/', '\\']).replace('/', "\\");
    let mapped = Path::new(&to).join(rest);
    mapped.exists().then(|| mapped.to_string_lossy().to_string())
}
