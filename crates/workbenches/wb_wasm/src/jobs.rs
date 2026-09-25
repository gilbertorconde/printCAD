//! Jobs: a package's long work, run in an instance of its own on a thread
//! of its own, stoppable, with its progress where the panel can show it;
//! and the native helpers a job may run.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use wasmtime::UpdateDeadline;

use crate::engine::Running;
use crate::guest::Loaded;
use crate::host::JobLine;

struct Handle {
    cancelled: Arc<AtomicBool>,
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
}

#[derive(Default)]
struct Board {
    running: HashMap<u64, Handle>,
    finished: Vec<(u64, Result<String, String>)>,
}

/// A package's jobs: those running and those finished but not yet told to
/// the bench.
pub(crate) struct JobBoard {
    loaded: Arc<Loaded>,
    next: AtomicU64,
    board: Mutex<Board>,
}

impl JobBoard {
    pub(crate) fn new(loaded: Arc<Loaded>) -> Arc<Self> {
        Arc::new(Self {
            loaded,
            next: AtomicU64::new(1),
            board: Mutex::new(Board::default()),
        })
    }

    /// Start `entry` with `input`; its number, the result to follow.
    pub(crate) fn start(self: &Arc<Self>, entry: String, input: String) -> Result<u64, String> {
        let job = self.next.fetch_add(1, Ordering::Relaxed);
        let line = Handle {
            cancelled: Arc::new(AtomicBool::new(false)),
            done: Arc::new(AtomicU64::new(0)),
            total: Arc::new(AtomicU64::new(0)),
        };
        let own = JobLine {
            cancelled: line.cancelled.clone(),
            done: line.done.clone(),
            total: line.total.clone(),
        };
        self.board
            .lock()
            .map_err(|_| "jobs lost")?
            .running
            .insert(job, line);
        let board = self.clone();
        std::thread::Builder::new()
            .name(format!("printcad-job-{}-{job}", self.loaded.package.id))
            .spawn(move || {
                let result = board.run(&entry, &input, own);
                if let Ok(mut b) = board.board.lock() {
                    b.running.remove(&job);
                    b.finished.push((job, result));
                }
            })
            .map_err(|e| format!("cannot start a job: {e}"))?;
        Ok(job)
    }

    fn run(self: &Arc<Self>, entry: &str, input: &str, line: JobLine) -> Result<String, String> {
        let cancelled = line.cancelled.clone();
        let (mut store, bindings) = self.loaded.instantiate(self.clone())?;
        store.data_mut().job = Some(line);
        let stop = cancelled.clone();
        store.epoch_deadline_callback(move |_| {
            if stop.load(Ordering::Relaxed) {
                Err(wasmtime::Error::msg("stopped"))
            } else {
                Ok(UpdateDeadline::Continue(1))
            }
        });
        store.set_epoch_deadline(1);
        let _running = Running::start();
        match bindings
            .printcad_workbench_bench()
            .call_job_run(&mut store, entry, input)
        {
            Ok(result) => result,
            Err(_) if cancelled.load(Ordering::Relaxed) => Err("stopped".into()),
            Err(error) => Err(format!("{error:#}")),
        }
    }

    pub(crate) fn cancel(&self, job: u64) {
        if let Ok(board) = self.board.lock()
            && let Some(handle) = board.running.get(&job)
        {
            handle.cancelled.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) fn cancel_all(&self) {
        if let Ok(board) = self.board.lock() {
            for handle in board.running.values() {
                handle.cancelled.store(true, Ordering::Relaxed);
            }
        }
    }

    /// How far job `job` is, `(done, total)`, while it runs.
    pub(crate) fn progress(&self, job: u64) -> Option<(u64, u64)> {
        let board = self.board.lock().ok()?;
        let h = board.running.get(&job)?;
        Some((
            h.done.load(Ordering::Relaxed),
            h.total.load(Ordering::Relaxed),
        ))
    }

    /// The jobs finished since this was last asked.
    pub(crate) fn take_finished(&self) -> Vec<(u64, Result<String, String>)> {
        self.board
            .lock()
            .map(|mut b| std::mem::take(&mut b.finished))
            .unwrap_or_default()
    }

    /// A job runs.
    pub(crate) fn running(&self) -> bool {
        self.board.lock().is_ok_and(|b| !b.running.is_empty())
    }

    /// A job finished that the bench has not been told of.
    pub(crate) fn has_finished(&self) -> bool {
        self.board.lock().is_ok_and(|b| !b.finished.is_empty())
    }
}

/// The folder of a package's helpers for this system.
pub(crate) fn helpers_dir(package: &Path) -> std::path::PathBuf {
    package.join("helpers").join(format!(
        "{}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    ))
}

/// Run helper `name` from `dir` with `input` on its standard input; what it
/// wrote to its standard output when it ends well. Stopping the job kills
/// it.
pub(crate) fn run_helper(
    dir: &Path,
    name: &str,
    input: &[u8],
    cancelled: &AtomicBool,
) -> Result<Vec<u8>, String> {
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !name.starts_with('.');
    if !plain {
        return Err(format!("`{name}` is not a helper name"));
    }
    let mut path = dir.join(name);
    if cfg!(windows) && path.extension().is_none() {
        path.set_extension("exe");
    }
    if !path.is_file() {
        return Err(format!(
            "the package has no helper `{name}` for this system"
        ));
    }
    let mut child = Command::new(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run helper `{name}`: {e}"))?;
    let mut stdin = child.stdin.take().expect("piped");
    let input = input.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let mut stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        out
    });
    let mut stderr = child.stderr.take().expect("piped");
    let errors = std::thread::spawn(move || {
        let mut out = String::new();
        let _ = stderr.read_to_string(&mut out);
        out
    });
    let status = loop {
        if cancelled.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("stopped".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("helper `{name}`: {e}")),
        }
    };
    let _ = writer.join();
    let out = reader.join().unwrap_or_default();
    let err = errors.join().unwrap_or_default();
    if status.success() {
        Ok(out)
    } else {
        let tail: String = err.lines().rev().take(5).collect::<Vec<_>>().join(" / ");
        Err(format!("helper `{name}` failed ({status}): {tail}"))
    }
}
