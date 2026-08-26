//! The op stream is the document: replaying a session's captured ops onto a
//! baseline snapshot must land on the same replicated state the live edits
//! produced. This is the property every future sync transport rests on.

use core_document::datum::{DatumAttachment, DatumFeature, DatumShape};
use core_document::units::Unit;
use core_document::{BasePlane, Document};

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
    doc.rename_feature(d2, "Datum 2 (moved)");
    doc.set_feature_suppressed(d3, true);
    doc.set_feature_visible(d1, false);
    doc.set_feature_dependencies(d3, vec![d1]);
    assert!(doc.move_feature_in_history(d2, true), "swap d1/d2");
    doc.remove_feature(d1).expect("remove");
    doc.set_body_tip(body, Some(d2));
    doc.set_display_unit(Unit::In);
    doc.set_name("Replayed");
}

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

    let mut replica = baseline;
    for op in &ops {
        replica.apply_op(op);
    }

    assert_eq!(
        live.replicated_projection(),
        replica.replicated_projection(),
        "replay must converge on the live document's replicated state"
    );
}

/// The op stream survives a serde round-trip unchanged — what the wire and
/// the on-disk op log will do to it.
#[test]
fn ops_replay_identically_after_a_serde_round_trip() {
    let mut live = Document::new("Wire");
    let baseline = live.clone();
    let _ = live.take_pending_ops();

    scripted_session(&mut live);
    let ops = live.take_pending_ops();

    let json = serde_json::to_string(&ops).expect("ops serialize");
    let decoded: Vec<core_document::op::DocumentOp> =
        serde_json::from_str(&json).expect("ops deserialize");

    let mut replica = baseline;
    for op in &decoded {
        replica.apply_op(op);
    }
    assert_eq!(
        live.replicated_projection(),
        replica.replicated_projection()
    );
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
