//! Finds corrections worth remembering in the difference between what was
//! written and what it should have been.

use crate::diff::{align, normalize, words, Op};
use crate::format::Replacement;

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
    row[b.len()] <= a.len().max(b.len()).div_ceil(3)
}

#[cfg(test)]
mod tests {
    use super::*;

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
