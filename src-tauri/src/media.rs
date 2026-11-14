//! Turning a video into study material: pull subtitles and audio out of the file,
//! listen to the audio with Whisper, and line everything up.
use crate::align::{self, Heard};
use crate::components;
use crate::dict;
use crate::llm;
use crate::subs::{self, Cue};
use crate::util::{cmd, decode_text, lang2, Res};
use crate::AppState;
use anyhow::{anyhow, bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncBufReadExt, BufReader};

const TEXT_CODECS: &[&str] = &["subrip", "srt", "ass", "ssa", "mov_text", "tx3g", "webvtt", "vtt", "text"];
const VIDEO_EXT: &[&str] = &["mkv", "mp4", "m4v", "avi", "mov", "webm", "ts", "wmv"];
const PREPARED_VERSION: u32 = 2;

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
    /// Where the item lives on Plex, so missing subtitles can be fetched through the server.
    #[serde(default)]
    pub plex: Option<PlexRef>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PlexRef {
    pub url: String,
    pub token: String,
    pub id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PrepareOpts {
    pub target_lang: String,
    pub native_lang: String,
    /// "auto", "whisper" (transcribe the audio), "translate" (from the spoken language) or a track id.
    pub target_sub: String,
    /// "auto", "none" or a track id.
    pub native_sub: String,
    /// Listen to the audio to time each word.
    pub align: bool,
    /// Machine translation style: "natural" or "beginner".
    #[serde(default)]
    pub style: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Prepared {
    pub version: u32,
    pub key: String,
    pub title: String,
    /// Where the video was played from when it was prepared.
    #[serde(default)]
    pub url: String,
    pub lang: String,
    /// Language of the audio. Differs from `lang` for, say, an English film with French subtitles.
    #[serde(default)]
    pub spoken: String,
    /// The learned-language lines were machine-translated.
    #[serde(default)]
    pub translated: bool,
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
        let partial = (t.forced || title.contains("forc") || title.contains("sign") || title.contains("song")) as i32;
        partial * 10 - variant_score(lang, &t.lang, &t.title)
    });
    candidates.first().copied()
}

/// Prefer the regional variant the learner asked for. Spanish means Castilian (Spain)
/// here, so European tracks beat Latin American ones when a video has both.
fn variant_score(lang: &str, tag: &str, title: &str) -> i32 {
    if lang != "es" {
        return 0;
    }
    let text = format!("{} {}", tag.to_lowercase(), title.to_lowercase());
    let has = |keys: &[&str]| keys.iter().any(|k| text.contains(k));
    if has(&["es-es", "castellano", "castilian", "españa", "spain", "european", "europe"]) {
        1
    } else if has(&["es-419", "es-mx", "es-ar", "latin", "latino", "mexic", "américa", "america"]) {
        -1
    } else {
        0
    }
}

fn emit(app: &AppHandle, key: &str, stage: &str, pct: f64, note: &str) {
    if let Some(state) = app.try_state::<AppState>() {
        *state.progress.lock().unwrap() = Some((key.to_string(), stage.to_string(), pct));
    }
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
    let pid = child.id();
    track(state, pid, true);
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
    track(state, pid, false);
    let json_path = PathBuf::from(format!("{}.json", out_base.display()));
    if !status.success() || !json_path.exists() {
        bail!("speech recognition failed: {}", tail.join(" | "));
    }
    let raw = tokio::fs::read(&json_path).await?;
    let v: Value = serde_json::from_str(&String::from_utf8_lossy(&raw))?;
    Ok(words_from_whisper_json(&v, matches!(lang, "ja" | "zh")))
}

