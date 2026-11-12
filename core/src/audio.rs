//! Sample conversion, silence trimming, WAV files and the bar's level meter.

use crate::Result;

/// Everything downstream works on 16 kHz mono.
pub const RATE: u32 = 16_000;
const FRAME: usize = 320; // 20 ms
/// Speech shorter than this is a cough or a key click.
pub const MIN_SPEECH_MS: u64 = 160;

/// Mixes to mono and resamples to 16 kHz by linear interpolation, which
/// is plenty for speech recognition and costs almost nothing.
pub fn to_mono_16k(samples: &[f32], channels: u16, rate: u32) -> Vec<f32> {
    let channels = channels.max(1) as usize;
    let mono: Vec<f32> = samples
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect();
    if rate == RATE || mono.is_empty() || rate == 0 {
        return mono;
    }
    let step = rate as f64 / RATE as f64;
    let length = (mono.len() as f64 / step) as usize;
    (0..length)
        .map(|i| {
            let position = i as f64 * step;
            let index = position as usize;
            let next = mono[(index + 1).min(mono.len() - 1)];
            mono[index] + (next - mono[index]) * (position - index as f64) as f32
        })
        .collect()
}

fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt()
}

/// Loudness from 0 to 1 for the bar's animation: -50 dB and below is 0.
pub fn level(samples: &[f32]) -> f32 {
    let db = 20.0 * rms(samples).max(1e-6).log10();
    ((db + 50.0) / 40.0).clamp(0.0, 1.0)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Trimmed {
    pub samples: Vec<f32>,
    /// Time spent speaking, used for words-per-minute.
    pub speech_ms: u64,
}

/// Cuts leading and trailing silence. Returns no samples when nobody spoke,
/// which also keeps the recogniser from inventing words out of noise.
pub fn trim(samples: &[f32]) -> Trimmed {
    let energies: Vec<f32> = samples.chunks(FRAME).map(rms).collect();
    let mut sorted = energies.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let noise = sorted.get(sorted.len() / 10).copied().unwrap_or(0.0);
    let peak = sorted.last().copied().unwrap_or(0.0);
    // Quiet speech needs contrast with the room, not a fixed microphone gain.
    // Retain the ceiling for recordings spoken throughout at normal volume.
    let threshold = (noise * 3.0).min(0.03).max(peak * 0.12).max(1e-6);
    let speech: Vec<usize> = (0..energies.len()).filter(|i| energies[*i] > threshold).collect();
    let speech_ms = speech.len() as u64 * 20;
    let (Some(first), Some(last)) = (speech.first(), speech.last()) else {
        return Trimmed {
            samples: vec![],
            speech_ms: 0,
        };
    };
    // Isolated clicks can accumulate 160 ms over a long recording. Require
    // at least one sustained 80 ms sound as well as enough total speech.
    let sustained = energies
        .windows(4)
        .any(|run| run.iter().all(|energy| *energy > threshold));
    if speech_ms < MIN_SPEECH_MS || !sustained {
        return Trimmed {
            samples: vec![],
            speech_ms: 0,
        };
    }
    // Keep a little room so word edges are not clipped.
    let start = first.saturating_sub(10) * FRAME;
    let end = ((last + 15) * FRAME).min(samples.len());
    Trimmed {
        // Raise only quiet, already-detected speech. Bound both amplification
        // and sample peaks; silence/noise is never amplified into speech.
        samples: {
            let amplitude = samples[start..end].iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
            let gain = (0.02 / peak).min(16.0).min(0.9 / amplitude).max(1.0);
            samples[start..end].iter().map(|s| s * gain).collect()
        },
        speech_ms,
    }
}

/// 16-bit mono WAV at 16 kHz.
pub fn encode_wav(samples: &[f32]) -> Vec<u8> {
    let data = samples.len() as u32 * 2;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

/// Reads 16-bit PCM WAV of any rate and channel count into 16 kHz mono.
pub fn decode_wav(bytes: &[u8]) -> Result<Vec<f32>> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("Not a WAV file".into());
    }
    let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let (mut at, mut format) = (12, None);
    while at + 8 <= bytes.len() {
        let size = u32_at(at + 4) as usize;
        let body = at + 8;
        let end = (body + size).min(bytes.len());
        match &bytes[at..at + 4] {
            b"fmt " if size >= 16 && end - body >= 16 => {
                format = Some((u16_at(body), u16_at(body + 2), u32_at(body + 4), u16_at(body + 14)));
            }
            b"data" => {
                let Some((1, channels, rate, 16)) = format else {
                    return Err("Only 16-bit PCM WAV files are supported".into());
                };
                let samples: Vec<f32> = bytes[body..end]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| i16::from_le_bytes([pair[0], pair[1]]) as f32 / 32768.0)
                    .collect();
                return Ok(to_mono_16k(&samples, channels, rate));
            }
            _ => {}
        }
        at = body + size + (size & 1);
    }
    Err("The WAV file has no audio".into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chime {
    Start,
    Stop,
    Error,
}

/// A short two-note cue, synthesised so the app ships no sound files.
pub fn chime(kind: Chime, rate: u32) -> Vec<f32> {
    let notes: [f32; 2] = match kind {
        Chime::Start => [587.33, 880.0],
        Chime::Stop => [880.0, 587.33],
        Chime::Error => [311.13, 233.08],
    };
    let per_note = (rate as f32 * 0.07) as usize;
    let mut out = Vec::with_capacity(per_note * 2);
    for frequency in notes {
        for i in 0..per_note {
            let t = i as f32 / rate as f32;
            // Fast attack, smooth decay: a soft "tick" rather than a beep.
            let envelope = (i as f32 / 60.0).min(1.0) * (1.0 - i as f32 / per_note as f32).powi(2);
            out.push((t * frequency * std::f32::consts::TAU).sin() * envelope * 0.25);
        }
    }
    out
}

#[cfg(test)]
pub(crate) fn tone(ms: usize, amplitude: f32) -> Vec<f32> {
    (0..ms * 16).map(|i| (i as f32 * 0.05).sin() * amplitude).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixes_and_resamples() {
        assert_eq!(to_mono_16k(&[0.2, 0.4, -1.0, 1.0], 2, RATE), vec![0.3, 0.0]);
        let down = to_mono_16k(&vec![0.5; 48_000], 1, 48_000);
        assert_eq!(down.len(), 16_000);
        assert!(down.iter().all(|s| (*s - 0.5).abs() < 1e-6));
        let up = to_mono_16k(&[0.0, 1.0], 1, 8_000);
        assert_eq!(up, vec![0.0, 0.5, 1.0, 1.0]);
        assert!(to_mono_16k(&[], 0, 44_100).is_empty());
        assert_eq!(to_mono_16k(&[0.1], 1, 0), vec![0.1]);
    }

    #[test]
    fn level_tracks_loudness() {
        assert_eq!(level(&[]), 0.0);
        assert_eq!(level(&[0.0; 100]), 0.0);
        assert_eq!(level(&[1.0; 100]), 1.0);
        let quiet = level(&tone(50, 0.02));
        let loud = level(&tone(50, 0.3));
        assert!(0.0 < quiet && quiet < loud && loud < 1.0);
    }

    #[test]
    fn trims_to_the_spoken_part() {
        let mut clip = vec![0.0005; 16_000];
        clip.extend(tone(1000, 0.3));
        clip.extend(vec![0.0005; 16_000]);
        let trimmed = trim(&clip);
        assert_eq!(trimmed.speech_ms, 1000);
        assert_eq!(trimmed.samples.len(), 16_000 + 10 * FRAME + 14 * FRAME);
        // Speech running to the very end is not cut short.
        let abrupt = trim(&tone(400, 0.3));
        assert_eq!(abrupt.samples.len(), 6400);
    }

    #[test]
    fn silence_and_clicks_yield_nothing() {
        assert_eq!(
            trim(&[]),
            Trimmed {
                samples: vec![],
                speech_ms: 0
            }
        );
        assert!(trim(&vec![0.001; 32_000]).samples.is_empty());
        let mut click = vec![0.0; 16_000];
        click.extend(tone(60, 0.5));
        click.extend(vec![0.0; 16_000]);
        assert!(trim(&click).samples.is_empty());
    }

    #[test]
    fn quiet_clear_speech_survives_independent_of_microphone_gain() {
        // The discarded evaluation recording peaked below 0.004 RMS. Model
        // its syllable envelope and a much quieter room without dataset audio.
        let mut clip = vec![0.00005; 16000];
        for amplitude in [0.001, 0.005, 0.002, 0.004] {
            clip.extend(tone(200, amplitude));
            clip.extend(vec![0.00005; 1600]);
        }
        clip.extend(vec![0.00005; 16000]);
        let quiet = trim(&clip);
        assert_eq!(quiet.speech_ms, 800);
        assert!(rms(&quiet.samples) > rms(&clip) * 3.0);
        let normal = trim(&clip.iter().map(|s| s * 100.0).collect::<Vec<_>>());
        assert_eq!(normal.speech_ms, quiet.speech_ms);
        assert_eq!(normal.samples.len(), quiet.samples.len());
    }

    #[test]
    fn room_noise_and_repeated_short_clicks_are_not_speech() {
        let room: Vec<f32> = (0..32000)
            .map(|i| ((i * 7919 % 997) as f32 / 997.0 - 0.5) * 0.006)
            .collect();
        assert!(trim(&room).samples.is_empty());
        let mut clicks = Vec::new();
        for _ in 0..12 {
            clicks.extend(tone(20, 0.1));
            clicks.extend(vec![0.0; 3200]);
        }
        assert!(trim(&clicks).samples.is_empty());
        assert!(trim(&vec![0.0; 32000]).samples.is_empty());
    }

    #[test]
    fn wav_round_trip() {
        let samples = vec![0.0, 0.5, -0.5, 1.0, -1.0];
        let decoded = decode_wav(&encode_wav(&samples)).unwrap();
        assert_eq!(decoded.len(), 5);
        assert!(decoded.iter().zip(&samples).all(|(a, b)| (a - b).abs() < 0.001));
    }

    #[test]
    fn reads_other_layouts_and_rejects_the_rest() {
        // Stereo 8 kHz with a LIST chunk of odd length before the data.
        let mut wav = b"RIFF\0\0\0\0WAVELIST\x03\0\0\0abc\0fmt \x10\0\0\0\x01\0\x02\0\x40\x1f\0\0\0\x7d\0\0\x04\0\x10\0data\x08\0\0\0".to_vec();
        wav.extend_from_slice(&[0, 0x40, 0, 0x40, 0, 0x20, 0, 0x20]);
        assert_eq!(decode_wav(&wav).unwrap(), vec![0.5, 0.375, 0.25, 0.25]);
        assert_eq!(decode_wav(b"nope").unwrap_err(), "Not a WAV file");
        assert_eq!(
            decode_wav(b"RIFF\0\0\0\0WAVE").unwrap_err(),
            "The WAV file has no audio"
        );
        assert_eq!(
            decode_wav(b"RIFF\0\0\0\0WAVEdata\x02\0\0\0\0\0").unwrap_err(),
            "Only 16-bit PCM WAV files are supported"
        );
        let float =
            b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x03\0\x01\0\x80\x3e\0\0\0\x7d\0\0\x04\0\x20\0data\x04\0\0\0\0\0\0\0";
        assert_eq!(
            decode_wav(float).unwrap_err(),
            "Only 16-bit PCM WAV files are supported"
        );
    }

    #[test]
    fn chimes_are_short_and_quiet() {
        for kind in [Chime::Start, Chime::Stop, Chime::Error] {
            let sound = chime(kind, 48_000);
            assert_eq!(sound.len(), 6720);
            assert!(sound.iter().all(|s| s.abs() <= 0.25));
            assert!(sound.iter().any(|s| s.abs() > 0.05));
        }
        assert_ne!(chime(Chime::Start, 48_000), chime(Chime::Stop, 48_000));
    }
}
