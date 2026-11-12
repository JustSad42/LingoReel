//! Subtitle parsing, cleaning and tokenising.
use crate::dict::{norm, Dict};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Tok {
    /// Text exactly as shown.
    pub t: String,
    /// True for words, false for spaces and punctuation.
    pub w: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub lemma: Option<String>,
    /// When the word is spoken (seconds).
    #[serde(default)]
    pub s: f64,
    #[serde(default)]
    pub e: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
    #[serde(default)]
    pub tokens: Vec<Tok>,
    /// Matching native-language line.
    #[serde(default)]
    pub en: String,
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).unwrap())
}

fn parse_time(s: &str) -> Option<f64> {
    let s = s.trim().replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    let mut secs = 0.0;
    for p in &parts {
        secs = secs * 60.0 + p.trim().parse::<f64>().ok()?;
    }
    Some(secs)
}

/// Strip markup, sound descriptions and music lines; returns "" when nothing is spoken.
pub fn clean_text(raw: &str) -> String {
    static TAGS: OnceLock<Regex> = OnceLock::new();
    static HI: OnceLock<Regex> = OnceLock::new();
    static SPEAKER: OnceLock<Regex> = OnceLock::new();
    static WS: OnceLock<Regex> = OnceLock::new();
    let s = re(&TAGS, r"\{[^}]*\}|<[^>]*>").replace_all(raw, "");
    let s = re(&HI, r"\[[^\]]*\]|\([^)]*\)|[♪♫#]+[^♪♫#\n]*[♪♫#]+|[♪♫]").replace_all(&s, " ");
    let s = s.replace("\\N", "\n").replace("\\n", "\n").replace("\\h", " ");
    let mut lines = vec![];
    for line in s.lines() {
        // "MARIE: Bonjour" style speaker labels
        let line = re(&SPEAKER, r"^\s*(-\s*)?[A-ZÀ-Ý][A-ZÀ-Ý .'-]{1,20}\s?:\s+").replace(line, "$1");
        let line = line.trim();
        if !line.is_empty() && line.chars().any(|c| c.is_alphanumeric()) {
            lines.push(line.to_string());
        }
    }
    re(&WS, r"\s+").replace_all(&lines.join(" "), " ").trim().to_string()
}

/// Parse SRT or WebVTT text into cleaned cues.
pub fn parse_srt(text: &str) -> Vec<Cue> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut cues: Vec<Cue> = vec![];
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        let Some(ti) = lines.iter().position(|l| l.contains("-->")) else { continue };
        let mut parts = lines[ti].split("-->");
        let start = parts.next().and_then(parse_time);
        let end = parts.next().and_then(|e| parse_time(e.trim().split(' ').next().unwrap_or("")));
        let (Some(start), Some(end)) = (start, end) else { continue };
        let body = clean_text(&lines[ti + 1..].join("\n"));
        if body.is_empty() || end <= start {
            continue;
        }
        // Some files repeat the same line across consecutive cues; merge them.
        if let Some(last) = cues.last_mut() {
            if last.text == body && start - last.end < 0.1 {
                last.end = end;
                continue;
            }
        }
        cues.push(Cue { start, end, text: body, ..Default::default() });
    }
    cues.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    cues
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

const FR_ELIDE: &[&str] = &[
    "l", "d", "j", "n", "m", "t", "s", "c", "qu", "jusqu", "lorsqu", "puisqu", "quoiqu",
];

/// Split a line into word and non-word tokens. Words keep inner apostrophes and hyphens
/// only when the dictionary knows the whole thing ("aujourd'hui", "peut-être").
pub fn tokenize(lang: &str, text: &str, dict: Option<&mut Dict>) -> Vec<Tok> {
    let chars: Vec<char> = text.chars().collect();
    let mut chunks: Vec<(String, bool)> = vec![];
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if is_word_char(c) {
            let mut j = i;
            while j < chars.len() {
                let cj = chars[j];
                let joiner = matches!(cj, '\'' | '\u{2019}' | '-')
                    && j + 1 < chars.len()
                    && is_word_char(chars[j + 1]);
                if is_word_char(cj) || joiner {
                    j += 1;
                } else {
                    break;
                }
            }
            // A trailing apostrophe belongs to the word ("l' homme" spacing in some subs).
            if j < chars.len() && matches!(chars[j], '\'' | '\u{2019}') {
                j += 1;
            }
            chunks.push((chars[i..j].iter().collect(), true));
            i = j;
        } else {
            let mut j = i;
            while j < chars.len() && !is_word_char(chars[j]) {
                j += 1;
            }
            chunks.push((chars[i..j].iter().collect(), false));
            i = j;
        }
    }

    let mut dict = dict;
    let mut known = |w: &str| dict.as_deref_mut().map(|d| d.exists(&norm(w))).unwrap_or(false);
    let mut out: Vec<Tok> = vec![];
    let push = |out: &mut Vec<Tok>, t: &str, w: bool| {
        if !t.is_empty() {
            out.push(Tok { t: t.to_string(), w, ..Default::default() })
        }
    };
    for (chunk, is_word) in chunks {
        if !is_word {
            push(&mut out, &chunk, false);
            continue;
        }
        let has_joiner = chunk.contains(['\'', '\u{2019}', '-']);
        if !has_joiner || known(&chunk) {
            push(&mut out, &chunk, true);
            continue;
        }
        // Elision: l'homme -> l' + homme
        let mut rest = chunk.as_str();
        if lang == "fr" {
            if let Some(pos) = rest.find(['\'', '\u{2019}']) {
                let prefix = norm(&rest[..pos]);
                if FR_ELIDE.contains(&prefix.as_str()) {
                    let cut = pos + rest[pos..].chars().next().unwrap().len_utf8();
                    push(&mut out, &rest[..cut], true);
                    rest = &rest[cut..];
                }
            }
        }
        if rest.is_empty() {
            continue;
        }
        if !rest.contains('-') || known(rest) {
            push(&mut out, rest, true);
            continue;
        }
        // Inversion: allez-vous, a-t-il
        let mut first = true;
        for part in rest.split('-') {
            if !first {
                push(&mut out, "-", false);
            }
            push(&mut out, part, true);
            first = false;
        }
    }
    if let Some(d) = dict.as_deref_mut() {
        for t in out.iter_mut().filter(|t| t.w) {
            t.lemma = d.lemma(&t.t);
        }
    }
    out
}

