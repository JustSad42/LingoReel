//! Mixed subtitles: the English line with the words the learner knows swapped for the
//! studied language's words. Pairs are found through the dictionary's English glosses.
use crate::dict::{self, norm};
use crate::media::load_prepared;
use crate::util::Res;
use crate::AppState;
use std::collections::HashMap;
use tauri::State;

/// English words never swapped: they carry grammar more than meaning.
const STOP: &[&str] = &[
    "the", "a", "an", "to", "of", "and", "or", "but", "is", "are", "am", "was", "were", "be", "been", "being", "it", "i",
    "you", "he", "she", "we", "they", "me", "him", "her", "us", "them", "my", "your", "his", "its", "our", "their",
    "this", "that", "these", "those", "in", "on", "at", "for", "with", "as", "by", "from", "not", "no", "do", "does",
    "did", "so", "if", "there", "here", "what", "who", "s", "t", "ll", "re", "ve", "d", "m", "oh", "yeah", "okay", "ok",
    "just", "will", "would", "can", "could", "should", "has", "have", "had", "get", "got",
];

fn irregular(w: &str) -> Option<&'static str> {
    Some(match w {
        "went" | "gone" | "goes" => "go",
        "said" | "says" => "say",
        "made" => "make",
        "knew" | "known" => "know",
        "thought" => "think",
        "saw" | "seen" => "see",
        "came" => "come",
        "took" | "taken" => "take",
        "gave" | "given" => "give",
        "told" => "tell",
        "felt" => "feel",
        "left" => "leave",
        "found" => "find",
        "ate" | "eaten" => "eat",
        "wrote" | "written" => "write",
        "spoke" | "spoken" => "speak",
        "bought" => "buy",
        "brought" => "bring",
        "kept" => "keep",
        "slept" => "sleep",
        "lost" => "lose",
        "met" => "meet",
        "paid" => "pay",
        "sat" => "sit",
        "stood" => "stand",
        "understood" => "understand",
        "won" => "win",
        "began" | "begun" => "begin",
        "drank" | "drunk" => "drink",
        "ran" => "run",
        "sang" | "sung" => "sing",
        "men" => "man",
        "women" => "woman",
        "children" => "child",
        "people" => "person",
        "feet" => "foot",
        "better" | "best" => "good",
        "worse" | "worst" => "bad",
        _ => return None,
    })
}

/// Crude English stemmer: enough to match "houses" with "house" and "talked" with "talk".
pub fn stem(word: &str) -> String {
    let w = word.to_lowercase().replace('’', "'");
    let w = w.trim_end_matches("'s").trim_matches('\'');
    if let Some(base) = irregular(w) {
        return base.to_string();
    }
    let undouble = |s: &str| -> String {
        let c: Vec<char> = s.chars().collect();
        let n = c.len();
        if n >= 3 && c[n - 1] == c[n - 2] && !"aeioulsz".contains(c[n - 1]) {
            c[..n - 1].iter().collect()
        } else {
            s.to_string()
        }
    };
    let n = w.chars().count();
    let mut s = if n > 4 && w.ends_with("ies") {
        format!("{}y", &w[..w.len() - 3])
    } else if n > 5 && w.ends_with("ing") {
        undouble(&w[..w.len() - 3])
    } else if n > 4 && w.ends_with("ed") {
        undouble(&w[..w.len() - 2])
    } else if n > 4 && ["ses", "xes", "zes", "ches", "shes"].iter().any(|e| w.ends_with(e)) {
        w[..w.len() - 2].to_string()
    } else if n > 3 && w.ends_with('s') && !w.ends_with("ss") && !w.ends_with("us") {
        w[..w.len() - 1].to_string()
    } else {
        w.to_string()
    };
    if s.chars().count() > 3 && s.ends_with('e') {
        s.pop();
    }
    s
}

/// One-word English heads of dictionary glosses: "to see" -> see, "house, home" -> house, home.
fn heads(gloss: &str) -> Vec<String> {
    let mut clean = String::new();
    let mut depth = 0;
    for c in gloss.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => clean.push(c),
            _ => {}
        }
    }
    clean
        .split([';', ',', '/'])
        .filter_map(|part| {
            let words: Vec<&str> = part
                .split_whitespace()
                .filter(|w| !matches!(w.to_lowercase().as_str(), "to" | "a" | "an" | "the" | "one's" | "oneself" | "something" | "someone"))
                .collect();
            (words.len() == 1 && words[0].chars().all(|c| c.is_alphabetic() || c == '-')).then(|| stem(words[0]))
        })
        .collect()
}

/// Split an English line into words exactly as the UI does (letters, digits, apostrophes).
pub fn english_words(en: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    for c in en.chars() {
        if c.is_alphanumeric() || c == '\'' || c == '’' {
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// For every line: pairs of (English word index, studied-language token index).
#[tauri::command]
pub fn media_mix(state: State<AppState>, key: String) -> Res<Vec<Vec<(u32, u32)>>> {
    let Some(p) = load_prepared(&state, &key) else { return Ok(vec![]) };
    let out = dict::with_dict(&state, &p.lang, |d| {
        let mut cache: HashMap<String, Vec<String>> = HashMap::new();
        p.cues
            .iter()
            .map(|cue| {
                let words = english_words(&cue.en);
                let stems: Vec<String> = words.iter().map(|w| stem(w)).collect();
                let skip: Vec<bool> = words.iter().map(|w| STOP.contains(&w.to_lowercase().replace('’', "'").as_str())).collect();
                let mut used = vec![false; words.len()];
                let mut pairs = vec![];
                for (ti, tok) in cue.tokens.iter().enumerate() {
                    let Some(lemma) = tok.lemma.as_deref().filter(|_| tok.w) else { continue };
                    let cands = cache
                        .entry(lemma.to_string())
                        .or_insert_with(|| d.glosses(&norm(lemma)).iter().flat_map(|g| heads(g)).collect());
                    if cands.is_empty() {
                        continue;
                    }
                    if let Some(j) = (0..words.len()).find(|&j| !used[j] && !skip[j] && cands.contains(&stems[j])) {
                        used[j] = true;
                        pairs.push((j as u32, ti as u32));
                    }
                }
                pairs
            })
            .collect()
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_and_heads_line_up() {
        assert_eq!(stem("houses"), stem("house"));
        assert_eq!(stem("talked"), stem("talk"));
        assert_eq!(stem("running"), stem("run"));
        assert_eq!(stem("went"), "go");
        assert_eq!(heads("(transitive) to see; to watch"), vec![stem("see"), stem("watch")]);
        assert_eq!(heads("house, home"), vec![stem("house"), stem("home")]);
        assert!(heads("to go out").is_empty());
        assert_eq!(english_words("I don't have the time."), ["I", "don't", "have", "the", "time"]);
    }
}
