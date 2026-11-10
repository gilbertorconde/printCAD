//! A body's tip is the point in its history on show: a new feature goes in
//! right after it, the tip follows the new feature, and moving the tip on
//! its own is navigation, not an edit to undo.

use core_document::datum::{DatumAttachment, DatumFeature, DatumShape};
use core_document::history::OpJournal;
use core_document::{BasePlane, BodyId, Document, FeatureId};

fn datum() -> DatumFeature {
    DatumFeature {
        shape: DatumShape::Plane { size: 10.0 },
        attachment: DatumAttachment::BasePlane(BasePlane::XY),
        offset: Default::default(),
    }
}

/// The body's features, in history order.
fn history(doc: &Document, body: BodyId) -> Vec<FeatureId> {
    let mut all: Vec<(u64, FeatureId)> = doc
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.body == Some(body))
        .map(|(id, n)| (n.seq, *id))
        .collect();
    all.sort();
    all.into_iter().map(|(_, id)| id).collect()
}

fn tip(doc: &Document, body: BodyId) -> Option<FeatureId> {
    doc.body(body).and_then(|b| b.tip)
}

fn add(doc: &mut Document, body: BodyId, name: &str) -> FeatureId {
    doc.add_feature_in_body(datum(), name.into(), Some(body))
        .unwrap()
}

#[test]
fn features_added_at_an_earlier_point_go_in_there_and_the_tip_follows() {
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    let a = add(&mut doc, body, "a");
    let b = add(&mut doc, body, "b");
    let c = add(&mut doc, body, "c");

    // At the end of history nothing moves.
    assert_eq!(history(&doc, body), [a, b, c]);
    assert_eq!(tip(&doc, body), None);

    // Back at `a`: two features go in after it, in the order made.
    doc.set_body_tip(body, Some(a));
    let x = add(&mut doc, body, "x");
    assert_eq!(history(&doc, body), [a, x, b, c]);
    assert_eq!(tip(&doc, body), Some(x));
    let y = add(&mut doc, body, "y");
    assert_eq!(history(&doc, body), [a, x, y, b, c]);
    assert_eq!(tip(&doc, body), Some(y));

    // Another body's history is not touched.
    let other = doc.create_body(None);
    let z = add(&mut doc, other, "z");
    assert_eq!(history(&doc, other), [z]);
    assert_eq!(history(&doc, body), [a, x, y, b, c]);
}

#[test]
fn moving_the_tip_alone_is_no_undo_step_and_an_insert_undoes_whole() {
    let mut doc = Document::new("t");
    let mut journal = OpJournal::new(50);
    let body = doc.create_body(None);
    let a = add(&mut doc, body, "a");
    let b = add(&mut doc, body, "b");
    journal.note(&mut doc);

    // Navigation: the tip moves back to `a`, and undo still takes back
    // the adds, the one step there is.
    doc.set_body_tip(body, Some(a));
    journal.note(&mut doc);
    journal.undo(&mut doc).expect("the adds");
    assert!(doc.get_feature_meta(a).is_none() && doc.get_feature_meta(b).is_none());
    assert!(!journal.can_undo(), "the tip move made no step of its own");
    journal.redo(&mut doc);
    doc.set_body_tip(body, Some(a));
    journal.note(&mut doc);

    // An insert at the tip and the tip moving with it are one step.
    let x = add(&mut doc, body, "x");
    journal.note(&mut doc);
    assert_eq!(history(&doc, body), [a, x, b]);
    journal.undo(&mut doc).expect("the insert");
    assert!(doc.get_feature_meta(x).is_none());
    assert_eq!(tip(&doc, body), Some(a), "the tip goes back with it");
    assert_eq!(history(&doc, body), [a, b]);
}

#[test]
fn a_feature_moves_to_another_body_with_what_only_it_uses() {
    let mut doc = Document::new("t");
    let mut journal = OpJournal::new(50);
    let from = doc.create_body(None);
    let to = doc.create_body(None);
    let a = add(&mut doc, from, "a");
    let sketch = add(&mut doc, from, "sketch");
    let pad = add(&mut doc, from, "pad");
    doc.set_feature_dependencies(pad, vec![sketch]);
    let t1 = add(&mut doc, to, "t1");
    let t2 = add(&mut doc, to, "t2");
    doc.set_body_tip(to, Some(t1));
    doc.set_body_tip(from, Some(pad));
    journal.note(&mut doc);

    let moved = doc.move_feature_to_body(pad, to).expect("moves");
    journal.note(&mut doc);
    assert_eq!(moved, [sketch, pad], "its sketch goes with it, first");
    assert_eq!(history(&doc, from), [a]);
    assert_eq!(tip(&doc, from), Some(a), "the tip steps back");
    assert_eq!(history(&doc, to), [t1, sketch, pad, t2], "in at the tip");
    assert_eq!(tip(&doc, to), Some(pad));
    assert!(doc.feature_tree().get_node(t2).unwrap().dirty);

    journal.undo(&mut doc).expect("the move");
    assert_eq!(history(&doc, from), [a, sketch, pad]);
    assert_eq!(history(&doc, to), [t1, t2]);
    assert_eq!(tip(&doc, from), Some(pad));
}

