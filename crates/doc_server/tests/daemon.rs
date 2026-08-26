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

/// Messages polled but not yet consumed by a `wait_for`. Poll batches can
/// carry several messages; without this, waiting for one would silently
/// discard the others in its batch.
#[derive(Default)]
struct Inbox(std::collections::VecDeque<ServerMessage>);

impl Inbox {
    fn next(&mut self, client: &mut DaemonClient) -> Option<ServerMessage> {
        if let Some(message) = self.0.pop_front() {
            return Some(message);
        }
        self.0.extend(client.poll());
        self.0.pop_front()
    }
}

fn wait_for<T>(
    client: &mut DaemonClient,
    inbox: &mut Inbox,
    what: &str,
    mut pick: impl FnMut(&ServerMessage) -> Option<T>,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        while let Some(message) = inbox.next(client) {
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
    let mut inbox = Inbox::default();
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
    let saved_seq = wait_for(&mut client, &mut inbox, "save completion", |m| match m {
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
    let bytes = wait_for(&mut client, &mut inbox, "open", |m| match m {
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

/// Two clients, one document: edits made by one arrive at the other in
/// order, never echo back to their author, and both replicas converge.
#[test]
fn a_peers_edits_relay_and_the_replicas_converge() {
    daemon_env();
    let home = TestHome::new("relay");

    let mut alice = DaemonClient::spawn_or_connect(&home.socket()).expect("first client");
    let _alice_inbox = Inbox::default();
    let mut bob = DaemonClient::spawn_or_connect(&home.socket()).expect("second client");
    let mut bob_inbox = Inbox::default();
    assert_ne!(alice.actor(), bob.actor());

    // Both replicas start from the same baseline.
    let mut alice_doc = Document::new("Shared");
    let _ = alice_doc.take_pending_ops();
    let mut bob_doc = alice_doc.clone();

    // Alice edits; her ops go to the server.
    let body = alice_doc.create_body(Some("Frame".into()));
    alice_doc.rename_body(body, "Frame (alice)");
    let ops = alice_doc.take_pending_ops();
    let alice_actor = alice.actor();
    alice.send(ClientMessage::Ops(ops));

    // Bob hears them (and only them), attributed to Alice.
    let relayed = wait_for(&mut bob, &mut bob_inbox, "relayed ops", |m| match m {
        ServerMessage::Ops { actor, ops } => {
            assert_eq!(*actor, alice_actor, "author must be preserved");
            Some(ops.clone())
        }
        _ => None,
    });
    for op in &relayed {
        bob_doc.apply_op(op);
    }
    assert_eq!(
        alice_doc.replicated_projection(),
        bob_doc.replicated_projection(),
        "replicas must converge after relay"
    );

    // Alice must never hear her own ops back.
    std::thread::sleep(Duration::from_millis(200));
    for message in alice.poll() {
        assert!(
            !matches!(message, ServerMessage::Ops { .. }),
            "an author must not receive an echo of its own ops"
        );
    }

    // Peer counts: each side sees one other editor.
    assert_eq!(bob.status().peers, 1);

    drop(bob);
    drop(alice); // last client out; the daemon exits and removes its socket
    let gone = Instant::now() + Duration::from_secs(5);
    while home.socket().exists() {
        assert!(Instant::now() < gone, "daemon lingered after last client");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Big payloads leave the log for the content-addressed blob store, and the
/// same bytes imported twice share one stored blob.
#[test]
fn large_blobs_are_extracted_and_deduplicated() {
    daemon_env();
    let home = TestHome::new("blobs");

    let mut client = DaemonClient::spawn_or_connect(&home.socket()).expect("client");
    let mut inbox = Inbox::default();

    // Home the log first so blobs land beside the document.
    let mut doc = Document::new("Blobby");
    let bytes = doc
        .save_to_bytes(core_document::Compression::None)
        .expect("serialize");
    client.send(ClientMessage::SaveDocument {
        path: home.document(),
        bytes,
        at_seq: doc.mutation_seq(),
    });
    wait_for(&mut client, &mut inbox, "save", |m| match m {
        ServerMessage::SaveCompleted { .. } => Some(()),
        ServerMessage::SaveFailed { error, .. } => panic!("save failed: {error}"),
        _ => None,
    });

    // An op with a payload comfortably over the extraction threshold, twice.
    let big = vec![0xA5u8; 512 * 1024];
    for _ in 0..2 {
        let mut d = Document::new("x");
        let _ = d.take_pending_ops();
        d.add_asset_with_data(
            core_document::AssetReference::new(
                "assets/big.step".to_string(),
                core_document::AssetType::Step,
                serde_json::json!({}),
            ),
            big.clone(),
        );
        client.send(ClientMessage::Ops(d.take_pending_ops()));
    }
    client.flush();

    let oplog = home.document().with_extension("oplog.jsonl");
    let blob_dir = home.document().with_extension("oplog.blobs");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let log_ok = std::fs::read_to_string(&oplog)
            .map(|t| t.lines().count() >= 2 && t.contains("blob:sha256:") && t.len() < 64 * 1024)
            .unwrap_or(false);
        let blobs: usize = std::fs::read_dir(&blob_dir).map(|d| d.count()).unwrap_or(0);
        if log_ok && blobs == 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "expected a small log with blob markers and ONE stored blob; log_ok={log_ok}, blobs={blobs}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Presence relays to peers and dies with its author — and never touches
/// the op log.
#[test]
fn presence_relays_and_dies_with_its_author() {
    daemon_env();
    let home = TestHome::new("presence");

    let mut alice = DaemonClient::spawn_or_connect(&home.socket()).expect("alice");
    let _alice_inbox = Inbox::default();
    let mut bob = DaemonClient::spawn_or_connect(&home.socket()).expect("bob");
    let mut bob_inbox = Inbox::default();
    let alice_actor = alice.actor();

    let body = uuid::Uuid::new_v4();
    alice.send(ClientMessage::Presence(
        core_document::server::PresenceState {
            display_name: "alice".into(),
            selected_body: Some(body),
        },
    ));
    let state = wait_for(&mut bob, &mut bob_inbox, "presence", |m| match m {
        ServerMessage::PresencePeer { actor, state } => {
            assert_eq!(*actor, alice_actor);
            Some(state.clone())
        }
        _ => None,
    });
    assert_eq!(state.selected_body, Some(body));

    drop(alice);
    wait_for(&mut bob, &mut bob_inbox, "presence gone", |m| match m {
        ServerMessage::PresenceGone { actor } => {
            assert_eq!(*actor, alice_actor);
            Some(())
        }
        _ => None,
    });
}

/// A client joining mid-session gets the saved file PLUS the ops since the
/// save — the unsaved present — and converges with the live editor.
#[test]
fn a_late_joiner_catches_up_to_the_unsaved_present() {
    daemon_env();
    let home = TestHome::new("latejoin");

    let mut alice = DaemonClient::spawn_or_connect(&home.socket()).expect("alice");
    let mut alice_inbox = Inbox::default();

    // Alice saves a baseline, then keeps editing without saving.
    let mut alice_doc = Document::new("Late");
    let body = alice_doc.create_body(Some("Base".into()));
    alice.send(ClientMessage::Ops(alice_doc.take_pending_ops()));
    let bytes = alice_doc
        .save_to_bytes(core_document::Compression::None)
        .expect("serialize");
    alice.send(ClientMessage::SaveDocument {
        path: home.document(),
        bytes,
        at_seq: alice_doc.mutation_seq(),
    });
    wait_for(&mut alice, &mut alice_inbox, "save", |m| match m {
        ServerMessage::SaveCompleted { .. } => Some(()),
        ServerMessage::SaveFailed { error, .. } => panic!("save failed: {error}"),
        _ => None,
    });
    alice_doc.rename_body(body, "Edited after save");
    alice.send(ClientMessage::Ops(alice_doc.take_pending_ops()));
    alice.flush();
    // Wait until the daemon has processed the rename (observable in the
    // log) so bob's join deterministically exercises the TAIL path rather
    // than the live-relay path.
    let oplog = home.document().with_extension("oplog.jsonl");
    let seen = Instant::now() + Duration::from_secs(5);
    while !std::fs::read_to_string(&oplog)
        .map(|t| t.contains("Edited after save"))
        .unwrap_or(false)
    {
        assert!(Instant::now() < seen, "rename never reached the daemon log");
        std::thread::sleep(Duration::from_millis(10));
    }

    // Bob joins and opens: file bytes, then the tail.
    let mut bob = DaemonClient::spawn_or_connect(&home.socket()).expect("bob");
    let mut bob_inbox = Inbox::default();
    bob.send(ClientMessage::OpenDocument {
        path: home.document(),
        token: 1,
    });
    let file_bytes = wait_for(&mut bob, &mut bob_inbox, "opened", |m| match m {
        ServerMessage::Opened { bytes, .. } => Some(bytes.clone()),
        ServerMessage::OpenFailed { error, .. } => panic!("open failed: {error}"),
        _ => None,
    });
    let mut bob_doc = Document::load_from_bytes(file_bytes).expect("parse");
    assert_eq!(
        bob_doc.bodies()[0].name,
        "Base",
        "the file is the saved past"
    );

    let tail = wait_for(&mut bob, &mut bob_inbox, "tail ops", |m| match m {
        ServerMessage::Ops { ops, .. } => Some(ops.clone()),
        _ => None,
    });
    for op in &tail {
        bob_doc.apply_remote_op(op);
    }
    assert_eq!(
        bob_doc.bodies()[0].name,
        "Edited after save",
        "the tail is the unsaved present"
    );
}
