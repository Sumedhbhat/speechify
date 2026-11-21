//! Speech recognition: the trait, a remote implementation, and guards
//! against a recogniser's favourite mistakes.

use crate::audio::encode_wav;
use crate::http::Http;
use crate::Result;
use regex::Regex;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::Duration;

/// What the recogniser is told about the speaker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Hint<'a> {
    /// ISO code such as "en"; empty lets the model detect it.
    pub language: &'a str,
    /// Names and terms to prefer.
    pub vocabulary: &'a [String],
    /// What was said just before this audio, when a recording is recognised
    /// in pieces. Whisper continues the style and spelling of its prompt.
    pub previous: &'a str,
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
    /// Like `transcribe`, but a recogniser that can gives up early once
    /// `stop` is set: nobody wants the answer any more.
    fn transcribe_until(&mut self, audio: &[f32], hint: &Hint, _stop: &AtomicBool) -> Result<String> {
        self.transcribe(audio, hint)
    }
    /// Another handle on this recogniser, for a thread of its own, if it
    /// can be used from one. `None` means it works only where it is.
    fn share(&mut self) -> Option<Box<dyn Recognizer + Send>> {
        None
    }
    /// Whether a handle shared from this one is in the middle of something,
    /// so that using this one now would wait for it.
    fn busy(&self) -> bool {
        false
    }
    /// Whether this recogniser is a model made to take audio as it comes.
    fn streams(&self) -> bool {
        false
    }
    /// Starts a stream, abandoning whatever one came before, unless `stop`
    /// is already set: a cancelled start must not reset a newer stream.
    fn stream_start(&mut self, _hint: &Hint, _stop: &AtomicBool) -> Result<()> {
        Err(NOT_STREAMING.into())
    }
    /// Gives the stream more audio, and says what it has made of it all so
    /// far. Gives up early once `stop` is set.
    fn stream_feed(&mut self, _audio: &[f32], _stop: &AtomicBool) -> Result<Streamed> {
        Err(NOT_STREAMING.into())
    }
    /// Ends the stream: the audio it still held back is recognised too.
    fn stream_end(&mut self, _stop: &AtomicBool) -> Result<Streamed> {
        Err(NOT_STREAMING.into())
    }
}

pub const NOT_STREAMING: &str = "This speech model cannot recognise speech as it comes";

/// What a stream has made of its audio so far.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Streamed {
    /// The start of the text, which no later audio will change.
    pub settled: String,
    /// All of the text as the model now hears it.
    pub full: String,
}

/// A recogniser that threads take turns with: each use waits for the one
/// before it, on whichever handle that was.
pub struct Shared<R> {
    inner: Arc<Mutex<R>>,
    name: String,
    /// Known without the lock, which a stream on another thread may hold.
    streams: bool,
}

impl<R: Recognizer> Shared<R> {
    pub fn new(recognizer: R) -> Shared<R> {
        Shared {
            name: recognizer.name().to_string(),
            streams: recognizer.streams(),
            inner: Arc::new(Mutex::new(recognizer)),
        }
    }

