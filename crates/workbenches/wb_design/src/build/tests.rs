//! The plan tests shared by every family: their helpers, and what holds
//! for a whole history (its order, the tip, suppression, rebuild flags).

use super::*;
use crate::feature::MirrorPlane;
use crate::feature::{ExtrudeMode, HelixMode, PatternAxis, PipeOrientation, TransformStep};
use crate::hole_tables::{ScrewSeat, ThreadStandard};
use crate::references::retarget_feature_sketch;
use crate::test_support::{HoleFields, bare_geometry, hole, pad, set_hole};
use kernel_api::{ExtrudeTermination, ProfileSegment, SweepKind};
use wb_sketch::sketch::{Circle, GeometryElement, Line, Point, Sketch, Vec2D};

mod bodies;
mod dressup;
mod extrude;
mod holes;
mod patterns;
mod sweeps;

fn rect_sketch() -> SketchFeature {
    crate::test_support::rect_sketch(10.0, 5.0)
}

fn pocket(sketch: FeatureId, depth: f32, reversed: bool, through_all: bool) -> DesignFeature {
    DesignFeature::Pocket {
        regions: Vec::new(),
        keep_inside: false,
        profile_borrowed: None,
        extras: Default::default(),
        refine: false,
        boolean_tolerance: None,
        sketch: Some(sketch),
        depth,
        reversed,
        symmetric: false,
        through_all,
        mode: ExtrudeMode::Dimension,
        depth2: 0.0,
        taper_deg: 0.0,
        up_to_face: None,
        up_to_offset: 0.0,
        profile_face: None,
        direction: Default::default(),
        up_to_shape: Vec::new(),
        mode2: None,
        up_to_face2: None,
        up_to_offset2: 0.0,
        up_to_shape2: Vec::new(),
    }
}

fn doc_with_body_sketch() -> (Document, BodyId, FeatureId) {
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let sketch_id = add(&mut doc, body, rect_sketch(), "sketch");
    (doc, body, sketch_id)
}

/// `feature` added to `body` as `name`.
fn add(
    doc: &mut Document,
    body: BodyId,
    feature: impl core_document::WorkbenchFeature,
    name: &str,
) -> FeatureId {
    doc.add_feature_in_body(feature, name.into(), Some(body))
        .unwrap()
}

fn extrude_distance(op: &SolidOp) -> f64 {
    match op {
        SolidOp::Sweep {
            kind:
                SweepKind::Extrude {
                    termination: ExtrudeTermination::Blind { distance },
                    ..
                },
            ..
        } => *distance,
        _ => panic!("not a blind extrude: {op:?}"),
    }
}

fn boolean_of(op: &SolidOp) -> BooleanOp {
    op.boolean_op().expect("shape-producing op")
}

/// `originals` repeated along X over 30 mm, the `skipped` occurrences
/// left out.
fn linear_pattern(originals: Vec<FeatureId>, occurrences: u32, skipped: Vec<u32>) -> DesignFeature {
    DesignFeature::LinearPattern {
        skipped,
        refine: false,
        boolean_tolerance: None,
        originals,
        axis: PatternAxis::X,
        length: 30.0,
        occurrences,
        spacing_mode: false,
        spacings: Vec::new(),
        reversed: false,
    }
}

/// A sketch of `points` in `body`.
fn points_sketch(doc: &mut Document, body: BodyId, points: &[(f64, f64)]) -> FeatureId {
    let mut sketch = Sketch::new("points");
    for &(x, y) in points {
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))));
    }
    add_sketch(doc, body, sketch)
}

fn add_sketch(doc: &mut Document, body: BodyId, sketch: Sketch) -> FeatureId {
    let plane = sketch.plane;
    let name = sketch.name.clone();
    doc.add_feature_in_body(SketchFeature::new(sketch, plane), name, Some(body))
        .unwrap()
}

