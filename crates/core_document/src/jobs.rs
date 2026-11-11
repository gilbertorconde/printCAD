//! Work run away from the window, in two shapes.
//!
//! [`Keyed`] holds results that read the document and are kept by the key
//! of what they were worked out from. Asked for a key it lacks, it starts
//! a thread, and a newer key supersedes a run still going: the older
//! answer is dropped when it lands.
//!
//! A [`Ticket`] is one [`Job`] with the inputs it was asked under. Its
//! answer is taken only while those inputs still hold, which is what a
//! result that writes the document needs. A bench keeps tickets in its
//! tab state, so an answer reaches only the tab that asked.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};

/// Results by id, each kept with the key it was worked out from.
pub struct Keyed<Id, V> {
    state: Arc<Mutex<KeyedState<Id, V>>>,
}

struct KeyedState<Id, V> {
    made: HashMap<Id, (u64, V)>,
    failed: HashMap<Id, (u64, String)>,
    /// The key each id's current run works from.
    running: HashMap<Id, u64>,
}

impl<Id, V> Default for Keyed<Id, V> {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(KeyedState {
                made: HashMap::new(),
                failed: HashMap::new(),
                running: HashMap::new(),
            })),
        }
    }
}

impl<Id, V> Clone for Keyed<Id, V> {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
        }
    }
}

impl<Id: Copy + Eq + Hash + Send + 'static, V: Clone + Send + 'static> Keyed<Id, V> {
    /// `id`'s result when it was worked out from what `key` names.
    pub fn get(&self, id: Id, key: u64) -> Option<V> {
        let state = self.state.lock().ok()?;
        state
            .made
            .get(&id)
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
    }

    /// Why `id` could not be worked out from what `key` names, if it could
    /// not.
    pub fn failure(&self, id: Id, key: u64) -> Option<String> {
        let state = self.state.lock().ok()?;
        state
            .failed
            .get(&id)
            .filter(|(k, _)| *k == key)
            .map(|(_, why)| why.clone())
    }

    /// `id`'s result for `key` once it is in. When it is not, and has not
    /// failed for `key` or started for it, `job` starts on a thread of its
    /// own, superseding a run for an older key.
    pub fn ensure(
        &self,
        id: Id,
        key: u64,
        job: impl FnOnce() -> Result<V, String> + Send + 'static,
    ) -> Option<V> {
        {
            let mut state = self.state.lock().ok()?;
            if let Some((_, v)) = state.made.get(&id).filter(|(k, _)| *k == key) {
                return Some(v.clone());
            }
            if state.running.get(&id) == Some(&key)
                || state.failed.get(&id).is_some_and(|(k, _)| *k == key)
            {
                return None;
            }
            state.running.insert(id, key);
        }
        let shared = Arc::clone(&self.state);
        let spawned = std::thread::Builder::new()
            .name("printcad-keyed".into())
            .spawn(move || {
                let answer = job();
                let Ok(mut state) = shared.lock() else {
                    return;
                };
                if state.running.get(&id) != Some(&key) {
                    return;
                }
                state.running.remove(&id);
                match answer {
                    Ok(v) => {
                        state.failed.remove(&id);
                        state.made.insert(id, (key, v));
                    }
                    Err(why) => {
                        state.failed.insert(id, (key, why));
                    }
                }
            });
        if spawned.is_err()
            && let Ok(mut state) = self.state.lock()
        {
            state.running.remove(&id);
            state
                .failed
                .insert(id, (key, "the work could not start".into()));
        }
        None
    }

    /// Keep `value` as `id`'s, worked out from what `key` names, superseding
    /// a run for any key.
    pub fn store(&self, id: Id, key: u64, value: V) {
        if let Ok(mut state) = self.state.lock() {
            state.running.remove(&id);
            state.failed.remove(&id);
            state.made.insert(id, (key, value));
        }
    }

    /// Whether a run whose answer is still wanted is going.
    pub fn busy(&self) -> bool {
        self.state.lock().is_ok_and(|s| !s.running.is_empty())
    }
}

/// Work on a thread of its own: its answer to come, how far it has got of
/// its total, and the flag that stops it. Dropping it stops the work at
/// its next step.
pub struct Job<T> {
    answer: Receiver<Result<T, String>>,
    done: Arc<AtomicUsize>,
    total: usize,
    stop: Arc<AtomicBool>,
}

/// The working end of a [`Job`]: where its answer goes, the count of steps
/// done and the stop flag.
pub struct Reply<T> {
    send: Sender<Result<T, String>>,
    done: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

impl<T> Reply<T> {
    /// Give the answer.
    pub fn send(self, answer: Result<T, String>) {
        let _ = self.send.send(answer);
    }

    /// The count of steps done, for the work to raise.
    pub fn done(&self) -> &AtomicUsize {
        &self.done
    }

