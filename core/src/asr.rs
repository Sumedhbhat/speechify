//! Speech recognition: the trait, a remote implementation, and guards
//! against a recogniser's favourite mistakes.

use crate::audio::encode_wav;
use crate::http::Http;
use crate::Result;
use regex::Regex;
use std::sync::LazyLock;
use std::time::Duration;

/// What the recogniser is told about the speaker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Hint<'a> {
    /// ISO code such as "en"; empty lets the model detect it.
    pub language: &'a str,
    /// Names and terms to prefer.
    pub vocabulary: &'a [String],
}

pub trait Recognizer {
    /// Transcribes 16 kHz mono audio.
    fn transcribe(&mut self, audio: &[f32], hint: &Hint) -> Result<String>;
    fn name(&self) -> &str;
    /// Where the speech is, as sample ranges, judged at `threshold` (a speech
    /// probability), if this recogniser has a voice detector. `None` means
    /// "no detector": the loudness trim is used.
    fn detect(&mut self, _audio: &[f32], _threshold: f64) -> Option<Result<Vec<std::ops::Range<usize>>>> {
        None
    }
    /// The words of the last transcription with the recogniser's confidence
    /// in each. Empty when the recogniser does not report it.
    fn heard(&self) -> Vec<Heard> {
        Vec::new()
    }
}

/// One recognised word and how sure the recogniser was of it, 0 to 1.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Heard {
    pub word: String,
    pub p: f32,
}

/// The part of a recording to recognise. Quiet recordings are first raised
/// to a normal level, so a soft voice reaches the detector and the model as
/// clearly as a loud one. A recording with no signal at all yields nothing.
pub fn isolate(recognizer: &mut dyn Recognizer, samples: &[f32]) -> Result<crate::audio::Trimmed> {
    let Some(loud) = crate::audio::normalize(samples) else { return Ok(crate::audio::Trimmed::default()) };
    let span = crate::audio::loudness_span(samples);
    let Some(found) = recognizer.detect(&loud, crate::audio::DETECTION.threshold) else {
        return Ok(crate::audio::trim(samples));
    };
    let mut regions = found?;
    // A very soft whisper can stay under the detector's usual threshold. If
    // the recording also has sustained sound above its own background, a lower
    // threshold decides: losing someone's words is worse than a stray noise.
    if regions.is_empty() && span.is_some() {
        regions = recognizer
            .detect(&loud, crate::audio::FALLBACK_THRESHOLD)
            .unwrap_or(Ok(Vec::new()))?;
    }
    // The detector decides whether anyone spoke; the cut also keeps the span
    // loudness finds, so soft whispered words at the edges survive.
    Ok(crate::audio::trim_to(&loud, &regions, span))
}

/// Whisper copies the style of its prompt. Without one it sometimes drops
/// capitals and punctuation for a whole dictation, so it always gets this.
pub const STYLE_PROMPT: &str = "Hello. This is a clear, well-punctuated transcript in Indian English.";

/// The prompt given to a recogniser: writing style, then known terms.
pub fn prompt(hint: &Hint) -> String {
    let mut text = if hint.language.is_empty() || hint.language == "en" {
        STYLE_PROMPT.to_string()
    } else {
        String::new()
    };
    if !hint.vocabulary.is_empty() {
        // Whisper's prompt holds about 220 tokens; the newest terms matter most.
        let mut terms: Vec<&str> = Vec::new();
        let mut budget = 400usize;
        for term in hint.vocabulary.iter().rev() {
            if term.len() + 2 > budget {
                break;
            }
            budget -= term.len() + 2;
            terms.push(term);
        }
        terms.reverse();
        text.push_str(&format!(" Glossary: {}.", terms.join(", ")));
    }
    text.trim().to_string()
}

/// Phrases Whisper produces from silence or noise, learned from video captions.
const PHANTOMS: &[&str] = &[
    "thank you",
    "thanks for watching",
    "thank you for watching",
    "you",
    "bye",
    "thanks",
    "thank you very much",
    "please subscribe",
    "subtitles by the amara org community",
    "so",
    "okay",
    "the end",
    "i'm sorry",
    "music",
];

