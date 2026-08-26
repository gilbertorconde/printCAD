//! The document server seam.
//!
//! printCAD is always-multiplayer in architecture: a *document server* owns
//! the document's file and its op log, and the app is a client. By default
//! the server is a local daemon (`printcad-serverd`, one per document, unix
//! socket); a future plugin replaces the implementation with a remote
//! transport. The trait is deliberately message-shaped: what
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
///
/// v2: `Hello` carries the client's actor id; the server relays each
/// client's ops to every *other* client as [`ServerMessage::Ops`].
pub const SERVER_PROTOCOL_VERSION: u32 = 2;

/// Client → server messages.
#[derive(Debug, Serialize, Deserialize)]
pub enum ClientMessage {
    /// First message on a connection; anything else first is an error.
    /// `actor` identifies this client among the document's editors — it is
    /// how relayed ops name their author.
    Hello { protocol: u32, actor: uuid::Uuid },
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
    /// Ephemeral presence — who this editor is and what they have selected.
    /// Relayed to peers, never logged: presence is now-state, not history.
    Presence(PresenceState),
}

/// What a peer sees of another editor. Deliberately selection-level, not
/// cursor-level: a selected body is stable, meaningful across viewports,
/// and cheap; live cursors can layer on later without protocol changes
/// (this struct just grows fields with serde defaults).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresenceState {
    /// Human-facing name (login name by default).
    pub display_name: String,
    /// The body this editor currently has selected, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_body: Option<uuid::Uuid>,
    /// Where this editor's pointer hovers in world space (mm), quantized by
    /// the sender so idle jitter does not spam the wire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_world: Option<[f32; 3]>,
}

/// Server → client messages.
#[derive(Debug, Serialize, Deserialize)]
pub enum ServerMessage {
    HelloOk {
        protocol: u32,
        /// How many other clients are editing this document right now.
        peers: u32,
    },
    /// Another client's edits, relayed in the order the server received
    /// them. The server never echoes a client's own ops back to it, so
    /// applying these verbatim is echo-safe.
    Ops {
        actor: uuid::Uuid,
        ops: Vec<DocumentOp>,
    },
    /// A peer joined or left; `peers` counts the *other* editors.
    Peers {
        peers: u32,
    },
    /// A peer's presence changed.
    PresencePeer {
        actor: uuid::Uuid,
        state: PresenceState,
    },
    /// A peer disconnected; forget its presence.
    PresenceGone {
        actor: uuid::Uuid,
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
    /// Other clients currently editing the same document.
    pub peers: u32,
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
