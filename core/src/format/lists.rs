//! Turns spoken enumerations into numbered or bulleted lists.

use super::{tidy::capitalize, Edits};

const ORDINALS: &[&[&str]] = &[
    &["first", "firstly"],
    &["second", "secondly"],
    &["third", "thirdly"],
    &["fourth", "fourthly"],
    &["fifth"],
    &["sixth"],
    &["seventh"],
    &["eighth"],
    &["ninth"],
    &["tenth"],
];
const CARDINALS: &[&str] = &[
    "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
];
/// Words that introduce a marker and carry nothing: "and second", "number two".
const LEADS: &[&str] = &["and", "then", "number", "point", "also"];

fn core(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

fn ends_clause(word: &str) -> bool {
    word.ends_with(['.', ',', ';', ':', '!', '?'])
}

#[derive(Clone, Copy)]
struct Marker {
    /// First token of the marker including lead words.
    start: usize,
    /// First token of the item text.
    end: usize,
    value: usize,
    /// Set off by punctuation or an ordinal, so it is not an ordinary number.
    firm: bool,
}

fn marker_at(words: &[&str], at: usize) -> Option<Marker> {
    let word = words[at];
    let bare = core(word);
    let ordinal = ORDINALS.iter().position(|names| names.contains(&bare.as_str()));
    let cardinal = CARDINALS.iter().position(|name| *name == bare).or_else(|| {
        bare.parse::<usize>()
            .ok()
            .filter(|n| (1..=20).contains(n) && bare.len() <= 2)
            .map(|n| n - 1)
    });
    let value = ordinal.or(cardinal)? + 1;
    let mut start = at;
    while start > 0 && LEADS.contains(&core(words[start - 1]).as_str()) && !ends_clause(words[start - 1]) {
        start -= 1;
    }
    let led = start < at
        && words[start..at]
            .iter()
            .any(|w| matches!(core(w).as_str(), "number" | "point"));
    let at_boundary = start == 0 || ends_clause(words[start - 1]);
    let marked = word.ends_with([',', '.', ':', ')']);
    if ordinal.is_some() && !(at_boundary || marked) {
        // "the first time" is not a list.
        return None;
    }
    let firm = led
        || (at_boundary && (marked || ordinal.is_some()))
        || (marked && cardinal.is_some() && word.ends_with(['.', ')', ':']));
    let mut end = at + 1;
    // "first of all", "first is"
    if words.get(end).map(|w| core(w)) == Some("of".into()) && words.get(end + 1).map(|w| core(w)) == Some("all".into())
    {
        end += 2;
    }
    Some(Marker {
        start,
        end,
        value,
        firm,
    })
}

fn clean_item(words: &[&str]) -> String {
    let text = words.join(" ");
    let text = text
        .trim()
        .trim_end_matches(['.', ',', ';'])
        .trim_start_matches([',', ':', '.', ' ']);
    capitalize(text.trim())
}

fn numbered(words: &[&str], loose_needed: usize) -> Option<String> {
    let all: Vec<Marker> = (0..words.len()).filter_map(|i| marker_at(words, i)).collect();
    let first = all.iter().position(|m| m.value == 1)?;
    let mut chain = vec![all[first]];
    for marker in &all[first + 1..] {
        let last = chain.last().unwrap();
        if marker.value == last.value + 1 && marker.start >= last.end {
            chain.push(*marker);
        }
    }
    // Loose markers ("one buy milk two call mom three…") need a third to be believed.
    let needed = if chain.iter().all(|m| m.firm) { 2 } else { loose_needed };
    if chain.len() < needed {
        return None;
    }
    let mut items = Vec::new();
    let mut outro = String::new();
    for (i, marker) in chain.iter().enumerate() {
        let stop = chain.get(i + 1).map_or(words.len(), |next| next.start);
        let mut body = &words[marker.end..stop];
        if i + 1 == chain.len() {
            // Text after the last item's sentence is ordinary prose again,
            // provided the earlier items were single sentences too.
            let single = items.iter().all(|item: &String| !item.contains(". "));
            if let Some(cut) = body
                .iter()
                .position(|w| w.ends_with(['.', '!', '?']))
                .filter(|cut| single && cut + 1 < body.len())
            {
                outro = body[cut + 1..].join(" ");
                body = &body[..=cut];
            }
        }
        let item = clean_item(body);
        if item.is_empty() {
            return None;
        }
        items.push(item);
    }
    let intro = words[..chain[0].start].join(" ");
    Some(assemble(
        &intro,
        items
            .iter()
            .enumerate()
            .map(|(i, item)| format!("{}. {item}", i + 1))
            .collect(),
        &outro,
    ))
}

fn bulleted(words: &[&str]) -> Option<String> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let here = core(words[i]);
        let next = words.get(i + 1).map(|w| core(w)).unwrap_or_default();
        if (here == "bullet" && next == "point") || (here == "next" && next == "bullet") {
            starts.push((i, i + 2));
            i += 2;
        } else if here == "bullet" {
            starts.push((i, i + 1));
            i += 1;
        } else {
            i += 1;
        }
    }
    if starts.len() < 2 {
        return None;
    }
    let mut items = Vec::new();
    for (n, (_, begin)) in starts.iter().enumerate() {
        let stop = starts.get(n + 1).map_or(words.len(), |next| next.0);
        let item = clean_item(&words[*begin..stop]);
        if item.is_empty() {
            return None;
        }
        items.push(format!("- {item}"));
    }
    Some(assemble(&words[..starts[0].0].join(" "), items, ""))
}