/// Turn whisper.cpp's full JSON into words. Each token carries `t_dtw`, the moment
/// (in 1/100 s) the token finishes being spoken; a word starts where the previous one ended.
pub fn words_from_whisper_json(v: &Value, unspaced: bool) -> Vec<Heard> {
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
            // Japanese and Chinese have no spaces, so every token stands alone.
            if unspaced || text.starts_with(' ') || words.is_empty() {
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
        let use_clean = !unspaced && clean.len() == words.len();
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
    fn subtitles_follow_the_retimed_reference() {
        let cue = |s: f64, e: f64| Cue { start: s, end: e, text: "x".into(), ..Default::default() };
        let before = vec![cue(10.0, 12.0), cue(20.0, 22.0)];
        let after = vec![cue(8.5, 10.4), cue(19.0, 21.0)];
        let mut french = vec![cue(10.2, 12.1), cue(19.8, 22.3)];
        follow_timing(&mut french, &before, &after);
        assert!((french[0].start - 8.7).abs() < 1e-9 && (french[0].end - 10.6).abs() < 1e-9);
        assert!((french[1].start - 18.8).abs() < 1e-9);
    }

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
        let w = words_from_whisper_json(&v, false);
        let texts: Vec<&str> = w.iter().map(|h| h.text.as_str()).collect();
        assert_eq!(texts, ["Où", "est", "l'élève"]);
        assert!((w[0].e - 1.30).abs() < 1e-9);
        // "est" cannot plausibly last longer than 0.275 s, so it starts at 1.325, not at 1.30.
        assert!((w[1].s - 1.325).abs() < 1e-9 && (w[1].e - 1.60).abs() < 1e-9);
        assert!((w[2].e - 2.30).abs() < 1e-9);
    }
}


fn fetch_enabled(state: &AppState) -> bool {
    let db = state.db.lock().unwrap();
    crate::db::setting(&db, "fetchSubs").as_bool().unwrap_or(true)
}

fn truthy(v: &Value) -> bool {
    v.as_bool().unwrap_or(false) || v.as_i64() == Some(1) || v.as_str() == Some("1")
}

/// Ask the Plex server to search for subtitles in `lang` and download the best match.
/// Plex adds them to the video as a separate subtitle file, so they also show up in Plex.
async fn plex_subtitles(state: &AppState, plex: &PlexRef, lang: &str) -> anyhow::Result<Option<SubTrack>> {
    use std::time::Duration;
    let base = format!("{}/library/metadata/{}", plex.url.trim_end_matches('/'), plex.id);
    let get = |url: &str| {
        state
            .http
            .get(url)
            .header("X-Plex-Token", &plex.token)
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(60))
    };
    let found: Value = get(&format!("{base}/subtitles"))
        .query(&[("language", lang), ("hearingImpaired", "3"), ("forced", "3")])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let results = found["MediaContainer"]["Stream"].as_array().cloned().unwrap_or_default();
    let best = results.iter().filter(|s| s["key"].is_string()).max_by_key(|s| {
        let tag = s["languageTag"].as_str().or(s["languageCode"].as_str()).unwrap_or("");
        let title = format!("{} {}", s["displayTitle"].as_str().unwrap_or(""), s["title"].as_str().unwrap_or(""));
        (variant_score(lang, tag, &title), truthy(&s["perfectMatch"]) as i32, s["score"].as_i64().unwrap_or(0))
    });
    let Some(best) = best else { return Ok(None) };
    let chosen = best["key"].as_str().unwrap_or_default().to_string();
    state
        .http
        .put(format!("{base}/subtitles"))
        .query(&[("key", chosen.as_str())])
        .header("X-Plex-Token", &plex.token)
        .timeout(Duration::from_secs(60))
        .send()
        .await?
        .error_for_status()?;
    // Plex downloads in the background; wait for the file to appear on the video.
    for _ in 0..45 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let md: Value = get(&base).send().await?.error_for_status()?.json().await?;
        let streams = md["MediaContainer"]["Metadata"][0]["Media"][0]["Part"][0]["Stream"].as_array().cloned().unwrap_or_default();
        for st in streams {
            let tag = st["languageTag"].as_str().or(st["languageCode"].as_str()).unwrap_or("").to_string();
            let (Some(path), true) = (st["key"].as_str(), st["streamType"].as_i64() == Some(3)) else { continue };
            let track = SubTrack {
                id: format!("p{}", st["id"]),
                lang: tag.clone(),
                codec: st["codec"].as_str().unwrap_or("srt").to_string(),
                title: st["displayTitle"].as_str().unwrap_or("").to_string(),
                forced: false,
                index: None,
                url: Some(format!(
                    "{}{}{}X-Plex-Token={}",
                    plex.url.trim_end_matches('/'),
                    path,
                    if path.contains('?') { "&" } else { "?" },
                    plex.token
                )),
                path: None,
            };
            if lang2(&tag) == lang && is_text(&track) {
                return Ok(Some(track));
            }
        }
    }
    bail!("the download did not finish in time")
}

