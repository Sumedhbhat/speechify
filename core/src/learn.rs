//! Finds corrections worth remembering in the difference between what was
//! written and what it should have been.

use crate::asr::Heard;
use crate::diff::{align, normalize, words, Op};
use crate::format::Replacement;

/// Below this the recogniser was close to guessing between words, as in
/// a 50/50 call. Confidence is the lowest token probability in a word.
pub const UNSURE: f32 = 0.6;

/// Everyday words are never dictionary material on their own; swapping one
/// for another is a rewording, not a recognition error.
const COMMON: &str = " a about after again all also am an and any are as at back be because been before being but by can come could day did do does done down each even first for from get go going good had has have he her here him his how i if in into is it its just know like look made make many me more most much my new no not now of on one only or other our out over people said same see she should so some take than that the their them then there these they thing think this those time to too two up us use very want was way we well were what when where which who why will with work would year yes you your shall may might must okay ok hi hello thanks thank please sorry today tomorrow yesterday ";

fn common(word: &str) -> bool {
    COMMON.contains(&format!(" {} ", normalize(word)))
}

/// Paths, links, numbers and code are not vocabulary.
fn learnable(word: &str) -> bool {
    let bare = word.trim_matches(|c: char| !c.is_alphanumeric());
    !bare.is_empty()
        && bare.chars().count() <= 40
        && !bare.chars().any(|c| c.is_ascii_digit())
        // "two" to "three" is a change of fact, never a spelling fix.
        && !crate::format::is_number_word(&bare.to_lowercase())
        && !word.contains(['/', '\\', '@', '_', '=', '(', '{', '<'])
        && !word.contains("://")
        && !word.contains("::")
}

fn strip(word: &str) -> &str {
    word.trim_matches(|c: char| !c.is_alphanumeric() && c != '+' && c != '#')
}

/// Corrections found by comparing `before` with the corrected `after`.
/// Only small swaps count: a rewritten sentence teaches nothing about spelling.
pub fn corrections(before: &str, after: &str) -> Vec<Replacement> {
    let ops = align(before, after);
    let total = words(before).len().max(1);
    let changed: usize = ops
        .iter()
        .map(|op| if let Op::Change(left, _) = op { left.len().max(1) } else { 0 })
        .sum();
    if changed * 2 > total && total > 3 {
        return vec![];
    }
    let mut found: Vec<Replacement> = Vec::new();
    for op in &ops {
        let Op::Change(left, right) = op else {
            continue;
        };
        if left.is_empty() || right.is_empty() || left.len() > 3 || right.len() > 3 {
            continue;
        }
        if !left.iter().chain(right.iter()).all(|word| learnable(word)) {
            continue;
        }
        // Replacing one common word with another is editing, not correcting.
        if right.iter().all(|word| common(word)) {
            continue;
        }
        let from = left
            .iter()
            .map(|word| strip(word))
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let to = right.iter().map(|word| strip(word)).collect::<Vec<_>>().join(" ");
        if from == to.to_lowercase() || found.iter().any(|known| known.from == from) {
            continue;
        }
        found.push(Replacement { from, to });
    }
    // Capitalisation fixes ("github" → "GitHub") align as equal words.
    for op in &ops {
        let Op::Same(left, right) = op else { continue };
        let (left, right) = (strip(left), strip(right));
        let cased = right.chars().skip(1).any(char::is_uppercase);
        if left != right && cased && learnable(right) && !found.iter().any(|known| known.from == left.to_lowercase()) {
            found.push(Replacement {
                from: left.to_lowercase(),
                to: right.to_string(),
            });
        }
    }
    found
}

/// Names and terms in a text that a recogniser is likely to get wrong:
/// mixed-case or capitalised mid-sentence words that are not everyday English.
pub fn notable_terms(text: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    let mut sentence_start = true;
    for word in text.split_whitespace() {
        let bare = strip(word);
        let capital = bare.chars().next().is_some_and(char::is_uppercase);
        let mixed = bare.chars().skip(1).any(char::is_uppercase);
        if learnable(word)
            && !common(bare)
            && bare.chars().count() > 2
            && (mixed || (capital && !sentence_start))
            && !terms.iter().any(|t| t == bare)
        {
            terms.push(bare.to_string());
        }
        sentence_start = word.ends_with(['.', '!', '?', ':']);
    }
    terms
}

