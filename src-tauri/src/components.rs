//! Things the app downloads once on first run instead of shipping in the installer:
//! the video engine, speech recognition, dictionary and (optionally) a local LLM.
use crate::util::{cmd, find_file, on_path, Res};
use crate::AppState;
use anyhow::{anyhow, bail, Context};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, State};
use tokio::io::AsyncWriteExt;

const WHISPER_RELEASE: &str = "https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.2";
const LLAMA_TAG: &str = "b12304";
const WHISPER_MODELS: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1";
const LLM_REPO: &str = "https://huggingface.co/bartowski/google_gemma-3-4b-it-GGUF/resolve/71506238f970075ca85125cd749c28b1b0eee84e";
const LLM_LARGE_REPO: &str = "https://huggingface.co/bartowski/google_gemma-3-12b-it-GGUF/resolve/648e3a36a77c8a9f12d86e741f9dcb9089c769c4";

/// SHA-256 of every file with a fixed version, by file name. Most of these are programs we then run:
/// nothing is used unless it matches. Update the hash together with the version it belongs to.
const PINNED: &[(&str, &str)] = &[
    ("whisper-bin-x64.zip", "49dcc16de826f20bd53d44f947a1ae49dfa81f86cad67a64d80820cb192d674a"),
    ("whisper-cublas-12.4.0-bin-x64.zip", "443110ddaad70d4290ab2e77179e31cf712035bbc4fad56bb4519a90c917b39c"),
    ("llama-b12304-bin-win-vulkan-x64.zip", "d8b990953aca480412581521045a52f7263db84c5f2cbb89ada9aa47413ce6b1"),
    ("piper_windows_amd64.zip", "f3c58906402b24f3a96d92145f58acba6d86c9b5db896d207f78dc80811efcea"),
    ("ggml-large-v3-turbo.bin", "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69"),
    ("ggml-small.bin", "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b"),
    ("google_gemma-3-4b-it-Q4_K_M.gguf", "4996030242583a40aa151ff93f49ed787ac8c25e4120c3ae4588b2e2a7d1ae94"),
    ("google_gemma-3-12b-it-Q4_K_M.gguf", "fc57f67efa46d711c346e587cbef7d049e95f3df8db2eb2271153343ef0acc7b"),
    ("fr_FR-siwis-medium.onnx", "641d1ab097da2b81128c076810edb052b385decc8be3381814802a64a73baf99"),
    ("fr_FR-siwis-medium.onnx.json", "39479916c2db192b5ac9764daddd0c744d83e023ad890c6976c0633ae4df8959"),
    ("es_ES-davefx-medium.onnx", "6658b03b1a6c316ee4c265a9896abc1393353c2d9e1bca7d66c2c442e222a917"),
    ("es_ES-davefx-medium.onnx.json", "0e0dda87c732f6f38771ff274a6380d9252f327dca77aa2963d5fbdf9ec54842"),
    ("de_DE-thorsten-medium.onnx", "7e64762d8e5118bb578f2eea6207e1a35a8e0c30595010b666f983fc87bb7819"),
    ("de_DE-thorsten-medium.onnx.json", "974adee790533adb273a1ac88f49027d2a1b8f0f2cf4905954a4791e79264e85"),
    ("it_IT-paola-medium.onnx", "6fc918b5a0ea6137382833dddfa567bffbe6a5060c02043c87192ee59c04210c"),
    ("it_IT-paola-medium.onnx.json", "aea19c0a7fce29fbc359b93f10e7902854401e4c95ae2ea328ae516b15d296cf"),
    ("pt_BR-faber-medium.onnx", "858555e3a064209c57088fe6bd70c4c3dc54d03eaa00c45d5ecaf43a33f95aa7"),
    ("pt_BR-faber-medium.onnx.json", "7e694de195ae3fc36dd732c445eb04fb49b649854893cb5506b978f0d50a1d6f"),
    ("en_US-lessac-medium.onnx", "5efe09e69902187827af646e1a6e9d269dee769f9877d17b16b1b46eeaaf019f"),
    ("en_US-lessac-medium.onnx.json", "efe19c417bed055f2d69908248c6ba650fa135bc868b0e6abb3da181dab690a0"),
];

