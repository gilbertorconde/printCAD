//! Full-stack Part Design test: sketch geometry → Pad/Pocket features →
//! `wb_part::body_build_ops` → `OgeomKernel::execute_solid_chain` → mesh.
//! This exercises the exact pipeline the app's recompute driver runs.

use core_document::{BodyId, Document, FeatureId};
use kernel_api::TessellationSettings;
use kernel_ogeom::OgeomKernel;
use wb_part::PartFeature;
use wb_sketch::SketchFeature;
use wb_sketch::sketch::{Circle, GeometryElement, Line, Point, Sketch, Vec2D};

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
        refine: false,
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
        refine: false,
        sketch,
        depth,
        reversed: false,
        symmetric: false,
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
    let (mut doc, body, sketch_id) = setup(10.0, 5.0);
    doc.add_feature_in_body(
        pad_feature(sketch_id, 8.0, false, false),
        "Pad".into(),
        Some(body),
    )
    .unwrap();

    let ops = wb_part::body_build_ops(&doc, body).unwrap().ops;
    let mut kernel = OgeomKernel::new();
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
    let (mut doc, body, rect_id) = setup(20.0, 20.0);
    // The hole sketch sits on the pad's top face (z = 6, normal +Z), exactly
    // as produced by clicking the face and choosing "Selected face". The
    // plane's frame starts at the document origin, so the pad's middle is at
    // (10, 10) on it.
    let top_face = wb_sketch::sketch::SketchPlane::from_face([10.0, 10.0, 6.0], [0.0, 0.0, 1.0]);
    let hole_id = doc
        .add_feature_in_body(
            circle_sketch_on(top_face, 10.0, 10.0, 3.0),
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

    let mut kernel = OgeomKernel::new();
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
    let mut kernel = OgeomKernel::new();
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
    let mut kernel = OgeomKernel::new();
    let result = kernel
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .unwrap();
    let (min, max) = mesh_bounds(&result.mesh);
    assert!((max[2] - min[2] - 3.0).abs() < 1e-3, "new length");
    assert!(max[2].abs() < 1e-3, "reversed: solid below the plane");
}

#[test]
fn revolution_feature_builds_a_ring_through_the_full_stack() {
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
            refine: false,
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
    let mut kernel = OgeomKernel::new();
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
    let (mut doc, body, sketch_id) = setup(20.0, 20.0);
    doc.add_feature_in_body(
        pad_feature(sketch_id, 10.0, false, false),
        "Pad".into(),
        Some(body),
    )
    .unwrap();

    let mut kernel = OgeomKernel::new();
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
    let (mut doc, body, rect_id) = setup(30.0, 20.0);
    doc.add_feature_in_body(
        pad_feature(rect_id, 6.0, false, false),
        "Pad".into(),
        Some(body),
    )
    .unwrap();

    // Two hole positions on the pad's top face, either side of its middle
    // (the plane's frame starts at the document origin).
    let top_face = wb_sketch::sketch::SketchPlane::from_face([15.0, 10.0, 6.0], [0.0, 0.0, 1.0]);
    let mut holes = Sketch::new("holes");
    holes.plane = top_face;
    for x in [7.0f32, 23.0] {
        let center = holes.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 10.0))));
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
            refine: false,
            sketch: holes_id,
            diameter: 4.0,
            depth: 3.0,
            through_all: true,
            cut: wb_part::HoleCut::None,
            metric_index: None,
            threaded: false,
            modeled_thread: false,
            thread_depth: 0.0,
            fit: wb_part::HoleFit::Normal,
            reversed: false,
        },
        "Hole".into(),
        Some(body),
    )
    .unwrap();

    let mut kernel = OgeomKernel::new();
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
            refine: false,
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

    let mut kernel = OgeomKernel::new();
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
    let (mut doc, body, sketch_id) = setup(10.0, 5.0);
    doc.add_feature_in_body(
        pad_feature(sketch_id, 8.0, false, true),
        "Pad".into(),
        Some(body),
    )
    .unwrap();
    let ops = wb_part::body_build_ops(&doc, body).unwrap().ops;
    let mut kernel = OgeomKernel::new();
    let result = kernel
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .unwrap();
    let (min, max) = mesh_bounds(&result.mesh);
    assert!(
        (max[2] - 4.0).abs() < 1e-3 && (min[2] + 4.0).abs() < 1e-3,
        "±4 about the plane"
    );
}

