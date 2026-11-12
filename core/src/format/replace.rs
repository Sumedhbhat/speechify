//! Whole-phrase replacements: dictionary corrections and snippets.

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replacement {
    pub from: String,
    pub to: String,
}

impl Replacement {
    pub fn new(from: &str, to: &str) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
        }
    }
}

/// Replaces each phrase where it stands as whole words, ignoring case.
/// Longer phrases win, and a recogniser's stray commas inside a phrase
/// ("my email, address") do not stop a match. Returns the indexes that fired.
pub fn apply_replacements(text: &str, replacements: &[Replacement]) -> (String, Vec<usize>) {
    let mut order: Vec<usize> = (0..replacements.len())
        .filter(|i| !replacements[*i].from.trim().is_empty())
        .collect();
    order.sort_by_key(|i| std::cmp::Reverse(replacements[*i].from.chars().count()));
    if order.is_empty() {
        return (text.to_string(), vec![]);
    }
    let pattern = order
        .iter()
        .map(|i| {
            let words: Vec<String> = replacements[*i].from.split_whitespace().map(regex::escape).collect();
            format!("({})", words.join(r"[,.]?\s+"))
        })
        .collect::<Vec<_>>()
        .join("|");
    let matcher = Regex::new(&format!("(?i){pattern}")).expect("escaped phrases always compile");
    let mut out = String::with_capacity(text.len());
    let mut fired = Vec::new();
    let mut last = 0;
    for caps in matcher.captures_iter(text) {
        let found = caps.get(0).unwrap();
        let before = text[..found.start()].chars().next_back();
        let rest = &text[found.end()..];
        let whole = !before.is_some_and(|c| c.is_alphanumeric())
            && !rest.chars().next().is_some_and(|c| c.is_alphanumeric())
            // "can't" and "don't" must not match rules for "can" and "do"; "Asha's" may.
            && !rest.to_lowercase().starts_with("n't")
            && !contraction(rest)
            && !before.is_some_and(|c| c == '\'');
        if !whole || found.start() < last {
            continue;
        }
        let index = order[(1..caps.len()).position(|group| caps.get(group).is_some()).unwrap()];
        out.push_str(&text[last..found.start()]);
        out.push_str(&replacements[index].to);
        last = found.end();
        if !fired.contains(&index) {
            fired.push(index);
        }
    }
    out.push_str(&text[last..]);
    fired.sort_unstable();
    (out, fired)
}

/// An apostrophe continuing the word, other than a possessive.
fn contraction(rest: &str) -> bool {
    let mut chars = rest.chars();
    let (quote, letter, after) = (chars.next(), chars.next(), chars.next());
    quote == Some('\'')
        && letter.is_some_and(char::is_alphabetic)
        && !(letter == Some('s') && !after.is_some_and(char::is_alphanumeric))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, pairs: &[(&str, &str)]) -> (String, Vec<usize>) {
        let list: Vec<Replacement> = pairs.iter().map(|(a, b)| Replacement::new(a, b)).collect();
        apply_replacements(text, &list)
    }

    #[test]
    fn whole_words_any_case_longest_first() {
        let rules = [("sumit", "Sumedh"), ("sumit bat", "Sumedh Bhat"), ("k8s", "Kubernetes")];
        assert_eq!(
            run("Ask Sumit Bat about K8S, not sumit.", &rules),
            ("Ask Sumedh Bhat about Kubernetes, not Sumedh.".into(), vec![0, 1, 2])
        );
        assert_eq!(
            run("summits and Sumit's laptop", &rules).0,
            "summits and Sumedh's laptop"
        );
    }

    #[test]
    fn contractions_and_partial_words_are_skipped() {
        assert_eq!(
            run("I can't scan the can.", &[("can", "tin")]).0,
            "I can't scan the tin."
        );
        assert_eq!(run("don't", &[("t", "x")]), ("don't".into(), vec![]));
    }

    #[test]
    fn snippets_match_through_recogniser_punctuation() {
        let rules = [("my email address", "me@example.com"), ("c++", "C++")];
        assert_eq!(
            run("Send it to my email, address please. I write c++.", &rules).0,
            "Send it to me@example.com please. I write C++."
        );
    }

    #[test]
    fn empty_rules_change_nothing() {
        assert_eq!(run("hello", &[]), ("hello".into(), vec![]));
        assert_eq!(run("hello", &[(" ", "x")]), ("hello".into(), vec![]));
    }
}