/// A file to fetch and the SHA-256 it must have.
struct Source {
    url: String,
    sha256: String,
}

fn pinned(url: String) -> anyhow::Result<Source> {
    let name = url.rsplit('/').next().unwrap_or_default();
    let sha256 = PINNED.iter().find(|p| p.0 == name).map(|p| p.1.to_string()).with_context(|| format!("no checksum recorded for {name}"))?;
    Ok(Source { url, sha256 })
}

/// A file from a release whose files change over time (mpv, FFmpeg, our own data release):
/// checked against the SHA-256 GitHub publishes for it.
async fn released(state: &AppState, repo: &str, release: &str, wanted: impl Fn(&str) -> bool) -> anyhow::Result<Source> {
    let rel: serde_json::Value = state
        .http
        .get(format!("https://api.github.com/repos/{repo}/releases/{release}"))
        .header("User-Agent", "LingoReel")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    pick_asset(&rel, wanted).with_context(|| format!("no verifiable download found in {repo} ({release})"))
}

fn pick_asset(release: &serde_json::Value, wanted: impl Fn(&str) -> bool) -> Option<Source> {
    let asset = release["assets"].as_array()?.iter().find(|a| a["name"].as_str().is_some_and(&wanted))?;
    Some(Source {
        url: asset["browser_download_url"].as_str()?.to_string(),
        sha256: asset["digest"].as_str()?.strip_prefix("sha256:")?.to_lowercase(),
    })
}

#[derive(Serialize, Clone)]
pub struct Component {
    pub id: String,
    pub name: String,
    pub about: String,
    pub size_mb: u32,
    pub required: bool,
    pub installed: bool,
    /// Where it was found, when it is one we did not download ourselves.
    pub note: String,
}

struct Spec {
    id: &'static str,
    name: &'static str,
    about: &'static str,
    size_mb: u32,
    required: bool,
}

/// Light setup: the small speech model even on a PC with a graphics card.
pub static LIGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn light() -> bool {
    LIGHT.load(std::sync::atomic::Ordering::Relaxed) || gpu() == Gpu::None
}

pub fn set_light_from(settings: &serde_json::Value) {
    LIGHT.store(settings["whisperSize"].as_str() == Some("small"), std::sync::atomic::Ordering::Relaxed);
}

pub fn has_nvidia() -> bool {
    on_path("nvidia-smi.exe").is_some() || Path::new(r"C:\Windows\System32\nvidia-smi.exe").exists()
}

/// Which speech-recognition build suits this PC.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Gpu {
    /// NVIDIA: CUDA build.
    Nvidia,
    /// AMD Radeon or Intel Arc: Vulkan build.
    Vulkan,
    /// No suitable graphics card: CPU build with a smaller model.
    None,
}

pub fn gpu() -> Gpu {
    static DETECTED: std::sync::OnceLock<Gpu> = std::sync::OnceLock::new();
    *DETECTED.get_or_init(|| {
        match std::env::var("LINGOREEL_GPU").as_deref() {
            Ok("nvidia") => return Gpu::Nvidia,
            Ok("vulkan") => return Gpu::Vulkan,
            Ok("none") => return Gpu::None,
            _ => {}
        }
        if has_nvidia() {
            Gpu::Nvidia
        } else if has_vulkan_card() {
            Gpu::Vulkan
        } else {
            Gpu::None
        }
    })
}

