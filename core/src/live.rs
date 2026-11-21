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
//!
//! A recogniser that can work on a thread of its own recognises each piece
//! there, so a key release or a cancel never waits for a piece to finish:
//! a cancel drops it, and a release waits only for the words it needs.
//!
//! A model made to stream (Parakeet Unified, say) needs no pauses: it is
//! given the audio as it arrives and settles the text as it goes. When the
//! speaker stops, only the last second or so is left for it.

use crate::asr::{is_phantom, isolate, strip_tags, Heard, Hint, Recognizer, Streamed};
use crate::audio::{self, Trimmed, RATE};
use crate::job::Job;
use crate::Result;
use std::ops::Range;
use std::time::Instant;

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
/// A streaming model is given new audio once this much has arrived.
pub const FEED_MS: usize = 480;
/// A streaming model this far behind the speaker cannot keep up (a laptop
/// CPU, say): it is given up, and the recording recognised in one go at the
/// end, rather than make the user wait for it to catch up.
pub const BEHIND_MS: usize = 4_000;

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
    /// The pieces handed out while recording are not the start of `raw`:
    /// a streaming model changed its mind about them, or failed and the
    /// whole recording was recognised again.
    pub revised: bool,
}

/// A piece being recognised on a thread of its own: its words, what the
/// recogniser made of each, and how long it took.
type Answer = (String, Vec<Heard>, u64);

#[derive(Debug)]
struct Flight {
    job: Job<Answer>,
    /// Where the piece ends, counted from `done`.
    cut: usize,
    speech_ms: u64,
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
    /// The piece being recognised elsewhere, if any. Dropping it, as a
    /// cancel does, tells the recogniser to stop.
    flight: Option<Flight>,
    /// Set when the recogniser is a model made to stream.
    stream: Option<Stream>,
}

/// What a streaming model gives back for some audio: its text, and how
/// much of that audio was speech.
type Fed = (Streamed, u64);

/// A recording given to a model made to stream, as it is made.
#[derive(Debug, Default)]
struct Stream {
    /// The model accepted the start of the stream.
    started: bool,
    /// Something went wrong: the whole recording is recognised at the end.
    broken: bool,
    /// The text the model has settled on so far.
    settled: String,
    /// How much of `settled` has been handed out as pieces.
    handed: usize,
    /// Audio being fed on a thread of its own, and how long it is.
    feeding: Option<(Job<Fed>, usize)>,
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
    ///
    /// When the recogniser can be shared, the piece is recognised on a
    /// thread of its own and this returns at once; a later call collects
    /// the words and returns true then.
    pub fn advance(&mut self, pending: &[f32], recognizer: &mut dyn Recognizer, hint: &Hint) -> Result<bool> {
        if self.stream.is_none() && self.done == 0 && recognizer.streams() {
            self.stream = Some(Stream::default());
        }
        if self.stream.is_some() {
            return self.flow(pending, recognizer, hint);
        }
        if let Some(flight) = &self.flight {
            let Some(answer) = flight.job.poll() else { return Ok(false) };
            let Flight { cut, speech_ms, .. } = self.flight.take().expect("a piece in flight");
            return self.land(cut, speech_ms, answer).map(|()| true);
        }
        let length = self.done + pending.len();
        let shortest = self.cuts.min_piece.max(self.cost_ms as usize * 2 * MS);
        // A piece a cancelled recording left behind may still be finishing:
        // looking now would wait for it.
        if self.blind || length < self.looked + self.cuts.look_every || pending.len() < shortest || recognizer.busy() {
            return Ok(false);
        }
        self.looked = length;
        let Some(cut) = self.find_cut(pending, recognizer)? else { return Ok(false) };
        let trimmed = isolate(recognizer, &pending[..cut])?;
        // Silence needs no recogniser.
        let shared = if trimmed.samples.is_empty() { None } else { recognizer.share() };
        if let Some(shared) = shared {
            let job = self.launch(shared, trimmed.samples, hint);
            let speech_ms = trimmed.speech_ms;
            self.flight = Some(Flight { job, cut, speech_ms });
            return Ok(false);
        }
        self.take(trimmed, recognizer, hint)?;
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
        if self.stream.as_ref().is_some_and(|stream| stream.started) {
            return self.end_stream(recording, recognizer, hint);
        }
        if let Some(Flight { job, cut, speech_ms }) = self.flight.take() {
            // A piece that failed is simply recognised again with the rest.
            let _ = self.land(cut, speech_ms, job.wait());
        }
        let rest = &recording[self.done.min(recording.len())..];
        let trimmed = isolate(recognizer, rest)?;
        self.take(trimmed, recognizer, hint)?;
        Ok(self.spoke.then(|| Transcript {
            raw: self.pieces.join(" "),
            speech_ms: self.speech_ms,
            heard: std::mem::take(&mut self.heard),
            pieces: self.taken,
            parts: std::mem::take(&mut self.pieces),
            revised: false,
        }))
    }

