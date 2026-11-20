//! The daemon: one document, one socket, N clients, no `Document` parsing.
//!
//! Responsibilities: own the document's file (reads on `OpenDocument`,
//! atomic writes on `SaveDocument`), append every op batch to the
//! document's log (`crate::oplog_path_for`: one JSON envelope per line, in
//! the application's data folder), and put the clients'
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
//! naming the batches still unsaved. When the last client leaves, it names
//! none if that client said it was done (`Leave`: what it held unsaved it
//! let go); a client that went without a word (it crashed) leaves the tail
//! named. The tail a log holds when a daemon takes up its document (one
//! that died left `<socket>.home` naming it, or a client opens it) is not
//! put back by itself: it waits as the recoverable edits, which an open
//! says it has, and comes back into the order on `RestoreUnsaved` (a
//! client rejoining sends that first) or goes on `DiscardUnsaved`.

use local_ipc::{Listener as UnixListener, Stream as UnixStream};
use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
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
    /// A client has said Hello. Until one has, a connection that goes
    /// (another daemon or the start page asking whether this socket is
    /// served) does not end the daemon its client is still to reach.
    joined: AtomicBool,
}

#[derive(Default)]
struct State {
    /// The last sequence number given to a batch.
    seq: u64,
    /// Batches since the last save, in order: the unsaved present a
    /// joining client gets after the file.
    tail: Vec<Logged>,
    clients: HashMap<u64, Client>,
    /// The batches already ordered, by id, with their sequence number
    /// (`None` for one a daemon before this one ordered, read from the
    /// log), so a batch sent again is answered rather than ordered twice.
    ordered: HashMap<uuid::Uuid, Option<u64>>,
    /// `ordered`'s ids, oldest first, so it forgets the oldest past
    /// [`REMEMBERED_BATCHES`].
    ordered_ids: std::collections::VecDeque<uuid::Uuid>,
    /// This daemon started where another stopped and could not read back
    /// the unsaved edits that one held; a save or a rebase ends it.
    unsaved_lost: bool,
    /// The unsaved edits the log held when this daemon took up its
    /// document, which no client has asked back yet. Marks keep naming
    /// them until one is asked for them (`RestoreUnsaved`), lets them go
    /// (`DiscardUnsaved`) or saves.
    recoverable: Vec<Logged>,
    /// The last client to leave said it was done with the document.
    last_left_cleanly: bool,
    /// The socket whose daemon's log the next save that writes takes up
    /// (`AdoptLog`).
    adopt: Option<PathBuf>,
}

