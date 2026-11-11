//! Jobs: a package's long work, run in an instance of its own on a thread
//! of its own, stoppable, with its progress where the panel can show it;
//! and the native helpers a job may run.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use uuid::Uuid;

use wasmtime::UpdateDeadline;

use crate::engine::Running;
use crate::guest::Loaded;
use crate::host::JobLine;

/// One tab's session of the bench: the one on screen holds it, and a tab
/// kept off screen holds its own in the state the bench suspended into.
/// Once nothing holds it the tab has closed (or the bench was unloaded).
#[derive(Debug, Default)]
pub(crate) struct Session;

/// Where a job was started: the tab's session and the document then in it.
#[derive(Clone)]
struct Owner {
    session: Weak<Session>,
    document: Option<Uuid>,
}

impl Owner {
    /// What becomes of a job of this owner with `session` on screen
    /// holding `document`.
    fn fate(&self, session: &Arc<Session>, document: Uuid) -> Fate {
        match self.session.upgrade() {
            None => Fate::Drop,
            Some(own) if !Arc::ptr_eq(&own, session) => Fate::Hold,
            // The tab holds another document than the one it started for.
            Some(_) if self.document.is_some_and(|d| d != document) => Fate::Drop,
            Some(_) => Fate::Deliver,
        }
    }
}

#[derive(PartialEq)]
enum Fate {
    Deliver,
    Hold,
    Drop,
}

struct Handle {
    cancelled: Arc<AtomicBool>,
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    owner: Owner,
}

/// A finished job not yet told to the bench.
struct Finished {
    job: u64,
    owner: Owner,
    result: Result<String, String>,
}

#[derive(Default)]
struct Board {
    running: HashMap<u64, Handle>,
    finished: Vec<Finished>,
    /// The session on screen, which a job started now belongs to.
    session: Weak<Session>,
}

/// A package's jobs: those running and those finished but not yet told to
/// the bench. Each belongs to the tab it was started in and is told only
/// there; one whose tab closed, or whose tab holds another document now,
/// is stopped and never told.
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

    /// The session now on screen: jobs started from here on are its.
    pub(crate) fn set_session(&self, session: &Arc<Session>) {
        if let Ok(mut board) = self.board.lock() {
            board.session = Arc::downgrade(session);
        }
    }

    /// Start `entry` with `input` for `document`, in the session on
    /// screen; its number, the result to follow.
    pub(crate) fn start(
        self: &Arc<Self>,
        entry: String,
        input: String,
        document: Option<Uuid>,
    ) -> Result<u64, String> {
        let job = self.next.fetch_add(1, Ordering::Relaxed);
        let mut board = self.board.lock().map_err(|_| "jobs lost")?;
        let owner = Owner {
            session: board.session.clone(),
            document,
        };
        let line = Handle {
            cancelled: Arc::new(AtomicBool::new(false)),
            done: Arc::new(AtomicU64::new(0)),
            total: Arc::new(AtomicU64::new(0)),
            owner: owner.clone(),
        };
        let own = JobLine {
            cancelled: line.cancelled.clone(),
            done: line.done.clone(),
            total: line.total.clone(),
        };
        board.running.insert(job, line);
        drop(board);
        let board = self.clone();
        std::thread::Builder::new()
            .name(format!("printcad-job-{}-{job}", self.loaded.package.id))
            .spawn(move || {
                let result = board.run(&entry, &input, own);
                if let Ok(mut b) = board.board.lock() {
                    b.running.remove(&job);
                    b.finished.push(Finished { job, owner, result });
                }
            })
            .map_err(|e| format!("cannot start a job: {e}"))?;
        Ok(job)
    }

    fn run(self: &Arc<Self>, entry: &str, input: &str, line: JobLine) -> Result<String, String> {
        let cancelled = line.cancelled.clone();
        let (mut store, bindings) = self.loaded.instantiate(self.clone())?;
        store.data_mut().reach.job = Some(line);
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

    /// The jobs finished for `session`, with `document` in it, since this
    /// was last asked. Those of other tabs kept off screen wait for their
    /// tab; those of a tab gone, or of a document gone from it, are let go,
    /// and those still running for one are stopped.
    pub(crate) fn take_finished(
        &self,
        session: &Arc<Session>,
        document: Uuid,
    ) -> Vec<(u64, Result<String, String>)> {
        let Ok(mut board) = self.board.lock() else {
            return Vec::new();
        };
        for handle in board.running.values() {
            if handle.owner.fate(session, document) == Fate::Drop {
                handle.cancelled.store(true, Ordering::Relaxed);
            }
        }
        let mut told = Vec::new();
        board
            .finished
            .retain_mut(|f| match f.owner.fate(session, document) {
                Fate::Hold => true,
                Fate::Drop => false,
                Fate::Deliver => {
                    let result = std::mem::replace(&mut f.result, Ok(String::new()));
                    told.push((f.job, result));
                    false
                }
            });
        told
    }

    pub(crate) fn running(&self) -> bool {
        self.board.lock().is_ok_and(|b| !b.running.is_empty())
    }

    /// A job finished that the bench on screen in `session` has not been
    /// told of (or let go of): one waiting for another tab does not count.
    pub(crate) fn has_finished(&self, session: &Arc<Session>) -> bool {
        self.board.lock().is_ok_and(|b| {
            b.finished.iter().any(|f| {
                f.owner
                    .session
                    .upgrade()
                    .is_none_or(|own| Arc::ptr_eq(&own, session))
            })
        })
    }
}

