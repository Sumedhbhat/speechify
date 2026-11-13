//! Spoken punctuation and line-break commands.

use regex::Regex;
use std::sync::LazyLock;

static BREAKS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)[,.]?\s*\b(start a new paragraph|new paragraph|next paragraph|new line|next line|line break|skip a line)\b[,.]?\s*").unwrap()
});

static MARKS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(\b[\w']+)?([,.]?\s*)\b(full stop|comma|question mark|exclamation mark|exclamation point|semicolon|colon)\b[,.]?").unwrap()
});

/// "Remove the comma" is talking about punctuation, not dictating it.
const DETERMINERS: &[&str] = &[
    "the", "a", "an", "this", "that", "your", "my", "each", "every", "no", "extra", "missing", "of",
];

pub fn apply(text: &str) -> String {
    let text = MARKS.replace_all(text, |caps: &regex::Captures| {
        let previous = caps.get(1).map_or("", |m| m.as_str());
        if DETERMINERS.contains(&previous.to_lowercase().as_str()) {
            return caps[0].to_string();
        }
        let mark = match caps[3].to_lowercase().as_str() {
            "full stop" => ".",
            "comma" => ",",
            "question mark" => "?",
            "semicolon" => ";",
            "colon" => ":",
            _ => "!",
        };
        format!("{previous}{mark}")
    });
    let text = BREAKS.replace_all(&text, |caps: &regex::Captures| {
        let command = caps.get(1).unwrap();
        let previous = text[..command.start()]
            .trim_end_matches(|c: char| c.is_whitespace() || c == ',')
            .split_whitespace()
            .last()
            .unwrap_or("")
            .to_lowercase();
        let following = text[command.end()..]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        // Noun phrases ("a new paragraph about…", "the next line") are
        // content. A following preposition also attaches content to the noun.
        let noun = DETERMINERS.contains(&previous.as_str())
            || ["of", "about", "in", "for", "from", "with", "on"].contains(&following.as_str());
        if noun {
            caps[0].to_string()
        } else if command.as_str().to_lowercase().contains("paragraph") {
            "\n\n".into()
        } else {
            "\n".into()
        }
    });
    let lines: Vec<String> = text.split('\n').map(super::tidy::tidy).collect();
    lines.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_commands_become_breaks() {
        assert_eq!(
            apply("Hi Asha, new line. Thanks for the notes"),
            "Hi Asha\nThanks for the notes"
        );
        assert_eq!(
            apply("First part. New paragraph. Second part."),
            "First part\n\nSecond part."
        );
        assert_eq!(apply("one skip a line two"), "One\nTwo");
    }

    #[test]
    fn layout_nouns_stay_in_natural_sentences() {
        for text in [
            "We should discuss the new line of credit.",
            "We launched a new line of products.",
            "Read the next line of the poem.",
            "Write a new paragraph about it.",
            "The new paragraph is clearer.",
            "New paragraph on the economy.",
        ] {
            assert_eq!(apply(text), text);
        }
        assert_eq!(apply("milk new line eggs"), "Milk\nEggs");
        assert_eq!(apply("new line."), "");
        assert_eq!(apply("milk new paragraph eggs"), "Milk\n\nEggs");
        assert_eq!(apply("new paragraph."), "");
    }

    #[test]
    fn spoken_marks_become_punctuation() {
        assert_eq!(apply("hello comma how are you question mark"), "Hello, how are you?");
        assert_eq!(apply("Great exclamation point Done full stop"), "Great! Done.");
        assert_eq!(
            apply("Note colon bring ID semicolon arrive early"),
            "Note: bring ID; arrive early"
        );
    }

    #[test]
    fn talking_about_punctuation_keeps_the_words() {
        assert_eq!(apply("Remove the comma after hello"), "Remove the comma after hello");
        assert_eq!(apply("Add a question mark there"), "Add a question mark there");
    }
}
