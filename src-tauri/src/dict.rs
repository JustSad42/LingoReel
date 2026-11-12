//! Offline dictionary: one SQLite file per language, built by tools/build_dict.py.
use crate::util::Res;
use crate::AppState;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use tauri::State;
use unicode_normalization::UnicodeNormalization;

pub fn norm(s: &str) -> String {
    s.nfc()
        .collect::<String>()
        .replace(['\u{2019}', '\u{02bc}'], "'")
        .to_lowercase()
        .trim()
        .to_string()
}

#[derive(Serialize, Clone)]
pub struct Entry {
    pub word: String,
    pub pos: String,
    pub gender: String,
    pub ipa: String,
    pub glosses: Vec<String>,
    /// How the looked-up form relates to this entry ("first-person plural imperfect"), if inflected.
    pub form_tags: Vec<String>,
}

#[derive(Serialize, Clone, Default)]
pub struct Lookup {
    pub word: String,
    pub lemma: Option<String>,
    pub rank: Option<i64>,
    pub entries: Vec<Entry>,
}

pub struct Dict {
    db: Connection,
    lang: String,
    lemma_cache: HashMap<String, Option<String>>,
}

const FR_ELISIONS: &[(&str, &str)] = &[
    ("l'", "le"), ("d'", "de"), ("j'", "je"), ("n'", "ne"), ("m'", "me"), ("t'", "te"),
    ("s'", "se"), ("c'", "ce"), ("qu'", "que"), ("jusqu'", "jusque"), ("lorsqu'", "lorsque"),
    ("puisqu'", "puisque"), ("quoiqu'", "quoique"),
];

impl Dict {
    pub fn open(path: &Path, lang: &str) -> anyhow::Result<Dict> {
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        Ok(Dict { db, lang: lang.to_string(), lemma_cache: HashMap::new() })
    }

    pub fn rank(&self, norm: &str) -> Option<i64> {
        self.db.query_row("SELECT rank FROM freq WHERE norm=?1", [norm], |r| r.get(0)).ok()
    }

    pub fn has_entry(&self, norm: &str) -> bool {
        self.db
            .query_row("SELECT 1 FROM entry WHERE norm=?1 LIMIT 1", [norm], |_| Ok(()))
            .is_ok()
    }

    pub fn exists(&self, norm: &str) -> bool {
        self.has_entry(norm)
            || self.db.query_row("SELECT 1 FROM form WHERE norm=?1 LIMIT 1", [norm], |_| Ok(())).is_ok()
    }

