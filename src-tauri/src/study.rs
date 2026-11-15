//! Study helpers built on the dictionary and prepared videos: set phrases and noun genders
//! in each line, word families, conjugation tables, tense examples and level recommendations.
use crate::dict::{self, norm};
use crate::media::{self, Prepared};
use crate::util::Res;
use crate::AppState;
use serde::Serialize;
use std::collections::HashMap;
use tauri::State;

#[derive(Serialize, Default)]
pub struct Extras {
    /// Per line: (first token, last token, phrase, meaning).
    pub phrases: Vec<Vec<(u32, u32, String, String)>>,
    /// Per line: (token, "m" | "f" | "n").
    pub genders: Vec<Vec<(u32, String)>>,
}

/// Set phrases and noun genders for every line of a prepared video.
#[tauri::command(async)]
pub fn media_extras(state: State<AppState>, key: String) -> Res<Extras> {
    let Some(p) = media::load_prepared(&state, &key) else { return Ok(Extras::default()) };
    if matches!(p.lang.as_str(), "ja" | "zh") {
        return Ok(Extras { phrases: vec![vec![]; p.cues.len()], genders: vec![vec![]; p.cues.len()] });
    }
    let out = dict::with_dict(&state, &p.lang, |d| {
        let mut phrase_cache: HashMap<String, Option<String>> = HashMap::new();
        let mut gender_cache: HashMap<String, Option<&'static str>> = HashMap::new();
        let mut extras = Extras::default();
        for cue in &p.cues {
            let words: Vec<usize> = (0..cue.tokens.len()).filter(|&i| cue.tokens[i].w).collect();
            // Longest phrase first, without overlaps.
            let mut found = vec![];
            let mut w = 0;
            while w < words.len() {
                let mut hit = None;
                for n in (2..=4).rev() {
                    if w + n > words.len() {
                        continue;
                    }
                    let (a, b) = (words[w], words[w + n - 1]);
                    let text: String = cue.tokens[a..=b].iter().map(|t| t.t.as_str()).collect();
                    let key = norm(&text.split_whitespace().collect::<Vec<_>>().join(" "));
                    let gloss = phrase_cache
                        .entry(key.clone())
                        .or_insert_with(|| {
                            // Two of the commonest words together ("a la", "de la") are grammar, not a set phrase.
                            let parts: Vec<&str> = key.split(' ').collect();
                            if parts.len() == 2 && parts.iter().all(|w| d.rank(w).is_some_and(|r| r <= 60)) {
                                return None;
                            }
                            d.phrase_gloss(&key)
                        })
                        .clone();
                    if let Some(g) = gloss {
                        hit = Some((a as u32, b as u32, text, g, n));
                        break;
                    }
                }
                match hit {
                    Some((a, b, text, g, n)) => {
                        found.push((a, b, text, g));
                        w += n;
                    }
                    None => w += 1,
                }
            }
            extras.phrases.push(found);
            let mut genders = vec![];
            for (i, t) in cue.tokens.iter().enumerate() {
                let Some(lemma) = t.lemma.as_deref().filter(|_| t.w && t.g.is_none()) else { continue };
                let g = *gender_cache.entry(lemma.to_string()).or_insert_with(|| {
                    if d.is_verb(lemma) {
                        None
                    } else {
                        d.noun_gender(lemma)
                    }
                });
                if let Some(g) = g {
                    genders.push((i as u32, g.to_string()));
                }
            }
            extras.genders.push(genders);
        }
        extras
    })?;
    Ok(out)
}

/// Related words sharing the same root ("parler" -> parole, parleur), most common first.
#[tauri::command]
pub fn dict_family(state: State<AppState>, lang: String, lemma: String) -> Res<Vec<(String, String, String)>> {
    Ok(dict::with_dict(&state, &lang, |d| d.family(&norm(&lemma)))?)
}

/// Conjugation table for a verb, or None when the word is not a verb.
#[tauri::command]
pub fn dict_conjugate(state: State<AppState>, lang: String, lemma: String) -> Res<Option<dict::Conjugation>> {
    Ok(dict::with_dict(&state, &lang, |d| d.conjugation(&norm(&lemma)))?)
}

#[derive(Serialize)]
pub struct TenseExample {
    pub title: String,
    pub text: String,
    pub en: String,
    pub word: String,
}

/// Lines from the learner's own prepared videos that use a tense.
#[tauri::command(async)]
pub fn tense_examples(state: State<AppState>, lang: String, code: String) -> Res<Vec<TenseExample>> {
    let mut out = vec![];
    let Ok(rd) = std::fs::read_dir(state.dir.join("media")) else { return Ok(out) };
    let marker = format!("\"g\":\"{code}\"");
    for e in rd.flatten() {
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        if !text.contains(&marker) {
            continue;
        }
        let Ok(p) = serde_json::from_str::<Prepared>(&text) else { continue };
        if p.lang != lang {
            continue;
        }
        // A couple of short lines per video, so examples come from several places.
        let mut taken = 0;
        for cue in p.cues.iter().filter(|c| c.text.chars().count() <= 90) {
            // The same line can appear in several copies or versions of a video.
            if out.iter().any(|x: &TenseExample| x.text == cue.text) {
                continue;
            }
            if let Some(t) = cue.tokens.iter().find(|t| t.g.as_deref() == Some(code.as_str())) {
                out.push(TenseExample { title: p.title.clone(), text: cue.text.clone(), en: cue.en.clone(), word: t.t.clone() });
                taken += 1;
                if taken == 2 {
                    break;
                }
            }
        }
        if out.len() >= 8 {
            break;
        }
    }
    Ok(out)
}

#[derive(Serialize)]
pub struct Overview {
    pub key: String,
    pub title: String,
    pub url: String,
    /// Share of spoken words the learner knows, 0 to 100.
    pub known: f64,
    pub unique: u32,
}

/// Difficulty of every prepared video in a language, for "for your level" suggestions.
#[tauri::command(async)]
pub fn media_overview(state: State<AppState>, lang: String) -> Res<Vec<Overview>> {
    let mut out = vec![];
    let status = media::statuses(&state, &lang)?;
    let Ok(rd) = std::fs::read_dir(state.dir.join("media")) else { return Ok(out) };
    for e in rd.flatten() {
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        let Ok(p) = serde_json::from_str::<Prepared>(&text) else { continue };
        if p.lang != lang {
            continue;
        }
        let s = media::compute_stats(&state, &p, &status, false);
        out.push(Overview {
            key: p.key.clone(),
            title: p.title.clone(),
            url: p.url.clone(),
            known: if s.words > 0 { s.known as f64 * 100.0 / s.words as f64 } else { 0.0 },
            unique: s.unique,
        });
    }
    Ok(out)
}
