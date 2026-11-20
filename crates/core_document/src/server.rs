//! The document server seam.
//!
//! printCAD is always-multiplayer in architecture: a *document server* owns
//! the document's file and its op log, and the app is a client. By default
//! the server is a local daemon (`printcad-serverd`, one per document, unix
//! socket). The trait is deliberately message-shaped: what
//! crosses [`DocumentServer::send`]/[`DocumentServer::poll`] **is** the wire
//! protocol, serde-serialized verbatim by the socket transport, so promoting
//! an implementation from in-process to daemon to remote changes transport,
//! never semantics.
//!
//! The server puts every client's ops in one order: each batch gets the
//! next sequence number and goes out to every client in that order, as
//! [`ServerMessage::Ops`] to the others and [`ServerMessage::Acked`] to its
//! sender. A client applies its own edits at once and others' as they come,
//! rolling its unconfirmed edits back beneath each and replaying them on
//! top (`Document::apply_server_ops`), so every replica ends in the state
//! the server's order gives: for two writes of one field, the later in that
//! order wins everywhere.
//!
//! The server never deserializes a `Document`. Snapshots cross the boundary
//! as opaque `.prtcad` container bytes (see `Document::save_to_bytes` /
//! `load_from_bytes`) and edits as [`DocumentOp`] envelopes, so a daemon
//! keeps serving clients whose document schema it has never seen.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::op::DocumentOp;

/// Version of the client↔server protocol; the `Hello` handshake refuses a
/// mismatch loudly rather than misreading frames quietly.
pub const SERVER_PROTOCOL_VERSION: u32 = 6;

/// A message whose container bytes travel beside it rather than inside it.
///
/// A document's archive is far too big to encode into the message itself: as
/// JSON it is an array of decimal numbers, four times the size of what it
/// carries, and a large document overruns the frame limit outright. The bytes
/// are lifted out before the message is encoded and put back after it is
/// decoded, so they cross as themselves.
pub trait Payload {
    /// The bytes that travel beside this message; empty for most of them.
    fn payload(&self) -> &[u8];
    /// Put bytes back into a message that was decoded without them.
    fn set_payload(&mut self, bytes: Vec<u8>);
}

impl Payload for ClientMessage {
    fn payload(&self) -> &[u8] {
        match self {
            ClientMessage::SaveDocument { bytes, .. } => bytes,
            _ => &[],
        }
    }

    fn set_payload(&mut self, payload: Vec<u8>) {
        if let ClientMessage::SaveDocument { bytes, .. } = self {
            *bytes = payload;
        }
    }
}

impl Payload for ServerMessage {
    fn payload(&self) -> &[u8] {
        match self {
            ServerMessage::Opened { bytes, .. } => bytes,
            _ => &[],
        }
    }

    fn set_payload(&mut self, payload: Vec<u8>) {
        if let ServerMessage::Opened { bytes, .. } = self {
            *bytes = payload;
        }
    }
}