/// How many batch ids a daemon remembers to know a batch sent again. A
/// client sends again only what was unanswered when its connection went,
/// so the newest few thousand are what matters.
const REMEMBERED_BATCHES: usize = 1 << 16;

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
    /// It said it was done with the document (`Leave`).
    done: bool,
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

    /// Queue `message` for every client.
    fn tell_all(&mut self, limit: usize, message: impl Fn() -> ServerMessage) {
        self.tell_others(u64::MAX, limit, message);
    }

    /// Let `id` go: out of the document's clients, its peers told.
    fn leave(&mut self, id: u64, limit: usize) {
        if let Some(client) = self.clients.remove(&id) {
            self.last_left_cleanly = client.done;
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

    /// Remember that batch `id` was ordered, at `seq` when this daemon
    /// ordered it.
    fn remember(&mut self, id: uuid::Uuid, seq: Option<u64>) {
        if self.ordered.insert(id, seq).is_none() {
            self.ordered_ids.push_back(id);
        }
        while self.ordered_ids.len() > REMEMBERED_BATCHES {
            if let Some(old) = self.ordered_ids.pop_front() {
                self.ordered.remove(&old);
            }
        }
    }

    /// The ids of the batches still unsaved, for the log's mark: the
    /// recoverable ones, then the tail. An empty batch has no lines in the
    /// log and changes nothing, so it is left out.
    fn unsaved(&self) -> Vec<uuid::Uuid> {
        self.recoverable
            .iter()
            .chain(&self.tail)
            .filter(|logged| !logged.ops.is_empty())
            .map(|logged| logged.id)
            .collect()
    }

    /// Bring batches read back from a log into the order, after everything
    /// so far, and send them to every client.
    fn restore(&mut self, batches: Vec<Logged>, limit: usize) {
        for mut logged in batches {
            self.seq += 1;
            logged.seq = self.seq;
            self.remember(logged.id, Some(logged.seq));
            let (actor, seq) = (logged.actor, logged.seq);
            self.tell_all(limit, || ServerMessage::Ops {
                actor,
                seq,
                ops: logged.ops.clone(),
            });
            self.tail.push(logged);
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
    hub.joined.store(true, Ordering::Release);

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
                done: false,
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
            ClientMessage::Ops { batch, ops } => order_batch(hub, id, actor, batch, ops),
            ClientMessage::Presence(state) => {
                // Now-state, not history: relayed, never logged.
                lock(&hub.state).tell_others(id, limit, || ServerMessage::PresencePeer {
                    actor,
                    state: state.clone(),
                });
            }
            ClientMessage::Leave => {
                if let Some(client) = lock(&hub.state).clients.get_mut(&id) {
                    client.done = true;
                }
            }
            ClientMessage::RestoreUnsaved => {
                let mut state = lock(&hub.state);
                let recoverable = std::mem::take(&mut state.recoverable);
                state.restore(recoverable, limit);
            }
            ClientMessage::AdoptLog { socket } => {
                let mut state = lock(&hub.state);
                if state.adopt.as_ref() != Some(&socket) {
                    note_adopt(&socket);
                    state.adopt = Some(socket);
                }
            }
            ClientMessage::RestoreLog { socket } => restore_log(hub, &socket),
            ClientMessage::DiscardUnsaved => {
                let mut state = lock(&hub.state);
                if !state.recoverable.is_empty() {
                    state.recoverable.clear();
                    mark_unsaved(&state.unsaved(), home_document().as_deref());
                }
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
                    state.recoverable.clear();
                    state.unsaved_lost = false;
                }
            }
            ClientMessage::SaveDocument {
                path,
                bytes,
                at_seq,
                seen_seq,
                sent_batches,
            } => {
                // The daemon whose log this save takes up writes until its
                // client's last frame is read: it is let finish first.
                let adopt = lock(&hub.state).adopt.clone();
                if let Some(socket) = &adopt {
                    wait_until_unserved(socket);
                }
                let _file = lock(&hub.file);
                let written = write_atomically(&path, &bytes);
                let mut state = lock(&hub.state);
                let reply = match written {
                    Ok(()) => {
                        if adopt.is_some() && state.adopt == adopt {
                            state.adopt = None;
                        }
                        let earlier = adopt.as_deref().map(crate::unhomed_oplog_for);
                        set_oplog_home(&path, earlier.as_deref());
                        // The file now holds what this client chose; edits
                        // left from before describe the file it replaced.
                        state.recoverable.clear();
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
                        state.unsaved_lost = false;
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
                let mut state = lock(&hub.state);
                match read {
                    Ok(bytes) => {
                        if set_oplog_home(&path, None) {
                            take_up_unsaved(&mut state, &path);
                        }
                        // The file is the saved past; the tail is the
                        // unsaved present. Queued together under the lock
                        // that orders batches: every batch heard before
                        // `Opened` is in one of them, every one after is
                        // newer than both.
                        state.reply(
                            id,
                            ServerMessage::Opened {
                                token,
                                path,
                                bytes,
                                seq: state.seq,
                                unsaved_lost: state.unsaved_lost,
                                recoverable: state.recoverable.len() as u32,
                            },
                        );
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
/// A batch already ordered (sent again by a client whose connection went
/// before its answer came) is only answered, with its place when this
/// daemon ordered it.
fn order_batch(hub: &Hub, id: u64, actor: uuid::Uuid, batch_id: uuid::Uuid, ops: Vec<DocumentOp>) {
    let mut state = lock(&hub.state);
    let limit = hub.queue_limit;
    let batch = match state.clients.get_mut(&id) {
        Some(client) => {
            client.batches += 1;
            client.batches - 1
        }
        None => return,
    };
    if let Some(&known) = state.ordered.get(&batch_id) {
        let seq = known.unwrap_or(state.seq);
        state.tell(id, limit, ServerMessage::Acked { seq });
        return;
    }
    state.seq += 1;
    let seq = state.seq;
    state.remember(batch_id, Some(seq));
    if let Err(err) = append_ops(actor, batch_id, &ops) {
        tracing::error!("op log append failed: {err}");
    }
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
    let log = crate::unhomed_oplog_for(socket);
    if crate::is_untitled_socket(socket) {
        note_socket(&log, socket);
    }
    *lock(&UNHOMED_PATH) = Some(log);
}

/// Note beside the unhomed `log` the socket its daemon serves
/// (`<log>.socket`): what finds the log of an untitled session a crash
/// left, and tells whether a daemon still serves it.
fn note_socket(log: &Path, socket: &Path) {
    let note = log.with_extension("socket");
    let noted = std::fs::create_dir_all(crate::runtime_dir_for_logs())
        .map_err(|err| err.to_string())
        .and_then(|()| serde_json::to_vec(socket).map_err(|err| err.to_string()))
        .and_then(|text| write_atomically(&note, &text).map_err(|err| err.to_string()));
    if let Err(err) = noted {
        tracing::warn!("the untitled log's socket was not noted: {err}");
    }
}

fn unhomed_oplog() -> PathBuf {
    let claimed = lock(&UNHOMED_PATH).clone();
    claimed.unwrap_or_else(|| crate::runtime_dir_for_logs().join("unhomed.oplog.jsonl"))
}

fn document_oplog(document: &Path) -> PathBuf {
    crate::oplog_path_for(document)
}

/// Where a document's log was kept before logs had a folder of their own:
/// beside the document.
fn legacy_oplog(document: &Path) -> PathBuf {
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

/// The document this daemon's log is homed for.
static HOME_DOCUMENT: Mutex<Option<PathBuf>> = Mutex::new(None);

fn home_document() -> Option<PathBuf> {
    lock(&HOME_DOCUMENT).clone()
}

/// Home the op log for `document`; true when it moved there now. Taken
/// under the log's write lock, so no append lands in the unhomed log
/// after its lines were carried over. A log still beside the document
/// moves into place first, once. `earlier`, another daemon's unhomed log
/// (an untitled tab's, saved for the first time), joins the log ahead of
/// this daemon's own unhomed lines, or at its end when it had its home.
fn set_oplog_home(document: &Path, earlier: Option<&Path>) -> bool {
    let _guard = lock(&OPLOG_WRITE);
    let home = document_oplog(document);
    let mut slot = lock(&OPLOG_PATH);
    if slot.as_deref() == Some(home.as_path()) {
        if let Some(earlier) = earlier {
            carry_log(earlier, &home);
            prune_blobs(&home);
        }
        return false;
    }
    remember_home(document);
    *lock(&HOME_DOCUMENT) = Some(document.to_path_buf());
    if let Some(dir) = home.parent()
        && let Err(err) = std::fs::create_dir_all(dir)
    {
        tracing::warn!("the op log's folder {} was not made: {err}", dir.display());
    }
    adopt_legacy_log(&legacy_oplog(document), &home);
    match serde_json::to_vec(document) {
        Ok(text) => {
            if let Err(err) = write_atomically(&home.with_extension("document"), &text) {
                tracing::warn!("the op log's document was not noted: {err}");
            }
        }
        Err(err) => tracing::warn!("the op log's document cannot be noted: {err}"),
    }
    // Ops recorded before the document had a file live in the unhomed
    // logs; carry them over, so the document's history starts at its
    // beginning, not at its first save.
    for log in earlier
        .into_iter()
        .map(Path::to_path_buf)
        .chain([unhomed_oplog()])
    {
        carry_log(&log, &home);
    }
    prune_blobs(&home);
    *slot = Some(home);
    true
}

/// Append the unhomed log `from`, oldest generation first, to `into`, and
/// move the blobs its lines name into `into`'s store. Taken under the
/// log's write lock. Once carried, `from` and its note are gone rather than
/// emptied: what is left behind would be one more thing to explain.
fn carry_log(from: &Path, into: &Path) {
    let mut sources = generations(from);
    sources.push(from.to_path_buf());
    let mut carried = Ok(());
    for source in &sources {
        let Ok(text) = std::fs::read(source) else {
            continue;
        };
        carried = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(into)
            .and_then(|mut file| file.write_all(&text));
        if carried.is_err() {
            break;
        }
    }
    if let Err(err) = carried {
        tracing::warn!("unhomed op log migration failed: {err}");
        return;
    }
    for source in &sources {
        let _ = std::fs::remove_file(source);
    }
    let _ = std::fs::remove_file(from.with_extension("socket"));
    let from_blobs = from.with_extension("blobs");
    let into_blobs = into.with_extension("blobs");
    if let Ok(entries) = std::fs::read_dir(&from_blobs) {
        let moved = std::fs::create_dir_all(&into_blobs).and_then(|()| {
            entries.flatten().try_for_each(|entry| {
                std::fs::rename(entry.path(), into_blobs.join(entry.file_name()))
            })
        });
        match moved {
            Ok(()) => {
                let _ = std::fs::remove_dir(&from_blobs);
            }
            Err(err) => tracing::warn!(
                "the op log's blobs did not all move to {}: {err}",
                into_blobs.display()
            ),
        }
    }
}

/// Move a log kept beside its document, with its generations and blobs,
/// to `home`, unless a log is there already.
fn adopt_legacy_log(legacy: &Path, home: &Path) {
    if std::fs::symlink_metadata(legacy).is_err() || std::fs::symlink_metadata(home).is_ok() {
        return;
    }
    let mut moves: Vec<(PathBuf, PathBuf)> = generations(legacy)
        .into_iter()
        .filter_map(|old| {
            let number = old.extension()?.to_str()?.to_owned();
            Some((old, home.with_extension(format!("jsonl.{number}"))))
        })
        .collect();
    moves.push((legacy.to_path_buf(), home.to_path_buf()));
    let blobs = legacy.with_extension("blobs");
    if let Ok(entries) = std::fs::read_dir(&blobs) {
        let home_blobs = home.with_extension("blobs");
        if let Err(err) = std::fs::create_dir_all(&home_blobs) {
            tracing::warn!("the op log's blobs have nowhere to go: {err}");
            return;
        }
        moves.extend(
            entries
                .flatten()
                .map(|entry| (entry.path(), home_blobs.join(entry.file_name()))),
        );
    }
    for (from, to) in moves {
        // Across file systems a move is a copy and a removal.
        let moved = std::fs::rename(&from, &to)
            .or_else(|_| std::fs::copy(&from, &to).and_then(|_| std::fs::remove_file(&from)));
        if let Err(err) = moved {
            tracing::warn!("{} did not move to {}: {err}", from.display(), to.display());
        }
    }
    let _ = std::fs::remove_dir(&blobs);
    tracing::info!(
        from = %legacy.display(),
        to = %home.display(),
        "moved a document's op log into the op log folder"
    );
}

/// The log's rotated generations, oldest first: `<log>.1`, `<log>.2`, ...
fn generations(log: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (log.parent(), log.file_name()) else {
        return Vec::new();
    };
    let prefix = format!("{}.", name.to_string_lossy());
    let mut found: Vec<(u64, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name();
            let number = file.to_str()?.strip_prefix(&prefix)?.parse().ok()?;
            Some((number, entry.path()))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, path)| path).collect()
}

/// How many rotated generations a log keeps at most: past this the oldest
/// go, and with them any unsaved edits they held.
const MAX_GENERATIONS: usize = 16;

/// Start a new generation: the live log takes the next number.
fn rotate(log: &Path) {
    let older = generations(log);
    let next = older
        .last()
        .and_then(|path| path.extension()?.to_str()?.parse::<u64>().ok())
        .map_or(1, |n| n + 1);
    if let Err(err) = std::fs::rename(log, log.with_extension(format!("jsonl.{next}"))) {
        tracing::warn!("the op log did not rotate, so it grows past its cap: {err}");
        return;
    }
    let over = (older.len() + 1).saturating_sub(MAX_GENERATIONS);
    if over > 0 {
        tracing::warn!(
            "the op log passed {MAX_GENERATIONS} generations; the oldest go, and a restart \
             may not restore every unsaved edit"
        );
    }
    for stale in older.iter().take(over) {
        let _ = std::fs::remove_file(stale);
    }
}

/// Take away every rotated generation: nothing before the live log is
/// needed to restore the unsaved edits. True when there were some.
fn drop_generations(log: &Path) -> bool {
    let older = generations(log);
    for generation in &older {
        let _ = std::fs::remove_file(generation);
    }
    !older.is_empty()
}

/// Remove the blobs in `log`'s store that no line of the log or its kept
/// generations names. Run under the log's write lock, so no append is
/// storing a blob its line has not named yet.
fn prune_blobs(log: &Path) {
    use std::io::BufRead as _;
    let store = log.with_extension("blobs");
    let Ok(entries) = std::fs::read_dir(&store) else {
        return;
    };
    let mut named = std::collections::HashSet::new();
    let mut sources = generations(log);
    sources.push(log.to_path_buf());
    for source in sources {
        let Ok(file) = std::fs::File::open(&source) else {
            continue;
        };
        let mut reader = std::io::BufReader::new(file);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) => named.extend(blob_names(&line)),
                Err(err) => {
                    tracing::warn!("the op log was not read whole; its blobs stay: {err}");
                    return;
                }
            }
        }
    }
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_str().is_some_and(|name| named.contains(name)) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The hashes of the `blob:sha256:<hex>` markers in a log line.
fn blob_names(line: &[u8]) -> Vec<String> {
    const MARKER: &[u8] = b"blob:sha256:";
    let mut names = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.windows(MARKER.len()).position(|w| w == MARKER) {
        rest = &rest[at + MARKER.len()..];
        let hex = rest.iter().take_while(|b| b.is_ascii_hexdigit()).count();
        names.push(String::from_utf8_lossy(&rest[..hex]).into_owned());
        rest = &rest[hex..];
    }
    names
}

/// Remove this daemon's unhomed log, its generations and its blobs: its
/// document never got a file and its last client let its edits go.
fn remove_unhomed_log() {
    let _guard = lock(&OPLOG_WRITE);
    remove_log_files(&unhomed_oplog());
}

/// Remove `log`, its generations, its blobs and the note of its socket.
fn remove_log_files(log: &Path) {
    drop_generations(log);
    let _ = std::fs::remove_file(log);
    let _ = std::fs::remove_dir_all(log.with_extension("blobs"));
    let _ = std::fs::remove_file(log.with_extension("socket"));
}

fn oplog_path() -> PathBuf {
    let slot = lock(&OPLOG_PATH);
    slot.clone().unwrap_or_else(unhomed_oplog)
}

/// Payload strings at or above this length are pulled out of the log into
/// the content-addressed blob store. Well under any real import, well over
/// any parameter payload.
const BLOB_EXTRACT_THRESHOLD: usize = 256 * 1024;

/// The log rotates once it passes this size, unless the daemon is started
/// with another `--log-bytes`.
pub const DEFAULT_LOG_BYTES: u64 = 64 * 1024 * 1024;

/// The size this daemon's log rotates at.
static LOG_BYTES: AtomicU64 = AtomicU64::new(DEFAULT_LOG_BYTES);

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
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_BYTES.load(Ordering::Relaxed)) {
        rotate(&path);
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

/// Empty the log for a document put in place of the one it described. A
/// log with no home starts again with a line saying so (`rebased`): its
/// edits apply to that document, not to an empty one, so they cannot
/// bring an untitled session back alone.
fn truncate_oplog() -> std::io::Result<()> {
    let _guard = lock(&OPLOG_WRITE);
    let homed = lock(&OPLOG_PATH).is_some();
    let path = oplog_path();
    drop_generations(&path);
    if !homed {
        let line = serde_json::json!({
            "v": core_document::op::OP_PROTOCOL_VERSION,
            "rebased": true,
        });
        std::fs::write(&path, format!("{line}\n"))?;
    } else if path.exists() {
        std::fs::write(&path, b"")?;
    }
    prune_blobs(&path);
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
/// stamp. A mark naming none leaves the rotated generations nothing to
/// restore, so they go.
fn mark_unsaved(unsaved: &[uuid::Uuid], document: Option<&Path>) {
    let _guard = lock(&OPLOG_WRITE);
    append_mark(&oplog_path(), unsaved, document);
}

/// [`mark_unsaved`] on the log at `path`.
fn append_mark(path: &Path, unsaved: &[uuid::Uuid], document: Option<&Path>) {
    let mark = serde_json::json!({
        "v": core_document::op::OP_PROTOCOL_VERSION,
        "unsaved": unsaved,
        "file": document.and_then(file_stamp),
    });
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| writeln!(file, "{mark}"));
    match written {
        Ok(()) if unsaved.is_empty() => {
            if drop_generations(path) {
                prune_blobs(path);
            }
        }
        Ok(()) => {}
        Err(err) => tracing::error!("the op log's unsaved mark was not written: {err}"),
    }
}

/// A document whose log ends with edits a session left unsaved, while no
/// daemon serves it: what the start page offers back.
#[derive(Debug, Clone, PartialEq)]
pub struct UnsavedLog {
    pub document: PathBuf,
    /// When the log was last written, in milliseconds since the Unix epoch.
    pub modified_ms: u64,
}

/// The documents whose logs in [`crate::oplog_dir`] end with unsaved edits.
pub fn unsaved_logs() -> Vec<UnsavedLog> {
    unsaved_logs_in(&crate::oplog_dir())
}

/// [`unsaved_logs`] in the log folder `dir`: each log is found by the note
/// naming its document (`<key>.oplog.document`), and what its last line
/// says (an edit, or a mark naming some unsaved) is enough. Opening the
/// document reads the log whole and says what can come back.
pub fn unsaved_logs_in(dir: &Path) -> Vec<UnsavedLog> {
    let mut found: Vec<UnsavedLog> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|note| note.to_string_lossy().ends_with(".oplog.document"))
        .filter_map(|note| {
            let document: PathBuf = serde_json::from_slice(&std::fs::read(&note).ok()?).ok()?;
            let log = note.with_extension("jsonl");
            let modified = std::fs::metadata(&log).ok()?.modified().ok()?;
            (document.is_file() && log_ends_unsaved(&log) && !served(&document)).then(|| {
                UnsavedLog {
                    document,
                    modified_ms: modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_millis() as u64),
                }
            })
        })
        .collect();
    found.sort_by_key(|log| std::cmp::Reverse(log.modified_ms));
    found
}

/// Whether the log's last edit or mark leaves edits unsaved: an edit, or
/// a mark naming some. Only the end of the log is read.
fn log_ends_unsaved(log: &Path) -> bool {
    use std::io::{Read as _, Seek as _};
    let Ok(mut file) = std::fs::File::open(log) else {
        return false;
    };
    let len = file.metadata().map_or(0, |m| m.len());
    let mut end = Vec::new();
    if file
        .seek(std::io::SeekFrom::Start(len.saturating_sub(1 << 20)))
        .and_then(|_| file.read_to_end(&mut end))
        .is_err()
    {
        return false;
    }
    let text = String::from_utf8_lossy(&end);
    for last in text.lines().rev().filter(|line| !line.trim().is_empty()) {
        match serde_json::from_str::<serde_json::Value>(last) {
            Ok(line) if line.get("unsaved").is_some() => {
                return line["unsaved"]
                    .as_array()
                    .is_some_and(|ids| !ids.is_empty());
            }
            Ok(line) if line.get("op").is_some() => return true,
            // A note (the untitled log a save was to take up, a rebase)
            // says nothing of what is unsaved: the line before it does.
            Ok(_) => {}
            // A line longer than what was read: an edit's.
            Err(_) => return true,
        }
    }
    false
}

/// Whether a daemon serves `document` now.
fn served(document: &Path) -> bool {
    listening(&crate::socket_path_for(document))
}

/// Whether a daemon listens on `socket` now.
fn listening(socket: &Path) -> bool {
    UnixStream::connect(socket).is_ok()
}

/// How long a save waits for the daemon whose log it takes up to finish.
const ADOPT_PATIENCE: std::time::Duration = std::time::Duration::from_secs(5);

/// Wait, at most [`ADOPT_PATIENCE`], until no daemon listens on `socket`.
fn wait_until_unserved(socket: &Path) {
    let deadline = std::time::Instant::now() + ADOPT_PATIENCE;
    while listening(socket) {
        if std::time::Instant::now() >= deadline {
            tracing::warn!(
                socket = %socket.display(),
                "the daemon whose log this save takes up still runs; its last edits may stay behind"
            );
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// The log an untitled tab's daemon kept, left by a session that ended
/// without saving or letting it go (a crash): what the start page offers
/// back as a new untitled document.
#[derive(Debug, Clone, PartialEq)]
pub struct UntitledLog {
    /// The socket the tab's daemon served, which names the log.
    pub socket: PathBuf,
    /// When the log was last written, in milliseconds since the Unix epoch.
    pub modified_ms: u64,
}

/// The untitled sessions' logs in the runtime folder that hold edits an
/// empty document takes, while no daemon serves them.
pub fn untitled_logs() -> Vec<UntitledLog> {
    let mut found: Vec<UntitledLog> = std::fs::read_dir(crate::runtime_dir_for_logs())
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|note| note.to_string_lossy().ends_with(".oplog.socket"))
        .filter_map(|note| {
            let socket: PathBuf = serde_json::from_slice(&std::fs::read(&note).ok()?).ok()?;
            let log = crate::unhomed_oplog_for(&socket);
            let modified = std::fs::metadata(&log).ok()?.modified().ok()?;
            (log_ends_unsaved(&log) && !log_starts_rebased(&log) && !listening(&socket)).then(
                || UntitledLog {
                    socket,
                    modified_ms: modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_millis() as u64),
                },
            )
        })
        .collect();
    found.sort_by_key(|log| std::cmp::Reverse(log.modified_ms));
    found
}

/// Whether `log` begins at a document put in place (a `rebased` line), as
/// its oldest generation's first line says.
fn log_starts_rebased(log: &Path) -> bool {
    use std::io::BufRead as _;
    let oldest = generations(log)
        .into_iter()
        .next()
        .unwrap_or_else(|| log.to_path_buf());
    let Ok(file) = std::fs::File::open(oldest) else {
        return false;
    };
    let mut first = String::new();
    let _ = std::io::BufReader::new(file).read_line(&mut first);
    serde_json::from_str::<serde_json::Value>(&first)
        .is_ok_and(|line| line.get("rebased").is_some())
}

/// Let go of the log an untitled tab's daemon on `socket` kept, while no
/// daemon serves it: the start page's Discard, or a tab whose first save's
/// server never took it up closing.
pub fn discard_untitled_log(socket: &Path) {
    if !listening(socket) {
        remove_log_files(&crate::unhomed_oplog_for(socket));
    }
}

/// `RestoreLog`: the log the untitled tab's daemon on `socket` kept joins
/// this daemon's, and its edits come back into the order. A log a daemon
/// still writes, one that begins at a document put in place, or one that
/// does not read whole is left where it is.
fn restore_log(hub: &Hub, socket: &Path) {
    if listening(socket) {
        tracing::warn!(socket = %socket.display(), "an untitled log still in use is not restored");
        return;
    }
    let log = crate::unhomed_oplog_for(socket);
    let lines = read_log(&log);
    if lines.iter().any(|line| line.get("rebased").is_some()) {
        tracing::warn!("an untitled log that begins at a document put in place is not restored");
        return;
    }
    let Some(tail) = restore_tail_from(&lines, &log, None) else {
        return;
    };
    let mut state = lock(&hub.state);
    {
        let _guard = lock(&OPLOG_WRITE);
        let into = oplog_path();
        carry_log(&log, &into);
        prune_blobs(&into);
    }
    tracing::info!(batches = tail.len(), "restored an untitled session's log");
    state.restore(recoverable(tail), hub.queue_limit);
}

/// Let go of the unsaved edits `document`'s log holds, while no daemon
/// serves it: the start page's Discard.
pub fn discard_unsaved_log(document: &Path) {
    let log = document_oplog(document);
    if served(document) || std::fs::symlink_metadata(&log).is_err() {
        return;
    }
    append_mark(&log, &[], Some(document));
}

/// A batch read back from the log.
struct Restored {
    id: uuid::Uuid,
    actor: uuid::Uuid,
    ops: Vec<DocumentOp>,
}

/// Every line of `log` that reads, its generations first, so a tail the
/// log rotated over is still found.
fn read_log(log: &Path) -> Vec<serde_json::Value> {
    let mut lines = Vec::new();
    let mut sources = generations(log);
    sources.push(log.to_path_buf());
    for generation in sources {
        if let Ok(text) = std::fs::read_to_string(&generation) {
            lines.extend(
                text.lines()
                    .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok()),
            );
        }
    }
    lines
}

/// The ids of the batches the log's lines hold, oldest first.
fn logged_batches(lines: &[serde_json::Value]) -> Vec<uuid::Uuid> {
    let mut ids: Vec<uuid::Uuid> = lines
        .iter()
        .filter_map(|line| line["batch"].as_str()?.parse().ok())
        .collect();
    ids.dedup();
    ids
}

/// The batches the log's `lines` hold that `document` does not: those its
/// last mark names, then every batch after it (every batch, before any
/// mark). None when the log cannot say: a line no batch id names where
/// the tail would be (a daemon that marks nothing wrote it, so a save may
/// have taken it), a file changed since the mark, a batch it names that
/// the log no longer holds (its generation dropped past the cap), a blob
/// or an op that does not read. A batch whose lines come again later
/// (sent once more to a daemon that did not know it had been logged) is
/// taken once, where it first stands.
fn restore_tail_from(
    lines: &[serde_json::Value],
    log: &Path,
    document: Option<&Path>,
) -> Option<Vec<Restored>> {
    let mark = lines.iter().rposition(|line| line.get("unsaved").is_some());
    let (listed, after) = match mark {
        Some(at) => {
            let mark = &lines[at];
            if !mark["file"].is_null()
                && Some(&mark["file"]) != document.and_then(file_stamp).as_ref()
            {
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
    let mut taken = std::collections::HashSet::new();
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
        let current = tail.last().map(|last| last.id);
        if current != Some(batch) && !taken.insert(batch) {
            continue;
        }
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
fn resume(state: &mut State, socket: &Path, note: &Path) {
    let Some(document) = noted_home(note).filter(|document| document.is_file()) else {
        // A daemon on a document with no file yet writes on where the one
        // before it stopped; what it ordered is in the log already.
        let log = oplog_path();
        let lines = read_log(&log);
        for id in logged_batches(&lines) {
            state.remember(id, None);
        }
        set_aside_leftover(socket, &log, &lines);
        return;
    };
    let log = document_oplog(&document);
    let lines = read_log(&log);
    for id in logged_batches(&lines) {
        state.remember(id, None);
    }
    let tail = restore_tail_from(&lines, &log, Some(&document)).unwrap_or_else(|| {
        state.unsaved_lost = true;
        Vec::new()
    });
    state.recoverable = recoverable(tail);
    set_oplog_home(&document, None);
    mark_unsaved(&state.unsaved(), Some(&document));
    tracing::info!(
        document = %document.display(),
        unsaved = state.recoverable.len(),
        "took up a document whose daemon stopped"
    );
}

/// Note in the unhomed log the untitled log a save is to take up
/// (`AdoptLog`): what tells the next daemon on this socket, should this
/// one die before the save writes, whose edits its lines are.
fn note_adopt(socket: &Path) {
    let _guard = lock(&OPLOG_WRITE);
    if lock(&OPLOG_PATH).is_some() {
        return;
    }
    let line = serde_json::json!({
        "v": core_document::op::OP_PROTOCOL_VERSION,
        "adopt": socket,
    });
    let path = unhomed_oplog();
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
        })
        .and_then(|mut file| writeln!(file, "{line}"));
    if let Err(err) = written {
        tracing::warn!("the untitled log a save takes up was not noted: {err}");
    }
}

/// The edits a file's daemon that died before its first save or open
/// left in this socket's unhomed `log` (its `lines`) are the session's
/// that sent them, never the document this daemon opens or saves next,
/// which holds none of them. An untitled tab's first save noted the
/// untitled log it was to take up: when that log is still kept and no
/// daemon serves it, these edits join it, where the start page offers
/// them back with the rest of that session. Otherwise a mark naming none
/// closes them off, so a later open does not offer them. An untitled
/// socket's log is left alone: it is that session's whole history.
fn set_aside_leftover(socket: &Path, log: &Path, lines: &[serde_json::Value]) {
    if crate::is_untitled_socket(socket) {
        return;
    }
    let mark = lines.iter().rposition(|line| line.get("unsaved").is_some());
    let left = &lines[mark.map_or(0, |at| at + 1)..];
    if !left.iter().any(|line| line.get("op").is_some()) {
        return;
    }
    let untitled = left
        .iter()
        .rev()
        .find_map(|line| serde_json::from_value::<PathBuf>(line.get("adopt")?.clone()).ok())
        .filter(|untitled| !listening(untitled))
        .map(|untitled| crate::unhomed_oplog_for(&untitled))
        .filter(|into| std::fs::symlink_metadata(into.with_extension("socket")).is_ok());
    let _guard = lock(&OPLOG_WRITE);
    match untitled {
        Some(into) if mark.is_none() => {
            carry_log(log, &into);
            tracing::info!(
                into = %into.display(),
                "edits left by a daemon that died before its first save joined their untitled log"
            );
        }
        _ => {
            append_mark(log, &[], None);
            tracing::info!("edits left by a daemon that died before its first save are set aside");
        }
    }
}

/// Batches read back from the log, as edits waiting to be asked back.
fn recoverable(tail: Vec<Restored>) -> Vec<Logged> {
    tail.into_iter()
        .map(|batch| Logged {
            seq: 0,
            id: batch.id,
            actor: batch.actor,
            client: None,
            batch: 0,
            ops: batch.ops,
        })
        .collect()
}

/// The log just got its home on an open: the unsaved edits it holds from a
/// session that ended without saving them wait to be asked back, and the
/// log is marked.
fn take_up_unsaved(state: &mut State, document: &Path) {
    let log = document_oplog(document);
    let lines = read_log(&log);
    for id in logged_batches(&lines) {
        state.remember(id, None);
    }
    let known: std::collections::HashSet<uuid::Uuid> = state.unsaved().into_iter().collect();
    let tail = restore_tail_from(&lines, &log, Some(document)).unwrap_or_default();
    state.recoverable = recoverable(
        tail.into_iter()
            .filter(|batch| !known.contains(&batch.id))
            .collect(),
    );
    if !state.recoverable.is_empty() {
        tracing::info!(
            document = %document.display(),
            unsaved = state.recoverable.len(),
            "the document's log holds edits a session left unsaved"
        );
    }
    mark_unsaved(&state.unsaved(), Some(document));
}

/// What a daemon holds to: how far a client may fall behind and how
/// large its op log grows before it rotates.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Messages a client may have queued and unwritten before it is
    /// disconnected.
    pub queue: usize,
    /// Bytes the op log holds before it rotates to a new generation.
    pub log_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            queue: DEFAULT_QUEUE_LIMIT,
            log_bytes: DEFAULT_LOG_BYTES,
        }
    }
}

/// Run the daemon on `socket` with the default limits.
pub fn run(socket: &Path) -> std::io::Result<()> {
    run_with(socket, Limits::default())
}

/// Run the daemon on `socket`: accept clients, serve each on its own
/// thread, exit when the last one leaves.
pub fn run_with(socket: &Path, limits: Limits) -> std::io::Result<()> {
    let queue_limit = limits.queue;
    LOG_BYTES.store(limits.log_bytes.max(1), Ordering::Relaxed);
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    claim_unhomed_oplog(socket);
    let note = socket.with_extension("home");
    let mut state = State::default();
    resume(&mut state, socket, &note);
    *lock(&HOME_NOTE) = Some(note.clone());
    // A stale socket from a dead daemon would make bind fail forever.
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    tracing::info!(socket = %socket.display(), "printcad-serverd listening");

    let hub = Arc::new(Hub {
        state: Mutex::new(state),
        file: Mutex::new(()),
        queue_limit,
        joined: AtomicBool::new(false),
    });
    let live = Arc::new(AtomicU64::new(0));
    let ids = AtomicU64::new(0);
    // The first drop-to-zero once a client has joined ends the daemon.
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
                if live.fetch_sub(1, Ordering::AcqRel) == 1 && hub.joined.load(Ordering::Acquire) {
                    let _ = exit_tx.send(());
                }
            });
        }
    });

    // Block until the last client leaves.
    let _ = exit_rx.recv();
    // Everyone has gone. When the last said it was done, the unsaved edits
    // went with it (all but those still waiting to be asked back); one that
    // went without a word leaves them named, for the next open to offer.
    // A document that never got a file has nothing to come back to once
    // its last client let its edits go, so its log goes too; after a crash
    // it stays, for the next daemon on the socket to write on.
    let homed = lock(&OPLOG_PATH).is_some();
    let state = lock(&hub.state);
    if homed {
        let unsaved = if state.last_left_cleanly {
            state.recoverable.iter().map(|logged| logged.id).collect()
        } else {
            state.unsaved()
        };
        mark_unsaved(&unsaved, home_document().as_deref());
    } else if state.last_left_cleanly {
        remove_unhomed_log();
    }
    drop(state);
    let _ = std::fs::remove_file(&note);
    let _ = std::fs::remove_file(socket);
    tracing::info!("last client disconnected; printcad-serverd exiting");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keep the logs these tests write out of the user's data folder.
    fn test_log_dir() {
        static POINTED: std::sync::Once = std::sync::Once::new();
        POINTED.call_once(|| {
            let dir = std::env::temp_dir().join(format!("printcad_oplogs_{}", std::process::id()));
            // Sound because every test reading the variable calls this
            // first, and `Once` orders those reads after the single write.
            unsafe { std::env::set_var("PRINTCAD_OPLOG_DIR", dir) };
        });
    }

    #[test]
    fn compressed_documents_keep_distinct_logs_and_blobs() {
        test_log_dir();
        let paths = ["a.prtcad", "a.prtcad.gz", "a.prtcad.zst"];
        for name in [document_oplog, legacy_oplog] {
            let logs: Vec<_> = paths.iter().map(|p| name(Path::new(p))).collect();
            for i in 0..logs.len() {
                for j in 0..i {
                    assert_ne!(logs[i], logs[j]);
                    assert_ne!(
                        logs[i].with_extension("blobs"),
                        logs[j].with_extension("blobs")
                    );
                }
            }
        }
        assert_eq!(
            legacy_oplog(Path::new("a.prtcad")),
            Path::new("a.oplog.jsonl")
        );
        assert_eq!(
            document_oplog(Path::new("a.prtcad")).parent(),
            Some(crate::oplog_dir().as_path())
        );
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

    /// Homing the log waits for an append under way, so no line lands in
    /// the unhomed log after its lines were carried over.
    #[test]
    fn homing_the_log_waits_for_its_write_lock() {
        test_log_dir();
        let dir = std::env::temp_dir().join(format!("printcad_homing_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let unhomed = dir.join("unhomed.oplog.jsonl");
        std::fs::write(&unhomed, "{\"op\":1}\n").expect("unhomed log");
        *lock(&UNHOMED_PATH) = Some(unhomed.clone());
        let document = dir.join("model.prtcad");

        let appending = lock(&OPLOG_WRITE);
        let homing = std::thread::spawn({
            let document = document.clone();
            move || set_oplog_home(&document, None)
        });
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(
            !homing.is_finished(),
            "homing went ahead of an append under way"
        );
        drop(appending);
        assert!(homing.join().expect("homing"));
        let homed = std::fs::read_to_string(document_oplog(&document)).expect("homed log");
        assert_eq!(homed, "{\"op\":1}\n");
        assert!(std::fs::symlink_metadata(&unhomed).is_err());
        *lock(&OPLOG_PATH) = None;
        *lock(&UNHOMED_PATH) = None;
        let _ = std::fs::remove_dir_all(&dir);
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
        let log = dir.join("model.oplog.jsonl");

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
        let restore_tail =
            |log: &Path, document: &Path| restore_tail_from(&read_log(log), log, Some(document));
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