    /// Gives a streaming model the audio recorded since it was last fed.
    /// Returns whether new sentences were settled.
    fn flow(&mut self, pending: &[f32], recognizer: &mut dyn Recognizer, hint: &Hint) -> Result<bool> {
        let stream = self.stream.as_mut().expect("a stream");
        stream.keep_up(pending.len());
        if let Some((job, length)) = &stream.feeding {
            let Some(answer) = job.poll() else { return Ok(false) };
            let length = *length;
            stream.feeding = None;
            return self.fed(length, answer);
        }
        if stream.broken || pending.len() < FEED_MS * MS || recognizer.busy() {
            return Ok(false);
        }
        let begin = !stream.started;
        stream.started = true;
        let length = pending.len();
        match recognizer.share() {
            Some(mut shared) => {
                let samples = pending.to_vec();
                let language = hint.language.to_string();
                let job = Job::start(move |stop| {
                    if begin {
                        shared.stream_start(&Hint {
                            language: &language,
                            ..Hint::default()
                        })?;
                    }
                    feed(shared.as_mut(), &samples, stop)
                });
                self.stream.as_mut().expect("a stream").feeding = Some((job, length));
                Ok(false)
            }
            None => {
                let answer = if begin { recognizer.stream_start(hint) } else { Ok(()) }
                    .and_then(|()| feed(recognizer, pending, &std::sync::atomic::AtomicBool::new(false)));
                self.fed(length, answer)
            }
        }
    }

    /// Takes in what the model made of `length` more samples, and hands out
    /// any sentences it has settled. On an error the stream is given up,
    /// and the whole recording is recognised when it ends.
    fn fed(&mut self, length: usize, answer: Result<Fed>) -> Result<bool> {
        let stream = self.stream.as_mut().expect("a stream");
        let (streamed, speech_ms) = match answer {
            Ok(answer) => answer,
            Err(error) => {
                stream.broken = true;
                return Err(error);
            }
        };
        self.done += length;
        self.speech_ms += speech_ms;
        stream.settled = streamed.settled;
        let Some(end) = sentences_end(&stream.settled, stream.handed) else { return Ok(false) };
        let piece = stream.settled[stream.handed..end].trim().to_string();
        stream.handed = end;
        self.pieces.push(piece);
        Ok(true)
    }

