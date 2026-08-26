//! The daemon: one document, one socket, N clients, no `Document` parsing.
//!
//! Responsibilities: own the document's file (reads on `OpenDocument`,
//! atomic writes on `SaveDocument`), append every op frame to a sidecar log
//! (`<file>.oplog.jsonl` — one JSON envelope per line, truncated on
//! `Rebase`), and **relay** each client's ops to every other client in the
//! order they arrived — the server's receive order is the document's total
//! order. A client never hears its own ops back, so applying relayed ops
//! verbatim is echo-safe. Everything stored or relayed is opaque bytes or
//! op envelopes, so daemon and app can be versions apart and still
//! cooperate. The daemon exits when its last client disconnects.

use std::collections::HashMap;
use std::io::Write as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use core_document::server::{ClientMessage, ServerMessage, SERVER_PROTOCOL_VERSION};

use crate::framing::{read_frame, write_frame};

/// Everyone currently connected. Writes to a client go through its mutex —
/// its own replies and relays from other clients' threads interleave here.
#[derive(Default, Clone)]
struct Roster {
    inner: Arc<Mutex<HashMap<u64, Peer>>>,
}

#[derive(Clone)]
struct Peer {
    stream: Arc<Mutex<UnixStream>>,
}

impl Roster {
    /// Returns how many peers were already present.
    fn join(&self, id: u64, peer: Peer) -> u32 {
        let mut inner = lock(&self.inner);
        let existing = inner.len() as u32;
        inner.insert(id, peer);
        existing
    }

    fn leave(&self, id: u64) {
        lock(&self.inner).remove(&id);
    }

    /// Send to every client except `from`; a peer whose write fails is
    /// dropped from the roster (its reader thread will notice separately).
    fn broadcast_except(&self, from: u64, message: &ServerMessage) {
        let peers: Vec<(u64, Peer)> = lock(&self.inner)
            .iter()
            .filter(|(id, _)| **id != from)
            .map(|(id, peer)| (*id, peer.clone()))
            .collect();
        for (id, peer) in peers {
            let mut stream = lock(&peer.stream);
            if write_frame(&mut *stream, message).is_err() {
                drop(stream);
                self.leave(id);
            }
        }
    }

