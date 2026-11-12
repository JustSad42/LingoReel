//! Spaced repetition scheduling (FSRS-4.5 with default parameters, 90% target retention).

const W: [f64; 17] = [
    0.4872, 1.4003, 3.7145, 13.8206, 5.1618, 1.2298, 0.8975, 0.031, 1.6474, 0.1367, 1.0461, 2.1072,
    0.0793, 0.3246, 1.587, 0.2272, 2.8755,
];
const DECAY: f64 = -0.5;
const FACTOR: f64 = 19.0 / 81.0;
const MINUTE: i64 = 60;
const DAY: i64 = 86_400;

pub const NEW: i64 = 0;
pub const LEARNING: i64 = 1;
pub const REVIEW: i64 = 2;
pub const RELEARNING: i64 = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sched {
    pub state: i64,
    pub stability: f64,
    pub difficulty: f64,
    pub reps: i64,
    pub lapses: i64,
    pub last_review: i64,
    pub due: i64,
}

fn clamp_d(d: f64) -> f64 {
    d.clamp(1.0, 10.0)
}

fn init_difficulty(rating: i64) -> f64 {
    clamp_d(W[4] - (rating - 3) as f64 * W[5])
}

fn next_difficulty(d: f64, rating: i64) -> f64 {
    let moved = d - W[6] * (rating - 3) as f64;
    clamp_d(W[7] * init_difficulty(3) + (1.0 - W[7]) * moved)
}

fn retrievability(elapsed_days: f64, stability: f64) -> f64 {
    (1.0 + FACTOR * elapsed_days / stability).powf(DECAY)
}

fn stability_after_recall(d: f64, s: f64, r: f64, rating: i64) -> f64 {
    let hard = if rating == 2 { W[15] } else { 1.0 };
    let easy = if rating == 4 { W[16] } else { 1.0 };
    s * (1.0 + W[8].exp() * (11.0 - d) * s.powf(-W[9]) * ((W[10] * (1.0 - r)).exp() - 1.0) * hard * easy)
}

fn stability_after_forget(d: f64, s: f64, r: f64) -> f64 {
    (W[11] * d.powf(-W[12]) * ((s + 1.0).powf(W[13]) - 1.0) * (W[14] * (1.0 - r)).exp()).min(s).max(0.05)
}

fn days(stability: f64) -> i64 {
    (stability.round() as i64).clamp(1, 36_500) * DAY
}

/// Apply a rating (1 again, 2 hard, 3 good, 4 easy) at time `now` (unix seconds).
pub fn grade(card: Sched, rating: i64, now: i64) -> Sched {
    let mut c = card;
    c.reps += 1;
    match card.state {
        NEW => {
            c.stability = W[(rating - 1) as usize];
            c.difficulty = init_difficulty(rating);
            match rating {
                1 => (c.state, c.due) = (LEARNING, now + MINUTE),
                2 => (c.state, c.due) = (LEARNING, now + 5 * MINUTE),
                3 => (c.state, c.due) = (LEARNING, now + 10 * MINUTE),
                _ => (c.state, c.due) = (REVIEW, now + days(c.stability)),
            }
        }
        LEARNING | RELEARNING => {
            c.difficulty = next_difficulty(card.difficulty, rating);
            match rating {
                1 => c.due = now + MINUTE,
                2 => c.due = now + 5 * MINUTE,
                3 => (c.state, c.due) = (REVIEW, now + days(c.stability)),
                _ => {
                    c.stability = c.stability.max(W[3].min(c.stability * 2.0 + 1.0));
                    (c.state, c.due) = (REVIEW, now + days(c.stability));
                }
            }
        }
        _ => {
            let elapsed = ((now - card.last_review) as f64 / DAY as f64).max(0.0);
            let r = retrievability(elapsed, card.stability);
            c.difficulty = next_difficulty(card.difficulty, rating);
            if rating == 1 {
                c.stability = stability_after_forget(card.difficulty, card.stability, r);
                c.lapses += 1;
                (c.state, c.due) = (RELEARNING, now + 10 * MINUTE);
            } else {
                c.stability = stability_after_recall(card.difficulty, card.stability, r, rating);
                c.due = now + days(c.stability);
            }
        }
    }
    c.last_review = now;
    c
}

/// Seconds until the card would next be due for each of the four ratings.
pub fn preview(card: Sched, now: i64) -> [i64; 4] {
    [1, 2, 3, 4].map(|r| grade(card, r, now).due - now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_card() -> Sched {
        Sched { state: NEW, stability: 0.0, difficulty: 0.0, reps: 0, lapses: 0, last_review: 0, due: 0 }
    }

    #[test]
    fn new_card_steps() {
        let now = 1_000_000;
        let again = grade(new_card(), 1, now);
        assert_eq!((again.state, again.due - now), (LEARNING, 60));
        let easy = grade(new_card(), 4, now);
        assert_eq!((easy.state, easy.due - now), (REVIEW, 14 * DAY));
        let good = grade(grade(new_card(), 3, now), 3, now + 600);
        assert_eq!((good.state, good.due - (now + 600)), (REVIEW, 4 * DAY));
    }

    #[test]
    fn intervals_grow_on_success_and_shrink_on_lapse() {
        let mut c = grade(grade(new_card(), 3, 0), 3, 600);
        let mut last = c.due - c.last_review;
        for _ in 0..4 {
            let now = c.due;
            c = grade(c, 3, now);
            let interval = c.due - now;
            assert!(interval > last, "{interval} <= {last}");
            last = interval;
        }
        let stable = c.stability;
        let lapsed = grade(c, 1, c.due);
        assert_eq!(lapsed.state, RELEARNING);
        assert_eq!(lapsed.lapses, 1);
        assert!(lapsed.stability < stable);
        let p = preview(c, c.due);
        assert!(p[0] < p[1] && p[1] < p[2] && p[2] < p[3]);
    }
}