/// Whether every face of `mesh` winds outward: its triangles' normals point
/// away from the mesh centroid, as a solid's skin must whichever way the
/// profile it came from was drawn.
fn every_face_winds_outward(mesh: &kernel_api::TriMesh) -> bool {
    let (min, max) = mesh_bounds(mesh);
    let centre = [
        (min[0] + max[0]) / 2.0,
        (min[1] + max[1]) / 2.0,
        (min[2] + max[2]) / 2.0,
    ];
    mesh.indices.chunks(3).all(|tri| {
        let p = |i: u32| mesh.positions[i as usize];
        let (a, b, c) = (p(tri[0]), p(tri[1]), p(tri[2]));
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let out = [
            (a[0] + b[0] + c[0]) / 3.0 - centre[0],
            (a[1] + b[1] + c[1]) / 3.0 - centre[1],
            (a[2] + b[2] + c[2]) / 3.0 - centre[2],
        ];
        n[0] * out[0] + n[1] * out[1] + n[2] * out[2] > 0.0
    })
}

/// A square drawn clockwise pads to the same solid as one drawn
/// counter-clockwise: every face of its skin faces out.
#[test]
fn a_clockwise_profile_pads_to_an_outward_facing_solid() {
    for clockwise in [false, true] {
        let mut sketch = Sketch::new("s");
        let mut corners = [(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)];
        if clockwise {
            corners.reverse();
        }
        let ids: Vec<_> = corners
            .iter()
            .map(|(x, y)| {
                sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(*x, *y))))
            })
            .collect();
        for i in 0..4 {
            sketch.add_geometry(GeometryElement::Line(Line::new(ids[i], ids[(i + 1) % 4])));
        }
        let plane = sketch.plane;
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Body".into()));
        let sketch_id = doc
            .add_feature_in_body(
                SketchFeature::new(sketch, plane),
                "sketch".into(),
                Some(body),
            )
            .unwrap();
        doc.add_feature_in_body(
            pad_feature(sketch_id, 10.0, false, false),
            "Pad".into(),
            Some(body),
        )
        .unwrap();

        let ops = wb_part::body_build_ops(&doc, body).unwrap().ops;
        let mesh = OgeomKernel::new()
            .execute_solid_chain(&ops, &TessellationSettings::default())
            .unwrap()
            .mesh;
        assert_eq!(mesh.indices.len() / 3, 12, "clockwise={clockwise}: a box");
        assert!(
            every_face_winds_outward(&mesh),
            "clockwise={clockwise}: a face winds into the solid"
        );
    }
}

/// A second pad stacked flush on the first leaves each side split along
/// the seam; with Refine on it the block comes out with its six faces.
#[test]
fn a_refined_pad_stacked_on_a_pad_leaves_six_faces() {
    let faces_with = |refine: bool| {
        let (mut doc, body, sketch_id) = setup(20.0, 20.0);
        doc.add_feature_in_body(
            pad_feature(sketch_id, 10.0, false, false),
            "Pad".into(),
            Some(body),
        )
        .unwrap();
        let raised = wb_sketch::sketch::SketchPlane {
            origin: [0.0, 0.0, 10.0],
            ..wb_sketch::sketch::SketchPlane::xy()
        };
        let upper = doc
            .add_feature_in_body(
                rect_sketch_on(raised, 20.0, 20.0),
                "upper".into(),
                Some(body),
            )
            .unwrap();
        let mut second = pad_feature(upper, 10.0, false, false);
        second.set_refine(refine);
        doc.add_feature_in_body(second, "Pad001".into(), Some(body))
            .unwrap();
        let ops = wb_part::body_build_ops(&doc, body).unwrap().ops;
        let mesh = OgeomKernel::new()
            .execute_solid_chain(&ops, &TessellationSettings::default())
            .unwrap()
            .mesh;
        mesh.faces
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    };
    assert!(faces_with(false) > 6, "unrefined, the sides are split");
    assert_eq!(faces_with(true), 6, "refined, one face per side");
}