    /// Tell every client how many *other* editors it currently has.
    fn announce_peers(&self) {
        let peers: Vec<Peer> = lock(&self.inner).values().cloned().collect();
        let total = peers.len();
        for peer in peers {
            let mut stream = lock(&peer.stream);
            let _ = write_frame(
                &mut *stream,
                &ServerMessage::Peers {
                    peers: total.saturating_sub(1) as u32,
                },
            );
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}

/// One client, served to disconnect on its own thread.
fn serve_client(stream: UnixStream, id: u64, roster: Roster) {
    let Ok(mut reader) = stream.try_clone() else {
        return;
    };
    let writer = Arc::new(Mutex::new(stream));

    // Handshake first: a version mismatch is a refusal, not a guess.
    let actor = match read_frame::<_, ClientMessage>(&mut reader) {
        Ok(ClientMessage::Hello { protocol, actor }) if protocol == SERVER_PROTOCOL_VERSION => {
            actor
        }
        Ok(ClientMessage::Hello { protocol, .. }) => {
            tracing::warn!(
                theirs = protocol,
                ours = SERVER_PROTOCOL_VERSION,
                "protocol mismatch"
            );
            return;
        }
        _ => {
            tracing::warn!("client spoke before Hello; dropping");
            return;
        }
    };

    let peers_at_join = roster.join(
        id,
        Peer {
            stream: Arc::clone(&writer),
        },
    );
    {
        let mut stream = lock(&writer);
        if write_frame(
            &mut *stream,
            &ServerMessage::HelloOk {
                protocol: SERVER_PROTOCOL_VERSION,
                peers: peers_at_join,
            },
        )
        .is_err()
        {
            roster.leave(id);
            return;
        }
    }
    roster.announce_peers();
    tracing::info!(%actor, peers = peers_at_join, "client joined");

    // Serve until disconnect (or a broken frame — no resync point here).
    while let Ok(message) = read_frame::<_, ClientMessage>(&mut reader) {
        match message {
            ClientMessage::Hello { .. } => {
                tracing::warn!("repeated Hello mid-session; ignoring");
            }
            ClientMessage::Ops(ops) => {
                if let Err(err) = append_ops(actor, &ops) {
                    tracing::error!("op log append failed: {err}");
                }
                // Receive order is the total order; relaying inside this
                // reader loop means no later op of this client can overtake
                // these on any peer.
                roster.broadcast_except(id, &ServerMessage::Ops { actor, ops });
            }
            ClientMessage::Presence(state) => {
                // Now-state, not history: relayed, never logged.
                roster.broadcast_except(id, &ServerMessage::PresencePeer { actor, state });
            }
            ClientMessage::Rebase => {
                if let Err(err) = truncate_oplog() {
                    tracing::error!("op log truncate failed: {err}");
                }
            }
            ClientMessage::SaveDocument {
                path,
                bytes,
                at_seq,
            } => {
                let reply = match write_atomically(&path, &bytes) {
                    Ok(()) => {
                        set_oplog_home(&path);
                        ServerMessage::SaveCompleted { path, at_seq }
                    }
                    Err(err) => ServerMessage::SaveFailed {
                        path,
                        error: err.to_string(),
                    },
                };
                let mut stream = lock(&writer);
                if write_frame(&mut *stream, &reply).is_err() {
                    break;
                }
            }
            ClientMessage::OpenDocument { path, token } => {
                let reply = match std::fs::read(&path) {
                    Ok(bytes) => {
                        set_oplog_home(&path);
                        ServerMessage::Opened { token, path, bytes }
                    }
                    Err(err) => ServerMessage::OpenFailed {
                        token,
                        path,
                        error: err.to_string(),
                    },
                };
                let mut stream = lock(&writer);
                if write_frame(&mut *stream, &reply).is_err() {
                    break;
                }
            }
        }
    }

    roster.leave(id);
    roster.announce_peers();
    roster.broadcast_except(id, &ServerMessage::PresenceGone { actor });
    tracing::info!(%actor, "client left");
}

/// A save must never leave a half-written file where a document was: write
/// beside the target, fsync, rename over it.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
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

// The op log lives beside the document once we know where the document is.
// Before the first save/open of a session it accumulates next to the socket.
static OPLOG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
/// Appends and truncations serialize here — ops arrive from N client threads.
static OPLOG_WRITE: Mutex<()> = Mutex::new(());

fn set_oplog_home(document: &Path) {
    let home = document.with_extension("oplog.jsonl");
    let mut slot = lock(&OPLOG_PATH);
    if slot.as_deref() == Some(home.as_path()) {
        return;
    }
    // Ops recorded before the document had a file live in the unhomed log;
    // carry them over so the document's history starts at its beginning,
    // not at its first save.
    let orphan = crate::runtime_dir_for_logs().join("unhomed.oplog.jsonl");
    if let Ok(text) = std::fs::read_to_string(&orphan) {
        if !text.is_empty() {
            let appended = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&home)
                .and_then(|mut file| file.write_all(text.as_bytes()));
            match appended {
                Ok(()) => {
                    let _ = std::fs::write(&orphan, b"");
                    // The log's blob markers reference the unhomed store;
                    // the blobs move with the lines they back.
                    let orphan_blobs = orphan.with_extension("blobs");
                    let home_blobs = home.with_extension("blobs");
                    if let Ok(entries) = std::fs::read_dir(&orphan_blobs) {
                        let _ = std::fs::create_dir_all(&home_blobs);
                        for entry in entries.flatten() {
                            let _ =
                                std::fs::rename(entry.path(), home_blobs.join(entry.file_name()));
                        }
                        let _ = std::fs::remove_dir(&orphan_blobs);
                    }
                }
                Err(err) => tracing::warn!("unhomed op log migration failed: {err}"),
            }
        }
    }
    *slot = Some(home);
}

fn oplog_path() -> PathBuf {
    let slot = lock(&OPLOG_PATH);
    slot.clone()
        .unwrap_or_else(|| crate::runtime_dir_for_logs().join("unhomed.oplog.jsonl"))
}

/// Payload strings at or above this length are pulled out of the log into
/// the content-addressed blob store. Well under any real import, well over
/// any parameter payload.
const BLOB_EXTRACT_THRESHOLD: usize = 256 * 1024;

/// The log rotates once it passes this size; one previous generation is
/// kept (`.oplog.jsonl.1`).
const OPLOG_ROTATE_BYTES: u64 = 64 * 1024 * 1024;

fn append_ops(actor: uuid::Uuid, ops: &[core_document::op::DocumentOp]) -> std::io::Result<()> {
    let _guard = lock(&OPLOG_WRITE);
    let path = oplog_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Rotation before append: a single import op can be large, but the cap
    // is about unbounded sessions, not about splitting one op.
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > OPLOG_ROTATE_BYTES) {
        let _ = std::fs::rename(&path, path.with_extension("jsonl.1"));
    }
    let blob_dir = path.with_extension("blobs");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    for op in ops {
        let mut value = serde_json::to_value(op)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        extract_blobs(&mut value, &blob_dir);
        let envelope = serde_json::json!({
            "v": core_document::op::OP_PROTOCOL_VERSION,
            "actor": actor,
            "op": value,
        });
        let line = serde_json::to_string(&envelope)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writeln!(file, "{line}")?;
    }
    Ok(())
}

