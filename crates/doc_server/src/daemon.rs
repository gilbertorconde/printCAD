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
//!
//! Each client's queue holds at most `queue_limit` messages it has not been
//! written yet; a client that falls further behind is disconnected, and
//! rejoins by opening the document again (the file and the tail).
//!
//! The log marks what the file holds: each batch's lines carry the batch's
//! id, and every save (and every time the log gets its home) adds a line
//! naming the batches still unsaved. A daemon that starts where one died
//! reads its document from `<socket>.home` and its unsaved tail from the
//! log, so a client joining it still gets the edits nobody saved.

use local_ipc::{Listener as UnixListener, Stream as UnixStream};
use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

use core_document::op::DocumentOp;
use core_document::server::{ClientMessage, SERVER_PROTOCOL_VERSION, ServerMessage};

use crate::file_io::write_atomically;
use crate::framing::{read_frame, write_frame};

/// How many messages a client may have queued and unwritten before it is
/// disconnected, unless the daemon is started with another
/// `--queue-limit`. A client's reader takes frames off its socket as they
/// come, so only a client that has stopped altogether falls this far behind.
pub const DEFAULT_QUEUE_LIMIT: usize = 4096;

/// The document's shared state: who is connected and the order of edits.
struct Hub {
    state: Mutex<State>,
    /// Held across a read or write of the document's file and the change
    /// to the tail that goes with it, so a joiner's file and tail, or a
    /// save and the tail it leaves, describe one moment. Taken before
    /// `state`, never after.
    file: Mutex<()>,
    queue_limit: usize,
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
    actor: uuid::Uuid,
    /// Everything this client is to hear, written in order by its writer
    /// thread; the flag says whether the message counts in `behind`.
    outbox: Sender<(ServerMessage, bool)>,
    /// Counted messages queued and not yet written.
    behind: Arc<AtomicUsize>,
    /// The socket, shut to disconnect a client that fell behind.
    socket: UnixStream,
    /// Batches this client has sent.
    batches: u64,
}

impl Client {
    /// Queue `message`; false once the client is too far behind to keep.
    fn send(&self, message: ServerMessage, limit: usize) -> bool {
        let behind = self.behind.fetch_add(1, Ordering::AcqRel) + 1;
        let _ = self.outbox.send((message, true));
        behind <= limit
    }

    /// Queue a reply to the client's own request, which it asked for and
    /// so never counts against it: the file and tail an open sends.
    fn reply(&self, message: ServerMessage) {
        let _ = self.outbox.send((message, false));
    }
}

struct Logged {
    seq: u64,
    /// The batch's id in the op log.
    id: uuid::Uuid,
    actor: uuid::Uuid,
    /// The connection that sent it, and which of its batches it was;
    /// `None` for a batch read back from the log, which no connection
    /// here sent and every client holds (from the file and tail it opened,
    /// or from before the daemon restarted), so any save takes it.
    client: Option<u64>,
    batch: u64,
    ops: Vec<DocumentOp>,
}

impl State {
    /// Queue `message` for every client but `except`.
    fn tell_others(&mut self, except: u64, limit: usize, message: impl Fn() -> ServerMessage) {
        let mut stalled = Vec::new();
        for (id, client) in &self.clients {
            if *id != except && !client.send(message(), limit) {
                stalled.push(*id);
            }
        }
        self.disconnect(stalled, limit);
    }

    fn tell(&mut self, id: u64, limit: usize, message: ServerMessage) {
        if let Some(client) = self.clients.get(&id)
            && !client.send(message, limit)
        {
            self.disconnect(vec![id], limit);
        }
    }

    fn reply(&self, id: u64, message: ServerMessage) {
        if let Some(client) = self.clients.get(&id) {
            client.reply(message);
        }
    }

    /// Tell every client how many *other* editors it currently has.
    fn announce_peers(&mut self, limit: usize) {
        let peers = self.clients.len().saturating_sub(1) as u32;
        let stalled: Vec<u64> = self
            .clients
            .iter()
            .filter(|(_, client)| !client.send(ServerMessage::Peers { peers }, limit))
            .map(|(id, _)| *id)
            .collect();
        self.disconnect(stalled, limit);
    }