/// True for a dedicated AMD or Intel graphics card with a Vulkan driver installed.
fn has_vulkan_card() -> bool {
    if !Path::new(r"C:\Windows\System32\vulkan-1.dll").exists() {
        return false;
    }
    let mut c = std::process::Command::new("reg");
    c.args([
        "query",
        r"HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}",
        "/s",
        "/v",
        "DriverDesc",
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let Ok(out) = c.output() else { return false };
    let names = String::from_utf8_lossy(&out.stdout).to_lowercase();
    // "Radeon RX 9070 XT", "Radeon RX 7800", "Intel(R) Arc(TM) A770". Integrated chips are too slow.
    names.contains("radeon rx") || names.contains("radeon pro") || names.contains("arc(tm)") || names.contains("intel arc")
}

/// Languages with a dictionary: (code, name, download size in MB).
pub const LANGUAGES: &[(&str, &str, u32)] = &[
    ("fr", "French", 22),
    ("es", "Spanish", 61),
    ("de", "German", 58),
    ("it", "Italian", 42),
    ("pt", "Portuguese", 22),
    ("ja", "Japanese", 30),
];

/// Offline voices (Piper) per studied language: folder on the voices repository and file name.
/// Spanish uses a voice from Spain. Japanese has none that this Piper build can read.
pub const VOICES: &[(&str, &str, &str)] = &[
    ("fr", "fr/fr_FR/siwis/medium", "fr_FR-siwis-medium"),
    ("es", "es/es_ES/davefx/medium", "es_ES-davefx-medium"),
    ("de", "de/de_DE/thorsten/medium", "de_DE-thorsten-medium"),
    ("it", "it/it_IT/paola/medium", "it_IT-paola-medium"),
    ("pt", "pt/pt_BR/faber/medium", "pt_BR-faber-medium"),
    // For the English side: mixed subtitles and the translation on review cards.
    ("en", "en/en_US/lessac/medium", "en_US-lessac-medium"),
];
const PIPER_URL: &str = "https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_windows_amd64.zip";
// Pinned: voices retrained for newer Piper versions use sound codes this engine cannot read.
const VOICE_BASE: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0";

fn voice(id: &str) -> Option<&'static (&'static str, &'static str, &'static str)> {
    let code = id.strip_prefix("voice-")?;
    VOICES.iter().find(|v| v.0 == code)
}

struct DictSpec {
    id: String,
    name: String,
    size_mb: u32,
}

fn specs() -> Vec<Spec> {
    let card = gpu();
    let gpu = !light();
    vec![
        Spec { id: "mpv", name: "Video player engine (mpv)", about: "Plays every video format, including the ones on your server.", size_mb: 35, required: true },
        Spec { id: "ffmpeg", name: "Media tools (FFmpeg)", about: "Reads subtitle and audio tracks out of your video files.", size_mb: 200, required: true },
        Spec {
            id: "whisper",
            name: "Speech recognition engine (Whisper)",
            about: match card {
                Gpu::Nvidia => "Listens to the audio to time every word. NVIDIA graphics card build.",
                Gpu::Vulkan => "Listens to the audio to time every word. Build for AMD and Intel graphics cards.",
                Gpu::None => "Listens to the audio to time every word. Processor-only build (slow without a graphics card).",
            },
            size_mb: match card {
                Gpu::Nvidia => 670,
                Gpu::Vulkan => 25,
                Gpu::None => 8,
            },
            required: false,
        },
        Spec {
            id: "whisper-model",
            name: "Speech recognition model",
            about: if gpu { "large-v3-turbo: accurate, a few minutes per episode on a recent graphics card." } else { "small: a quicker download and lighter on the PC, a little less accurate." },
            size_mb: if gpu { 1625 } else { 490 },
            required: false,
        },
        Spec { id: "piper", name: "Voice engine (Piper)", about: "Reads words and lines aloud, offline. Most useful for films in English.", size_mb: 22, required: false },
        Spec { id: "llama", name: "Local LLM engine (llama.cpp)", about: "Runs the explanation model on your own machine. Nothing leaves your PC.", size_mb: 40, required: false },
        Spec { id: "llm-model", name: "Explanation model (Gemma 3 4B)", about: "Explains phrases, idioms and grammar in context, fully offline.", size_mb: 2490, required: false },
        Spec {
            id: "llm-model-large",
            name: "Better model (Gemma 3 12B, optional)",
            about: "Noticeably better translations and explanations. Needs about 9 GB of free graphics memory; choose it in Settings.",
            size_mb: 7300,
            required: false,
        },
    ]
}

pub fn dir(state: &AppState, id: &str) -> PathBuf {
    // LINGOREEL_COMPONENTS lets a development build share an existing download folder.
    let base = std::env::var_os("LINGOREEL_COMPONENTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| state.dir.join("components"));
    match id {
        "whisper-model" | "llm-model" | "llm-model-large" => base.join("models"),
        id if id.starts_with("dict-") => base.join("dict"),
        id if id.starts_with("voice-") => base.join("voices"),
        other => base.join(other),
    }
}

pub fn whisper_model_name() -> &'static str {
    if !light() { "ggml-large-v3-turbo.bin" } else { "ggml-small.bin" }
}

/// Alignment-head preset matching the installed Whisper model.
pub fn whisper_dtw_preset() -> &'static str {
    if !light() { "large.v3.turbo" } else { "small" }
}

