//! Background preparation queue. Videos wait their turn and are prepared one after another
//! while the app is open, whether or not anything is playing. The list is kept in the
//! database, so it carries on after a restart.
use crate::media::{self, MediaReq, PrepareOpts};
use crate::util::Res;
use crate::AppState;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Default)]
pub struct Queue {
    wake: tokio::sync::Notify,
    /// The video being prepared and the switch that stops it.
    current: Mutex<Option<(String, Option<tokio::sync::oneshot::Sender<()>>)>>,
    paused: AtomicBool,
}

#[derive(Deserialize)]
pub struct NewItem {
    pub req: MediaReq,
    pub opts: PrepareOpts,
}

#[derive(Serialize)]
pub struct Item {
    pub key: String,
    pub title: String,
    /// "working", "waiting" or "failed".
    pub state: String,
    pub error: String,
    pub stage: String,
    pub pct: f64,
}

#[derive(Serialize)]
pub struct List {
    pub items: Vec<Item>,
    pub paused: bool,
}

fn changed(app: &AppHandle) {
    let _ = app.emit("queue-changed", ());
}

fn current_key(state: &AppState) -> Option<String> {
    state.queue.current.lock().unwrap().as_ref().map(|(k, _)| k.clone())
}

/// Stop the video being prepared; it goes back to waiting where it was in the line.
fn interrupt(state: &AppState) {
    if let Some((_, stop)) = state.queue.current.lock().unwrap().as_mut() {
        if let Some(tx) = stop.take() {
            let _ = tx.send(());
        }
    }
}

fn already_prepared(state: &AppState, key: &str, lang: &str) -> bool {
    media::load_prepared(state, key).map(|p| p.lang == lang).unwrap_or(false)
}

pub fn init(state: &AppState) {
    let db = state.db.lock().unwrap();
    let _ = db.execute("UPDATE queue SET state='waiting' WHERE state='working'", []);
    let paused: Option<String> = db.query_row("SELECT v FROM cache WHERE k='queue-paused'", [], |r| r.get(0)).ok();
    state.queue.paused.store(paused.as_deref() == Some("1"), Ordering::SeqCst);
}

/// Add videos to the queue. `front` puts them next in line; `now` also pauses whatever is
/// being prepared so they start straight away (the player uses this), even when the queue is paused.
#[tauri::command]
pub fn queue_add(app: AppHandle, state: State<AppState>, items: Vec<NewItem>, front: bool, now: bool) -> Res<usize> {
    let current = current_key(&state);
    let fresh: Vec<&NewItem> = items
        .iter()
        .filter(|it| current.as_deref() != Some(it.req.key.as_str()))
        .filter(|it| !already_prepared(&state, &it.req.key, &it.opts.target_lang))
        .collect();
    {
        let db = state.db.lock().unwrap();
        let edge: f64 = db.query_row(
            if front { "SELECT COALESCE(MIN(pos), 0) FROM queue" } else { "SELECT COALESCE(MAX(pos), 0) FROM queue" },
            [],
            |r| r.get(0),
        )?;
        let n = fresh.len() as f64;
        for (i, it) in fresh.iter().enumerate() {
            let pos = if front { edge - n + i as f64 } else { edge + 1.0 + i as f64 };
            db.execute(
                "INSERT INTO queue(key, title, req, opts, state, error, pos, urgent) VALUES(?1, ?2, ?3, ?4, 'waiting', '', ?5, ?6)
                 ON CONFLICT(key) DO UPDATE SET title=excluded.title, req=excluded.req, opts=excluded.opts, state='waiting',
                 error='', urgent=MAX(queue.urgent, excluded.urgent), pos=CASE WHEN ?7 THEN excluded.pos ELSE queue.pos END",
                params![
                    it.req.key,
                    it.req.title,
                    serde_json::to_string(&it.req)?,
                    serde_json::to_string(&it.opts)?,
                    pos,
                    now as i64,
                    front
                ],
            )?;
        }
    }
    if now && !fresh.is_empty() {
        interrupt(&state);
    }
    state.queue.wake.notify_one();
    changed(&app);
    Ok(fresh.len())
}

#[tauri::command]
pub fn queue_list(state: State<AppState>) -> Res<List> {
    let current = current_key(&state);
    let progress = state.progress.lock().unwrap().clone();
    let db = state.db.lock().unwrap();
    let mut stmt = db.prepare(
        "SELECT key, title, state, error FROM queue
         ORDER BY CASE state WHEN 'working' THEN 0 WHEN 'waiting' THEN 1 ELSE 2 END, urgent DESC, pos",
    )?;
    let items = stmt
        .query_map([], |r| {
            Ok(Item { key: r.get(0)?, title: r.get(1)?, state: r.get(2)?, error: r.get(3)?, stage: String::new(), pct: 0.0 })
        })?
        .filter_map(Result::ok)
        .map(|mut it| {
            if current.as_deref() == Some(it.key.as_str()) {
                if let Some((k, stage, pct)) = &progress {
                    if *k == it.key {
                        it.stage = stage.clone();
                        it.pct = *pct;
                    }
                }
            }
            it
        })
        .collect();
    Ok(List { items, paused: state.queue.paused.load(Ordering::SeqCst) })
}

