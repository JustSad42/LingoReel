//! Saved words and sentences, their scene clips, and the review queue.
use crate::components;
use crate::srs::{self, Sched};
use crate::util::{cmd, now, Res};
use crate::AppState;
use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Deserialize)]
pub struct NewCard {
    pub lang: String,
    pub lemma: String,
    pub word: String,
    pub sentence: String,
    pub translation: String,
    pub definition: String,
    pub media_key: String,
    pub media_title: String,
    /// Line start/end in the video, for cutting the scene clip.
    pub start: f64,
    pub end: f64,
    pub url: String,
    pub audio_index: Option<i64>,
}

#[derive(Serialize)]
pub struct Card {
    pub id: i64,
    pub lang: String,
    pub lemma: String,
    pub word: String,
    pub sentence: String,
    pub translation: String,
    pub definition: String,
    pub media_key: String,
    pub media_title: String,
    pub pos: f64,
    pub clip: String,
    pub image: String,
    pub created: i64,
    pub due: i64,
    pub state: i64,
    pub reps: i64,
    pub lapses: i64,
    pub stability: f64,
    /// Seconds until next review for each answer button.
    pub preview: [i64; 4],
}

const COLS: &str = "id,lang,lemma,word,sentence,translation,definition,media_key,media_title,pos,clip,image,created,due,stability,difficulty,reps,lapses,state,last_review";

fn sched_of(r: &Row) -> rusqlite::Result<Sched> {
    Ok(Sched {
        due: r.get(13)?,
        stability: r.get(14)?,
        difficulty: r.get(15)?,
        reps: r.get(16)?,
        lapses: r.get(17)?,
        state: r.get(18)?,
        last_review: r.get(19)?,
    })
}

fn card_of(r: &Row) -> rusqlite::Result<Card> {
    let s = sched_of(r)?;
    Ok(Card {
        id: r.get(0)?,
        lang: r.get(1)?,
        lemma: r.get(2)?,
        word: r.get(3)?,
        sentence: r.get(4)?,
        translation: r.get(5)?,
        definition: r.get(6)?,
        media_key: r.get(7)?,
        media_title: r.get(8)?,
        pos: r.get(9)?,
        clip: r.get(10)?,
        image: r.get(11)?,
        created: r.get(12)?,
        due: s.due,
        state: s.state,
        reps: s.reps,
        lapses: s.lapses,
        stability: s.stability,
        preview: srs::preview(s, now()),
    })
}

/// Cut a short low-resolution clip and a still of the line, for replay during review.
async fn make_clip(app: AppHandle, id: i64, c: NewCard) {
    let state = app.state::<AppState>();
    let Ok(ffmpeg) = components::require(&state, "ffmpeg") else { return };
    let dir = state.dir.join("clips");
    let _ = tokio::fs::create_dir_all(&dir).await;
    let (clip, image) = (dir.join(format!("{id}.mp4")), dir.join(format!("{id}.jpg")));
    let from = (c.start - 0.4).max(0.0);
    let len = (c.end - c.start + 0.9).clamp(1.0, 20.0);
    let audio_map = c.audio_index.map(|i| format!("0:{i}")).unwrap_or_else(|| "0:a:0".into());
    let res = cmd(&ffmpeg)
        .args(["-y", "-v", "error", "-nostdin", "-ss", &format!("{from:.3}"), "-i"])
        .arg(&c.url)
        .args(["-t", &format!("{len:.3}"), "-map", "0:v:0", "-map", &audio_map, "-sn"])
        .args(["-vf", "scale=-2:360", "-pix_fmt", "yuv420p", "-c:v", "libx264", "-preset", "veryfast", "-crf", "28"])
        .args(["-c:a", "aac", "-ac", "2", "-b:a", "96k", "-movflags", "+faststart"])
        .arg(&clip)
        .args(["-ss", &format!("{:.3}", len / 2.0), "-map", "0:v:0", "-frames:v", "1", "-vf", "scale=-2:360", "-q:v", "4"])
        .arg(&image)
        .output()
        .await;
    let ok = matches!(&res, Ok(o) if o.status.success()) && clip.exists();
    if ok {
        let db = state.db.lock().unwrap();
        let _ = db.execute(
            "UPDATE cards SET clip=?1, image=?2 WHERE id=?3",
            params![
                clip.to_string_lossy(),
                if image.exists() { image.to_string_lossy().to_string() } else { String::new() },
                id
            ],
        );
    }
    let _ = app.emit("card-clip", json!({"id": id, "ok": ok}));
}

