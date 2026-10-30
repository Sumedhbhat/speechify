//! Wire three independently owned loops with bounded queues.
mod inference;
mod keyboard;
mod recording;

use crate::{audio::capture::Capture, config::Result, keys::Action, service::Service};
use std::{
    sync::{mpsc, Arc},
    thread::JoinHandle,
};

pub(crate) struct CaptureCommand {
    pub action: Action,
    pub target: u64,
    pub generation: u64,
}

struct Dictation {
    audio: Vec<f32>,
    target: u64,
    generation: u64,
}

pub(super) fn start(
    service: Arc<Service>,
    navigate: impl Fn(&str) + Send + 'static,
    indicator: impl Fn(bool) + Send + 'static,
    open_capture: impl Fn(&str) -> Result<Capture> + Send + 'static,
) -> [JoinHandle<()>; 3] {
    let (dictation_tx, dictation_rx) = mpsc::sync_channel(4);
    let (capture_tx, capture_rx) = mpsc::sync_channel(32);
    *service.capture_tx.lock().unwrap() = Some(capture_tx.clone());

    let infer = service.clone();
    let inference = std::thread::spawn(move || inference::run(infer, dictation_rx));
    let recorder = service.clone();
    let recording = std::thread::spawn(move || {
        recording::run(recorder, capture_rx, dictation_tx, indicator, open_capture)
    });
    let keyboard = std::thread::spawn(move || keyboard::run(service, capture_tx, navigate));
    [inference, recording, keyboard]
}
