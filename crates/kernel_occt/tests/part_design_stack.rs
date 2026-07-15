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

fn circle_sketch(cx: f32, cy: f32, r: f32) -> SketchFeature {
    let mut sketch = Sketch::new("c");
    let center = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(cx, cy))));
    sketch.add_geometry(GeometryElement::Circle(Circle::new(center, r)));
    let plane = sketch.plane;
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

fn mesh_bounds(mesh: &kernel_api::TriMesh) -> ([f32; 3], [f32; 3]) {
    mesh.bounds().expect("non-empty mesh")
}

#[test]
fn pad_feature_builds_a_box_through_the_full_stack() {
    let _serial = occt_guard();
    let (mut doc, body, sketch_id) = setup(10.0, 5.0);
    doc.add_feature_in_body(
        PartFeature::Pad {
            sketch: sketch_id,
            length: 8.0,
            reversed: false,
            symmetric: false,
        },
        "Pad".into(),
        Some(body),
    )
    .unwrap();

    let ops = wb_part::body_build_ops(&doc, body).unwrap();
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

#[test]
fn pocket_feature_cuts_into_the_pad() {
    let _serial = occt_guard();
    let (mut doc, body, rect_id) = setup(20.0, 20.0);
    let hole_id = doc
        .add_feature_in_body(circle_sketch(10.0, 10.0, 3.0), "hole".into(), Some(body))
        .unwrap();
    doc.add_feature_in_body(
        PartFeature::Pad {
            sketch: rect_id,
            length: 6.0,
            reversed: false,
            symmetric: false,
        },
        "Pad".into(),
        Some(body),
    )
    .unwrap();
    doc.add_feature_in_body(
        PartFeature::Pocket {
            sketch: hole_id,
            depth: 6.0,
            reversed: false,
            through_all: false,
        },
        "Pocket".into(),
        Some(body),
    )
    .unwrap();

    let ops = wb_part::body_build_ops(&doc, body).unwrap();
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
        PartFeature::Pad {
            sketch: sketch_id,
            length: 6.0,
            reversed: false,
            symmetric: false,
        },
        "Pad".into(),
        Some(body),
    )
    .unwrap();

    let ops = wb_part::body_build_ops(&doc, body).unwrap();
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
            PartFeature::Pad {
                sketch: sketch_id,
                length: 8.0,
                reversed: false,
                symmetric: false,
            },
            "Pad".into(),
            Some(body),
        )
        .unwrap();

    // Simulate the panel edit: update data, mark dirty, rebuild.
    use core_document::WorkbenchFeature;
    doc.update_feature_data(
        pad_id,
        PartFeature::Pad {
            sketch: sketch_id,
            length: 3.0,
            reversed: true,
            symmetric: false,
        }
        .to_json(),
    )
    .unwrap();
    doc.mark_feature_dirty(pad_id);
    assert_eq!(wb_part::pending_body_rebuilds(&doc), vec![body]);

    let ops = wb_part::body_build_ops(&doc, body).unwrap();
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
        },
        "Revolution".into(),
        Some(body),
    )
    .unwrap();

    let ops = wb_part::body_build_ops(&doc, body).unwrap();
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
fn symmetric_pad_straddles_the_sketch_plane() {
    let _serial = occt_guard();
    let (mut doc, body, sketch_id) = setup(10.0, 5.0);
    doc.add_feature_in_body(
        PartFeature::Pad {
            sketch: sketch_id,
            length: 8.0,
            reversed: false,
            symmetric: true,
        },
        "Pad".into(),
        Some(body),
    )
    .unwrap();
    let ops = wb_part::body_build_ops(&doc, body).unwrap();
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
