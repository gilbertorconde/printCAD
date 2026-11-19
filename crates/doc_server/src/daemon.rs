//! The daemon: one document, one socket, N clients, no `Document` parsing.
//!
//! Responsibilities: own the document's file (reads on `OpenDocument`,
//! atomic writes on `SaveDocument`), append every op batch to a sidecar log
//! (`<file>.oplog.jsonl`: one JSON envelope per line), and put the clients'
//! batches in **one order**: under a single lock each batch gets the next
//! sequence number, goes into the log and the unsaved tail, and is queued
//! for every client, as `Ops` to the others and `Acked` to its sender. Every
//! message to a client goes through that client's queue and its writer
//! thread, so all clients hear all batches in the same order, and a reply
//! never overtakes a batch queued before it. A client joining gets the file
//! and then the tail, both taken under the same locks that order batches
//! and saves, so it misses none and hears none twice. Everything stored or
//! relayed is opaque bytes or op envelopes, so daemon and app can be
//! versions apart and still cooperate. The daemon exits when its last
//! client disconnects.

use local_ipc::{Listener as UnixListener, Stream as UnixStream};
use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

use core_document::op::DocumentOp;
use core_document::server::{ClientMessage, SERVER_PROTOCOL_VERSION, ServerMessage};

use crate::file_io::write_atomically;
use crate::framing::{read_frame, write_frame};

/// The document's shared state: who is connected and the order of edits.
#[derive(Default)]
struct Hub {
    state: Mutex<State>,
    /// Held across a read or write of the document's file and the change
    /// to the tail that goes with it, so a joiner's file and tail, or a
    /// save and the tail it leaves, describe one moment. Taken before
    /// `state`, never after.
    file: Mutex<()>,
}

#[derive(Default)]
struct State {
    /// The last sequence number given to a batch.
    seq: u64,
    /// Batches since the last save, in order: the unsaved present a
    /// joining client gets after the file.
    tail: Vec<Logged>,
    clients: HashMap<u64, Client>,
}

struct Client {
    /// Everything this client is to hear, written in order by its writer
    /// thread.
    outbox: Sender<ServerMessage>,
    /// Batches this client has sent.
    batches: u64,
}

struct Logged {
    seq: u64,
    actor: uuid::Uuid,
    /// The connection that sent it, and which of its batches it was.
    client: u64,
    batch: u64,
    ops: Vec<DocumentOp>,
}

impl State {
    /// Queue `message` for every client but `except`.
    fn tell_others(&self, except: u64, message: impl Fn() -> ServerMessage) {
        for (id, client) in &self.clients {
            if *id != except {
                let _ = client.outbox.send(message());
            }
        }
    }

    fn tell(&self, id: u64, message: ServerMessage) {
        if let Some(client) = self.clients.get(&id) {
            let _ = client.outbox.send(message);
        }
    }

