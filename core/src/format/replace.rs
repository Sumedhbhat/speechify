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
    let mut candidates = Vec::new();
    for index in order {
        let words: Vec<String> = replacements[index].from.split_whitespace().map(regex::escape).collect();
        let matcher = Regex::new(&format!("(?i){}", words.join(r"[,.]?\s+"))).expect("escaped phrases always compile");
        for found in matcher.find_iter(text) {
            let before = text[..found.start()].chars().next_back();
            let rest = &text[found.end()..];
            let whole = !before.is_some_and(|c| c.is_alphanumeric())
                && !rest.chars().next().is_some_and(|c| c.is_alphanumeric())
                // "can't" and "don't" must not match "can" and "do"; "Asha's" may.
                && !rest.to_lowercase().starts_with("n't")
                && !contraction(rest)
                && before != Some('\'');
            if whole {
                candidates.push((found, index));
            }
        }
    }
    // Stable sorting preserves longest-rule priority at the same position.
    candidates.sort_by_key(|(found, _)| found.start());
    let mut out = String::with_capacity(text.len());
    let mut fired = Vec::new();
    let mut last = 0;
    for (found, index) in candidates {
        if found.start() < last {
            continue;
        }
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

    #[test]
    fn invalid_longer_rules_do_not_hide_valid_shorter_rules() {
        assert_eq!(
            run("foo barista foo bar", &[("foo bar", "long"), ("foo", "short")]),
            ("short barista long".into(), vec![0, 1])
        );
        assert_eq!(
            run("foo barista", &[("foo bar", "long"), ("barista", "B")]),
            ("foo B".into(), vec![1])
        );
    }
}
