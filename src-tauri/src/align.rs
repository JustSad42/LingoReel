//! Match subtitle words to the words Whisper heard, so each word lights up when spoken
//! and badly-timed subtitle files get corrected.
use crate::subs::{spread_tokens, Cue};
use std::collections::HashMap;
use unicode_normalization::UnicodeNormalization;

/// One word as heard by Whisper.
#[derive(Clone, Debug)]
pub struct Heard {
    pub text: String,
    pub s: f64,
    pub e: f64,
}

/// Comparable pieces of a word: lowercase, split on apostrophes/hyphens/punctuation.
pub fn keys(text: &str) -> Vec<String> {
    let mut out = vec![];
    for piece in text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|p| !p.is_empty()) {
        if piece.chars().any(is_cjk) {
            // No spaces between words: compare character by character.
            out.extend(piece.chars().map(|c| c.to_string()));
        } else {
            out.push(piece.to_string());
        }
    }
    out
}

pub fn is_cjk(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}')
}

fn deaccent(s: &str) -> String {
    s.nfd().filter(|c| !('\u{0300}'..='\u{036f}').contains(c)).collect()
}

fn edit_distance_le1(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (short, long) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
    if long.len() - short.len() > 1 {
        return false;
    }
    let mut i = 0;
    while i < short.len() && short[i] == long[i] {
        i += 1;
    }
    if short.len() == long.len() {
        short[i.min(short.len())..].iter().skip(1).eq(long[i.min(long.len())..].iter().skip(1))
    } else {
        short[i..].iter().eq(long[i + 1..].iter())
    }
}

/// How well two word pieces match: 0 = not at all.
fn similarity(a: &str, b: &str) -> i32 {
    let short = a.chars().count() <= 2;
    if a == b {
        return if short { 1 } else { 3 };
    }
    if short || b.chars().count() <= 2 {
        return 0;
    }
    let (da, db) = (deaccent(a), deaccent(b));
    if da == db {
        return 2;
    }
    // Whisper and subtitles often disagree on an ending: "parlé" / "parler", "ils" / "il".
    if da.chars().count() >= 4 && edit_distance_le1(&da, &db) {
        return 1;
    }
    0
}

struct SubKey {
    key: String,
    cue: usize,
    tok: usize,
    /// Rough time from the subtitle file.
    t: f64,
}

/// Constant offset between subtitle file and audio, found by voting with distinctive words.
fn estimate_offset(sub: &[SubKey], heard: &[(String, f64, f64)]) -> f64 {
    let mut where_heard: HashMap<&str, Vec<f64>> = HashMap::new();
    for (k, s, _) in heard {
        if k.chars().count() >= 5 {
            where_heard.entry(k.as_str()).or_default().push(*s);
        }
    }
    const BIN: f64 = 0.25;
    const RANGE: f64 = 120.0;
    let mut votes: HashMap<i64, u32> = HashMap::new();
    let mut deltas: Vec<f64> = vec![];
    for sk in sub {
        if let Some(times) = where_heard.get(sk.key.as_str()) {
            for t in times {
                let d = t - sk.t;
                if d.abs() <= RANGE {
                    *votes.entry((d / BIN).round() as i64).or_default() += 1;
                    deltas.push(d);
                }
            }
        }
    }
    let Some((&peak, &count)) = votes.iter().max_by_key(|(_, c)| **c) else { return 0.0 };
    if count < 5 {
        return 0.0;
    }
    let centre = peak as f64 * BIN;
    let near: Vec<f64> = deltas.into_iter().filter(|d| (d - centre).abs() <= 0.5).collect();
    near.iter().sum::<f64>() / near.len() as f64
}

pub struct AlignStats {
    pub offset: f64,
    pub matched: usize,
    pub total: usize,
}

