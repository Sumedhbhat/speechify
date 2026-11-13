//! Removes hesitation sounds and stuttered words.

use super::Edits;
use regex::Regex;
use std::sync::LazyLock;

static FILLER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(^|[\s,])(?:u+m+|u+h+m*|e+r+m+|er|a+h+|h+m+|m+h+m+|mm+)\b[,.]?").unwrap());

/// Words people repeat while thinking. Anything else ("had had", "very very")
/// can be deliberate.
const STUTTERS: &[&str] = &[
    "the", "a", "an", "i", "to", "and", "we", "is", "it", "of", "in", "so", "but", "my", "you", "that's", "it's",
    "i'm", "for", "on", "with", "this", "he", "she", "they", "if", "can",
];

pub fn remove(text: &str, edits: &mut Edits) -> String {
    let mut count = 0;
    let stripped = FILLER.replace_all(text, |caps: &regex::Captures| {
        count += 1;
        // The separator before the filler stays: "so, um, yes" → "so, yes".
        caps[1].to_string()
    });
    let mut words: Vec<&str> = Vec::new();
    for word in stripped.split(' ') {
        let repeated = words.last().is_some_and(|last: &&str| {
            last.eq_ignore_ascii_case(word) && STUTTERS.contains(&word.to_lowercase().as_str())
        });
        if repeated {
            count += 1;
        } else {
            words.push(word);
        }
    }
    edits.fillers += count;
    super::tidy::tidy(&words.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> (String, u32) {
        let mut edits = Edits::default();
        let out = remove(text, &mut edits);
        (out, edits.fillers)
    }

    #[test]
    fn strips_hesitations_and_fixes_the_punctuation_around_them() {
        assert_eq!(
            run("Um, I think, uh, we should go."),
            ("I think, we should go.".into(), 2)
        );
        assert_eq!(
            run("So uhm that works. Hmm, maybe."),
            ("So that works. Maybe.".into(), 2)
        );
        assert_eq!(run("Erm. Okay"), ("Okay".into(), 1));
    }

    #[test]
    fn leaves_real_words_alone() {
        assert_eq!(run("The umbrella and the summer hum.").1, 0);
        assert_eq!(run("He had had enough, very very much.").1, 0);
    }

    #[test]
    fn collapses_stutters() {
        assert_eq!(
            run("I I think the the plan is is fine"),
            ("I think the plan is fine".into(), 3)
        );
    }
}
