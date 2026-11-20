//! The daemon round-trip: spawn the real binary, speak the real protocol.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use core_document::Document;
use core_document::server::{ClientMessage, DocumentServer, ServerMessage};
use doc_server::DaemonClient;

/// How long any wait in these tests lasts before it fails, naming what it
/// waited for: a stall on one platform is a failure, never a hung run.
const PATIENCE: Duration = Duration::from_secs(30);

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

    /// Whether the socket's file is there. Asked of the entry itself: on
    /// Windows a socket file cannot be opened, so `exists`, which follows
    /// it, says no while a daemon listens on it.
    fn socket_present(&self) -> bool {
        std::fs::symlink_metadata(self.socket()).is_ok()
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
    static POINTED: std::sync::Once = std::sync::Once::new();
    POINTED.call_once(|| {
        let mut path = std::env::current_exe().expect("test exe");
        path.pop(); // deps/
        path.pop(); // <profile>/
        path.push(local_ipc::program_name("printcad-serverd"));
        // Sound because every test calls this before it first reads the
        // variable through the client, and `Once` orders those reads after
        // the single write; a test-local process could still race a
        // concurrent write, so this is the only writer in the binary.
        unsafe { std::env::set_var("PRINTCAD_SERVERD", &path) };
    });
}

/// Start the daemon by hand, as a process this test can stop, and wait for
/// it to listen.
fn start_daemon(home: &TestHome, args: &[&str]) -> std::process::Child {
    daemon_env();
    let binary = std::env::var_os("PRINTCAD_SERVERD").expect("daemon binary");
    let _ = std::fs::remove_file(home.socket());
    let child = std::process::Command::new(binary)
        .arg("--socket")
        .arg(home.socket())
        .args(args)
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("daemon starts");
    let deadline = Instant::now() + PATIENCE;
    while !home.socket_present() {
        assert!(Instant::now() < deadline, "the daemon never listened");
        std::thread::sleep(Duration::from_millis(10));
    }
    child
}

