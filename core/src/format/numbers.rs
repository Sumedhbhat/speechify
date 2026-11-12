//! Spoken numbers to digits, with Indian lakh/crore grouping.

use super::Edits;
use crate::settings::NumberSystem;
use regex::Regex;
use std::sync::LazyLock;

const UNITS: &[&str] = &[
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];
const TENS: &[&str] = &[
    "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];
const ORDINALS: &[&str] = &[
    "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth",
];
const UNIT_WORDS: &[&str] = &["percent", "rupees", "rupee", "dollars", "dollar"];

fn scale(word: &str) -> Option<u64> {
    Some(match word {
        "thousand" => 1_000,
        "lakh" | "lakhs" | "lac" | "lacs" => 100_000,
        "million" => 1_000_000,
        "crore" | "crores" => 10_000_000,
        "billion" => 1_000_000_000,
        _ => return None,
    })
}

fn small(word: &str) -> Option<u64> {
    let units = UNITS.iter().position(|u| *u == word).map(|i| i as u64);
    units.or_else(|| TENS.iter().position(|t| *t == word).map(|i| (i as u64 + 2) * 10))
}

/// "hundred", "thousand", "lakh": words that scale the number before them.
pub fn is_multiplier(word: &str) -> bool {
    word == "hundred" || scale(word).is_some()
}

pub fn is_number_word(word: &str) -> bool {
    small(word).is_some() || word == "hundred" || scale(word).is_some()
}

fn core(word: &str) -> String {
    word.trim_end_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

fn trailing(word: &str) -> &str {
    &word[word.trim_end_matches(|c: char| !c.is_alphanumeric()).len()..]
}

fn digits(word: &str) -> Option<u64> {
    let plain = word.replace(',', "");
    (plain.len() <= 12 && !plain.is_empty() && plain.bytes().all(|b| b.is_ascii_digit()))
        .then(|| plain.parse().unwrap())
}

/// Groups an integer's digits: 250000 is "2,50,000" in India and "250,000" elsewhere.
/// Four-digit numbers stay bare so years and PINs are not split.
pub fn group_digits(value: u64, system: NumberSystem) -> String {
    let plain = value.to_string();
    if plain.len() < 5 {
        return plain;
    }
    let (head, last) = plain.split_at(plain.len() - 3);
    let size = if system == NumberSystem::Indian { 2 } else { 3 };
    let mut groups: Vec<&str> = Vec::new();
    let mut end = head.len();
    while end > 0 {
        let start = end.saturating_sub(size);
        groups.push(&head[start..end]);
        end = start;
    }
    groups.reverse();
    format!("{},{last}", groups.join(","))
}

static GROUPED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d{1,3}(?:,\d{2,3})+\b").unwrap());
static PERCENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(\d(?:\.\d+)?)\s+percent\b").unwrap());
static MONEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(\d[\d,]*(?:\.\d+)?(?:\s+(?:thousand|lakh|crore|million|billion))?)\s+(rupees?|dollars?)\b")
        .unwrap()
});

/// Rewrites digit groups a model or recogniser produced in the other system.
pub fn regroup(text: &str, system: NumberSystem) -> String {
    GROUPED
        .replace_all(text, |caps: &regex::Captures| match digits(&caps[0]) {
            Some(value) => group_digits(value, system),
            None => caps[0].to_string(),
        })
        .into_owned()
}

/// One parsed run of number words starting at a token.
struct Run {
    value: u64,
    tokens: usize,
    /// Exactly "<n> lakh" and the like: kept in words, as people write it.
    short_scale: Option<&'static str>,
    words_only: bool,
    plain: bool,
}