/// A body whose geometry names a source file is an import, with no
/// history to rebuild; a rebuild's own result names none, and the body
/// rebuilds from its features.
#[test]
fn only_an_imported_solid_stops_a_rebuild_from_features() {
    let (mut doc, body, sketch_id) = doc_with_body_sketch();
    let pad_id = add(&mut doc, body, pad(sketch_id, 7.0), "Pad");
    doc.set_imported_geometry(body, bare_geometry(None));
    assert!(!doc.body_solid_is_imported(body));
    assert!(body_build_ops(&doc, body).is_ok(), "rebuilds normally");

    doc.set_imported_geometry(body, bare_geometry(Some(uuid::Uuid::new_v4())));
    let err = body_build_ops(&doc, body).expect_err("the import has no history to rebuild");
    assert_eq!(err.feature, Some(pad_id));
    assert!(err.message.contains("import"), "{}", err.message);
}

#[test]
fn second_pad_fuses_and_pocket_cuts() {
    let (mut doc, body, sketch_id) = doc_with_body_sketch();
    for feature in [
        pad(sketch_id, 5.0),
        pad(sketch_id, 2.0),
        pocket(sketch_id, 3.0, false, false),
    ] {
        let label = feature.kind_label().to_string();
        doc.add_feature_in_body(feature, label, Some(body)).unwrap();
    }
    let plan = body_build_ops(&doc, body).unwrap();
    assert_eq!(plan.ops.len(), 3);
    assert_eq!(plan.op_features.len(), 3);
    assert_eq!(boolean_of(&plan.ops[0]), BooleanOp::NewSolid);
    assert!((extrude_distance(&plan.ops[0]) - 5.0).abs() < 1e-9);
    assert_eq!(boolean_of(&plan.ops[1]), BooleanOp::Fuse);
    assert_eq!(boolean_of(&plan.ops[2]), BooleanOp::Cut);
    // The default pocket cuts against the sketch normal via `reversed`.
    assert!(matches!(
        &plan.ops[2],
        SolidOp::Sweep {
            kind: SweepKind::Extrude { reversed: true, .. },
            ..
        }
    ));
}

#[test]
fn pocket_first_is_an_error() {
    let (mut doc, body, sketch_id) = doc_with_body_sketch();
    add(
        &mut doc,
        body,
        pocket(sketch_id, 3.0, false, false),
        "Pocket",
    );
    let err = body_build_ops(&doc, body).unwrap_err();
    assert!(err.message.contains("material"), "{}", err.message);
}

#[test]
fn rebuild_jobs_settle_the_flags_of_the_plan_and_its_inputs_only() {
    let (mut doc, body, sketch_id) = doc_with_body_sketch();
    let pad_id = add(&mut doc, body, pad(sketch_id, 7.0), "Pad");
    // A sketch of another body, dirty on its own: not this plan's input.
    let other = doc.create_body(None);
    let other_sketch = add(&mut doc, other, rect_sketch(), "Other");
    doc.mark_feature_dirty(other_sketch);
    doc.mark_feature_dirty(sketch_id);

    let jobs = rebuild_jobs(&mut doc);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].body, body);
    assert!(jobs[0].plan.as_ref().is_ok_and(|p| !p.ops.is_empty()));
    assert!(!doc.get_feature_meta(pad_id).unwrap().dirty);
    assert!(!doc.get_feature_meta(sketch_id).unwrap().dirty);
    assert!(doc.get_feature_meta(other_sketch).unwrap().dirty);
    assert!(rebuild_jobs(&mut doc).is_empty(), "nothing comes back");
}

