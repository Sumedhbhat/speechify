//! Own the microphone, pre-roll buffer and recording indicator.
use super::{CaptureCommand, Dictation};
use crate::{audio::capture::Capture, config::Result, keys::Action, service::Service};
use serde_json::json;
use std::{
    sync::{
        atomic::Ordering,
        mpsc::{Receiver, SyncSender},
        Arc,
    },
    time::{Duration, Instant},
};

pub(super) fn run(
    service: Arc<Service>,
    capture_rx: Receiver<CaptureCommand>,
    tx: SyncSender<Dictation>,
    indicator: impl Fn(bool),
    open_capture: impl Fn(&str) -> Result<Capture>,
) {
    let mut capture: Option<Capture> = None;
    let mut active_target = 0;
    let mut active_capture = false;
    let mut active_generation = 0;
    let mut last_input = String::new();
    let mut hide_at = None;
    while !service.stopping.load(Ordering::SeqCst) {
        let command = capture_rx.recv_timeout(Duration::from_millis(15));
        let cfg = service.config.lock().unwrap().clone();
        let keep = cfg["keep_mic_open"] == true;
        let input = cfg["input_device"].as_str().unwrap_or("");
        if last_input != input {
            capture = None;
            last_input = input.into();
        }
        if let Ok(CaptureCommand {
            action,
            target,
            generation,
        }) = command
        {
            match action {
                Action::Start => {
                    active_capture = true;
                    active_target = target;
                    active_generation = generation;
                    if capture.is_none() {
                        match open_capture(input) {
                            Ok(c) => {
                                service.state.lock().unwrap()["actual_input"] =
                                    json!(&c.actual_device);
                                capture = Some(c);
                            }
                            Err(e) => {
                                service.error(e);
                                service.recording.store(false, Ordering::SeqCst);
                            }
                        }
                    }
                    if capture.is_some() && service.recording.load(Ordering::SeqCst) {
                        service.set_stage("Recording");
                        service.log(
                            "microphone_ready",
                            json!({"clock_ms":crate::store::now()*1000.}),
                        );
                        indicator(true);
                    }
                }
                Action::Cancel => {
                    active_capture = false;
                    if keep {
                        if let Some(c) = &capture {
                            c.audio.clear();
                        }
                    } else {
                        capture = None;
                    }
                    service.set_stage("Ready");
                    indicator(false);
                }
                Action::Submit => {
                    if !active_capture {
                        continue;
                    }
                    active_capture = false;
                    if let Some(c) = &capture {
                        let taken = c.audio.take();
                        let stream_failed = taken.is_err();
                        let queued = taken.and_then(|audio| {
                            if audio.len() < 160 {
                                return Err(
                                    "Microphone was not ready before release; try holding longer"
                                        .into(),
                                );
                            }
                            tx.try_send(Dictation {
                                audio,
                                target: active_target,
                                generation: active_generation,
                            })
                            .map_err(|_| {
                                "Dictation queue is full (4 clips); wait for transcription".into()
                            })
                        });
                        match queued {
                            Ok(()) => service.set_stage("Queued"),
                            Err(e) => service.error(e),
                        }
                        if stream_failed {
                            capture = None;
                        }
                    }
                    if !keep {
                        capture = None;
                    }
                    hide_at = Some(Instant::now() + Duration::from_secs(3));
                }
            }
        }
        if keep && capture.is_none() && !service.recording.load(Ordering::SeqCst) {
            match open_capture(input) {
                Ok(c) => {
                    service.state.lock().unwrap()["actual_input"] = json!(&c.actual_device);
                    capture = Some(c);
                }
                Err(e) => {
                    service.error(e);
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }
        if !keep && !active_capture {
            capture = None;
        }
        if let Some(c) = &capture {
            if !active_capture {
                c.audio.preroll();
            }
            service.state.lock().unwrap()["level"] = json!(c.audio.level());
            let err = c.audio.take_error();
            if let Some(e) = err {
                service.error(e);
                capture = None;
                service.recording.store(false, Ordering::SeqCst);
            }
        } else {
            service.state.lock().unwrap()["level"] = json!(0.);
        }
        if hide_at.is_some_and(|t| Instant::now() > t)
            && ["Ready", "Pasted", "Error", "Cancelled"].contains(
                &service.state.lock().unwrap()["stage"]
                    .as_str()
                    .unwrap_or(""),
            )
            && !service.recording.load(Ordering::SeqCst)
        {
            indicator(false);
            hide_at = None;
        }
    }
}
