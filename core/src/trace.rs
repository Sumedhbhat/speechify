use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    #[default]
    Stop,
    Load,
    Speech,
    Settle,
    Cleanup,
    Paste,
    Detect,
    Scan,
    Piece,
    Feed,
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelTime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer_tokens: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Detail {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Span {
    pub stage: Stage,
    pub at_ms: i64,
    pub ms: u32,
    #[serde(flatten)]
    pub detail: Detail,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timed {
    pub stage: Stage,
    pub start: Instant,
    pub took: Duration,
    pub detail: Detail,
}

impl Timed {
    pub fn since(stage: Stage, start: Instant) -> Timed {
        Timed {
            stage,
            start,
            took: start.elapsed(),
            detail: Detail::default(),
        }
    }

    pub fn with(mut self, detail: Detail) -> Timed {
        self.detail = detail;
        self
    }

    pub fn at(&self, released: Instant) -> Span {
        let at_ms = match self.start.checked_duration_since(released) {
            Some(after) => after.as_millis() as i64,
            None => -(released.duration_since(self.start).as_millis() as i64),
        };
        Span {
            stage: self.stage,
            at_ms,
            ms: self.took.as_millis().min(u32::MAX as u128) as u32,
            detail: self.detail,
        }
    }
}

pub fn spans(timed: &[Timed], released: Instant) -> Vec<Span> {
    let mut spans: Vec<Span> = timed.iter().map(|t| t.at(released)).collect();
    spans.sort_by_key(|span| span.at_ms);
    spans
}

pub fn words(text: &str) -> Option<u32> {
    Some(text.split_whitespace().count() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_are_placed_around_the_release_and_sorted() {
        let early = Instant::now();
        let released = early + Duration::from_millis(1500);
        let late = Timed {
            stage: Stage::Paste,
            start: released + Duration::from_millis(40),
            took: Duration::from_millis(12),
            detail: Detail::default(),
        };
        let piece = Timed {
            stage: Stage::Piece,
            start: early,
            took: Duration::from_millis(300),
            detail: Detail {
                chunk: Some(0),
                audio_ms: Some(4000),
                words: words("one two three"),
                model: None,
            },
        };
        let spans = spans(&[late, piece], released);
        assert_eq!(
            (spans[0].stage, spans[0].at_ms, spans[0].ms),
            (Stage::Piece, -1500, 300)
        );
        assert_eq!((spans[1].stage, spans[1].at_ms, spans[1].ms), (Stage::Paste, 40, 12));
        let json = serde_json::to_value(spans[0]).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"stage": "piece", "at_ms": -1500, "ms": 300, "chunk": 0, "audio_ms": 4000, "words": 3})
        );
        let back: Span = serde_json::from_value(json).unwrap();
        assert_eq!(back, spans[0]);
    }

    #[test]
    fn a_span_measured_now_has_its_stage_and_detail() {
        let start = Instant::now();
        let timed = Timed::since(Stage::Model, start).with(Detail {
            model: Some(ModelTime {
                prompt_tokens: Some(10),
                ..ModelTime::default()
            }),
            ..Detail::default()
        });
        let span = timed.at(start);
        assert_eq!((span.stage, span.at_ms), (Stage::Model, 0));
        assert_eq!(span.detail.model.unwrap().prompt_tokens, Some(10));
    }
}