/// Half an ellipse closed by its major axis pads to the half-elliptic
/// prism: the arc's endpoints and the line's meet exactly in the profile.
#[test]
fn an_arc_of_ellipse_closed_by_a_line_pads_to_its_area() {
    use wb_sketch::sketch::Ellipse;
    let (a, b, height) = (10.0f32, 5.0f32, 4.0f32);
    let mut sketch = Sketch::new("half ellipse");
    let center = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
    let right = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(a, 0.0))));
    let left = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(-a, 0.0))));
    sketch.add_geometry(GeometryElement::Ellipse(Ellipse::new_arc(
        center,
        Vec2D::new(a, 0.0),
        b / a,
        right,
        left,
    )));
    sketch.add_geometry(GeometryElement::Line(Line::new(left, right)));
    let plane = sketch.plane;

    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let sketch_id = doc
        .add_feature_in_body(
            SketchFeature::new(sketch, plane),
            "sketch".into(),
            Some(body),
        )
        .unwrap();
    doc.add_feature_in_body(
        pad_feature(sketch_id, height, false, false),
        "Pad".into(),
        Some(body),
    )
    .unwrap();
    let ops = wb_part::body_build_ops(&doc, body).unwrap().ops;
    let mut kernel = OgeomKernel::new();
    let result = kernel
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .expect("the half ellipse pads");
    let (lo, hi) = result.bounds_mm.expect("bounds");
    for (got, want) in [
        (lo[0], -a),
        (hi[0], a),
        (lo[1], 0.0),
        (hi[1], b),
        (hi[2] - lo[2], height),
    ] {
        assert!((got - want).abs() < 1e-3, "bounds {lo:?}..{hi:?}");
    }
    // The elliptic wall is a line swept along an ellipse, which the kernel
    // integrates in closed form: the volume is exact, and says so.
    let props = kernel.physical_properties(&result.brep_blob).unwrap();
    let volume = props.volume_mm3.expect("a closed solid");
    let expected = f64::from(std::f32::consts::PI * a * b / 2.0 * height);
    assert!(!props.approximate);
    assert!(
        (volume - expected).abs() < 1e-5 * expected,
        "volume {volume} vs {expected}"
    );
}

/// A block with a bore through it, the bore's top rim rounded: the fillet
/// a printed part's hole mouth takes most often.
#[test]
fn bore_rim_fillets() {
    let (mut doc, body, rect_id) = setup(40.0, 30.0);
    doc.add_feature_in_body(
        pad_feature(rect_id, 12.0, false, false),
        "Pad".into(),
        Some(body),
    )
    .unwrap();
    let top = wb_sketch::sketch::SketchPlane {
        origin: [0.0, 0.0, 12.0],
        ..Default::default()
    };
    let bore = doc
        .add_feature_in_body(
            circle_sketch_on(top, 20.0, 15.0, 6.0),
            "bore".into(),
            Some(body),
        )
        .unwrap();
    doc.add_feature_in_body(
        PartFeature::Pocket {
            refine: false,
            sketch: bore,
            depth: 12.0,
            reversed: false,
            symmetric: false,
            through_all: true,
            mode: wb_part::ExtrudeMode::Dimension,
            depth2: 0.0,
            taper_deg: 0.0,
            up_to_face: None,
            up_to_offset: 0.0,
        },
        "Pocket".into(),
        Some(body),
    )
    .unwrap();
    doc.add_feature_in_body(
        PartFeature::Fillet {
            radius: 2.0,
            edges: wb_part::EdgeSel::Edges(vec![wb_part::EdgePick {
                point: [26.0, 15.0, 12.0],
                direction: [0.0, 1.0, 0.0],
            }]),
        },
        "Fillet".into(),
        Some(body),
    )
    .unwrap();

    let mut kernel = OgeomKernel::new();
    let result = kernel
        .execute_solid_chain(
            &wb_part::body_build_ops(&doc, body).unwrap().ops,
            &TessellationSettings::default(),
        )
        .expect("the rim rounds");
    // The fillet takes the ring of area rho^2 (1 - pi/4) off the rim, its
    // centroid rho (10 - 3 pi) / (3 (4 - pi)) out from it (Pappus).
    let (r, rho) = (6.0_f64, 2.0_f64);
    let pi = std::f64::consts::PI;
    let area = rho * rho * (1.0 - pi / 4.0);
    let out = rho * (10.0 - 3.0 * pi) / (3.0 * (4.0 - pi));
    let expected = 40.0 * 30.0 * 12.0 - pi * r * r * 12.0 - 2.0 * pi * (r + out) * area;
    let volume = kernel
        .physical_properties(&result.brep_blob)
        .unwrap()
        .volume_mm3
        .expect("a closed solid");
    assert!(
        (volume - expected).abs() < 1e-3 * expected,
        "volume {volume} vs {expected}"
    );
}

