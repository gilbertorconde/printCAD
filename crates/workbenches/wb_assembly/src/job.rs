//! Work run away from the window: its answer to come, how far it has got
//! of its total, and the flag that stops it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};

pub(crate) struct Job<T> {
    answer: Receiver<Result<T, String>>,
    done: Arc<AtomicUsize>,
    total: usize,
    stop: Arc<AtomicBool>,
}

impl<T: Send + 'static> Job<T> {
    /// Run `work` on a thread called `name`; it counts what it has done
    /// of `total` and looks at the stop flag between steps.
    pub(crate) fn spawn(
        name: &str,
        total: usize,
        work: impl FnOnce(&AtomicUsize, &AtomicBool) -> Result<T, String> + Send + 'static,
    ) -> std::io::Result<Self> {
        let done = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (send, answer) = std::sync::mpsc::channel();
        let (counted, stopped) = (done.clone(), stop.clone());
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let _ = send.send(work(&counted, &stopped));
            })?;
        Ok(Self {
            answer,
            done,
            total,
            stop,
        })
    }
}

impl<T> Job<T> {
    /// The answer once it has come; a thread that ended without one
    /// answers `ended`.
    pub(crate) fn answer(&self, ended: &str) -> Option<Result<T, String>> {
        match self.answer.try_recv() {
            Ok(answer) => Some(answer),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(ended.to_string())),
        }
    }

    /// Steps done, of how many.
    pub(crate) fn progress(&self) -> (usize, usize) {
        (self.done.load(Ordering::Relaxed), self.total)
    }

    /// Stop at the next step; what it found so far still comes.
    pub(crate) fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl<T> Drop for Job<T> {
    /// Work nobody waits for stops at its next step.
    fn drop(&mut self) {
        self.stop();
    }
}
