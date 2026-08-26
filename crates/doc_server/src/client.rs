//! The socket client: spawn-or-connect to `printcad-serverd`, then speak
//! frames. A reader thread turns the socket into an mpsc the UI drains once
//! per frame — the same shape as the kernel worker, so the app's frame loop
//! treats both alike.

use std::io::Write as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use core_document::server::{
    ClientMessage, DocumentServer, ServerMessage, ServerStatus, SERVER_PROTOCOL_VERSION,
};

use crate::framing::{read_frame, write_frame};

/// How long spawn-or-connect waits for the daemon's socket to appear.
const SPAWN_WAIT: Duration = Duration::from_secs(5);

pub struct DaemonClient {
    /// This client's identity among the document's editors; relayed ops
    /// name their author with it.
    actor: uuid::Uuid,
    stream: UnixStream,
    /// Frames the reader thread has decoded, drained by `poll`.
    rx: Receiver<ServerMessage>,
    /// Keeps the child handle so a daemon we spawned is reaped on drop.
    child: Option<std::process::Child>,
    opens_in_flight: u32,
    saves_in_flight: u32,
    peers: u32,
    connected: bool,
    last_error: Option<String>,
}

impl DaemonClient {
    /// This client's actor id (stable for the connection's lifetime).
    pub fn actor(&self) -> uuid::Uuid {
        self.actor
    }

    /// Connect to the daemon for `socket`, spawning one if none listens.
    /// The handshake completes before this returns, so a protocol mismatch
    /// is a construction error, not a later surprise.
    pub fn spawn_or_connect(socket: &Path) -> std::io::Result<Self> {
        let stream = match UnixStream::connect(socket) {
            Ok(stream) => stream,
            Err(_) => {
                let child = spawn_daemon(socket)?;
                let stream = wait_for_socket(socket, SPAWN_WAIT)?;
                return Self::finish_handshake(stream, Some(child));
            }
        };
        Self::finish_handshake(stream, None)
    }

    fn finish_handshake(
        mut stream: UnixStream,
        child: Option<std::process::Child>,
    ) -> std::io::Result<Self> {
        let actor = uuid::Uuid::new_v4();
        write_frame(
            &mut stream,
            &ClientMessage::Hello {
                protocol: SERVER_PROTOCOL_VERSION,
                actor,
            },
        )?;
        let reply: ServerMessage = read_frame(&mut stream)?;
        let peers = match reply {
            ServerMessage::HelloOk { protocol, peers } if protocol == SERVER_PROTOCOL_VERSION => {
                peers
            }
            other => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("handshake failed: {other:?}"),
                ));
            }
        };

        let (tx, rx): (Sender<ServerMessage>, Receiver<ServerMessage>) = channel();
        let reader = stream.try_clone()?;
        std::thread::Builder::new()
            .name("printcad-server-reader".into())
            .spawn(move || {
                let mut reader = reader;
                loop {
                    match read_frame::<_, ServerMessage>(&mut reader) {
                        Ok(message) => {
                            if tx.send(message).is_err() {
                                return;
                            }
                        }
                        Err(_) => return, // disconnect; poll() notices via rx closure
                    }
                }
            })?;

        Ok(Self {
            actor,
            stream,
            rx,
            child,
            opens_in_flight: 0,
            saves_in_flight: 0,
            peers,
            connected: true,
            last_error: None,
        })
    }
}

impl DocumentServer for DaemonClient {
    fn name(&self) -> &str {
        "local daemon"
    }

    fn send(&mut self, msg: ClientMessage) {
        match &msg {
            ClientMessage::OpenDocument { .. } => {
                self.opens_in_flight = self.opens_in_flight.saturating_add(1);
            }
            ClientMessage::SaveDocument { .. } => {
                self.saves_in_flight = self.saves_in_flight.saturating_add(1);
            }
            _ => {}
        }
        if let Err(err) = write_frame(&mut self.stream, &msg) {
            self.connected = false;
            self.last_error = Some(format!("send failed: {err}"));
            // The in-flight request will never be answered; undo the count
            // so the frame loop doesn't spin on a dead connection.
            match &msg {
                ClientMessage::OpenDocument { .. } => {
                    self.opens_in_flight = self.opens_in_flight.saturating_sub(1);
                }
                ClientMessage::SaveDocument { .. } => {
                    self.saves_in_flight = self.saves_in_flight.saturating_sub(1);
                }
                _ => {}
            }
        }
    }