/// Rewrite cue and token timings in place from what Whisper heard.
pub fn align(cues: &mut [Cue], heard_words: &[Heard]) -> AlignStats {
    // Flatten both sides into comparable pieces.
    let mut sub: Vec<SubKey> = vec![];
    for (ci, cue) in cues.iter_mut().enumerate() {
        spread_tokens(cue);
        for (ti, tok) in cue.tokens.iter().enumerate().filter(|(_, t)| t.w) {
            for key in keys(&tok.t) {
                sub.push(SubKey { key, cue: ci, tok: ti, t: tok.s });
            }
        }
    }
    let mut heard: Vec<(String, f64, f64)> = vec![];
    for h in heard_words {
        let ks = keys(&h.text);
        let total: usize = ks.iter().map(|k| k.chars().count()).sum();
        let mut acc = 0usize;
        for k in ks {
            let len = k.chars().count();
            let s = h.s + (h.e - h.s) * acc as f64 / total as f64;
            acc += len;
            let e = h.s + (h.e - h.s) * acc as f64 / total as f64;
            heard.push((k, s, e));
        }
    }
    let (n, m) = (sub.len(), heard.len());
    let mut stats = AlignStats { offset: 0.0, matched: 0, total: n };
    if n == 0 || m == 0 {
        return stats;
    }
    let offset = estimate_offset(&sub, &heard);
    stats.offset = offset;

    // Banded weighted-LCS: subtitle piece i may only match heard pieces within WINDOW seconds
    // of where the (offset-corrected) subtitle file says it should be.
    const WINDOW: f64 = 20.0;
    let starts: Vec<f64> = heard.iter().map(|h| h.1).collect();
    let mut lo = vec![0usize; n];
    let mut hi = vec![0usize; n]; // exclusive
    let (mut prev_lo, mut prev_hi) = (0usize, 0usize);
    for i in 0..n {
        let t = sub[i].t + offset;
        let l = starts.partition_point(|s| *s < t - WINDOW).max(prev_lo);
        let h = starts.partition_point(|s| *s <= t + WINDOW).max(prev_hi).max(l);
        lo[i] = l;
        hi[i] = h;
        prev_lo = l;
        prev_hi = h;
    }
    // score[i][j - lo[i]] = best score using sub[..=i] and heard[..=j]; back = move taken.
    let mut back: Vec<Vec<u8>> = Vec::with_capacity(n);
    let mut prev_row: Vec<i32> = vec![];
    let (mut p_lo, mut p_hi, mut p_base) = (0usize, 0usize, 0i32);
    for i in 0..n {
        let width = hi[i] - lo[i];
        let mut row = vec![0i32; width];
        let mut brow = vec![0u8; width];
        // Best score of the previous row at heard index < j (clamped into its band).
        let prev_at = |j_excl: usize| -> i32 {
            if prev_row.is_empty() {
                0
            } else if j_excl <= p_lo {
                p_base
            } else {
                prev_row[(j_excl.min(p_hi) - p_lo) - 1]
            }
        };
        let base = prev_at(lo[i]);
        for (c, j) in (lo[i]..hi[i]).enumerate() {
            let up = prev_at(j + 1); // skip this subtitle piece
            let left = if c > 0 { row[c - 1] } else { prev_at(j) }; // skip this heard piece
            let sim = similarity(&sub[i].key, &heard[j].0);
            let diag = if sim > 0 { prev_at(j) + sim } else { i32::MIN };
            let (best, mv) = if diag >= up && diag >= left {
                (diag, 1u8)
            } else if up >= left {
                (up, 2u8)
            } else {
                (left, 3u8)
            };
            row[c] = best;
            brow[c] = mv;
        }
        back.push(brow);
        if width > 0 {
            prev_row = row;
            p_base = base;
            p_lo = lo[i];
            p_hi = hi[i];
        }
    }
    // Trace back.
    let mut matched: Vec<Option<usize>> = vec![None; n];
    let mut i = n as isize - 1;
    let mut j = hi[n - 1] as isize - 1;
    while i >= 0 && j >= 0 {
        let iu = i as usize;
        let ju = (j as usize).min(hi[iu].saturating_sub(1));
        if hi[iu] == lo[iu] || ju < lo[iu] {
            i -= 1;
            continue;
        }
        j = ju as isize;
        match back[iu][ju - lo[iu]] {
            1 => {
                matched[iu] = Some(ju);
                i -= 1;
                j -= 1;
            }
            2 => i -= 1,
            _ => j -= 1,
        }
    }

    // Copy times onto tokens. A token made of several pieces spans all of them.
    for cue in cues.iter_mut() {
        for t in cue.tokens.iter_mut() {
            t.s = -1.0;
            t.e = -1.0;
        }
    }
    for (sk, mj) in sub.iter().zip(&matched) {
        if let Some(j) = mj {
            let tok = &mut cues[sk.cue].tokens[sk.tok];
            let (_, s, e) = &heard[*j];
            tok.s = if tok.s < 0.0 { *s } else { tok.s.min(*s) };
            tok.e = tok.e.max(*e);
        }
    }

    let mut last_shift = offset;
    for cue in cues.iter_mut() {
        let orig = (cue.start, cue.end);
        let word_idx: Vec<usize> = (0..cue.tokens.len()).filter(|&k| cue.tokens[k].w).collect();
        // Drop matches that sit far from the rest of the line (a stray "de" matched elsewhere).
        let mut times: Vec<f64> = word_idx.iter().map(|&k| cue.tokens[k].s).filter(|s| *s >= 0.0).collect();
        if !times.is_empty() {
            times.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let median = times[times.len() / 2];
            let tolerance = (orig.1 - orig.0) + 2.0;
            for &k in &word_idx {
                if cue.tokens[k].s >= 0.0 && (cue.tokens[k].s - median).abs() > tolerance {
                    cue.tokens[k].s = -1.0;
                    cue.tokens[k].e = -1.0;
                }
            }
        }
        let hit: Vec<usize> = word_idx.iter().copied().filter(|&k| cue.tokens[k].s >= 0.0).collect();
        stats.matched += hit.len();
        if hit.is_empty() || hit.len() * 2 < word_idx.len() {
            // Not enough evidence: keep the file's timing, shifted like its neighbours.
            cue.start = orig.0 + last_shift;
            cue.end = orig.1 + last_shift;
            spread_tokens(cue);
            continue;
        }
        const PER_CHAR: f64 = 0.065;
        let chars = |cue: &Cue, k: usize| cue.tokens[k].t.chars().count() as f64 + 1.0;
        // Fill unmatched words between/around the matched ones.
        let mut w = 0;
        while w < word_idx.len() {
            if cue.tokens[word_idx[w]].s >= 0.0 {
                w += 1;
                continue;
            }
            let run_start = w;
            while w < word_idx.len() && cue.tokens[word_idx[w]].s < 0.0 {
                w += 1;
            }
            let run = &word_idx[run_start..w];
            let need: f64 = run.iter().map(|&k| chars(cue, k)).sum::<f64>() * PER_CHAR;
            let before = (run_start > 0).then(|| cue.tokens[word_idx[run_start - 1]].e);
            let after = (w < word_idx.len()).then(|| cue.tokens[word_idx[w]].s);
            let (a, b) = match (before, after) {
                (Some(a), Some(b)) => (a, b.max(a)),
                (None, Some(b)) => (b - need, b),
                (Some(a), None) => (a, a + need),
                (None, None) => unreachable!(),
            };
            let total: f64 = run.iter().map(|&k| chars(cue, k)).sum();
            let mut acc = 0.0;
            for &k in run {
                let len = chars(cue, k);
                cue.tokens[k].s = a + (b - a) * acc / total;
                acc += len;
                cue.tokens[k].e = a + (b - a) * acc / total;
            }
        }
        cue.start = (cue.tokens[word_idx[0]].s - 0.1).max(0.0);
        cue.end = cue.tokens[*word_idx.last().unwrap()].e + 0.3;
        last_shift = cue.start - orig.0;
    }
    // Keep cues in order and non-overlapping.
    for k in 1..cues.len() {
        if cues[k].start < cues[k - 1].start {
            cues[k].start = cues[k - 1].start;
            cues[k].end = cues[k].end.max(cues[k].start + 0.2);
        }
        if cues[k - 1].end > cues[k].start {
            cues[k - 1].end = cues[k].start.max(cues[k - 1].start + 0.05);
        }
    }
    stats
}