/// A refined feature is followed by a Refine op that answers to it,
/// and a pattern of that feature re-runs its tool, not the refine.
#[test]
fn a_refined_feature_is_followed_by_a_refine_its_pattern_skips() {
    let (mut doc, body, sketch_id) = doc_with_body_sketch();
    let mut padded = pad(sketch_id, 7.0);
    padded.set_refine(true);
    let pad_id = add(&mut doc, body, padded, "Pad");
    let plain = body_build_ops(&doc, body).unwrap();
    assert!(matches!(plain.ops.last(), Some(SolidOp::Refine)));
    assert_eq!(plain.op_features.last(), Some(&pad_id));
    assert_eq!(plain.ops.len(), plain.op_features.len());

    let pattern = linear_pattern(vec![pad_id], 2, Vec::new());
    add(&mut doc, body, pattern, "Pattern");
    let patterned = body_build_ops(&doc, body).unwrap();
    match patterned.ops.last() {
        Some(SolidOp::Transform { originals, .. }) => {
            assert_eq!(originals, &vec![0], "the pad's own op, not its refine");
        }
        other => panic!("the pattern ends the chain, not {other:?}"),
    }

    let mut unrefined = pad(sketch_id, 7.0);
    unrefined.set_refine(false);
    assert!(!unrefined.refine());
    assert!(unrefined.can_refine());
    assert!(
        !DesignFeature::Fillet {
            radius: 1.0,
            edges: crate::feature::EdgeSel::All,
            follow_tangent: false,
        }
        .can_refine()
    );
}

/// A stated boolean tolerance wraps each op the feature makes, in its
/// place, so a pattern still counts the original's op and the refine
/// after it runs as it is.
#[test]
fn a_stated_tolerance_wraps_the_feature_s_ops_in_place() {
    let (mut doc, body, sketch_id) = doc_with_body_sketch();
    add(&mut doc, body, pad(sketch_id, 7.0), "Base");
    let mut padded = pad(sketch_id, 3.0);
    padded.set_refine(true);
    padded.set_boolean_tolerance(Some(0.05));
    let pad_id = add(&mut doc, body, padded, "Pad");
    let mut pattern = linear_pattern(vec![pad_id], 2, Vec::new());
    pattern.set_boolean_tolerance(Some(0.02));
    add(&mut doc, body, pattern, "Pattern");
    let plan = body_build_ops(&doc, body).unwrap();
    assert_eq!(plan.ops.len(), plan.op_features.len());
    assert_eq!(
        plan.ops[0].boolean_tolerance(),
        None,
        "the base states none"
    );
    assert_eq!(plan.ops[1].boolean_tolerance(), Some(0.05_f32.into()));
    assert!(matches!(plan.ops[1].inner(), SolidOp::Sweep { .. }));
    assert!(matches!(plan.ops[2], SolidOp::Refine));
    assert_eq!(plan.ops[3].boolean_tolerance(), Some(0.02_f32.into()));
    match plan.ops[3].inner() {
        SolidOp::Transform { originals, .. } => assert_eq!(originals, &vec![1]),
        other => panic!("the pattern ends the chain, not {other:?}"),
    }
}

#[test]
fn deleting_a_feature_reveals_its_sketch_and_restarts_the_body() {
    let (mut doc, body, sketch_id) = doc_with_body_sketch();
    let pad_id = add(&mut doc, body, pad(sketch_id, 7.0), "Pad");
    let pocket_id = add(
        &mut doc,
        body,
        pocket(sketch_id, 2.0, false, false),
        "Pocket",
    );
    doc.set_feature_visible(sketch_id, false);
    doc.clear_feature_dirty(pad_id);
    doc.clear_feature_dirty(pocket_id);

    assert!(delete_feature(&mut doc, pocket_id));
    assert!(doc.get_feature_meta(pocket_id).is_none());
    assert!(doc.get_feature_meta(sketch_id).unwrap().visible);
    assert!(
        doc.get_feature_meta(pad_id).unwrap().dirty,
        "the body restarts"
    );
    assert!(!delete_feature(&mut doc, pocket_id), "gone already");
}

