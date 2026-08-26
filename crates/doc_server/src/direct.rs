//! The no-daemon fallback: the same `DocumentServer` contract served by
//! plain worker threads doing file I/O in-process. Behaviorally identical
//! to the daemon path from the app's point of view — which is the proof the
//! trait actually seals the seam. Ops are counted and dropped: with no
//! server process there is nowhere durable to log them, and pretending
//! otherwise would be worse than saying so.

use std::sync::mpsc::{channel, Receiver, Sender};

use core_document::server::{ClientMessage, DocumentServer, ServerMessage, ServerStatus};

pub struct DirectFiles {
    tx: Sender<ServerMessage>,
    rx: Receiver<ServerMessage>,
    workers: Vec<std::thread::JoinHandle<()>>,
    opens_in_flight: u32,
    saves_in_flight: u32,
    ops_dropped: u64,
}

impl Default for DirectFiles {
    fn default() -> Self {
        Self::new()
    }
}

impl DirectFiles {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx,
            workers: Vec::new(),
            opens_in_flight: 0,
            saves_in_flight: 0,
            ops_dropped: 0,
        }
    }
}

impl DocumentServer for DirectFiles {
    fn name(&self) -> &str {
        "direct files (no daemon)"
    }

    fn send(&mut self, msg: ClientMessage) {
        self.workers.retain(|handle| !handle.is_finished());
        match msg {
            ClientMessage::Hello { .. } | ClientMessage::Rebase => {}
            ClientMessage::Ops(ops) => {
                self.ops_dropped += ops.len() as u64;
            }
            ClientMessage::SaveDocument {
                path,
                bytes,
                at_seq,
            } => {
                self.saves_in_flight = self.saves_in_flight.saturating_add(1);
                let tx = self.tx.clone();
                let handle = std::thread::spawn(move || {
                    let reply = match write_atomically(&path, &bytes) {
                        Ok(()) => ServerMessage::SaveCompleted { path, at_seq },
                        Err(err) => ServerMessage::SaveFailed {
                            path,
                            error: err.to_string(),
                        },
                    };
                    let _ = tx.send(reply);
                });
                self.workers.push(handle);
            }
            ClientMessage::OpenDocument { path, token } => {
                self.opens_in_flight = self.opens_in_flight.saturating_add(1);
                let tx = self.tx.clone();
                let handle = std::thread::spawn(move || {
                    let reply = match std::fs::read(&path) {
                        Ok(bytes) => ServerMessage::Opened { token, path, bytes },
                        Err(err) => ServerMessage::OpenFailed {
                            token,
                            path,
                            error: err.to_string(),
                        },
                    };
                    let _ = tx.send(reply);
                });
                self.workers.push(handle);
            }
        }
    }

    fn poll(&mut self) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        while let Ok(message) = self.rx.try_recv() {
            match &message {
                ServerMessage::Opened { .. } | ServerMessage::OpenFailed { .. } => {
                    self.opens_in_flight = self.opens_in_flight.saturating_sub(1);
                }
                ServerMessage::SaveCompleted { .. } | ServerMessage::SaveFailed { .. } => {
                    self.saves_in_flight = self.saves_in_flight.saturating_sub(1);
                }
                _ => {}
            }
            out.push(message);
        }
        out
    }

    fn status(&self) -> ServerStatus {
        ServerStatus {
            opens_in_flight: self.opens_in_flight,
            saves_in_flight: self.saves_in_flight,
            connected: true,
            last_error: None,
        }
    }

    fn flush(&mut self) {
        for handle in self.workers.drain(..) {
            let _ = handle.join();
        }
        // Drain replies so in-flight counters settle for any later status().
        let _ = self.poll();
    }
}

fn write_atomically(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("prtcad.writing");
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}
