//! Recognising a recording while it is still being made.
//!
//! Waiting for the whole recording before recognising any of it makes the
//! wait after a long dictation grow with its length. Instead, whenever the
//! speaker pauses, everything up to the pause is recognised straight away,
//! while they go on talking. When they stop, only the words since the last
//! pause are left, so the wait stays about the same however long they spoke.
//!
//! Pieces end only in pauses the voice detector found, never inside a word.
//! A recogniser without a detector gets the whole recording at the end, as
//! before.

use crate::asr::{is_phantom, isolate, strip_tags, Heard, Hint, Recognizer};
use crate::audio::{self, RATE};
use crate::Result;
use std::ops::Range;

const MS: usize = RATE as usize / 1000;
/// Pieces are at least this long: the recogniser does better with some context.
pub const MIN_PIECE_MS: usize = 3_000;
/// A silence at least this long between two stretches of speech can end a piece.
pub const PAUSE_MS: usize = 400;
/// Speech counts as finished once this much silence follows it.
pub const SETTLED_MS: usize = 700;
/// How much new audio arrives between looks for a pause.
pub const LOOK_EVERY_MS: usize = 400;
/// A piece longer than this ends at its longest silence, however short.
pub const MAX_PIECE_MS: usize = 20_000;

/// Where a piece may end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cuts {
    pub min_piece: usize,
    pub pause: usize,
    pub settled: usize,
    pub look_every: usize,
    pub max_piece: usize,
}

impl Default for Cuts {
    fn default() -> Cuts {
        Cuts {
            min_piece: MIN_PIECE_MS * MS,
            pause: PAUSE_MS * MS,
            settled: SETTLED_MS * MS,
            look_every: LOOK_EVERY_MS * MS,
            max_piece: MAX_PIECE_MS * MS,
        }
    }
}

/// What was recognised in a whole recording.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Transcript {
    pub raw: String,
    /// Time spent speaking.
    pub speech_ms: u64,
    pub heard: Vec<Heard>,
    /// How many pieces the recording was recognised in.
    pub pieces: usize,
    /// The words of each piece that had any, in order; `raw` joins them.
    pub parts: Vec<String>,
}

/// A recording being recognised piece by piece.
#[derive(Debug, Default)]
pub struct Live {
    cuts: Cuts,
    /// Samples at the start of the recording already recognised.
    done: usize,
    /// The recording's length the last time a pause was looked for.
    looked: usize,
    pieces: Vec<String>,
    speech_ms: u64,
    heard: Vec<Heard>,
    /// Some piece had speech in it, even if nothing came of it.
    spoke: bool,
    /// Pieces recognised, including those that came to nothing.
    taken: usize,
    /// The recogniser has no voice detector, so nothing ends early.
    blind: bool,
    /// How long the last piece took to recognise, in milliseconds.
    cost_ms: u64,
}

impl Live {
    pub fn new(cuts: Cuts) -> Live {
        Live {
            cuts,
            ..Live::default()
        }
    }

    /// A recording for a recogniser that took `cost_ms` for its last piece.
    /// A piece is never shorter than twice that, so recognising early can
    /// never fall behind the speaker: on a slow machine (Whisper without a
    /// graphics card spends half a minute on any piece) a short dictation is
    /// recognised in one go at the end, as before.
    pub fn costing(cost_ms: u64) -> Live {
        Live {
            cost_ms,
            ..Live::default()
        }
    }

    /// How long the last piece with speech took to recognise. A piece of
    /// silence is not recognised and leaves this as it was.
    pub fn cost_ms(&self) -> u64 {
        self.cost_ms
    }

    /// Samples at the start of the recording already recognised. The next
    /// call to `advance` wants the recording from here on.
    pub fn done(&self) -> usize {
        self.done
    }

    /// Whether `advance` can do anything: false once the recogniser turned
    /// out to have no voice detector, so the caller need not fetch audio.
    pub fn listening(&self) -> bool {
        !self.blind
    }

