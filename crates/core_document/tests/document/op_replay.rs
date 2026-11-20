//! The op stream is the document: replaying a session's captured ops onto a
//! baseline snapshot must land on the same replicated state the live edits
//! produced. This is the property any sync transport rests on.

use core_document::datum::{DatumAttachment, DatumFeature, DatumShape};
use core_document::units::Unit;
use core_document::{BasePlane, Document};

use crate::support::{assert_replays, geometry, reloaded};

fn datum(plane: BasePlane) -> DatumFeature {
    DatumFeature {
        shape: DatumShape::Plane { size: 10.0 },
        attachment: DatumAttachment::BasePlane(plane),
        offset: Default::default(),
    }
}

/// A representative editing session: bodies, features, payload updates,
/// renames, reorders, suppression, deletion, visibility, unit changes.
fn scripted_session(doc: &mut Document) {
    let body = doc.create_body(Some("Base".into()));
    let other = doc.create_body(None);
    doc.rename_body(other, "Bracket");

    let d1 = doc
        .add_feature_in_body(datum(BasePlane::XY), "Datum 1".into(), Some(body))
        .expect("add d1");
    let d2 = doc
        .add_feature_in_body(datum(BasePlane::XZ), "Datum 2".into(), Some(body))
        .expect("add d2");
    let d3 = doc
        .add_feature_in_body(datum(BasePlane::YZ), "Datum 3".into(), Some(body))
        .expect("add d3");

    doc.update_feature_data(d1, serde_json::json!({"k": 1}))
        .expect("update");
    // Drag-like burst: coalesces in the outbox, final payload wins.
    for i in 0..20 {
        doc.update_feature_data(d2, serde_json::json!({"drag": i}))
            .expect("update");
    }
    doc.rename_feature(d2, "Datum 2 (moved)").unwrap();
    doc.set_feature_suppressed(d3, true);
    doc.set_feature_visible(d1, false);
    doc.set_feature_dependencies(d3, vec![d1]);
    assert!(doc.move_feature_in_history(d2, true), "swap d1/d2");
    doc.remove_feature(d1).expect("remove");
    doc.set_body_tip(body, Some(d2));
    let note = core_document::Note::pinned("Wall".into(), [1.0, 2.0, 3.0], [1.0, 2.0, 9.0], None);
    let note = doc.add_note(body, note).expect("a note on the body");
    let mut edited = doc.note(note).expect("the note").1.clone();
    edited.text = "=1 mm + 2 mm".into();
    assert!(doc.set_note(edited));
    let asked = doc.new_note(
        body,
        "Thicker?".into(),
        core_document::NoteAuthor::Agent,
        None,
        core_document::NoteWant::Edge(None),
    );
    let asked = doc.add_note(body, asked).expect("an agent's note");
    let mut answered = doc.note(asked).expect("the note").1.clone();
    answered.resolved = Some("Yes".into());
    assert!(doc.set_note(answered));
    assert!(doc.remove_body(other), "the second body goes away");
    let op = doc.add_feature_of_kind(
        core_document::WorkbenchId::new("acme.cam.op"),
        "Pocket op".into(),
        Some(body),
        Vec::new(),
        serde_json::json!({"depth": 2}),
        core_document::FeatureOrigin::new("acme.cam 0.1.0", Some("acme/cam".into())),
    );
    doc.set_feature_origin(
        op,
        core_document::FeatureOrigin::new("acme.cam 0.2.0", Some("acme/cam".into())),
    )
    .expect("origin");
    doc.set_display_unit(Unit::In);
    doc.set_measure_unit(Some(Unit::Cm));
    doc.set_agent_rules("Keep every wall at least 1.2 mm thick.");
    doc.save_section(section("Mid", body));
    doc.set_name("Replayed");
}

