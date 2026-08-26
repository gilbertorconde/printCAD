//! The daemon round-trip: spawn the real binary, speak the real protocol.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use core_document::server::{ClientMessage, DocumentServer, ServerMessage};
use core_document::Document;
use doc_server::DaemonClient;

/// Each test gets its own socket + document dir, torn down with the daemon.
struct TestHome {
    dir: PathBuf,
}

impl TestHome {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("printcad_daemon_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("test dir");
        Self { dir }
    }

    fn socket(&self) -> PathBuf {
        self.dir.join("doc.sock")
    }

    fn document(&self) -> PathBuf {
        self.dir.join("model.prtcad")
    }
}

impl Drop for TestHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Point the client at the freshly built daemon binary. Cargo puts test
/// binaries under target/<profile>/deps; the daemon sits one level up.
fn daemon_env() {
    let mut path = std::env::current_exe().expect("test exe");
    path.pop(); // deps/
    path.pop(); // <profile>/
    path.push("printcad-serverd");
    std::env::set_var("PRINTCAD_SERVERD", &path);
}

fn wait_for<T>(
    client: &mut DaemonClient,
    what: &str,
    mut pick: impl FnMut(&ServerMessage) -> Option<T>,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        for message in client.poll() {
            if let Some(found) = pick(&message) {
                return found;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn a_session_round_trips_through_the_daemon() {
    daemon_env();
    let home = TestHome::new("roundtrip");

    let mut client = DaemonClient::spawn_or_connect(&home.socket()).expect("spawn daemon");
    assert!(client.status().connected, "handshake completed");

    // Build a small document client-side, save it THROUGH the daemon.
    let mut doc = Document::new("Daemon test");
    let body = doc.create_body(Some("Base".into()));
    doc.rename_body(body, "Renamed");
    let ops = doc.take_pending_ops();
    assert_eq!(ops.len(), 2);
    client.send(ClientMessage::Ops(ops));

    let bytes = doc
        .save_to_bytes(core_document::Compression::None)
        .expect("serialize");
    client.send(ClientMessage::SaveDocument {
        path: home.document(),
        bytes,
        at_seq: doc.mutation_seq(),
    });
    let saved_seq = wait_for(&mut client, "save completion", |m| match m {
        ServerMessage::SaveCompleted { at_seq, .. } => Some(*at_seq),
        ServerMessage::SaveFailed { error, .. } => panic!("save failed: {error}"),
        _ => None,
    });
    assert_eq!(saved_seq, doc.mutation_seq());
    assert!(home.document().is_file(), "the daemon wrote the file");

    // The op log lives beside the document and holds our two envelopes.
    let oplog = home.document().with_extension("oplog.jsonl");
    // Ops sent before the first save land in the unhomed log; send more now
    // that the log has a home and verify they arrive.
    doc.rename_body(body, "Renamed again");
    client.send(ClientMessage::Ops(doc.take_pending_ops()));
    let log_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if oplog.is_file() {
            let text = std::fs::read_to_string(&oplog).expect("read oplog");
            if text.lines().count() >= 1 {
                let first: serde_json::Value =
                    serde_json::from_str(text.lines().next().expect("line")).expect("envelope");
                assert_eq!(first["v"], core_document::op::OP_PROTOCOL_VERSION);
                assert!(first["op"].is_object());
                break;
            }
        }
        assert!(Instant::now() < log_deadline, "op log never appeared");
        std::thread::sleep(Duration::from_millis(20));
    }

    // Open it back through the daemon; the parsed document matches.
    client.send(ClientMessage::OpenDocument {
        path: home.document(),
        token: 42,
    });
    let bytes = wait_for(&mut client, "open", |m| match m {
        ServerMessage::Opened { token, bytes, .. } => {
            assert_eq!(*token, 42);
            Some(bytes.clone())
        }
        ServerMessage::OpenFailed { error, .. } => panic!("open failed: {error}"),
        _ => None,
    });
    let reopened = Document::load_from_bytes(bytes).expect("parse");
    assert_eq!(reopened.bodies().len(), 1);
    assert_eq!(reopened.bodies()[0].name, "Renamed");

    // Rebase truncates the op log.
    client.send(ClientMessage::Rebase);
    let truncate_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let len = std::fs::read_to_string(&oplog)
            .map(|t| t.len())
            .unwrap_or(usize::MAX);
        if len == 0 {
            break;
        }
        assert!(Instant::now() < truncate_deadline, "rebase never truncated");
        std::thread::sleep(Duration::from_millis(20));
    }

    client.flush();
    drop(client); // disconnect; the daemon should exit and remove its socket
    let gone_deadline = Instant::now() + Duration::from_secs(5);
    while home.socket().exists() {
        assert!(
            Instant::now() < gone_deadline,
            "daemon did not clean up its socket on disconnect"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_second_client_is_refused_while_the_first_holds_the_document() {
    daemon_env();
    let home = TestHome::new("secondclient");

    let first = DaemonClient::spawn_or_connect(&home.socket()).expect("first client");
    assert!(first.status().connected);

    // The daemon accepts the connection then drops it without HelloOk.
    let second = DaemonClient::spawn_or_connect(&home.socket());
    assert!(
        second.is_err(),
        "a second writer must be refused, got a connection"
    );
    drop(first);
}
