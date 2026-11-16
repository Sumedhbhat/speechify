//! Self-corrections: "at 2, actually 3" and "scratch that".

use super::{numbers, Edits};

/// A marker and whether it is unambiguous. Weak markers ("actually",
/// "I mean") are everyday words, so they only count when what follows
/// clearly restates what came before.
const MARKERS: &[(&[&str], Kind)] = &[
    (&["scratch", "that"], Kind::Erase),
    (&["strike", "that"], Kind::Erase),
    (&["delete", "that"], Kind::Erase),
    (&["never", "mind"], Kind::Erase),
    (&["no", "actually"], Kind::Strong),
    (&["actually", "no"], Kind::Strong),
    (&["no", "wait"], Kind::Strong),
    (&["wait", "no"], Kind::Strong),
    (&["no", "sorry"], Kind::Strong),
    (&["sorry", "i", "mean"], Kind::Strong),
    (&["or", "rather"], Kind::Weak),
    (&["i", "mean"], Kind::Weak),
    (&["actually"], Kind::Weak),
    (&["sorry"], Kind::Weak),
];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Erase,
    Strong,
    Weak,
}

/// How far back a correction may reach.
const REACH: usize = 15;
/// A weak marker followed by more than this is a new thought, not a fix.
const WEAK_TAIL: usize = 4;

