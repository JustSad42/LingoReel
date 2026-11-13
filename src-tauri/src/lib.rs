mod align;
mod cards;
mod components;
mod db;
mod dict;
mod llm;
mod media;
mod mpv;
mod queue;
mod srs;
mod subs;
mod util;

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;

pub struct AppState {
    /// Everything we store lives here: database, downloaded components, prepared videos, clips.
    pub dir: PathBuf,
    pub db: Mutex<rusqlite::Connection>,
    pub dicts: Mutex<HashMap<String, dict::Dict>>,
    pub http: reqwest::Client,
    pub mpv: tokio::sync::Mutex<Option<mpv::Mpv>>,
    pub llm: tokio::sync::Mutex<Option<llm::Server>>,
    /// Serialises GPU-heavy work (Whisper).
    pub prepare_lock: tokio::sync::Mutex<()>,
    /// Limits concurrent scene-clip encodes.
    pub clip_slots: tokio::sync::Semaphore,
    pub queue: queue::Queue,
    /// Latest preparation progress: video key, stage, percent.
    pub progress: Mutex<Option<(String, String, f64)>>,
    /// Helper processes (FFmpeg, Whisper) to stop when the app closes.
    pub children: Mutex<std::collections::HashSet<u32>>,
}

#[tauri::command]
fn app_info(app: tauri::AppHandle, state: tauri::State<AppState>) -> Value {
    json!({
        "version": app.package_info().version.to_string(),
        "dataDir": state.dir.to_string_lossy(),
        "nvidia": components::has_nvidia(),
        "gpu": format!("{:?}", components::gpu()),
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_window_state::Builder::new().build())
        .setup(|app| {
            let dir = app.path().app_local_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let _ = std::fs::remove_dir_all(dir.join("tmp"));
            let db = db::open(&dir)?;
            app.manage(AppState {
                dir,
                db: Mutex::new(db),
                dicts: Mutex::new(HashMap::new()),
                http: reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(10)).build()?,
                mpv: tokio::sync::Mutex::new(None),
                llm: tokio::sync::Mutex::new(None),
                prepare_lock: tokio::sync::Mutex::new(()),
                clip_slots: tokio::sync::Semaphore::new(2),
                queue: Default::default(),
                progress: Mutex::new(None),
                children: Mutex::new(Default::default()),
            });
            queue::init(&app.state::<AppState>());
            tauri::async_runtime::spawn(queue::run(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            db::settings_get,
            db::settings_set,
            db::words_all,
            db::words_set,
            db::progress_set,
            db::progress_all,
            db::cache_get,
            db::cache_set,
            dict::dict_lookup,
            dict::dict_phrase,
            dict::dict_top_lemmas,
            components::components_status,
            components::component_install,
            components::component_remove,
            mpv::mpv_start,
            mpv::mpv_cmd,
            mpv::mpv_stop,
            media::prepare_media,
            media::prepared_get,
            media::prepared_delete,
            media::media_stats,
            media::media_summaries,
            media::media_mine,
            media::word_occurrences,
            media::scan_folder,
            media::probe_file,
            media::map_path,
            cards::card_add,
            cards::cards_due,
            cards::cards_list,
            cards::card_grade,
            cards::card_delete,
            cards::cards_counts,
            llm::llm_status,
            llm::llm_explain,
            llm::llm_stop,
            llm::shadow_score,
            llm::http_json,
            queue::queue_add,
            queue::queue_list,
            queue::queue_remove,
            queue::queue_retry,
            queue::queue_clear,
            queue::queue_pause,
        ])
        .build(tauri::generate_context!())
        .expect("error while building LingoReel");

    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event {
            // Do not leave the player or the model server running after the window closes.
            let state = handle.state::<AppState>();
            if let Ok(mut g) = state.mpv.try_lock() {
                if let Some(m) = g.as_mut() {
                    m.kill();
                }
            }
            queue::stop(&state);
            for pid in state.children.lock().unwrap().drain() {
                let mut c = std::process::Command::new("taskkill");
                c.args(["/PID", &pid.to_string(), "/T", "/F"]);
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    c.creation_flags(0x0800_0000);
                }
                let _ = c.status();
            }
            if let Ok(mut g) = state.llm.try_lock() {
                if let Some(s) = g.as_mut() {
                    s.kill();
                }
            };
        }
    });
}
