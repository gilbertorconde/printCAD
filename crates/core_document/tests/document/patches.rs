//! Patches of feature data: recorded, replayed, undone and ordered by the
//! server as any op is, a patch that no longer fits its data refused or
//! passed over without touching it.

use core_document::history::OpJournal;
use core_document::op::DocumentOp;
use core_document::patch::{DataPatch, PathStep};
use core_document::{Document, FeatureId};
use serde_json::{Value, json};

use crate::support::assert_replays;

fn items() -> Vec<PathStep> {
    vec![PathStep::key("items")]
}

fn item(id: &str) -> Vec<PathStep> {
    vec![PathStep::key("items"), PathStep::item(id)]
}

fn put(after: Option<&str>, id: &str, x: i64) -> DataPatch {
    let mut patch = DataPatch::new();
    patch.insert(items(), after.map(str::to_owned), json!({"id": id, "x": x}));
    patch
}

fn moved(id: &str, x: i64) -> DataPatch {
    let mut patch = DataPatch::new();
    patch.set(item(id), json!({"id": id, "x": x}));
    patch
}

fn taken(id: &str) -> DataPatch {
    let mut patch = DataPatch::new();
    patch.remove(item(id));
    patch
}

/// A document holding one feature whose data is a list of items with ids,
/// its capture window clean.
fn listed() -> (Document, FeatureId) {
    let mut doc = Document::new("Patched");
    let id = doc.add_feature_of_kind(
        core_document::WorkbenchId::new("test.list"),
        "List".into(),
        None,
        Vec::new(),
        json!({"name": "list", "items": [{"id": "a", "x": 0}, {"id": "b", "x": 1}]}),
        Default::default(),
    );
    let _ = doc.take_pending_ops();
    doc.confirm_sent_ops();
    (doc, id)
}

fn data(doc: &Document, id: FeatureId) -> Value {
    Value::clone(&doc.get_feature_meta(id).expect("the feature").data)
}

fn ids(doc: &Document, id: FeatureId) -> Vec<String> {
    data(doc, id)["items"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|i| i["id"].as_str().expect("an id").to_owned())
        .collect()
}