/// Replaying a session's captured ops onto the baseline, as they are and
/// through the JSON the wire and the on-disk op log carry them as, lands on
/// the live document's replicated state.
#[test]
fn replaying_captured_ops_reproduces_the_replicated_state() {
    let mut live = Document::new("Session");
    let baseline = live.clone();
    let _ = live.take_pending_ops(); // start the capture window clean

    scripted_session(&mut live);
    let ops = live.take_pending_ops();
    assert!(
        ops.len() >= 14,
        "the session should capture one op per effective edit, got {}",
        ops.len()
    );

    let mut replica = baseline.clone();
    for op in &ops {
        replica.apply_op(op);
    }
    assert_eq!(
        live.replicated_projection(),
        replica.replicated_projection(),
        "replay must converge on the live document's replicated state"
    );
    assert_replays(&live, baseline, &ops);
}

/// A drag burst must not flood the outbox: consecutive whole-payload writes
/// to one feature coalesce to the final payload.
#[test]
fn a_drag_burst_coalesces_to_one_op() {
    let mut doc = Document::new("Drag");
    let body = doc.create_body(None);
    let d = doc
        .add_feature_in_body(datum(BasePlane::XY), "D".into(), Some(body))
        .expect("add");
    let _ = doc.take_pending_ops();

    for i in 0..100 {
        doc.update_feature_data(d, serde_json::json!({"x": i}))
            .expect("update");
    }
    let ops = doc.take_pending_ops();
    assert_eq!(ops.len(), 1, "a hundred drag frames, one op");
}

/// A foreign edit must trigger THIS replica's recompute: applying a remote
/// op marks the features whose build inputs it changed as dirty.
#[test]
fn remote_ops_mark_affected_features_dirty_here() {
    let mut alice = Document::new("Shared");
    let _ = alice.take_pending_ops();
    let mut bob = alice.clone();

    let body = alice.create_body(Some("Base".into()));
    let d = alice
        .add_feature_in_body(datum(BasePlane::XY), "D".into(), Some(body))
        .expect("add");
    alice
        .update_feature_data(d, serde_json::json!({"edited": true}))
        .expect("update");
    let ops = alice.take_pending_ops();

    for op in &ops {
        bob.apply_remote_op(op);
    }
    assert_eq!(
        alice.replicated_projection(),
        bob.replicated_projection(),
        "replicas converge"
    );
    assert!(
        bob.dirty_features().contains(&d),
        "bob's replica must know it has geometry to re-derive"
    );
}

/// Two replicas inserting features concurrently take the same `seq`; the
/// server's order settles it: the insert ordered later moves after the
/// other on every replica, so each history number names one feature.
#[test]
fn concurrent_inserts_take_distinct_places_in_the_servers_order() {
    let mut alice = Document::new("Order");
    let body = alice.create_body(Some("B".into()));
    let _ = alice.take_pending_ops();
    alice.confirm_sent_ops();
    let mut bob = alice.clone();

    // Each replica inserts one feature without seeing the other's yet:
    // both take the same seq.
    let a = alice
        .add_feature_in_body(datum(BasePlane::XY), "From alice".into(), Some(body))
        .expect("alice add");
    let b = bob
        .add_feature_in_body(datum(BasePlane::XZ), "From bob".into(), Some(body))
        .expect("bob add");
    let seq = |doc: &Document, id| doc.feature_tree().get_node(id).expect("node").seq;
    assert_eq!(seq(&alice, a), seq(&bob, b), "the collision this settles");
    let alice_ops = alice.take_pending_ops();
    let bob_ops = bob.take_pending_ops();

    // The server orders Alice's batch first: she hears her ack, then
    // Bob's batch; Bob hears hers beneath his unconfirmed insert.
    alice.confirm_sent_ops();
    assert!(alice.apply_server_ops(&bob_ops));
    assert!(bob.apply_server_ops(&alice_ops));
    bob.confirm_sent_ops();

    for doc in [&alice, &bob] {
        assert!(seq(doc, a) < seq(doc, b), "the later insert comes after");
        let names: Vec<_> = doc
            .feature_tree()
            .ordered_nodes(|_| true)
            .map(|(_, n)| n.name.clone())
            .collect();
        assert_eq!(names, ["From alice", "From bob"]);
    }
    assert_eq!(
        alice.replicated_projection(),
        bob.replicated_projection(),
        "the replicas converge, history numbers included"
    );
}

