use super::Edits;

const LONGEST: usize = 6;
const INSERTED: usize = 2;
const EXACT: usize = 3;
const JOINERS: &[&str] = &["and", "or", "but", "nor", "then", "than", "vs", "versus"];
const OTHERS: &[&str] = &["other", "another", "second", "next", "different", "wrong", "same"];
const VERBS: &[&str] = &[
    "is", "are", "was", "were", "am", "be", "been", "can", "could", "will", "would", "should", "shall", "must", "may",
    "might", "has", "have", "had", "do", "does", "did", "stays", "needs", "wants", "looks", "seems", "goes", "works",
];

fn core(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

fn pauses(word: &str) -> bool {
    word.ends_with([',', '.', '!', '?', ';', ':'])
}

fn negates(word: &str) -> bool {
    let word = core(word).replace('’', "'");
    matches!(word.as_str(), "not" | "no" | "never" | "nor") || word.ends_with("n't")
}

fn restated(left: &[String], right: &[String]) -> Option<usize> {
    let (first, last) = (core(&left[0]), core(&left[left.len() - 1]));
    if right.is_empty() || core(&right[0]) != first || left[..left.len() - 1].iter().any(|w| pauses(w)) {
        return None;
    }
    let exact = left.len() >= EXACT
        && right.len() > left.len()
        && left.iter().zip(right).all(|(a, b)| core(a) == core(b))
        && !pauses(&right[left.len() - 1]);
    if exact {
        return Some(left.len());
    }
    let mut at = 1;
    let mut inserted = 0;
    for wanted in &left[1..] {
        let wanted = core(wanted);
        loop {
            let word = right.get(at)?;
            if pauses(word) && core(word) != wanted {
                return None;
            }
            at += 1;
            if core(word) == wanted {
                break;
            }
            let plain = core(word);
            if JOINERS.contains(&plain.as_str())
                || OTHERS.contains(&plain.as_str())
                || negates(word)
                || word.ends_with('\'')
                || word.contains("'s")
            {
                return None;
            }
            inserted += 1;
            if inserted > INSERTED {
                return None;
            }
        }
    }
    (inserted > 0 && core(&right[at - 1]) == last && first != last).then_some(at)
}

fn restarts(words: &[String], at: usize) -> Option<usize> {
    if !words[at - 1].ends_with(',') {
        return None;
    }
    (2..=LONGEST.min(at)).find(|&len| {
        let start = at - len;
        let follows = |said: usize| {
            words
                .get(at + said)
                .is_some_and(|next| negates(next) || VERBS.contains(&core(next).as_str()))
        };
        (start == 0 || !negates(&words[start - 1]))
            && restated(&words[start..at], &words[at..]).is_some_and(|said| !follows(said))
    })
}

pub fn apply(text: &str, edits: &mut Edits) -> String {
    let mut changed = false;
    let lines: Vec<String> = text
        .split('\n')
        .map(|line| {
            let mut words: Vec<String> = line.split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect();
            let mut at = 1;
            while at < words.len() {
                match restarts(&words, at) {
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
            "Let's merge the latest branch latest master branch into this branch",
            "Don't delete the branch, delete the remote branch.",
            "Don't restart the server, restart the dev server.",
            "Not the button, the blue button.",
            "Not this file, this other file.",
            "We know it works, we know it never works on Windows.",
            "It is working, it is not working.",
            "You know you don't know.",
            "I fixed the bug, the other bug is still open.",
            "Call the client, the new client wants a demo.",
            "Book the room, the big room is taken.",
            "Close the tab, the other tab stays open.",
            "Merge the PR, the second PR can wait.",
            "I said the date, the due date is Monday.",
            "Ship the fix, the fix isn't ready.",
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
