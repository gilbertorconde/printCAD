//! A feature's stated boolean tolerance: faces that nearly meet are
//! joined by its fuse or cut rather than left with a gap or a sliver.

use core_document::{BodyId, Document, WorkbenchFeature};
use kernel_api::{BooleanOp, Placement, PrimitiveKind, SolidBuildResult, SolidOp};
use wb_design::DesignFeature;
use wb_sketch::sketch::SketchPlane;

use super::common::{Features, chain, count_shapes, pad, rect_at, volume};

/// A body holding a 10 mm cube padded from the XY plane.
fn cube_body() -> (Document, BodyId) {
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let base = doc.add_in(
        body,
        "base",
        rect_at(
            SketchPlane {
                origin: [0.0; 3],
                ..SketchPlane::xy()
            },
            0.0,
            0.0,
            10.0,
            10.0,
        ),
    );
    doc.add_in(body, "Pad", pad(base, 10.0));
    (doc, body)
}

/// The cube, and a second cube padded from a sketch at height `z`, its
/// fuse at `tolerance`.
fn stacked(z: f64, tolerance: Option<f32>) -> (Document, BodyId) {
    let (mut doc, body) = cube_body();
    let upper = doc.add_in(
        body,
        "upper",
        rect_at(
            SketchPlane {
                origin: [0.0, 0.0, z],
                ..SketchPlane::xy()
            },
            0.0,
            0.0,
            10.0,
            10.0,
        ),
    );
    let mut second = pad(upper, 10.0);
    second.set_boolean_tolerance(tolerance);
    doc.add_in(body, "Pad001", second);
    (doc, body)
}

fn build(doc: &Document, body: BodyId) -> SolidBuildResult {
    let ops = wb_design::body_build_ops(doc, body).unwrap().ops;
    chain(&ops)
}

/// The solids and faces of a build's shape.
fn solids_and_faces(built: &SolidBuildResult) -> (usize, usize) {
    use ogeom::topo::ShapeType;
    let count = |kind| count_shapes(&built.brep_blob, kind);
    (count(ShapeType::Solid), count(ShapeType::Face))
}

/// A pad 5 µm above the cube under it stays a second solid with the gap
/// between them; at a tolerance of 10 µm the two are one solid.
#[test]
fn a_stated_tolerance_closes_a_gap_a_pad_leaves() {
    let (doc, body) = stacked(10.005, None);
    assert_eq!(
        solids_and_faces(&build(&doc, body)),
        (2, 12),
        "two cubes with a gap"
    );

    let (doc, body) = stacked(10.005, Some(0.01));
    let joined = build(&doc, body);
    assert_eq!(solids_and_faces(&joined).0, 1, "one solid across the gap");
    assert!(
        (volume(&joined.brep_blob) - 2000.0).abs() < 1.0,
        "both cubes' material: {}",
        volume(&joined.brep_blob)
    );
}

/// A pad sunk 5 µm into the cube under it fuses with a sliver of side
/// faces around the seam; at a tolerance of 10 µm the seam leaves none.
#[test]
fn a_stated_tolerance_leaves_no_sliver_at_a_near_flush_seam() {
    let (doc, body) = stacked(9.995, None);
    let (solids, sliver) = solids_and_faces(&build(&doc, body));
    assert_eq!(solids, 1);

    let (doc, body) = stacked(9.995, Some(0.01));
    let (solids, clean) = solids_and_faces(&build(&doc, body));
    assert_eq!(solids, 1);
    assert!(
        clean < sliver,
        "the sliver's faces are gone: {clean} faces against {sliver}"
    );
    assert_eq!(clean, 10, "each side split once at the seam, no strip");
}

/// A pocket stopping 5 µm short of the floor leaves a skin; at a
/// tolerance of 10 µm it cuts through.
#[test]
fn a_stated_tolerance_cuts_through_a_skin_a_pocket_leaves() {
    let pocketed = |tolerance: Option<f32>| {
        let (mut doc, body) = cube_body();
        let top = doc.add_in(
            body,
            "top",
            rect_at(
                SketchPlane {
                    origin: [3.0, 3.0, 10.0],
                    ..SketchPlane::xy()
                },
                0.0,
                0.0,
                4.0,
                4.0,
            ),
        );
        let mut pocket = DesignFeature::from_json(&serde_json::json!({
            "Pocket": { "sketch": top, "depth": 9.995, "reversed": false }
        }))
        .unwrap();
        pocket.set_boolean_tolerance(tolerance);
        doc.add_in(body, "Pocket", pocket);
        volume(&build(&doc, body).brep_blob)
    };
    let hole = 4.0 * 4.0 * 10.0;
    let kept = pocketed(None);
    assert!(
        (kept - (1000.0 - hole + 16.0 * 0.005)).abs() < 0.01,
        "the skin stays: {kept}"
    );
    let cut = pocketed(Some(0.01));
    assert!((cut - (1000.0 - hole)).abs() < 0.05, "cut through: {cut}");
}

/// A cube sunk 0.05 mm into another, fused at a tolerance of 0.1 mm, is
/// both cubes' material.
#[test]
fn a_tolerance_of_a_tenth_keeps_both_cubes() {
    let cube = |z: f64, op: BooleanOp| SolidOp::Primitive {
        kind: PrimitiveKind::Box {
            length: 10.0,
            width: 10.0,
            height: 10.0,
        },
        placement: Placement {
            origin: [0.0, 0.0, z],
            ..Placement::default()
        },
        op,
    };
    let ops = [
        cube(0.0, BooleanOp::NewSolid),
        cube(9.95, BooleanOp::Fuse).with_boolean_tolerance(Some(0.1)),
    ];
    let built = chain(&ops);
    // Two 1000 mm³ cubes sharing a 0.05 mm slab of 100 mm².
    assert!(
        (volume(&built.brep_blob) - 1995.0).abs() < 0.01,
        "{}",
        volume(&built.brep_blob)
    );
}
