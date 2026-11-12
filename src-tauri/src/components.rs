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

fn specs() -> Vec<Spec> {
    let gpu = has_nvidia();
    vec![
        Spec { id: "mpv", name: "Video player engine (mpv)", about: "Plays every video format, including the ones on your server.", size_mb: 35, required: true },
        Spec { id: "ffmpeg", name: "Media tools (FFmpeg)", about: "Reads subtitle and audio tracks out of your video files.", size_mb: 200, required: true },
        Spec { id: "dict-fr", name: "French dictionary", about: "Offline French to English dictionary with word frequencies (Wiktionary).", size_mb: 22, required: true },
        Spec {
            id: "whisper",
            name: "Speech recognition engine (Whisper)",
            about: if gpu { "Listens to the audio to time every word. NVIDIA GPU build." } else { "Listens to the audio to time every word. CPU build (slow without an NVIDIA GPU)." },
            size_mb: if gpu { 670 } else { 8 },
            required: false,
        },
        Spec {
            id: "whisper-model",
            name: "Speech recognition model",
            about: if gpu { "large-v3-turbo: accurate, about 1 to 2 minutes per episode on a recent NVIDIA GPU." } else { "small: lighter model suited to CPU-only machines." },
            size_mb: if gpu { 1625 } else { 490 },
            required: false,
        },
        Spec { id: "llama", name: "Local LLM engine (llama.cpp)", about: "Runs the explanation model on your own machine. Nothing leaves your PC.", size_mb: 40, required: false },
        Spec { id: "llm-model", name: "Explanation model (Gemma 3 4B)", about: "Explains phrases, idioms and grammar in context, fully offline.", size_mb: 2490, required: false },
    ]
}

pub fn dir(state: &AppState, id: &str) -> PathBuf {
    let base = state.dir.join("components");
    match id {
        "whisper-model" | "llm-model" => base.join("models"),
        id if id.starts_with("dict-") => base.join("dict"),
        other => base.join(other),
    }
}

pub fn whisper_model_name() -> &'static str {
    if has_nvidia() { "ggml-large-v3-turbo.bin" } else { "ggml-small.bin" }
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
        _ => None,
    }
}

pub fn require(state: &AppState, id: &str) -> anyhow::Result<PathBuf> {
    locate(state, id).ok_or_else(|| anyhow!("Component '{id}' is not installed. Open Setup to download it."))
}

#[tauri::command]
pub fn components_status(state: State<AppState>) -> Res<Vec<Component>> {
    Ok(specs()
        .into_iter()
        .map(|s| {
            let found = locate(&state, s.id);
            let ours = found.as_ref().map(|p| p.starts_with(&state.dir)).unwrap_or(false);
            Component {
                id: s.id.into(),
                name: s.name.into(),
                about: s.about.into(),
                size_mb: s.size_mb,
                required: s.required,
                installed: found.is_some(),
                note: match (&found, ours) {
                    (Some(p), false) => format!("Using the copy already on this PC: {}", p.display()),
                    _ => String::new(),
                },
            }
        })
        .collect())
}

/// Download URL(s) for a component. mpv builds are published daily and old ones are
/// removed, so that one is resolved through the GitHub API at install time.
async fn urls(state: &AppState, id: &str) -> anyhow::Result<Vec<String>> {
    let gpu = has_nvidia();
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
        "whisper" => vec![format!(
            "{WHISPER_RELEASE}/{}",
            if gpu { "whisper-cublas-12.4.0-bin-x64.zip" } else { "whisper-bin-x64.zip" }
        )],
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
            } else {
                tokio::fs::remove_dir_all(dir(&state, &id)).await?;
            }
        }
        _ => {}
    }
    Ok(())
}
