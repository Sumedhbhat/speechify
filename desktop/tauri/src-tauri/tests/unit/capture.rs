use super::capture::CapturedAudio;
use super::RATE;

#[test]
fn supported_microphone_formats_produce_the_same_normalized_audio() {
    let expected = [-1., -0.5, 0., 0.5, 32767. / 32768.];
    let float = CapturedAudio::new(RATE, 1);
    float.push(&expected);
    let signed = CapturedAudio::new(RATE, 1);
    signed.push(&[-32768i16, -16384, 0, 16384, 32767]);
    let unsigned = CapturedAudio::new(RATE, 1);
    unsigned.push(&[0u16, 16384, 32768, 49152, 65535]);
    for clip in [
        float.take().unwrap(),
        signed.take().unwrap(),
        unsigned.take().unwrap(),
    ] {
        assert_eq!(clip, expected);
    }
}

#[test]
fn audio_is_consumed_once_and_the_meter_tracks_recent_samples() {
    let audio = CapturedAudio::new(RATE, 1);
    assert_eq!(audio.level(), 0.);
    assert!(audio.take().unwrap().is_empty());
    audio.push(&vec![0.25f32; 2048]);
    assert!((audio.level() - 0.25).abs() < 0.0001);
    audio.push(&vec![-0.5f32; 1024]);
    assert!((audio.level() - 0.5).abs() < 0.0001);
    let clip = audio.take().unwrap();
    assert_eq!(clip.len(), 3072);
    assert_eq!(clip[0], 0.25);
    assert_eq!(clip[3071], -0.5);
    assert!(audio.take().unwrap().is_empty());
    assert_eq!(audio.level(), 0.);
}

#[test]
fn keep_open_retains_only_300_ms_before_the_next_recording() {
    let audio = CapturedAudio::new(48000, 2);
    audio.push(&vec![0.25f32; 48000 * 2]);
    audio.preroll();
    audio.push(&vec![0.75f32; 4800 * 2]);
    let clip = audio.take().unwrap();
    assert_eq!(clip.len(), 6400);
    assert!((clip[100] - 0.25).abs() < 0.001);
    assert!((clip[6300] - 0.75).abs() < 0.001);
    audio.push(&[0.5f32; 100]);
    audio.preroll();
    assert_eq!(audio.take().unwrap().len(), 16);
}

#[test]
fn cancellation_discards_audio_and_microphone_errors_are_reported() {
    let audio = CapturedAudio::new(RATE, 1);
    audio.push(&[0.3f32; 160]);
    audio.clear();
    assert!(audio.take().unwrap().is_empty());
    audio.fail("Microphone disconnected".into());
    assert_eq!(audio.take().unwrap_err(), "Microphone disconnected");
    assert!(audio.take_error().is_none());
    audio.push(&[0.4f32; 160]);
    assert_eq!(audio.take().unwrap(), vec![0.4; 160]);
}

#[test]
fn microphone_callback_and_recorder_share_the_same_owned_buffer() {
    let audio = CapturedAudio::new(RATE, 1);
    let callback = audio.clone();
    let producer = std::thread::spawn(move || {
        callback.push(&[0.2f32; 480]);
        callback.push(&[0.3f32; 480]);
    });
    producer.join().unwrap();
    let clip = audio.take().unwrap();
    assert_eq!(clip[..480], [0.2; 480]);
    assert_eq!(clip[480..], [0.3; 480]);
}

#[test]
fn exceeding_ten_minutes_reports_an_error_and_never_returns_a_truncated_success() {
    let audio = CapturedAudio::new(RATE, 1);
    audio.push(&vec![0f32; RATE as usize * 600]);
    assert!(audio.take_error().is_none());
    audio.push(&[0.1f32]);
    assert!(audio.take().unwrap_err().contains("ten-minute limit"));
    audio.clear();
    assert_eq!(audio.level(), 0.);
}
