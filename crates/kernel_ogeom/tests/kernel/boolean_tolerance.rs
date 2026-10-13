//! A feature's stated boolean tolerance: faces that nearly meet are
//! joined by its fuse or cut rather than left with a gap or a sliver.

use core_document::{BodyId, Document, FeatureId, WorkbenchFeature};
use kernel_api::{
    BooleanOp, Placement, PrimitiveKind, SolidBuildResult, SolidOp, TessellationSettings,
};
use kernel_ogeom::OgeomKernel;
use wb_design::DesignFeature;
use wb_sketch::SketchFeature;
use wb_sketch::sketch::{GeometryElement, Line, Point, Sketch, SketchPlane, Vec2D};

/// A square of `side` on the plane parallel to XY through `origin`, its
/// corner there.
fn square_at(origin: [f64; 3], side: f64) -> SketchFeature {
    let plane = SketchPlane {
        origin,
        ..SketchPlane::xy()
    };
    let mut sketch = Sketch::new("s");
    sketch.plane = plane;
    let corners = [(0.0, 0.0), (side, 0.0), (side, side), (0.0, side)]
        .map(|(x, y)| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y)))));
    for i in 0..4 {
        sketch.add_geometry(GeometryElement::Line(Line::new(
            corners[i],
            corners[(i + 1) % 4],
        )));
    }
    SketchFeature::new(sketch, plane)
}

fn pad(sketch: FeatureId, length: f32) -> DesignFeature {
    DesignFeature::from_json(&serde_json::json!({
        "Pad": { "sketch": sketch, "length": length, "reversed": false }
    }))
    .unwrap()
}

/// A body holding a 10 mm cube padded from the XY plane.
fn cube_body() -> (Document, BodyId) {
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let base = doc
        .add_feature_in_body(square_at([0.0; 3], 10.0), "base".into(), Some(body))
        .unwrap();
    doc.add_feature_in_body(pad(base, 10.0), "Pad".into(), Some(body))
        .unwrap();
    (doc, body)
}

/// The cube, and a second cube padded from a sketch at height `z`, its
/// fuse at `tolerance`.
fn stacked(z: f64, tolerance: Option<f32>) -> (Document, BodyId) {
    let (mut doc, body) = cube_body();
    let upper = doc
        .add_feature_in_body(square_at([0.0, 0.0, z], 10.0), "upper".into(), Some(body))
        .unwrap();
    let mut second = pad(upper, 10.0);
    second.set_boolean_tolerance(tolerance);
    doc.add_feature_in_body(second, "Pad001".into(), Some(body))
        .unwrap();
    (doc, body)
}

fn build(doc: &Document, body: BodyId) -> SolidBuildResult {
    let ops = wb_design::body_build_ops(doc, body).unwrap().ops;
    OgeomKernel::new()
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .unwrap()
}

/// The solids and faces of a build's shape.
fn solids_and_faces(built: &SolidBuildResult) -> (usize, usize) {
    use ogeom::topo::{Model, ShapeType, explore_unique};
    let mut model = Model::new();
    let text = std::str::from_utf8(&built.brep_blob).unwrap();
    let shape = ogeom::io::native::read_into(&mut model, text)
        .unwrap()
        .shapes
        .remove(0);
    let count = |kind| explore_unique(&model, &shape, kind).unwrap().len();
    (count(ShapeType::Solid), count(ShapeType::Face))
}

fn volume(built: &SolidBuildResult) -> f64 {
    OgeomKernel::new()
        .physical_properties(&built.brep_blob)
        .unwrap()
        .volume_mm3
        .expect("a closed solid")
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
        (volume(&joined) - 2000.0).abs() < 1.0,
        "both cubes' material: {}",
        volume(&joined)
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
        let top = doc
            .add_feature_in_body(square_at([3.0, 3.0, 10.0], 4.0), "top".into(), Some(body))
            .unwrap();
        let mut pocket = DesignFeature::from_json(&serde_json::json!({
            "Pocket": { "sketch": top, "depth": 9.995, "reversed": false }
        }))
        .unwrap();
        pocket.set_boolean_tolerance(tolerance);
        doc.add_feature_in_body(pocket, "Pocket".into(), Some(body))
            .unwrap();
        volume(&build(&doc, body))
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
#[ignore = "kernel: fuse_fuzzy at a fuzz of 0.1 mm returns the first solid alone (ogeom-rs#215)"]
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
    let built = OgeomKernel::new()
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .unwrap();
    assert!((volume(&built) - 1999.5).abs() < 0.01, "{}", volume(&built));
}