const LLM_MODEL_FILE: &str = "google_gemma-3-4b-it-Q4_K_M.gguf";
const LLM_LARGE_FILE: &str = "google_gemma-3-12b-it-Q4_K_M.gguf";

/// Path of an installed component's main file, if present.
pub fn locate(state: &AppState, id: &str) -> Option<PathBuf> {
    let d = dir(state, id);
    match id {
        "mpv" => find_file(&d, "mpv.exe"),
        "ffmpeg" => find_file(&d, "ffmpeg.exe").or_else(|| on_path("ffmpeg.exe")),
        "ffprobe" => find_file(&dir(state, "ffmpeg"), "ffprobe.exe").or_else(|| on_path("ffprobe.exe")),
        "whisper" => find_file(&d, "whisper-cli.exe"),
        "whisper-model" => Some(d.join(whisper_model_name())).filter(|p| p.exists()),
        "llama" => find_file(&d, "llama-server.exe"),
        "llm-model" => Some(d.join(LLM_MODEL_FILE)).filter(|p| p.exists()),
        "llm-model-large" => Some(d.join(LLM_LARGE_FILE)).filter(|p| p.exists()),
        id if id.starts_with("dict-") => Some(d.join(format!("{id}.sqlite"))).filter(|p| p.exists()),
        "piper" => find_file(&d, "piper.exe"),
        id if id.starts_with("voice-") => {
            let (_, _, file) = voice(id)?;
            let model = d.join(format!("{file}.onnx"));
            (model.exists() && d.join(format!("{file}.onnx.json")).exists()).then_some(model)
        }
        _ => None,
    }
}

pub fn require(state: &AppState, id: &str) -> anyhow::Result<PathBuf> {
    locate(state, id).ok_or_else(|| anyhow!("Component '{id}' is not installed. Open Setup to download it."))
}

fn describe(state: &AppState, id: &str, name: &str, about: &str, size_mb: u32, required: bool) -> Component {
    let found = locate(state, id);
    let ours = found
        .as_ref()
        .map(|p| p.starts_with(&state.dir) || std::env::var_os("LINGOREEL_COMPONENTS").is_some())
        .unwrap_or(false);
    Component {
        id: id.into(),
        name: name.into(),
        about: about.into(),
        size_mb,
        required,
        installed: found.is_some(),
        note: match (&found, ours) {
            (Some(p), false) => format!("Using the copy already on this PC: {}", p.display()),
            _ => String::new(),
        },
    }
}

#[tauri::command]
pub fn components_status(state: State<AppState>) -> Res<Vec<Component>> {
    let target = {
        let db = state.db.lock().unwrap();
        crate::db::setting(&db, "targetLang").as_str().unwrap_or("fr").to_string()
    };
    let mut out: Vec<Component> = vec![];
    for s in specs().into_iter().filter(|s| s.required) {
        out.push(describe(&state, s.id, s.name, s.about, s.size_mb, true));
    }
    // The dictionary for the language being learned is required; others appear once installed.
    for (code, name, size) in LANGUAGES {
        let d = DictSpec { id: format!("dict-{code}"), name: format!("{name} dictionary"), size_mb: *size };
        let is_target = *code == target;
        if is_target || locate(&state, &d.id).is_some() {
            out.push(describe(
                &state,
                &d.id,
                &d.name,
                &format!("Offline {name} to English dictionary (Wiktionary)."),
                d.size_mb,
                is_target,
            ));
        }
    }
    for s in specs().into_iter().filter(|s| !s.required) {
        out.push(describe(&state, s.id, s.name, s.about, s.size_mb, false));
        if s.id == "piper" {
            for (code, name, _) in LANGUAGES {
                let id = format!("voice-{code}");
                if voice(&id).is_some() && (*code == target || locate(&state, &id).is_some()) {
                    let about = if *code == "es" { "Voice from Spain (castellano).".to_string() } else { format!("Offline {name} voice.") };
                    out.push(describe(&state, &id, &format!("{name} voice"), &about, 64, false));
                }
            }
            out.push(describe(
                &state,
                "voice-en",
                "English voice",
                "Reads the English words of mixed subtitles and the translation on review cards.",
                64,
                false,
            ));
        }
    }
    Ok(out)
}

