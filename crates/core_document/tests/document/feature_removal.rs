//! Deleting a feature and undoing it puts the feature back as it was: the
//! features that read it read it again, in the place they read it, and it
//! keeps its visibility and suppression. The ops that do it replay.

use core_document::history::OpJournal;
use core_document::op::DocumentOp;
use core_document::{Document, FeatureId, FeatureOrigin, WorkbenchId};

use crate::support::assert_replays;

fn add(doc: &mut Document, name: &str, deps: Vec<FeatureId>) -> FeatureId {
    doc.add_feature_of_kind(
        WorkbenchId::new("test.thing"),
        name.into(),
        None,
        deps,
        serde_json::json!({ "name": name }),
        FeatureOrigin::default(),
    )
}

/// The replicated state with the lists the tree keeps as sets (its roots,
/// what reads each feature) sorted: the order those are kept in means
/// nothing, the order of what each feature reads does.
fn graph(doc: &Document) -> serde_json::Value {
    let mut value = doc.replicated_projection();
    let tree = &mut value["feature_tree"];
    let sort = |list: &mut serde_json::Value| {
        if let Some(items) = list.as_array_mut() {
            items.sort_by_key(|item| item.to_string());
        }
    };
    sort(&mut tree["roots"]);
    if let Some(map) = tree["dependents"].as_object_mut() {
        map.values_mut().for_each(sort);
    }
    value
}

fn clear_dirty(doc: &mut Document) {
    let ids: Vec<FeatureId> = doc.feature_tree().all_nodes().map(|(id, _)| *id).collect();
    for id in ids {
        doc.clear_feature_dirty(id);
    }
}

#[test]
fn undoing_a_delete_puts_back_what_read_it() {
    let mut doc = Document::new("t");
    let mut journal = OpJournal::new(50);
    let datum = add(&mut doc, "datum", vec![]);
    let sketch = add(&mut doc, "sketch", vec![datum]);
    let other = add(&mut doc, "other", vec![]);
    // The pad reads the sketch first, so its place among the pad's inputs
    // is something to put back.
    let pad = add(&mut doc, "pad", vec![sketch, other]);
    let fillet = add(&mut doc, "fillet", vec![pad, sketch]);
    journal.note(&mut doc);
    let before = graph(&doc);
    let baseline = doc.clone();
    doc.take_pending_ops();

    doc.remove_feature(sketch).unwrap();
    journal.note(&mut doc);
    assert_eq!(doc.feature_tree().dependencies(pad), [other]);

    journal.undo(&mut doc).expect("the delete undoes");
    let tree = doc.feature_tree();
    assert_eq!(tree.dependencies(sketch), [datum]);
    assert_eq!(tree.dependencies(pad), [sketch, other]);
    assert_eq!(tree.dependencies(fillet), [pad, sketch]);
    assert_eq!(
        graph(&doc),
        before,
        "the document is as the delete found it"
    );

    // An edit to the sketch reaches the pad and what follows it again.
    clear_dirty(&mut doc);
    doc.mark_feature_dirty(sketch);
    for id in [pad, fillet] {
        assert!(doc.get_feature_meta(id).unwrap().dirty);
    }
    assert!(!doc.get_feature_meta(other).unwrap().dirty);

    // Redo deletes it again, and a peer hearing the delete and its undo
    // lands where this document does.
    journal.redo(&mut doc).expect("the delete redoes");
    assert_eq!(doc.feature_tree().dependencies(pad), [other]);
    journal.undo(&mut doc).expect("and undoes again");
    let ops = doc.take_pending_ops();
    assert!(ops.len() >= 4, "delete, undo, redo, undo: {ops:?}");
    assert_replays(&doc, baseline, &ops);
}

#[test]
fn a_hidden_suppressed_feature_comes_back_hidden_and_suppressed() {
    let mut doc = Document::new("t");
    let mut journal = OpJournal::new(50);
    let sketch = add(&mut doc, "sketch", vec![]);
    let pad = add(&mut doc, "pad", vec![sketch]);
    doc.set_feature_visible(pad, false);
    doc.set_feature_suppressed(pad, true);
    journal.note(&mut doc);
    let before = graph(&doc);
    let baseline = doc.clone();
    doc.take_pending_ops();

    doc.remove_feature(pad).unwrap();
    journal.note(&mut doc);
    journal.undo(&mut doc).expect("the delete undoes");
    let node = doc.get_feature_meta(pad).unwrap();
    assert!(!node.visible && node.suppressed);
    assert_eq!(graph(&doc), before);
    let ops = doc.take_pending_ops();
    assert_replays(&doc, baseline, &ops);
}

/// Deleting what a feature reads and then the feature, in one step, undoes
/// to both and the edge between them, whichever order they went in.
#[test]
fn deleting_a_feature_and_what_it_reads_together_undoes_whole() {
    for reader_first in [false, true] {
        let mut doc = Document::new("t");
        let mut journal = OpJournal::new(50);
        let sketch = add(&mut doc, "sketch", vec![]);
        let pad = add(&mut doc, "pad", vec![sketch]);
        journal.note(&mut doc);
        let before = graph(&doc);

        let order = if reader_first {
            [pad, sketch]
        } else {
            [sketch, pad]
        };
        for id in order {
            doc.remove_feature(id).unwrap();
        }
        journal.note(&mut doc);
        assert!(doc.feature_tree().all_nodes().next().is_none());
        assert!(doc.feature_tree().roots().is_empty());

        journal.undo(&mut doc).expect("both come back");
        assert_eq!(doc.feature_tree().dependencies(pad), [sketch]);
        assert_eq!(graph(&doc), before, "reader first: {reader_first}");
    }
}

/// An op log written before the op carried the feature's readers and
/// flags still replays, the feature shown and active.
#[test]
fn an_old_add_feature_op_still_reads() {
    let id = FeatureId::new();
    let json = serde_json::json!({
        "AddFeature": {
            "id": id,
            "workbench_id": "test.thing",
            "name": "old",
            "body": null,
            "deps": [],
            "data": {},
            "seq": 0,
            "created_at": 0
        }
    });
    let op: DocumentOp = serde_json::from_value(json).expect("an old op reads");
    let mut doc = Document::new("t");
    doc.apply_op(&op);
    let node = doc.get_feature_meta(id).unwrap();
    assert!(node.visible && !node.suppressed);
}