/// Pull large payload strings out of an op's JSON into the blob store,
/// leaving a `blob:sha256:<hex>` marker. A generic walk over fields named
/// `bytes`, not op knowledge — the daemon stays schema-blind, and two
/// imports of the same file share one stored blob.
fn extract_blobs(value: &mut serde_json::Value, blob_dir: &Path) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if key == "bytes" {
                    if let serde_json::Value::String(payload) = child {
                        if payload.len() >= BLOB_EXTRACT_THRESHOLD {
                            use base64::Engine as _;
                            use sha2::Digest as _;
                            let decoded = base64::engine::general_purpose::STANDARD
                                .decode(payload.as_bytes())
                                .unwrap_or_else(|_| payload.clone().into_bytes());
                            let hash = hex_digest(sha2::Sha256::digest(&decoded));
                            let target = blob_dir.join(&hash);
                            let stored = target.exists()
                                || (std::fs::create_dir_all(blob_dir).is_ok()
                                    && std::fs::write(&target, &decoded).is_ok());
                            if stored {
                                *child = serde_json::Value::String(format!("blob:sha256:{hash}"));
                            }
                            continue;
                        }
                    }
                }
                extract_blobs(child, blob_dir);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items.iter_mut() {
                extract_blobs(child, blob_dir);
            }
        }
        _ => {}
    }
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    digest.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn truncate_oplog() -> std::io::Result<()> {
    let _guard = lock(&OPLOG_WRITE);
    let path = oplog_path();
    if path.exists() {
        std::fs::write(path, b"")?;
    }
    Ok(())
}

/// Run the daemon on `socket`: accept clients, serve each on its own
/// thread, exit when the last one leaves.
pub fn run(socket: &Path) -> std::io::Result<()> {
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // A stale socket from a dead daemon would make bind fail forever.
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    tracing::info!(socket = %socket.display(), "printcad-serverd listening");

    let roster = Roster::default();
    let live = Arc::new(AtomicU64::new(0));
    let ids = AtomicU64::new(0);
    // The first drop-to-zero ends the daemon.
    let (exit_tx, exit_rx) = std::sync::mpsc::channel::<()>();

    let accept_roster = roster.clone();
    let accept_live = Arc::clone(&live);
    let accept_listener = listener.try_clone()?;
    std::thread::spawn(move || {
        for stream in accept_listener.incoming().flatten() {
            let id = ids.fetch_add(1, Ordering::Relaxed);
            let roster = accept_roster.clone();
            let live = Arc::clone(&accept_live);
            let exit_tx = exit_tx.clone();
            live.fetch_add(1, Ordering::AcqRel);
            std::thread::spawn(move || {
                serve_client(stream, id, roster);
                if live.fetch_sub(1, Ordering::AcqRel) == 1 {
                    let _ = exit_tx.send(());
                }
            });
        }
    });

    // Block until the last client leaves.
    let _ = exit_rx.recv();
    let _ = std::fs::remove_file(socket);
    tracing::info!("last client disconnected; printcad-serverd exiting");
    Ok(())
}
