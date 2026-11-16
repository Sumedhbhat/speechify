//! American to Indian (British) spelling.

use super::Edits;
use crate::settings::Spelling;
use regex::Regex;
use std::sync::LazyLock;

/// Stems whose -ize/-yze family takes an s: organize, organizing, organization.
const IZE: &str = "organ|real|recogn|apolog|priorit|final|util|optim|custom|summar|emphas|minim|maxim|author|categor|critic|general|special|standard|normal|visual|initial|synchron|memor|modern|central|character|capital|familiar|personal|local|monet|legal|mobil|stabil|neutral|anal|paral|digit|global|social|formal|ital|materi|hospital|civil|subsid";

/// Whole words; `|` separates the American form from the Indian one.
const WORDS: &[&str] = &[
    "color|colour",
    "colors|colours",
    "colored|coloured",
    "coloring|colouring",
    "colorful|colourful",
    "favor|favour",
    "favors|favours",
    "favorite|favourite",
    "favorites|favourites",
    "favorable|favourable",
    "honor|honour",
    "honors|honours",
    "honored|honoured",
    "labor|labour",
    "neighbor|neighbour",
    "neighbors|neighbours",
    "neighborhood|neighbourhood",
    "behavior|behaviour",
    "behaviors|behaviours",
    "flavor|flavour",
    "flavors|flavours",
    "humor|humour",
    "rumor|rumour",
    "rumors|rumours",
    "harbor|harbour",
    "endeavor|endeavour",
    "center|centre",
    "centers|centres",
    "centered|centred",
    "theater|theatre",
    "theaters|theatres",
    "liter|litre",
    "liters|litres",
    "kilometer|kilometre",
    "kilometers|kilometres",
    "centimeter|centimetre",
    "centimeters|centimetres",
    "millimeter|millimetre",
    "fiber|fibre",
    "defense|defence",
    "offense|offence",
    "catalog|catalogue",
    "dialog|dialogue",
    "gray|grey",
    "traveling|travelling",
    "traveled|travelled",
    "traveler|traveller",
    "travelers|travellers",
    "canceled|cancelled",
    "canceling|cancelling",
    "modeling|modelling",
    "modeled|modelled",
    "labeled|labelled",
    "labeling|labelling",
    "fulfill|fulfil",
    "enrollment|enrolment",
    "aluminum|aluminium",
    "jewelry|jewellery",
    "skeptical|sceptical",
    "aging|ageing",
    "pajamas|pyjamas",
    "math|maths",
    "cozy|cosy",
    "plow|plough",
    "check-in|check-in",
    "counselor|counsellor",
    "pretense|pretence",
    "maneuver|manoeuvre",
    "tumor|tumour",
    "savior|saviour",
    "vapor|vapour",
    "odor|odour",
    "rigor|rigour",
    "vigor|vigour",
    "armor|armour",
    "caliber|calibre",
    "somber|sombre",
    "specter|spectre",
    "luster|lustre",
    "meager|meagre",
    "program|program",
    "inquiry|enquiry",
    "licorice|liquorice",
    "mold|mould",
    "smolder|smoulder",
    "story|story",
    "mustache|moustache",
    "tire|tire",
    "curb|curb",
    "draft|draft",
    "artifact|artefact",
    "artifacts|artefacts",
    "acknowledgment|acknowledgement",
    "judgment|judgement",
    "dreamed|dreamt",
    "learned|learnt",
    "spelled|spelt",
    "burned|burnt",
    "towards|towards",
    "toward|towards",
    "afterward|afterwards",
    "grayscale|greyscale",
    "installment|instalment",
    "installments|instalments",
    "skillful|skilful",
    "willful|wilful",
    "marvelous|marvellous",
    "fueled|fuelled",
    "fueling|fuelling",
    "signaling|signalling",
    "signaled|signalled",
    "leveled|levelled",
    "quarreled|quarrelled",
    "woolen|woollen",
    "paycheck|paycheque",
    "practicing|practising",
    "practiced|practised",
    "licensing|licensing",
    "mom|mom",
    "anesthesia|anaesthesia",
    "pediatric|paediatric",
    "pediatrician|paediatrician",
    "estrogen|oestrogen",
    "diarrhea|diarrhoea",
    "fetus|foetus",
    "encyclopedia|encyclopaedia",
    "orthopedic|orthopaedic",
    "leukemia|leukaemia",
];

static MATCHER: LazyLock<Regex> = LazyLock::new(|| {
    let words: Vec<&str> = WORDS.iter().map(|pair| pair.split('|').next().unwrap()).collect();
    Regex::new(&format!(
        r"(?i)\b(?:((?:{IZE})(?:iz|yz)(?:e|es|ed|ing|er|ers|ation|ations))|({}))\b",
        words.join("|")
    ))
    .unwrap()
});

fn match_case(original: &str, replacement: &str) -> String {
    if original.len() > 1 && original.chars().all(|c| !c.is_lowercase()) {
        replacement.to_uppercase()
    } else if original.chars().next().is_some_and(char::is_uppercase) {
        super::tidy::capitalize(replacement)
    } else {
        replacement.to_string()
    }
}

pub fn apply(text: &str, spelling: Spelling, edits: &mut Edits) -> String {
    if spelling == Spelling::American {
        return text.to_string();
    }
    let mut count = 0;
    let out = MATCHER.replace_all(text, |caps: &regex::Captures| {
        let found = &caps[0];
        let lower = found.to_lowercase();
        let fixed = if caps.get(1).is_some() {
            // The last z before the suffix: "organization" → "organisation".
            let at = lower.rfind('z').unwrap();
            format!("{}s{}", &lower[..at], &lower[at + 1..])
        } else {
            let pair = WORDS
                .iter()
                .find(|pair| pair.split('|').next() == Some(lower.as_str()))
                .unwrap();
            pair.split('|').nth(1).unwrap().to_string()
        };
        if fixed != lower {
            count += 1;
        }
        match_case(found, &fixed)
    });
    edits.spellings += count;
    out.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> String {
        apply(text, Spelling::Indian, &mut Edits::default())
    }

    #[test]
    fn converts_common_families() {
        assert_eq!(run("My favorite color is gray."), "My favourite colour is grey.");
        assert_eq!(
            run("Please organize and prioritize the organization's work."),
            "Please organise and prioritise the organisation's work."
        );
        assert_eq!(
            run("We analyzed it while traveling to the center."),
            "We analysed it while travelling to the centre."
        );
    }

    #[test]
    fn keeps_case() {
        assert_eq!(run("Color COLOR color"), "Colour COLOUR colour");
        assert_eq!(run("Realize it"), "Realise it");
    }

    #[test]
    fn leaves_other_words_and_american_users_alone() {
        assert_eq!(
            run("The size of the prize, a program and a colorado trip."),
            "The size of the prize, a program and a colorado trip."
        );
        let mut edits = Edits::default();
        assert_eq!(apply("color", Spelling::American, &mut edits), "color");
        apply("color and program", Spelling::Indian, &mut edits);
        assert_eq!(edits.spellings, 1);
    }
}