fn core(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

fn ends_sentence(word: &str) -> bool {
    word.ends_with(['.', '!', '?'])
}

fn terminal(word: &str) -> &str {
    let trimmed = word.trim_end_matches(['.', '!', '?', ',', ';', ':']);
    let mark = &word[trimmed.len()..];
    if mark.contains(['.', '!', '?']) {
        mark
    } else {
        ""
    }
}

/// Things people commonly restate: a number, a day, a month.
fn class(word: &str) -> Option<u8> {
    const DAYS: &[&str] = &[
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
        "today",
        "tomorrow",
        "yesterday",
        "tonight",
    ];
    const MONTHS: &[&str] = &[
        "january",
        "february",
        "march",
        "april",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let word = core(word);
    if word.chars().next().is_some_and(|c| c.is_ascii_digit()) || numbers::is_number_word(&word) {
        Some(0)
    } else if DAYS.contains(&word.as_str()) {
        Some(1)
    } else if MONTHS.contains(&word.as_str()) {
        Some(2)
    } else {
        None
    }
}

pub fn apply(text: &str, edits: &mut Edits) -> String {
    let mut out = Vec::new();
    for line in text.split('\n') {
        let mut words: Vec<String> = line.split(' ').map(str::to_string).collect();
        let mut from = 0;
        while let Some((at, len, kind)) = find(&words, from) {
            match correct(&words, at, len, kind) {
                Some(fixed) => {
                    words = fixed;
                    edits.backtracks += 1;
                    from = 0;
                }
                None => from = at + len,
            }
        }
        out.push(words.join(" "));
    }
    super::tidy::tidy(&out.join("\n"))
}

fn find(words: &[String], from: usize) -> Option<(usize, usize, Kind)> {
    (from..words.len()).find_map(|at| {
        MARKERS.iter().find_map(|(marker, kind)| {
            let fits = at + marker.len() <= words.len()
                && marker.iter().enumerate().all(|(i, part)| core(&words[at + i]) == *part)
                // Punctuation inside a marker means it is two separate thoughts.
                && (0..marker.len() - 1).all(|i| !ends_sentence(&words[at + i]));
            fits.then_some((at, marker.len(), *kind))
        })
    })
}

fn correct(words: &[String], at: usize, len: usize, kind: Kind) -> Option<Vec<String>> {
    let before = &words[..at];
    let after = &words[at + len..];
    // The start of the sentence the speaker is taking back.
    let sentence_start = |end: usize| (0..end).rev().find(|&i| ends_sentence(&before[i])).map_or(0, |i| i + 1);
    if kind == Kind::Erase {
        let start = if at == 0 {
            return None;
        } else if sentence_start(at) == at {
            sentence_start(at - 1)
        } else {
            sentence_start(at)
        };
        return Some([&before[..start], after].concat());
    }
    let tail_len = after
        .iter()
        .position(|w| ends_sentence(w))
        .map_or(after.len(), |i| i + 1);
    let (tail, rest) = after.split_at(tail_len);
    let first = tail.first()?;
    let floor = match kind {
        Kind::Weak => sentence_start(at),
        _ => at.saturating_sub(REACH),
    };
    if kind == Kind::Weak && tail.len() > WEAK_TAIL {
        return None;
    }
    let single = tail.len() == 1;
    let multiplier = |word: &str| numbers::is_multiplier(&core(word));
    // May is a month only with case or nearby calendar evidence. Sentence-
    // initial "May I" and ordinary modal verbs must not become dates.
    let calendar = before.iter().chain(tail).any(|w| matches!(class(w), Some(1 | 2)));
    let contextual_class = |word: &str, mid_sentence: bool| {
        if core(word) == "may" && (calendar || (mid_sentence && word.starts_with('M'))) {
            Some(2)
        } else {
            class(word)
        }
    };
    let same_class = contextual_class(first, at > 0).and_then(|wanted| {
        // "two lakh, actually three": a lone number replaces the count, not the "lakh".
        let found = (floor..at).rev().find(|&i| {
            contextual_class(&before[i], i > 0 && !ends_sentence(&before[i - 1])) == Some(wanted)
                && (!single || multiplier(&before[i]) == multiplier(first))
        })?;
        // A restated phrase replaces the whole number it follows.
        let run_start = (floor..found)
            .rev()
            .take_while(|&i| class(&before[i]) == Some(0))
            .last();
        Some(if single || wanted != 0 { found } else { run_start.unwrap_or(found) })
    });
    let restart = || {
        let wanted = core(first);
        (floor..at).rev().find(|&i| core(&before[i]) == wanted)
    };
    let target = match same_class {
        Some(i) => i,
        // "the store, no wait, the mall": the speaker restarts from a repeated word.
        None => restart()?,
    };
    let mut fixed = before[..target].to_vec();
    if same_class.is_some() && single && target + 1 < at {
        // "at 2 pm actually 3": only the number changes; "pm" stays.
        let mark = terminal(first).to_string();
        fixed.push(first.trim_end_matches(['.', '!', '?', ',']).to_string());
        let mut kept = before[target + 1..].to_vec();
        if let Some(last) = kept.last_mut() {
            *last = format!("{}{mark}", last.trim_end_matches([',', ';', '.']));
        }
        fixed.extend(kept);
    } else {
        fixed.extend_from_slice(tail);
    }
    fixed.extend_from_slice(rest);
    Some(fixed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> String {
        apply(text, &mut Edits::default())
    }

    #[test]
    fn restated_numbers_and_days_replace_the_first_attempt() {
        assert_eq!(run("Let's do coffee at 2 actually 3"), "Let's do coffee at 3");
        assert_eq!(run("Let's do coffee at 2 pm, actually 3."), "Let's do coffee at 3 pm.");
        assert_eq!(run("Let's do coffee at 2 pm actually 3,"), "Let's do coffee at 3 pm");
        assert_eq!(run("Meet on Monday. No wait, Tuesday."), "Meet on Tuesday.");
        assert_eq!(
            run("Ship it in March, sorry, April. Thanks"),
            "Ship it in April. Thanks"
        );
        assert_eq!(run("I need five, no actually six copies"), "I need six copies");
    }

    #[test]
    fn may_needs_calendar_or_mid_sentence_capitalisation() {
        assert_eq!(run("Send it in May, actually June."), "Send it in June.");
        assert_eq!(run("Send it in April, actually May."), "Send it in May.");
        assert_eq!(run("Send it in may, actually june."), "Send it in june.");
        assert_eq!(
            run("Send it on Friday, actually may."),
            "Send it on Friday, actually may."
        );
        for text in ["We may actually go.", "May I actually go?", "We may, actually might."] {
            assert_eq!(run(text), text);
        }
        assert_eq!(run("Send it in May, actually May."), "Send it in May.");
    }

    #[test]
    fn restarts_replace_from_the_repeated_word() {
        assert_eq!(run("I'll go to the store, no wait, the mall."), "I'll go to the mall.");
        assert_eq!(run("We think, I mean, we know it works."), "We know it works.");
    }

    #[test]
    fn erase_markers_drop_the_sentence() {
        assert_eq!(
            run("Send it today. Scratch that. Send it tomorrow."),
            "Send it tomorrow."
        );
        assert_eq!(
            run("Hello team. I will be late, never mind, I am on time."),
            "Hello team. I am on time."
        );
        assert_eq!(run("Scratch that"), "Scratch that");
    }

    #[test]
    fn ordinary_uses_are_kept() {
        for text in [
            "I actually enjoyed the film.",
            "Sorry for the delay.",
            "I have 2 cats, actually 3 of them are still kittens today.",
            "The plan is good. I mean, the plan works for me.",
            "That is fine, actually no, forget it",
            "No. Wait for me",
            "Actually",
        ] {
            assert_eq!(run(text), text);
        }
    }

    #[test]
    fn counts_each_correction_and_keeps_lines() {
        let mut edits = Edits::default();
        let out = apply("At 2 actually 3\nOn Monday, I mean Friday", &mut edits);
        assert_eq!(out, "At 3\nOn Friday");
        assert_eq!(edits.backtracks, 2);
    }
}
