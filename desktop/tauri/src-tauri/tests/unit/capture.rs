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

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Requires the synthetic ALSA configuration supplied by test-coverage.sh"]
fn selected_and_default_microphones_open_capture_and_close_through_the_real_driver() {
    assert_eq!(
        std::env::var("SPEECHIFY_TEST_SYNTHETIC_ALSA").as_deref(),
        Ok("1")
    );
    let devices = super::capture::devices().unwrap();
    let selected = devices
        .iter()
        .find(|name| name.as_str() == "Speechify test input")
        .expect("synthetic ALSA microphone was not enumerated");
    for name in ["", selected.as_str(), "A disconnected microphone"] {
        let capture = super::capture::Capture::open(name).unwrap();
        assert!(!capture.actual_device.is_empty());
        let mut clip = Vec::new();
        crate::behavior_tests::wait_until(|| {
            clip = capture.audio.take().unwrap();
            !clip.is_empty()
        });
        assert!(clip.iter().all(|sample| sample.is_finite()));
        assert!(capture.audio.level().is_finite());
        capture.audio.preroll();
        capture.audio.clear();
        drop(capture);
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Requires the synthetic ALSA driver supplied by test-coverage.sh"]
fn driver_sample_formats_and_disconnects_follow_the_same_capture_contract() {
    assert_eq!(
        std::env::var("SPEECHIFY_TEST_SYNTHETIC_ALSA").as_deref(),
        Ok("1")
    );
    for kind in ["float", "signed", "unsigned"] {
        let capture = super::capture::Capture::open(&format!("Speechify {kind} input")).unwrap();
        let mut audio = Vec::new();
        crate::behavior_tests::wait_until(|| {
            audio = capture.audio.take().unwrap();
            !audio.is_empty()
        });
        assert!(audio.iter().all(|sample| (*sample - 0.25).abs() < 0.001));
        drop(capture);
        let capture =
            super::capture::Capture::open(&format!("Speechify {kind} disconnect")).unwrap();
        let mut error = None;
        crate::behavior_tests::wait_until(|| {
            error = capture.audio.take_error();
            error.is_some()
        });
        assert!(error
            .unwrap()
            .contains("Microphone disconnected or unavailable"));
        assert!(super::capture::Capture::open(&format!("Speechify {kind} setup error")).is_err());
    }
    let unsupported = super::capture::Capture::open("Speechify unsupported input")
        .err()
        .unwrap();
    assert!(unsupported.contains("Unsupported microphone sample format"));
    assert!(super::capture::Capture::open("Speechify start error").is_err());
}