/// The shortest believable time to say one of the phrases above: a fixed
/// start plus time per word (nobody speaks faster than about six words a
/// second). A real "Thank you." said faster than this is dropped along with
/// the invented ones; the two cannot be told apart from text and timing.
///
/// Kept deliberately strict for now (decided 2026-11-13). To let brief
/// replies through later, lower these, or check the voice detector's
/// speech probability for the clip instead of its length.
pub const PHANTOM_BASE_MS: u64 = 350;
pub const PHANTOM_MS_PER_WORD: u64 = 170;

/// True when `text` is one of those phrases and there was too little speech
/// to have said it in earnest.
pub fn is_phantom(text: &str, speech_ms: u64) -> bool {
    let plain: String = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '\'')
        .collect();
    let words = plain.split_whitespace().count() as u64;
    PHANTOMS.contains(&plain.trim()) && speech_ms < PHANTOM_BASE_MS + words * PHANTOM_MS_PER_WORD
}

/// Removes bracketed sound tags ("[BLANK_AUDIO]", "(music)") and tidies spaces.
pub fn strip_tags(text: &str) -> String {
    static TAGS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\[(?:blank_audio|silence|music|noise|applause|laughter|inaudible)\]|\((?:blank_audio|silence|music|noise|applause|laughter|inaudible)\)").unwrap()
    });
    TAGS.replace_all(text, "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A recogniser behind an OpenAI-compatible `/audio/transcriptions` endpoint,
/// for people who run a speech server on another machine.
pub struct Remote {
    http: Http,
    url: String,
    key: String,
    model: String,
}

impl Remote {
    pub fn new(url: &str, key: &str, model: &str) -> Remote {
        // Allow a full 20-minute recording to upload and transcribe on a remote CPU.
        Self::with_timeout(url, key, model, Duration::from_secs(25 * 60))
    }

    /// Sets the complete upload and transcription budget for a speech server.
    pub fn with_timeout(url: &str, key: &str, model: &str, timeout: Duration) -> Remote {
        Remote {
            http: Http::new(Some(timeout)),
            url: url.trim().trim_end_matches('/').to_string(),
            key: key.into(),
            model: model.into(),
        }
    }
}

impl Recognizer for Remote {
    fn transcribe(&mut self, audio: &[f32], hint: &Hint) -> Result<String> {
        const BOUNDARY: &str = "speechify-7f3a9c1e5b2d";
        let mut body = Vec::new();
        let mut field = |name: &str, value: &str| {
            if !value.is_empty() {
                body.extend_from_slice(
                    format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
                        .as_bytes(),
                );
            }
        };
        field("model", &self.model);
        field("language", hint.language);
        field("prompt", &prompt(hint));
        field("response_format", "json");
        body.extend_from_slice(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes());
        body.extend_from_slice(&encode_wav(audio));
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
        let answer = self.http.post_bytes(
            &format!("{}/audio/transcriptions", self.url),
            &self.key,
            &format!("multipart/form-data; boundary={BOUNDARY}"),
            &body,
        )?;
        answer["text"]
            .as_str()
            .map(|text| text.trim().to_string())
            .ok_or_else(|| "The speech server sent no text".to_string())
    }

    fn name(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
pub(crate) struct Fixed {
    pub answers: Vec<Result<String>>,
    pub heard: Vec<(usize, String, Vec<String>)>,
}

#[cfg(test)]
impl Fixed {
    pub fn new(answers: &[&str]) -> Fixed {
        Fixed {
            answers: answers.iter().map(|text| Ok(text.to_string())).collect(),
            heard: vec![],
        }
    }
}

#[cfg(test)]
impl Recognizer for Fixed {
    fn transcribe(&mut self, audio: &[f32], hint: &Hint) -> Result<String> {
        self.heard
            .push((audio.len(), hint.language.to_string(), hint.vocabulary.to_vec()));
        self.answers.remove(0)
    }

    fn name(&self) -> &str {
        "fixed"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Reply, Server};

    /// A recogniser whose detector gives these answers in turn, then none.
    struct Detecting(Vec<Option<Result<Vec<std::ops::Range<usize>>>>>);

    impl Recognizer for Detecting {
        fn transcribe(&mut self, audio: &[f32], _: &Hint) -> Result<String> {
            Ok(audio.len().to_string())
        }
        fn name(&self) -> &str {
            "detecting"
        }
        fn detect(&mut self, _: &[f32], _: f64) -> Option<Result<Vec<std::ops::Range<usize>>>> {
            if self.0.is_empty() {
                None
            } else {
                self.0.remove(0)
            }
        }
    }

    #[test]
    fn isolate_uses_the_detector_and_falls_back_to_loudness() {
        let speech = crate::audio::tone(1000, 0.01);
        let mut found = Detecting(vec![Some(Ok(std::iter::once(4000..8000).collect()))]);
        let trimmed = isolate(&mut found, &speech).unwrap();
        assert_eq!((trimmed.speech_ms, trimmed.samples.len()), (250, 12_000));
        // What reaches the recogniser has been raised to a normal level.
        assert!(trimmed.samples.iter().any(|sample| sample.abs() > 0.2));
        assert_eq!(found.transcribe(&trimmed.samples, &Hint::default()).unwrap(), "12000");
        assert_eq!(found.name(), "detecting");
        // With no detector, the loudness trim of the original recording is used.
        assert_eq!(isolate(&mut found, &speech).unwrap(), crate::audio::trim(&speech));
        let mut broken = Detecting(vec![Some(Err("The voice detector failed".into()))]);
        assert_eq!(isolate(&mut broken, &speech).unwrap_err(), "The voice detector failed");
        // No signal at all: nothing to recognise, and the detector is not asked.
        let mut untouched = Detecting(vec![Some(Err("must not run".into()))]);
        assert_eq!(
            isolate(&mut untouched, &[0.0; 16_000]).unwrap(),
            crate::audio::Trimmed::default()
        );
    }

    #[test]
    fn a_soft_whisper_gets_a_second_look_at_a_lower_threshold() {
        // Sustained soft sound over a quiet room: the loudness trim finds it.
        let mut soft = vec![0.0005; 8000];
        soft.extend(crate::audio::tone(1000, 0.02));
        soft.extend(vec![0.0005; 8000]);
        let mut second_look = Detecting(vec![
            Some(Ok(vec![])),
            Some(Ok(std::iter::once(9000..20_000).collect())),
        ]);
        let kept = isolate(&mut second_look, &soft).unwrap();
        assert_eq!(kept.speech_ms, 687);
        assert!(!kept.samples.is_empty());
        // Nothing at the lower threshold either: nothing is kept.
        let mut nothing = Detecting(vec![Some(Ok(vec![])), Some(Ok(vec![]))]);
        assert!(isolate(&mut nothing, &soft).unwrap().samples.is_empty());
        let mut gone = Detecting(vec![Some(Ok(vec![]))]);
        assert!(isolate(&mut gone, &soft).unwrap().samples.is_empty());
        // Without sustained sound the lower threshold is never consulted.
        let steady = crate::audio::tone(1000, 0.01);
        let mut untouched = Detecting(vec![Some(Ok(vec![])), Some(Err("must not run".into()))]);
        assert!(isolate(&mut untouched, &steady).unwrap().samples.is_empty());
    }

    #[test]
    fn prompt_sets_style_and_newest_terms() {
        assert_eq!(
            prompt(&Hint {
                language: "en",
                vocabulary: &[]
            }),
            STYLE_PROMPT
        );
        assert_eq!(
            prompt(&Hint {
                language: "hi",
                vocabulary: &[]
            }),
            ""
        );
        let terms = vec!["Sumedh".to_string(), "kubectl".to_string()];
        assert_eq!(
            prompt(&Hint {
                language: "",
                vocabulary: &terms
            }),
            format!("{STYLE_PROMPT} Glossary: Sumedh, kubectl.")
        );
        assert_eq!(
            prompt(&Hint {
                language: "hi",
                vocabulary: &terms
            }),
            "Glossary: Sumedh, kubectl."
        );
        let many: Vec<String> = (0..100).map(|i| format!("Term{i:03}")).collect();
        let long = prompt(&Hint {
            language: "hi",
            vocabulary: &many,
        });
        assert!(long.ends_with("Term098, Term099.") && !long.contains("Term000") && long.len() < 430);
    }

    #[test]
    fn phantom_phrases_need_too_little_speech() {
        assert!(is_phantom("Thank you.", 300));
        assert!(is_phantom(" Thanks for watching! ", 500));
        assert!(!is_phantom("Thank you.", 900));
        assert!(!is_phantom("Thank you for the update.", 200));
        assert!(is_phantom("you", 100));
    }

    #[test]
    fn sound_tags_are_removed() {
        assert_eq!(strip_tags("[BLANK_AUDIO]"), "");
        assert_eq!(strip_tags("Hello (music) there [noise] friend"), "Hello there friend");
        assert_eq!(strip_tags("a ) b"), "a ) b");
    }

    #[test]
    fn remote_uploads_a_wav_with_the_prompt() {
        let server = Server::start(vec![
            Reply::json(r#"{"text":" Hello there. "}"#),
            Reply::json("{}"),
            Reply::status(503, "loading"),
        ]);
        let mut remote = Remote::new(&format!("{}/v1/", server.base()), "key", "whisper-large-v3-turbo");
        let terms = vec!["Sumedh".to_string()];
        assert_eq!(
            remote
                .transcribe(
                    &[0.0; 160],
                    &Hint {
                        language: "en",
                        vocabulary: &terms
                    }
                )
                .unwrap(),
            "Hello there."
        );
        assert_eq!(remote.name(), "whisper-large-v3-turbo");
        assert!(remote.detect(&[0.0], 0.35).is_none());
        {
            let requests = server.requests();
            let body = String::from_utf8_lossy(&requests[0].body).to_string();
            assert!(requests[0].head.starts_with("POST /v1/audio/transcriptions "));
            assert!(requests[0].head.contains("multipart/form-data; boundary=speechify-"));
            assert!(body.contains("name=\"model\"\r\n\r\nwhisper-large-v3-turbo\r\n"));
            assert!(body.contains("name=\"language\"\r\n\r\nen\r\n"));
            assert!(body.contains("Glossary: Sumedh."));
            assert!(body.contains("filename=\"audio.wav\"") && body.contains("RIFF"));
        }
        assert_eq!(
            remote.transcribe(&[0.0; 160], &Hint::default()).unwrap_err(),
            "The speech server sent no text"
        );
        assert!(!String::from_utf8_lossy(&server.requests()[1].body).contains("name=\"language\""));
        assert!(remote
            .transcribe(&[0.0; 160], &Hint::default())
            .unwrap_err()
            .contains("503"));
    }

    #[test]
    fn fixed_recogniser_replays_and_records() {
        let mut fixed = Fixed::new(&["one"]);
        assert_eq!(
            fixed
                .transcribe(
                    &[0.0; 3],
                    &Hint {
                        language: "en",
                        vocabulary: &[]
                    }
                )
                .unwrap(),
            "one"
        );
        assert_eq!(fixed.heard, vec![(3, "en".to_string(), vec![])]);
        assert_eq!(fixed.name(), "fixed");
    }

    #[test]
    fn sound_tags_preserve_parentheses_brackets_and_unclosed_content() {
        assert_eq!(
            strip_tags("Meet me (tomorrow) [at noon]. (MUSIC)"),
            "Meet me (tomorrow) [at noon]."
        );
        assert_eq!(strip_tags("Keep (an unfinished thought"), "Keep (an unfinished thought");
        assert_eq!(
            strip_tags("Use array[index] and (nested (notes))"),
            "Use array[index] and (nested (notes))"
        );
    }

    #[test]
    fn remote_transcription_timeout_can_be_set_for_the_server() {
        let server = Server::start(vec![Reply::json(r#"{"text":"hello"}"#).delayed(250)]);
        let mut remote = Remote::with_timeout(&server.base(), "", "asr", Duration::from_millis(50));
        assert!(remote
            .transcribe(&[0.0; 160], &Hint::default())
            .unwrap_err()
            .contains("took too long"));
        let server = Server::start(vec![Reply::json(r#"{"text":"hello"}"#).delayed(250)]);
        let mut remote = Remote::with_timeout(&server.base(), "", "asr", Duration::from_secs(5));
        assert_eq!(remote.transcribe(&[0.0; 160], &Hint::default()).unwrap(), "hello");
    }
}
