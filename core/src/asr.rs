//! Speech recognition: the trait, a remote implementation, and guards
//! against a recogniser's favourite mistakes.

use crate::audio::encode_wav;
use crate::http::Http;
use crate::Result;
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

/// True when `text` is one of those phrases and there was too little speech
/// to have said it in earnest.
pub fn is_phantom(text: &str, speech_ms: u64) -> bool {
    let plain: String = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '\'')
        .collect();
    let words = plain.split_whitespace().count() as u64;
    // Nobody speaks faster than about six words a second.
    PHANTOMS.contains(&plain.trim()) && speech_ms < 350 + words * 170
}

/// Removes bracketed sound tags ("[BLANK_AUDIO]", "(music)") and tidies spaces.
pub fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0;
    for ch in text.chars() {
        match ch {
            '[' | '(' => depth += 1,
            ']' | ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
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
        Remote {
            http: Http::new(Some(Duration::from_secs(60))),
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
}
