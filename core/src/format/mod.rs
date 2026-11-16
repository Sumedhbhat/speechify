//! Deterministic transcript formatting.
//!
//! This is what turns "um one buy milk two call mom" into a numbered list
//! without a language model. When a model is available it does the heavy
//! rewriting and these passes run around it (see `pipeline`).

mod backtrack;
mod fillers;
mod lists;
mod numbers;
mod replace;
mod spelling;
mod spoken;
mod style;
mod tidy;

pub(crate) use fillers::remove as remove_fillers;
pub(crate) use lists::enumerated;
pub use numbers::{group_digits, is_number_word};
pub use replace::{apply_replacements, Replacement};
pub use style::apply_style;
pub use tidy::tidy;

use crate::settings::{NumberSystem, Spelling, Style};
use serde::{Deserialize, Serialize};

/// What the formatter changed, shown to the user as "fixes made".
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edits {
    pub fillers: u32,
    pub backtracks: u32,
    pub lists: u32,
    pub numbers: u32,
    pub replacements: u32,
    pub spellings: u32,
}

impl Edits {
    pub fn total(&self) -> u32 {
        self.fillers + self.backtracks + self.lists + self.numbers + self.replacements + self.spellings
    }
}

#[derive(Debug, Clone)]
pub struct Options<'a> {
    pub style: Style,
    pub spelling: Spelling,
    pub numbers: NumberSystem,
    /// Dictionary corrections and snippets, applied last so they always win.
    pub replacements: &'a [Replacement],
    /// Code and terminal targets keep their wording: no spelling or style changes.
    pub literal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formatted {
    pub text: String,
    pub edits: Edits,
    /// Indexes into `Options::replacements` that fired, for hit counts.
    pub fired: Vec<usize>,
}

/// The full rule-based pass, used when no language model rewrites the text.
pub fn format(raw: &str, options: &Options) -> Formatted {
    let mut edits = Edits::default();
    if options.literal {
        return finish(raw, options, edits);
    }
    let text = tidy::collapse(raw);
    let text = fillers::remove(&text, &mut edits);
    let text = backtrack::apply(&text, &mut edits);
    let text = spoken::apply(&text);
    let text = numbers::apply(&text, options.numbers, &mut edits);
    let text = lists::apply(&text, &mut edits);
    finish(&text, options, edits)
}

/// The passes that run after a language model: the user's own words,
/// locale and style are enforced no matter what the model wrote.
pub fn finish(text: &str, options: &Options, mut edits: Edits) -> Formatted {
    let mut text = text.to_string();
    if !options.literal {
        text = numbers::regroup(&text, options.numbers);
        text = spelling::apply(&text, options.spelling, &mut edits);
    }
    let (text, fired) = apply_replacements(&text, options.replacements);
    edits.replacements += fired.len() as u32;
    let mut text = text;
    if !options.literal {
        text = tidy(&text);
        text = apply_style(&text, options.style);
    }
    Formatted { text, edits, fired }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(raw: &str) -> String {
        let options = Options {
            style: Style::Formal,
            spelling: Spelling::Indian,
            numbers: NumberSystem::Indian,
            replacements: &[],
            literal: false,
        };
        format(raw, &options).text
    }

    #[test]
    fn spoken_enumeration_becomes_a_numbered_list() {
        assert_eq!(
            run("um my plan for today is one, buy milk. two, call mom. three, pay the bills"),
            "My plan for today is:\n1. Buy milk\n2. Call mom\n3. Pay the bills"
        );
    }

    #[test]
    fn corrections_numbers_and_spelling_combine() {
        assert_eq!(
            run("the color budget is two lakh fifty thousand rupees, actually three lakh rupees"),
            "The colour budget is ₹3 lakh."
        );
    }

    #[test]
    fn literal_targets_keep_wording_but_still_get_the_dictionary() {
        let replacements = [Replacement::new("get hub", "GitHub")];
        let options = Options {
            style: Style::VeryCasual,
            spelling: Spelling::Indian,
            numbers: NumberSystem::Indian,
            replacements: &replacements,
            literal: true,
        };
        let out = format("Push the color fix to get hub.", &options);
        assert_eq!(out.text, "Push the color fix to GitHub.");
        assert_eq!(out.fired, vec![0]);
        assert_eq!(out.edits.total(), 1);
    }

    #[test]
    fn literal_commands_keep_case_numbers_spacing_and_identifiers() {
        let replacements = [Replacement::new("get hub", "GitHub")];
        let options = Options {
            style: Style::Formal,
            spelling: Spelling::Indian,
            numbers: NumberSystem::Indian,
            replacements: &replacements,
            literal: true,
        };
        let raw = "git status; printf '1,234'\n  color = get hub";
        for result in [format(raw, &options), finish(raw, &options, Edits::default())] {
            assert_eq!(result.text, "git status; printf '1,234'\n  color = GitHub");
            assert_eq!(result.fired, vec![0]);
            assert_eq!(result.edits.total(), 1);
        }
    }
}