#[test]
fn a_variable_drives_the_pad_and_changing_it_rebuilds_the_solid() {
    use core_document::{DocumentService, Variable, VariableSet, WorkbenchFeature};
    let mut registry = DocumentService::default();
    registry
        .register_workbench(Box::new(wb_part::PartDesignWorkbench::default()))
        .unwrap();
    let (mut doc, body, sketch_id) = setup(10.0, 5.0);
    let sizes = doc
        .add_feature(
            VariableSet {
                variables: vec![
                    Variable {
                        name: "base".into(),
                        formula: "4 mm".into(),
                        comment: String::new(),
                    },
                    Variable {
                        name: "height".into(),
                        formula: "Sizes.base * 2 - 1 mm".into(),
                        comment: String::new(),
                    },
                ],
            },
            "Sizes".into(),
        )
        .unwrap();
    let pad_id = doc
        .add_feature_in_body(
            pad_feature(sketch_id, 8.0, false, false),
            "Pad".into(),
            Some(body),
        )
        .unwrap();
    doc.set_feature_formula(pad_id, "/Pad/length", Some("Sizes.height".into()))
        .unwrap();

    let height = |doc: &mut Document| {
        let jobs = registry.rebuild_jobs(doc);
        let job = jobs
            .into_iter()
            .find(|j| j.body == body)
            .expect("a rebuild");
        let ops = job.plan.unwrap().ops;
        let result = OgeomKernel::new()
            .execute_solid_chain(&ops, &TessellationSettings::default())
            .unwrap();
        let (min, max) = mesh_bounds(&result.mesh);
        max[2] - min[2]
    };
    assert!((height(&mut doc) - 7.0).abs() < 1e-3, "2 * 4 - 1");

    // Change the variable: the pad rebuilds at the new height, and nothing
    // else asked for it.
    let mut set = VariableSet::from_json(doc.get_feature_data(sizes).unwrap()).unwrap();
    set.variables[0].formula = "6 mm".into();
    doc.update_feature_data(sizes, set.to_json()).unwrap();
    assert!((height(&mut doc) - 11.0).abs() < 1e-3, "2 * 6 - 1");
    assert!(registry.rebuild_jobs(&mut doc).is_empty(), "settled");
}

#[test]
fn a_variable_drives_a_named_sketch_dimension_and_the_pad_on_it() {
    use core_document::{DocumentService, Variable, VariableSet, WorkbenchFeature};
    use wb_sketch::sketch::{Constraint, ConstraintKind};
    let mut registry = DocumentService::default();
    registry
        .register_workbench(Box::new(wb_sketch::SketchWorkbench::default()))
        .unwrap();
    registry
        .register_workbench(Box::new(wb_part::PartDesignWorkbench::default()))
        .unwrap();

    // A 10 x 5 rectangle, fixed at the origin, square, its bottom named
    // `width`.
    let mut sketch = Sketch::new("s");
    let at = |x, y| GeometryElement::Point(Point::new(Vec2D::new(x, y)));
    let a = sketch.add_geometry(at(0.0, 0.0));
    let b = sketch.add_geometry(at(10.0, 0.0));
    let c = sketch.add_geometry(at(10.0, 5.0));
    let d = sketch.add_geometry(at(0.0, 5.0));
    let lines: Vec<_> = [(a, b), (b, c), (c, d), (d, a)]
        .into_iter()
        .map(|(s, e)| sketch.add_geometry(GeometryElement::Line(Line::new(s, e))))
        .collect();
    let mut add = |kind| sketch.constraints.push(Constraint::new(kind));
    add(ConstraintKind::FixedPoint {
        point: a,
        position: Vec2D::new(0.0, 0.0),
    });
    add(ConstraintKind::Horizontal { element: lines[0] });
    add(ConstraintKind::Vertical { element: lines[1] });
    add(ConstraintKind::Horizontal { element: lines[2] });
    add(ConstraintKind::Vertical { element: lines[3] });
    add(ConstraintKind::Length {
        line: lines[1],
        length: 5.0,
    });
    let mut width = Constraint::new(ConstraintKind::Length {
        line: lines[0],
        length: 10.0,
    });
    width.name = Some("width".into());
    let width_key = width.id.to_string();
    sketch.constraints.push(width);
    let plane = sketch.plane;

    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let sizes = doc
        .add_feature(
            VariableSet {
                variables: vec![Variable {
                    name: "w".into(),
                    formula: "24 mm".into(),
                    comment: String::new(),
                }],
            },
            "Sizes".into(),
        )
        .unwrap();
    let sketch_id = doc
        .add_feature_in_body(
            SketchFeature::new(sketch, plane),
            "Profile".into(),
            Some(body),
        )
        .unwrap();
    doc.set_feature_formula(sketch_id, &width_key, Some("Sizes.w".into()))
        .unwrap();
    let pad_id = doc
        .add_feature_in_body(
            pad_feature(sketch_id, 3.0, false, false),
            "Pad".into(),
            Some(body),
        )
        .unwrap();
    // The pad reads the sketch's named dimension too.
    doc.set_feature_formula(pad_id, "/Pad/length", Some("Profile.width / 4".into()))
        .unwrap();

    let size = |doc: &mut Document| {
        let job = registry
            .rebuild_jobs(doc)
            .into_iter()
            .find(|j| j.body == body)
            .expect("a rebuild");
        let result = OgeomKernel::new()
            .execute_solid_chain(&job.plan.unwrap().ops, &TessellationSettings::default())
            .unwrap();
        let (min, max) = mesh_bounds(&result.mesh);
        [max[0] - min[0], max[1] - min[1], max[2] - min[2]]
    };
    let [x, y, z] = size(&mut doc);
    assert!(
        (x - 24.0).abs() < 1e-3,
        "the sketch solved to the variable: {x}"
    );
    assert!((y - 5.0).abs() < 1e-3, "{y}");
    assert!((z - 6.0).abs() < 1e-3, "the pad read Profile.width: {z}");

    let mut set = VariableSet::from_json(doc.get_feature_data(sizes).unwrap()).unwrap();
    set.variables[0].formula = "16 mm".into();
    doc.update_feature_data(sizes, set.to_json()).unwrap();
    let [x, _, z] = size(&mut doc);
    assert!((x - 16.0).abs() < 1e-3, "{x}");
    assert!((z - 4.0).abs() < 1e-3, "{z}");
}

