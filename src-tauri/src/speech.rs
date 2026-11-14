//! Reading words and lines aloud with Piper, an offline text-to-speech engine. Useful above
//! all for films whose audio is in another language: the learner still hears the line.
use crate::components;
use crate::util::{cmd, Res};
use crate::AppState;
use anyhow::Context;
use std::process::Stdio;
use tauri::State;
use tokio::io::AsyncWriteExt;

/// Whether this language can be read aloud right now.
#[tauri::command]
pub fn tts_ready(state: State<AppState>, lang: String) -> bool {
    components::locate(&state, "piper").is_some() && components::locate(&state, &format!("voice-{lang}")).is_some()
}

fn hash(text: &str) -> u64 {
    // FNV-1a: stable across runs, so the cache survives restarts.
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// Speak `text` into a WAV file (cached) and return its path for the UI to play.
#[tauri::command]
pub async fn tts_speak(state: State<'_, AppState>, lang: String, text: String, slow: bool) -> Res<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return Err(anyhow::anyhow!("nothing to read").into());
    }
    let exe = components::require(&state, "piper")?;
    let voice = components::require(&state, &format!("voice-{lang}"))?;
    let dir = state.dir.join("tts");
    tokio::fs::create_dir_all(&dir).await?;
    let out = dir.join(format!("{:016x}.wav", hash(&format!("{lang}|{slow}|{text}"))));
    if out.exists() {
        return Ok(out.to_string_lossy().into_owned());
    }
    let part = out.with_extension("part.wav");
    let mut child = cmd(&exe)
        .current_dir(exe.parent().unwrap())
        .arg("--model")
        .arg(&voice)
        .arg("--output_file")
        .arg(&part)
        .args(["--length_scale", if slow { "1.35" } else { "1.0" }, "--sentence_silence", "0.15"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting the voice engine")?;
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(text.as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    drop(stdin);
    let res = child.wait_with_output().await?;
    if !res.status.success() || !part.exists() {
        let _ = tokio::fs::remove_file(&part).await;
        let err = String::from_utf8_lossy(&res.stderr);
        return Err(anyhow::anyhow!("the voice engine failed: {}", err.lines().last().unwrap_or("unknown error")).into());
    }
    tokio::fs::rename(&part, &out).await?;
    Ok(out.to_string_lossy().into_owned())
}
