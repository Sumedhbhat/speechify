//! Tone per destination: formal, casual, very casual, excited.

use crate::settings::Style;

/// Casual drops the closing full stop only on messages this short.
const CASUAL_SENTENCES: usize = 10;

fn sentences(text: &str) -> usize {
    text.split_inclusive(['.', '!', '?'])
        .filter(|part| part.chars().any(char::is_alphanumeric))
        .count()
}

fn is_list(text: &str) -> bool {
    text.lines()
        .any(|line| line.starts_with("- ") || line.split_once(". ").is_some_and(|(n, _)| n.parse::<u32>().is_ok()))
}

fn drop_final_stop(text: &str) -> String {
    match text.strip_suffix('.') {
        Some(rest) if !rest.ends_with('.') => rest.to_string(),
        _ => text.to_string(),
    }
}

/// Lowercases sentence starts, keeping "I", acronyms and mixed-case names.
fn lowercase_starts(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut start = true;
    for word in text.split_inclusive([' ', '\n']) {
        let bare = word.trim_matches(|c: char| !c.is_alphanumeric());
        let plain = bare.len() > 1
            && bare.chars().next().is_some_and(char::is_uppercase)
            && bare.chars().skip(1).all(|c| c.is_lowercase() || c == '\'')
            && !bare.starts_with("I'");
        if start && plain {
            let at = word.find(char::is_alphabetic).unwrap();
            out.push_str(&word[..at]);
            out.push_str(&word[at..at + 1].to_lowercase());
            out.push_str(&word[at + 1..]);
        } else {
            out.push_str(word);
        }
        let end = word.trim_end();
        start = end.ends_with(['.', '!', '?']) || word.ends_with('\n') || (start && bare.is_empty());
    }
    out
}

pub fn apply_style(text: &str, style: Style) -> String {
    let list = is_list(text);
    let ends_plain = text.chars().last().is_some_and(char::is_alphanumeric);
    match style {
        // A sentence gets its full stop; fragments, lists and one-word replies do not.
        Style::Formal if ends_plain && !list && text.split_whitespace().count() >= 3 => {
            format!("{text}.")
        }
        Style::Formal => text.to_string(),
        Style::Casual if sentences(text) <= CASUAL_SENTENCES && !list => drop_final_stop(text),
        Style::Casual => text.to_string(),
        Style::VeryCasual if list => text.to_string(),
        Style::VeryCasual => lowercase_starts(&drop_final_stop(text)),
        Style::Excited if list => text.to_string(),
        Style::Excited if ends_plain => format!("{text}!"),
        Style::Excited => match text.strip_suffix('.') {
            Some(rest) if !rest.ends_with('.') => format!("{rest}!"),
            _ => text.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formal_closes_sentences_only() {
        assert_eq!(
            apply_style("See you at the office", Style::Formal),
            "See you at the office."
        );
        assert_eq!(apply_style("Sounds good", Style::Formal), "Sounds good");
        assert_eq!(apply_style("Really?", Style::Formal), "Really?");
        assert_eq!(
            apply_style("Buy:\n1. Milk\n2. Tea leaves today", Style::Formal),
            "Buy:\n1. Milk\n2. Tea leaves today"
        );
    }

    #[test]
    fn casual_drops_the_last_full_stop_on_short_messages() {
        assert_eq!(
            apply_style("On my way. See you soon.", Style::Casual),
            "On my way. See you soon"
        );
        assert_eq!(apply_style("Wait...", Style::Casual), "Wait...");
        assert_eq!(apply_style("Really?", Style::Casual), "Really?");
        let long = "A b. ".repeat(11);
        assert_eq!(apply_style(long.trim(), Style::Casual), long.trim());
        assert_eq!(apply_style("- Milk.", Style::Casual), "- Milk.");
    }

    #[test]
    fn very_casual_lowercases_starts_but_not_names() {
        assert_eq!(
            apply_style("Hey. I'm late. Tell Asha I said hi.", Style::VeryCasual),
            "hey. I'm late. tell Asha I said hi"
        );
        assert_eq!(
            apply_style("NASA called. GitHub is down.\nOk", Style::VeryCasual),
            "NASA called. GitHub is down.\nok"
        );
        assert_eq!(apply_style("1. Milk\n2. Tea", Style::VeryCasual), "1. Milk\n2. Tea");
        assert_eq!(apply_style("\"Yes\" he said", Style::VeryCasual), "\"yes\" he said");
    }

    #[test]
    fn excited_ends_on_an_exclamation() {
        assert_eq!(apply_style("That works.", Style::Excited), "That works!");
        assert_eq!(apply_style("That works", Style::Excited), "That works!");
        assert_eq!(apply_style("Does it?", Style::Excited), "Does it?");
        assert_eq!(apply_style("Hmm...", Style::Excited), "Hmm...");
        assert_eq!(apply_style("- A\n- B", Style::Excited), "- A\n- B");
    }
}