    /// The words recognised so far, piece by piece.
    pub fn pieces(&self) -> &[String] {
        &self.pieces
    }

    /// Recognises the finished part of what has been recorded so far, if a
    /// pause ends it. `pending` is the recording from `done()` on. Returns
    /// whether a piece was recognised. On an error nothing is lost: the same
    /// audio is tried again later, at the latest when the recording ends.
    pub fn advance(&mut self, pending: &[f32], recognizer: &mut dyn Recognizer, hint: &Hint) -> Result<bool> {
        let length = self.done + pending.len();
        let shortest = self.cuts.min_piece.max(self.cost_ms as usize * 2 * MS);
        if self.blind || length < self.looked + self.cuts.look_every || pending.len() < shortest {
            return Ok(false);
        }
        self.looked = length;
        let Some(cut) = self.find_cut(pending, recognizer)? else { return Ok(false) };
        self.take(&pending[..cut], recognizer, hint)?;
        self.done += cut;
        Ok(true)
    }

    /// Recognises what is left of `recording`, the whole of it, and returns
    /// the transcript, or `None` when there was no speech in it at all.
    /// The recording is left behind, so `cost_ms` can still be read.
    pub fn finish(
        &mut self,
        recording: &[f32],
        recognizer: &mut dyn Recognizer,
        hint: &Hint,
    ) -> Result<Option<Transcript>> {
        let rest = &recording[self.done.min(recording.len())..];
        self.take(rest, recognizer, hint)?;
        Ok(self.spoke.then(|| Transcript {
            raw: self.pieces.join(" "),
            speech_ms: self.speech_ms,
            heard: std::mem::take(&mut self.heard),
            pieces: self.taken,
            parts: std::mem::take(&mut self.pieces),
        }))
    }

    /// Where the finished part of `pending` ends, if anywhere: the middle
    /// of the last pause that leaves a long enough piece before it.
    fn find_cut(&mut self, pending: &[f32], recognizer: &mut dyn Recognizer) -> Result<Option<usize>> {
        let Some(loud) = audio::normalize(pending) else { return Ok(None) };
        let Some(found) = recognizer.detect(&loud, audio::DETECTION.threshold) else {
            self.blind = true;
            return Ok(None);
        };
        let regions = found?;
        let Some(last) = regions.last() else { return Ok(None) };
        let cuts = &self.cuts;
        let mut gaps: Vec<Range<usize>> = regions.windows(2).map(|pair| pair[0].end..pair[1].start).collect();
        // Speech followed by enough silence has ended, even if nothing came after it yet.
        if pending.len() >= last.end + cuts.settled {
            gaps.push(last.end..pending.len());
        }
        let middle = |gap: &Range<usize>| gap.start + gap.len() / 2;
        let shortest = cuts.min_piece.max(self.cost_ms as usize * 2 * MS);
        let usable = |gap: &&Range<usize>| middle(gap) >= shortest;
        if let Some(gap) = gaps.iter().filter(|gap| gap.len() >= cuts.pause).rfind(usable) {
            return Ok(Some(middle(gap)));
        }
        // Someone talking without a real pause: rather than let the piece
        // grow without end, end it at the longest silence there is.
        if pending.len() >= cuts.max_piece {
            let longest = gaps.iter().filter(usable).max_by_key(|gap| gap.len());
            return Ok(longest.map(middle));
        }
        Ok(None)
    }