/// A file from this repository's own data release.
async fn ours(state: &AppState, name: &str) -> anyhow::Result<Source> {
    released(state, "JustSad42/LingoReel", "tags/data-v1", |n| n == name).await
}

/// The file(s) to download for a component. mpv builds are published daily and old ones are
/// removed, so that one (like FFmpeg) is resolved through the GitHub API at install time.
async fn sources(state: &AppState, id: &str) -> anyhow::Result<Vec<Source>> {
    Ok(match id {
        "mpv" => vec![released(state, "shinchiro/mpv-winbuild-cmake", "latest", |n| n.starts_with("mpv-x86_64-2") && n.ends_with(".7z")).await?],
        "ffmpeg" => vec![released(state, "BtbN/FFmpeg-Builds", "tags/latest", |n| n == "ffmpeg-master-latest-win64-gpl.zip").await?],
        "whisper" => vec![match gpu() {
            Gpu::Nvidia => pinned(format!("{WHISPER_RELEASE}/whisper-cublas-12.4.0-bin-x64.zip"))?,
            // Upstream publishes no Vulkan build for Windows; ours is built by a workflow in this repository.
            Gpu::Vulkan => ours(state, "whisper-vulkan-bin-x64.zip").await?,
            Gpu::None => pinned(format!("{WHISPER_RELEASE}/whisper-bin-x64.zip"))?,
        }],
        "whisper-model" => vec![pinned(format!("{WHISPER_MODELS}/{}", whisper_model_name()))?],
        "llama" => vec![pinned(format!(
            "https://github.com/ggml-org/llama.cpp/releases/download/{LLAMA_TAG}/llama-{LLAMA_TAG}-bin-win-vulkan-x64.zip"
        ))?],
        "llm-model-large" => vec![pinned(format!("{LLM_LARGE_REPO}/{LLM_LARGE_FILE}"))?],
        "llm-model" => vec![pinned(format!("{LLM_REPO}/{LLM_MODEL_FILE}"))?],
        id if id.starts_with("dict-") => vec![ours(state, &format!("{id}.sqlite.gz")).await?],
        "piper" => vec![pinned(PIPER_URL.into())?],
        id if id.starts_with("voice-") => {
            let (_, folder, file) = voice(id).context("no voice for this language")?;
            vec![pinned(format!("{VOICE_BASE}/{folder}/{file}.onnx"))?, pinned(format!("{VOICE_BASE}/{folder}/{file}.onnx.json"))?]
        }
        _ => bail!("unknown component {id}"),
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

async fn download(app: &AppHandle, state: &AppState, id: &str, source: &Source, dest: &Path) -> anyhow::Result<()> {
    let resp = state.http.get(&source.url).header("User-Agent", "LingoReel").send().await?.error_for_status()?;
    let total = resp.content_length().unwrap_or(0);
    let part = dest.with_extension("part");
    let mut file = tokio::fs::File::create(&part).await?;
    let mut stream = resp.bytes_stream();
    let mut hash = Sha256::new();
    let (mut done, mut last_emit) = (0u64, 0u64);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        hash.update(&chunk);
        file.write_all(&chunk).await?;
        done += chunk.len() as u64;
        if done - last_emit > 2_000_000 {
            last_emit = done;
            let _ = app.emit("component-progress", json!({"id": id, "done": done, "total": total, "stage": "download"}));
        }
    }
    file.flush().await?;
    drop(file);
    let got = hex(&hash.finalize());
    if got != source.sha256 {
        let _ = tokio::fs::remove_file(&part).await;
        bail!("it doesn't match its published checksum, so it was discarded");
    }
    tokio::fs::rename(&part, dest).await?;
    Ok(())
}

#[tauri::command]
pub async fn component_install(app: AppHandle, state: State<'_, AppState>, id: String) -> Res<()> {
    let target = dir(&state, &id);
    tokio::fs::create_dir_all(&target).await?;
    for source in sources(&state, &id).await? {
        let file_name = source.url.rsplit('/').next().unwrap_or("download").to_string();
        let is_archive = file_name.ends_with(".zip") || file_name.ends_with(".7z");
        let dest = target.join(&file_name);
        download(&app, &state, &id, &source, &dest)
            .await
            .with_context(|| format!("downloading {file_name}"))?;
        let _ = app.emit("component-progress", json!({"id": id, "done": 1, "total": 1, "stage": "unpack"}));
        if is_archive {
            // Windows ships bsdtar, which reads both zip and 7z.
            let tar = Path::new(r"C:\Windows\System32\tar.exe");
            let out = cmd(if tar.exists() { tar.as_os_str() } else { "tar".as_ref() })
                .arg("-xf")
                .arg(&dest)
                .arg("-C")
                .arg(&target)
                .output()
                .await
                .context("running tar")?;
            if !out.status.success() {
                return Err(anyhow!("could not unpack {file_name}: {}", String::from_utf8_lossy(&out.stderr)).into());
            }
            let _ = tokio::fs::remove_file(&dest).await;
        } else if file_name.ends_with(".gz") {
            let out_path = target.join(file_name.trim_end_matches(".gz"));
            let (src, dst) = (dest.clone(), out_path.clone());
            tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                let mut gz = flate2::read::GzDecoder::new(std::fs::File::open(&src)?);
                let mut out = std::fs::File::create(&dst)?;
                std::io::copy(&mut gz, &mut out)?;
                Ok(())
            })
            .await??;
            let _ = tokio::fs::remove_file(&dest).await;
        }
    }
    if locate(&state, &id).is_none() {
        return Err(anyhow!("{id} was downloaded but its main file is missing").into());
    }
    let _ = app.emit("component-progress", json!({"id": id, "done": 1, "total": 1, "stage": "done"}));
    Ok(())
}

