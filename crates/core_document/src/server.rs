//! The document server seam.
//!
//! printCAD is always-multiplayer in architecture: a *document server* owns
//! the document's file and its op log, and the app is a client. By default
//! the server is a local daemon (`printcad-serverd`, one per document, unix
//! socket — the X11 model); a future plugin replaces the implementation with
//! a remote transport. The trait is deliberately message-shaped: what
//! crosses [`DocumentServer::send`]/[`DocumentServer::poll`] **is** the wire
//! protocol, serde-serialized verbatim by the socket transport, so promoting
//! an implementation from in-process to daemon to remote changes transport,
//! never semantics.
//!
//! The server never deserializes a `Document`. Snapshots cross the boundary
//! as opaque `.prtcad` container bytes (see `Document::save_to_bytes` /
//! `load_from_bytes`) and edits as [`DocumentOp`] envelopes — so a daemon
//! keeps serving clients whose document schema it has never seen.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::op::DocumentOp;

/// Version of the client↔server protocol; the `Hello` handshake refuses a
/// mismatch loudly rather than misreading frames quietly.
pub const SERVER_PROTOCOL_VERSION: u32 = 1;

/// Client → server messages.
#[derive(Debug, Serialize, Deserialize)]
pub enum ClientMessage {
    /// First message on a connection; anything else first is an error.
    Hello { protocol: u32 },
    /// User edits drained from the document's outbox, in order.
    Ops(Vec<DocumentOp>),
    /// The client's document history jumped (undo/redo/new/open): ops
    /// recorded before this point no longer describe the client's state.
    /// The server truncates its log; a future sync server re-baselines.
    Rebase,
    /// Persist a client-serialized `.prtcad` container. `at_seq` is the
    /// client's mutation counter when the snapshot was taken; it rides back
    /// in [`ServerMessage::SaveCompleted`] so the client can decide whether
    /// the document is truly clean (edits may have landed mid-save).
    SaveDocument {
        path: PathBuf,
        bytes: Vec<u8>,
        at_seq: u64,
    },
    /// Read a document's bytes. `token` correlates the eventual response
    /// with the request that asked for it (an open may be abandoned by a
    /// newer one).
    OpenDocument { path: PathBuf, token: u64 },
}

/// Server → client messages.
#[derive(Debug, Serialize, Deserialize)]
pub enum ServerMessage {
    HelloOk {
        protocol: u32,
    },
    Opened {
        token: u64,
        path: PathBuf,
        bytes: Vec<u8>,
    },
    OpenFailed {
        token: u64,
        path: PathBuf,
        error: String,
    },
    SaveCompleted {
        path: PathBuf,
        at_seq: u64,
    },
    SaveFailed {
        path: PathBuf,
        error: String,
    },
}

/// What the client-side handle reports about in-flight work; feeds the
/// status bar and the frame scheduler's work-pending predicate.
#[derive(Debug, Clone, Default)]
pub struct ServerStatus {
    pub opens_in_flight: u32,
    pub saves_in_flight: u32,
    /// False when the transport is degraded (daemon died, socket gone).
    pub connected: bool,
    pub last_error: Option<String>,
}

impl ServerStatus {
    /// Whether the frame loop must keep spinning for this component.
    pub fn busy(&self) -> bool {
        self.opens_in_flight > 0 || self.saves_in_flight > 0
    }
}

/// The replaceable server connection. Implementations: a unix-socket client
/// to the local `printcad-serverd`, a direct-file fallback, and — later — a
/// remote transport behind a plugin.
pub trait DocumentServer: Send {
    /// Human-readable implementation name for logs and the status bar.
    fn name(&self) -> &str;

    /// Queue a message. Never blocks; delivery failures surface through
    /// [`Self::status`] and error messages in [`Self::poll`].
    fn send(&mut self, msg: ClientMessage);

    /// Drain responses that have arrived since the last poll. Called once
    /// per frame; must never block.
    fn poll(&mut self) -> Vec<ServerMessage>;

    fn status(&self) -> ServerStatus;

    /// Block until every queued write has been durably handled. Every exit
    /// path must call this — the process exiting mid-save truncates files
    /// (the same invariant the old in-app save threads had).
    fn flush(&mut self);
}