    /// The recogniser itself, once no other handle is using it.
    pub fn lock(&self) -> MutexGuard<'_, R> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Waits up to `wait` for every other handle to be dropped, so that
    /// dropping this one frees the recogniser here and now. Returns whether
    /// that happened in time.
    pub fn wait_alone(&self, wait: Duration) -> bool {
        let deadline = std::time::Instant::now() + wait;
        while Arc::strong_count(&self.inner) > 1 {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        true
    }
}

impl<R> Clone for Shared<R> {
    fn clone(&self) -> Shared<R> {
        Shared {
            inner: self.inner.clone(),
            name: self.name.clone(),
            streams: self.streams,
        }
    }
}

impl<R: Recognizer + Send + 'static> Recognizer for Shared<R> {
    fn transcribe(&mut self, audio: &[f32], hint: &Hint) -> Result<String> {
        self.lock().transcribe(audio, hint)
    }
    fn transcribe_until(&mut self, audio: &[f32], hint: &Hint, stop: &AtomicBool) -> Result<String> {
        self.lock().transcribe_until(audio, hint, stop)
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn detect(&mut self, audio: &[f32], threshold: f64) -> Option<Result<Vec<std::ops::Range<usize>>>> {
        self.lock().detect(audio, threshold)
    }
    fn heard(&self) -> Vec<Heard> {
        self.lock().heard()
    }
    fn share(&mut self) -> Option<Box<dyn Recognizer + Send>> {
        Some(Box::new(self.clone()))
    }
    fn busy(&self) -> bool {
        matches!(self.inner.try_lock(), Err(std::sync::TryLockError::WouldBlock))
    }
    fn streams(&self) -> bool {
        self.streams
    }
    fn stream_start(&mut self, hint: &Hint, stop: &AtomicBool) -> Result<()> {
        // `stop` is read under the lock, so a cancelled job cannot start
        // after a newer recording has begun its own stream.
        self.lock().stream_start(hint, stop)
    }
    fn stream_feed(&mut self, audio: &[f32], stop: &AtomicBool) -> Result<Streamed> {
        self.lock().stream_feed(audio, stop)
    }
    fn stream_end(&mut self, stop: &AtomicBool) -> Result<Streamed> {
        self.lock().stream_end(stop)
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
    let previous = hint.previous.trim();
    if !previous.is_empty() {
        // The most recent words, cut at a word boundary, last in the prompt
        // where Whisper expects the preceding transcript.
        let start = if previous.len() <= PREVIOUS_CHARS {
            0
        } else {
            previous
                .char_indices()
                .find(|(index, c)| c.is_whitespace() && previous.len() - index <= PREVIOUS_CHARS)
                .map_or(previous.len(), |(index, c)| index + c.len_utf8())
        };
        text.push(' ');
        text.push_str(&previous[start..]);
    }
    text.trim().to_string()
}

/// How much of the preceding transcript a piece's prompt carries.
const PREVIOUS_CHARS: usize = 200;

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
#[derive(Clone)]
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

    /// A server needs nothing in common between requests.
    fn share(&mut self) -> Option<Box<dyn Recognizer + Send>> {
        Some(Box::new(self.clone()))
    }
}

#[cfg(test)]
pub(crate) struct Fixed {
    pub answers: Vec<Result<String>>,
    /// Each call: samples, language, vocabulary, previous text.
    pub heard: Vec<(usize, String, Vec<String>, String)>,
    /// Finds speech exactly, wherever the samples are not silent.
    pub detects: bool,
    /// Each answer waits until this is free.
    pub gate: Option<Arc<Mutex<()>>>,
}

#[cfg(test)]
impl Fixed {
    pub fn new(answers: &[&str]) -> Fixed {
        Fixed {
            answers: answers.iter().map(|text| Ok(text.to_string())).collect(),
            heard: vec![],
            detects: false,
            gate: None,
        }
    }

    /// Like `new`, with a voice detector.
    pub fn detecting(answers: &[&str]) -> Fixed {
        Fixed {
            detects: true,
            ..Fixed::new(answers)
        }
    }
}

#[cfg(test)]
impl Recognizer for Fixed {
    fn transcribe(&mut self, audio: &[f32], hint: &Hint) -> Result<String> {
        self.heard.push((
            audio.len(),
            hint.language.to_string(),
            hint.vocabulary.to_vec(),
            hint.previous.to_string(),
        ));
        let _open = self.gate.as_ref().map(|gate| gate.lock().unwrap());
        self.answers.remove(0)
    }

    fn name(&self) -> &str {
        "fixed"
    }

