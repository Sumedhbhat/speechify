//! Translate native key transitions into recording and navigation commands.
use super::CaptureCommand;
use crate::{
    keys::{Action, Hold},
    platform,
    service::Service,
};
use std::{
    collections::BTreeSet,
    sync::{atomic::Ordering, mpsc::SyncSender, Arc},
    time::{Duration, Instant},
};

pub(super) fn run(
    service: Arc<Service>,
    capture_tx: SyncSender<CaptureCommand>,
    navigate: impl Fn(&str),
) {
    let poll = match platform::Poller::new() {
        Ok(p) => p,
        Err(e) => {
            service.error(e);
            return;
        }
    };
    let mut cfg = service.config.lock().unwrap().clone();
    let mut hold = Hold::new(
        cfg["ptt_key"].as_str().unwrap(),
        cfg["min_hold_ms"].as_u64().unwrap(),
    );
    let mut nav: Vec<_> = ["correct_hotkey", "history_hotkey", "models_hotkey"]
        .into_iter()
        .map(|k| Hold::new(cfg[k].as_str().unwrap(), 0))
        .collect();
    let mut previous = BTreeSet::new();
    let clock = Instant::now();
    while !service.stopping.load(Ordering::SeqCst) {
        let current = service.config.lock().unwrap().clone();
        if current != cfg {
            cfg = current;
            hold = Hold::new(
                cfg["ptt_key"].as_str().unwrap(),
                cfg["min_hold_ms"].as_u64().unwrap(),
            );
            nav = ["correct_hotkey", "history_hotkey", "models_hotkey"]
                .into_iter()
                .map(|k| Hold::new(cfg[k].as_str().unwrap(), 0))
                .collect();
            previous.clear();
        }
        let down = match poll.down() {
            Ok(d) => d,
            Err(e) => {
                service.recording.store(false, Ordering::SeqCst);
                let _ = capture_tx.try_send(CaptureCommand {
                    action: Action::Cancel,
                    target: 0,
                    generation: service.generation.load(Ordering::SeqCst),
                });
                service.error(e);
                break;
            }
        };
        let events: Vec<_> = previous
            .difference(&down)
            .map(|k| (k.clone(), false))
            .chain(down.difference(&previous).map(|k| (k.clone(), true)))
            .collect();
        for (key, pressed) in events {
            for (i, n) in nav.iter_mut().enumerate() {
                if n.event(&key, pressed, clock.elapsed().as_millis() as u64) == Some(Action::Start)
                {
                    navigate(["History", "History", "Models"][i]);
                }
            }
            if service.manual_started.lock().unwrap().is_some() {
                continue;
            }
            if let Some(action) = hold.event(&key, pressed, clock.elapsed().as_millis() as u64) {
                let mut target = 0;
                if action == Action::Start {
                    *service.last_activity.lock().unwrap() = Instant::now();
                    if service.learning_busy.load(Ordering::SeqCst) {
                        service.cancel_work();
                    }
                    target = poll.focus().unwrap_or(0);
                    service.recording.store(true, Ordering::SeqCst);
                } else {
                    service.recording.store(false, Ordering::SeqCst);
                }
                if capture_tx
                    .try_send(CaptureCommand {
                        action: action,
                        target: target,
                        generation: service.generation.load(Ordering::SeqCst),
                    })
                    .is_err()
                {
                    service.recording.store(false, Ordering::SeqCst);
                    service.error("Microphone command queue full; restart Speechify");
                }
            }
        }
        previous = down;
        std::thread::sleep(Duration::from_millis(8));
    }
}