#[test]
fn invalidating_a_body_restarts_its_history_and_leaves_clearing_to_the_registry() {
    let (mut doc, body, sketch_id) = doc_with_body_sketch();
    let pad_id = add(&mut doc, body, pad(sketch_id, 7.0), "Pad");
    doc.clear_feature_dirty(pad_id);
    invalidate_body(&mut doc, body);
    assert!(doc.get_feature_meta(pad_id).unwrap().dirty);

    doc.remove_feature(pad_id).unwrap();
    doc.set_imported_geometry(body, bare_geometry(None));
    invalidate_body(&mut doc, body);
    assert!(
        doc.imported_geometry(body).is_some(),
        "another bench's features may build it: the registry clears it"
    );
}

#[test]
fn dirty_pad_flags_body_for_rebuild_and_sketch_edit_propagates() {
    let (mut doc, body, sketch_id) = doc_with_body_sketch();
    let pad_id = add(&mut doc, body, pad(sketch_id, 7.0), "Pad");
    assert!(pending_body_rebuilds(&doc).is_empty());

    doc.mark_feature_dirty(sketch_id);
    assert_eq!(pending_body_rebuilds(&doc), vec![body]);

    doc.clear_feature_dirty(pad_id);
    doc.clear_feature_dirty(sketch_id);
    assert!(pending_body_rebuilds(&doc).is_empty());
}

/// A suppressed feature and the features after the body's tip are left
/// out of the build.
#[test]
fn suppressed_features_and_those_after_the_tip_are_skipped_in_build() {
    let (mut doc, body, sketch_id) = doc_with_body_sketch();
    let pad_a = add(&mut doc, body, pad(sketch_id, 5.0), "PadA");
    add(&mut doc, body, pad(sketch_id, 9.0), "PadB");

    assert_eq!(body_build_ops(&doc, body).unwrap().ops.len(), 2);
    doc.set_feature_suppressed(pad_a, true);
    let plan = body_build_ops(&doc, body).unwrap();
    assert_eq!(plan.ops.len(), 1);
    // The remaining pad becomes the first op → NewSolid.
    assert_eq!(boolean_of(&plan.ops[0]), BooleanOp::NewSolid);
    assert!((extrude_distance(&plan.ops[0]) - 9.0).abs() < 1e-9);

    doc.set_feature_suppressed(pad_a, false);
    doc.set_body_tip(body, Some(pad_a));
    let plan = body_build_ops(&doc, body).unwrap();
    assert_eq!(plan.ops.len(), 1, "features after the tip are excluded");
    assert!((extrude_distance(&plan.ops[0]) - 5.0).abs() < 1e-9);
    doc.set_body_tip(body, None);
    assert_eq!(body_build_ops(&doc, body).unwrap().ops.len(), 2);
}

#[test]
fn history_reorder_swaps_build_order_and_respects_dependencies() {
    let (mut doc, body, sketch_id) = doc_with_body_sketch();
    let pad_a = add(&mut doc, body, pad(sketch_id, 5.0), "PadA");
    let pad_b = add(&mut doc, body, pad(sketch_id, 9.0), "PadB");

    assert!(doc.move_feature_in_history(pad_b, true));
    let plan = body_build_ops(&doc, body).unwrap();
    assert!(
        (extrude_distance(&plan.ops[0]) - 9.0).abs() < 1e-9,
        "B first"
    );
    assert!((extrude_distance(&plan.ops[1]) - 5.0).abs() < 1e-9);

    // A pattern must not move before its original.
    let pattern = doc
        .add_feature_in_body(
            DesignFeature::Mirrored {
                skipped: Vec::new(),
                refine: false,
                boolean_tolerance: None,
                originals: vec![pad_a],
                plane: MirrorPlane::YZ,
            },
            "Mirror".into(),
            Some(body),
        )
        .unwrap();
    assert!(
        !doc.move_feature_in_history(pattern, true),
        "moving the mirror above its original is blocked"
    );
    // But the original may not move below its dependent either.
    assert!(!doc.move_feature_in_history(pad_a, false));
}

