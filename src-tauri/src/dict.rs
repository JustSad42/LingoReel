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

#[derive(Serialize)]
pub struct ConjRow {
    /// "pres", "pc", "imp", "ps", "fut", "cond", "subj", "subj-imp", "imper".
    pub code: String,
    /// I, you, he/she, we, you (plural), they.
    pub cells: Vec<String>,
}

#[derive(Serialize)]
pub struct Conjugation {
    pub lemma: String,
    pub rows: Vec<ConjRow>,
    pub participle: String,
    pub gerund: String,
}

/// Usage labels for senses that should not be offered first: regional, slang, archaic and so on.
const SKIP_LABELS: &[&str] = &[
    "belgium", "quebec", "canada", "switzerland", "africa", "louisiana", "france, ", "latin america", "mexico", "argentina",
    "chile", "colombia", "cuba", "peru", "venezuela", "caribbean", "central america", "rioplatense", "slang", "vulgar",
    "dated", "archaic", "obsolete", "rare", "dialect", "regional", "euphemistic", "nonstandard", "proscribed", "humorous",
    "pejorative", "offensive", "nautical", "military", "historical", "jargon", "verlan", "colloquial", "middle ages",
];

/// French verbs that build the passé composé with être.
const FR_ETRE: &[&str] = &[
    "aller", "venir", "arriver", "partir", "entrer", "sortir", "naître", "mourir", "rester", "tomber", "monter", "descendre",
    "retourner", "devenir", "revenir", "rentrer", "apparaître", "décéder", "parvenir", "survenir", "intervenir", "repartir",
    "ressortir", "redevenir",
];

pub struct Dict {
    db: Connection,
    lang: String,
    lemma_cache: HashMap<String, Option<String>>,
}

