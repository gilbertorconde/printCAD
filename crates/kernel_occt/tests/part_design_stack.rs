//! Full-stack Part Design test: sketch geometry → Pad/Pocket features →
//! `wb_part::body_build_ops` → `OcctKernel::execute_solid_chain` → mesh.
//! This exercises the exact pipeline the app's recompute driver runs.

use std::sync::{Mutex, MutexGuard};

use core_document::{BodyId, Document, FeatureId};
use kernel_api::TessellationSettings;
use kernel_occt::OcctKernel;
use wb_part::PartFeature;
use wb_sketch::sketch::{Circle, GeometryElement, Line, Point, Sketch, Vec2D};
use wb_sketch::SketchFeature;

/// OCCT is not thread-safe across concurrent kernel use in one process.
static OCCT_SERIAL: Mutex<()> = Mutex::new(());

fn occt_guard() -> MutexGuard<'static, ()> {
    OCCT_SERIAL.lock().unwrap_or_else(|p| p.into_inner())
}

fn rect_sketch_on(plane: wb_sketch::sketch::SketchPlane, width: f32, height: f32) -> SketchFeature {
    let mut sketch = Sketch::new("s");
    sketch.plane = plane;
    let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
    let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(width, 0.0))));
    let c = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(
        width, height,
    ))));
    let d = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, height))));
    for (s, e) in [(a, b), (b, c), (c, d), (d, a)] {
        sketch.add_geometry(GeometryElement::Line(Line::new(s, e)));
    }
    SketchFeature::new(sketch, plane)
}

fn rect_sketch(width: f32, height: f32) -> SketchFeature {
    let mut sketch = Sketch::new("s");
    let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
    let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(width, 0.0))));
    let c = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(
        width, height,
    ))));
    let d = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, height))));
    for (s, e) in [(a, b), (b, c), (c, d), (d, a)] {
        sketch.add_geometry(GeometryElement::Line(Line::new(s, e)));
    }
    let plane = sketch.plane;
    SketchFeature::new(sketch, plane)
}

fn circle_sketch_on(
    plane: wb_sketch::sketch::SketchPlane,
    cx: f32,
    cy: f32,
    r: f32,
) -> SketchFeature {
    let mut sketch = Sketch::new("c");
    sketch.plane = plane;
    let center = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(cx, cy))));
    sketch.add_geometry(GeometryElement::Circle(Circle::new(center, r)));
    SketchFeature::new(sketch, plane)
}

fn setup(width: f32, height: f32) -> (Document, BodyId, FeatureId) {
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let sketch_id = doc
        .add_feature_in_body(rect_sketch(width, height), "sketch".into(), Some(body))
        .unwrap();
    (doc, body, sketch_id)
}

fn pad_feature(sketch: FeatureId, length: f32, reversed: bool, symmetric: bool) -> PartFeature {
    PartFeature::Pad {
        sketch,
        length,
        reversed,
        symmetric,
        mode: wb_part::ExtrudeMode::Dimension,
        length2: 0.0,
        taper_deg: 0.0,
        up_to_face: None,
        up_to_offset: 0.0,
    }
}

fn pocket_feature(sketch: FeatureId, depth: f32) -> PartFeature {
    PartFeature::Pocket {
        sketch,
        depth,
        reversed: false,
        through_all: false,
        mode: wb_part::ExtrudeMode::Dimension,
        depth2: 0.0,
        taper_deg: 0.0,
        up_to_face: None,
        up_to_offset: 0.0,
    }
}

fn mesh_bounds(mesh: &kernel_api::TriMesh) -> ([f32; 3], [f32; 3]) {
    mesh.bounds().expect("non-empty mesh")
}

#[test]
fn pad_feature_builds_a_box_through_the_full_stack() {
    let _serial = occt_guard();
    let (mut doc, body, sketch_id) = setup(10.0, 5.0);
    doc.add_feature_in_body(
        pad_feature(sketch_id, 8.0, false, false),
        "Pad".into(),
        Some(body),
    )
    .unwrap();

    let ops = wb_part::body_build_ops(&doc, body).unwrap().ops;
    let mut kernel = OcctKernel::new();
    let result = kernel
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .unwrap();

    assert!(!result.brep_blob.is_empty());
    let (min, max) = mesh_bounds(&result.mesh);
    assert!((max[0] - min[0] - 10.0).abs() < 1e-3, "width");
    assert!((max[1] - min[1] - 5.0).abs() < 1e-3, "height");
    assert!((max[2] - min[2] - 8.0).abs() < 1e-3, "pad length");
    assert!(min[2].abs() < 1e-3, "starts on the sketch plane");
}

