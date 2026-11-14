//! Things the app downloads once on first run instead of shipping in the installer:
//! the video engine, speech recognition, dictionary and (optionally) a local LLM.
use crate::util::{cmd, find_file, on_path, Res};
use crate::AppState;
use anyhow::{anyhow, bail, Context};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::json;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, State};
use tokio::io::AsyncWriteExt;

const DATA_RELEASE: &str = "https://github.com/JustSad42/LingoReel/releases/download/data-v1";
const WHISPER_RELEASE: &str = "https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.2";
const LLAMA_TAG: &str = "b12304";

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
];
const PIPER_URL: &str = "https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_windows_amd64.zip";
const VOICE_BASE: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/main";

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
    let gpu = card != Gpu::None;
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
            about: if gpu { "large-v3-turbo: accurate, a few minutes per episode on a recent graphics card." } else { "small: lighter model suited to PCs without a graphics card." },
            size_mb: if gpu { 1625 } else { 490 },
            required: false,
        },
        Spec { id: "piper", name: "Voice engine (Piper)", about: "Reads words and lines aloud, offline. Most useful for films in English.", size_mb: 22, required: false },
        Spec { id: "llama", name: "Local LLM engine (llama.cpp)", about: "Runs the explanation model on your own machine. Nothing leaves your PC.", size_mb: 40, required: false },
        Spec { id: "llm-model", name: "Explanation model (Gemma 3 4B)", about: "Explains phrases, idioms and grammar in context, fully offline.", size_mb: 2490, required: false },
    ]
}

pub fn dir(state: &AppState, id: &str) -> PathBuf {
    // LINGOREEL_COMPONENTS lets a development build share an existing download folder.
    let base = std::env::var_os("LINGOREEL_COMPONENTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| state.dir.join("components"));
    match id {
        "whisper-model" | "llm-model" => base.join("models"),
        id if id.starts_with("dict-") => base.join("dict"),
        id if id.starts_with("voice-") => base.join("voices"),
        other => base.join(other),
    }
}

pub fn whisper_model_name() -> &'static str {
    if gpu() != Gpu::None { "ggml-large-v3-turbo.bin" } else { "ggml-small.bin" }
}

/// Alignment-head preset matching the installed Whisper model.
pub fn whisper_dtw_preset() -> &'static str {
    if gpu() != Gpu::None { "large.v3.turbo" } else { "small" }
}

const LLM_MODEL_FILE: &str = "google_gemma-3-4b-it-Q4_K_M.gguf";

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
        }
    }
    Ok(out)
}

/// Download URL(s) for a component. mpv builds are published daily and old ones are
/// removed, so that one is resolved through the GitHub API at install time.
async fn urls(state: &AppState, id: &str) -> anyhow::Result<Vec<String>> {
    Ok(match id {
        "mpv" => {
            let rel: serde_json::Value = state
                .http
                .get("https://api.github.com/repos/shinchiro/mpv-winbuild-cmake/releases/latest")
                .header("User-Agent", "LingoReel")
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let url = rel["assets"]
                .as_array()
                .and_then(|a| {
                    a.iter().find_map(|x| {
                        let n = x["name"].as_str()?;
                        let plain = n.starts_with("mpv-x86_64-2") && n.ends_with(".7z");
                        plain.then(|| x["browser_download_url"].as_str().map(String::from)).flatten()
                    })
                })
                .context("no mpv build found in the latest release")?;
            vec![url]
        }
        "ffmpeg" => vec!["https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip".into()],
        "whisper" => vec![match gpu() {
            Gpu::Nvidia => format!("{WHISPER_RELEASE}/whisper-cublas-12.4.0-bin-x64.zip"),
            // Upstream publishes no Vulkan build for Windows; ours is built by a workflow in this repository.
            Gpu::Vulkan => format!("{DATA_RELEASE}/whisper-vulkan-bin-x64.zip"),
            Gpu::None => format!("{WHISPER_RELEASE}/whisper-bin-x64.zip"),
        }],
        "whisper-model" => vec![format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
            whisper_model_name()
        )],
        "llama" => vec![format!(
            "https://github.com/ggml-org/llama.cpp/releases/download/{LLAMA_TAG}/llama-{LLAMA_TAG}-bin-win-vulkan-x64.zip"
        )],
        "llm-model" => vec![format!(
            "https://huggingface.co/bartowski/google_gemma-3-4b-it-GGUF/resolve/main/{LLM_MODEL_FILE}"
        )],
        id if id.starts_with("dict-") => vec![format!("{DATA_RELEASE}/{id}.sqlite.gz")],
        "piper" => vec![PIPER_URL.into()],
        id if id.starts_with("voice-") => {
            let (_, folder, file) = voice(id).context("no voice for this language")?;
            vec![format!("{VOICE_BASE}/{folder}/{file}.onnx"), format!("{VOICE_BASE}/{folder}/{file}.onnx.json")]
        }
        _ => bail!("unknown component {id}"),
    })
}

async fn download(app: &AppHandle, state: &AppState, id: &str, url: &str, dest: &Path) -> anyhow::Result<()> {
    let resp = state.http.get(url).header("User-Agent", "LingoReel").send().await?.error_for_status()?;
    let total = resp.content_length().unwrap_or(0);
    let part = dest.with_extension("part");
    let mut file = tokio::fs::File::create(&part).await?;
    let mut stream = resp.bytes_stream();
    let (mut done, mut last_emit) = (0u64, 0u64);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        done += chunk.len() as u64;
        if done - last_emit > 2_000_000 {
            last_emit = done;
            let _ = app.emit("component-progress", json!({"id": id, "done": done, "total": total, "stage": "download"}));
        }
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&part, dest).await?;
    Ok(())
}

#[tauri::command]
pub async fn component_install(app: AppHandle, state: State<'_, AppState>, id: String) -> Res<()> {
    let target = dir(&state, &id);
    tokio::fs::create_dir_all(&target).await?;
    for url in urls(&state, &id).await? {
        let file_name = url.rsplit('/').next().unwrap_or("download").to_string();
        let is_archive = file_name.ends_with(".zip") || file_name.ends_with(".7z");
        let dest = target.join(&file_name);
        download(&app, &state, &id, &url, &dest)
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
            if matches!(id.as_str(), "whisper-model" | "llm-model") || id.starts_with("dict-") {
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
