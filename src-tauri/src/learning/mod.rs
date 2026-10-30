//! Schedule idle learning; teachers and training each own their workflow.
mod power;
mod teacher;
mod training;

pub use power::mains;
#[cfg(test)]
use teacher::judge;
use teacher::teacher;
#[cfg(test)]
use training::gates;
use training::start_training_on_power;
pub use training::{rollback, start_training};

use crate::{
    service::Service,
    store::{label, now},
};
use serde_json::{json, Value};
use std::{
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
#[cfg(test)]
#[path = "../../tests/unit/learning.rs"]
mod behavior;
pub fn scheduler(service: Arc<Service>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut retry = Instant::now();
        while !service.stopping.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_secs(5));
            if service.stopping.load(Ordering::SeqCst) {
                break;
            }
            let cfg = service.config.lock().unwrap().clone();
            scheduler_tick(&service, &cfg, mains(), &mut retry);
        }
    })
}
fn scheduler_tick(service: &Arc<Service>, cfg: &Value, on_mains: bool, retry: &mut Instant) {
    let idle = service
        .last_activity
        .lock()
        .unwrap()
        .elapsed()
        .as_secs_f64()
        >= cfg["idle_minutes"].as_f64().unwrap_or(5.) * 60.;
    if !on_mains {
        if service.learning_busy.load(Ordering::SeqCst) {
            service.cancel_work();
        }
        return;
    }
    if service.downloading.load(Ordering::SeqCst)
        || !idle
        || service.recording.load(Ordering::SeqCst)
        || cfg["learn_auto"] != true
        || Instant::now() < *retry
        || service.learning_busy.load(Ordering::SeqCst)
    {
        return;
    }
    if let Err(e) = teacher(service, cfg) {
        let _ = service
            .store
            .lock()
            .unwrap()
            .state("teacher_status", &json!(e));
        *retry = Instant::now() + Duration::from_secs(3600);
    }
    let counts = service
        .store
        .lock()
        .unwrap()
        .history("", "")
        .unwrap_or_default()
        .iter()
        .filter(|r| label(r, cfg["teacher_min_confidence"].as_f64().unwrap_or(0.7)).is_some())
        .count();
    let previous = service
        .store
        .lock()
        .unwrap()
        .get_state("last_training")
        .unwrap_or(json!({"count":0,"ts":now()}));
    if due(
        counts,
        previous["count"].as_u64().unwrap_or(0) as usize,
        previous["ts"].as_f64().unwrap_or(now()),
        now(),
        &cfg,
    ) {
        let _ = start_training_on_power(service.clone(), on_mains);
    }
}
pub fn due(total: usize, previous: usize, ts: f64, now: f64, cfg: &Value) -> bool {
    total >= 50
        && total > previous
        && (total - previous >= cfg["retrain_after_labels"].as_u64().unwrap_or(300) as usize
            || now - ts >= cfg["retrain_after_days"].as_f64().unwrap_or(7.) * 86400.)
}
#[cfg(test)]
#[path = "../../tests/unit/learning_guards.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/teacher_selection.rs"]
mod selection_tests;

#[cfg(all(test, target_os = "linux"))]
use power::linux_mains;
#[cfg(test)]
use teacher::{choose_teacher, gpu_memory};
#[cfg(test)]
use training::prune;
