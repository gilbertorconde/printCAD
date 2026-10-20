//! Helpers kept running for a package to talk to, a line at a time
//! (`host::helper_open`): a helper that holds a run in memory answers
//! questions about it as they come, rather than everything up front.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How many of a helper's last error lines a failure names.
const ERROR_TAIL: usize = 5;

/// A helper started and kept running; dropped, it is killed.
pub(crate) struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    errors: Arc<Mutex<Vec<String>>>,
}

impl Session {
    /// Start helper `name` from `dir`. With `progress`, a job's counts,
    /// its `progress <done> <total>` lines move that job's bar.
    pub(crate) fn open(
        dir: &Path,
        name: &str,
        progress: Option<(Arc<AtomicU64>, Arc<AtomicU64>)>,
    ) -> Result<Self, String> {
        let path = crate::jobs::helper_path(dir, name)?;
        let mut child = local_ipc::background(&mut Command::new(&path))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run helper `{name}`: {e}"))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped");
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let errors = Arc::new(Mutex::new(Vec::new()));
        let stderr = child.stderr.take().expect("piped");
        let tail = errors.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                match (crate::jobs::progress_line(&line), &progress) {
                    (Some((d, t)), Some((done, total))) => {
                        done.store(d, Ordering::Relaxed);
                        total.store(t, Ordering::Relaxed);
                    }
                    (Some(_), None) => {}
                    (None, _) => {
                        let mut tail = tail.lock().unwrap_or_else(|p| p.into_inner());
                        tail.push(line);
                        let excess = tail.len().saturating_sub(ERROR_TAIL);
                        tail.drain(..excess);
                    }
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            lines,
            errors,
        })
    }

    fn ended(&self) -> String {
        let tail = self.errors.lock().unwrap_or_else(|p| p.into_inner());
        if tail.is_empty() {
            "the helper has ended".into()
        } else {
            format!("the helper has ended: {}", tail.join(" / "))
        }
    }

    /// Write `line` to the helper's standard input.
    pub(crate) fn send(&mut self, line: &str) -> Result<(), String> {
        if line.contains(['\n', '\r']) {
            return Err("a line has no line break in it".into());
        }
        let stdin = self.stdin.as_mut().ok_or("the helper's input is closed")?;
        if writeln!(stdin, "{line}")
            .and_then(|()| stdin.flush())
            .is_err()
        {
            return Err(self.ended());
        }
        Ok(())
    }

    /// The helper's next line, waiting up to `wait` for it unless
    /// `cancelled` is set meanwhile.
    pub(crate) fn receive(
        &mut self,
        wait: Duration,
        cancelled: Option<&AtomicBool>,
    ) -> Result<Option<String>, String> {
        let until = Instant::now() + wait;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left.min(Duration::from_millis(50))) {
                Ok(line) => return Ok(Some(line)),
                Err(RecvTimeoutError::Disconnected) => return Err(self.ended()),
                Err(RecvTimeoutError::Timeout) => {}
            }
            if cancelled.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err("stopped".into());
            }
            if left.is_zero() {
                return Ok(None);
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A helper kept running answers each line it is sent, moves the bar
    /// with its progress lines, and is killed when its session goes.
    #[test]
    fn a_session_talks_to_its_helper_a_line_at_a_time() {
        let dir = std::env::temp_dir().join(format!("printcad-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("echo");
        std::fs::write(
            &script,
            "#!/bin/sh\nwhile read line; do echo 'progress 1 2' >&2; echo \"got $line\"; done\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (done, total) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
        let mut session = Session::open(&dir, "echo", Some((done.clone(), total.clone()))).unwrap();
        assert_eq!(
            session.receive(Duration::ZERO, None),
            Ok(None),
            "nothing yet"
        );
        session.send("move 12").unwrap();
        let answer = session.receive(Duration::from_secs(10), None).unwrap();
        assert_eq!(answer.as_deref(), Some("got move 12"));
        session.send("again").unwrap();
        assert_eq!(
            session
                .receive(Duration::from_secs(10), None)
                .unwrap()
                .as_deref(),
            Some("got again")
        );
        assert_eq!(
            (done.load(Ordering::Relaxed), total.load(Ordering::Relaxed)),
            (1, 2)
        );
        assert!(session.send("two\nlines").is_err());
        let stop = AtomicBool::new(true);
        assert_eq!(
            session.receive(Duration::from_secs(10), Some(&stop)),
            Err("stopped".into())
        );
        let pid = session.child.id();
        drop(session);
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists()
                || std::fs::read_to_string(format!("/proc/{pid}/stat"))
                    .is_ok_and(|s| s.contains(") Z ")),
            "the helper is gone"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
