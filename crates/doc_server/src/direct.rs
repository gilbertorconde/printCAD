//! The no-daemon fallback: the same `DocumentServer` contract served by
//! plain worker threads doing file I/O in-process. Behaviorally identical
//! to the daemon path from the app's point of view, which is the proof the
//! trait actually seals the seam. Ops are counted, acknowledged in the
//! order sent and dropped: with no server process there is nowhere durable
//! to log them, and pretending otherwise would be worse than saying so.

use crate::file_io::write_atomically;

use std::sync::mpsc::{Receiver, Sender, channel};

use core_document::server::{ClientMessage, DocumentServer, ServerMessage, ServerStatus};

pub struct DirectFiles {
    tx: Sender<ServerMessage>,
    rx: Receiver<ServerMessage>,
    workers: Vec<std::thread::JoinHandle<()>>,
    /// The latest save's writer. Each save's thread waits for the one
    /// before it, so saves are written and answered in the order sent and
    /// the file ends up holding the last.
    last_save: Option<std::thread::JoinHandle<()>>,
    /// Replies `flush` took in, handed out by the next `poll`.
    held: Vec<ServerMessage>,
    opens_in_flight: u32,
    saves_in_flight: u32,
    ops_dropped: u64,
    /// The last sequence number acknowledged; this client is the only one.
    seq: u64,
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
            last_save: None,
            held: Vec::new(),
            opens_in_flight: 0,
            saves_in_flight: 0,
            ops_dropped: 0,
            seq: 0,
        }
    }
}

impl DocumentServer for DirectFiles {
    fn name(&self) -> &str {
        "direct files (no daemon)"
    }

    fn standalone(&self) -> bool {
        true
    }

    fn send(&mut self, msg: ClientMessage) {
        self.workers.retain(|handle| !handle.is_finished());
        match msg {
            // No peers without a daemon; presence has no audience.
            ClientMessage::Hello { .. }
            | ClientMessage::Rebase
            | ClientMessage::Presence(_)
            | ClientMessage::RestoreUnsaved
            | ClientMessage::DiscardUnsaved
            | ClientMessage::Leave => {}
            ClientMessage::Ops { ops, .. } => {
                self.ops_dropped += ops.len() as u64;
                self.seq += 1;
                let _ = self.tx.send(ServerMessage::Acked { seq: self.seq });
            }
            ClientMessage::SaveDocument {
                path,
                bytes,
                at_seq,
                ..
            } => {
                self.saves_in_flight = self.saves_in_flight.saturating_add(1);
                let tx = self.tx.clone();
                let before = self.last_save.take();
                let handle = std::thread::spawn(move || {
                    if let Some(before) = before {
                        let _ = before.join();
                    }
                    let reply = match write_atomically(&path, &bytes) {
                        Ok(()) => ServerMessage::SaveCompleted { path, at_seq },
                        Err(err) => ServerMessage::SaveFailed {
                            path,
                            error: err.to_string(),
                        },
                    };
                    let _ = tx.send(reply);
                });
                self.last_save = Some(handle);
            }
            ClientMessage::OpenDocument { path, token } => {
                self.opens_in_flight = self.opens_in_flight.saturating_add(1);
                let tx = self.tx.clone();
                let handle = std::thread::spawn(move || {
                    let reply = match std::fs::read(&path) {
                        Ok(bytes) => ServerMessage::Opened {
                            token,
                            path,
                            bytes,
                            seq: 0,
                            unsaved_lost: false,
                            recoverable: 0,
                        },
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
        let mut out = std::mem::take(&mut self.held);
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
            peers: 0,
            connected: true,
            last_error: None,
        }
    }

    fn flush(&mut self) {
        // The latest save waits for every save before it.
        for handle in self
            .last_save
            .take()
            .into_iter()
            .chain(self.workers.drain(..))
        {
            let _ = handle.join();
        }
        // The replies settle the in-flight counters now and wait for the
        // next `poll`, which hands them out.
        self.held = self.poll();
    }
}