/// Tokenise every cue (with the dictionary, when it is installed).
fn tokenise(state: &AppState, lang: &str, cues: &mut [Cue]) -> anyhow::Result<()> {
    if components::locate(state, &format!("dict-{lang}")).is_some() {
        dict::with_dict(state, lang, |d| {
            for c in cues.iter_mut() {
                c.tokens = subs::tokenize(lang, &c.text, Some(d));
            }
        })?;
    } else {
        for c in cues.iter_mut() {
            c.tokens = subs::tokenize(lang, &c.text, None);
        }
    }
    Ok(())
}

/// Move cues the way the reference cues they overlap moved when those were lined up with
/// the speech. `before` and `after` are the same reference cues before and after alignment.
fn follow_timing(cues: &mut [Cue], before: &[Cue], after: &[Cue]) {
    if before.is_empty() || before.len() != after.len() {
        return;
    }
    let mut from = 0usize;
    for c in cues.iter_mut() {
        while from + 1 < before.len() && before[from].end < c.start - 10.0 {
            from += 1;
        }
        let mut best: Option<(usize, f64)> = None;
        let mut k = from;
        while k < before.len() && before[k].start < c.end + 10.0 {
            let overlap = c.end.min(before[k].end) - c.start.max(before[k].start);
            if best.map(|(_, o)| overlap > o).unwrap_or(true) {
                best = Some((k, overlap));
            }
            k += 1;
        }
        let k = best.map(|(k, _)| k).unwrap_or(from);
        let shift = after[k].start - before[k].start;
        c.start = (c.start + shift).max(0.0);
        c.end = (c.end + shift).max(c.start + 0.2);
    }
    for k in 1..cues.len() {
        if cues[k - 1].end > cues[k].start {
            cues[k - 1].end = cues[k].start.max(cues[k - 1].start + 0.05);
        }
    }
}

/// Pull embedded subtitle tracks and (optionally) the audio out of the file in one FFmpeg pass.
async fn extract(
    app: &AppHandle,
    state: &AppState,
    req: &MediaReq,
    subs_out: &[(i64, &Path)],
    audio_out: Option<(String, &Path)>,
) -> anyhow::Result<()> {
    if subs_out.is_empty() && audio_out.is_none() {
        return Ok(());
    }
    let key = req.key.as_str();
    let ffmpeg = components::require(state, "ffmpeg")?;
    let mut c = cmd(&ffmpeg);
    c.args(["-y", "-v", "error", "-nostdin", "-progress", "pipe:1", "-i"]).arg(&req.url);
    for (i, path) in subs_out {
        c.args(["-map", &format!("0:{i}"), "-c:s", "srt"]).arg(path);
    }
    if let Some((map, wav)) = &audio_out {
        c.args(["-map", map, "-vn", "-sn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"]).arg(wav);
    }
    emit(app, key, "extract", 0.0, "Reading subtitles and audio from the file");
    let mut child = c.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().context("starting ffmpeg")?;
    let pid = child.id();
    track(state, pid, true);
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
    track(state, pid, false);
    let err = err_task.await.unwrap_or_default();
    if !status.success() {
        bail!("FFmpeg could not read the video: {}", err.lines().last().unwrap_or("unknown error"));
    }
    Ok(())
}

/// Remember (or forget) a helper process so it can be stopped when the app closes.
pub fn track(state: &AppState, pid: Option<u32>, add: bool) {
    if let Some(pid) = pid {
        let mut set = state.children.lock().unwrap();
        if add {
            set.insert(pid);
        } else {
            set.remove(&pid);
        }
    }
}

async fn read_track(state: &AppState, track: Option<&SubTrack>, embedded: &Path, tmp: &Path) -> anyhow::Result<Vec<Cue>> {
    match track {
        None => Ok(vec![]),
        Some(t) if t.index.is_some() => Ok(subs::parse_srt(&decode_text(&tokio::fs::read(embedded).await?))),
        Some(t) => read_external(state, t, tmp).await,
    }
}