#[tauri::command]
pub async fn component_remove(state: State<'_, AppState>, id: String) -> Res<()> {
    state.dicts.lock().unwrap().clear();
    match locate(&state, &id) {
        Some(p) if p.starts_with(&state.dir) => {
            if matches!(id.as_str(), "whisper-model" | "llm-model" | "llm-model-large") || id.starts_with("dict-") {
                tokio::fs::remove_file(p).await?;
            } else if id.starts_with("voice-") {
                let _ = tokio::fs::remove_file(p.with_extension("onnx.json")).await;
                tokio::fs::remove_file(p).await?;
            } else {
                tokio::fs::remove_dir_all(dir(&state, &id)).await?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fixed_file_has_a_checksum() {
        for (name, sha) in PINNED {
            assert!(sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()), "{name}");
        }
        for (_, folder, file) in VOICES {
            assert!(pinned(format!("{VOICE_BASE}/{folder}/{file}.onnx")).is_ok());
            assert!(pinned(format!("{VOICE_BASE}/{folder}/{file}.onnx.json")).is_ok());
        }
        for file in [LLM_MODEL_FILE, LLM_LARGE_FILE, "ggml-small.bin", "ggml-large-v3-turbo.bin", "whisper-bin-x64.zip", "whisper-cublas-12.4.0-bin-x64.zip"] {
            assert!(pinned(file.to_string()).is_ok(), "{file}");
        }
        assert!(pinned(PIPER_URL.to_string()).is_ok());
        assert!(pinned(format!("llama-{LLAMA_TAG}-bin-win-vulkan-x64.zip")).is_ok());
        assert!(pinned("https://example.com/other.zip".into()).is_err());
    }

    #[test]
    fn release_files_without_a_digest_are_refused() {
        let rel = json!({"assets": [
            {"name": "a.zip", "browser_download_url": "u/a.zip", "digest": null},
            {"name": "b.zip", "browser_download_url": "u/b.zip", "digest": "sha256:AB"},
        ]});
        assert!(pick_asset(&rel, |n| n == "a.zip").is_none());
        assert_eq!(pick_asset(&rel, |n| n == "b.zip").unwrap().sha256, "ab");
    }
}