/// An M6 hole with its thread modeled: the thread's groove is cut into the
/// tap-drilled wall, out toward the M6 major diameter, a closed solid.
#[test]
fn a_modeled_thread_cuts_its_groove_into_the_hole_wall() {
    let (mut doc, body, rect_id) = setup(20.0, 20.0);
    doc.add_feature_in_body(
        pad_feature(rect_id, 10.0, false, false),
        "Pad".into(),
        Some(body),
    )
    .unwrap();
    let top_face = wb_sketch::sketch::SketchPlane::from_face([10.0, 10.0, 10.0], [0.0, 0.0, 1.0]);
    let mut holes = Sketch::new("holes");
    holes.plane = top_face;
    holes.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 10.0))));
    let holes_id = doc
        .add_feature_in_body(
            SketchFeature::new(holes, top_face),
            "holes".into(),
            Some(body),
        )
        .unwrap();
    let m6 = wb_part::METRIC_SIZES
        .iter()
        .position(|(name, ..)| *name == "M6")
        .unwrap();
    let hole = |modeled_thread: bool| PartFeature::Hole {
        refine: false,
        sketch: holes_id,
        diameter: 5.0,
        depth: 8.0,
        through_all: false,
        cut: wb_part::HoleCut::None,
        metric_index: Some(m6),
        threaded: true,
        modeled_thread,
        thread_depth: 6.0,
        fit: wb_part::HoleFit::Normal,
        reversed: false,
    };
    let hole_id = doc
        .add_feature_in_body(hole(false), "Hole".into(), Some(body))
        .unwrap();
    let mut kernel = OgeomKernel::new();
    let mut volume = |doc: &Document| {
        let plan = wb_part::body_build_ops(doc, body).unwrap();
        let result = kernel
            .execute_solid_chain(&plan.ops, &TessellationSettings::default())
            .unwrap_or_else(|e| panic!("builds: {e}"));
        kernel
            .physical_properties(&result.brep_blob)
            .expect("measures")
            .volume_mm3
            .expect("a volume")
    };
    let tapped = volume(&doc);
    let block = 20.0 * 20.0 * 10.0;
    let drill = std::f64::consts::PI * 2.5 * 2.5 * 8.0;
    assert!((tapped - (block - drill)).abs() < 0.01, "{tapped}");
    doc.update_feature_data(hole_id, serde_json::to_value(hole(true)).unwrap())
        .unwrap();
    let threaded = volume(&doc);
    let major = std::f64::consts::PI * 3.0 * 3.0 * 8.0;
    assert!(
        threaded < tapped - 5.0 && threaded > block - major,
        "the groove takes some of the wall, not all of it: {threaded} (tapped {tapped})"
    );
}