/// Build cues straight from Whisper's words when there is no subtitle file at all.
pub fn cues_from_heard(heard: &[Heard], spaced: bool) -> Vec<Cue> {
    let mut cues: Vec<Cue> = vec![];
    let mut cur: Vec<&Heard> = vec![];
    let flush = |cur: &mut Vec<&Heard>, cues: &mut Vec<Cue>| {
        if cur.is_empty() {
            return;
        }
        let text = cur.iter().map(|h| h.text.trim()).collect::<Vec<_>>().join(if spaced { " " } else { "" });
        cues.push(Cue {
            start: cur[0].s,
            end: cur.last().unwrap().e + 0.2,
            text,
            ..Default::default()
        });
        cur.clear();
    };
    for h in heard {
        if let Some(last) = cur.last() {
            let len: usize = cur.iter().map(|x| x.text.len() + 1).sum();
            let sentence_end = last.text.trim_end().ends_with(['.', '?', '!', '…', '。', '？', '！']);
            let pause = h.s - last.e;
            if pause > 0.8 || (sentence_end && (len > 24 || pause > 0.3)) || len > 80 {
                flush(&mut cur, &mut cues);
            }
        }
        cur.push(h);
    }
    flush(&mut cur, &mut cues);
    cues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subs::tokenize;

    fn cue(start: f64, end: f64, text: &str) -> Cue {
        Cue { start, end, text: text.into(), tokens: tokenize("fr", text, None), ..Default::default() }
    }

    fn heard(words: &[(&str, f64, f64)]) -> Vec<Heard> {
        words.iter().map(|(t, s, e)| Heard { text: t.to_string(), s: *s, e: *e }).collect()
    }

    #[test]
    fn similarity_rules() {
        assert_eq!(similarity("bonjour", "bonjour"), 3);
        assert_eq!(similarity("de", "de"), 1);
        assert_eq!(similarity("élève", "eleve"), 2);
        assert_eq!(similarity("parlé", "parler"), 1);
        assert_eq!(similarity("maison", "voiture"), 0);
    }

    #[test]
    fn aligns_words_and_fixes_offset() {
        // Subtitle file is ~3 s early; the speech holds the true timing.
        let mut cues = vec![
            cue(7.0, 9.0, "Bonjour, comment allez-vous ?"),
            cue(10.0, 12.0, "Je m'appelle Marie Dupont."),
            cue(13.0, 15.0, "L'homme regarde la télévision maintenant."),
        ];
        let mut script: Vec<(&str, f64, f64)> = vec![
            ("Bonjour,", 10.0, 10.5), ("comment", 10.6, 11.0), ("allez-vous?", 11.0, 11.6),
            ("Euh", 12.0, 12.2), // heard but not in the subtitles
            ("Je", 13.0, 13.2), ("m'appelle", 13.2, 13.7), ("Marie", 13.7, 14.1), ("Dupont.", 14.1, 14.6),
            ("L'homme", 16.0, 16.5), ("regarde", 16.5, 17.0), ("la", 17.0, 17.1),
            ("télévision", 17.1, 17.8), ("maintenant.", 17.8, 18.4),
        ];
        // Padding with distinctive words so the offset vote has enough evidence.
        let filler = ["absolument", "certainement", "probablement", "évidemment", "naturellement", "heureusement"];
        let mut extra = vec![];
        for (k, w) in filler.iter().enumerate() {
            let t = 20.0 + k as f64 * 2.0;
            extra.push(cue(t, t + 1.0, w));
            script.push((w, t + 3.0, t + 3.8));
        }
        cues.extend(extra);
        let stats = align(&mut cues, &heard(&script));
        assert!((stats.offset - 3.0).abs() < 0.6, "offset {}", stats.offset);
        let word = |c: &Cue, w: &str| c.tokens.iter().find(|t| t.t == w).unwrap().clone();
        assert!((word(&cues[0], "comment").s - 10.6).abs() < 0.01);
        assert!((word(&cues[0], "vous").e - 11.6).abs() < 0.01);
        assert!((word(&cues[1], "Marie").s - 13.7).abs() < 0.01);
        assert!((word(&cues[2], "télévision").s - 17.1).abs() < 0.01);
        assert!((cues[0].start - 9.9).abs() < 0.01);
        assert!(cues[0].end <= cues[1].start);
        // Every word has a sane, ordered time.
        for c in &cues {
            let mut last = 0.0;
            for t in c.tokens.iter().filter(|t| t.w) {
                assert!(t.s >= last - 1e-6 && t.e >= t.s, "{:?}", t);
                last = t.s;
            }
        }
    }

    #[test]
    fn unmatched_words_are_interpolated() {
        let mut cues = vec![cue(1.0, 4.0, "Bonjour mademoiselle Catherine, bienvenue ici")];
        let h = heard(&[("Bonjour", 1.0, 1.4), ("bienvenue", 2.6, 3.1), ("ici", 3.1, 3.4)]);
        align(&mut cues, &h);
        let toks: Vec<_> = cues[0].tokens.iter().filter(|t| t.w).collect();
        assert!(toks[1].s >= 1.4 - 1e-6 && toks[2].e <= 2.6 + 1e-6);
        assert!(toks[1].e <= toks[2].s + 1e-6);
    }

    #[test]
    fn japanese_aligns_by_character() {
        assert_eq!(keys("映画を見た"), ["映", "画", "を", "見", "た"]);
        let mut cues = vec![Cue {
            start: 0.5,
            end: 3.0,
            text: "映画を見た".into(),
            tokens: tokenize("ja", "映画を見た", None),
            ..Default::default()
        }];
        let h = heard(&[("映", 1.0, 1.2), ("画", 1.2, 1.5), ("を", 1.5, 1.6), ("見", 1.6, 1.9), ("た", 1.9, 2.1)]);
        let stats = align(&mut cues, &h);
        assert_eq!(stats.matched, cues[0].tokens.iter().filter(|t| t.w).count());
        let first = cues[0].tokens.iter().find(|t| t.t == "映画").unwrap();
        assert!((first.s - 1.0).abs() < 0.01 && (first.e - 1.5).abs() < 0.01);
    }

    #[test]
    fn builds_cues_from_speech() {
        let h = heard(&[("Bonjour.", 0.0, 0.5), ("Ça", 2.0, 2.2), ("va", 2.2, 2.4), ("bien?", 2.4, 2.8)]);
        let cues = cues_from_heard(&h, true);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[1].text, "Ça va bien?");
    }
}