/// A rough sound of a word: letters grouped by how they are spoken, vowels
/// merged, silent letters dropped, repeats collapsed. "Sumedh Bhat" and
/// "sumit bat" share one sound; "open" and "close" do not.
fn sound(text: &str) -> Vec<char> {
    let letters: Vec<char> = text.to_lowercase().chars().filter(char::is_ascii_alphabetic).collect();
    let mut key: Vec<char> = Vec::new();
    for (index, letter) in letters.iter().enumerate() {
        let next = letters.get(index + 1).copied().unwrap_or(' ');
        let class = match letter {
            'a' | 'e' | 'i' | 'o' | 'u' | 'y' => 'V',
            'b' | 'p' => 'B',
            'f' | 'v' => 'F',
            'd' | 't' => 'T',
            'l' => 'L',
            'm' | 'n' => 'M',
            'r' => 'R',
            'c' if matches!(next, 'e' | 'i' | 'y') => 'S',
            'c' | 'k' | 'q' | 'g' => 'K',
            's' | 'z' | 'x' => 'S',
            'j' => 'J',
            // h and w: silent, or part of a vowel, in most words and names.
            _ => continue,
        };
        if key.last() != Some(&class) {
            key.push(class);
        }
    }
    key
}

/// Levenshtein distance between two sequences.
fn distance(a: &[char], b: &[char]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, left) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, right) in b.iter().enumerate() {
            let previous = row[j + 1];
            row[j + 1] = (diagonal + usize::from(left != right))
                .min(row[j] + 1)
                .min(previous + 1);
            diagonal = previous;
        }
    }
    row[b.len()]
}

/// Whether `to` sounds like `from`: a fix of what the recogniser heard,
/// rather than a change of what the user meant.
pub fn sounds_alike(from: &str, to: &str) -> bool {
    let (a, b) = (sound(from), sound(to));
    !a.is_empty() && distance(&a, &b) * 5 <= a.len().max(b.len()) * 2
}

/// How sure the recogniser was of `phrase`: the lowest confidence among its
/// words, or `None` when the recogniser did not say or the words are not found.
pub fn certainty(phrase: &str, heard: &[Heard]) -> Option<f32> {
    let wanted: Vec<String> = words(phrase).into_iter().map(normalize).collect();
    let found: Vec<f32> = heard
        .iter()
        .filter(|h| wanted.contains(&normalize(&h.word)))
        .map(|h| h.p)
        .collect();
    (found.len() >= wanted.len() && !wanted.is_empty()).then(|| found.iter().copied().fold(1.0, f32::min))
}

/// How much a correction proves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// It sounds like what was heard, and the recogniser was unsure: learn it now.
    Strong,
    /// It sounds alike, but the recogniser was confident or did not say:
    /// learn it only if the same correction comes up again.
    Weak,
    /// It changes the meaning: never learn it.
    None,
}

/// Judges a correction of words the recogniser heard (`heard` is its
/// per-word confidence for that dictation, possibly empty).
pub fn evidence(correction: &Replacement, heard: &[Heard]) -> Evidence {
    if !sounds_alike(&correction.from, &correction.to) {
        return Evidence::None;
    }
    match certainty(&correction.from, heard) {
        Some(p) if p < UNSURE => Evidence::Strong,
        _ => Evidence::Weak,
    }
}

