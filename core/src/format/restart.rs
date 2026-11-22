use super::Edits;

const LONGEST: usize = 6;
const INSERTED: usize = 2;
const EXACT: usize = 3;
const JOINERS: &[&str] = &["and", "or", "but", "nor", "then", "than", "vs", "versus"];

fn core(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

fn ends_sentence(word: &str) -> bool {
    word.ends_with(['.', '!', '?', ';', ':'])
}

fn pauses(word: &str) -> bool {
    word.ends_with([',', '.', '!', '?', ';', ':'])
}

fn restated(left: &[String], right: &[String]) -> bool {
    let (first, last) = (core(&left[0]), core(&left[left.len() - 1]));
    if right.is_empty() || core(&right[0]) != first || left[..left.len() - 1].iter().any(|w| pauses(w)) {
        return false;
    }
    let exact = left.len() >= EXACT
        && left[left.len() - 1].ends_with(',')
        && right.len() > left.len()
        && left.iter().zip(right).all(|(a, b)| core(a) == core(b))
        && !pauses(&right[left.len() - 1]);
    if exact {
        return true;
    }
    let mut at = 1;
    let mut inserted = 0;
    for wanted in &left[1..] {
        let wanted = core(wanted);
        loop {
            let Some(word) = right.get(at) else { return false };
            if pauses(word) && core(word) != wanted {
                return false;
            }
            at += 1;
            if core(word) == wanted {
                break;
            }
            if JOINERS.contains(&core(word).as_str()) || word.ends_with('\'') || word.contains("'s") {
                return false;
            }
            inserted += 1;
            if inserted > INSERTED {
                return false;
            }
        }
    }
    inserted > 0 && core(&right[at - 1]) == last && first != last
}

pub fn apply(text: &str, edits: &mut Edits) -> String {
    let mut changed = false;
    let lines: Vec<String> = text
        .split('\n')
        .map(|line| {
            let mut words: Vec<String> = line.split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect();
            let mut at = 1;
            while at < words.len() {
                let found = (2..=LONGEST.min(at))
                    .find(|&len| !ends_sentence(&words[at - 1]) && restated(&words[at - len..at], &words[at..]));
                match found {
                    Some(len) => {
                        let start = at - len;
                        if words[start].starts_with(char::is_uppercase) {
                            words[at] = super::tidy::capitalize(&words[at]);
                        }
                        words.drain(start..at);
                        edits.backtracks += 1;
                        changed = true;
                        at = start.max(1);
                    }
                    None => at += 1,
                }
            }
            words.join(" ")
        })
        .collect();
    if changed {
        lines.join("\n")
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> String {
        apply(text, &mut Edits::default())
    }

    #[test]
    fn a_phrase_said_again_with_more_detail_keeps_only_the_second_try() {
        for (said, meant) in [
            (
                "Let's merge the latest branch, latest master branch into this branch, please.",
                "Let's merge the latest master branch into this branch, please.",
            ),
            (
                "Let's merge the latest branch latest master branch into this branch please",
                "Let's merge the latest master branch into this branch please",
            ),
            (
                "Send it to the team, the design team by Friday.",
                "Send it to the design team by Friday.",
            ),
            (
                "We should run the tests, run the integration tests before we push.",
                "We should run the integration tests before we push.",
            ),
            (
                "Let's rebase on main, on the latest main and then push.",
                "Let's rebase on the latest main and then push.",
            ),
            (
                "Can you open the settings page, the settings page for the models?",
                "Can you open the settings page for the models?",
            ),
            ("I think we need to, we need to talk.", "I think we need to talk."),
            ("Run the tests, run the unit tests.", "Run the unit tests."),
            (
                "Fix it.\nThe readme, the main readme file.",
                "Fix it.\nThe main readme file.",
            ),
        ] {
            assert_eq!(run(said), meant, "{said}");
        }
    }

    #[test]
    fn deliberate_repetition_and_lists_are_kept() {
        for text in [
            "He had had enough, very very much.",
            "Merge the latest branch and the master branch into this one.",
            "Check the logs, the metrics and the traces.",
            "Thank you, thank you so much for coming.",
            "We tested it on Tuesday, on Wednesday and on Thursday.",
            "Send it to the team, the one that owns billing, by Friday.",
            "It is a long, long road ahead.",
            "No, no, that is not what I meant.",
            "Run the unit tests, then run the integration tests.",
            "Run the unit tests, run the integration tests.",
            "Feed the cats, the dogs and cats.",
            "Ask the team, the team's lead knows.",
            "I know you, I know you.",
            "go go go go go go go",
            "um uh um uh um uh um uh",
            "we need to we need to talk",
            "Run the tests. Run the integration tests.",
            "The plan, the outline of the plan.",
            "Read it, then read it again.",
            "",
        ] {
            assert_eq!(run(text), text, "{text}");
        }
    }

    #[test]
    fn counts_each_restart() {
        let mut edits = Edits::default();
        assert_eq!(
            apply(
                "Open the report, the quarterly report and the doc, the shared doc",
                &mut edits
            ),
            "Open the quarterly report and the shared doc"
        );
        assert_eq!(edits.backtracks, 2);
    }
}
