//! Work done on a thread of its own while a dictation goes on.
//!
//! Recognising a piece or cleaning up a block can take seconds. Done on the
//! thread that handles keys, it would hold up a key release or a cancel
//! until it was over. A job runs elsewhere instead: the caller looks in on
//! it now and then, waits for it only when it needs the answer, and drops
//! it when it no longer does.

use crate::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, TryRecvError};
use std::sync::Arc;

/// Work running on its own thread. Dropping it asks the work to stop.
pub struct Job<T> {
    result: Receiver<Result<T>>,
    stop: Arc<AtomicBool>,
}

const LOST: &str = "The work stopped unexpectedly";

impl<T: Send + 'static> Job<T> {
    /// Starts `work`, which is told through its flag when nobody wants its
    /// answer any more, and can stop early then.
    pub fn start(work: impl FnOnce(&AtomicBool) -> Result<T> + Send + 'static) -> Job<T> {
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, result) = sync_channel(1);
        let flag = stop.clone();
        std::thread::spawn(move || {
            let _ = sender.send(work(&flag));
        });
        Job { result, stop }
    }

    /// The answer, once the work is over.
    pub fn poll(&self) -> Option<Result<T>> {
        match self.result.try_recv() {
            Ok(answer) => Some(answer),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(LOST.into())),
        }
    }

    /// Waits for the answer.
    pub fn wait(self) -> Result<T> {
        self.result.recv().unwrap_or_else(|_| Err(LOST.into()))
    }
}

impl<T> std::fmt::Debug for Job<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Job")
    }
}

impl<T> Drop for Job<T> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    #[test]
    fn a_job_answers_when_done_and_is_told_to_stop_when_dropped() {
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        let inside = gate.clone();
        let job = Job::start(move |_| {
            let _open = inside.lock().unwrap();
            Ok(7)
        });
        assert!(job.poll().is_none());
        drop(held);
        assert_eq!(job.wait(), Ok(7));

        let stopped = Arc::new(AtomicBool::new(false));
        let seen = stopped.clone();
        let (go, wait) = std::sync::mpsc::channel::<()>();
        let job = Job::start(move |stop| {
            wait.recv().unwrap();
            seen.store(stop.load(Ordering::SeqCst), Ordering::SeqCst);
            Ok(())
        });
        assert_eq!(format!("{job:?}"), "Job");
        drop(job);
        go.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !stopped.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(stopped.load(Ordering::SeqCst));
    }

    #[test]
    fn work_that_panics_is_reported_as_an_error() {
        let job = Job::<()>::start(|_| panic!("boom"));
        let deadline = Instant::now() + Duration::from_secs(5);
        let answer = loop {
            if let Some(answer) = job.poll() {
                break answer;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(answer, Err(LOST.into()));
        assert_eq!(Job::<()>::start(|_| panic!("boom")).wait(), Err(LOST.into()));
    }
}