#[test]
fn a_move_that_would_split_what_is_shared_is_refused() {
    let mut doc = Document::new("t");
    let from = doc.create_body(None);
    let to = doc.create_body(None);
    let sketch = add(&mut doc, from, "sketch");
    let pad = add(&mut doc, from, "pad");
    let pocket = add(&mut doc, from, "pocket");
    doc.set_feature_dependencies(pad, vec![sketch]);
    doc.set_feature_dependencies(pocket, vec![sketch]);
    assert!(doc.move_feature_to_body(pad, to).is_err(), "shared sketch");
    assert!(
        doc.move_feature_to_body(sketch, to).is_err(),
        "used by others"
    );
    assert!(doc.move_feature_to_body(pad, from).is_err(), "same body");
    assert_eq!(history(&doc, from), [sketch, pad, pocket]);
}

/// A body as it was made comes back on undo once removed, so taking away
/// the empty body a cancelled tool made leaves undo working; one that
/// carried a feature cannot come back, and its removal clears history.
#[test]
fn removing_a_bare_body_undoes_and_a_used_one_is_a_barrier() {
    let mut doc = Document::new("t");
    let mut journal = OpJournal::new(16);
    let kept = doc.create_body(Some("Kept".into()));
    journal.note(&mut doc);
    let bare = doc.create_body(Some("Bare".into()));
    journal.note(&mut doc);

    assert!(doc.remove_body(bare));
    journal.note(&mut doc);
    assert!(journal.can_undo(), "a bare body's removal is not a barrier");
    journal.undo(&mut doc);
    let back = doc.body(bare).expect("it comes back");
    assert_eq!(back.name, "Bare");
    journal.undo(&mut doc);
    journal.undo(&mut doc);
    assert!(
        doc.bodies().is_empty(),
        "the history before it still undoes"
    );
    journal.redo(&mut doc);
    journal.redo(&mut doc);
    journal.redo(&mut doc);
    assert!(doc.body(bare).is_none());

    // A cancelled tool takes its feature away, then the body it made.
    let made = doc.create_body(Some("Made".into()));
    let feature = add(&mut doc, made, "Plane");
    doc.remove_feature(feature).unwrap();
    assert!(doc.remove_body(made));
    journal.note(&mut doc);
    assert!(
        journal.can_undo(),
        "the body is bare once its feature is gone"
    );

    add(&mut doc, kept, "Plane");
    journal.note(&mut doc);
    assert!(doc.remove_body(kept));
    journal.note(&mut doc);
    assert!(
        !journal.can_undo(),
        "a body with a feature on it cannot come back"
    );
}

#[test]
fn a_move_refused_partway_leaves_the_history_as_it_was_and_a_whole_move_undoes_whole() {
    let mut doc = Document::new("t");
    let mut journal = OpJournal::new(50);
    let body = doc.create_body(None);
    let a = add(&mut doc, body, "a");
    let b = add(&mut doc, body, "b");
    let c = add(&mut doc, body, "c");
    let d = add(&mut doc, body, "d");
    doc.set_feature_dependencies(d, vec![b]);
    journal.note(&mut doc);

    // To the front, `d` would pass `c` and then `b`, which it uses.
    assert!(doc.move_feature_after(d, None).is_err());
    assert_eq!(history(&doc, body), [a, b, c, d], "nothing moved");

    // As far as it may go: right after `b`.
    doc.move_feature_after(d, Some(b)).unwrap();
    journal.note(&mut doc);
    assert_eq!(history(&doc, body), [a, b, d, c]);
    // Three places on, in one step to undo.
    doc.move_feature_after(a, Some(c)).unwrap();
    journal.note(&mut doc);
    assert_eq!(history(&doc, body), [b, d, c, a]);
    journal.undo(&mut doc).expect("the second move");
    assert_eq!(history(&doc, body), [a, b, d, c]);
}