/// Wait for a daemon this test started to exit, killing it if it does not.
fn wait_for_exit(daemon: &mut std::process::Child, why: &str) {
    let deadline = Instant::now() + PATIENCE;
    while daemon.try_wait().expect("daemon status").is_none() {
        if Instant::now() >= deadline {
            let _ = daemon.kill();
            let _ = daemon.wait();
            panic!("timed out waiting for the daemon to exit {why}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
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
    let deadline = Instant::now() + PATIENCE;
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

/// Two documents are two daemons, and until each is saved its ops have
/// nowhere of its own to live. Neither may end up holding the other's
/// history.
#[test]
fn two_documents_do_not_swap_their_unsaved_history() {
    daemon_env();
    let home_a = TestHome::new("split_a");
    let home_b = TestHome::new("split_b");

    let mut a = DaemonClient::spawn_or_connect(&home_a.socket()).expect("daemon a");
    let mut b = DaemonClient::spawn_or_connect(&home_b.socket()).expect("daemon b");
    let mut inbox_a = Inbox::default();
    let mut inbox_b = Inbox::default();

    // Both record an edit before either has a file, so both histories are
    // homeless at the same time.
    let mut doc_a = Document::new("A");
    doc_a.create_body(Some("only-in-a".into()));
    a.send(ClientMessage::Ops(doc_a.take_pending_ops()));

    let mut doc_b = Document::new("B");
    doc_b.create_body(Some("only-in-b".into()));
    b.send(ClientMessage::Ops(doc_b.take_pending_ops()));

    // Wait until both histories really are homeless together, which is when
    // one could take the other's: the daemons are separate processes, so
    // without this the first save can land before the second edit is even
    // written.
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("printcad");
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        let mut seen_a = false;
        let mut seen_b = false;
        if let Ok(entries) = std::fs::read_dir(&runtime) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if !name.starts_with("unhomed") {
                    continue;
                }
                let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
                seen_a |= text.contains("only-in-a");
                seen_b |= text.contains("only-in-b");
            }
        }
        if seen_a && seen_b {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    for (client, inbox, doc, home) in [
        (&mut a, &mut inbox_a, &mut doc_a, &home_a),
        (&mut b, &mut inbox_b, &mut doc_b, &home_b),
    ] {
        let bytes = doc
            .save_to_bytes(core_document::Compression::None)
            .expect("serialize");
        client.send(ClientMessage::SaveDocument {
            path: home.document(),
            bytes,
            at_seq: doc.mutation_seq(),
            seen_seq: None,
            sent_batches: None,
        });
        wait_for(client, inbox, "save completion", |m| match m {
            ServerMessage::SaveCompleted { .. } => Some(()),
            ServerMessage::SaveFailed { error, .. } => panic!("save failed: {error}"),
            _ => None,
        });
    }

    let read = |home: &TestHome| -> String {
        let log = home.document().with_extension("oplog.jsonl");
        let deadline = Instant::now() + PATIENCE;
        loop {
            let text = std::fs::read_to_string(&log).unwrap_or_default();
            if !text.is_empty() || Instant::now() > deadline {
                return text;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    let log_a = read(&home_a);
    let log_b = read(&home_b);

    assert!(
        log_a.contains("only-in-a"),
        "A kept its own history: {log_a}"
    );
    assert!(
        log_b.contains("only-in-b"),
        "B kept its own history: {log_b}"
    );
    assert!(
        !log_a.contains("only-in-b"),
        "A swallowed B's history: {log_a}"
    );
    assert!(
        !log_b.contains("only-in-a"),
        "B swallowed A's history: {log_b}"
    );
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
        seen_seq: None,
        sent_batches: None,
    });
    let saved_seq = wait_for(&mut client, &mut inbox, "save completion", |m| match m {
        ServerMessage::SaveCompleted { at_seq, .. } => Some(*at_seq),
        ServerMessage::SaveFailed { error, .. } => panic!("save failed: {error}"),
        _ => None,
    });
    assert_eq!(saved_seq, doc.mutation_seq());
    assert!(home.document().is_file(), "the daemon wrote the file");

    // The op log lives beside the document: the save carries the ops sent
    // before it over from the unhomed log, and later ones append to it.
    let oplog = home.document().with_extension("oplog.jsonl");
    doc.rename_body(body, "Renamed again");
    client.send(ClientMessage::Ops(doc.take_pending_ops()));
    let log_deadline = Instant::now() + PATIENCE;
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
    let truncate_deadline = Instant::now() + PATIENCE;
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
    let gone_deadline = Instant::now() + PATIENCE;
    while home.socket_present() {
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
        ServerMessage::Ops { actor, ops, .. } => {
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
    let gone = Instant::now() + PATIENCE;
    while home.socket_present() {
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
        seen_seq: None,
        sent_batches: None,
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
    let deadline = Instant::now() + PATIENCE;
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

/// Presence relays to peers and dies with its author.
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
            cursor_world: Some([10.0, 20.0, 0.0]),
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
/// save (the unsaved present) and converges with the live editor.
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
        seen_seq: None,
        sent_batches: None,
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
    let seen = Instant::now() + PATIENCE;
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

/// A save `flush` waited for still answers on the next `poll`, through
/// either server, failed or completed: the answer is how the app learns
/// the file is written, and a close or quit goes on only once it is.
#[test]
fn a_flushed_save_still_answers_on_the_next_poll() {
    daemon_env();
    let home = TestHome::new("flush_answers");
    let daemon = DaemonClient::spawn_or_connect(&home.socket()).expect("spawn daemon");
    let servers: [(Box<dyn DocumentServer>, &str); 2] = [
        (Box::new(daemon), "daemon"),
        (Box::new(doc_server::DirectFiles::new()), "direct"),
    ];
    let bytes = Document::new("Flushed")
        .save_to_bytes(core_document::Compression::None)
        .expect("serialize");
    for (mut server, name) in servers {
        let path = home.dir.join(format!("{name}.prtcad"));
        // A folder where the file should go: that write fails.
        let blocked = home.dir.join(format!("{name}-blocked.prtcad"));
        std::fs::create_dir_all(&blocked).expect("blocking folder");
        for (path, at_seq) in [(&path, 7), (&blocked, 8)] {
            server.send(ClientMessage::SaveDocument {
                path: path.clone(),
                bytes: bytes.clone(),
                at_seq,
                seen_seq: None,
                sent_batches: None,
            });
        }
        server.flush();
        assert!(path.is_file(), "{name}: the save is on disk after flush");
        assert_eq!(server.status().saves_in_flight, 0, "{name}");
        let answers = server.poll();
        let completed = answers.iter().any(
            |m| matches!(m, ServerMessage::SaveCompleted { path: p, at_seq: 7 } if *p == path),
        );
        let failed = answers
            .iter()
            .any(|m| matches!(m, ServerMessage::SaveFailed { path: p, .. } if *p == blocked));
        assert!(
            completed && failed,
            "{name}: both saves answer: {answers:?}"
        );
    }
}

/// Two saves of one file sent back to back are written and answered in
/// the order sent, through either server: the file holds the later one
/// even when the earlier takes far longer to write.
#[test]
fn saves_sent_back_to_back_land_in_the_order_sent() {
    daemon_env();
    let home = TestHome::new("save_order");
    let daemon = DaemonClient::spawn_or_connect(&home.socket()).expect("spawn daemon");
    let servers: [(Box<dyn DocumentServer>, &str); 2] = [
        (Box::new(daemon), "daemon"),
        (Box::new(doc_server::DirectFiles::new()), "direct"),
    ];
    let bytes = |name: &str| {
        Document::new(name)
            .save_to_bytes(core_document::Compression::None)
            .expect("serialize")
    };
    let earlier = bytes(&"e".repeat(16 << 20));
    let later = bytes("Later");
    for (mut server, name) in servers {
        let path = home.dir.join(format!("{name}.prtcad"));
        for (bytes, at_seq) in [(&earlier, 1), (&later, 2)] {
            server.send(ClientMessage::SaveDocument {
                path: path.clone(),
                bytes: bytes.clone(),
                at_seq,
                seen_seq: None,
                sent_batches: None,
            });
        }
        server.flush();
        let deadline = Instant::now() + PATIENCE;
        let mut order = Vec::new();
        while order.len() < 2 && Instant::now() < deadline {
            order.extend(server.poll().into_iter().filter_map(|m| match m {
                ServerMessage::SaveCompleted { at_seq, .. } => Some(at_seq),
                _ => None,
            }));
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(order, [1, 2], "{name}: answered in the order sent");
        assert!(
            std::fs::read(&path).expect("written") == later,
            "{name}: the file holds the later save"
        );
    }
}

/// One editor as the app runs it: a connection and a document, its own
/// edits applied at once and sent as batches, the server's applied in the
/// order the server gives.
struct Replica {
    client: DaemonClient,
    inbox: Inbox,
    doc: Document,
    /// The newest sequence number heard, as another's batch or an ack.
    seen: u64,
    /// Batches this replica sent on its connection.
    sent: u64,
    /// Every sequence number heard, in the order heard.
    heard: Vec<u64>,
    /// The other editors the server last counted.
    peers: Option<u32>,
}

impl Replica {
    fn join(home: &TestHome, doc: Document) -> Self {
        Self {
            client: DaemonClient::spawn_or_connect(&home.socket()).expect("client"),
            inbox: Inbox::default(),
            doc,
            seen: 0,
            sent: 0,
            heard: Vec::new(),
            peers: None,
        }
    }

    /// Hand the outbox to the server as one batch.
    fn send(&mut self) {
        let ops = self.doc.take_pending_ops();
        if !ops.is_empty() {
            self.client.send(ClientMessage::Ops(ops));
            self.sent += 1;
        }
    }

    fn take(&mut self, message: ServerMessage) {
        match message {
            ServerMessage::Ops { seq, ops, .. } => {
                assert!(self.doc.apply_server_ops(&ops), "rolled back exactly");
                self.note(seq);
            }
            ServerMessage::Acked { seq } => {
                self.doc.confirm_sent_ops();
                self.note(seq);
            }
            ServerMessage::Peers { peers } => self.peers = Some(peers),
            _ => {}
        }
    }

    fn note(&mut self, seq: u64) {
        assert!(seq > self.seen, "sequence {seq} heard after {}", self.seen);
        self.seen = seq;
        self.heard.push(seq);
    }

    /// Take in what the server sends until `done` holds.
    fn take_until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done(self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            match self.inbox.next(&mut self.client) {
                Some(message) => self.take(message),
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }

    /// Wait until every batch sent is confirmed, and say where in the
    /// order the last one landed.
    fn settle(&mut self) -> u64 {
        self.take_until("confirmations", |r| r.doc.sent_ops_unconfirmed() == 0);
        self.seen
    }

    /// Wait until sequence number `seq` is heard.
    fn catch_up(&mut self, seq: u64) {
        self.take_until("the order to catch up", |r| r.seen >= seq);
    }

    /// Open `path` as the app does: whatever is heard before the file is
    /// in the file or in the tail that follows it.
    fn open(home: &TestHome, path: &std::path::Path) -> Self {
        let mut replica = Self::join(home, Document::new("opening"));
        replica.client.send(ClientMessage::OpenDocument {
            path: path.to_path_buf(),
            token: 9,
        });
        let bytes = wait_for(
            &mut replica.client,
            &mut replica.inbox,
            "open",
            |m| match m {
                ServerMessage::Opened { bytes, .. } => Some(bytes.clone()),
                ServerMessage::OpenFailed { error, .. } => panic!("open failed: {error}"),
                _ => None,
            },
        );
        replica.doc = Document::load_from_bytes(bytes).expect("parse");
        replica
    }

    /// Save as the app does, saying what the snapshot holds, and wait for
    /// the answer without taking in what arrives meanwhile.
    fn save(&mut self, path: &std::path::Path) {
        self.send();
        let bytes = self
            .doc
            .save_to_bytes(core_document::Compression::None)
            .expect("serialize");
        self.client.send(ClientMessage::SaveDocument {
            path: path.to_path_buf(),
            bytes,
            at_seq: self.doc.mutation_seq(),
            seen_seq: Some(self.seen),
            sent_batches: Some(self.sent),
        });
        let mut held = Vec::new();
        wait_for(&mut self.client, &mut self.inbox, "save", |m| match m {
            ServerMessage::SaveCompleted { .. } => Some(()),
            ServerMessage::SaveFailed { error, .. } => panic!("save failed: {error}"),
            ServerMessage::Ops { seq, ops, actor } => {
                held.push(ServerMessage::Ops {
                    actor: *actor,
                    seq: *seq,
                    ops: ops.clone(),
                });
                None
            }
            ServerMessage::Acked { seq } => {
                held.push(ServerMessage::Acked { seq: *seq });
                None
            }
            _ => None,
        });
        for message in held.into_iter().rev() {
            self.inbox.0.push_front(message);
        }
    }
}

/// Three editors write one field at once, one of them from an edit it
/// has not sent yet (a drag still held): every replica ends with the same
/// value, the one the server ordered last.
#[test]
fn concurrent_writes_to_one_field_converge_on_the_servers_order() {
    daemon_env();
    let home = TestHome::new("same_field");
    let mut base = Document::new("Shared");
    let body = base.create_body(Some("Frame".into()));
    let _ = base.take_pending_ops();
    let mut replicas: Vec<Replica> = (0..3).map(|_| Replica::join(&home, base.clone())).collect();

    // Alice and Bob each rename the body and send before hearing anything.
    replicas[0].doc.rename_body(body, "alice");
    replicas[1].doc.rename_body(body, "bob");
    // Carol renames it too but holds the edit back.
    replicas[2].doc.rename_body(body, "carol");
    replicas[0].send();
    replicas[1].send();
    let alice = replicas[0].settle();
    let bob = replicas[1].settle();
    let last = alice.max(bob);
    for replica in &mut replicas {
        replica.catch_up(last);
    }
    // Carol's held edit comes after both and still shows on her screen.
    assert_eq!(replicas[2].doc.bodies()[0].name, "carol");
    let later = if alice > bob { "alice" } else { "bob" };
    assert_eq!(replicas[0].doc.bodies()[0].name, later);
    assert_eq!(replicas[1].doc.bodies()[0].name, later);

    replicas[2].send();
    let last = replicas[2].settle();
    for replica in &mut replicas {
        replica.catch_up(last);
    }
    let projection = replicas[0].doc.replicated_projection();
    for replica in &replicas {
        assert_eq!(replica.doc.bodies()[0].name, "carol");
        assert_eq!(replica.doc.replicated_projection(), projection);
    }
}

/// Two editors send many batches at once: every client hears every batch
/// in one order, numbered without gaps, and the replicas end equal.
#[test]
fn batches_from_two_clients_arrive_in_one_order_everywhere() {
    daemon_env();
    let home = TestHome::new("one_order");
    let mut base = Document::new("Shared");
    let body = base.create_body(Some("Frame".into()));
    let _ = base.take_pending_ops();
    let mut replicas: Vec<Replica> = (0..4).map(|_| Replica::join(&home, base.clone())).collect();

    for round in 0..20 {
        for (writer, replica) in replicas.iter_mut().take(2).enumerate() {
            let name = format!("{writer}-{round}");
            replica.doc.rename_body(body, name.as_str());
            if round % 3 == 0 {
                replica.doc.set_name(name);
            }
            replica.send();
        }
    }
    let last = replicas[0].settle().max(replicas[1].settle());
    assert_eq!(last, 40, "one number per batch");
    for replica in &mut replicas {
        replica.catch_up(last);
    }
    let projection = replicas[0].doc.replicated_projection();
    for replica in &replicas {
        assert_eq!(
            replica.heard,
            (1..=last).collect::<Vec<_>>(),
            "every batch, in one order"
        );
        assert_eq!(replica.doc.replicated_projection(), projection);
    }
}

/// A client that opens a document another edits gets the unsaved edits;
/// a rebase sent while others edit leaves those edits for the next
/// joiner, and edits made after a join reach everyone.
#[test]
fn joiners_see_the_unsaved_edits_and_a_rebase_keeps_them() {
    daemon_env();
    let home = TestHome::new("join_tail");
    let mut alice = Replica::join(&home, Document::new("Joined"));
    let body = alice.doc.create_body(Some("Base".into()));
    alice.save(&home.document());
    alice.doc.rename_body(body, "unsaved by alice");
    alice.send();
    let after_rename = alice.settle();

    let mut bob = Replica::open(&home, &home.document());
    assert_eq!(
        bob.doc.bodies()[0].name,
        "Base",
        "the file is the saved past"
    );
    bob.catch_up(after_rename);
    assert_eq!(bob.doc.bodies()[0].name, "unsaved by alice");
    // A rebase while Alice edits must not take her edits from the tail.
    bob.client.send(ClientMessage::Rebase);
    let second = bob.doc.create_body(Some("by bob".into()));
    bob.send();
    let after_bob = bob.settle();
    alice.catch_up(after_bob);

    let mut carol = Replica::open(&home, &home.document());
    carol.catch_up(after_bob);
    let projection = alice.doc.replicated_projection();
    for replica in [&alice, &bob, &carol] {
        assert_eq!(replica.doc.bodies()[0].name, "unsaved by alice");
        assert!(replica.doc.body(second).is_some(), "bob's body reached all");
        assert_eq!(replica.doc.replicated_projection(), projection);
    }
}

/// A save takes from the tail only what its snapshot holds: a peer's edit
/// the saver had not heard yet stays for clients that join later.
#[test]
fn a_save_keeps_the_edits_its_snapshot_lacks() {
    daemon_env();
    let home = TestHome::new("save_tail");
    let mut alice = Replica::join(&home, Document::new("Saved"));
    let body = alice.doc.create_body(Some("Base".into()));
    alice.save(&home.document());
    let mut bob = Replica::open(&home, &home.document());

    bob.doc.rename_body(body, "bob, unheard by alice");
    bob.send();
    let after_bob = bob.settle();
    // Alice saves before taking Bob's edit in.
    alice.doc.set_name("Saved by alice");
    alice.save(&home.document());

    let mut carol = Replica::open(&home, &home.document());
    carol.catch_up(after_bob);
    assert_eq!(carol.doc.name(), "Saved by alice", "the snapshot");
    assert_eq!(
        carol.doc.bodies()[0].name,
        "bob, unheard by alice",
        "the edit the snapshot lacks"
    );
}

/// A daemon that dies leaves the unsaved edits in its log; the daemon that
/// takes its place reads them back, so a client joining after the restart
/// still gets them after the file.
#[test]
fn a_restarted_daemon_gives_joiners_the_unsaved_tail() {
    let home = TestHome::new("restart_tail");
    let mut daemon = start_daemon(&home, &[]);
    let mut alice = Replica::join(&home, Document::new("Restarted"));
    let body = alice.doc.create_body(Some("Base".into()));
    alice.save(&home.document());
    alice.doc.rename_body(body, "unsaved when it died");
    alice.send();
    alice.settle();
    daemon.kill().expect("the daemon stops");
    wait_for_exit(&mut daemon, "once killed");

    let mut bob = Replica::open(&home, &home.document());
    assert_eq!(
        bob.doc.bodies()[0].name,
        "Base",
        "the file is the saved past"
    );
    bob.take_until("the unsaved tail", |r| {
        r.doc.bodies()[0].name == "unsaved when it died"
    });
    // Bob's own edits come after the tail in the new daemon's order.
    bob.doc.set_name("after the restart");
    bob.send();
    assert!(bob.settle() > 1);
}

/// When every client has left, their unsaved edits are gone with them: the
/// next daemon on the document starts from the file alone.
#[test]
fn a_daemon_everyone_left_starts_again_from_the_file() {
    daemon_env();
    let home = TestHome::new("left_tail");
    let mut alice = Replica::join(&home, Document::new("Left"));
    let body = alice.doc.create_body(Some("Base".into()));
    alice.save(&home.document());
    alice.doc.rename_body(body, "discarded");
    alice.send();
    alice.settle();
    drop(alice);
    let deadline = Instant::now() + PATIENCE;
    while home.socket_present() {
        assert!(Instant::now() < deadline, "the daemon never exited");
        std::thread::sleep(Duration::from_millis(10));
    }

    let mut bob = Replica::open(&home, &home.document());
    bob.doc.set_name("reopened");
    bob.send();
    assert_eq!(bob.settle(), 1, "nothing was ordered before bob's edit");
    assert_eq!(bob.doc.bodies()[0].name, "Base");
}

/// A client that stops reading is disconnected once its queue passes the
/// limit, its peers told; it rejoins by opening the document again and
/// gets every edit it missed from the file and the tail.
#[test]
fn a_client_too_far_behind_is_dropped_and_rejoins() {
    let home = TestHome::new("slow_client");
    let mut daemon = start_daemon(&home, &["--queue-limit", "16"]);
    let mut alice = Replica::join(&home, Document::new("Busy"));
    let body = alice.doc.create_body(Some("Base".into()));
    alice.save(&home.document());

    let mut stalled = local_ipc::Stream::connect(home.socket()).expect("connect");
    doc_server::framing::write_frame(
        &mut stalled,
        &ClientMessage::Hello {
            protocol: core_document::server::SERVER_PROTOCOL_VERSION,
            actor: uuid::Uuid::new_v4(),
        },
    )
    .expect("hello");
    alice.take_until("a second editor", |r| r.peers == Some(1));

    let rounds = 60;
    let padding = "x".repeat(200_000);
    for round in 0..rounds {
        alice.doc.rename_body(body, format!("{round} {padding}"));
        alice.send();
    }
    alice.take_until("the stalled editor dropped", |r| r.peers == Some(0));
    let last = alice.settle();

    // What was written before the disconnect, then the end of the stream.
    stalled
        .set_read_timeout(Some(PATIENCE))
        .expect("read timeout");
    let mut relayed = 0;
    loop {
        match doc_server::framing::read_frame::<_, ServerMessage>(&mut stalled) {
            Ok(ServerMessage::Ops { .. }) => relayed += 1,
            Ok(_) => {}
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                panic!("timed out waiting for the dropped client's stream to end")
            }
            Err(_) => break,
        }
    }
    assert!(relayed < rounds, "the stream ended before the edits did");
    // Closed from this end too: a daemon's reader blocked on the socket
    // ends at the latest when its peer closes, whatever the platform's
    // shutdown does to a read already waiting.
    drop(stalled);

    let mut again = Replica::open(&home, &home.document());
    again.catch_up(last);
    assert_eq!(
        again.doc.replicated_projection(),
        alice.doc.replicated_projection(),
        "the rejoined client holds every edit"
    );
    drop(again);
    drop(alice);
    wait_for_exit(&mut daemon, "with its last client");
}
