//! One wasmtime engine for every package and the clock that stops a call
//! running over its budget.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use wasmtime::{Config, Engine};

use crate::exports::Budget;

/// How often the engine's epoch moves while a call runs: how often a
/// running call is asked whether it is within its budget.
pub(crate) const TICK: Duration = Duration::from_millis(5);

impl Budget {
    /// How long a call may run.
    pub(crate) fn limit(self) -> Duration {
        match self {
            Budget::Frame => Duration::from_millis(25),
            Budget::Long => Duration::from_secs(1),
        }
    }
}

pub(crate) static ENGINE: LazyLock<Engine> = LazyLock::new(|| {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.epoch_interruption(true);
    Engine::new(&config).expect("the engine's configuration is valid")
});

/// The thread that moves the epoch, running only while a call or a job
/// does, so an idle app never wakes for it. The epoch counts ticks of real
/// time: a sleep the system stretches (a loaded machine, a timer macOS
/// coalesces) moves it by every tick that passed at once. A tick only
/// makes a call look at its own clock (`guest::Clock`), so such a burst
/// never counts time that passed before the call started.
struct Ticker {
    running: AtomicUsize,
    thread: Mutex<Option<std::thread::Thread>>,
}

static TICKER: LazyLock<Ticker> = LazyLock::new(|| {
    let handle = std::thread::Builder::new()
        .name("printcad-wasm-epoch".into())
        .spawn(|| {
            let mut counted = Instant::now();
            loop {
                if TICKER.running.load(Ordering::Acquire) == 0 {
                    std::thread::park();
                    counted = Instant::now();
                    continue;
                }
                std::thread::sleep(TICK);
                let passed = counted.elapsed().as_nanos() / TICK.as_nanos();
                for _ in 0..passed {
                    ENGINE.increment_epoch();
                }
                counted += TICK * passed as u32;
            }
        })
        .expect("spawn the epoch thread");
    Ticker {
        running: AtomicUsize::new(0),
        thread: Mutex::new(Some(handle.thread().clone())),
    }
});

/// Keeps the epoch moving while it lives.
pub(crate) struct Running;

impl Running {
    pub(crate) fn start() -> Self {
        let ticker = &*TICKER;
        if ticker.running.fetch_add(1, Ordering::AcqRel) == 0
            && let Ok(thread) = ticker.thread.lock()
            && let Some(thread) = thread.as_ref()
        {
            thread.unpark();
        }
        Running
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        TICKER.running.fetch_sub(1, Ordering::AcqRel);
    }
}