    /// Let `id` go: out of the document's clients, its peers told.
    fn leave(&mut self, id: u64, limit: usize) {
        if let Some(client) = self.clients.remove(&id) {
            let actor = client.actor;
            self.announce_peers(limit);
            self.tell_others(id, limit, || ServerMessage::PresenceGone { actor });
        }
    }

    /// Disconnect clients too far behind: their sockets shut, which ends
    /// their writer and reader threads, and they leave. A client that
    /// reconnects opens the document again and gets the file and the tail.
    fn disconnect(&mut self, stalled: Vec<u64>, limit: usize) {
        for id in stalled {
            if let Some(client) = self.clients.get(&id) {
                tracing::warn!(
                    actor = %client.actor,
                    limit,
                    "a client fell too far behind; disconnecting it"
                );
                let _ = client.socket.shutdown(std::net::Shutdown::Both);
                self.leave(id, limit);
            }
        }
    }

    /// The ids of the batches still unsaved, for the log's mark; an empty
    /// batch has no lines in the log and changes nothing, so it is left out.
    fn unsaved(&self) -> Vec<uuid::Uuid> {
        self.tail
            .iter()
            .filter(|logged| !logged.ops.is_empty())
            .map(|logged| logged.id)
            .collect()
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

    let Ok(socket) = stream.try_clone() else {
        return;
    };
    // The writer: one thread per client drains its queue onto the socket.
    // A failed write shuts the socket, which ends the reader below too.
    let (outbox, queue) = channel::<(ServerMessage, bool)>();
    let behind = Arc::new(AtomicUsize::new(0));
    let written = Arc::clone(&behind);
    let mut writer = stream;
    std::thread::spawn(move || {
        for (message, counted) in queue {
            if write_frame(&mut writer, &message).is_err() {
                let _ = writer.shutdown(std::net::Shutdown::Both);
                return;
            }
            if counted {
                written.fetch_sub(1, Ordering::AcqRel);
            }
        }
    });

    let limit = hub.queue_limit;
    let peers_at_join = {
        let mut state = lock(&hub.state);
        let peers = state.clients.len() as u32;
        let _ = outbox.send((
            ServerMessage::HelloOk {
                protocol: SERVER_PROTOCOL_VERSION,
                peers,
            },
            false,
        ));
        state.clients.insert(
            id,
            Client {
                actor,
                outbox,
                behind,
                socket,
                batches: 0,
            },
        );
        state.announce_peers(limit);
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
                lock(&hub.state).tell_others(id, limit, || ServerMessage::PresencePeer {
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
                        state.tail.retain(|logged| match logged.client {
                            Some(client) if client == id => {
                                sent_batches.is_some_and(|sent| logged.batch >= sent)
                            }
                            Some(_) => seen_seq.is_some_and(|seen| logged.seq > seen),
                            None => false,
                        });
                        mark_unsaved(&state.unsaved(), Some(&path));
                        ServerMessage::SaveCompleted { path, at_seq }
                    }
                    Err(err) => ServerMessage::SaveFailed {
                        path,
                        error: err.to_string(),
                    },
                };
                state.tell(id, limit, reply);
            }
            ClientMessage::OpenDocument { path, token } => {
                let _file = lock(&hub.file);
                let read = std::fs::read(&path);
                let state = lock(&hub.state);
                match read {
                    Ok(bytes) => {
                        if set_oplog_home(&path) {
                            mark_unsaved(&state.unsaved(), Some(&path));
                        }
                        // The file is the saved past; the tail is the
                        // unsaved present. Queued together under the lock
                        // that orders batches: every batch heard before
                        // `Opened` is in one of them, every one after is
                        // newer than both.
                        state.reply(id, ServerMessage::Opened { token, path, bytes });
                        for logged in &state.tail {
                            state.reply(
                                id,
                                ServerMessage::Ops {
                                    actor: logged.actor,
                                    seq: logged.seq,
                                    ops: logged.ops.clone(),
                                },
                            );
                        }
                    }
                    Err(err) => state.reply(
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

    lock(&hub.state).leave(id, limit);
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
    let batch_id = uuid::Uuid::new_v4();
    if let Err(err) = append_ops(actor, batch_id, &ops) {
        tracing::error!("op log append failed: {err}");
    }
    let limit = hub.queue_limit;
    let stalled: Vec<u64> = state
        .clients
        .iter()
        .filter(|(peer, client)| {
            let message = if **peer == id {
                ServerMessage::Acked { seq }
            } else {
                ServerMessage::Ops {
                    actor,
                    seq,
                    ops: ops.clone(),
                }
            };
            !client.send(message, limit)
        })
        .map(|(peer, _)| *peer)
        .collect();
    state.tail.push(Logged {
        seq,
        id: batch_id,
        actor,
        client: Some(id),
        batch,
        ops,
    });
    state.disconnect(stalled, limit);
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

/// Home the op log beside `document`; true when it moved there now.
fn set_oplog_home(document: &Path) -> bool {
    let home = document_oplog(document);
    let mut slot = lock(&OPLOG_PATH);
    if slot.as_deref() == Some(home.as_path()) {
        return false;
    }
    remember_home(document);
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
    true
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

fn append_ops(
    actor: uuid::Uuid,
    batch: uuid::Uuid,
    ops: &[core_document::op::DocumentOp],
) -> std::io::Result<()> {
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
            "batch": batch,
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

/// Where this daemon notes its document's path once it has one:
/// `<socket>.home`, read by the next daemon on the socket if this one dies.
static HOME_NOTE: Mutex<Option<PathBuf>> = Mutex::new(None);

fn remember_home(document: &Path) {
    let Some(note) = lock(&HOME_NOTE).clone() else {
        return;
    };
    match serde_json::to_vec(document) {
        Ok(text) => {
            if let Err(err) = write_atomically(&note, &text) {
                tracing::warn!("the document's path was not noted for a restart: {err}");
            }
        }
        Err(err) => tracing::warn!("the document's path cannot be noted for a restart: {err}"),
    }
}

/// The document a daemon that died on this socket was serving.
fn noted_home(note: &Path) -> Option<PathBuf> {
    let text = std::fs::read(note).ok()?;
    serde_json::from_slice(&text).ok()
}

/// What identifies the file a save left: its length and when it was
/// written. A mark whose file has changed since describes another file.
fn file_stamp(document: &Path) -> Option<serde_json::Value> {
    let meta = std::fs::metadata(document).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    Some(serde_json::json!({
        "len": meta.len(),
        "modified_ns": u64::try_from(modified.as_nanos()).ok()?,
    }))
}

/// Add a mark to the log: the batches still unsaved (every batch logged
/// before this line and not named here is in the file), with the file's
/// stamp.
fn mark_unsaved(unsaved: &[uuid::Uuid], document: Option<&Path>) {
    let _guard = lock(&OPLOG_WRITE);
    let path = oplog_path();
    let mark = serde_json::json!({
        "v": core_document::op::OP_PROTOCOL_VERSION,
        "unsaved": unsaved,
        "file": document.and_then(file_stamp),
    });
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| writeln!(file, "{mark}"));
    if let Err(err) = written {
        tracing::error!("the op log's unsaved mark was not written: {err}");
    }
}

/// A batch read back from the log.
struct Restored {
    id: uuid::Uuid,
    actor: uuid::Uuid,
    ops: Vec<DocumentOp>,
}

/// The batches `log` holds that `document` does not: those its last mark
/// names, then every batch after it (every batch, before any mark). None
/// when the log cannot say: a line no batch id names where the tail would
/// be (a daemon that marks nothing wrote it, so a save may have taken
/// it), a file changed since the mark, a batch it names that the log no
/// longer holds (rotated out of both generations), a blob or an op that
/// does not read. The previous generation is read first, so a tail the
/// log rotated over once is still found.
fn restore_tail(log: &Path, document: &Path) -> Option<Vec<Restored>> {
    let mut lines = Vec::new();
    for generation in [log.with_extension("jsonl.1"), log.to_path_buf()] {
        if let Ok(text) = std::fs::read_to_string(&generation) {
            lines.extend(
                text.lines()
                    .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok()),
            );
        }
    }
    let mark = lines.iter().rposition(|line| line.get("unsaved").is_some());
    let (listed, after) = match mark {
        Some(at) => {
            let mark = &lines[at];
            if !mark["file"].is_null() && Some(&mark["file"]) != file_stamp(document).as_ref() {
                tracing::warn!("the document changed since its log was marked; no unsaved tail");
                return None;
            }
            let listed: std::collections::HashSet<String> = mark["unsaved"]
                .as_array()?
                .iter()
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect();
            (listed, at + 1)
        }
        None => (Default::default(), 0),
    };
    let blob_dir = log.with_extension("blobs");
    let mut tail: Vec<Restored> = Vec::new();
    let mut found = std::collections::HashSet::new();
    for (at, line) in lines.iter().enumerate() {
        if line.get("op").is_none() {
            continue;
        }
        let batch = line["batch"].as_str();
        let wanted = at >= after || batch.is_some_and(|id| listed.contains(id));
        if !wanted {
            continue;
        }
        if at < after {
            found.extend(batch.map(str::to_owned));
        }
        let Some(batch) = batch.and_then(|id| id.parse::<uuid::Uuid>().ok()) else {
            tracing::warn!(
                "the op log holds edits of no batch past its last mark; no unsaved tail"
            );
            return None;
        };
        let mut op = line["op"].clone();
        if !inline_blobs(&mut op, &blob_dir) {
            tracing::warn!("a blob the op log names is missing; no unsaved tail");
            return None;
        }
        let op: DocumentOp = match serde_json::from_value(op) {
            Ok(op) => op,
            Err(err) => {
                tracing::warn!(
                    "an unsaved edit in the op log does not read ({err}); no unsaved tail"
                );
                return None;
            }
        };
        match tail.last_mut() {
            Some(last) if last.id == batch => last.ops.push(op),
            _ => tail.push(Restored {
                id: batch,
                actor: line["actor"]
                    .as_str()
                    .and_then(|actor| actor.parse().ok())
                    .unwrap_or_default(),
                ops: vec![op],
            }),
        }
    }
    if found.len() < listed.len() {
        tracing::warn!("unsaved edits the op log names are gone from it; no unsaved tail");
        return None;
    }
    Some(tail)
}

/// Put back the payloads `extract_blobs` moved to the blob store; false
/// when one is missing.
fn inline_blobs(value: &mut serde_json::Value, blob_dir: &Path) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter_mut().all(|(key, child)| {
            if key == "bytes"
                && let serde_json::Value::String(text) = child
                && let Some(hash) = text.strip_prefix("blob:sha256:")
            {
                use base64::Engine as _;
                return match std::fs::read(blob_dir.join(hash)) {
                    Ok(bytes) => {
                        *text = base64::engine::general_purpose::STANDARD.encode(bytes);
                        true
                    }
                    Err(_) => false,
                };
            }
            inline_blobs(child, blob_dir)
        }),
        serde_json::Value::Array(items) => {
            items.iter_mut().all(|child| inline_blobs(child, blob_dir))
        }
        _ => true,
    }
}

/// Pick up where a daemon that died on this socket left off: its
/// document's log home and the unsaved tail the log holds, numbered from
/// the start of this daemon's order.
fn resume(state: &mut State, note: &Path) {
    let Some(document) = noted_home(note) else {
        return;
    };
    if !document.is_file() {
        return;
    }
    let tail = restore_tail(&document_oplog(&document), &document).unwrap_or_default();
    for batch in tail {
        state.seq += 1;
        state.tail.push(Logged {
            seq: state.seq,
            id: batch.id,
            actor: batch.actor,
            client: None,
            batch: 0,
            ops: batch.ops,
        });
    }
    set_oplog_home(&document);
    mark_unsaved(&state.unsaved(), Some(&document));
    tracing::info!(
        document = %document.display(),
        unsaved = state.tail.len(),
        "resumed a document whose daemon stopped"
    );
}

/// Run the daemon on `socket` with the default queue limit.
pub fn run(socket: &Path) -> std::io::Result<()> {
    run_with(socket, DEFAULT_QUEUE_LIMIT)
}

/// Run the daemon on `socket`: accept clients, serve each on its own
/// thread, exit when the last one leaves. A client with more than
/// `queue_limit` messages unwritten is disconnected.
pub fn run_with(socket: &Path, queue_limit: usize) -> std::io::Result<()> {
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    claim_unhomed_oplog(socket);
    let note = socket.with_extension("home");
    let mut state = State::default();
    resume(&mut state, &note);
    *lock(&HOME_NOTE) = Some(note.clone());
    // A stale socket from a dead daemon would make bind fail forever.
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    tracing::info!(socket = %socket.display(), "printcad-serverd listening");

    let hub = Arc::new(Hub {
        state: Mutex::new(state),
        file: Mutex::new(()),
        queue_limit,
    });
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
    // Everyone has gone, and the unsaved edits with them: the next daemon
    // on this document starts from its file alone.
    if lock(&OPLOG_PATH).is_some() {
        mark_unsaved(&[], None);
    }
    let _ = std::fs::remove_file(&note);
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

    /// The tail read back is what the last mark leaves unsaved plus what
    /// follows it; lines from a daemon that marks nothing are never taken
    /// for unsaved, and past the mark they make the tail unknown.
    #[test]
    fn the_tail_read_back_is_what_the_last_mark_leaves_unsaved() {
        let dir = std::env::temp_dir().join(format!("printcad_tail_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let document = dir.join("model.prtcad");
        std::fs::write(&document, b"saved").expect("file");
        let log = document_oplog(&document);

        let mut doc = core_document::Document::new("Log");
        let mut op = |name: &str| {
            doc.set_name(name);
            serde_json::to_value(&doc.take_pending_ops()[0]).expect("op")
        };
        let actor = uuid::Uuid::new_v4();
        let [saved, kept, after] = [0, 1, 2].map(|_| uuid::Uuid::new_v4());
        let line = |batch: Option<uuid::Uuid>, op: serde_json::Value| {
            let mut line = serde_json::json!({"v": 1, "actor": actor, "op": op});
            if let Some(batch) = batch {
                line["batch"] = serde_json::json!(batch);
            }
            line.to_string()
        };
        let mark = serde_json::json!({
            "v": 1,
            "unsaved": [kept],
            "file": file_stamp(&document),
        });
        let lines = [
            line(None, op("from a daemon that marks nothing")),
            line(Some(saved), op("saved")),
            line(Some(kept), op("kept")),
            mark.to_string(),
            line(Some(after), op("after")),
        ];
        std::fs::write(&log, lines.join("\n") + "\n").expect("log");
        let names = |tail: Vec<Restored>| -> Vec<(uuid::Uuid, String)> {
            tail.into_iter()
                .map(|batch| match &batch.ops[0] {
                    DocumentOp::SetDocumentName { name } => (batch.id, name.clone()),
                    other => panic!("unexpected {other:?}"),
                })
                .collect()
        };
        let tail = restore_tail(&log, &document).expect("a tail");
        assert_eq!(
            names(tail),
            [(kept, "kept".to_string()), (after, "after".to_string())]
        );

        let unmarked = line(None, op("unmarked, after the mark"));
        std::fs::write(&log, lines.join("\n") + "\n" + &unmarked + "\n").expect("log");
        assert!(restore_tail(&log, &document).is_none());

        std::fs::write(&log, lines[3..].join("\n") + "\n").expect("log");
        assert!(
            restore_tail(&log, &document).is_none(),
            "a batch the mark names rotated out"
        );

        std::fs::write(&log, lines.join("\n") + "\n").expect("log");
        std::fs::write(&document, b"saved elsewhere").expect("file");
        assert!(restore_tail(&log, &document).is_none(), "the file changed");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
