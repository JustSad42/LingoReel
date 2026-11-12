//! mpv runs as a child process drawing into our window (behind the transparent web UI)
//! and is driven over its JSON IPC pipe.
use crate::components;
use crate::util::{cmd, Res};
use crate::AppState;
use anyhow::{anyhow, Context};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

pub struct Mpv {
    child: tokio::process::Child,
    tx: mpsc::UnboundedSender<String>,
}

impl Mpv {
    pub fn send(&self, command: Value) {
        let _ = self.tx.send(json!({ "command": command }).to_string());
    }
    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
    pub fn kill(&mut self) {
        let _ = self.child.start_kill();
    }
}

const OBSERVED: &[&str] = &[
    "time-pos", "pause", "duration", "speed", "eof-reached", "volume", "mute", "track-list", "aid",
    "paused-for-cache",
];

async fn spawn(app: &AppHandle, state: &AppState) -> anyhow::Result<Mpv> {
    let exe = components::require(state, "mpv")?;
    let window = app.get_webview_window("main").context("no main window")?;
    let hwnd = window.hwnd()?.0 as isize;
    let pipe = format!(r"\\.\pipe\lingoreel-mpv-{}", std::process::id());
    let child = cmd(&exe)
        .arg(format!("--wid={hwnd}"))
        .arg(format!("--input-ipc-server={pipe}"))
        .args([
            "--no-config",
            "--idle=yes",
            "--force-window=yes",
            "--keep-open=yes",
            "--no-osc",
            "--no-osd-bar",
            "--osd-level=0",
            "--no-input-default-bindings",
            "--input-vo-keyboard=no",
            "--input-cursor=no",
            "--cursor-autohide=no",
            "--sub-auto=no",
            "--sid=no",
            "--sub-visibility=no",
            "--hwdec=auto-safe",
            "--hr-seek=yes",
            "--cache=yes",
            "--demuxer-max-bytes=256MiB",
            "--background-color=#000000",
        ])
        .spawn()
        .context("starting mpv")?;

    // The pipe appears a moment after the process starts.
    let mut client = None;
    for _ in 0..100 {
        match tokio::net::windows::named_pipe::ClientOptions::new().open(&pipe) {
            Ok(c) => {
                client = Some(c);
                break;
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
        }
    }
    let client = client.ok_or_else(|| anyhow!("mpv did not open its control pipe"))?;
    let (reader, mut writer) = tokio::io::split(client);
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    tauri::async_runtime::spawn(async move {
        while let Some(line) = rx.recv().await {
            if writer.write_all(line.as_bytes()).await.is_err() || writer.write_all(b"\n").await.is_err() {
                break;
            }
        }
    });
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if v.get("event").is_some() {
                    let _ = app2.emit("mpv", v);
                }
            }
        }
        let _ = app2.emit("mpv", json!({"event": "closed"}));
    });

    let mpv = Mpv { child, tx };
    for (i, prop) in OBSERVED.iter().enumerate() {
        mpv.send(json!(["observe_property", i + 1, prop]));
    }
    Ok(mpv)
}

/// Start mpv if it is not running.
#[tauri::command]
pub async fn mpv_start(app: AppHandle, state: State<'_, AppState>) -> Res<()> {
    let mut guard = state.mpv.lock().await;
    if let Some(m) = guard.as_mut() {
        if m.alive() {
            return Ok(());
        }
    }
    *guard = Some(spawn(&app, &state).await?);
    Ok(())
}

/// Send any mpv command, e.g. ["loadfile", url] or ["set_property", "pause", true].
#[tauri::command]
pub async fn mpv_cmd(state: State<'_, AppState>, args: Value) -> Res<()> {
    let guard = state.mpv.lock().await;
    guard.as_ref().ok_or_else(|| anyhow!("player is not running"))?.send(args);
    Ok(())
}

#[tauri::command]
pub async fn mpv_stop(state: State<'_, AppState>) -> Res<()> {
    if let Some(mut m) = state.mpv.lock().await.take() {
        m.send(json!(["quit"]));
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        m.kill();
    }
    Ok(())
}