/// Client → server messages.
#[derive(Debug, Serialize, Deserialize)]
pub enum ClientMessage {
    /// First message on a connection; anything else first is an error.
    /// `actor` identifies this client among the document's editors: it is
    /// how relayed ops name their author.
    Hello { protocol: u32, actor: uuid::Uuid },
    /// User edits drained from the document's outbox, in order: one
    /// batch, answered by [`ServerMessage::Acked`] once it has its place in
    /// the document's order. `batch` names it in the op log; a batch sent
    /// again under an id the server already ordered (a client that lost
    /// its connection before the answer came) is answered without being
    /// ordered twice.
    Ops {
        batch: uuid::Uuid,
        ops: Vec<DocumentOp>,
    },
    /// The client put another document in place of the one it had (a new
    /// document, a recovered copy): the ops logged so far do not describe
    /// its state. The server clears its log and unsaved tail only while
    /// this client is its only one; with others editing, their unsaved
    /// edits stay and the rebase is refused in the server's log.
    Rebase,
    /// Persist a client-serialized `.prtcad` container. `at_seq` is the
    /// client's mutation counter when the snapshot was taken; it rides back
    /// in [`ServerMessage::SaveCompleted`] so the client can decide whether
    /// the document is truly clean (edits may have landed mid-save).
    /// `seen_seq` and `sent_batches` say which ops the snapshot holds: the
    /// others' up to that sequence number, and the first `sent_batches`
    /// batches this client sent on this connection; `None` stands for all
    /// of them so far. The server keeps the rest in its unsaved tail for
    /// clients that join later.
    SaveDocument {
        path: PathBuf,
        /// Carried beside the message, not inside it (see [`Payload`]).
        #[serde(skip)]
        bytes: Vec<u8>,
        at_seq: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seen_seq: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sent_batches: Option<u64>,
    },
    /// Read a document's bytes. `token` correlates the eventual response
    /// with the request that asked for it (an open may be abandoned by a
    /// newer one).
    OpenDocument { path: PathBuf, token: u64 },
    /// The unsaved edits a session left in the document's log (the
    /// `recoverable` an open reports) come back: the server orders them
    /// after everything so far and sends them to every client.
    RestoreUnsaved,
    /// Let those edits go: the log no longer offers them.
    DiscardUnsaved,
    /// The log the daemon on `socket` kept while its document had no file
    /// (the tab's own, while it was untitled) joins this server's log at
    /// the next save that writes, ahead of what this server logged, so the
    /// document's log begins with its first edit. The save holds those
    /// edits, so the log marks them saved. Should this server stop before
    /// that save writes, the next one on its socket moves what it logged
    /// into that log.
    AdoptLog { socket: PathBuf },
    /// The edits in the log the daemon on `socket` kept while its document
    /// had no file (an untitled tab's, left by a crash) come back: the log
    /// joins this server's, and the server orders its edits after
    /// everything so far and sends them to every client.
    RestoreLog { socket: PathBuf },
    /// This client is done with the document: what it holds unsaved, it
    /// chose to let go. A client that leaves without it (one that crashed)
    /// leaves its unsaved edits in the log, for the next open to offer.
    Leave,
    /// Ephemeral presence: who this editor is and what they have selected.
    /// Relayed to peers, never logged: presence is now-state, not history.
    Presence(PresenceState),
}

/// What a peer sees of another editor: chiefly the selection, since a
/// selected body is stable, meaningful across viewports, and cheap. New
/// fields take serde defaults, so the struct grows without a protocol
/// change.
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
    /// Another client's batch of edits and its place in the document's
    /// order. Every client hears every batch in that order: the others' as
    /// these, its own as [`ServerMessage::Acked`]. A client that opens a
    /// document gets its unsaved tail as these, right after
    /// [`ServerMessage::Opened`].
    Ops {
        actor: uuid::Uuid,
        seq: u64,
        ops: Vec<DocumentOp>,
    },
    /// This client's oldest unanswered batch of ops has sequence number
    /// `seq`: every batch heard before this came earlier in the order,
    /// every one after it later.
    Acked {
        seq: u64,
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
    /// The document's file, followed by its unsaved tail as
    /// [`ServerMessage::Ops`].
    Opened {
        token: u64,
        path: PathBuf,
        /// Carried beside the message, not inside it (see [`Payload`]).
        #[serde(skip)]
        bytes: Vec<u8>,
        /// The newest sequence number in the order when the file was read:
        /// the tail that follows runs up to it.
        #[serde(default)]
        seq: u64,
        /// The server started where another stopped and could not read
        /// back the unsaved edits that one held: the file and tail lack
        /// them, and only a client that held them still has them.
        #[serde(default)]
        unsaved_lost: bool,
        /// Batches of unsaved edits the document's log holds from a
        /// session that ended without saving them, offered back:
        /// [`ClientMessage::RestoreUnsaved`] brings them into the order,
        /// [`ClientMessage::DiscardUnsaved`] lets them go.
        #[serde(default)]
        recoverable: u32,
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
/// to the local `printcad-serverd` and a direct-file fallback.
pub trait DocumentServer: Send {
    /// Human-readable implementation name for logs and the status bar.
    fn name(&self) -> &str;

    /// The app reads and writes the files itself, with no server process:
    /// no peers, no op log beside the file.
    fn standalone(&self) -> bool {
        false
    }

    /// Queue a message. Never blocks; delivery failures surface through
    /// [`Self::status`] and error messages in [`Self::poll`].
    fn send(&mut self, msg: ClientMessage);

    /// Drain responses that have arrived since the last poll. Called once
    /// per frame; must never block.
    fn poll(&mut self) -> Vec<ServerMessage>;

    fn status(&self) -> ServerStatus;

    /// Block until every queued write has been durably handled. Every exit
    /// path must call this: the process exiting mid-save truncates files.
    fn flush(&mut self);
}
