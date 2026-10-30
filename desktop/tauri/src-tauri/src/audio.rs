use crate::config::Result;
use std::path::Path;
pub const RATE: u32 = 16000;
#[cfg(all(test, feature = "desktop"))]
#[path = "../tests/unit/capture.rs"]
mod behavior;
pub fn resample(input: &[f32], rate: u32, channels: usize) -> Vec<f32> {
    if rate == 0 || channels == 0 {
        return vec![];
    }
    let mono: Vec<f32> = input
        .chunks_exact(channels)
        .map(|s| s.iter().sum::<f32>() / channels as f32)
        .collect();
    if rate == RATE {
        return mono;
    }
    let count = (mono.len() as u64 * RATE as u64 / rate as u64) as usize;
    let step = rate as f64 / RATE as f64;
    // Windowed-sinc low-pass interpolation avoids aliasing when downsampling.
    let cutoff = (RATE as f64 / rate as f64).min(1.) * 0.94;
    let radius = 24i64;
    (0..count)
        .map(|i| {
            let pos = i as f64 * step;
            let center = pos.floor() as i64;
            let (mut sum, mut weight) = (0., 0.);
            for j in center - radius..=center + radius {
                if j < 0 || j >= mono.len() as i64 {
                    continue;
                }
                let d = pos - j as f64;
                let x = std::f64::consts::PI * d * cutoff;
                let sinc = if x.abs() < 1e-8 { 1. } else { x.sin() / x };
                let win = 0.5 + 0.5 * (std::f64::consts::PI * d / (radius as f64 + 1.)).cos();
                let w = sinc * win * cutoff;
                sum += mono[j as usize] as f64 * w;
                weight += w;
            }
            if weight.abs() > 1e-8 {
                (sum / weight) as f32
            } else {
                0.
            }
        })
        .collect()
}
pub fn load(path: &Path) -> Result<Vec<f32>> {
    let mut r = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let s = r.spec();
    if s.sample_rate == 0
        || s.channels == 0
        || u64::from(r.duration()) > u64::from(s.sample_rate) * 600
    {
        return Err("WAV must be at most 10 minutes".into());
    }
    let samples = if s.sample_format == hound::SampleFormat::Float {
        r.samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    } else {
        let scale = 2f32.powi(s.bits_per_sample as i32 - 1);
        r.samples::<i32>()
            .map(|x| x.map(|v| v as f32 / scale))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    if samples.iter().any(|x| !x.is_finite()) {
        return Err("Non-finite WAV samples".into());
    }
    Ok(resample(&samples, s.sample_rate, s.channels as usize))
}
pub fn save(path: &Path, audio: &[f32]) -> Result<()> {
    let mut w = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 1,
            sample_rate: RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .map_err(|e| e.to_string())?;
    for s in audio {
        w.write_sample((s.clamp(-1., 1.) * 32767.) as i16)
            .map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())
}
#[cfg(feature = "desktop")]
pub mod capture {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::{Arc, Mutex};
    pub fn devices() -> Result<Vec<String>> {
        cpal::default_host()
            .input_devices()
            .map_err(|e| e.to_string())?
            .map(|d| {
                d.description()
                    .map(|x| x.name().to_string())
                    .map_err(|e| e.to_string())
            })
            .collect()
    }
    pub struct Capture {
        pub _stream: cpal::Stream,
        pub actual_device: String,
        pub audio: CapturedAudio,
    }
    #[derive(Clone)]
    pub struct CapturedAudio {
        data: Arc<Mutex<Vec<f32>>>,
        error: Arc<Mutex<Option<String>>>,
        rate: u32,
        channels: usize,
    }
    impl Capture {
        pub fn open(name: &str) -> Result<Self> {
            let host = cpal::default_host();
            let device = if name.is_empty() {
                host.default_input_device()
            } else {
                host.input_devices()
                    .map_err(|e| e.to_string())?
                    .find(|d| d.description().is_ok_and(|x| x.name() == name))
            }
            .or_else(|| host.default_input_device())
            .ok_or("No microphone found; connect an input and check OS microphone permission")?;
            let actual_device = device
                .description()
                .map(|d| d.name().to_string())
                .unwrap_or_else(|_| "System default microphone".into());
            let config = device.default_input_config().map_err(|e| e.to_string())?;
            let rate = config.sample_rate();
            let channels = config.channels() as usize;
            let audio = CapturedAudio::new(rate, channels);
            macro_rules! stream {
                ($t:ty) => {{
                    let captured = audio.clone();
                    device
                        .build_input_stream(
                            config.clone().into(),
                            move |buf: &[$t], _: &cpal::InputCallbackInfo| {
                                captured.push(buf);
                            },
                            {
                                let captured = audio.clone();
                                move |err| {
                                    captured.fail(format!(
                                        "Microphone disconnected or unavailable: {err}"
                                    ));
                                }
                            },
                            None,
                        )
                        .map_err(|e| e.to_string())?
                }};
            }
            let stream = match config.sample_format() {
                cpal::SampleFormat::F32 => stream!(f32),
                cpal::SampleFormat::I16 => stream!(i16),
                cpal::SampleFormat::U16 => stream!(u16),
                f => return Err(format!("Unsupported microphone sample format: {f}")),
            };
            stream.play().map_err(|e| e.to_string())?;
            Ok(Self {
                _stream: stream,
                actual_device,
                audio,
            })
        }
    }
    impl CapturedAudio {
        pub fn new(rate: u32, channels: usize) -> Self {
            Self {
                data: Arc::new(Mutex::new(Vec::new())),
                error: Arc::new(Mutex::new(None)),
                rate,
                channels,
            }
        }
        pub fn push<T: cpal::Sample>(&self, samples: &[T])
        where
            f32: cpal::FromSample<T>,
        {
            let mut data = self.data.lock().unwrap();
            if data.len() + samples.len() <= self.rate as usize * self.channels * 600 {
                data.extend(samples.iter().map(|sample| sample.to_sample::<f32>()));
            } else {
                self.fail("Recording exceeds ten-minute limit".into());
            }
        }
        pub fn fail(&self, message: String) {
            *self.error.lock().unwrap() = Some(message);
        }
        pub fn take_error(&self) -> Option<String> {
            self.error.lock().unwrap().take()
        }
        pub fn clear(&self) {
            self.data.lock().unwrap().clear();
        }
        pub fn take(&self) -> Result<Vec<f32>> {
            if let Some(e) = self.take_error() {
                return Err(e);
            }
            Ok(resample(
                &std::mem::take(&mut *self.data.lock().unwrap()),
                self.rate,
                self.channels,
            ))
        }
        pub fn preroll(&self) {
            let mut d = self.data.lock().unwrap();
            let keep = self.rate as usize * self.channels * 3 / 10;
            if d.len() > keep {
                let n = d.len() - keep;
                d.drain(..n);
            }
        }
        pub fn level(&self) -> f32 {
            let d = self.data.lock().unwrap();
            let tail = &d[d.len().saturating_sub(1024)..];
            if tail.is_empty() {
                0.
            } else {
                (tail.iter().map(|x| x * x).sum::<f32>() / tail.len() as f32).sqrt()
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resampling() {
        let input = vec![0.4; 48000 * 2];
        let out = resample(&input, 48000, 2);
        assert_eq!(out.len(), 16000);
        assert!((out[500] - 0.4).abs() < 0.001);
    }
    #[test]
    fn anti_alias() {
        let input: Vec<_> = (0..48000)
            .map(|i| (2. * std::f32::consts::PI * 12000. * i as f32 / 48000.).sin())
            .collect();
        let out = resample(&input, 48000, 1);
        assert!(out[100..15000].iter().map(|x| x.abs()).sum::<f32>() / 14900. < 0.02);
    }
}