/// Regression for the reported "pocket did nothing" bug: a sketch drawn on
/// the TOP FACE of a pad has its normal pointing out of the material; the
/// pocket must cut against that normal (into the pad) by default.
#[test]
fn pocket_feature_cuts_into_the_pad() {
    let _serial = occt_guard();
    let (mut doc, body, rect_id) = setup(20.0, 20.0);
    // The hole sketch sits on the pad's top face (z = 6, normal +Z), exactly
    // as produced by clicking the face and choosing "Selected face".
    let top_face = wb_sketch::sketch::SketchPlane::from_face([10.0, 10.0, 6.0], [0.0, 0.0, 1.0]);
    let hole_id = doc
        .add_feature_in_body(
            circle_sketch_on(top_face, 0.0, 0.0, 3.0),
            "hole".into(),
            Some(body),
        )
        .unwrap();
    doc.add_feature_in_body(
        pad_feature(rect_id, 6.0, false, false),
        "Pad".into(),
        Some(body),
    )
    .unwrap();
    doc.add_feature_in_body(pocket_feature(hole_id, 6.0), "Pocket".into(), Some(body))
        .unwrap();

    let ops = wb_part::body_build_ops(&doc, body).unwrap().ops;
    assert_eq!(ops.len(), 2);

    let mut kernel = OcctKernel::new();
    let detail = TessellationSettings::default();

    // Pad only.
    let solid = kernel.execute_solid_chain(&ops[..1], &detail).unwrap();
    // Pad + pocket: same bounds, more triangles (the bore adds a wall).
    let with_hole = kernel.execute_solid_chain(&ops, &detail).unwrap();

    let (a_min, a_max) = mesh_bounds(&solid.mesh);
    let (b_min, b_max) = mesh_bounds(&with_hole.mesh);
    for i in 0..3 {
        assert!((a_min[i] - b_min[i]).abs() < 1e-3);
        assert!((a_max[i] - b_max[i]).abs() < 1e-3);
    }
    assert!(
        with_hole.mesh.indices.len() > solid.mesh.indices.len(),
        "through-hole adds bore triangles ({} vs {})",
        with_hole.mesh.indices.len(),
        solid.mesh.indices.len()
    );
}

#[test]
fn pad_on_front_plane_extrudes_along_minus_y() {
    let _serial = occt_guard();
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    // Front (XZ) plane: sketch x → world X, sketch y → world Z, normal -Y.
    let sketch_id = doc
        .add_feature_in_body(
            rect_sketch_on(wb_sketch::sketch::SketchPlane::xz(), 10.0, 4.0),
            "front".into(),
            Some(body),
        )
        .unwrap();
    doc.add_feature_in_body(
        pad_feature(sketch_id, 6.0, false, false),
        "Pad".into(),
        Some(body),
    )
    .unwrap();

    let ops = wb_part::body_build_ops(&doc, body).unwrap().ops;
    let mut kernel = OcctKernel::new();
    let result = kernel
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .unwrap();
    let (min, max) = mesh_bounds(&result.mesh);
    assert!((max[0] - min[0] - 10.0).abs() < 1e-3, "world X = sketch x");
    assert!((max[2] - min[2] - 4.0).abs() < 1e-3, "world Z = sketch y");
    assert!((max[1] - min[1] - 6.0).abs() < 1e-3, "extruded along Y");
    assert!(max[1].abs() < 1e-3, "normal is -Y: solid at negative Y");
}

