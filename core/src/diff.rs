//! Word alignment between two texts, for accuracy scores and learning.

/// Lowercase, without surrounding punctuation: how words are compared.
pub fn normalize(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

pub fn words(text: &str) -> Vec<&str> {
    text.split_whitespace()
        .filter(|word| !normalize(word).is_empty())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op<'a> {
    Same(&'a str, &'a str),
    /// Words in the first text replaced by words in the second.
    Change(Vec<&'a str>, Vec<&'a str>),
}

/// Aligns two texts word by word and groups neighbouring differences.
pub fn align<'a>(before: &'a str, after: &'a str) -> Vec<Op<'a>> {
    let (a, b) = (words(before), words(after));
    let (na, nb): (Vec<String>, Vec<String>) = (
        a.iter().map(|w| normalize(w)).collect(),
        b.iter().map(|w| normalize(w)).collect(),
    );
    let width = b.len() + 1;
    let mut cost = vec![0u32; (a.len() + 1) * width];
    for i in 0..=a.len() {
        for j in 0..=b.len() {
            cost[i * width + j] = if i == 0 {
                j as u32
            } else if j == 0 {
                i as u32
            } else {
                let change = cost[(i - 1) * width + j - 1] + u32::from(na[i - 1] != nb[j - 1]);
                change
                    .min(cost[(i - 1) * width + j] + 1)
                    .min(cost[i * width + j - 1] + 1)
            };
        }
    }
    let mut ops: Vec<Op> = Vec::new();
    let push_change = |ops: &mut Vec<Op<'a>>, from: Option<&'a str>, to: Option<&'a str>| {
        if !matches!(ops.last(), Some(Op::Change(..))) {
            ops.push(Op::Change(vec![], vec![]));
        }
        if let Some(Op::Change(left, right)) = ops.last_mut() {
            left.extend(from);
            right.extend(to);
        }
    };
    let (mut i, mut j) = (a.len(), b.len());
    while i > 0 || j > 0 {
        let here = cost[i * width + j];
        if i > 0 && j > 0 && na[i - 1] == nb[j - 1] && here == cost[(i - 1) * width + j - 1] {
            ops.push(Op::Same(a[i - 1], b[j - 1]));
            (i, j) = (i - 1, j - 1);
        } else if i > 0 && j > 0 && here == cost[(i - 1) * width + j - 1] + 1 {
            push_change(&mut ops, Some(a[i - 1]), Some(b[j - 1]));
            (i, j) = (i - 1, j - 1);
        } else if i > 0 && here == cost[(i - 1) * width + j] + 1 {
            push_change(&mut ops, Some(a[i - 1]), None);
            i -= 1;
        } else {
            push_change(&mut ops, None, Some(b[j - 1]));
            j -= 1;
        }
    }
    ops.reverse();
    for op in &mut ops {
        if let Op::Change(left, right) = op {
            left.reverse();
            right.reverse();
        }
    }
    ops
}

/// Word error rate of `hypothesis` against `reference`, from 0 upward.
pub fn wer(reference: &str, hypothesis: &str) -> f64 {
    let total = words(reference).len();
    let errors: usize = align(reference, hypothesis)
        .iter()
        .map(|op| match op {
            Op::Same(..) => 0,
            Op::Change(left, right) => left.len().max(right.len()),
        })
        .sum();
    if total == 0 {
        return if errors == 0 { 0.0 } else { 1.0 };
    }
    errors as f64 / total as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_and_groups_changes() {
        assert_eq!(
            align("Ask Sumit Bat about it.", "Ask Sumedh Bhat about it today"),
            vec![
                Op::Same("Ask", "Ask"),
                Op::Change(vec!["Sumit", "Bat"], vec!["Sumedh", "Bhat"]),
                Op::Same("about", "about"),
                Op::Same("it.", "it"),
                Op::Change(vec![], vec!["today"]),
            ]
        );
        assert_eq!(
            align("a b c", "a c"),
            vec![Op::Same("a", "a"), Op::Change(vec!["b"], vec![]), Op::Same("c", "c")]
        );
        assert_eq!(align("", ""), vec![]);
    }

    #[test]
    fn word_error_rate() {
        assert_eq!(wer("the quick brown fox", "The quick, brown fox!"), 0.0);
        assert_eq!(wer("the quick brown fox", "the quick red fox"), 0.25);
        assert_eq!(wer("one two", "one"), 0.5);
        assert_eq!(wer("", ""), 0.0);
        assert_eq!(wer("", "extra"), 1.0);
        assert_eq!(wer("a", "b c d"), 3.0);
        assert_eq!(words(" - hi -- there "), vec!["hi", "there"]);
    }
}