fn parse(words: &[String]) -> Option<Run> {
    let mut total = 0u64;
    let mut current: Option<u64> = None;
    let (mut last_scale, mut after_tens, mut after_hundred) = (u64::MAX, false, false);
    let (mut tokens, mut words_only, mut plain, mut scales) = (0, true, true, 0);
    let mut short_scale = None;
    for (i, raw) in words.iter().enumerate() {
        let word = core(raw);
        let next = words.get(i + 1).map(|w| core(w)).unwrap_or_default();
        let word_value = small(&word);
        let value = word_value.or_else(|| digits(&word));
        if let Some(v) = value {
            match current {
                None => current = Some(v),
                Some(c) if after_tens && v < 10 && word_value.is_some() => current = Some(c + v),
                Some(c) if after_hundred && v < 100 => current = Some(c + v),
                Some(_) => break,
            }
            words_only &= word_value.is_some();
            after_tens = word_value.is_some_and(|v| v >= 20);
            after_hundred = false;
        } else if word == "hundred" && current.is_some_and(|c| c < 100) {
            current = current.map(|c| c * 100);
            (after_tens, after_hundred, plain) = (false, true, false);
        } else if let (Some(s), Some(c)) = (scale(&word).filter(|s| *s < last_scale), current) {
            total += c * s;
            current = None;
            last_scale = s;
            scales += 1;
            (after_tens, after_hundred, plain) = (false, false, false);
            short_scale = (scales == 1 && tokens == 1 && s != 1_000 && c < 1_000).then_some(match s {
                100_000 => "lakh",
                1_000_000 => "million",
                10_000_000 => "crore",
                _ => "billion",
            });
        } else if word == "and"
            && (after_hundred || (current.is_none() && tokens > 0))
            && (small(&next).is_some() || digits(&next).is_some())
            && trailing(raw).is_empty()
        {
            tokens += 1;
            continue;
        } else {
            break;
        }
        tokens += 1;
        // Punctuation ends the number: "two, three" is a list, not 5.
        if !trailing(raw).is_empty() {
            break;
        }
    }
    if tokens == 0 {
        return None;
    }
    if current.is_some() {
        short_scale = None;
    }
    Some(Run {
        value: total + current.unwrap_or(0),
        tokens,
        short_scale,
        words_only,
        plain,
    })
}

fn convert_line(line: &str, system: NumberSystem, edits: &mut Edits) -> String {
    let words: Vec<String> = line
        .split(' ')
        .map(|w| w.replace('-', " "))
        .flat_map(|w| {
            // "twenty-five" parses as two words; other hyphenated words stay whole.
            let parts: Vec<String> = w.split(' ').map(str::to_string).collect();
            if parts.len() > 1 && parts.iter().all(|p| small(&core(p)).is_some()) {
                parts
            } else {
                vec![w.replace(' ', "-")]
            }
        })
        .collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let Some(run) = parse(&words[i..]) else {
            out.push(words[i].clone());
            i += 1;
            continue;
        };
        let last = &words[i + run.tokens - 1];
        let mark = trailing(last).to_string();
        let next = if mark.is_empty() {
            words.get(i + run.tokens).map(|w| core(w)).unwrap_or_default()
        } else {
            String::new()
        };
        // A spoken digit string: phone numbers, OTPs, PIN codes.
        let single = |w: &String| small(&core(w)).filter(|v| *v < 10);
        let string_len = words[i..]
            .iter()
            .enumerate()
            .take_while(|(n, w)| single(w).is_some() && (*n == 0 || trailing(&words[i + n - 1]).is_empty()))
            .count();
        if string_len >= 4 {
            let joined: String = words[i..i + string_len]
                .iter()
                .map(|w| single(w).unwrap().to_string())
                .collect();
            out.push(format!("{joined}{}", trailing(&words[i + string_len - 1])));
            edits.numbers += 1;
            i += string_len;
            continue;
        }
        // "three point five" → 3.5
        let decimals: Vec<String> = if next == "point" && run.short_scale.is_none() {
            words[i + run.tokens + 1..]
                .iter()
                .scan(true, |open, w| {
                    let digit = single(w).filter(|_| *open)?;
                    *open = trailing(w).is_empty();
                    Some(digit.to_string())
                })
                .collect()
        } else {
            vec![]
        };
        let is_digits = !run.words_only && run.tokens == 1 && decimals.is_empty();
        let lone_small = run.tokens == 1 && run.value < 10;
        let keep = is_digits
            || ORDINALS.contains(&next.as_str())
            || (lone_small && !UNIT_WORDS.contains(&next.as_str()) && decimals.is_empty() && scale(&next).is_none());
        if keep {
            out.extend_from_slice(&words[i..i + run.tokens]);
            i += run.tokens;
            continue;
        }
        let mut rendered = match run.short_scale {
            Some(name) => format!("{} {name}", run.value / scale(name).unwrap()),
            None => group_digits(run.value, system),
        };
        let mut used = run.tokens;
        // "nineteen ninety nine", "twenty twenty six": a year said as two pairs.
        if run.plain && run.words_only && (18..=21).contains(&run.value) && mark.is_empty() {
            if let Some(second) =
                parse(&words[i + used..]).filter(|r| r.plain && r.words_only && (10..100).contains(&r.value))
            {
                rendered = format!("{}{}", run.value, second.value);
                used += second.tokens;
            }
        }
        if !decimals.is_empty() && used == run.tokens {
            rendered = format!("{}.{}", run.value, decimals.concat());
            used += 1 + decimals.len();
        }
        out.push(format!("{rendered}{}", trailing(&words[i + used - 1])));
        edits.numbers += 1;
        i += used;
    }
    out.join(" ")
}