/// Start the helper at `path`, its three streams piped.
///
/// A helper written moments ago can still be held open for writing by a
/// process another thread is starting: a fork copies every open file, and
/// the copy stays open until that process's own program replaces it. Until
/// then the system refuses to run the helper ("text file busy"), so the
/// start is tried again for a little while.
pub(crate) fn spawn_helper(path: &Path, name: &str) -> Result<std::process::Child, String> {
    let mut tries = 0;
    loop {
        let started = local_ipc::background(&mut Command::new(path))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        match started {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < BUSY_TRIES => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            other => return other.map_err(|e| format!("cannot run helper `{name}`: {e}")),
        }
    }
}

/// How many times a busy helper's start is tried again, 10 ms apart.
const BUSY_TRIES: u32 = 200;

/// Run helper `name` from `dir` with `input` on its standard input; what it
/// wrote to its standard output when it ends well. Stopping the job kills
/// it.
pub(crate) fn run_helper(
    dir: &Path,
    name: &str,
    input: &[u8],
    job: &JobLine,
) -> Result<Vec<u8>, String> {
    let cancelled = &job.cancelled;
    let path = helper_path(dir, name)?;
    let mut child = spawn_helper(&path, name)?;
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
    // A `progress <done> <total>` line moves the job's bar as the helper
    // writes it; every other line is kept for a failure's message.
    let stderr = child.stderr.take().expect("piped");
    let (done, total) = (job.done.clone(), job.total.clone());
    let errors = std::thread::spawn(move || {
        let mut out = String::new();
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            match progress_line(&line) {
                Some((d, t)) => {
                    done.store(d, Ordering::Relaxed);
                    total.store(t, Ordering::Relaxed);
                }
                None => {
                    out.push_str(&line);
                    out.push('\n');
                }
            }
        }
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

/// Helper `name`'s program in `dir`: a plain name, the system's own
/// executable beside it on Windows.
pub(crate) fn helper_path(dir: &Path, name: &str) -> Result<std::path::PathBuf, String> {
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
    Ok(path)
}

/// The two counts of a helper's `progress <done> <total>` line.
pub(crate) fn progress_line(line: &str) -> Option<(u64, u64)> {
    let mut words = line.split_whitespace();
    if words.next()? != "progress" {
        return None;
    }
    let done = words.next()?.parse().ok()?;
    let total = words.next()?.parse().ok()?;
    words.next().is_none().then_some((done, total))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_helper_moves_its_jobs_bar_while_it_runs() {
        let dir = std::env::temp_dir().join(format!("printcad-helper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("stages");
        std::fs::write(
            &script,
            "#!/bin/sh\necho 'progress 2 5' >&2\nsleep 1\necho 'progress 5 5' >&2\necho 'progress soon' >&2\necho 'the last stage failed' >&2\nexit 1\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let job = JobLine {
            cancelled: Arc::new(AtomicBool::new(false)),
            done: Arc::new(AtomicU64::new(0)),
            total: Arc::new(AtomicU64::new(0)),
        };
        let (done, total) = (job.done.clone(), job.total.clone());
        let running = std::thread::spawn({
            let dir = dir.clone();
            move || run_helper(&dir, "stages", b"", &job)
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while done.load(Ordering::Relaxed) != 2 {
            assert!(
                std::time::Instant::now() < deadline,
                "the first stage shows"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!running.is_finished(), "shown while the helper still runs");
        assert_eq!(total.load(Ordering::Relaxed), 5);
        let failed = running.join().unwrap().unwrap_err();
        assert_eq!(
            (done.load(Ordering::Relaxed), total.load(Ordering::Relaxed)),
            (5, 5)
        );
        assert!(failed.contains("the last stage failed"), "{failed}");
        assert!(failed.contains("progress soon"), "{failed}");
        assert!(!failed.contains("progress 5 5"), "{failed}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