/// A single recognition mismatch needs independent spelling evidence as well
/// as a small edit distance. Title case alone is not proof of a correction.
/// Ordinary lower-case words never qualify for automatic one-shot replacement.
pub fn corroborated(correction: &Replacement, before: &str, vocabulary: &[String]) -> bool {
    let source = notable_terms(before)
        .iter()
        .any(|term| normalize(term) == correction.from);
    let target = vocabulary.iter().any(|term| term == &correction.to);
    let a: Vec<char> = correction.from.chars().collect();
    let b: Vec<char> = correction.to.to_lowercase().chars().collect();
    if !source
        || !target
        || !correction.to.chars().any(char::is_uppercase)
        || a.len() < 4
        || common(&correction.from)
        || common(&correction.to)
    {
        return false;
    }
    distance(&a, &b) <= a.len().max(b.len()).div_ceil(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(words: &[(&str, f32)]) -> Vec<Heard> {
        words
            .iter()
            .map(|(word, p)| Heard {
                word: (*word).into(),
                p: *p,
            })
            .collect()
    }

    #[test]
    fn sound_alike_fixes_are_recognition_errors_and_rewordings_are_not() {
        for (from, to) in [
            ("sumit bat", "Sumedh Bhat"),
            ("cube control", "kubectl"),
            ("github", "GitHub"),
            ("ekanatha", "Ekanatham"),
            ("vikram", "Vikraam"),
        ] {
            assert!(sounds_alike(from, to), "{from} -> {to}");
        }
        for (from, to) in [
            ("open", "close"),
            ("thursday", "wednesday"),
            ("send", "delete"),
            ("yes", "no"),
            ("", "x"),
        ] {
            assert!(!sounds_alike(from, to), "{from} -> {to}");
        }
        // Every letter has a sound class; c softens before e, i and y.
        assert_eq!(
            sound("Jumping over the quick brown fox, lazy dog, cinema")
                .iter()
                .collect::<String>(),
            "JVMBVMKVFVRTVKVKBRVMFVSLVSVTVKSVMVMV"
        );
    }

    #[test]
    fn certainty_is_the_least_sure_word_of_the_phrase() {
        let said = heard(&[("cube", 0.4), ("control,", 0.9), ("tonight", 0.99)]);
        assert_eq!(certainty("cube control", &said), Some(0.4));
        assert_eq!(certainty("Tonight.", &said), Some(0.99));
        assert_eq!(certainty("kubectl", &said), None);
        assert_eq!(certainty("cube control", &[]), None);
        assert_eq!(certainty("", &said), None);
    }

    #[test]
    fn evidence_combines_sound_and_certainty() {
        let unsure = heard(&[("sumit", 0.45), ("bat", 0.7), ("open", 0.2)]);
        let sure = heard(&[("sumit", 0.95), ("bat", 0.97)]);
        let name = Replacement::new("sumit bat", "Sumedh Bhat");
        assert_eq!(evidence(&name, &unsure), Evidence::Strong);
        assert_eq!(evidence(&name, &sure), Evidence::Weak);
        assert_eq!(evidence(&name, &[]), Evidence::Weak);
        assert_eq!(evidence(&Replacement::new("open", "close"), &unsure), Evidence::None);
    }

    #[test]
    fn number_words_are_never_vocabulary() {
        assert!(corrections("order three boxes today", "order two boxes today").is_empty());
    }

    fn pairs(before: &str, after: &str) -> Vec<(String, String)> {
        corrections(before, after).into_iter().map(|r| (r.from, r.to)).collect()
    }

    #[test]
    fn learns_names_and_terms() {
        assert_eq!(
            pairs(
                "Ask Sumit Bat to deploy on cube control today.",
                "Ask Sumedh Bhat to deploy on kubectl today."
            ),
            vec![
                ("sumit bat".into(), "Sumedh Bhat".into()),
                ("cube control".into(), "kubectl".into())
            ]
        );
        assert_eq!(
            pairs("push it to github now", "push it to GitHub now"),
            vec![("github".into(), "GitHub".into())]
        );
        assert_eq!(
            pairs(
                "we use see plus plus for the engine code",
                "we use C++ for the engine code"
            ),
            vec![("see plus plus".into(), "C++".into())]
        );
    }

    #[test]
    fn ignores_rewording_numbers_and_code() {
        assert!(pairs("I will go there today", "I shall go there tomorrow").is_empty());
        assert!(pairs("it costs 500 rupees", "it costs 700 rupees").is_empty());
        assert!(pairs("open the config file", "open src/config.rs file").is_empty());
        assert!(pairs("see the site", "see https://example.com").is_empty());
        assert!(pairs("a b c d e f", "totally different words written here now").is_empty());
        assert!(pairs("hello there", "hello there friend").is_empty());
        assert!(pairs("Hello there.", "hello there").is_empty());
        assert!(pairs("call the one two three four team", "call the Zephyr team").is_empty());
    }

    #[test]
    fn repeated_corrections_are_reported_once() {
        assert_eq!(
            pairs(
                "sumit and sumit met rahul and vikram",
                "Sumedh and Sumedh met rahul and vikram"
            )
            .len(),
            1
        );
        assert_eq!(
            pairs("use github and github daily ok", "use GitHub and GitHub daily ok").len(),
            1
        );
    }

    #[test]
    fn one_sighting_needs_a_nearby_name_and_independent_spelling() {
        let known = vec!["GitHub".into(), "Kher".into(), "Shorts".into(), "Ekanatham".into()];
        assert!(corroborated(
            &Replacement::new("kerr", "Kher"),
            "Ask Kerr about the film",
            &known
        ));
        assert!(corroborated(
            &Replacement::new("githab", "GitHub"),
            "Use Githab for this",
            &known
        ));
        assert!(!corroborated(
            &Replacement::new("shorts", "Shorts"),
            "Composition of shorts",
            &known
        ));
        assert!(!corroborated(
            &Replacement::new("ekanatha", "Ekanatham"),
            "Ask Ekanatha",
            &[]
        ));
        assert!(!corroborated(
            &Replacement::new("kerr", "Kher"),
            "Kerr asked about it",
            &known
        ));
        assert!(!corroborated(&Replacement::new("kerr", "GitHub"), "Ask Kerr", &known));
        assert!(!corroborated(
            &Replacement::new("may", "May"),
            "It May rain",
            &["May".into()]
        ));
    }

    #[test]
    fn spots_names_and_terms() {
        assert_eq!(
            notable_terms("Tell Priya the Kubernetes rollout on GitHub is done. Then call HDFC about it. iPhone too."),
            vec!["Priya", "Kubernetes", "GitHub", "HDFC", "iPhone"]
        );
        assert!(notable_terms("The first one. I think so. Go to src/Main.rs or R2D2").is_empty());
    }
}