/// Prepare one video. Called by the background queue, one video at a time.
pub async fn prepare(app: &AppHandle, state: &AppState, req: &MediaReq, opts: &PrepareOpts) -> anyhow::Result<Prepared> {
    let key = req.key.as_str();
    let tmp = state.dir.join("tmp").join(format!("prep-{}", uuid::Uuid::new_v4().simple()));
    tokio::fs::create_dir_all(&tmp).await?;
    let result = prepare_in(app, state, req, opts, &tmp).await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    let prepared = result?;
    let path = prepared_path(state, key);
    tokio::fs::create_dir_all(path.parent().unwrap()).await?;
    tokio::fs::write(&path, serde_json::to_vec(&prepared)?).await?;
    emit(app, key, "done", 100.0, "");
    Ok(prepared)
}

async fn prepare_in(app: &AppHandle, state: &AppState, req: &MediaReq, opts: &PrepareOpts, tmp: &Path) -> anyhow::Result<Prepared> {
    let key = req.key.as_str();
    let can_listen = components::locate(state, "whisper").is_some()
        && components::locate(state, "whisper-model").is_some();
    let audio = req
        .audio
        .iter()
        .filter(|a| lang2(&a.lang) == opts.target_lang)
        .max_by_key(|a| variant_score(&opts.target_lang, &a.lang, &a.title))
        .or_else(|| req.audio.first());
    // The language actually spoken. Unlabelled audio is assumed to be the one being learned.
    let spoken = audio
        .map(|a| lang2(&a.lang))
        .filter(|l| !l.is_empty() && !matches!(l.as_str(), "und" | "zxx" | "mul" | "mis"))
        .unwrap_or_else(|| opts.target_lang.clone());
    if spoken != opts.target_lang {
        return prepare_foreign(app, state, req, opts, tmp, audio, &spoken, can_listen).await;
    }

    let target = if opts.target_sub == "whisper" { None } else { pick_sub(&req.subs, &opts.target_lang, &opts.target_sub) };
    let native = if opts.native_sub == "none" { None } else { pick_sub(&req.subs, &opts.native_lang, &opts.native_sub) };
    if target.is_none() && !can_listen {
        bail!(
            "This video has no {} text subtitles. Install the speech recognition components in Setup to transcribe the audio instead.",
            llm::lang_name(&opts.target_lang)
        );
    }
    let listen = can_listen && (opts.align || target.is_none());

    let (t_srt, n_srt, wav) = (tmp.join("target.srt"), tmp.join("native.srt"), tmp.join("audio.wav"));
    let mut subs_out: Vec<(i64, &Path)> = vec![];
    if let Some(i) = target.and_then(|t| t.index) {
        subs_out.push((i, &t_srt));
    }
    if let Some(i) = native.and_then(|t| t.index) {
        subs_out.push((i, &n_srt));
    }
    let map = audio.map(|a| format!("0:{}", a.index)).unwrap_or_else(|| "0:a:0".into());
    extract(app, state, req, &subs_out, listen.then(|| (map, wav.as_path()))).await?;

    let mut cues = read_track(state, target, &t_srt, tmp).await.context("reading target-language subtitles")?;
    let native_cues = read_track(state, native, &n_srt, tmp).await.unwrap_or_default();

    let mut heard: Vec<Heard> = vec![];
    if listen {
        // Speech recognition needs the graphics memory; the translation model reloads later if needed.
        llm::unload(state).await;
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
        cues = align::cues_from_heard(&heard, !matches!(opts.target_lang.as_str(), "ja" | "zh"));
        target_source = "speech";
    }
    let lang = opts.target_lang.clone();
    let mut prepared = Prepared {
        version: PREPARED_VERSION,
        key: key.to_string(),
        title: req.title.clone(),
        url: req.url.clone(),
        lang: lang.clone(),
        spoken: spoken.clone(),
        target_source: target_source.into(),
        native_source: if native_cues.is_empty() { "none".into() } else { "subtitles".into() },
        audio_index: audio.map(|a| a.index),
        ..Default::default()
    };
    tokenise(state, &lang, &mut cues)?;
    if heard.is_empty() {
        cues.iter_mut().for_each(subs::spread_tokens);
    } else {
        let original = cues.clone();
        let stats = align::align(&mut cues, &heard);
        let rate = stats.matched as f64 / stats.total.max(1) as f64;
        if rate < 0.3 && target_source != "speech" {
            // The audio does not say what the subtitles say (a dub with its own script).
            // Keep the file's own timing.
            cues = original;
            cues.iter_mut().for_each(subs::spread_tokens);
            prepared.note = format!(
                "Only {:.0}% of subtitle words were heard in the audio, so word timing is estimated.",
                rate * 100.0
            );
        } else {
            prepared.aligned = true;
            prepared.offset = stats.offset;
        }
    }
    subs::pair_native(&mut cues, &native_cues);
    prepared.cues = cues;
    Ok(prepared)
}