#[test]
fn editing_the_pad_length_changes_the_solid() {
    let _serial = occt_guard();
    let (mut doc, body, sketch_id) = setup(10.0, 5.0);
    let pad_id = doc
        .add_feature_in_body(
            pad_feature(sketch_id, 8.0, false, false),
            "Pad".into(),
            Some(body),
        )
        .unwrap();

    // Simulate the panel edit: update data, mark dirty, rebuild.
    use core_document::WorkbenchFeature;
    doc.update_feature_data(pad_id, pad_feature(sketch_id, 3.0, true, false).to_json())
        .unwrap();
    doc.mark_feature_dirty(pad_id);
    assert_eq!(wb_part::pending_body_rebuilds(&doc), vec![body]);

    let ops = wb_part::body_build_ops(&doc, body).unwrap().ops;
    let mut kernel = OcctKernel::new();
    let result = kernel
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .unwrap();
    let (min, max) = mesh_bounds(&result.mesh);
    assert!((max[2] - min[2] - 3.0).abs() < 1e-3, "new length");
    assert!(max[2].abs() < 1e-3, "reversed: solid below the plane");
}

#[test]
fn revolution_feature_builds_a_ring_through_the_full_stack() {
    let _serial = occt_guard();
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    // Rectangle x ∈ [5, 8], y ∈ [0, 2]: revolving about the sketch Y axis
    // sweeps a ring of outer radius 8.
    let mut sketch = Sketch::new("ring");
    let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, 0.0))));
    let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(8.0, 0.0))));
    let c = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(8.0, 2.0))));
    let d = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, 2.0))));
    for (s, e) in [(a, b), (b, c), (c, d), (d, a)] {
        sketch.add_geometry(GeometryElement::Line(Line::new(s, e)));
    }
    let plane = sketch.plane;
    let sketch_id = doc
        .add_feature_in_body(SketchFeature::new(sketch, plane), "ring".into(), Some(body))
        .unwrap();
    doc.add_feature_in_body(
        wb_part::PartFeature::Revolution {
            sketch: sketch_id,
            angle_deg: 360.0,
            axis: wb_part::RevolveAxis::SketchY,
            reversed: false,
            midplane: false,
            second_angle_deg: None,
        },
        "Revolution".into(),
        Some(body),
    )
    .unwrap();

    let ops = wb_part::body_build_ops(&doc, body).unwrap().ops;
    let mut kernel = OcctKernel::new();
    let result = kernel
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .unwrap();
    let (min, max) = mesh_bounds(&result.mesh);
    // XY sketch plane, revolve about its y axis (world Y through origin):
    // the swept ring spans ±8 in world X and Z, height 2 in world Y.
    assert!(
        (max[0] - 8.0).abs() < 0.1 && (min[0] + 8.0).abs() < 0.1,
        "x span"
    );
    assert!(
        (max[2] - 8.0).abs() < 0.1 && (min[2] + 8.0).abs() < 0.1,
        "z span"
    );
    assert!((max[1] - min[1] - 2.0).abs() < 0.1, "height");
}

#[test]
fn fillet_feature_rounds_the_pad_through_the_full_stack() {
    let _serial = occt_guard();
    let (mut doc, body, sketch_id) = setup(20.0, 20.0);
    doc.add_feature_in_body(
        pad_feature(sketch_id, 10.0, false, false),
        "Pad".into(),
        Some(body),
    )
    .unwrap();

    let mut kernel = OcctKernel::new();
    let detail = TessellationSettings::default();
    let plain = kernel
        .execute_solid_chain(&wb_part::body_build_ops(&doc, body).unwrap().ops, &detail)
        .unwrap();

    doc.add_feature_in_body(
        PartFeature::Fillet {
            radius: 2.0,
            edges: wb_part::EdgeSel::All,
        },
        "Fillet".into(),
        Some(body),
    )
    .unwrap();
    let filleted = kernel
        .execute_solid_chain(&wb_part::body_build_ops(&doc, body).unwrap().ops, &detail)
        .unwrap();
    assert!(
        filleted.mesh.indices.len() > plain.mesh.indices.len(),
        "fillets add curved faces"
    );
}