#[tauri::command]
pub fn queue_remove(app: AppHandle, state: State<AppState>, key: String) -> Res<()> {
    state.db.lock().unwrap().execute("DELETE FROM queue WHERE key=?1", [&key])?;
    if current_key(&state).as_deref() == Some(key.as_str()) {
        interrupt(&state);
    }
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn queue_retry(app: AppHandle, state: State<AppState>, key: String) -> Res<()> {
    state.db.lock().unwrap().execute("UPDATE queue SET state='waiting', error='' WHERE key=?1", [&key])?;
    state.queue.wake.notify_one();
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn queue_clear(app: AppHandle, state: State<AppState>, failed_only: bool) -> Res<()> {
    {
        let db = state.db.lock().unwrap();
        if failed_only {
            db.execute("DELETE FROM queue WHERE state='failed'", [])?;
        } else {
            db.execute("DELETE FROM queue", [])?;
        }
    }
    if !failed_only {
        interrupt(&state);
    }
    changed(&app);
    Ok(())
}

/// Pausing stops the video in progress too (it resumes from the start later), so the
/// graphics card is free straight away, for a game say.
#[tauri::command]
pub fn queue_pause(app: AppHandle, state: State<AppState>, paused: bool) -> Res<()> {
    state.queue.paused.store(paused, Ordering::SeqCst);
    state.db.lock().unwrap().execute(
        "INSERT INTO cache(k, v, ts) VALUES('queue-paused', ?1, 0) ON CONFLICT(k) DO UPDATE SET v=excluded.v",
        [if paused { "1" } else { "0" }],
    )?;
    if paused {
        // Leave a video the player is waiting for alone.
        let urgent: bool = current_key(&state)
            .and_then(|k| {
                state.db.lock().unwrap().query_row("SELECT urgent FROM queue WHERE key=?1", [&k], |r| r.get::<_, i64>(0)).ok()
            })
            .map(|u| u == 1)
            .unwrap_or(false);
        if !urgent {
            interrupt(&state);
            unload_model(&state);
        }
    }
    state.queue.wake.notify_one();
    changed(&app);
    Ok(())
}

/// Free the graphics memory held by the local model; it starts again when next needed.
fn unload_model(state: &AppState) {
    if let Ok(mut g) = state.llm.try_lock() {
        if let Some(s) = g.as_mut() {
            s.kill();
        }
        *g = None;
    }
}

/// Stop whatever is being prepared, for when the app closes.
pub fn stop(state: &AppState) {
    interrupt(state);
}

fn next_item(state: &AppState) -> Option<(String, MediaReq, PrepareOpts)> {
    let paused = state.queue.paused.load(Ordering::SeqCst);
    let db = state.db.lock().unwrap();
    let row: Option<(String, String, String)> = db
        .query_row(
            "SELECT key, req, opts FROM queue WHERE state='waiting' AND (?1 = 0 OR urgent = 1) ORDER BY urgent DESC, pos LIMIT 1",
            [paused as i64],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();
    let (key, req, opts) = row?;
    match (serde_json::from_str(&req), serde_json::from_str(&opts)) {
        (Ok(req), Ok(opts)) => Some((key, req, opts)),
        _ => {
            let _ = db.execute("UPDATE queue SET state='failed', error='Could not read this queue entry.' WHERE key=?1", [&key]);
            None
        }
    }
}

fn is_waiting(state: &AppState, key: &str) -> bool {
    state
        .db
        .lock()
        .unwrap()
        .query_row("SELECT state FROM queue WHERE key=?1", [key], |r| r.get::<_, String>(0))
        .map(|s| s == "waiting")
        .unwrap_or(false)
}

/// Runs for the life of the app.
pub async fn run(app: AppHandle) {
    let state = app.state::<AppState>();
    let mut translated = false;
    loop {
        let Some((key, req, opts)) = next_item(&state) else {
            // Done for now: a model loaded only for translating should not sit on the graphics card.
            if translated {
                unload_model(&state);
                translated = false;
            }
            state.queue.wake.notified().await;
            continue;
        };
        if already_prepared(&state, &key, &opts.target_lang) {
            let _ = state.db.lock().unwrap().execute("DELETE FROM queue WHERE key=?1", [&key]);
            let _ = app.emit("queue-done", json!({"key": key, "ok": true, "error": ""}));
            changed(&app);
            continue;
        }
        let busy = state.prepare_lock.lock().await;
        if !is_waiting(&state, &key) {
            continue;
        }
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        *state.queue.current.lock().unwrap() = Some((key.clone(), Some(tx)));
        let _ = state.db.lock().unwrap().execute("UPDATE queue SET state='working' WHERE key=?1", [&key]);
        changed(&app);

        let outcome = tokio::select! {
            r = media::prepare(&app, &state, &req, &opts) => Some(r),
            _ = rx => None,
        };
        drop(busy);
        *state.queue.current.lock().unwrap() = None;
        {
            let db = state.db.lock().unwrap();
            match &outcome {
                Some(Ok(p)) => {
                    translated |= p.translated;
                    let _ = db.execute("DELETE FROM queue WHERE key=?1", [&key]);
                }
                Some(Err(e)) => {
                    let _ = db.execute("UPDATE queue SET state='failed', error=?2, urgent=0 WHERE key=?1", params![key, format!("{e:#}")]);
                }
                None => {
                    let _ = db.execute("UPDATE queue SET state='waiting' WHERE key=?1", [&key]);
                }
            }
        }
        if let Some(r) = &outcome {
            let error = r.as_ref().err().map(|e| format!("{e:#}")).unwrap_or_default();
            let _ = app.emit("queue-done", json!({"key": key, "ok": r.is_ok(), "error": error}));
        }
        changed(&app);
    }
}