/// A move is one place in the body's whole history: the Pocket cannot
/// pass its own sketch, and the Pad never jumps past a sketch it does
/// not use.
#[test]
fn a_move_is_one_place_in_the_whole_history() {
    let (mut doc, body, base) = doc_with_body_sketch();
    let pad_id = add(&mut doc, body, pad(base, 5.0), "Pad");
    let cut_sketch = add(&mut doc, body, rect_sketch(), "cut");
    let pocket = add(
        &mut doc,
        body,
        pocket(cut_sketch, 2.0, false, false),
        "Pocket",
    );
    let order = |doc: &Document| {
        let mut nodes: Vec<(u64, FeatureId)> = doc
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.body == Some(body))
            .map(|(id, n)| (n.seq, *id))
            .collect();
        nodes.sort();
        nodes.into_iter().map(|(_, id)| id).collect::<Vec<_>>()
    };
    assert_eq!(
        doc.try_move_feature_in_history(pocket, true),
        Err(core_document::MoveRefused::Dependency {
            neighbour: cut_sketch
        }),
        "the pocket stays after its sketch, and names it"
    );
    assert_eq!(order(&doc), vec![base, pad_id, cut_sketch, pocket]);
    assert!(doc.move_feature_in_history(cut_sketch, true));
    assert_eq!(order(&doc), vec![base, cut_sketch, pad_id, pocket]);
    // The pocket does not use the pad, so the pad may move past it.
    assert!(doc.move_feature_in_history(pad_id, false));
    assert_eq!(order(&doc), vec![base, cut_sketch, pocket, pad_id]);
}

#[test]
fn retarget_moves_dependency_and_visibility() {
    let (mut doc, body, sketch_a) = doc_with_body_sketch();
    let sketch_b = add(&mut doc, body, rect_sketch(), "sketch_b");
    let pad_id = add(&mut doc, body, pad(sketch_a, 5.0), "Pad");
    doc.set_feature_visible(sketch_a, false);
    doc.clear_feature_dirty(pad_id);

    retarget_feature_sketch(&mut doc, pad_id, sketch_b).unwrap();

    assert_eq!(doc.feature_tree().dependencies(pad_id), vec![sketch_b]);
    assert!(doc.get_feature_meta(sketch_a).unwrap().visible);
    assert!(!doc.get_feature_meta(sketch_b).unwrap().visible);
    assert!(doc.get_feature_meta(pad_id).unwrap().dirty);
}

/// A feature is offered the sketches before it in history, and the
/// one it already has even when that one comes later.
#[test]
fn a_feature_is_offered_the_sketches_before_it() {
    let (mut doc, body, before) = doc_with_body_sketch();
    let pad_id = add(&mut doc, body, pad(before, 5.0), "Pad");
    let after = add(&mut doc, body, rect_sketch(), "after");
    let ids = |current| {
        sketch_choices(&doc, body, pad_id, current)
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(Some(before)), vec![before]);
    assert_eq!(ids(None), vec![before]);
    assert_eq!(ids(Some(after)), vec![before, after]);
}

#[test]
fn sketches_of_body_lists_in_creation_order() {
    let (mut doc, body, first) = doc_with_body_sketch();
    let second = add(&mut doc, body, rect_sketch(), "second");
    let list = sketches_of_body(&doc, body);
    assert_eq!(
        list.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![first, second]
    );
}

#[test]
fn an_open_profile_fails_on_its_feature() {
    let (mut doc, body, _) = doc_with_body_sketch();
    let mut sketch = Sketch::new("open");
    let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
    let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, 0.0))));
    sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
    let open_id = add_sketch(&mut doc, body, sketch);
    let bad = add(&mut doc, body, pad(open_id, 7.0), "BadPad");
    let err = body_build_ops(&doc, body).unwrap_err();
    assert_eq!(err.feature, Some(bad));
    // The feature names itself wherever the error shows.
    assert!(!err.message.contains("BadPad"), "{}", err.message);
}