    fn take(&mut self, piece: &[f32], recognizer: &mut dyn Recognizer, hint: &Hint) -> Result<()> {
        let trimmed = isolate(recognizer, piece)?;
        if trimmed.samples.is_empty() {
            return Ok(());
        }
        let previous = self.pieces.join(" ");
        let started = std::time::Instant::now();
        let raw = strip_tags(&recognizer.transcribe(
            &trimmed.samples,
            &Hint {
                previous: &previous,
                ..*hint
            },
        )?);
        self.cost_ms = started.elapsed().as_millis() as u64;
        self.spoke = true;
        self.taken += 1;
        self.speech_ms += trimmed.speech_ms;
        if !raw.is_empty() && !is_phantom(&raw, trimmed.speech_ms) {
            self.heard.extend(recognizer.heard());
            self.pieces.push(raw);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asr::Fixed;
    use crate::audio::tone;

    const HINT: Hint = Hint {
        language: "en",
        vocabulary: &[],
        previous: "ignored",
    };

    fn silence(ms: usize) -> Vec<f32> {
        vec![0.0; ms * MS]
    }

    fn join(parts: &[Vec<f32>]) -> Vec<f32> {
        parts.concat()
    }

    #[test]
    fn pieces_end_in_pauses_and_the_rest_waits_for_the_end() {
        let recording = join(&[tone(4000, 0.3), silence(1000), tone(2500, 0.3), silence(200)]);
        let mut asr = Fixed::detecting(&["first part.", "and the rest."]);
        let mut live = Live::default();
        // Too little to bother with yet.
        assert!(!live.advance(&recording[..2000 * MS], &mut asr, &HINT).unwrap());
        // Speech, but no pause after it yet.
        assert!(!live.advance(&recording[..4300 * MS], &mut asr, &HINT).unwrap());
        // Once the pause has lasted, the speech before it is recognised.
        assert!(live.advance(&recording[..4800 * MS], &mut asr, &HINT).unwrap());
        assert_eq!(live.done(), 4400 * MS);
        assert_eq!(asr.heard.len(), 1);
        // The second piece has not ended; it is recognised when the recording does.
        let rest = &recording[live.done()..];
        assert!(!live.advance(&rest[..2500 * MS], &mut asr, &HINT).unwrap());
        let heard = live.finish(&recording, &mut asr, &HINT).unwrap().unwrap();
        assert_eq!(heard.raw, "first part. and the rest.");
        assert_eq!(heard.pieces, 2);
        assert_eq!(heard.speech_ms, 6500);
        // Each piece is told what came before it, never the caller's text.
        assert_eq!(asr.heard[0].3, "");
        assert_eq!(asr.heard[1].3, "first part.");
    }

    #[test]
    fn a_slow_recogniser_gets_pieces_twice_as_long_as_it_takes() {
        let recording = join(&[tone(4000, 0.3), silence(1000), tone(3000, 0.3), silence(1000)]);
        let mut asr = Fixed::detecting(&["first", "second"]);
        // The last piece took 4 s: a piece must be 8 s long.
        let mut live = Live::costing(4000);
        assert_eq!(live.cost_ms(), 4000);
        assert!(!live.advance(&recording[..5000 * MS], &mut asr, &HINT).unwrap());
        assert!(live.advance(&recording, &mut asr, &HINT).unwrap());
        assert_eq!(live.done(), 8500 * MS);
        // Recognising "first" took no time at all.
        assert!(live.cost_ms() < 1000);
        // An ending with no speech is not recognised, and says nothing about speed.
        let mut quiet = Live::costing(4000);
        assert_eq!(quiet.finish(&silence(2000), &mut asr, &HINT).unwrap(), None);
        assert_eq!(quiet.cost_ms(), 4000);
    }

    #[test]
    fn a_look_for_a_pause_waits_for_new_audio() {
        let recording = join(&[tone(3500, 0.3), silence(300), tone(300, 0.3), silence(100)]);
        let mut asr = Fixed::detecting(&["one"]);
        let mut live = Live::default();
        assert!(!live.advance(&recording[..3600 * MS], &mut asr, &HINT).unwrap());
        // The pause is now long enough, but too little new audio has arrived to look again.
        let longer = join(&[recording.clone(), silence(800)]);
        assert!(!live.advance(&longer[..3900 * MS], &mut asr, &HINT).unwrap());
        assert!(live.advance(&longer, &mut asr, &HINT).unwrap());
    }

    #[test]
    fn short_pauses_end_a_piece_only_when_it_grows_too_long() {
        // Talking with only 200 ms breaths: no pause counts until 20 seconds.
        let mut breathless = Vec::new();
        for _ in 0..8 {
            breathless.extend(tone(2300, 0.3));
            breathless.extend(silence(200));
        }
        let mut asr = Fixed::detecting(&["long", "end"]);
        let mut live = Live::default();
        assert!(!live.advance(&breathless[..15_000 * MS], &mut asr, &HINT).unwrap());
        assert!(live.advance(&breathless, &mut asr, &HINT).unwrap());
        // The breaths are equally long; the latest one wins, leaving least behind.
        assert_eq!(live.done(), 17_400 * MS);
        // One sound with no gap at all is never cut.
        let unbroken = tone(21_000, 0.3);
        let mut live = Live::default();
        assert!(!live.advance(&unbroken, &mut asr, &HINT).unwrap());
    }

    #[test]
    fn silence_and_recognisers_without_a_detector_wait_for_the_end() {
        let mut asr = Fixed::detecting(&[]);
        let mut live = Live::default();
        assert!(!live.advance(&silence(5000), &mut asr, &HINT).unwrap());
        assert_eq!(live.finish(&silence(5000), &mut asr, &HINT).unwrap(), None);

        let recording = join(&[tone(4000, 0.3), silence(1000)]);
        let mut plain = Fixed::new(&["all of it"]);
        let mut live = Live::default();
        assert!(live.listening());
        assert!(!live.advance(&recording, &mut plain, &HINT).unwrap());
        // Having found no detector, it stops asking for audio.
        assert!(!live.listening());
        assert!(!live
            .advance(&join(&[recording.clone(), silence(1000)]), &mut plain, &HINT)
            .unwrap());
        let heard = live.finish(&recording, &mut plain, &HINT).unwrap().unwrap();
        assert_eq!((heard.raw.as_str(), heard.pieces), ("all of it", 1));
    }

    #[test]
    fn phantoms_and_empty_pieces_add_no_words() {
        let recording = join(&[tone(3500, 0.3), silence(1000), tone(200, 0.3), silence(100)]);
        let mut asr = Fixed::detecting(&["[BLANK_AUDIO]", "Thank you."]);
        let mut live = Live::default();
        assert!(live.advance(&recording, &mut asr, &HINT).unwrap());
        let heard = live.finish(&recording, &mut asr, &HINT).unwrap().unwrap();
        // Speech was there, so this is "nothing", not "silent".
        assert_eq!((heard.raw.as_str(), heard.pieces), ("", 2));
    }

    #[test]
    fn a_failed_piece_is_tried_again() {
        let recording = join(&[tone(3500, 0.3), silence(1000)]);
        let mut asr = Fixed::detecting(&[]);
        asr.answers = vec![Err("busy".into()), Ok("again".into())];
        let mut live = Live::default();
        assert_eq!(live.advance(&recording, &mut asr, &HINT).unwrap_err(), "busy");
        assert_eq!(live.done(), 0);
        let heard = live.finish(&recording, &mut asr, &HINT).unwrap().unwrap();
        assert_eq!(heard.raw, "again");
        // A detector failure is reported the same way.
        struct Broken;
        impl Recognizer for Broken {
            fn transcribe(&mut self, _: &[f32], _: &Hint) -> Result<String> {
                Ok(String::new())
            }
            fn name(&self) -> &str {
                "broken"
            }
            fn detect(&mut self, _: &[f32], _: f64) -> Option<Result<Vec<Range<usize>>>> {
                Some(Err("detector failed".into()))
            }
        }
        let mut live = Live::new(Cuts::default());
        assert_eq!(
            live.advance(&recording, &mut Broken, &HINT).unwrap_err(),
            "detector failed"
        );
        assert_eq!(Broken.transcribe(&[], &HINT).unwrap(), "");
        assert_eq!(Broken.name(), "broken");
    }
}