pub fn apply(text: &str, system: NumberSystem, edits: &mut Edits) -> String {
    let lines: Vec<String> = text.split('\n').map(|line| convert_line(line, system, edits)).collect();
    let joined = lines.join("\n");
    let text = PERCENT.replace_all(&joined, "$1%");
    let text = MONEY.replace_all(&text, |caps: &regex::Captures| {
        let symbol = if caps[2].to_lowercase().starts_with('r') { "₹" } else { "$" };
        format!("{symbol}{}", &caps[1])
    });
    regroup(&text, system)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> String {
        apply(text, NumberSystem::Indian, &mut Edits::default())
    }

    #[test]
    fn groups_in_both_systems() {
        assert_eq!(group_digits(2024, NumberSystem::Indian), "2024");
        assert_eq!(group_digits(250000, NumberSystem::Indian), "2,50,000");
        assert_eq!(group_digits(12345678, NumberSystem::Indian), "1,23,45,678");
        assert_eq!(group_digits(12345678, NumberSystem::International), "12,345,678");
        assert_eq!(
            regroup("It cost 250,000 and 1,200.", NumberSystem::Indian),
            "It cost 2,50,000 and 1200."
        );
        assert_eq!(
            regroup("It cost 2,50,000.", NumberSystem::International),
            "It cost 250,000."
        );
        assert_eq!(
            regroup("1,234,567,890,123,456", NumberSystem::Indian),
            "1,234,567,890,123,456"
        );
    }

    #[test]
    fn converts_spoken_numbers() {
        assert_eq!(run("twenty five people"), "25 people");
        assert_eq!(run("twenty-five well-known people"), "25 well-known people");
        assert_eq!(run("one hundred and fifty items"), "150 items");
        assert_eq!(run("two lakh fifty thousand"), "2,50,000");
        assert_eq!(run("three crore twenty lakh"), "3,20,00,000");
        assert_eq!(run("fifty thousand, maybe more"), "50,000, maybe more");
        assert_eq!(run("2 lakh 50 thousand"), "2,50,000");
        assert_eq!(run("one thousand and five"), "1005");
    }

    #[test]
    fn keeps_round_lakh_and_crore_in_words() {
        assert_eq!(run("five lakh"), "5 lakh");
        assert_eq!(run("twelve crores"), "12 crore");
        assert_eq!(
            run("two million users and three billion rows"),
            "2 million users and 3 billion rows"
        );
        assert_eq!(run("two point five lakh"), "2.5 lakh");
    }

    #[test]
    fn leaves_small_and_ordinary_numbers_alone() {
        assert_eq!(run("one of the two options"), "one of the two options");
        assert_eq!(run("the twenty first century"), "the twenty first century");
        assert_eq!(run("in 2024 we had 15 people"), "in 2024 we had 15 people");
        assert_eq!(
            run("a hundred times and thousand thanks"),
            "a hundred times and thousand thanks"
        );
        assert_eq!(run("one, two, three"), "one, two, three");
        assert_eq!(run("bread and butter"), "bread and butter");
    }

    #[test]
    fn years_decimals_and_digit_strings() {
        assert_eq!(run("in nineteen ninety nine"), "in 1999");
        assert_eq!(run("by twenty twenty six."), "by 2026.");
        assert_eq!(run("version three point one four"), "version 3.14");
        assert_eq!(run("three point of view"), "three point of view");
        assert_eq!(run("call nine eight four five zero one"), "call 984501");
        assert_eq!(run("ten twenty"), "10 20");
        assert_eq!(run("twenty, twenty six"), "20, 26");
    }

    #[test]
    fn units_and_currency() {
        assert_eq!(run("five percent"), "5%");
        assert_eq!(run("12.5 percent growth"), "12.5% growth");
        assert_eq!(run("five hundred rupees"), "₹500");
        assert_eq!(run("3 lakh rupees"), "₹3 lakh");
        assert_eq!(run("twenty dollars."), "$20.");
        assert_eq!(run("one rupee"), "₹1");
    }

    #[test]
    fn counts_conversions_and_keeps_lines() {
        let mut edits = Edits::default();
        let out = apply("twenty\nthirty one", NumberSystem::International, &mut edits);
        assert_eq!(out, "20\n31");
        assert_eq!(edits.numbers, 2);
        assert!(is_number_word("lakh") && is_number_word("hundred") && !is_number_word("many"));
    }
}