/// Patches go out as their own op, a drag's frames joined into one, and
/// replayed on the document from before (as they are, and through the
/// JSON of the wire and the op log) land where the live edits did.
#[test]
fn recorded_patches_replay_to_the_live_data() {
    let (mut doc, id) = listed();
    let baseline = doc.clone();
    doc.patch_feature_data(id, put(Some("b"), "c", 2))
        .expect("append");
    doc.patch_feature_data(id, put(None, "d", 3))
        .expect("insert first");
    doc.patch_feature_data(id, taken("a")).expect("remove");
    for x in 0..30 {
        doc.patch_feature_data(id, moved("b", x)).expect("drag");
    }
    let mut renamed = DataPatch::new();
    renamed.set(vec![PathStep::key("name")], json!("renamed"));
    doc.patch_feature_data(id, renamed).expect("a field");
    assert_eq!(ids(&doc, id), ["d", "b", "c"]);

    let ops = doc.take_pending_ops();
    let [DocumentOp::PatchFeatureData { patch, .. }] = ops.as_slice() else {
        panic!("one joined patch expected, got {ops:?}");
    };
    assert_eq!(patch.len(), 5, "the drag's thirty frames are one write");
    let text = serde_json::to_string(&ops[0]).expect("writes");
    assert!(text.starts_with(r#"{"PatchFeatureData":{"id":"#), "{text}");
    assert_replays(&doc, baseline, &ops);
}

/// Each patch closed as a step undoes to the data before it and redoes
/// to the data after it; patches within one step undo together, to where
/// the step began.
#[test]
fn patches_undo_and_redo_step_by_step() {
    let (mut doc, id) = listed();
    let mut journal = OpJournal::new(16);
    journal.note(&mut doc);
    let mut states = vec![data(&doc, id)];
    for patch in [put(Some("b"), "c", 2), moved("a", 7), taken("b")] {
        doc.patch_feature_data(id, patch).expect("applies");
        journal.note(&mut doc);
        states.push(data(&doc, id));
    }
    for x in 0..5 {
        doc.patch_feature_data(id, moved("c", x)).expect("drag");
    }
    doc.patch_feature_data(id, put(None, "e", 9))
        .expect("in the same step");
    journal.note(&mut doc);
    states.push(data(&doc, id));

    for state in states.iter().rev().skip(1) {
        journal.undo(&mut doc).expect("a step to undo");
        assert_eq!(&data(&doc, id), state);
    }
    for state in states.iter().skip(1) {
        journal.redo(&mut doc).expect("a step to redo");
        assert_eq!(&data(&doc, id), state);
    }
}

/// Two editors patching one list at once converge on the server's order:
/// each one's unconfirmed patch rolls back beneath the other's and goes
/// on again, and a patch whose item the earlier one took away passes
/// over without touching the data.
#[test]
fn concurrent_patches_converge_in_the_servers_order() {
    let (mut alice, id) = listed();
    let mut bob = alice.clone();

    alice
        .patch_feature_data(id, put(Some("b"), "x", 5))
        .expect("alice adds");
    bob.patch_feature_data(id, put(Some("b"), "y", 6))
        .expect("bob adds");
    let from_alice = alice.take_pending_ops();
    let from_bob = bob.take_pending_ops();
    // The server orders Alice's first.
    alice.confirm_sent_ops();
    assert!(alice.apply_server_ops(&from_bob));
    assert!(bob.apply_server_ops(&from_alice));
    bob.confirm_sent_ops();
    assert_eq!(ids(&alice, id), ["a", "b", "y", "x"]);
    assert_eq!(alice.replicated_projection(), bob.replicated_projection());

    // Alice takes `a` away while Bob moves it; hers is ordered first.
    alice
        .patch_feature_data(id, taken("a"))
        .expect("alice removes");
    bob.patch_feature_data(id, moved("a", 40))
        .expect("bob moves");
    let from_alice = alice.take_pending_ops();
    let from_bob = bob.take_pending_ops();
    alice.confirm_sent_ops();
    assert!(alice.apply_server_ops(&from_bob));
    assert!(bob.apply_server_ops(&from_alice));
    bob.confirm_sent_ops();
    assert_eq!(ids(&bob, id), ["b", "y", "x"]);
    assert_eq!(alice.replicated_projection(), bob.replicated_projection());

    // Bob moves `x`, not yet sent, while Alice's removal of it arrives:
    // his move is dropped, and the edit he makes next goes out alone and
    // lands on Alice's copy too.
    bob.patch_feature_data(id, moved("x", 50))
        .expect("bob moves");
    alice
        .patch_feature_data(id, taken("x"))
        .expect("alice removes");
    let from_alice = alice.take_pending_ops();
    alice.confirm_sent_ops();
    assert!(bob.apply_server_ops(&from_alice));
    bob.patch_feature_data(id, moved("y", 60))
        .expect("bob moves another");
    let from_bob = bob.take_pending_ops();
    bob.confirm_sent_ops();
    assert!(alice.apply_server_ops(&from_bob));
    assert_eq!(data(&alice, id)["items"][1], json!({"id": "y", "x": 60}));
    assert_eq!(alice.replicated_projection(), bob.replicated_projection());
}

/// A patch that does not fit the data (an item it names is missing) is
/// refused whole: nothing is recorded and the data is as it was.
#[test]
fn a_patch_that_does_not_fit_is_refused_whole() {
    let (mut doc, id) = listed();
    let before = data(&doc, id);
    let mut patch = put(Some("b"), "c", 2);
    patch.then(moved("missing", 1));
    assert!(doc.patch_feature_data(id, patch).is_err());
    assert_eq!(data(&doc, id), before);
    assert!(doc.take_pending_ops().is_empty(), "nothing recorded");
    assert!(
        doc.patch_feature_data(FeatureId::new(), moved("a", 1))
            .is_err()
    );
}