/// Map Wiktionary's form description to a tense code; None for non-verb forms.
fn classify_tense(tags: &str) -> Option<&'static str> {
    let has = |k: &str| tags.contains(k);
    Some(if has("participle") {
        if has("past") { "pp" } else { "ppr" }
    } else if has("gerund") {
        "ppr"
    } else if has("subjunctive") {
        "subj"
    } else if has("imperative") {
        "imper"
    } else if has("conditional") {
        "cond"
    } else if has("future") {
        "fut"
    } else if has("historic") || (has("preterite") && !has("present")) {
        "ps"
    } else if has("imperfect") {
        "imp"
    } else if has("present") && has("indicative") {
        "pres"
    } else {
        return None;
    })
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

    pub fn is_verb(&self, norm: &str) -> bool {
        self.db
            .query_row("SELECT 1 FROM entry WHERE norm=?1 AND pos='verb' LIMIT 1", [norm], |_| Ok(()))
            .is_ok()
    }

    /// Meaning of a set phrase ("en train de", "tout à fait"), if the dictionary has it.
    /// Regional, slang and archaic senses are skipped, so "et des" is not flagged as Belgian slang.
    pub fn phrase_gloss(&self, norm: &str) -> Option<String> {
        if !norm.contains(' ') {
            return None;
        }
        let mut st = self
            .db
            .prepare_cached(
                "SELECT glosses FROM entry WHERE norm=?1 AND pos IN \
                 ('verb','adv','adj','noun','phrase','intj','prep','prep_phrase','conj','proverb') ORDER BY id",
            )
            .ok()?;
        let rows: Vec<String> = st.query_map([norm], |r| r.get(0)).ok()?.flatten().collect();
        for raw in rows {
            let Ok(glosses) = serde_json::from_str::<Vec<String>>(&raw) else { continue };
            for g in glosses {
                let lower = g.to_lowercase();
                if lower.starts_with('(') {
                    let label = lower[1..].split(')').next().unwrap_or("");
                    if SKIP_LABELS.iter().any(|l| label.contains(l)) {
                        continue;
                    }
                }
                if lower.contains("sexual") || lower.starts_with("alternative") || lower.contains(" form of ") || lower.contains("spelling of") {
                    continue;
                }
                // "at home (at one's home)" -> "at home"
                let mut text = g.trim().to_string();
                if text.ends_with(')') && !text.starts_with('(') {
                    if let Some(i) = text.rfind(" (") {
                        text.truncate(i);
                    }
                }
                if !text.is_empty() {
                    return Some(text);
                }
            }
        }
        None
    }

    /// Grammatical gender of a noun: "m", "f" or "n".
    pub fn noun_gender(&self, norm: &str) -> Option<&'static str> {
        // Only words whose main entry is a noun: "tous" also has a rare noun sense, but is not one here.
        let (pos, g): (String, String) = self
            .db
            .query_row("SELECT pos, gender FROM entry WHERE norm=?1 AND pos NOT IN ('name','proper noun') ORDER BY id LIMIT 1", [norm], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .ok()?;
        if pos != "noun" {
            return None;
        }
        match g.as_str() {
            "m" => Some("m"),
            "f" => Some("f"),
            "n" => Some("n"),
            _ => None,
        }
    }

    /// Words built on the same root, most common first: (word, part of speech, first meaning).
    pub fn family(&self, lemma: &str) -> Vec<(String, String, String)> {
        const ENDINGS: &[&str] = &[
            "ation", "ción", "sión", "zione", "ção", "mente", "ment", "eur", "euse", "dor", "dora", "ier", "ière", "ero", "era",
            "er", "ir", "re", "ar", "or", "en", "o", "a", "e",
        ];
        // A short stem matches unrelated words ("école" -> "écologique"), so only verbs may use 4 letters.
        let min = if self.is_verb(lemma) { 4 } else { 5 };
        let mut stem = lemma.to_string();
        for end in ENDINGS {
            if let Some(s) = lemma.strip_suffix(end) {
                if s.chars().count() >= min {
                    stem = s.to_string();
                    break;
                }
            }
        }
        if stem.chars().count() < min {
            return vec![];
        }
        let max_len = stem.chars().count() + 8;
        // Related words should share some meaning too: "chanteur" (singer) and "chanter" (to sing) do,
        // "chantier" (building site) does not. Compare the first four letters of the English words.
        const FILLER: &[&str] = &["that", "this", "with", "from", "which", "some", "someone", "something", "person", "thing", "other", "being", "having", "make", "very", "used", "into", "more", "about", "their", "also"];
        let roots = |text: &str| -> Vec<String> {
            text.to_lowercase()
                .split(|c: char| !c.is_alphabetic())
                .filter(|w| w.len() >= 4 && !FILLER.contains(w))
                .map(|w| w.chars().take(4).collect())
                .collect()
        };
        let own: std::collections::HashSet<String> = self.glosses(lemma).iter().flat_map(|g| roots(g)).collect();
        let mut st = match self.db.prepare_cached(
            "SELECT e.norm, e.pos, e.glosses FROM entry e LEFT JOIN freq f ON f.norm = e.norm \
             WHERE e.norm LIKE ?1 AND e.norm <> ?2 AND e.norm NOT LIKE '% %' AND e.pos NOT IN ('name','proper noun','character') \
             GROUP BY e.norm ORDER BY COALESCE(MIN(f.rank), 999999) LIMIT 40",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let rows = st.query_map(rusqlite::params![format!("{stem}%"), lemma], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
        });
        let Ok(rows) = rows else { return vec![] };
        rows.flatten()
            .filter(|(w, _, _)| w.chars().count() <= max_len)
            // Inflected forms (plurals, conjugations) are not separate family members.
            .filter(|(w, _, _)| !self.forms(w).iter().any(|(l, _)| norm(l) == lemma))
            .filter_map(|(w, pos, g)| {
                let first = serde_json::from_str::<Vec<String>>(&g).ok()?.into_iter().find(|x| {
                    let l = x.to_lowercase();
                    let label = l.strip_prefix('(').and_then(|r| r.split(')').next()).unwrap_or("");
                    !l.starts_with("plural of") && !l.contains(" of ") && !SKIP_LABELS.iter().any(|k| label.contains(k))
                })?;
                let all = serde_json::from_str::<Vec<String>>(&g).ok()?.join(" ");
                if !own.is_empty() && !roots(&all).iter().any(|r| own.contains(r)) {
                    return None;
                }
                Some((w, pos, first))
            })
            .take(6)
            .collect()
    }

    /// Conjugation table assembled from the dictionary's inflected forms.
    pub fn conjugation(&self, lemma: &str) -> Option<Conjugation> {
        if !self.is_verb(lemma) {
            return None;
        }
        let mut st = self.db.prepare_cached("SELECT norm, tags FROM form WHERE lemma=?1").ok()?;
        let forms: Vec<(String, String)> = st
            .query_map([lemma], |r| Ok((r.get(0)?, r.get(1)?)))
            .ok()?
            .flatten()
            .collect();
        const SKIP: &[&str] = &[
            "archaic", "obsolete", "combined-form", "object-", "with-voseo", "with-vos", "voseo", "reflexive", "formal",
            "negative", "dialectal", "rare", "semantically", "accusative", "dative", "imperfect-se", "alternative",
        ];
        const ROWS: &[&str] = &["pres", "imp", "ps", "fut", "cond", "subj", "subj-imp", "imper"];
        let mut table: HashMap<&str, Vec<String>> = ROWS.iter().map(|r| (*r, vec![String::new(); 6])).collect();
        let (mut participle, mut gerund) = (String::new(), String::new());
        for (form, tags) in &forms {
            // Some entries carry a pronunciation instead of a spelling.
            if form.chars().any(|c| ('\u{0250}'..='\u{02AF}').contains(&c) || matches!(c, '\'' | '’' | '-' | ' ')) || SKIP.iter().any(|s| tags.contains(s)) {
                continue;
            }
            let has = |k: &str| tags.split_whitespace().any(|t| t == k);
            if has("participle") && has("past") {
                if participle.is_empty() || (has("masculine") && has("singular")) {
                    participle = form.clone();
                }
                continue;
            }
            if has("gerund") || (has("participle") && has("present")) {
                if gerund.is_empty() || form.chars().count() < gerund.chars().count() {
                    gerund = form.clone();
                }
                continue;
            }
            let subj = has("subjunctive");
            let rows: Vec<&str> = [
                ("imper", has("imperative")),
                ("subj", subj && has("present")),
                ("subj-imp", subj && has("imperfect")),
                ("cond", has("conditional")),
                ("fut", has("future") && !subj),
                ("ps", (has("historic") || has("preterite")) && !subj),
                ("imp", has("imperfect") && !subj),
                ("pres", has("present") && !subj && !has("imperative")),
            ]
            .into_iter()
            .filter(|(_, ok)| *ok)
            .map(|(r, _)| r)
            .collect();
            let (sg, pl) = (has("singular"), has("plural"));
            for (i, p) in ["first-person", "second-person", "third-person"].iter().enumerate() {
                if !has(p) {
                    continue;
                }
                for slot in [sg.then_some(i), pl.then_some(i + 3)].into_iter().flatten() {
                    for r in &rows {
                        let cells = table.get_mut(r).unwrap();
                        let se = |f: &str| ["se", "ses", "semos", "seis", "sen"].iter().any(|e| f.ends_with(e));
                        if cells[slot].is_empty() || (*r == "subj-imp" && self.lang == "es" && se(&cells[slot]) && !se(form)) {
                            cells[slot] = form.clone();
                        }
                    }
                }
            }
        }
        if participle.is_empty() && self.lang == "fr" {
            if let Some(stem) = lemma.strip_suffix("er") {
                participle = format!("{stem}é");
            } else if let Some(stem) = lemma.strip_suffix("ir") {
                participle = format!("{stem}i");
            }
        }
        if self.lang == "es" {
            // Usted and nosotros commands are the present subjunctive; vosotros drops the r for d.
            let subj = table.get("subj").cloned().unwrap_or_default();
            if let Some(imper) = table.get_mut("imper") {
                for i in [2, 3, 5] {
                    if !subj[i].is_empty() {
                        imper[i] = subj[i].clone();
                    }
                }
                if let Some(stem) = lemma.strip_suffix('r') {
                    imper[4] = format!("{stem}d");
                }
            }
        }
        let mut rows: Vec<ConjRow> = ROWS
            .iter()
            .map(|r| ConjRow { code: r.to_string(), cells: table.remove(r).unwrap_or_default() })
            .filter(|r| r.cells.iter().any(|c| !c.is_empty()))
            .collect();
        // The everyday past: auxiliary in the present plus the past participle.
        let aux = match self.lang.as_str() {
            "fr" if FR_ETRE.contains(&lemma) => Some("être"),
            "fr" => Some("avoir"),
            "es" => Some("haber"),
            _ => None,
        };
        if let (Some(aux), false) = (aux, participle.is_empty()) {
            if aux != lemma {
                let present: [&str; 6] = match aux {
                    "être" => ["suis", "es", "est", "sommes", "êtes", "sont"],
                    "avoir" => ["ai", "as", "a", "avons", "avez", "ont"],
                    _ => ["he", "has", "ha", "hemos", "habéis", "han"],
                };
                let cells = present
                    .iter()
                    .enumerate()
                    .map(|(i, c)| if aux == "être" && i >= 3 { format!("{c} {participle}s") } else { format!("{c} {participle}") })
                    .collect();
                rows.insert(1.min(rows.len()), ConjRow { code: "pc".into(), cells });
            }
        }
        Some(Conjugation { lemma: lemma.to_string(), rows, participle, gerund })
    }

    /// Tenses the surface form can be, as a form of `lemma` ("pres", "imp", "pp", ...).
    pub fn tenses(&self, surface: &str, lemma: &str) -> Vec<&'static str> {
        let n = norm(surface);
        let mut out: Vec<&'static str> = vec![];
        for (l, tags) in self.forms(&n) {
            if norm(&l) != lemma {
                continue;
            }
            if let Some(c) = classify_tense(&tags) {
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
        if n == lemma && self.is_verb(&n) {
            out.push("inf");
        }
        out
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
            // A form of a very common verb is almost always that verb, even when the form
            // also has its own entry: Spanish "vamos" is "ir", not the interjection.
            let r = if r <= 300 { r / 4 } else { r };
            if best.as_ref().map(|(br, _)| r < *br).unwrap_or(true) {
                best = Some((r, ln));
            }
        }
        if best.is_none() {
            // Regular plurals and feminines are often missing: "émerveillés" -> "émerveillé".
            return self.regular_base(n).and_then(|(base, _)| self.lemma_uncached(&base));
        }
        best.map(|(_, l)| l)
    }

    /// For a French word the dictionary does not list, the form it is regularly built from.
    fn regular_base(&self, n: &str) -> Option<(String, &'static str)> {
        if self.lang != "fr" {
            return None;
        }
        for (suffix, label) in [("es", "feminine plural"), ("s", "plural"), ("e", "feminine"), ("x", "plural")] {
            if let Some(base) = n.strip_suffix(suffix) {
                if base.chars().count() >= 3 && self.exists(base) {
                    return Some((base.to_string(), label));
                }
            }
        }
        None
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

    /// English glosses of a lemma (already normalised), first senses first.
    pub fn glosses(&self, norm_lemma: &str) -> Vec<String> {
        self.entries_for(norm_lemma, vec![])
            .into_iter()
            .filter(|e| !matches!(e.pos.as_str(), "name" | "proper noun"))
            .flat_map(|e| e.glosses.into_iter().take(4))
            .take(8)
            .collect()
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
                // "allions" is a verb form: show the verb "aller", not the noun "aller".
                let verbal = tags.iter().any(|t| {
                    ["person", "participle", "indicative", "subjunctive", "imperative", "infinitive", "gerund", "conditional"]
                        .iter()
                        .any(|k| t.contains(k))
                });
                let all = self.entries_for(&l, tags);
                let fitting: Vec<Entry> = all.iter().filter(|e| (e.pos == "verb") == verbal).cloned().collect();
                out.entries.extend(if fitting.is_empty() || self.lang != "fr" { all } else { fitting });
            }
        }
        if !own_first {
            out.entries.extend(own);
        }
        if out.entries.is_empty() {
            if let Some((base, label)) = self.regular_base(&n) {
                let mut inner = self.lookup(&base);
                for e in inner.entries.iter_mut() {
                    e.form_tags.insert(0, format!("{label} of {base}"));
                }
                out.entries = inner.entries;
            }
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
        let path = crate::components::dir(state, &format!("dict-{lang}")).join(format!("dict-{lang}.sqlite"));
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

#[cfg(test)]
mod real_dict {
    use super::*;

    /// Runs against an installed dictionary: LINGOREEL_DICT=path\dict-fr.sqlite cargo test real_dict -- --ignored --nocapture
    #[test]
    #[ignore]
    fn study_helpers() {
        let path = std::env::var("LINGOREEL_DICT").unwrap();
        let lang = if path.contains("-es") { "es" } else { "fr" };
        let d = Dict::open(Path::new(&path), lang).unwrap();
        let verbs: &[&str] = if lang == "fr" { &["parler", "aller", "finir", "être", "avoir", "prendre"] } else { &["hablar", "ir", "tener", "ser", "vivir"] };
        for v in verbs {
            let c = d.conjugation(v).unwrap();
            println!("== {v} pp={} ger={}", c.participle, c.gerund);
            for r in &c.rows {
                println!("  {:9} {}", r.code, r.cells.join(" | "));
            }
        }
        let fam: &[&str] = if lang == "fr" { &["parler", "chanter", "maison", "travailler", "école", "chanteur", "regarder"] } else { &["hablar", "trabajar", "casa"] };
        for w in fam {
            println!("family {w}: {:?}", d.family(w));
        }
        let ph: &[&str] = if lang == "fr" { &["en train de", "tout à fait", "il y a", "et des", "pas de", "première fois", "à la maison", "de nos jours"] } else { &["sin embargo", "a lo mejor", "por fin", "darse cuenta"] };
        for p in ph {
            println!("phrase {p}: {:?}", d.phrase_gloss(&norm(p)));
        }
        let ns: &[&str] = if lang == "fr" { &["maison", "livre", "voiture", "chat"] } else { &["casa", "libro", "coche", "agua"] };
        for n in ns {
            println!("gender {n}: {:?}", d.noun_gender(n));
        }
    }
}