    /// Ends the stream with the rest of `recording` and returns all of its
    /// text. When the stream broke, the whole recording is recognised again
    /// from the start instead.
    fn end_stream(
        &mut self,
        recording: &[f32],
        recognizer: &mut dyn Recognizer,
        hint: &Hint,
    ) -> Result<Option<Transcript>> {
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut stream = self.stream.take().expect("a stream");
        stream.keep_up(recording.len().saturating_sub(self.done));
        if let Some((job, length)) = stream.feeding.take() {
            self.stream = Some(stream);
            let _ = self.fed(length, job.wait());
            stream = self.stream.take().expect("a stream");
        }
        let rest = &recording[self.done.min(recording.len())..];
        let ended = if stream.broken {
            Err(String::new())
        } else {
            feed(recognizer, rest, &stop).and_then(|(_, speech_ms)| {
                self.speech_ms += speech_ms;
                recognizer.stream_end(&stop)
            })
        };
        let Ok(ended) = ended else {
            // Start again, as if nothing had been streamed.
            let handed = !self.pieces.is_empty();
            *self = Live::costing(self.cost_ms);
            let trimmed = isolate(recognizer, recording)?;
            self.take(trimmed, recognizer, hint)?;
            return Ok(self.spoke.then(|| Transcript {
                raw: self.pieces.join(" "),
                speech_ms: self.speech_ms,
                heard: std::mem::take(&mut self.heard),
                pieces: 1,
                parts: std::mem::take(&mut self.pieces),
                revised: handed,
            }));
        };
        self.done = recording.len();
        let raw = strip_tags(ended.full.trim());
        if self.speech_ms == 0 && raw.is_empty() {
            return Ok(None);
        }
        let raw = if is_phantom(&raw, self.speech_ms) { String::new() } else { raw };
        let handed = stream.settled[..stream.handed.min(stream.settled.len())].trim();
        let revised = !raw.starts_with(handed);
        let mut parts = std::mem::take(&mut self.pieces);
        if revised {
            parts = vec![raw.clone()];
        } else if !raw[handed.len()..].trim().is_empty() {
            parts.push(raw[handed.len()..].trim().to_string());
        }
        parts.retain(|part| !part.is_empty());
        Ok(Some(Transcript {
            raw,
            speech_ms: self.speech_ms,
            heard: recognizer.heard(),
            pieces: 1,
            parts,
            revised,
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

    fn take(&mut self, trimmed: Trimmed, recognizer: &mut dyn Recognizer, hint: &Hint) -> Result<()> {
        if trimmed.samples.is_empty() {
            return Ok(());
        }
        let previous = self.pieces.join(" ");
        let started = Instant::now();
        let raw = recognizer.transcribe(
            &trimmed.samples,
            &Hint {
                previous: &previous,
                ..*hint
            },
        )?;
        let cost_ms = started.elapsed().as_millis() as u64;
        self.record(&raw, recognizer.heard(), trimmed.speech_ms, cost_ms);
        Ok(())
    }

    /// Recognises a piece on a thread of its own.
    fn launch(&self, mut recognizer: Box<dyn Recognizer + Send>, samples: Vec<f32>, hint: &Hint) -> Job<Answer> {
        let language = hint.language.to_string();
        let vocabulary = hint.vocabulary.to_vec();
        let previous = self.pieces.join(" ");
        Job::start(move |stop| {
            let hint = Hint {
                language: &language,
                vocabulary: &vocabulary,
                previous: &previous,
            };
            let started = Instant::now();
            let raw = recognizer.transcribe_until(&samples, &hint, stop)?;
            Ok((raw, recognizer.heard(), started.elapsed().as_millis() as u64))
        })
    }

    /// Takes in a piece recognised elsewhere. On an error nothing moves on,
    /// so the piece is tried again.
    fn land(&mut self, cut: usize, speech_ms: u64, answer: Result<Answer>) -> Result<()> {
        let (raw, heard, cost_ms) = answer?;
        self.record(&raw, heard, speech_ms, cost_ms);
        self.done += cut;
        Ok(())
    }

    fn record(&mut self, raw: &str, heard: Vec<Heard>, speech_ms: u64, cost_ms: u64) {
        let raw = strip_tags(raw);
        self.cost_ms = cost_ms;
        self.spoke = true;
        self.taken += 1;
        self.speech_ms += speech_ms;
        if !raw.is_empty() && !is_phantom(&raw, speech_ms) {
            self.heard.extend(heard);
            self.pieces.push(raw);
        }
    }
}

impl Stream {
    /// Gives up on a stream that has fallen `behind` samples behind the
    /// speaker. The audio being fed is told to stop.
    fn keep_up(&mut self, behind: usize) {
        if behind > BEHIND_MS * MS {
            self.broken = true;
            self.feeding = None;
        }
    }
}

/// Gives a stream more audio, and finds how much of it was speech.
fn feed(recognizer: &mut dyn Recognizer, audio: &[f32], stop: &std::sync::atomic::AtomicBool) -> Result<Fed> {
    let speech = match audio::normalize(audio) {
        None => 0,
        Some(loud) => match recognizer.detect(&loud, audio::DETECTION.threshold) {
            Some(found) => found?.iter().map(Range::len).sum::<usize>(),
            None => audio.len(),
        },
    };
    let streamed = recognizer.stream_feed(audio, stop)?;
    Ok((streamed, (speech / MS) as u64))
}

/// Where the last whole sentence in `text` after byte `from` ends: just
/// after its full stop, question or exclamation mark, once the next word
/// has begun, so the sentence cannot still be going on.
fn sentences_end(text: &str, from: usize) -> Option<usize> {
    let tail = text.get(from..)?;
    let bytes = tail.as_bytes();
    (1..bytes.len())
        .rev()
        .find(|&i| matches!(bytes[i - 1], b'.' | b'?' | b'!') && bytes[i].is_ascii_whitespace())
        .map(|i| from + i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asr::{Fixed, Shared, Talker};
    use crate::audio::tone;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

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

    /// Waits for `ready`, for up to five seconds.
    fn eventually(mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while !ready() {
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn a_shared_recogniser_works_on_its_own_thread() {
        let recording = join(&[
            tone(4000, 0.3),
            silence(1000),
            tone(3500, 0.3),
            silence(1000),
            tone(2500, 0.3),
        ]);
        let gate = Arc::new(Mutex::new(()));
        let mut fixed = Fixed::detecting(&["first part.", "second part.", "the rest."]);
        fixed.gate = Some(gate.clone());
        let mut asr = Shared::new(fixed);
        assert_eq!(asr.name(), "fixed");
        let mut live = Live::default();
        let held = gate.lock().unwrap();
        // The piece is sent off, and the caller carries on at once.
        assert!(!live.advance(&recording[..4800 * MS], &mut asr, &HINT).unwrap());
        eventually(|| asr.busy());
        // While it is out, nothing more is looked at.
        assert!(!live.advance(&recording[..8800 * MS], &mut asr, &HINT).unwrap());
        assert_eq!(live.done(), 0);
        drop(held);
        let mut landed = false;
        eventually(|| {
            landed = live.advance(&recording[..8800 * MS], &mut asr, &HINT).unwrap();
            landed
        });
        assert_eq!(
            (live.done(), live.pieces()),
            (4400 * MS, &["first part.".to_string()][..])
        );
        // The next piece is still out when the recording ends: the end waits
        // for it, then recognises the rest.
        let pending = &recording[live.done()..9300 * MS];
        assert!(!live.advance(pending, &mut asr, &HINT).unwrap());
        let heard = live.finish(&recording, &mut asr, &HINT).unwrap().unwrap();
        assert_eq!(heard.raw, "first part. second part. the rest.");
        assert_eq!(heard.pieces, 3);
        let calls = asr.lock().heard.clone();
        assert_eq!(calls[1].3, "first part.");
        assert_eq!(calls[2].3, "first part. second part.");
        assert!(asr.heard().is_empty());
    }

    #[test]
    fn a_piece_that_fails_on_its_own_thread_is_tried_again() {
        let recording = join(&[tone(3500, 0.3), silence(1000)]);
        let mut fixed = Fixed::detecting(&[]);
        fixed.answers = vec![Err("busy".into()), Err("busy".into()), Ok("again".into())];
        let mut asr = Shared::new(fixed);
        let mut live = Live::default();
        assert!(!live.advance(&recording, &mut asr, &HINT).unwrap());
        let mut failed = None;
        eventually(|| {
            failed = live.advance(&recording, &mut asr, &HINT).err();
            failed.is_some()
        });
        assert_eq!((failed.unwrap().as_str(), live.done()), ("busy", 0));
        // Out again, and failing again, when the recording ends.
        live.looked = 0;
        assert!(!live.advance(&recording, &mut asr, &HINT).unwrap());
        assert_eq!(live.finish(&recording, &mut asr, &HINT).unwrap().unwrap().raw, "again");
    }

    /// Takes forever, unless told to stop.
    struct Patient;

    impl Recognizer for Patient {
        fn transcribe(&mut self, _: &[f32], _: &Hint) -> Result<String> {
            Ok("never".into())
        }
        fn transcribe_until(&mut self, _: &[f32], _: &Hint, stop: &AtomicBool) -> Result<String> {
            eventually(|| stop.load(Ordering::SeqCst));
            Err("stopped".into())
        }
        fn name(&self) -> &str {
            "patient"
        }
        fn detect(&mut self, audio: &[f32], _: f64) -> Option<Result<Vec<Range<usize>>>> {
            Some(Ok(crate::testing::sounding(audio)))
        }
    }

    #[test]
    fn a_cancelled_piece_is_told_to_stop_and_holds_nothing_up() {
        let recording = join(&[tone(4000, 0.3), silence(1000)]);
        let mut asr = Shared::new(Patient);
        assert_eq!(asr.lock().transcribe(&[], &HINT).unwrap(), "never");
        let mut live = Live::default();
        assert!(!live.advance(&recording, &mut asr, &HINT).unwrap());
        eventually(|| asr.busy());
        // A new recording meanwhile does not wait for the recogniser.
        let mut next = Live::default();
        assert!(!next.advance(&recording, &mut asr, &HINT).unwrap());
        assert_eq!(next.looked, 0);
        // Cancelling the first lets it go.
        drop(live);
        eventually(|| !asr.busy());
        assert!(!next.advance(&recording, &mut asr, &HINT).unwrap());
        assert!(next.flight.is_some());
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

    #[test]
    fn a_streaming_model_is_given_the_audio_as_it_comes() {
        let recording = tone(3500, 0.3);
        let mut asr = Talker::new(
            &["", "Hello there. How", "Hello there. How are you? I"],
            "Hello there. How are you? I am fine.",
        );
        let mut live = Live::default();
        // Too little to be worth feeding yet.
        assert!(!live.advance(&recording[..200 * MS], &mut asr, &HINT).unwrap());
        assert_eq!(asr.started, 0);
        // Nothing settled yet.
        assert!(!live.advance(&recording[..1000 * MS], &mut asr, &HINT).unwrap());
        assert_eq!((asr.started, live.done()), (1, 1000 * MS));
        // Each settled sentence becomes a piece, once the next one has begun.
        assert!(live.advance(&recording[1000 * MS..2000 * MS], &mut asr, &HINT).unwrap());
        assert!(live.advance(&recording[2000 * MS..3000 * MS], &mut asr, &HINT).unwrap());
        assert_eq!(live.pieces(), ["Hello there.", "How are you?"]);
        let heard = live.finish(&recording, &mut asr, &HINT).unwrap().unwrap();
        assert_eq!(heard.raw, "Hello there. How are you? I am fine.");
        assert_eq!(heard.parts, ["Hello there.", "How are you?", "I am fine."]);
        assert!(!heard.revised);
        assert_eq!((heard.speech_ms, heard.pieces), (3500, 1));
        // The stream was started once and given every sample once.
        assert_eq!((asr.started, asr.fed), (1, recording.len()));
        assert_eq!(asr.transcribe(&[], &HINT).unwrap(), "");
        assert_eq!(asr.name(), "talker");
    }

    #[test]
    fn a_stream_that_changes_its_mind_or_breaks_is_taken_from_the_start() {
        let recording = tone(2500, 0.3);
        // The model hears the start differently once it has heard the end.
        let mut asr = Talker::new(&["One. Two"], "Won. Two too.");
        let mut live = Live::default();
        assert!(live.advance(&recording[..1000 * MS], &mut asr, &HINT).unwrap());
        let heard = live.finish(&recording, &mut asr, &HINT).unwrap().unwrap();
        assert!(heard.revised);
        assert_eq!(heard.parts, ["Won. Two too."]);

        // A failed feed gives the stream up; the whole recording is
        // recognised at the end instead.
        let mut asr = Talker::new(&[], "never");
        asr.settles = vec![Err("The graphics card was lost".into())];
        asr.whole = "from the start".into();
        let mut live = Live::default();
        let failed = live.advance(&recording[..1000 * MS], &mut asr, &HINT).unwrap_err();
        assert_eq!(failed, "The graphics card was lost");
        assert!(!live.advance(&recording[..2000 * MS], &mut asr, &HINT).unwrap());
        let heard = live.finish(&recording, &mut asr, &HINT).unwrap().unwrap();
        assert_eq!((heard.raw.as_str(), heard.revised), ("from the start", false));

        // So does a failure at the very end, after pieces were handed out.
        let mut asr = Talker::new(&["Done. And"], "");
        asr.ending = Err("The graphics card was lost".into());
        asr.whole = "Done and more.".into();
        let mut live = Live::default();
        assert!(live.advance(&recording[..1000 * MS], &mut asr, &HINT).unwrap());
        let heard = live.finish(&recording, &mut asr, &HINT).unwrap().unwrap();
        assert_eq!((heard.raw.as_str(), heard.revised), ("Done and more.", true));
    }

    #[test]
    fn silence_streams_to_nothing_and_a_phantom_is_dropped() {
        let quiet = silence(1500);
        let mut asr = Talker::new(&[""], "");
        let mut live = Live::default();
        assert!(!live.advance(&quiet, &mut asr, &HINT).unwrap());
        assert_eq!(live.finish(&quiet, &mut asr, &HINT).unwrap(), None);
        // Without a detector, all the audio counts as speech: not enough
        // for "Thank you." said in earnest.
        let short = tone(600, 0.3);
        let mut asr = Talker::new(&[""], "Thank you.");
        asr.detects = false;
        let mut live = Live::default();
        assert!(!live.advance(&short, &mut asr, &HINT).unwrap());
        let heard = live.finish(&short, &mut asr, &HINT).unwrap().unwrap();
        assert_eq!((heard.raw.as_str(), heard.speech_ms), ("", 600));
        assert!(heard.parts.is_empty());
    }

    #[test]
    fn a_shared_streaming_model_is_fed_on_its_own_thread() {
        let recording = tone(2000, 0.3);
        let gate = Arc::new(Mutex::new(()));
        let mut talker = Talker::new(
            &["Fed elsewhere. Then", "Fed elsewhere. Then more"],
            "Fed elsewhere. Then more.",
        );
        talker.gate = Some(gate.clone());
        let mut asr = Shared::new(talker);
        let mut live = Live::default();
        let held = gate.lock().unwrap();
        assert!(!live.advance(&recording[..1000 * MS], &mut asr, &HINT).unwrap());
        // Still being fed: nothing to collect yet, and nothing new is sent.
        assert!(!live.advance(&recording[..1000 * MS], &mut asr, &HINT).unwrap());
        assert_eq!(live.done(), 0);
        drop(held);
        let mut settled = false;
        eventually(|| {
            settled = live
                .advance(&recording[live.done()..1000 * MS], &mut asr, &HINT)
                .unwrap();
            settled
        });
        assert_eq!(
            (live.done(), live.pieces()),
            (1000 * MS, &["Fed elsewhere.".to_string()][..])
        );
        // A release while audio is still being fed waits for it.
        assert!(!live.advance(&recording[1000 * MS..], &mut asr, &HINT).unwrap());
        let heard = live.finish(&recording, &mut asr, &HINT).unwrap().unwrap();
        assert_eq!(heard.parts, ["Fed elsewhere.", "Then more."]);
        assert_eq!(asr.lock().fed, recording.len());
    }

    #[test]
    fn a_stream_that_falls_behind_is_given_up() {
        let recording = tone(6000, 0.3);
        let gate = Arc::new(Mutex::new(()));
        let mut talker = Talker::new(&["Too slow. To"], "never");
        talker.whole = "in one go".into();
        talker.gate = Some(gate.clone());
        let mut asr = Shared::new(talker);
        let mut live = Live::default();
        let held = gate.lock().unwrap();
        assert!(!live.advance(&recording[..1000 * MS], &mut asr, &HINT).unwrap());
        // The model is still on the first second when the speaker is five in.
        assert!(!live.advance(&recording[..5000 * MS], &mut asr, &HINT).unwrap());
        drop(held);
        let heard = live.finish(&recording, &mut asr, &HINT).unwrap().unwrap();
        assert_eq!((heard.raw.as_str(), heard.revised), ("in one go", false));
        // Falling behind is noticed at the end too.
        let mut asr = Talker::new(&["Slow."], "never");
        asr.whole = "at the end".into();
        let mut live = Live::default();
        assert!(!live.advance(&recording[..1000 * MS], &mut asr, &HINT).unwrap());
        let heard = live.finish(&recording, &mut asr, &HINT).unwrap().unwrap();
        assert_eq!(heard.raw, "at the end");
    }

    #[test]
    fn a_sentence_ends_only_once_the_next_word_has_begun() {
        assert_eq!(sentences_end("Hi. There", 0), Some(3));
        assert_eq!(sentences_end("Hi. There", 3), None);
        assert_eq!(sentences_end("Hi.", 0), None);
        assert_eq!(sentences_end("3.5 kg", 0), None);
        assert_eq!(sentences_end("Hi", 9), None);
    }
}