    fn detect(&mut self, audio: &[f32], _: f64) -> Option<Result<Vec<std::ops::Range<usize>>>> {
        self.detects.then(|| Ok(crate::testing::sounding(audio)))
    }
}

/// A model made to stream: each feed settles the next text in `settles`
/// (the last one stays once they run out), and ending gives `ending`.
#[cfg(test)]
pub(crate) struct Talker {
    pub settles: Vec<Result<String>>,
    pub ending: Result<String>,
    /// What recognising the whole recording at once gives.
    pub whole: String,
    pub detects: bool,
    pub started: usize,
    pub fed: usize,
    /// Each feed waits until this is free.
    pub gate: Option<Arc<Mutex<()>>>,
}

#[cfg(test)]
impl Talker {
    pub fn new(settles: &[&str], ending: &str) -> Talker {
        Talker {
            settles: settles.iter().map(|text| Ok(text.to_string())).collect(),
            ending: Ok(ending.into()),
            whole: String::new(),
            detects: true,
            started: 0,
            fed: 0,
            gate: None,
        }
    }
}

#[cfg(test)]
impl Recognizer for Talker {
    fn transcribe(&mut self, _: &[f32], _: &Hint) -> Result<String> {
        Ok(self.whole.clone())
    }
    fn name(&self) -> &str {
        "talker"
    }
    fn detect(&mut self, audio: &[f32], _: f64) -> Option<Result<Vec<std::ops::Range<usize>>>> {
        self.detects.then(|| Ok(crate::testing::sounding(audio)))
    }
    fn streams(&self) -> bool {
        true
    }
    fn stream_start(&mut self, hint: &Hint, _: &AtomicBool) -> Result<()> {
        assert_eq!(hint.language, "en");
        self.started += 1;
        Ok(())
    }
    fn stream_feed(&mut self, audio: &[f32], _: &AtomicBool) -> Result<Streamed> {
        let _open = self.gate.as_ref().map(|gate| gate.lock().unwrap());
        self.fed += audio.len();
        let next = if self.settles.len() > 1 { self.settles.remove(0) } else { self.settles[0].clone() };
        next.map(|settled| Streamed {
            full: settled.clone(),
            settled,
        })
    }
    fn stream_end(&mut self, _: &AtomicBool) -> Result<Streamed> {
        self.ending.clone().map(|full| Streamed {
            settled: full.clone(),
            full,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Reply, Server};

    #[test]
    fn an_ordinary_recogniser_cannot_stream() {
        let mut fixed = Fixed::new(&[]);
        let stop = AtomicBool::new(false);
        assert!(!fixed.streams() && !Shared::new(Fixed::new(&[])).streams());
        assert_eq!(
            fixed
                .stream_start(&Hint::default(), &AtomicBool::new(false))
                .unwrap_err(),
            NOT_STREAMING
        );
        assert_eq!(fixed.stream_feed(&[0.0], &stop).unwrap_err(), NOT_STREAMING);
        assert_eq!(fixed.stream_end(&stop).unwrap_err(), NOT_STREAMING);
    }

    #[test]
    fn a_shared_recogniser_can_wait_for_its_other_handles() {
        let mut shared = Shared::new(Fixed::new(&[]));
        assert!(shared.wait_alone(Duration::ZERO));
        let other = shared.share().unwrap();
        assert!(!shared.wait_alone(Duration::from_millis(10)));
        let gone = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            drop(other);
        });
        assert!(shared.wait_alone(Duration::from_secs(5)));
        gone.join().unwrap();
        // A speech server can be asked from anywhere.
        let mut remote = Remote::new("http://127.0.0.1:9", "", "model");
        assert_eq!(remote.share().unwrap().name(), "model");
    }

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
                vocabulary: &[],
                previous: "",
            }),
            STYLE_PROMPT
        );
        assert_eq!(
            prompt(&Hint {
                language: "hi",
                vocabulary: &[],
                previous: "",
            }),
            ""
        );
        let terms = vec!["Sumedh".to_string(), "kubectl".to_string()];
        assert_eq!(
            prompt(&Hint {
                language: "",
                vocabulary: &terms,
                previous: "",
            }),
            format!("{STYLE_PROMPT} Glossary: Sumedh, kubectl.")
        );
        assert_eq!(
            prompt(&Hint {
                language: "hi",
                vocabulary: &terms,
                previous: "",
            }),
            "Glossary: Sumedh, kubectl."
        );
        let many: Vec<String> = (0..100).map(|i| format!("Term{i:03}")).collect();
        let long = prompt(&Hint {
            language: "hi",
            vocabulary: &many,
            previous: "",
        });
        assert!(long.ends_with("Term098, Term099.") && !long.contains("Term000") && long.len() < 430);
        // A later piece also gets the end of what was said before it.
        let before = prompt(&Hint {
            language: "en",
            vocabulary: &[],
            previous: " We met on Monday. ",
        });
        assert_eq!(before, format!("{STYLE_PROMPT} We met on Monday."));
        let story = (0..100).map(|i| format!("w{i:02}")).collect::<Vec<_>>().join(" ");
        let tail = prompt(&Hint {
            language: "hi",
            vocabulary: &[],
            previous: &story,
        });
        // Whole words only, the newest ones, at most 200 characters.
        assert!(
            tail.ends_with("w98 w99") && tail.starts_with('w') && tail.len() <= 200,
            "{tail}"
        );
        assert!(!tail.contains("w49"));
        // A wide space is still a word boundary, and is cut after whole.
        let wide = format!("{}\u{3000}{}", "x".repeat(150), "y".repeat(100));
        assert_eq!(
            prompt(&Hint {
                language: "hi",
                vocabulary: &[],
                previous: &wide,
            }),
            "y".repeat(100)
        );
        let unbroken = "x".repeat(300);
        assert_eq!(
            prompt(&Hint {
                language: "hi",
                vocabulary: &[],
                previous: &unbroken,
            }),
            ""
        );
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
                        vocabulary: &terms,
                        previous: "",
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
                        vocabulary: &[],
                        previous: "",
                    }
                )
                .unwrap(),
            "one"
        );
        assert_eq!(fixed.heard, vec![(3, "en".to_string(), vec![], String::new())]);
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