#[test]
fn hole_feature_drills_the_pad_through_the_full_stack() {
    let _serial = occt_guard();
    let (mut doc, body, rect_id) = setup(30.0, 20.0);
    doc.add_feature_in_body(
        pad_feature(rect_id, 6.0, false, false),
        "Pad".into(),
        Some(body),
    )
    .unwrap();

    // Two hole positions on the pad's top face.
    let top_face = wb_sketch::sketch::SketchPlane::from_face([15.0, 10.0, 6.0], [0.0, 0.0, 1.0]);
    let mut holes = Sketch::new("holes");
    holes.plane = top_face;
    for x in [-8.0f32, 8.0] {
        let center = holes.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 0.0))));
        holes.add_geometry(GeometryElement::Circle(Circle::new(center, 1.0)));
    }
    let holes_id = doc
        .add_feature_in_body(
            SketchFeature::new(holes, top_face),
            "holes".into(),
            Some(body),
        )
        .unwrap();
    doc.add_feature_in_body(
        PartFeature::Hole {
            sketch: holes_id,
            diameter: 4.0,
            depth: 3.0,
            through_all: true,
            cut: wb_part::HoleCut::None,
            metric_index: None,
            threaded: false,
            fit: wb_part::HoleFit::Normal,
            reversed: false,
        },
        "Hole".into(),
        Some(body),
    )
    .unwrap();

    let mut kernel = OcctKernel::new();
    let plan = wb_part::body_build_ops(&doc, body).unwrap();
    let result = kernel
        .execute_solid_chain(&plan.ops, &TessellationSettings::default())
        .unwrap();
    let (min, max) = mesh_bounds(&result.mesh);
    assert!((max[0] - min[0] - 30.0).abs() < 1e-3, "plate width kept");
    // The two through-bores add interior walls: more than the 12 box tris.
    assert!(result.mesh.indices.len() / 3 > 12);
}

#[test]
fn linear_pattern_feature_repeats_a_boss_through_the_full_stack() {
    let _serial = occt_guard();
    let (mut doc, body, plate_id) = setup(60.0, 20.0);
    doc.add_feature_in_body(
        pad_feature(plate_id, 4.0, false, false),
        "Pad".into(),
        Some(body),
    )
    .unwrap();

    let mut boss = Sketch::new("boss");
    let center = boss.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 10.0))));
    boss.add_geometry(GeometryElement::Circle(Circle::new(center, 3.0)));
    let boss_plane = boss.plane;
    let boss_id = doc
        .add_feature_in_body(
            SketchFeature::new(boss, boss_plane),
            "boss".into(),
            Some(body),
        )
        .unwrap();
    let boss_pad = doc
        .add_feature_in_body(
            pad_feature(boss_id, 12.0, false, false),
            "Boss".into(),
            Some(body),
        )
        .unwrap();
    doc.add_feature_in_body(
        PartFeature::LinearPattern {
            originals: vec![boss_pad],
            axis: wb_part::PatternAxis::X,
            length: 40.0,
            occurrences: 3,
            spacing_mode: false,
            reversed: false,
        },
        "Pattern".into(),
        Some(body),
    )
    .unwrap();

    let mut kernel = OcctKernel::new();
    let plan = wb_part::body_build_ops(&doc, body).unwrap();
    assert_eq!(plan.ops.len(), 3);
    let result = kernel
        .execute_solid_chain(&plan.ops, &TessellationSettings::default())
        .unwrap();
    let (min, max) = mesh_bounds(&result.mesh);
    // Bosses at x = 10, 30, 50, all inside the 60-wide plate.
    assert!((max[0] - min[0] - 60.0).abs() < 1e-3, "plate width kept");
    assert!((max[2] - 12.0).abs() < 1e-3, "boss height everywhere");
}

#[test]
fn symmetric_pad_straddles_the_sketch_plane() {
    let _serial = occt_guard();
    let (mut doc, body, sketch_id) = setup(10.0, 5.0);
    doc.add_feature_in_body(
        pad_feature(sketch_id, 8.0, false, true),
        "Pad".into(),
        Some(body),
    )
    .unwrap();
    let ops = wb_part::body_build_ops(&doc, body).unwrap().ops;
    let mut kernel = OcctKernel::new();
    let result = kernel
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .unwrap();
    let (min, max) = mesh_bounds(&result.mesh);
    assert!(
        (max[2] - 4.0).abs() < 1e-3 && (min[2] + 4.0).abs() < 1e-3,
        "±4 about the plane"
    );
}
