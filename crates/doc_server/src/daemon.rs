//! The daemon: one document, one socket, one client, no `Document` parsing.
//!
//! Responsibilities in this milestone: own the document's file (reads on
//! `OpenDocument`, atomic writes on `SaveDocument`), append every op frame
//! to a sidecar log (`<file>.oplog.jsonl` — one JSON envelope per line,
//! truncated on `Rebase`), and refuse a second client: single-writer is the
//! honest semantics until the sync milestone gives concurrent edits meaning.
//! Everything it stores is opaque bytes or op envelopes, so daemon and app
//! can be years apart in version and still cooperate.

use std::io::Write as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use core_document::server::{ClientMessage, ServerMessage, SERVER_PROTOCOL_VERSION};

use crate::framing::{read_frame, write_frame};

/// One accepted connection, served to disconnect. Returns when the client
/// goes away; the caller decides whether to accept another or exit.
fn serve_client(mut stream: UnixStream) -> std::io::Result<()> {
    // Handshake first: a version mismatch is a refusal, not a guess.
    let hello: ClientMessage = read_frame(&mut stream)?;
    match hello {
        ClientMessage::Hello { protocol } if protocol == SERVER_PROTOCOL_VERSION => {
            write_frame(
                &mut stream,
                &ServerMessage::HelloOk {
                    protocol: SERVER_PROTOCOL_VERSION,
                },
            )?;
        }
        ClientMessage::Hello { protocol } => {
            tracing::warn!(
                theirs = protocol,
                ours = SERVER_PROTOCOL_VERSION,
                "protocol mismatch"
            );
            return Ok(());
        }
        _ => {
            tracing::warn!("client spoke before Hello; dropping");
            return Ok(());
        }
    }

    loop {
        let message: ClientMessage = match read_frame(&mut stream) {
            Ok(m) => m,
            // Disconnect (or a broken frame, which we treat the same way:
            // this transport has no resync point).
            Err(_) => return Ok(()),
        };
        match message {
            ClientMessage::Hello { .. } => {
                tracing::warn!("repeated Hello mid-session; ignoring");
            }
            ClientMessage::Ops(ops) => {
                if let Err(err) = append_ops(&ops) {
                    tracing::error!("op log append failed: {err}");
                }
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
                write_frame(&mut stream, &reply)?;
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
                write_frame(&mut stream, &reply)?;
            }
        }
    }
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
static OPLOG_PATH: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

fn set_oplog_home(document: &Path) {
    let mut slot = OPLOG_PATH.lock().unwrap_or_else(|p| p.into_inner());
    *slot = Some(document.with_extension("oplog.jsonl"));
}

fn oplog_path() -> PathBuf {
    let slot = OPLOG_PATH.lock().unwrap_or_else(|p| p.into_inner());
    slot.clone()
        .unwrap_or_else(|| crate::runtime_dir_for_logs().join("unhomed.oplog.jsonl"))
}

fn append_ops(ops: &[core_document::op::DocumentOp]) -> std::io::Result<()> {
    let path = oplog_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    for op in ops {
        let envelope = serde_json::json!({
            "v": core_document::op::OP_PROTOCOL_VERSION,
            "op": op,
        });
        let line = serde_json::to_string(&envelope)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writeln!(file, "{line}")?;
    }
    Ok(())
}

fn truncate_oplog() -> std::io::Result<()> {
    let path = oplog_path();
    if path.exists() {
        std::fs::write(path, b"")?;
    }
    Ok(())
}

/// Run the daemon on `socket`: accept the first client, serve it to
/// disconnect, refuse extras meanwhile, then clean up and return.
pub fn run(socket: &Path) -> std::io::Result<()> {
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // A stale socket from a dead daemon would make bind fail forever.
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    tracing::info!(socket = %socket.display(), "printcad-serverd listening");

    let (stream, _addr) = listener.accept()?;

    // Single-writer: refuse any second connection while the first is live.
    let refusals = {
        let listener = listener.try_clone()?;
        std::thread::spawn(move || {
            for extra in listener.incoming().flatten() {
                tracing::warn!("second client refused: this document already has a writer");
                // The refused client sees EOF instead of HelloOk and reports
                // a busy document.
                drop(extra);
            }
        })
    };

    let served = serve_client(stream);
    drop(refusals); // detached; dies with the process
    let _ = std::fs::remove_file(socket);
    tracing::info!("client disconnected; printcad-serverd exiting");
    served
}