/// Give each word a time slice of the cue in proportion to its length. Used when no
/// speech alignment is available, and for cues the aligner could not anchor.
pub fn spread_tokens(cue: &mut Cue) {
    let total: usize = cue.tokens.iter().filter(|t| t.w).map(|t| t.t.chars().count() + 1).sum();
    if total == 0 {
        return;
    }
    let dur = (cue.end - cue.start).max(0.1);
    let mut acc = 0usize;
    for t in cue.tokens.iter_mut().filter(|t| t.w) {
        let len = t.t.chars().count() + 1;
        t.s = cue.start + dur * acc as f64 / total as f64;
        acc += len;
        t.e = cue.start + dur * acc as f64 / total as f64;
    }
}

/// Attach each native-language cue to the target cue it overlaps most.
pub fn pair_native(cues: &mut [Cue], native: &[Cue]) {
    if cues.is_empty() {
        return;
    }
    let mut texts: Vec<Vec<&str>> = vec![vec![]; cues.len()];
    let mut ci = 0usize;
    for n in native {
        while ci + 1 < cues.len() && cues[ci].end < n.start - 2.0 {
            ci += 1;
        }
        let mut best: Option<(usize, f64)> = None;
        let mut k = ci;
        while k < cues.len() && cues[k].start < n.end + 1.5 {
            let overlap = n.end.min(cues[k].end) - n.start.max(cues[k].start);
            // Negative overlap = distance; still allow near misses up to 1.5 s.
            if overlap > -1.5 && best.map(|(_, b)| overlap > b).unwrap_or(true) {
                best = Some((k, overlap));
            }
            k += 1;
        }
        if let Some((k, _)) = best {
            if !texts[k].contains(&n.text.as_str()) {
                texts[k].push(&n.text);
            }
        }
    }
    for (cue, t) in cues.iter_mut().zip(texts) {
        cue.en = t.join(" ");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_parses_and_cleans() {
        let srt = "1\n00:00:01,000 --> 00:00:02,500\n<i>Bonjour</i> {\\an8}tout le monde\n\n2\n00:00:03,000 --> 00:00:04,000\n[musique]\n\n3\n00:00:05,000 --> 00:00:06,000\n- Ça va ?\n- Oui.\n";
        let cues = parse_srt(srt);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "Bonjour tout le monde");
        assert!((cues[0].end - 2.5).abs() < 1e-9);
        assert_eq!(cues[1].text, "- Ça va ? - Oui.");
    }

    #[test]
    fn french_tokens_split_elisions_and_inversions() {
        let words = |s: &str| -> Vec<String> {
            tokenize("fr", s, None).into_iter().filter(|t| t.w).map(|t| t.t).collect()
        };
        assert_eq!(words("L'homme n'est pas là."), ["L'", "homme", "n'", "est", "pas", "là"]);
        assert_eq!(words("Où allez-vous ?"), ["Où", "allez", "vous"]);
        assert_eq!(words("Qu’est-ce que c’est"), ["Qu’", "est", "ce", "que", "c’", "est"]);
        let all: String = tokenize("fr", "Qu’est-ce que c’est ?", None).into_iter().map(|t| t.t).collect();
        assert_eq!(all, "Qu’est-ce que c’est ?");
    }

    #[test]
    fn native_lines_pair_by_overlap() {
        let mut cues = vec![
            Cue { start: 1.0, end: 2.0, text: "a".into(), ..Default::default() },
            Cue { start: 3.0, end: 5.0, text: "b".into(), ..Default::default() },
        ];
        let native = vec![
            Cue { start: 1.1, end: 2.2, text: "A".into(), ..Default::default() },
            Cue { start: 2.9, end: 4.0, text: "B1".into(), ..Default::default() },
            Cue { start: 4.0, end: 5.0, text: "B2".into(), ..Default::default() },
        ];
        pair_native(&mut cues, &native);
        assert_eq!(cues[0].en, "A");
        assert_eq!(cues[1].en, "B1 B2");
    }
}