    /// The flag the work looks at between steps.
    pub fn stop(&self) -> &AtomicBool {
        &self.stop
    }
}

impl<T> Job<T> {
    /// A job of `total` steps and its working end, for the caller to run
    /// where it likes.
    pub fn new(total: usize) -> (Self, Reply<T>) {
        let done = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (send, answer) = std::sync::mpsc::channel();
        let reply = Reply {
            send,
            done: done.clone(),
            stop: stop.clone(),
        };
        let job = Self {
            answer,
            done,
            total,
            stop,
        };
        (job, reply)
    }

    /// The answer once it has come; work that ended without one answers
    /// `ended`.
    pub fn answer(&self, ended: &str) -> Option<Result<T, String>> {
        match self.answer.try_recv() {
            Ok(answer) => Some(answer),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(ended.to_string())),
        }
    }

    /// Steps done, of how many.
    pub fn progress(&self) -> (usize, usize) {
        (self.done.load(Ordering::Relaxed), self.total)
    }

    /// Stop at the next step; what it found so far still comes.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl<T: Send + 'static> Job<T> {
    /// Run `work` on a thread called `name`; it counts what it has done of
    /// `total` and looks at the stop flag between steps.
    pub fn spawn(
        name: &str,
        total: usize,
        work: impl FnOnce(&AtomicUsize, &AtomicBool) -> Result<T, String> + Send + 'static,
    ) -> std::io::Result<Self> {
        let (job, reply) = Self::new(total);
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let answer = work(reply.done(), reply.stop());
                reply.send(answer);
            })?;
        Ok(job)
    }
}

impl<T> Drop for Job<T> {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A [`Job`] with the inputs it was asked under.
pub struct Ticket<K, T> {
    key: K,
    job: Job<T>,
}

/// What a ticket's answer comes to.
pub enum Landed<K, T> {
    /// The inputs still hold: the answer.
    Fresh(K, Result<T, String>),
    /// The inputs moved while it ran; the answer is dropped.
    Stale(K),
}

impl<K, T> Ticket<K, T> {
    pub fn new(key: K, job: Job<T>) -> Self {
        Self { key, job }
    }

    /// The inputs it was asked under.
    pub fn key(&self) -> &K {
        &self.key
    }

    pub fn job(&self) -> &Job<T> {
        &self.job
    }

    /// The ticket in `slot` once its answer is in, taken out of the slot
    /// and judged by `holds`, which says whether its inputs still stand;
    /// work that ended without an answer answers `ended`.
    pub fn land(
        slot: &mut Option<Self>,
        holds: impl FnOnce(&K) -> bool,
        ended: &str,
    ) -> Option<Landed<K, T>> {
        let answer = slot.as_ref()?.job.answer(ended)?;
        let ticket = slot.take()?;
        Some(if holds(&ticket.key) {
            Landed::Fresh(ticket.key, answer)
        } else {
            Landed::Stale(ticket.key)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    fn settle<V: Clone + Send + 'static>(keyed: &Keyed<u8, V>) {
        while keyed.busy() {
            std::thread::yield_now();
        }
    }

    /// A newer key starts at once; the superseded run's answer, landing
    /// after, is never kept, so the newer one is not measured again.
    #[test]
    fn a_newer_key_supersedes_a_running_older_one() {
        let keyed = Keyed::<u8, u32>::default();
        let (go, wait) = channel::<()>();
        assert_eq!(
            keyed.ensure(0, 1, move || {
                wait.recv().ok();
                Ok(1)
            }),
            None
        );
        keyed.ensure(0, 2, || Ok(2));
        settle(&keyed);
        assert_eq!(keyed.get(0, 2), Some(2));
        go.send(()).unwrap();
        // The old run has landed once its thread lets go of the state.
        while Arc::strong_count(&keyed.state) > 1 {
            std::thread::yield_now();
        }
        assert_eq!(keyed.get(0, 2), Some(2), "the newer answer stays");
        assert_eq!(keyed.get(0, 1), None);
        assert_eq!(keyed.ensure(0, 2, || Ok(9)), Some(2), "nothing runs again");
        assert!(!keyed.busy());

        // A failure is kept by its key and not run again for it.
        keyed.ensure(0, 3, || Err("no".into()));
        settle(&keyed);
        assert_eq!(keyed.failure(0, 3).as_deref(), Some("no"));
        assert_eq!(keyed.ensure(0, 3, || Ok(3)), None);
        assert!(!keyed.busy());
    }

    /// A ticket's answer lands only while its inputs hold; a dropped job
    /// is told to stop.
    #[test]
    fn a_ticket_lands_only_while_its_inputs_hold() {
        for holds in [true, false] {
            let (job, reply) = Job::<u32>::new(1);
            let mut slot = Some(Ticket::new(7u8, job));
            assert!(Ticket::land(&mut slot, |_| holds, "ended").is_none());
            reply.send(Ok(5));
            match Ticket::land(&mut slot, |k| *k == 7 && holds, "ended") {
                Some(Landed::Fresh(7, Ok(5))) => assert!(holds),
                Some(Landed::Stale(7)) => assert!(!holds),
                _ => panic!("the answer is in"),
            }
            assert!(slot.is_none());
        }
        let (job, reply) = Job::<u32>::new(1);
        drop(job);
        assert!(reply.stop().load(Ordering::Relaxed));
    }
}