    fn poll(&mut self) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(message) => {
                    match &message {
                        ServerMessage::Opened { .. } | ServerMessage::OpenFailed { .. } => {
                            self.opens_in_flight = self.opens_in_flight.saturating_sub(1);
                        }
                        ServerMessage::SaveCompleted { .. } | ServerMessage::SaveFailed { .. } => {
                            self.saves_in_flight = self.saves_in_flight.saturating_sub(1);
                        }
                        ServerMessage::Peers { peers } => {
                            self.peers = *peers;
                        }
                        _ => {}
                    }
                    out.push(message);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.connected {
                        self.connected = false;
                        self.last_error
                            .get_or_insert_with(|| "server connection lost".to_string());
                    }
                    break;
                }
            }
        }
        out
    }

    fn status(&self) -> ServerStatus {
        ServerStatus {
            opens_in_flight: self.opens_in_flight,
            saves_in_flight: self.saves_in_flight,
            peers: if self.connected { self.peers } else { 0 },
            connected: self.connected,
            last_error: self.last_error.clone(),
        }
    }

    fn flush(&mut self) {
        // Saves are answered in order; wait (bounded) until every queued
        // write has its reply, so exiting cannot truncate a document.
        let deadline = Instant::now() + Duration::from_secs(60);
        while self.saves_in_flight > 0 && self.connected && Instant::now() < deadline {
            match self.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(message) => match &message {
                    ServerMessage::SaveCompleted { .. } | ServerMessage::SaveFailed { .. } => {
                        self.saves_in_flight = self.saves_in_flight.saturating_sub(1);
                    }
                    ServerMessage::Opened { .. } | ServerMessage::OpenFailed { .. } => {
                        self.opens_in_flight = self.opens_in_flight.saturating_sub(1);
                    }
                    _ => {}
                },
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    self.connected = false;
                    break;
                }
            }
        }
        let _ = self.stream.flush();
    }
}

impl Drop for DaemonClient {
    fn drop(&mut self) {
        // Closing the stream tells the daemon we left. Reap it briefly if
        // we spawned it — but only briefly: other clients may be keeping it
        // alive, and blocking here would deadlock the spawner on its own
        // daemon. An unreaped child that exits later is a zombie until our
        // process ends; with one daemon per document that stays a handful.
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
        if let Some(child) = &mut self.child {
            for _ in 0..20 {
                match child.try_wait() {
                    Ok(Some(_)) => return,
                    Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                    Err(_) => return,
                }
            }
        }
    }
}

fn spawn_daemon(socket: &Path) -> std::io::Result<std::process::Child> {
    let binary = daemon_binary_path();
    std::process::Command::new(&binary)
        .arg("--socket")
        .arg(socket)
        .spawn()
        .map_err(|e| {
            std::io::Error::new(
                e.kind(),
                format!("spawning {} failed: {e}", binary.display()),
            )
        })
}

/// The daemon ships beside the app binary; `PRINTCAD_SERVERD` overrides for
/// development (cargo puts both in target/, so the default also works there).
fn daemon_binary_path() -> PathBuf {
    if let Some(explicit) = std::env::var_os("PRINTCAD_SERVERD") {
        return PathBuf::from(explicit);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("printcad-serverd")))
        .unwrap_or_else(|| PathBuf::from("printcad-serverd"))
}

fn wait_for_socket(socket: &Path, timeout: Duration) -> std::io::Result<UnixStream> {
    let deadline = Instant::now() + timeout;
    loop {
        match UnixStream::connect(socket) {
            Ok(stream) => return Ok(stream),
            Err(err) if Instant::now() >= deadline => return Err(err),
            Err(_) => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}