    fn forms(&self, norm: &str) -> Vec<(String, String)> {
        let mut st = match self.db.prepare_cached("SELECT lemma, tags FROM form WHERE norm=?1") {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        st.query_map([norm], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    fn elided(&self, norm: &str) -> Option<&'static str> {
        if self.lang != "fr" {
            return None;
        }
        FR_ELISIONS.iter().find(|(e, _)| *e == norm).map(|(_, full)| *full)
    }

    /// Dictionary headword for a surface form. Picks the most frequent candidate, so
    /// "suis" resolves to "être" rather than "suivre".
    pub fn lemma(&mut self, surface: &str) -> Option<String> {
        let n = norm(surface);
        if let Some(hit) = self.lemma_cache.get(&n) {
            return hit.clone();
        }
        let result = self.lemma_uncached(&n);
        self.lemma_cache.insert(n, result.clone());
        result
    }

    fn lemma_uncached(&self, n: &str) -> Option<String> {
        if let Some(full) = self.elided(n) {
            return Some(full.to_string());
        }
        let mut best: Option<(i64, String)> = None;
        if self.has_entry(n) {
            best = Some((self.rank(n).unwrap_or(900_000), n.to_string()));
        }
        for (lemma, _) in self.forms(n) {
            let ln = norm(&lemma);
            if ln.contains(' ') {
                continue;
            }
            let r = self.rank(&ln).unwrap_or(999_999);
            if best.as_ref().map(|(br, _)| r < *br).unwrap_or(true) {
                best = Some((r, ln));
            }
        }
        best.map(|(_, l)| l)
    }

    fn entries_for(&self, norm: &str, form_tags: Vec<String>) -> Vec<Entry> {
        let mut st = match self
            .db
            .prepare_cached("SELECT word,pos,gender,ipa,glosses FROM entry WHERE norm=?1 ORDER BY id")
        {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        st.query_map([norm], |r| {
            let glosses: String = r.get(4)?;
            Ok(Entry {
                word: r.get(0)?,
                pos: r.get(1)?,
                gender: r.get(2)?,
                ipa: r.get(3)?,
                glosses: serde_json::from_str(&glosses).unwrap_or_default(),
                form_tags: form_tags.clone(),
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    pub fn lookup(&mut self, surface: &str) -> Lookup {
        let n = norm(surface);
        let lemma = self.lemma(&n);
        let mut out = Lookup { word: surface.to_string(), lemma: lemma.clone(), rank: None, entries: vec![] };
        let mut by_lemma: Vec<(String, Vec<String>)> = vec![];
        if let Some(full) = self.elided(&n) {
            by_lemma.push((full.to_string(), vec![format!("elided form of {full}")]));
        }
        for (l, tags) in self.forms(&n) {
            let ln = norm(&l);
            match by_lemma.iter_mut().find(|(x, _)| *x == ln) {
                Some((_, t)) => {
                    if !tags.is_empty() && !t.contains(&tags) && t.len() < 4 {
                        t.push(tags)
                    }
                }
                None => by_lemma.push((ln, if tags.is_empty() { vec![] } else { vec![tags] })),
            }
        }
        // Most frequent headwords first; the chosen lemma always leads.
        by_lemma.sort_by_key(|(l, _)| (Some(l) != lemma.as_ref(), self.rank(l).unwrap_or(999_999)));
        let own = self.entries_for(&n, vec![]);
        let own_first = lemma.as_deref() == Some(n.as_str());
        if own_first {
            out.entries.extend(own.clone());
        }
        for (l, tags) in by_lemma.into_iter().take(4) {
            if l != n {
                out.entries.extend(self.entries_for(&l, tags));
            }
        }
        if !own_first {
            out.entries.extend(own);
        }
        out.rank = lemma.as_ref().and_then(|l| self.rank(l)).or_else(|| self.rank(&n));
        out
    }

    /// The N most frequent headwords, for "I already know the basics" onboarding.
    pub fn top_lemmas(&mut self, n: usize) -> Vec<String> {
        let forms: Vec<String> = {
            let mut st = match self.db.prepare("SELECT norm FROM freq ORDER BY rank LIMIT ?1") {
                Ok(s) => s,
                Err(_) => return vec![],
            };
            st.query_map([(n * 3) as i64], |r| r.get(0))
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default()
        };
        let mut out: Vec<String> = vec![];
        for f in forms {
            if let Some(l) = self.lemma(&f) {
                if !out.contains(&l) {
                    out.push(l);
                    if out.len() >= n {
                        break;
                    }
                }
            }
        }
        out
    }
}

/// Run `f` with the dictionary for `lang`, opening it on first use.
pub fn with_dict<T>(state: &AppState, lang: &str, f: impl FnOnce(&mut Dict) -> T) -> anyhow::Result<T> {
    let mut dicts = state.dicts.lock().unwrap();
    if !dicts.contains_key(lang) {
        let path = state.dir.join("components").join("dict").join(format!("dict-{lang}.sqlite"));
        if !path.exists() {
            anyhow::bail!("The {lang} dictionary is not installed yet (see Setup).");
        }
        dicts.insert(lang.to_string(), Dict::open(&path, lang)?);
    }
    Ok(f(dicts.get_mut(lang).unwrap()))
}

#[tauri::command]
pub fn dict_lookup(state: State<AppState>, lang: String, word: String) -> Res<Lookup> {
    Ok(with_dict(&state, &lang, |d| d.lookup(&word))?)
}

/// Exact multi-word lookup ("s'il vous plaît", "tout à fait").
#[tauri::command]
pub fn dict_phrase(state: State<AppState>, lang: String, phrase: String) -> Res<Vec<Entry>> {
    Ok(with_dict(&state, &lang, |d| d.entries_for(&norm(&phrase), vec![]))?)
}

#[tauri::command]
pub fn dict_top_lemmas(state: State<AppState>, lang: String, count: usize) -> Res<Vec<String>> {
    Ok(with_dict(&state, &lang, |d| d.top_lemmas(count))?)
}
