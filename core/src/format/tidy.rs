//! Whitespace, punctuation spacing and sentence capitals.

/// Collapses runs of spaces and trims; line breaks survive.
pub fn collapse(text: &str) -> String {
    let lines: Vec<String> = text
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    lines.join("\n").trim().to_string()
}

pub fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Repairs what earlier passes leave behind: stray spaces before
/// punctuation, doubled marks, lowercase sentence starts.
pub fn tidy(text: &str) -> String {
    let text = collapse(text);
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, ',' | '.' | ';' | ':' | '!' | '?') {
            while out.ends_with(' ') {
                out.pop();
            }
            // "milk,." keeps the stronger mark; "?." keeps the first.
            match out.chars().last() {
                Some(',') | Some(';') if ch != ',' && ch != ';' => {
                    out.pop();
                }
                Some(',') | Some(';') => continue,
                Some('!') | Some('?') | Some(':') if ch != '!' && ch != '?' => continue,
                Some('\n') | None if ch != '.' => continue,
                _ => {}
            }
        }
        out.push(ch);
    }
    // Dropping a leading mark can leave the space that followed it.
    let out = collapse(&out);
    let mut result = String::with_capacity(out.len());
    let mut start = true;
    let chars: Vec<char> = out.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if start && ch.is_alphabetic() {
            result.extend(ch.to_uppercase());
            start = false;
            continue;
        }
        if ch.is_alphanumeric() {
            start = false;
        }
        if ch == '\n' {
            start = true;
        }
        // A full stop ends a sentence only before a space: "3.5" and "a.m." do not.
        if matches!(ch, '.' | '!' | '?') && chars.get(i + 1).is_none_or(|next| next.is_whitespace()) {
            start = true;
        }
        result.push(ch);
    }
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_spaces_but_keeps_lines() {
        assert_eq!(collapse("  a   b \n  c  "), "a b\nc");
    }

    #[test]
    fn capitalizes_sentences_and_lines() {
        assert_eq!(
            tidy("hello there. how are you?\nfine"),
            "Hello there. How are you?\nFine"
        );
        assert_eq!(tidy("version 3.5 is out"), "Version 3.5 is out");
        assert_eq!(capitalize(""), "");
    }

    #[test]
    fn repairs_punctuation_left_by_other_passes() {
        assert_eq!(tidy("so , I went .  home"), "So, I went. Home");
        assert_eq!(tidy("milk,. eggs"), "Milk. Eggs");
        assert_eq!(tidy("milk,, eggs;, tea"), "Milk, eggs; tea");
        assert_eq!(tidy("really?. yes!, ok:."), "Really? Yes! Ok:");
        assert_eq!(tidy(", well\n, then"), "Well\nThen");
        assert_eq!(tidy("1. first"), "1. First");
    }
}