/// A server's op lands beneath this replica's unconfirmed ones when each
/// of those can be rolled back, and over them, reported, when one cannot.
#[test]
fn server_ops_land_beneath_unconfirmed_ones_that_roll_back() {
    let mut alice = Document::new("Under");
    let body = alice.create_body(Some("B".into()));
    let _ = alice.take_pending_ops();
    alice.confirm_sent_ops();
    let mut bob = alice.clone();
    bob.rename_body(body, "bob");
    let from_bob = bob.take_pending_ops();

    alice.rename_body(body, "alice, sent");
    let _ = alice.take_pending_ops();
    alice.rename_body(body, "alice, held");
    assert!(alice.apply_server_ops(&from_bob));
    assert_eq!(alice.bodies()[0].name, "alice, held");

    alice.add_asset_with_data(
        core_document::AssetReference::new(
            "assets/part.step".to_string(),
            core_document::AssetType::Step,
            serde_json::json!({}),
        ),
        vec![1, 2, 3],
    );
    assert!(
        !alice.apply_server_ops(&from_bob),
        "an asset has no inverse"
    );
    assert_eq!(alice.bodies()[0].name, "bob");
}

/// A replica rejoining its server carries the edits the server has not
/// confirmed onto the server's copy: they stay on top and unconfirmed
/// under the same ids, a gesture still open closes there, and undo steps
/// back through the user's own edits, the confirmed one too, leaving a
/// peer's edit alone.
#[test]
fn edits_carried_onto_the_servers_copy_stay_unconfirmed_and_still_undo() {
    let mut alice = Document::new("Rejoin");
    let mut journal = core_document::history::OpJournal::new(16);
    let body = alice.create_body(Some("Frame".into()));
    journal.note(&mut alice);
    let _ = alice.take_pending_ops();
    alice.confirm_sent_ops();
    let mut server = alice.clone();

    alice.rename_body(body, "confirmed");
    journal.note(&mut alice);
    let confirmed = alice.take_pending_ops();
    alice.confirm_sent_ops();
    let in_flight = alice.create_body(Some("in flight".into()));
    journal.note(&mut alice);
    let (batch, _) = alice.take_pending_batch().expect("a batch");
    alice.set_name("pending");

    // The server's copy: the file and its tail, which holds the confirmed
    // edit and a peer's.
    for op in &confirmed {
        server.apply_remote_op(op);
    }
    let mut peer = server.clone();
    let peers = peer.create_body(Some("peer".into()));
    for op in &peer.take_pending_ops() {
        server.apply_remote_op(op);
    }
    let carried = alice.carry_unconfirmed_to(&mut server);
    let mut alice = server;
    assert_eq!(carried.len(), 2);
    assert_eq!(alice.name(), "pending");
    assert!(alice.body(in_flight).is_some() && alice.body(peers).is_some());
    assert_eq!(alice.body(body).expect("frame").name, "confirmed");
    assert_eq!(alice.sent_batches()[0].0, batch, "the same id goes again");

    for _ in 0..3 {
        assert!(journal.undo(&mut alice).is_some());
    }
    assert_eq!(alice.name(), "Rejoin");
    assert!(alice.body(in_flight).is_none());
    assert_eq!(alice.body(body).expect("frame").name, "Frame");
    assert!(alice.body(peers).is_some(), "a peer's edit is not undone");
}

#[test]
fn removing_a_body_takes_its_features_and_its_geometry() {
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Base".into()));
    let keep = doc.create_body(Some("Keep".into()));
    let feature = doc
        .add_feature_in_body(datum(BasePlane::XY), "Datum".into(), Some(body))
        .expect("add");
    let kept = doc
        .add_feature_in_body(datum(BasePlane::XY), "Kept".into(), Some(keep))
        .expect("add");
    doc.set_imported_geometry(body, geometry(Default::default()));

    assert!(doc.remove_body(body));
    assert!(doc.bodies().iter().all(|b| b.id != body), "body gone");
    assert!(doc.get_feature_meta(feature).is_none(), "its features too");
    assert!(doc.imported_geometry(body).is_none(), "and its geometry");
    assert!(
        doc.get_feature_meta(kept).is_some(),
        "other bodies untouched"
    );
    assert!(!doc.remove_body(body), "removing it again does nothing");
}

