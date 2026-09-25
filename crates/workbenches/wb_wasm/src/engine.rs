//! One wasmtime engine for every package, the clock that stops a call
//! running over its budget, and compiling a package's component once.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use wasmtime::component::Component;
use wasmtime::{Config, Engine};

/// How often the engine's epoch moves while a call runs.
pub(crate) const TICK: Duration = Duration::from_millis(5);

/// How long a call may run, in ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Budget {
    /// Drawing and input: a frame must never wait on a bench.
    Frame,
    /// Commands, panel changes, rebuild plans, loading.
    Long,
}

impl Budget {
    pub(crate) fn ticks(self) -> u64 {
        match self {
            // 25 ms.
            Budget::Frame => 5,
            // 1 s.
            Budget::Long => 200,
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
/// does, so an idle app never wakes for it.
struct Ticker {
    running: AtomicUsize,
    thread: Mutex<Option<std::thread::Thread>>,
}

static TICKER: LazyLock<Ticker> = LazyLock::new(|| {
    let handle = std::thread::Builder::new()
        .name("printcad-wasm-epoch".into())
        .spawn(|| {
            loop {
                if TICKER.running.load(Ordering::Acquire) == 0 {
                    std::thread::park();
                    continue;
                }
                std::thread::sleep(TICK);
                ENGINE.increment_epoch();
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

/// The component in `wasm`, compiled once and kept in `cache` as `name`: a
/// later start reads the compiled form back when this engine made it from
/// the same bytes. A compiled form is native code, so `cache` is a folder
/// the app alone writes, never one a package archive unpacks into.
pub(crate) fn component(wasm: &Path, cache: &Path, name: &str) -> Result<Component, String> {
    let bytes = std::fs::read(wasm).map_err(|e| format!("cannot read {}: {e}", wasm.display()))?;
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut h);
        ENGINE.precompile_compatibility_hash().hash(&mut h);
        h.finish()
    };
    // `@` is in no package id, so one id's prefix is never another's.
    let prefix = format!("{name}@");
    let file = cache.join(format!("{prefix}{key:016x}.cwasm"));
    if file.is_file() {
        // SAFETY: the file is one this engine wrote from the same bytes
        // (its name carries both hashes) into the app's own cache folder;
        // deserializing checks the engine's settings again.
        if let Ok(component) = unsafe { Component::deserialize_file(&ENGINE, &file) } {
            return Ok(component);
        }
    }
    let component = Component::new(&ENGINE, &bytes).map_err(|e| format!("{e:#}"))?;
    if let Ok(compiled) = component.serialize()
        && std::fs::create_dir_all(cache).is_ok()
    {
        // The compiled forms of this package's earlier bytes go.
        if let Ok(entries) = std::fs::read_dir(cache) {
            for entry in entries.flatten() {
                let old = entry.file_name();
                let old = old.to_string_lossy();
                if old.starts_with(&prefix) && old.ends_with(".cwasm") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        let _ = std::fs::write(&file, compiled);
    }
    Ok(component)
}