#[tauri::command]
pub fn card_add(app: AppHandle, state: State<AppState>, card: NewCard) -> Res<i64> {
    let id = {
        let db = state.db.lock().unwrap();
        db.execute(
            "INSERT INTO cards(lang,lemma,word,sentence,translation,definition,media_key,media_title,pos,
                 clip,image,created,due,stability,difficulty,reps,lapses,state,last_review)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'','',?10,?10,0,0,0,0,0,0)",
            params![
                card.lang, card.lemma, card.word, card.sentence, card.translation, card.definition,
                card.media_key, card.media_title, card.start, now()
            ],
        )?;
        // Saving a word means you are now learning it.
        if !card.lemma.is_empty() {
            db.execute(
                "INSERT INTO words VALUES(?1,?2,1,?3) ON CONFLICT(lang,lemma) DO UPDATE SET status=1, updated=?3
                 WHERE status NOT IN (2)",
                params![card.lang, card.lemma, now()],
            )?;
        }
        db.last_insert_rowid()
    };
    if !card.url.is_empty() {
        tauri::async_runtime::spawn(make_clip(app, id, card));
    }
    Ok(id)
}

/// Cards to study now: due reviews first, then new cards.
#[tauri::command]
pub fn cards_due(state: State<AppState>, lang: String, limit: i64) -> Res<Vec<Card>> {
    let db = state.db.lock().unwrap();
    let mut st = db.prepare(&format!(
        "SELECT {COLS} FROM cards WHERE lang=?1 AND due<=?2 ORDER BY (state=0), due LIMIT ?3"
    ))?;
    let rows = st.query_map(params![lang, now(), limit], card_of)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[tauri::command]
pub fn cards_list(state: State<AppState>, lang: String) -> Res<Vec<Card>> {
    let db = state.db.lock().unwrap();
    let mut st = db.prepare(&format!("SELECT {COLS} FROM cards WHERE lang=?1 ORDER BY created DESC"))?;
    let rows = st.query_map([lang], card_of)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[tauri::command]
pub fn card_grade(state: State<AppState>, id: i64, rating: i64) -> Res<()> {
    let db = state.db.lock().unwrap();
    let (sched, lang, lemma): (Sched, String, String) = db.query_row(
        &format!("SELECT {COLS} FROM cards WHERE id=?1"),
        [id],
        |r| Ok((sched_of(r)?, r.get(1)?, r.get(2)?)),
    )?;
    let t = now();
    let next = srs::grade(sched, rating.clamp(1, 4), t);
    db.execute(
        "UPDATE cards SET due=?1, stability=?2, difficulty=?3, reps=?4, lapses=?5, state=?6, last_review=?7 WHERE id=?8",
        params![next.due, next.stability, next.difficulty, next.reps, next.lapses, next.state, next.last_review, id],
    )?;
    db.execute("INSERT INTO reviews(card_id, ts, rating) VALUES(?1,?2,?3)", params![id, t, rating])?;
    // A word that has survived a month-long interval counts as known.
    if next.state == srs::REVIEW && next.stability >= 30.0 && !lemma.is_empty() {
        db.execute(
            "INSERT INTO words VALUES(?1,?2,2,?3) ON CONFLICT(lang,lemma) DO UPDATE SET status=2, updated=?3",
            params![lang, lemma, t],
        )?;
    }
    Ok(())
}

#[tauri::command]
pub fn card_delete(state: State<AppState>, id: i64) -> Res<()> {
    let db = state.db.lock().unwrap();
    let files: Option<(String, String)> = db
        .query_row("SELECT clip, image FROM cards WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
        .ok();
    db.execute("DELETE FROM cards WHERE id=?1", [id])?;
    if let Some((clip, image)) = files {
        for f in [clip, image] {
            if !f.is_empty() {
                let _ = std::fs::remove_file(f);
            }
        }
    }
    Ok(())
}

#[derive(Serialize)]
pub struct ReviewCounts {
    pub due: i64,
    pub new: i64,
    pub total: i64,
    pub reviewed_today: i64,
}

#[tauri::command]
pub fn cards_counts(state: State<AppState>, lang: String) -> Res<ReviewCounts> {
    let db = state.db.lock().unwrap();
    let t = now();
    let one = |sql: &str, p: &[&dyn rusqlite::ToSql]| -> i64 { db.query_row(sql, p, |r| r.get(0)).unwrap_or(0) };
    Ok(ReviewCounts {
        due: one("SELECT COUNT(*) FROM cards WHERE lang=?1 AND state>0 AND due<=?2", &[&lang, &t]),
        new: one("SELECT COUNT(*) FROM cards WHERE lang=?1 AND state=0", &[&lang]),
        total: one("SELECT COUNT(*) FROM cards WHERE lang=?1", &[&lang]),
        reviewed_today: one("SELECT COUNT(*) FROM reviews WHERE ts>?1", &[&(t - 86_400)]),
    })
}
