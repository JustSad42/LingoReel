use crate::util::{now, Res};
use crate::AppState;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::path::Path;
use tauri::State;

pub fn open(dir: &Path) -> anyhow::Result<Connection> {
    let db = Connection::open(dir.join("lingoreel.sqlite"))?;
    db.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS settings(k TEXT PRIMARY KEY, v TEXT);
         CREATE TABLE IF NOT EXISTS words(lang TEXT, lemma TEXT, status INTEGER, updated INTEGER,
             PRIMARY KEY(lang, lemma));
         CREATE TABLE IF NOT EXISTS cards(id INTEGER PRIMARY KEY, lang TEXT, lemma TEXT, word TEXT,
             sentence TEXT, translation TEXT, definition TEXT, media_key TEXT, media_title TEXT,
             pos REAL, clip TEXT, image TEXT, created INTEGER, due INTEGER, stability REAL,
             difficulty REAL, reps INTEGER, lapses INTEGER, state INTEGER, last_review INTEGER);
         CREATE TABLE IF NOT EXISTS reviews(id INTEGER PRIMARY KEY, card_id INTEGER, ts INTEGER, rating INTEGER);
         CREATE TABLE IF NOT EXISTS media(key TEXT PRIMARY KEY, title TEXT, position REAL, duration REAL,
             updated INTEGER);",
    )?;
    Ok(db)
}

pub fn setting(db: &Connection, key: &str) -> Value {
    let raw: Option<String> = db
        .query_row("SELECT v FROM settings WHERE k='settings'", [], |r| r.get(0))
        .optional()
        .ok()
        .flatten();
    raw.and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get(key).cloned())
        .unwrap_or(Value::Null)
}

#[tauri::command]
pub fn settings_get(state: State<AppState>) -> Res<Value> {
    let db = state.db.lock().unwrap();
    let raw: Option<String> = db
        .query_row("SELECT v FROM settings WHERE k='settings'", [], |r| r.get(0))
        .optional()?;
    Ok(raw.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({})))
}

#[tauri::command]
pub fn settings_set(state: State<AppState>, value: Value) -> Res<()> {
    let db = state.db.lock().unwrap();
    db.execute(
        "INSERT INTO settings(k,v) VALUES('settings',?1) ON CONFLICT(k) DO UPDATE SET v=?1",
        [value.to_string()],
    )?;
    Ok(())
}

/// Every tracked word as [lemma, status]. Status: 1 learning, 2 known, 3 ignored.
#[tauri::command]
pub fn words_all(state: State<AppState>, lang: String) -> Res<Vec<(String, i64)>> {
    let db = state.db.lock().unwrap();
    let mut st = db.prepare("SELECT lemma, status FROM words WHERE lang=?1")?;
    let rows = st.query_map([lang], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[tauri::command]
pub fn words_set(state: State<AppState>, lang: String, lemmas: Vec<String>, status: i64) -> Res<()> {
    let mut db = state.db.lock().unwrap();
    let tx = db.transaction()?;
    for l in lemmas {
        if status == 0 {
            tx.execute("DELETE FROM words WHERE lang=?1 AND lemma=?2", params![lang, l])?;
        } else {
            tx.execute(
                "INSERT INTO words VALUES(?1,?2,?3,?4)
                 ON CONFLICT(lang,lemma) DO UPDATE SET status=?3, updated=?4",
                params![lang, l, status, now()],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

#[tauri::command]
pub fn progress_set(state: State<AppState>, key: String, title: String, position: f64, duration: f64) -> Res<()> {
    let db = state.db.lock().unwrap();
    db.execute(
        "INSERT INTO media VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(key) DO UPDATE SET title=?2, position=?3, duration=?4, updated=?5",
        params![key, title, position, duration, now()],
    )?;
    Ok(())
}

/// Saved positions for every media item: [key, position, duration].
#[tauri::command]
pub fn progress_all(state: State<AppState>) -> Res<Vec<(String, f64, f64)>> {
    let db = state.db.lock().unwrap();
    let mut st = db.prepare("SELECT key, position, duration FROM media")?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}