fn assemble(intro: &str, items: Vec<String>, outro: &str) -> String {
    let mut out = String::new();
    let intro = intro.trim().trim_end_matches([',', ':', '.']).trim();
    if !intro.is_empty() {
        out.push_str(&format!("{}:\n", capitalize(intro)));
    }
    out.push_str(&items.join("\n"));
    if !outro.trim().is_empty() {
        out.push_str(&format!("\n\n{}", capitalize(outro.trim())));
    }
    out
}

/// Evidence of an actual spoken enumeration, shared with model vetting.
pub(crate) fn enumerated(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    let introduced = words
        .iter()
        .any(|w| matches!(core(w).as_str(), "things" | "items" | "steps" | "list"));
    numbered(&words, if introduced { 2 } else { 3 })
        .or_else(|| bulleted(&words))
        .is_some()
}

pub fn apply(text: &str, edits: &mut Edits) -> String {
    // A transcript that already has line structure is left as dictated.
    if text.contains('\n') {
        return text.to_string();
    }
    let words: Vec<&str> = text.split(' ').collect();
    match numbered(&words, 3).or_else(|| bulleted(&words)) {
        Some(list) => {
            edits.lists += 1;
            list
        }
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> String {
        apply(text, &mut Edits::default())
    }

    #[test]
    fn cardinal_markers() {
        assert_eq!(
            run("One, buy milk. Two, call mom. Three, pay bills."),
            "1. Buy milk\n2. Call mom\n3. Pay bills"
        );
        assert_eq!(run("1. Buy milk 2. Call mom"), "1. Buy milk\n2. Call mom");
        assert_eq!(
            run("the list is one buy milk two call mom three pay bills"),
            "The list is:\n1. Buy milk\n2. Call mom\n3. Pay bills"
        );
        assert_eq!(run("Number one, speed. Number two, accuracy."), "1. Speed\n2. Accuracy");
    }

    #[test]
    fn ordinal_markers_with_intro_and_outro() {
        assert_eq!(
            run("We need three things. First, a laptop. Second, a charger. And third, a bag. Let me know if that works."),
            "We need three things:\n1. A laptop\n2. A charger\n3. A bag\n\nLet me know if that works."
        );
        assert_eq!(run("First of all, thanks. Secondly, sorry."), "1. Thanks\n2. Sorry");
    }

    #[test]
    fn multi_sentence_items_keep_the_tail() {
        assert_eq!(
            run("First, read it. Then sign. Second, send it. Keep a copy."),
            "1. Read it. Then sign\n2. Send it. Keep a copy"
        );
    }

    #[test]
    fn bullets() {
        assert_eq!(
            run("Agenda: bullet point budget, next bullet hiring, bullet roadmap."),
            "Agenda:\n- Budget\n- Hiring\n- Roadmap"
        );
        assert_eq!(run("bullet one bullet"), "bullet one bullet");
        assert_eq!(run("a bullet train"), "a bullet train");
    }

    #[test]
    fn ordinary_sentences_are_not_lists() {
        for text in [
            "I have one dog and two cats",
            "The first time I saw two of them",
            "One two three testing",
            "First, thank you all.",
            "One, two.",
            "Call me at 1 or 2 pm",
            "line one\nline two",
        ] {
            assert_eq!(run(text), text);
        }
    }

    #[test]
    fn enumeration_evidence_requires_items_not_ordinary_numbers() {
        assert!(enumerated("First, check the invoice. Second, pay the supplier."));
        assert!(enumerated("bullet rice bullet tea"));
        assert!(!enumerated("I have one meeting and two calls today"));
        assert!(!enumerated("The second train arrives after the first"));
    }

    #[test]
    fn counts() {
        let mut edits = Edits::default();
        apply("One, a. Two, b.", &mut edits);
        assert_eq!(edits.lists, 1);
    }
}