fn section(name: &str, uncut: core_document::BodyId) -> core_document::SavedSection {
    core_document::SavedSection {
        name: name.into(),
        normal: [0.0, 0.0, 1.0],
        base: 0.0,
        offset: 5.0,
        flipped: false,
        aim: core_document::SectionAim::Z,
        formula: Some("Plate.t / 2".into()),
        uncut: vec![uncut],
        wedge: Some(core_document::SavedWedge {
            normal: [1.0, 0.0, 0.0],
            base: 10.0,
            offset: 0.0,
            flipped: true,
        }),
    }
}

/// The rules agents keep to in a document, and its saved sections, are
/// saved with it and replay like any edit, a section saved again by its
/// name taking its place.
#[test]
fn a_document_keeps_its_agent_rules_and_saved_sections() {
    let mut doc = Document::new("rules");
    assert_eq!(doc.agent_rules(), "");
    doc.set_agent_rules("The lid stays 2 mm thick.");
    let body = doc.create_body(None);
    doc.save_section(section("Mid", body));
    doc.save_section(section("Top", body));
    let moved = core_document::SavedSection {
        offset: 8.0,
        ..section("Mid", body)
    };
    doc.save_section(moved.clone());
    assert!(!doc.remove_saved_section("Gone"));
    let ops = doc.take_pending_ops();
    assert_eq!(ops.len(), 5, "one op each, none for what changes nothing");
    let back = reloaded(&mut doc);
    assert_eq!(back.agent_rules(), "The lid stays 2 mm thick.");
    let names: Vec<&str> = back
        .saved_sections()
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(names, ["Mid", "Top"]);
    assert_eq!(back.saved_section("Mid"), Some(&moved));
}

/// The unit the measure tool reads in is saved with the document, replays
/// on a peer and is one undo step; a document saved without one reads in
/// its display unit.
#[test]
fn a_document_keeps_its_measure_unit() {
    let mut doc = Document::new("measured");
    let mut journal = core_document::history::OpJournal::new(16);
    journal.note(&mut doc);
    assert_eq!(doc.measure_unit(), None);
    doc.set_measure_unit(Some(Unit::In));
    journal.note(&mut doc);
    let ops = doc.take_pending_ops();
    assert_eq!(ops.len(), 1, "one op");

    let mut replica = Document::new("peer");
    replica.apply_remote_op(&ops[0]);
    assert_eq!(replica.measure_unit(), Some(Unit::In));

    let back = reloaded(&mut doc);
    assert_eq!(back.measure_unit(), Some(Unit::In));

    assert!(journal.undo(&mut doc).is_some());
    assert_eq!(doc.measure_unit(), None);
    assert!(journal.redo(&mut doc).is_some());
    assert_eq!(doc.measure_unit(), Some(Unit::In));

    let back = reloaded(&mut Document::new("plain"));
    assert_eq!(back.measure_unit(), None);
}

/// What is kept of a feature's reading (`read_feature`, `values_revision`,
/// `content_stamp`) follows every write of it, and a copy of the document
/// is told apart from the original.
#[test]
fn kept_readings_follow_each_write() {
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    let id = doc
        .add_feature_in_body(datum(BasePlane::XY), "Datum".into(), Some(body))
        .expect("add");
    let stamp = doc.content_stamp();
    let revision = doc.values_revision(id);
    let read = doc.read_feature::<DatumFeature>(id).expect("reads");
    assert_eq!(read.attachment, DatumAttachment::BasePlane(BasePlane::XY));
    assert_eq!(doc.content_stamp(), stamp, "reading changes nothing");
    assert_eq!(doc.values_revision(id), revision);

    let moved = serde_json::to_value(datum(BasePlane::YZ)).unwrap();
    doc.update_feature_data(id, moved).expect("update");
    assert_ne!(doc.content_stamp(), stamp);
    assert_ne!(doc.values_revision(id), revision);
    let read = doc.read_feature::<DatumFeature>(id).expect("reads");
    assert_eq!(read.attachment, DatumAttachment::BasePlane(BasePlane::YZ));

    let copy = doc.clone();
    assert_ne!(copy.content_stamp(), doc.content_stamp());
}