/// The audio is in another language than the one being learned: an English film studied
/// with French subtitles. Lines are timed to the speech through the English subtitles (or
/// what Whisper hears), and French lines are translated by the local model when the video
/// has none of its own.
#[allow(clippy::too_many_arguments)]
async fn prepare_foreign(
    app: &AppHandle,
    state: &AppState,
    req: &MediaReq,
    opts: &PrepareOpts,
    tmp: &Path,
    audio: Option<&AudioTrack>,
    spoken: &str,
    can_listen: bool,
) -> anyhow::Result<Prepared> {
    let key = req.key.as_str();
    let (target_name, spoken_name) = (llm::lang_name(&opts.target_lang), llm::lang_name(spoken));
    let force_translate = matches!(opts.target_sub.as_str(), "translate" | "whisper");
    let mut target = if force_translate { None } else { pick_sub(&req.subs, &opts.target_lang, &opts.target_sub) };
    // Real subtitles beat a machine translation: ask the Plex server to find some (OpenSubtitles).
    let mut fetched: Option<SubTrack> = None;
    let mut fetch_note = String::new();
    if target.is_none() && !force_translate && opts.target_sub == "auto" {
        if let Some(px) = req.plex.as_ref().filter(|_| fetch_enabled(state)) {
            emit(app, key, "subtitles", 0.0, &format!("Looking for {target_name} subtitles through Plex"));
            match plex_subtitles(state, px, &opts.target_lang).await {
                Ok(Some(t)) => {
                    fetch_note = format!(" The {target_name} subtitles were downloaded through your Plex server.");
                    fetched = Some(t);
                }
                Ok(None) => fetch_note = format!(" Plex found no {target_name} subtitles for this video."),
                Err(e) => fetch_note = format!(" Plex could not fetch {target_name} subtitles ({e:#})."),
            }
        }
    }
    if fetched.is_some() {
        target = fetched.as_ref();
    }
    let native = if opts.native_sub == "none" { None } else { pick_sub(&req.subs, &opts.native_lang, &opts.native_sub) };
    // Lines in the spoken language: the source for timing, and for translation.
    let source = native.or_else(|| pick_sub(&req.subs, spoken, "auto"));
    if target.is_none() {
        if !llm::available(state) {
            bail!(
                "The audio is {spoken_name} and the video has no {target_name} subtitles. Install the local language model in Setup \
                 so LingoReel can translate the {spoken_name} subtitles, or add a {target_name} subtitle file to the video."
            );
        }
        if source.is_none() && !can_listen {
            bail!(
                "The video has neither {target_name} nor {spoken_name} subtitles. Install speech recognition in Setup so LingoReel \
                 can write the {spoken_name} lines from the audio and translate them."
            );
        }
    }
    let listen = can_listen && (source.is_none() || opts.align);

    let (t_srt, s_srt, wav) = (tmp.join("target.srt"), tmp.join("source.srt"), tmp.join("audio.wav"));
    let mut subs_out: Vec<(i64, &Path)> = vec![];
    if let Some(i) = target.and_then(|t| t.index) {
        subs_out.push((i, &t_srt));
    }
    if let Some(i) = source.and_then(|t| t.index) {
        subs_out.push((i, &s_srt));
    }
    let map = audio.map(|a| format!("0:{}", a.index)).unwrap_or_else(|| "0:a:0".into());
    extract(app, state, req, &subs_out, listen.then(|| (map, wav.as_path()))).await?;

    let mut cues = read_track(state, target, &t_srt, tmp).await.context("reading target-language subtitles")?;
    let mut reference = read_track(state, source, &s_srt, tmp).await.unwrap_or_default();

    let mut heard: Vec<Heard> = vec![];
    if listen {
        // Speech recognition needs the graphics memory; the translation model reloads later if needed.
        llm::unload(state).await;
        emit(app, key, "listen", 0.0, "Listening to the audio");
        let app2 = app.clone();
        let key2 = key.to_string();
        heard = whisper_words(state, &wav, spoken, &tmp.join("whisper"), move |p| {
            emit(&app2, &key2, "listen", p, "Listening to the audio")
        })
        .await?;
    }

    emit(app, key, "align", 0.0, "Lining the subtitles up with the speech");
    let mut note = format!("{spoken_name} audio: each line is timed to the speech; words are not highlighted one by one.");
    let mut offset = 0.0;
    let mut reference_source = "subtitles";
    if reference.is_empty() {
        if heard.is_empty() && cues.is_empty() {
            bail!("No subtitles could be read and no speech was recognised.");
        }
        if !heard.is_empty() {
            reference = align::cues_from_heard(&heard, !matches!(spoken, "ja" | "zh"));
            reference_source = "speech";
        }
    } else if !heard.is_empty() {
        for c in reference.iter_mut() {
            c.tokens = subs::tokenize(spoken, &c.text, None);
        }
        let before = reference.clone();
        let stats = align::align(&mut reference, &heard);
        if stats.matched as f64 / stats.total.max(1) as f64 >= 0.3 {
            offset = stats.offset;
            // Subtitles in the learned language move with the ones that match the audio.
            follow_timing(&mut cues, &before, &reference);
        } else {
            reference = before;
        }
    }

    let mut translated = false;
    let mut target_source = target.map(|t| if t.index.is_some() { "embedded" } else { "external" }).unwrap_or("translated");
    if cues.is_empty() {
        if reference.is_empty() {
            bail!("There are no {spoken_name} lines to translate.");
        }
        let lines: Vec<String> = reference.iter().map(|c| c.text.clone()).collect();
        let message = format!("Translating the {spoken_name} lines into {target_name}");
        emit(app, key, "translate", 0.0, &message);
        let app2 = app.clone();
        let key2 = key.to_string();
        let out = llm::translate_lines(state, spoken, &opts.target_lang, &lines, &opts.style, move |p| {
            emit(&app2, &key2, "translate", p, &message)
        })
        .await?;
        cues = reference
            .iter()
            .zip(out)
            .filter(|(_, t)| !t.trim().is_empty())
            .map(|(c, t)| Cue { start: c.start, end: c.end, text: t.trim().to_string(), ..Default::default() })
            .collect();
        translated = true;
        target_source = if opts.style == "beginner" { "translated-simple" } else { "translated" };
        note.push_str(&format!(
            " The {target_name} lines were translated from the {spoken_name} by the local model and can contain mistakes."
        ));
    }

    note.push_str(&fetch_note);
    tokenise(state, &opts.target_lang, &mut cues)?;
    cues.iter_mut().for_each(subs::spread_tokens);
    // The second line shows the spoken language when that is the learner's own language.
    let native_lines: &[Cue] = if spoken == opts.native_lang || native.is_some() { &reference } else { &[] };
    subs::pair_native(&mut cues, native_lines);
    Ok(Prepared {
        version: PREPARED_VERSION,
        key: key.to_string(),
        title: req.title.clone(),
        url: req.url.clone(),
        lang: opts.target_lang.clone(),
        spoken: spoken.to_string(),
        translated,
        cues,
        aligned: false,
        target_source: target_source.into(),
        native_source: if native_lines.is_empty() { "none".into() } else { reference_source.into() },
        audio_index: audio.map(|a| a.index),
        offset,
        note,
    })
}