    /// Tell every client how many *other* editors it currently has.
    fn announce_peers(&self) {
        let peers = self.clients.len().saturating_sub(1) as u32;
        for client in self.clients.values() {
            let _ = client.outbox.send(ServerMessage::Peers { peers });
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}

/// One client, served to disconnect on its own thread.
fn serve_client(stream: UnixStream, id: u64, hub: &Hub) {
    let Ok(mut reader) = stream.try_clone() else {
        return;
    };

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

    // The writer: one thread per client drains its queue onto the socket.
    // A failed write shuts the socket, which ends the reader below too.
    let (outbox, queue) = channel::<ServerMessage>();
    let mut writer = stream;
    std::thread::spawn(move || {
        for message in queue {
            if write_frame(&mut writer, &message).is_err() {
                let _ = writer.shutdown(std::net::Shutdown::Both);
                return;
            }
        }
    });

    let peers_at_join = {
        let mut state = lock(&hub.state);
        let peers = state.clients.len() as u32;
        let _ = outbox.send(ServerMessage::HelloOk {
            protocol: SERVER_PROTOCOL_VERSION,
            peers,
        });
        state.clients.insert(id, Client { outbox, batches: 0 });
        state.announce_peers();
        peers
    };
    tracing::info!(%actor, peers = peers_at_join, "client joined");

    // Serve until disconnect, or a broken frame: there is no resync point.
    while let Ok(message) = read_frame::<_, ClientMessage>(&mut reader) {
        match message {
            ClientMessage::Hello { .. } => {
                tracing::warn!("repeated Hello mid-session; ignoring");
            }
            ClientMessage::Ops(ops) => order_batch(hub, id, actor, ops),
            ClientMessage::Presence(state) => {
                // Now-state, not history: relayed, never logged.
                lock(&hub.state).tell_others(id, || ServerMessage::PresencePeer {
                    actor,
                    state: state.clone(),
                });
            }
            ClientMessage::Rebase => {
                let mut state = lock(&hub.state);
                if state.clients.len() > 1 {
                    // The others' replicas hold the tail; clearing it would
                    // leave every later joiner without their unsaved edits.
                    tracing::warn!(
                        %actor,
                        "a rebase while others edit the document; the log and tail stay"
                    );
                } else {
                    if let Err(err) = truncate_oplog() {
                        tracing::error!("op log truncate failed: {err}");
                    }
                    state.tail.clear();
                }
            }
            ClientMessage::SaveDocument {
                path,
                bytes,
                at_seq,
                seen_seq,
                sent_batches,
            } => {
                let _file = lock(&hub.file);
                let written = write_atomically(&path, &bytes);
                let mut state = lock(&hub.state);
                let reply = match written {
                    Ok(()) => {
                        set_oplog_home(&path);
                        // What the snapshot holds leaves the tail: the
                        // others' batches this client had applied and its
                        // own sent before the snapshot. The rest, sent
                        // while it was packed, is still unsaved.
                        state.tail.retain(|logged| {
                            if logged.client == id {
                                sent_batches.is_some_and(|sent| logged.batch >= sent)
                            } else {
                                seen_seq.is_some_and(|seen| logged.seq > seen)
                            }
                        });
                        ServerMessage::SaveCompleted { path, at_seq }
                    }
                    Err(err) => ServerMessage::SaveFailed {
                        path,
                        error: err.to_string(),
                    },
                };
                state.tell(id, reply);
            }
            ClientMessage::OpenDocument { path, token } => {
                let _file = lock(&hub.file);
                let read = std::fs::read(&path);
                let state = lock(&hub.state);
                match read {
                    Ok(bytes) => {
                        set_oplog_home(&path);
                        // The file is the saved past; the tail is the
                        // unsaved present. Queued together under the lock
                        // that orders batches: every batch heard before
                        // `Opened` is in one of them, every one after is
                        // newer than both.
                        state.tell(id, ServerMessage::Opened { token, path, bytes });
                        for logged in &state.tail {
                            state.tell(
                                id,
                                ServerMessage::Ops {
                                    actor: logged.actor,
                                    seq: logged.seq,
                                    ops: logged.ops.clone(),
                                },
                            );
                        }
                    }
                    Err(err) => state.tell(
                        id,
                        ServerMessage::OpenFailed {
                            token,
                            path,
                            error: err.to_string(),
                        },
                    ),
                }
            }
        }
    }

    let mut state = lock(&hub.state);
    state.clients.remove(&id);
    state.announce_peers();
    state.tell_others(id, || ServerMessage::PresenceGone { actor });
    drop(state);
    tracing::info!(%actor, "client left");
}

/// Give a client's batch its place in the document's order: the next
/// sequence number, the log, the tail and every client's queue, all under
/// one lock, so no two clients ever hear two batches in different orders.
fn order_batch(hub: &Hub, id: u64, actor: uuid::Uuid, ops: Vec<DocumentOp>) {
    let mut state = lock(&hub.state);
    state.seq += 1;
    let seq = state.seq;
    let batch = match state.clients.get_mut(&id) {
        Some(client) => {
            client.batches += 1;
            client.batches - 1
        }
        None => return,
    };
    if let Err(err) = append_ops(actor, &ops) {
        tracing::error!("op log append failed: {err}");
    }
    for (peer, client) in &state.clients {
        let message = if *peer == id {
            ServerMessage::Acked { seq }
        } else {
            ServerMessage::Ops {
                actor,
                seq,
                ops: ops.clone(),
            }
        };
        let _ = client.outbox.send(message);
    }
    state.tail.push(Logged {
        seq,
        actor,
        client: id,
        batch,
        ops,
    });
}
// The op log lives beside the document once we know where the document is.
// Before the first save/open of a session it accumulates in the unhomed log.
static OPLOG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
/// Appends and truncations serialize here: ops arrive from N client threads.
static OPLOG_WRITE: Mutex<()> = Mutex::new(());

/// Where this daemon's ops live until its document has a file.
///
/// One per daemon, named after the socket it serves. A single shared file
/// would be a rendezvous between documents: two unsaved documents would both
/// write to it, and whichever saved first would take the other's history
/// into its own log and leave it with none.
static UNHOMED_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

fn claim_unhomed_oplog(socket: &Path) {
    // The whole path distinguishes equal socket names in different directories.
    let key = crate::path_key(socket);
    *lock(&UNHOMED_PATH) =
        Some(crate::runtime_dir_for_logs().join(format!("unhomed-{key:016x}.oplog.jsonl")));
}

fn unhomed_oplog() -> PathBuf {
    let claimed = lock(&UNHOMED_PATH).clone();
    claimed.unwrap_or_else(|| crate::runtime_dir_for_logs().join("unhomed.oplog.jsonl"))
}

fn document_oplog(document: &Path) -> PathBuf {
    if document
        .extension()
        .is_some_and(|ext| ext == "gz" || ext == "zst")
    {
        let mut name = document.as_os_str().to_os_string();
        name.push(".oplog.jsonl");
        PathBuf::from(name)
    } else {
        document.with_extension("oplog.jsonl")
    }
}

fn set_oplog_home(document: &Path) {
    let home = document_oplog(document);
    let mut slot = lock(&OPLOG_PATH);
    if slot.as_deref() == Some(home.as_path()) {
        return;
    }
    // Ops recorded before the document had a file live in the unhomed log;
    // carry them over so the document's history starts at its beginning,
    // not at its first save.
    let orphan = unhomed_oplog();
    if let Ok(text) = std::fs::read_to_string(&orphan)
        && !text.is_empty()
    {
        let appended = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&home)
            .and_then(|mut file| file.write_all(text.as_bytes()));
        match appended {
            Ok(()) => {
                // Gone rather than emptied: this daemon's history has a home
                // now, and a file left behind is one more thing to explain.
                let _ = std::fs::remove_file(&orphan);
                // The log's blob markers reference the unhomed store;
                // the blobs move with the lines they back.
                let orphan_blobs = orphan.with_extension("blobs");
                let home_blobs = home.with_extension("blobs");
                if let Ok(entries) = std::fs::read_dir(&orphan_blobs) {
                    let moved = std::fs::create_dir_all(&home_blobs).and_then(|()| {
                        entries.flatten().try_for_each(|entry| {
                            std::fs::rename(entry.path(), home_blobs.join(entry.file_name()))
                        })
                    });
                    match moved {
                        Ok(()) => {
                            let _ = std::fs::remove_dir(&orphan_blobs);
                        }
                        Err(err) => tracing::warn!(
                            "the op log's blobs did not all move to {}: {err}",
                            home_blobs.display()
                        ),
                    }
                }
            }
            Err(err) => tracing::warn!("unhomed op log migration failed: {err}"),
        }
    }
    *slot = Some(home);
}

fn oplog_path() -> PathBuf {
    let slot = lock(&OPLOG_PATH);
    slot.clone().unwrap_or_else(unhomed_oplog)
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
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > OPLOG_ROTATE_BYTES)
        && let Err(err) = std::fs::rename(&path, path.with_extension("jsonl.1"))
    {
        tracing::warn!("the op log did not rotate, so it grows past its cap: {err}");
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
/// `bytes`, not op knowledge: the daemon stays schema-blind, and two
/// imports of the same file share one stored blob.
fn extract_blobs(value: &mut serde_json::Value, blob_dir: &Path) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if key == "bytes"
                    && let serde_json::Value::String(payload) = child
                    && payload.len() >= BLOB_EXTRACT_THRESHOLD
                {
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
    claim_unhomed_oplog(socket);
    // A stale socket from a dead daemon would make bind fail forever.
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    tracing::info!(socket = %socket.display(), "printcad-serverd listening");

    let hub = Arc::new(Hub::default());
    let live = Arc::new(AtomicU64::new(0));
    let ids = AtomicU64::new(0);
    // The first drop-to-zero ends the daemon.
    let (exit_tx, exit_rx) = std::sync::mpsc::channel::<()>();

    let accept_hub = Arc::clone(&hub);
    let accept_live = Arc::clone(&live);
    let accept_listener = listener.try_clone()?;
    std::thread::spawn(move || {
        for stream in accept_listener.incoming().flatten() {
            let id = ids.fetch_add(1, Ordering::Relaxed);
            let hub = Arc::clone(&accept_hub);
            let live = Arc::clone(&accept_live);
            let exit_tx = exit_tx.clone();
            live.fetch_add(1, Ordering::AcqRel);
            std::thread::spawn(move || {
                serve_client(stream, id, &hub);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressed_documents_keep_distinct_logs_and_blobs() {
        let paths = ["a.prtcad", "a.prtcad.gz", "a.prtcad.zst"];
        let logs: Vec<_> = paths.iter().map(|p| document_oplog(Path::new(p))).collect();
        assert_eq!(logs[0], Path::new("a.oplog.jsonl"));
        for i in 0..logs.len() {
            for j in 0..i {
                assert_ne!(logs[i], logs[j]);
                assert_ne!(
                    logs[i].with_extension("blobs"),
                    logs[j].with_extension("blobs")
                );
            }
        }
        let tab = uuid::Uuid::nil();
        assert_eq!(
            crate::socket_path_for_untitled(tab),
            crate::socket_path_for_untitled(tab)
        );
        assert_ne!(
            crate::socket_path_for_untitled(tab),
            crate::socket_path_for_untitled(uuid::Uuid::from_u128(1))
        );
        let path = Path::new("stable.prtcad");
        assert_eq!(
            crate::socket_path_for(path),
            crate::runtime_dir().join(format!("{:016x}.sock", crate::path_key(path)))
        );
    }
}
