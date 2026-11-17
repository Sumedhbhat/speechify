//! Microphone capture and chime playback.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Called with each chunk of mono samples as it arrives, for the level meter.
pub type OnChunk = Box<dyn FnMut(&[f32]) + Send>;

fn name(device: &cpal::Device) -> String {
    device
        .description()
        .map(|description| description.name().to_string())
        .unwrap_or_default()
}

/// Names of the available input devices.
pub fn microphones() -> Vec<String> {
    let Ok(devices) = cpal::default_host().input_devices() else {
        return vec![];
    };
    devices
        .map(|device| name(&device))
        .filter(|name| !name.is_empty())
        .collect()
}

/// An open microphone. Dropping it closes the device, so the microphone is
/// only in use while the user is dictating.
pub struct Recorder {
    stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: u32,
    // CPAL backends have thread-affine lifetime requirements.
    _thread: PhantomData<Rc<()>>,
}

fn input<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    samples: Arc<Mutex<Vec<f32>>>,
    mut on_chunk: OnChunk,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels.max(1) as usize;
    let mut mono = Vec::new();
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            to_mono(data, channels, &mut mono);
            on_chunk(&mono);
            if let Ok(mut all) = samples.lock() {
                all.extend_from_slice(&mono);
            }
        },
        |_| {},
        None,
    )
}

fn to_mono<T: SizedSample>(data: &[T], channels: usize, mono: &mut Vec<f32>)
where
    f32: FromSample<T>,
{
    mono.clear();
    mono.extend(
        data.chunks_exact(channels.max(1))
            .map(|frame| frame.iter().map(|sample| f32::from_sample_(*sample)).sum::<f32>() / frame.len() as f32),
    );
}

impl Recorder {
    /// Opens the named microphone, or the system default when `microphone`
    /// is empty or no longer plugged in.
    pub fn start(microphone: &str, on_chunk: OnChunk) -> Result<Recorder, String> {
        let host = cpal::default_host();
        let named = host
            .input_devices()
            .ok()
            .and_then(|mut devices| devices.find(|device| !microphone.is_empty() && name(device) == microphone));
        let device = named
            .or_else(|| host.default_input_device())
            .ok_or("No microphone found. Plug one in or check your sound settings.")?;
        let supported = device
            .default_input_config()
            .map_err(|e| format!("The microphone could not be opened: {e}"))?;
        let config = supported.config();
        let rate = config.sample_rate;
        let samples = Arc::new(Mutex::new(Vec::with_capacity(rate as usize * 30)));
        let stream = match supported.sample_format() {
            SampleFormat::I8 => input::<i8>(&device, config, samples.clone(), on_chunk),
            SampleFormat::U8 => input::<u8>(&device, config, samples.clone(), on_chunk),
            SampleFormat::I24 => input::<cpal::I24>(&device, config, samples.clone(), on_chunk),
            SampleFormat::U24 => input::<cpal::U24>(&device, config, samples.clone(), on_chunk),
            SampleFormat::U32 => input::<u32>(&device, config, samples.clone(), on_chunk),
            SampleFormat::I64 => input::<i64>(&device, config, samples.clone(), on_chunk),
            SampleFormat::U64 => input::<u64>(&device, config, samples.clone(), on_chunk),
            SampleFormat::F64 => input::<f64>(&device, config, samples.clone(), on_chunk),
            SampleFormat::F32 => input::<f32>(&device, config, samples.clone(), on_chunk),
            SampleFormat::I16 => input::<i16>(&device, config, samples.clone(), on_chunk),
            SampleFormat::U16 => input::<u16>(&device, config, samples.clone(), on_chunk),
            SampleFormat::I32 => input::<i32>(&device, config, samples.clone(), on_chunk),
            other => return Err(format!("The microphone uses an unsupported format ({other:?}).")),
        }
        .map_err(|e| format!("The microphone could not be opened: {e}. Check that Speechify is allowed to use it."))?;
        stream
            .play()
            .map_err(|e| format!("The microphone could not be started: {e}"))?;
        Ok(Recorder {
            stream,
            samples,
            rate,
            _thread: PhantomData,
        })
    }

    /// Closes the microphone and returns the mono recording with its sample rate.
    pub fn stop(self) -> (Vec<f32>, u32) {
        drop(self.stream);
        let samples = self
            .samples
            .lock()
            .map(|mut all| std::mem::take(&mut *all))
            .unwrap_or_default();
        (samples, self.rate)
    }
}

fn output<T>(device: &cpal::Device, config: cpal::StreamConfig, sound: Vec<f32>) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels.max(1) as usize;
    let mut position = 0;
    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            for frame in data.chunks_mut(channels) {
                let value = sound.get(position).copied().unwrap_or(0.0);
                position += 1;
                frame.fill(T::from_sample_(value));
            }
        },
        |_| {},
        None,
    )
}

/// Plays a short sound on the default output without blocking. `make`
/// synthesises it for the device's sample rate. Failures are silent: a
/// missing speaker must never get in the way of dictation.
pub fn play(make: impl FnOnce(u32) -> Vec<f32> + Send + 'static) {
    std::thread::spawn(move || {
        let Some(device) = cpal::default_host().default_output_device() else {
            return;
        };
        let Ok(supported) = device.default_output_config() else {
            return;
        };
        let config = supported.config();
        let sound = make(config.sample_rate);
        let length =
            std::time::Duration::from_millis(sound.len() as u64 * 1000 / config.sample_rate.max(1) as u64 + 120);
        let stream = match supported.sample_format() {
            SampleFormat::I8 => output::<i8>(&device, config, sound),
            SampleFormat::U8 => output::<u8>(&device, config, sound),
            SampleFormat::I24 => output::<cpal::I24>(&device, config, sound),
            SampleFormat::U24 => output::<cpal::U24>(&device, config, sound),
            SampleFormat::I32 => output::<i32>(&device, config, sound),
            SampleFormat::U32 => output::<u32>(&device, config, sound),
            SampleFormat::I64 => output::<i64>(&device, config, sound),
            SampleFormat::U64 => output::<u64>(&device, config, sound),
            SampleFormat::F64 => output::<f64>(&device, config, sound),
            SampleFormat::F32 => output::<f32>(&device, config, sound),
            SampleFormat::I16 => output::<i16>(&device, config, sound),
            SampleFormat::U16 => output::<u16>(&device, config, sound),
            _ => return,
        };
        if let Ok(stream) = stream {
            if stream.play().is_ok() {
                std::thread::sleep(length);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::to_mono;

    #[test]
    fn integer_pcm_is_normalized_and_channels_are_averaged() {
        let mut mono = Vec::new();
        to_mono(&[i16::MIN, 0, 0, 0], 2, &mut mono);
        assert_eq!(mono, vec![-0.5, 0.0]);
        to_mono(&[32768_u16, 32768, 0, 0], 2, &mut mono);
        assert_eq!(mono, vec![0.0, -1.0]);
        to_mono(&[0.25_f64, 0.75, -0.25, -0.75, 99.0], 2, &mut mono);
        assert_eq!(mono, vec![0.5, -0.5]);
    }
}