/// A box off the origin mirrored across each base plane and across one of
/// its own faces: each copy lands on the far side of that plane.
#[test]
fn mirrored_copies_the_pad_across_every_plane_it_is_given() {
    use wb_part::{FacePick, MirrorPlane};
    let cases: [(MirrorPlane, [f32; 3], [f32; 3]); 4] = [
        (MirrorPlane::XY, [0.0, 0.0, -8.0], [10.0, 5.0, 8.0]),
        (MirrorPlane::XZ, [0.0, -5.0, 0.0], [10.0, 5.0, 8.0]),
        (MirrorPlane::YZ, [-10.0, 0.0, 0.0], [10.0, 5.0, 8.0]),
        (
            MirrorPlane::Face(FacePick {
                point: [10.0, 2.5, 4.0],
                normal: [1.0, 0.0, 0.0],
            }),
            [0.0, 0.0, 0.0],
            [20.0, 5.0, 8.0],
        ),
    ];
    let mut failures = Vec::new();
    for (plane, want_min, want_max) in cases {
        let (mut doc, body, sketch_id) = setup(10.0, 5.0);
        let pad = doc
            .add_feature_in_body(
                pad_feature(sketch_id, 8.0, false, false),
                "Pad".into(),
                Some(body),
            )
            .unwrap();
        doc.add_feature_in_body(
            PartFeature::Mirrored {
                refine: false,
                originals: vec![pad],
                plane,
            },
            "Mirror".into(),
            Some(body),
        )
        .unwrap();
        let plan = wb_part::body_build_ops(&doc, body).unwrap();
        let result =
            OgeomKernel::new().execute_solid_chain(&plan.ops, &TessellationSettings::default());
        let result = match result {
            Ok(result) => result,
            Err(e) => {
                failures.push(format!("{plane:?}: {e}"));
                continue;
            }
        };
        let (min, max) = mesh_bounds(&result.mesh);
        let fits = (0..3)
            .all(|i| (min[i] - want_min[i]).abs() < 1e-3 && (max[i] - want_max[i]).abs() < 1e-3);
        if !fits {
            failures.push(format!(
                "{plane:?}: bounds {min:?}..{max:?}, want {want_min:?}..{want_max:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// A quarter turn is not symmetric, so its mirror image shows whether the
/// copy turned the right way: across each base plane it must be the
/// original's reflection, filling the bounds the reflection fills.
#[test]
fn a_mirrored_revolution_turns_the_way_the_mirror_puts_it() {
    use wb_part::MirrorPlane;
    let build = |mirror: Option<MirrorPlane>| {
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Body".into()));
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
        let turn = doc
            .add_feature_in_body(
                PartFeature::Revolution {
                    refine: false,
                    sketch: sketch_id,
                    angle_deg: 90.0,
                    axis: wb_part::RevolveAxis::SketchY,
                    reversed: false,
                    midplane: false,
                    second_angle_deg: None,
                },
                "Revolution".into(),
                Some(body),
            )
            .unwrap();
        if let Some(plane) = mirror {
            doc.add_feature_in_body(
                PartFeature::Mirrored {
                    refine: false,
                    originals: vec![turn],
                    plane,
                },
                "Mirror".into(),
                Some(body),
            )
            .unwrap();
        }
        let ops = wb_part::body_build_ops(&doc, body).unwrap().ops;
        let result = OgeomKernel::new()
            .execute_solid_chain(&ops, &TessellationSettings::default())
            .unwrap();
        mesh_bounds(&result.mesh)
    };
    let (min, max) = build(None);
    for (plane, axis) in [
        (MirrorPlane::YZ, 0),
        (MirrorPlane::XZ, 1),
        (MirrorPlane::XY, 2),
    ] {
        let (got_min, got_max) = build(Some(plane));
        for i in 0..3 {
            let (want_lo, want_hi) = if i == axis {
                (min[i].min(-max[i]), max[i].max(-min[i]))
            } else {
                (min[i], max[i])
            };
            assert!(
                (got_min[i] - want_lo).abs() < 0.1 && (got_max[i] - want_hi).abs() < 0.1,
                "{plane:?} axis {i}: {:?}..{:?}, want {want_lo}..{want_hi} (original {min:?}..{max:?})",
                got_min[i],
                got_max[i]
            );
        }
    }
}

/// A sketch drawn on a datum plane follows it: the pad on the sketch moves
/// when the datum is moved, turned or flipped, and settles once rebuilt.
#[test]
fn a_pad_on_a_datum_sketch_follows_the_datum() {
    use core_document::{
        AttachmentOffset, BasePlane, DatumAttachment, DatumFeature, DatumShape, DocumentService,
        WorkbenchFeature,
    };
    let mut registry = DocumentService::default();
    registry
        .register_workbench(Box::new(wb_sketch::SketchWorkbench::default()))
        .unwrap();
    registry
        .register_workbench(Box::new(wb_part::PartDesignWorkbench::default()))
        .unwrap();
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let datum_at = |z: f32, rotation_deg: f32, flip: bool| DatumFeature {
        shape: DatumShape::Plane { size: 20.0 },
        attachment: DatumAttachment::BasePlane(BasePlane::XY),
        offset: AttachmentOffset {
            translation: [0.0, 0.0, z],
            rotation_deg,
            flip,
        },
    };
    let datum = doc
        .add_feature_in_body(datum_at(10.0, 0.0, false), "Datum".into(), Some(body))
        .unwrap();
    // A 4 x 2 rectangle drawn on the datum, as `sketch.new{on = datum}` makes it.
    let mut sketch = rect_sketch(4.0, 2.0);
    sketch.support = Some(wb_sketch::DatumSupport {
        datum,
        plane: None,
        offset: 0.0,
    });
    let sketch_id = doc
        .add_feature_in_body(sketch, "sketch".into(), Some(body))
        .unwrap();
    doc.add_feature_in_body(
        pad_feature(sketch_id, 3.0, false, false),
        "Pad".into(),
        Some(body),
    )
    .unwrap();

    let bounds = |doc: &mut Document| {
        let job = registry
            .rebuild_jobs(doc)
            .into_iter()
            .find(|j| j.body == body)
            .expect("a rebuild");
        let result = OgeomKernel::new()
            .execute_solid_chain(&job.plan.unwrap().ops, &TessellationSettings::default())
            .unwrap();
        mesh_bounds(&result.mesh)
    };
    let near = |a: [f32; 3], b: [f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3);

    let (min, max) = bounds(&mut doc);
    assert!(
        near(min, [0.0, 0.0, 10.0]) && near(max, [4.0, 2.0, 13.0]),
        "{min:?}..{max:?}"
    );

    // Moved up: the pad goes with it.
    doc.update_feature_data(datum, datum_at(20.0, 0.0, false).to_json())
        .unwrap();
    let (min, max) = bounds(&mut doc);
    assert!(
        near(min, [0.0, 0.0, 20.0]) && near(max, [4.0, 2.0, 23.0]),
        "moved: {min:?}..{max:?}"
    );

    // Turned a quarter about its normal: the rectangle stands along Y.
    doc.update_feature_data(datum, datum_at(20.0, 90.0, false).to_json())
        .unwrap();
    let (min, max) = bounds(&mut doc);
    assert!(
        near(min, [-2.0, 0.0, 20.0]) && near(max, [0.0, 4.0, 23.0]),
        "turned: {min:?}..{max:?}"
    );

    // Flipped: the pad grows down from the datum.
    doc.update_feature_data(datum, datum_at(20.0, 0.0, true).to_json())
        .unwrap();
    let (min, max) = bounds(&mut doc);
    assert!(
        (max[2] - 20.0).abs() < 1e-3 && (min[2] - 17.0).abs() < 1e-3,
        "flipped: {min:?}..{max:?}"
    );
    assert!(registry.rebuild_jobs(&mut doc).is_empty(), "settled");
}

/// A symmetric pocket from the top face cuts half its depth into the
/// material and half into the air above it.
#[test]
fn a_symmetric_pocket_cuts_half_its_depth_each_way() {
    let removed = |symmetric: bool| {
        let (mut doc, body, rect_id) = setup(20.0, 20.0);
        doc.add_feature_in_body(
            pad_feature(rect_id, 10.0, false, false),
            "Pad".into(),
            Some(body),
        )
        .unwrap();
        let top = wb_sketch::sketch::SketchPlane {
            origin: [5.0, 5.0, 10.0],
            ..Default::default()
        };
        let hole = doc
            .add_feature_in_body(rect_sketch_on(top, 5.0, 5.0), "top".into(), Some(body))
            .unwrap();
        doc.add_feature_in_body(
            PartFeature::Pocket {
                refine: false,
                sketch: hole,
                depth: 4.0,
                reversed: false,
                symmetric,
                through_all: false,
                mode: wb_part::ExtrudeMode::Dimension,
                depth2: 0.0,
                taper_deg: 0.0,
                up_to_face: None,
                up_to_offset: 0.0,
            },
            "Pocket".into(),
            Some(body),
        )
        .unwrap();
        let mut kernel = OgeomKernel::new();
        let result = kernel
            .execute_solid_chain(
                &wb_part::body_build_ops(&doc, body).unwrap().ops,
                &TessellationSettings::default(),
            )
            .unwrap_or_else(|e| panic!("symmetric {symmetric}: {e:?}"));
        let volume = kernel
            .physical_properties(&result.brep_blob)
            .unwrap()
            .volume_mm3
            .expect("a closed solid measures");
        20.0 * 20.0 * 10.0 - volume
    };
    assert!((removed(false) - 100.0).abs() < 1e-6, "{}", removed(false));
    assert!((removed(true) - 50.0).abs() < 1e-6, "{}", removed(true));
}

/// Rebuild every body the way the host does, until nothing is left to
/// rebuild: plan, build, store each solid.
fn settle(doc: &mut Document, kernel: &mut OgeomKernel) {
    for _ in 0..16 {
        let jobs = wb_part::rebuild_jobs(doc);
        if jobs.is_empty() {
            return;
        }
        for job in jobs {
            let Ok(plan) = job.plan else { continue };
            if plan.ops.is_empty() {
                continue;
            }
            if let Ok(result) =
                kernel.execute_solid_chain(&plan.ops, &TessellationSettings::default())
            {
                doc.set_imported_brep_data(job.body, result.brep_blob, Vec::new());
                doc.set_imported_geometry(
                    job.body,
                    core_document::ImportedGeometry {
                        mesh: std::sync::Arc::new(result.mesh),
                        source_asset: None,
                        revision: 0,
                        bounds_mm: result.bounds_mm,
                        brep_blob_path: None,
                        face_colors_path: None,
                        health: None,
                    },
                );
            }
        }
    }
    panic!("the rebuilds never settled");
}

fn volume_of_body(doc: &Document, kernel: &mut OgeomKernel, body: BodyId) -> f64 {
    let blob = doc.imported_brep_blob_arc(body).expect("a solid");
    kernel
        .physical_properties(&blob)
        .unwrap()
        .volume_mm3
        .expect("a closed solid")
}

/// A Boolean follows its tool body: made before the tool is built, it
/// waits for it, and a change to the tool rebuilds it.
#[test]
fn a_boolean_follows_its_tool_body() {
    let (mut doc, target, base) = setup(20.0, 20.0);
    doc.add_feature_in_body(
        pad_feature(base, 10.0, false, false),
        "Pad".into(),
        Some(target),
    )
    .unwrap();
    let tool = doc.create_body(Some("Tool".into()));
    doc.add_feature_in_body(
        PartFeature::BodyBoolean {
            refine: false,
            tool_body: tool,
            kind: kernel_api::BoolKind::Cut,
        },
        "Boolean".into(),
        Some(target),
    )
    .unwrap();
    let circle = doc
        .add_feature_in_body(
            circle_sketch_on(wb_sketch::sketch::SketchPlane::default(), 10.0, 10.0, 5.0),
            "circle".into(),
            Some(tool),
        )
        .unwrap();
    let cylinder = doc
        .add_feature_in_body(
            pad_feature(circle, 10.0, false, false),
            "Cylinder".into(),
            Some(tool),
        )
        .unwrap();

    // As a document opens: everything to build.
    wb_part::mark_all_part_features_dirty(&mut doc);
    let mut kernel = OgeomKernel::new();
    settle(&mut doc, &mut kernel);
    let pi = std::f64::consts::PI;
    let expect = |depth: f64| 4000.0 - pi * 25.0 * depth;
    let got = volume_of_body(&doc, &mut kernel, target);
    assert!((got - expect(10.0)).abs() < 1e-3, "{got}");
    assert!(
        doc.feature_tree()
            .all_nodes()
            .all(|(_, n)| n.error.is_none()),
        "nothing failed on the way"
    );

    doc.update_feature_data(
        cylinder,
        core_document::WorkbenchFeature::to_json(&pad_feature(circle, 5.0, false, false)),
    )
    .unwrap();
    doc.mark_feature_dirty(cylinder);
    settle(&mut doc, &mut kernel);
    let got = volume_of_body(&doc, &mut kernel, target);
    assert!((got - expect(5.0)).abs() < 1e-3, "{got}");
}

/// Two bodies that take each other as tools can never be built: the
/// Boolean says so rather than rebuilding them in turn forever.
#[test]
fn bodies_that_take_each_other_as_tools_fail_once() {
    let (mut doc, a, base) = setup(20.0, 20.0);
    doc.add_feature_in_body(pad_feature(base, 10.0, false, false), "Pad".into(), Some(a))
        .unwrap();
    let b = doc.create_body(Some("B".into()));
    let circle = doc
        .add_feature_in_body(
            circle_sketch_on(wb_sketch::sketch::SketchPlane::default(), 10.0, 10.0, 5.0),
            "circle".into(),
            Some(b),
        )
        .unwrap();
    doc.add_feature_in_body(
        pad_feature(circle, 10.0, false, false),
        "Cylinder".into(),
        Some(b),
    )
    .unwrap();
    for (body, tool) in [(a, b), (b, a)] {
        doc.add_feature_in_body(
            PartFeature::BodyBoolean {
                refine: false,
                tool_body: tool,
                kind: kernel_api::BoolKind::Fuse,
            },
            "Boolean".into(),
            Some(body),
        )
        .unwrap();
    }
    wb_part::mark_all_part_features_dirty(&mut doc);
    let mut kernel = OgeomKernel::new();
    settle(&mut doc, &mut kernel);
    let error = wb_part::body_build_ops(&doc, a).unwrap_err();
    assert!(
        error.message.contains("as a tool in turn"),
        "{}",
        error.message
    );
}