#[tauri::command]
pub async fn prepare_media(app: AppHandle, state: State<'_, AppState>, req: MediaReq, opts: PrepareOpts) -> Res<Prepared> {
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
pub struct MinedLine {
    pub cue: usize,
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub en: String,
    pub lemma: String,
    pub word: String,
    pub gloss: String,
    pub rank: Option<i64>,
}

/// Sentence mining: lines where exactly one word is not known yet. One line per word,
/// most common words first, skipping words that already have a review card.
#[tauri::command]
pub fn media_mine(state: State<AppState>, key: String) -> Res<Vec<MinedLine>> {
    let Some(p) = load_prepared(&state, &key) else { return Ok(vec![]) };
    let status = statuses(&state, &p.lang)?;
    let carded: std::collections::HashSet<String> = {
        let db = state.db.lock().unwrap();
        let mut st = db.prepare("SELECT DISTINCT lemma FROM cards WHERE lang=?1")?;
        let rows = st.query_map([&p.lang], |r| r.get::<_, String>(0))?;
        rows.flatten().collect()
    };
    let mut picked: HashMap<String, (usize, String)> = HashMap::new();
    for (i, cue) in p.cues.iter().enumerate() {
        let words: Vec<_> = cue.tokens.iter().filter(|t| t.w && t.lemma.is_some()).collect();
        if words.len() < 3 {
            continue;
        }
        let mut unknown: Vec<(&str, &str)> = vec![];
        for t in &words {
            let l = t.lemma.as_deref().unwrap();
            if status.get(l).copied().unwrap_or(0) < 2 && !unknown.iter().any(|(x, _)| *x == l) {
                unknown.push((l, t.t.as_str()));
            }
        }
        if let [(lemma, word)] = unknown[..] {
            if !carded.contains(lemma) && !picked.contains_key(lemma) {
                picked.insert(lemma.to_string(), (i, word.to_string()));
            }
        }
    }
    let mut out: Vec<MinedLine> = vec![];
    dict::with_dict(&state, &p.lang, |d| {
        for (lemma, (i, word)) in picked {
            let look = d.lookup(&lemma);
            let cue = &p.cues[i];
            out.push(MinedLine {
                cue: i,
                start: cue.start,
                end: cue.end,
                text: cue.text.clone(),
                en: cue.en.clone(),
                gloss: look.entries.first().map(|e| e.glosses.iter().take(3).cloned().collect::<Vec<_>>().join("; ")).unwrap_or_default(),
                rank: look.rank,
                lemma,
                word,
            });
        }
    })?;
    out.sort_by_key(|m| (m.rank.unwrap_or(i64::MAX), m.cue));
    out.truncate(150);
    Ok(out)
}

#[derive(Serialize)]
pub struct Occurrence {
    pub key: String,
    pub title: String,
    pub url: String,
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub en: String,
    pub word: String,
}

/// Every prepared scene where a word is spoken, spread across as many videos as possible.
#[tauri::command(async)]
pub fn word_occurrences(state: State<AppState>, lang: String, lemma: String, skip_key: String, skip_start: f64) -> Res<Vec<Occurrence>> {
    const PER_VIDEO: usize = 4;
    const TOTAL: usize = 24;
    let mut per_video: Vec<Vec<Occurrence>> = vec![];
    let Ok(rd) = std::fs::read_dir(state.dir.join("media")) else { return Ok(vec![]) };
    for e in rd.flatten() {
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        // Cheap pre-filter before parsing a few hundred kilobytes of JSON.
        if !text.contains(&format!("\"lemma\":\"{lemma}\"")) {
            continue;
        }
        let Ok(p) = serde_json::from_str::<Prepared>(&text) else { continue };
        if p.lang != lang || p.version != PREPARED_VERSION {
            continue;
        }
        let mut hits = vec![];
        for cue in &p.cues {
            if p.key == skip_key && (cue.start - skip_start).abs() < 0.01 {
                continue;
            }
            if let Some(t) = cue.tokens.iter().find(|t| t.lemma.as_deref() == Some(lemma.as_str())) {
                hits.push(Occurrence {
                    key: p.key.clone(),
                    title: p.title.clone(),
                    url: p.url.clone(),
                    start: cue.start,
                    end: cue.end,
                    text: cue.text.clone(),
                    en: cue.en.clone(),
                    word: t.t.clone(),
                });
                if hits.len() >= PER_VIDEO {
                    break;
                }
            }
        }
        if !hits.is_empty() {
            per_video.push(hits);
        }
    }
    // Round-robin so the list mixes videos instead of exhausting the first one.
    let mut out = vec![];
    let mut round = 0;
    while out.len() < TOTAL && per_video.iter().any(|v| v.len() > round) {
        for v in per_video.iter_mut() {
            if v.len() > round && out.len() < TOTAL {
                out.push(std::mem::replace(
                    &mut v[round],
                    Occurrence { key: String::new(), title: String::new(), url: String::new(), start: 0.0, end: 0.0, text: String::new(), en: String::new(), word: String::new() },
                ));
            }
        }
        round += 1;
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
