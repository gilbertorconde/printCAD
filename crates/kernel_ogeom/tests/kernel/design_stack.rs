//! Full-stack Design test: sketch geometry → Pad/Pocket features →
//! `wb_design::body_build_ops` → `OgeomKernel::execute_solid_chain` → mesh.
//! This exercises the exact pipeline the app's recompute driver runs.

use core_document::{BodyId, Document, FeatureId};
use kernel_api::TessellationSettings;
use kernel_ogeom::OgeomKernel;
use wb_design::DesignFeature;
use wb_sketch::SketchFeature;
use wb_sketch::sketch::{Circle, GeometryElement, Line, Point, Sketch, SketchPlane, Vec2D};

use super::common::{
    Features, add_variables, body_volume, chain, extrude, is_sound, pad, pocket, primitive,
    read_shape, rebuilt, rect_at, registry, set_variable, settle_as_host, store_solid, try_chain,
    volume,
};

fn circle_sketch_on(
    plane: wb_sketch::sketch::SketchPlane,
    cx: f64,
    cy: f64,
    r: f64,
) -> SketchFeature {
    let mut sketch = Sketch::new("c");
    sketch.plane = plane;
    let center = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(cx, cy))));
    sketch.add_geometry(GeometryElement::Circle(Circle::new(center, r)));
    SketchFeature::new(sketch, plane)
}

fn setup(width: f64, height: f64) -> (Document, BodyId, FeatureId) {
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let sketch_id = doc.add_in(
        body,
        "sketch",
        rect_at(SketchPlane::xy(), 0.0, 0.0, width, height),
    );
    (doc, body, sketch_id)
}

fn pad_feature(sketch: FeatureId, length: f32, reversed: bool, symmetric: bool) -> DesignFeature {
    extrude(
        "Pad",
        sketch,
        serde_json::json!({"length": length, "reversed": reversed, "symmetric": symmetric}),
    )
}

fn mesh_bounds(mesh: &kernel_api::TriMesh) -> ([f32; 3], [f32; 3]) {
    mesh.bounds().expect("non-empty mesh")
}

/// What a body's features built.
struct Built(kernel_api::SolidBuildResult);

impl Built {
    /// The solid's volume as the kernel measures it.
    fn volume(&self) -> f64 {
        volume(&self.0.brep_blob)
    }

    /// The solid's bounds as the kernel measures them.
    fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        self.0.bounds_mm.expect("bounds")
    }
}

/// Build the body's features as the app does, or say why they do not.
fn build(doc: &Document, body: BodyId) -> Result<Built, String> {
    let ops = wb_design::body_build_ops(doc, body)
        .map_err(|e| e.message)?
        .ops;
    try_chain(&ops).map(Built).map_err(|e| e.to_string())
}

/// [`build`], which must succeed.
fn built(doc: &Document, body: BodyId) -> Built {
    build(doc, body).unwrap_or_else(|e| panic!("it builds: {e}"))
}

/// `sketch`, on its own plane in a body of its own, padded `length`.
fn padded(sketch: Sketch, length: f32) -> Built {
    let plane = sketch.plane;
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let id = doc.add_in(body, "sketch", SketchFeature::new(sketch, plane));
    doc.add_in(body, "Pad", pad(id, length));
    built(&doc, body)
}

/// A pad runs up its sketch's normal from the sketch plane, or straddles
/// it when symmetric; on the front plane the sketch's x is world X, its y
/// world Z, and the normal -Y.
#[test]
fn a_pad_extrudes_along_its_sketchs_normal() {
    use wb_sketch::sketch::SketchPlane;
    let cases = [
        (SketchPlane::xy(), false, [0.0, 0.0, 0.0], [10.0, 4.0, 6.0]),
        (SketchPlane::xy(), true, [0.0, 0.0, -3.0], [10.0, 4.0, 3.0]),
        (SketchPlane::xz(), false, [0.0, -6.0, 0.0], [10.0, 0.0, 4.0]),
    ];
    for (plane, symmetric, want_lo, want_hi) in cases {
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Body".into()));
        let sketch = doc.add_in(body, "s", rect_at(plane, 0.0, 0.0, 10.0, 4.0));
        doc.add_in(body, "Pad", pad_feature(sketch, 6.0, false, symmetric));
        let (lo, hi) = built(&doc, body).bounds();
        assert!(
            (0..3).all(|i| (lo[i] - want_lo[i]).abs() < 1e-3 && (hi[i] - want_hi[i]).abs() < 1e-3),
            "normal {:?}, symmetric {symmetric}: {lo:?}..{hi:?}",
            plane.normal
        );
    }
}

#[test]
fn editing_the_pad_length_changes_the_solid() {
    let (mut doc, body, sketch_id) = setup(10.0, 5.0);
    let pad_id = doc.add_in(body, "Pad", pad(sketch_id, 8.0));

    // Simulate the panel edit: update data, mark dirty, rebuild.
    doc.set_feature(pad_id, pad_feature(sketch_id, 3.0, true, false));
    doc.mark_feature_dirty(pad_id);
    assert_eq!(wb_design::pending_body_rebuilds(&doc), vec![body]);

    let (min, max) = built(&doc, body).bounds();
    assert!((max[2] - min[2] - 3.0).abs() < 1e-3, "new length");
    assert!(max[2].abs() < 1e-3, "reversed: solid below the plane");
}

#[test]
fn revolution_feature_builds_a_ring_through_the_full_stack() {
    // Rectangle x ∈ [5, 8], y ∈ [0, 2]: revolving about the sketch Y axis
    // sweeps a ring of outer radius 8.
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let ring = doc.add_in(body, "ring", ring_section());
    doc.add_in(body, "Revolution", revolution(ring, serde_json::json!({})));
    let (min, max) = built(&doc, body).bounds();
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

/// The rectangle x ∈ [5, 8], y ∈ [0, 2] on the XY plane.
fn ring_section() -> SketchFeature {
    rect_at(wb_sketch::sketch::SketchPlane::xy(), 5.0, 0.0, 8.0, 2.0)
}

/// The bounds of a revolution of [`ring_section`] about the sketch's Y
/// axis.
fn revolved_bounds(angle_deg: f32, start_angle_deg: f32, reversed: bool) -> ([f32; 3], [f32; 3]) {
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let ring = doc.add_in(body, "ring", ring_section());
    let turn = serde_json::json!({
        "angle_deg": angle_deg,
        "start_angle_deg": start_angle_deg,
        "reversed": reversed,
    });
    doc.add_in(body, "Revolution", revolution(ring, turn));
    built(&doc, body).bounds()
}

/// A quarter turn started a quarter turn on carries on where a quarter
/// turn from the profile ends: the two together are the half turn, either
/// way round.
#[test]
fn a_revolution_starts_turned_by_its_start_angle() {
    let near = |a: [f32; 3], b: [f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 0.1);
    for reversed in [false, true] {
        let first = revolved_bounds(90.0, 0.0, reversed);
        let next = revolved_bounds(90.0, 90.0, reversed);
        let half = revolved_bounds(180.0, 0.0, reversed);
        let lo = std::array::from_fn(|i| first.0[i].min(next.0[i]));
        let hi = std::array::from_fn(|i| first.1[i].max(next.1[i]));
        assert!(
            !near(first.0, next.0) || !near(first.1, next.1),
            "reversed {reversed}: turned"
        );
        assert!(
            near(lo, half.0) && near(hi, half.1),
            "reversed {reversed}: {first:?} and {next:?} against {half:?}"
        );
    }
}

/// Every edge of the 20 × 20 × 10 block rounded to radius 2: each edge
/// loses (1 - π/4) r² of section along its straight run, and each corner
/// the r cube less its ball's eighth.
#[test]
fn fillet_feature_rounds_the_pad_through_the_full_stack() {
    let (mut doc, body) = block();
    doc.add_in(
        body,
        "Fillet",
        DesignFeature::Fillet {
            radius: 2.0,
            edges: wb_design::EdgeSel::All,
            follow_tangent: false,
        },
    );
    let (a, b, c, r) = (20.0, 20.0, 10.0, 2.0f64);
    let edges = (1.0 - PI / 4.0) * r * r * (4.0 * (a + b + c) - 24.0 * r);
    let corners = 8.0 * r.powi(3) * (1.0 - PI / 6.0);
    let want = a * b * c - edges - corners;
    assert_near(built(&doc, body).volume(), want, 1e-4, "every edge rounded");
}

#[test]
fn hole_feature_drills_the_pad_through_the_full_stack() {
    let (mut doc, body, rect_id) = setup(30.0, 20.0);
    doc.add_in(body, "Pad", pad(rect_id, 6.0));

    // Two hole positions on the pad's top face, either side of its middle
    // (the plane's frame starts at the document origin).
    let top_face = wb_sketch::sketch::SketchPlane::from_face([15.0, 10.0, 6.0], [0.0, 0.0, 1.0]);
    let mut holes = Sketch::new("holes");
    holes.plane = top_face;
    for x in [7.0f64, 23.0] {
        let center = holes.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 10.0))));
        holes.add_geometry(GeometryElement::Circle(Circle::new(center, 1.0)));
    }
    let holes_id = doc.add_in(body, "holes", SketchFeature::new(holes, top_face));
    doc.add_in(
        body,
        "Hole",
        edited(plain_hole(holes_id, 4.0, 3.0), |f| {
            if let DesignFeature::Hole { through_all, .. } = f {
                *through_all = true;
            }
        }),
    );
    let drilled = built(&doc, body);
    let (min, max) = drilled.bounds();
    assert!((max[0] - min[0] - 30.0).abs() < 1e-3, "plate width kept");
    // Two bores of radius 2 through the 6 mm plate.
    let want = 30.0 * 20.0 * 6.0 - 2.0 * PI * 4.0 * 6.0;
    assert_near(drilled.volume(), want, 1e-6, "two bores through");
}

/// A plate 30 × 20 × 6 with an M3 clearance hole at its middle, drilled
/// `depth` deep (or through), with `trap`; its volume once built, or the
/// plan's refusal.
fn plate_with_nut_trap(depth: Option<f32>, trap: wb_design::NutTrap) -> Result<f64, String> {
    let (mut doc, body, rect_id) = setup(30.0, 20.0);
    doc.add_in(body, "Pad", pad(rect_id, 6.0));
    let top_face = wb_sketch::sketch::SketchPlane::from_face([15.0, 10.0, 6.0], [0.0, 0.0, 1.0]);
    let mut holes = Sketch::new("holes");
    holes.plane = top_face;
    holes.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(15.0, 10.0))));
    let holes_id = doc.add_in(body, "holes", SketchFeature::new(holes, top_face));
    let hole = edited(plain_hole(holes_id, 3.4, depth.unwrap_or(6.0)), |f| {
        if let DesignFeature::Hole {
            through_all,
            thread,
            nut_trap,
            ..
        } = f
        {
            *through_all = depth.is_none();
            *thread = Some(iso("M3"));
            *nut_trap = Some(trap);
        }
    });
    doc.add_in(body, "Hole", hole);
    let built = build(&doc, body)?;
    let (min, max) = built.bounds();
    assert!(
        (max[2] - min[2] - 6.0).abs() < 1e-3,
        "the plate's thickness kept"
    );
    Ok(built.volume())
}

/// An M3 nut trap is a hexagon 5.5 mm across the flats and 2.4 mm thick
/// (ISO 4032), each with its 0.3 mm clearance, at the hole's mouth or
/// where a blind hole ends.
#[test]
fn a_nut_trap_pockets_the_nut_at_either_end_of_the_hole() {
    let plate = 30.0 * 20.0 * 6.0;
    let bore = std::f64::consts::PI * 1.7 * 1.7;
    let hex = 3f64.sqrt() / 2.0 * 5.8 * 5.8;
    let depth = 2.7;

    let top = plate_with_nut_trap(None, wb_design::NutTrap::default()).unwrap();
    let expected = plate - bore * (6.0 - depth) - hex * depth;
    assert!((top - expected).abs() < 1e-3, "{top} against {expected}");

    // A turned hexagon takes the same material.
    let turned = plate_with_nut_trap(
        None,
        wb_design::NutTrap {
            turn_deg: 30.0,
            ..Default::default()
        },
    )
    .unwrap();
    assert!((turned - expected).abs() < 1e-3);

    // At the bottom of a hole 5 deep: the nut buried under 1 mm.
    let bottom = plate_with_nut_trap(
        Some(5.0),
        wb_design::NutTrap {
            side: wb_design::NutSide::Bottom,
            ..Default::default()
        },
    )
    .unwrap();
    let expected = plate - bore * (5.0 - depth) - hex * depth;
    assert!(
        (bottom - expected).abs() < 1e-3,
        "{bottom} against {expected}"
    );

    // An own size and depth stand in for the nut's.
    let own = plate_with_nut_trap(
        None,
        wb_design::NutTrap {
            across_flats: Some(7.0),
            depth: Some(3.0),
            ..Default::default()
        },
    )
    .unwrap();
    let hex7 = 3f64.sqrt() / 2.0 * 49.0;
    let expected = plate - bore * 3.0 - hex7 * 3.0;
    assert!((own - expected).abs() < 1e-3, "{own} against {expected}");

    let refused = plate_with_nut_trap(
        None,
        wb_design::NutTrap {
            side: wb_design::NutSide::Bottom,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(refused.contains("depth"), "{refused}");
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
        let mesh = padded(sketch, 10.0).0.mesh;
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
        doc.add_in(body, "Pad", pad(sketch_id, 10.0));
        let raised = wb_sketch::sketch::SketchPlane {
            origin: [0.0, 0.0, 10.0],
            ..wb_sketch::sketch::SketchPlane::xy()
        };
        let upper = doc.add_in(body, "upper", rect_at(raised, 0.0, 0.0, 20.0, 20.0));
        let mut second = pad(upper, 10.0);
        second.set_refine(refine);
        doc.add_in(body, "Pad001", second);
        let mesh = built(&doc, body).0.mesh;
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
    let (a, b, height) = (10.0f64, 5.0f64, 4.0f32);
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
    let result = padded(sketch, height);
    let (lo, hi) = result.bounds();
    for (got, want) in [
        (lo[0], -a),
        (hi[0], a),
        (lo[1], 0.0),
        (hi[1], b),
        (hi[2] - lo[2], f64::from(height)),
    ] {
        assert!(
            (f64::from(got) - want).abs() < 1e-3,
            "bounds {lo:?}..{hi:?}"
        );
    }
    // The elliptic wall is a line swept along an ellipse, which the kernel
    // integrates in closed form: the volume is exact, and says so.
    let props = OgeomKernel::new()
        .physical_properties(&result.0.brep_blob)
        .unwrap();
    let volume = props.volume_mm3.expect("a closed solid");
    let expected = std::f64::consts::PI * a * b / 2.0 * f64::from(height);
    assert!(!props.approximate);
    assert!(
        (volume - expected).abs() < 1e-5 * expected,
        "volume {volume} vs {expected}"
    );
}

/// A circle made into a spline is the same circle, a rational one: it pads
/// to the cylinder the circle would.
#[test]
fn a_circle_made_into_a_spline_pads_to_its_cylinder() {
    use wb_sketch::sketch::Circle;
    let (r, height) = (6.0f64, 3.0f32);
    let mut sketch = Sketch::new("disc");
    let center = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
    let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, r)));
    let made = wb_sketch::spline_edit::to_bspline(&mut sketch, &[circle].into_iter().collect());
    assert!(made.changed);
    let volume = padded(sketch, height).volume();
    let expected = std::f64::consts::PI * r * r * f64::from(height);
    assert!(
        (volume - expected).abs() < 1e-3 * expected,
        "volume {volume} vs {expected}"
    );
}

/// Text pads as any profile does: letters with holes keep them, and a
/// letter apart from the rest makes a solid of its own.
#[test]
fn text_pads_to_raised_letters() {
    let mut sketch = Sketch::new("label");
    let spec = wb_sketch::text::TextSpec {
        text: "Oi".into(),
        size: 10.0,
        ..Default::default()
    };
    wb_sketch::text::add(&mut sketch, Vec2D::new(0.0, 0.0), &spec).unwrap();
    let letters = padded(sketch, 1.0);
    let (lo, hi) = letters.bounds();
    assert!((hi[2] - lo[2] - 1.0).abs() < 1e-3);
    assert!(hi[0] - lo[0] > 8.0 && hi[1] - lo[1] > 6.0, "{lo:?}..{hi:?}");
    let volume = letters.volume();
    // Raised letters cover far less than their box: the O's counter and
    // the space around the i are open.
    let box_volume = f64::from((hi[0] - lo[0]) * (hi[1] - lo[1]));
    assert!(
        volume > 0.05 * box_volume && volume < 0.6 * box_volume,
        "{volume}"
    );
}

/// A block with a bore through it, the bore's top rim rounded: the fillet
/// a printed part's hole mouth takes most often.
#[test]
fn bore_rim_fillets() {
    let (mut doc, body, rect_id) = setup(40.0, 30.0);
    doc.add_in(body, "Pad", pad(rect_id, 12.0));
    let top = wb_sketch::sketch::SketchPlane {
        origin: [0.0, 0.0, 12.0],
        ..Default::default()
    };
    let bore = doc.add_in(body, "bore", circle_sketch_on(top, 20.0, 15.0, 6.0));
    doc.add_in(
        body,
        "Pocket",
        with(pocket(bore, 12.0), serde_json::json!({"through_all": true})),
    );
    doc.add_in(
        body,
        "Fillet",
        DesignFeature::Fillet {
            radius: 2.0,
            edges: wb_design::EdgeSel::Edges(vec![wb_design::EdgePick {
                faces: [0, 0],
                point: [26.0, 15.0, 12.0],
                direction: [0.0, 1.0, 0.0],
            }]),
            follow_tangent: false,
        },
    );

    // The fillet takes the ring of area rho^2 (1 - pi/4) off the rim, its
    // centroid rho (10 - 3 pi) / (3 (4 - pi)) out from it (Pappus).
    let (r, rho) = (6.0_f64, 2.0_f64);
    let pi = std::f64::consts::PI;
    let area = rho * rho * (1.0 - pi / 4.0);
    let out = rho * (10.0 - 3.0 * pi) / (3.0 * (4.0 - pi));
    let expected = 40.0 * 30.0 * 12.0 - pi * r * r * 12.0 - 2.0 * pi * (r + out) * area;
    let volume = built(&doc, body).volume();
    assert!(
        (volume - expected).abs() < 1e-3 * expected,
        "volume {volume} vs {expected}"
    );
}

#[test]
fn a_variable_drives_the_pad_and_changing_it_rebuilds_the_solid() {
    let registry = registry();
    let (mut doc, body, sketch_id) = setup(10.0, 5.0);
    let sizes = add_variables(
        &mut doc,
        "Sizes",
        &[("base", "4 mm"), ("height", "Sizes.base * 2 - 1 mm")],
    );
    let pad_id = doc.add_in(body, "Pad", pad(sketch_id, 8.0));
    doc.set_feature_formula(pad_id, "/Pad/length", Some("Sizes.height".into()))
        .unwrap();

    let height = |doc: &mut Document| {
        let (min, max) = rebuilt_bounds(&registry, doc, body);
        max[2] - min[2]
    };
    assert!((height(&mut doc) - 7.0).abs() < 1e-3, "2 * 4 - 1");

    // Change the variable: the pad rebuilds at the new height, and nothing
    // else asked for it.
    set_variable(&mut doc, sizes, 0, "6 mm");
    assert!((height(&mut doc) - 11.0).abs() < 1e-3, "2 * 6 - 1");
    assert!(registry.rebuild_jobs(&mut doc).is_empty(), "settled");
}

/// A custom direction's components take formulas like any number: a
/// variable tilts the pad, and changing it tilts it again.
#[test]
fn a_variable_tilts_a_pad_s_custom_direction() {
    let registry = registry();
    let (mut doc, body, sketch_id) = setup(10.0, 5.0);
    let lean = add_variables(&mut doc, "Tilt", &[("lean", "0")]);
    let mut pad = pad(sketch_id, 8.0);
    if let DesignFeature::Pad { direction, .. } = &mut pad {
        *direction = wb_design::ExtrudeDirection::Custom([0.0, 0.0, 1.0]);
    }
    let pad_id = doc.add_in(body, "Pad", pad);
    doc.set_feature_formula(pad_id, "/Pad/direction/Custom/1", Some("Tilt.lean".into()))
        .unwrap();
    // A feature is built once marked stale, as the application marks a
    // new one.
    doc.mark_feature_stale(pad_id);

    let depth = |doc: &mut Document| {
        let (min, max) = rebuilt_bounds(&registry, doc, body);
        max[1] - min[1]
    };
    assert!((depth(&mut doc) - 5.0).abs() < 1e-3, "straight up");
    set_variable(&mut doc, lean, 0, "1");
    assert!(depth(&mut doc) > 6.0, "leaning along Y");
}

#[test]
fn a_variable_drives_a_named_sketch_dimension_and_the_pad_on_it() {
    use wb_sketch::sketch::{Constraint, ConstraintKind};
    let registry = registry();

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
    let sizes = add_variables(&mut doc, "Sizes", &[("w", "24 mm")]);
    let sketch_id = doc.add_in(body, "Profile", SketchFeature::new(sketch, plane));
    doc.set_feature_formula(sketch_id, &width_key, Some("Sizes.w".into()))
        .unwrap();
    let pad_id = doc.add_in(body, "Pad", pad(sketch_id, 3.0));
    // The pad reads the sketch's named dimension too.
    doc.set_feature_formula(pad_id, "/Pad/length", Some("Profile.width / 4".into()))
        .unwrap();

    let size = |doc: &mut Document| {
        let (min, max) = rebuilt_bounds(&registry, doc, body);
        [max[0] - min[0], max[1] - min[1], max[2] - min[2]]
    };
    let [x, y, z] = size(&mut doc);
    assert!(
        (x - 24.0).abs() < 1e-3,
        "the sketch solved to the variable: {x}"
    );
    assert!((y - 5.0).abs() < 1e-3, "{y}");
    assert!((z - 6.0).abs() < 1e-3, "the pad read Profile.width: {z}");

    set_variable(&mut doc, sizes, 0, "16 mm");
    let [x, _, z] = size(&mut doc);
    assert!((x - 16.0).abs() < 1e-3, "{x}");
    assert!((z - 4.0).abs() < 1e-3, "{z}");
}

/// An M6 hole with its thread modeled: the thread's groove is cut into the
/// tap-drilled wall, out toward the M6 major diameter, a closed solid.
#[test]
fn a_modeled_thread_cuts_its_groove_into_the_hole_wall() {
    let built = |modeled| {
        drilled_block(20.0, 10.0, |s| tapped_hole(s, 8.0, iso("M6"), modeled))
            .unwrap_or_else(|e| panic!("builds: {e}"))
    };
    let tapped = built(None);
    let block = 20.0 * 20.0 * 10.0;
    let drill = PI * 2.5 * 2.5 * 8.0;
    assert!((tapped - (block - drill)).abs() < 0.01, "{tapped}");
    let threaded = built(Some(3.0));
    let major = PI * 3.0 * 3.0 * 8.0;
    assert!(
        threaded < tapped - 5.0 && threaded > block - major,
        "the groove takes some of the wall, not all of it: {threaded} (tapped {tapped})"
    );
}

/// `original` mirrored across `plane`.
fn mirrored(original: FeatureId, plane: wb_design::MirrorPlane) -> DesignFeature {
    DesignFeature::Mirrored {
        skipped: Vec::new(),
        refine: false,
        boolean_tolerance: None,
        originals: vec![original],
        plane,
    }
}

/// A box off the origin mirrored across each base plane and across one of
/// its own faces: each copy lands on the far side of that plane.
#[test]
fn mirrored_copies_the_pad_across_every_plane_it_is_given() {
    use wb_design::{FacePick, MirrorPlane};
    let cases: [(MirrorPlane, [f32; 3], [f32; 3]); 4] = [
        (MirrorPlane::XY, [0.0, 0.0, -8.0], [10.0, 5.0, 8.0]),
        (MirrorPlane::XZ, [0.0, -5.0, 0.0], [10.0, 5.0, 8.0]),
        (MirrorPlane::YZ, [-10.0, 0.0, 0.0], [10.0, 5.0, 8.0]),
        (
            MirrorPlane::Face(FacePick {
                name: 0,
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
        let pad = doc.add_in(body, "Pad", pad(sketch_id, 8.0));
        doc.add_in(body, "Mirror", mirrored(pad, plane));
        let (min, max) = match build(&doc, body) {
            Ok(result) => result.bounds(),
            Err(e) => {
                failures.push(format!("{plane:?}: {e}"));
                continue;
            }
        };
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
    use wb_design::MirrorPlane;
    let build = |mirror: Option<MirrorPlane>| {
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Body".into()));
        let ring = doc.add_in(body, "ring", ring_section());
        let turn = doc.add_in(
            body,
            "Revolution",
            revolution(ring, serde_json::json!({"angle_deg": 90.0})),
        );
        if let Some(plane) = mirror {
            doc.add_in(body, "Mirror", mirrored(turn, plane));
        }
        built(&doc, body).bounds()
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
    use core_document::{AttachmentOffset, BasePlane, DatumAttachment, DatumFeature, DatumShape};
    let registry = registry();
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let datum_at = |z: f32, rotation_deg: f32, flip: bool| DatumFeature {
        shape: DatumShape::Plane { size: 20.0 },
        attachment: DatumAttachment::BasePlane(BasePlane::XY),
        offset: AttachmentOffset {
            tilt: [0.0; 2],
            translation: [0.0, 0.0, z],
            rotation_deg,
            flip,
        },
    };
    let datum = doc.add_in(body, "Datum", datum_at(10.0, 0.0, false));
    // A 4 x 2 rectangle drawn on the datum, as `sketch.new{on = datum}` makes it.
    let mut sketch = rect_at(SketchPlane::xy(), 0.0, 0.0, 4.0, 2.0);
    sketch.support = Some(wb_sketch::DatumSupport {
        datum,
        plane: None,
        offset: 0.0,
        shift: [0.0, 0.0],
        turn: 0.0,
    });
    let sketch_id = doc.add_in(body, "sketch", sketch);
    doc.add_in(body, "Pad", pad(sketch_id, 3.0));

    let bounds = |doc: &mut Document| rebuilt_bounds(&registry, doc, body);
    let near = |a: [f32; 3], b: [f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3);

    let (min, max) = bounds(&mut doc);
    assert!(
        near(min, [0.0, 0.0, 10.0]) && near(max, [4.0, 2.0, 13.0]),
        "{min:?}..{max:?}"
    );

    // Moved up: the pad goes with it.
    doc.set_feature(datum, datum_at(20.0, 0.0, false));
    let (min, max) = bounds(&mut doc);
    assert!(
        near(min, [0.0, 0.0, 20.0]) && near(max, [4.0, 2.0, 23.0]),
        "moved: {min:?}..{max:?}"
    );

    // Turned a quarter about its normal: the rectangle stands along Y.
    doc.set_feature(datum, datum_at(20.0, 90.0, false));
    let (min, max) = bounds(&mut doc);
    assert!(
        near(min, [-2.0, 0.0, 20.0]) && near(max, [0.0, 4.0, 23.0]),
        "turned: {min:?}..{max:?}"
    );

    // Flipped: the pad grows down from the datum.
    doc.set_feature(datum, datum_at(20.0, 0.0, true));
    let (min, max) = bounds(&mut doc);
    assert!(
        (max[2] - 20.0).abs() < 1e-3 && (min[2] - 17.0).abs() < 1e-3,
        "flipped: {min:?}..{max:?}"
    );
    assert!(registry.rebuild_jobs(&mut doc).is_empty(), "settled");
}

/// The bounds of what the registry's rebuild of `body` builds.
fn rebuilt_bounds(
    registry: &core_document::DocumentService,
    doc: &mut Document,
    body: BodyId,
) -> ([f32; 3], [f32; 3]) {
    mesh_bounds(&rebuilt(registry, doc, body).mesh)
}

/// A pad starting on the top face of the pad before it stands on that
/// face, and follows it when the first pad grows.
#[test]
fn a_pad_starting_on_a_face_follows_the_face() {
    let registry = registry();
    let (mut doc, body, base) = setup(40.0, 40.0);
    let first = doc.add_in(body, "Pad", pad(base, 10.0));
    // A 4 x 4 square drawn on the ground, started on the top face.
    let ground = wb_sketch::sketch::SketchPlane {
        origin: [18.0, 18.0, 0.0],
        ..Default::default()
    };
    let small = doc.add_in(body, "small", rect_at(ground, 0.0, 0.0, 4.0, 4.0));
    let mut boss = pad(small, 5.0);
    if let DesignFeature::Pad { extras, .. } = &mut boss {
        extras.start_face = Some(wb_design::FacePick {
            point: [20.0, 20.0, 10.0],
            normal: [0.0, 0.0, 1.0],
            name: 0,
        });
    }
    doc.add_in(body, "Boss", boss);
    let top = |doc: &Document| {
        doc.imported_geometry(body)
            .and_then(|g| g.bounds_mm)
            .expect("a solid")
            .1[2]
    };
    settle_as_host(&registry, &mut doc);
    assert!((top(&doc) - 15.0).abs() < 1e-3, "{}", top(&doc));

    doc.set_feature(first, pad(base, 20.0));
    doc.mark_feature_dirty(first);
    settle_as_host(&registry, &mut doc);
    assert!((top(&doc) - 25.0).abs() < 1e-3, "followed: {}", top(&doc));
}

/// A symmetric pocket from the top face cuts half its depth into the
/// material and half into the air above it.
#[test]
fn a_symmetric_pocket_cuts_half_its_depth_each_way() {
    let removed = |symmetric: bool| {
        let (mut doc, body, rect_id) = setup(20.0, 20.0);
        doc.add_in(body, "Pad", pad(rect_id, 10.0));
        let top = wb_sketch::sketch::SketchPlane {
            origin: [5.0, 5.0, 10.0],
            ..Default::default()
        };
        let hole = doc.add_in(body, "top", rect_at(top, 0.0, 0.0, 5.0, 5.0));
        doc.add_in(
            body,
            "Pocket",
            with(
                pocket(hole, 4.0),
                serde_json::json!({"symmetric": symmetric}),
            ),
        );
        let volume = build(&doc, body)
            .unwrap_or_else(|e| panic!("symmetric {symmetric}: {e}"))
            .volume();
        20.0 * 20.0 * 10.0 - volume
    };
    assert!((removed(false) - 100.0).abs() < 1e-6, "{}", removed(false));
    assert!((removed(true) - 50.0).abs() < 1e-6, "{}", removed(true));
}

/// Rebuild every body the way the host does, until nothing is left to
/// rebuild: plan, build, store each solid.
fn settle(doc: &mut Document) {
    for _ in 0..16 {
        let jobs = wb_design::rebuild_jobs(doc);
        if jobs.is_empty() {
            return;
        }
        for job in jobs {
            let Ok(plan) = job.plan else { continue };
            if plan.ops.is_empty() {
                continue;
            }
            if let Ok(result) = try_chain(&plan.ops) {
                store_solid(doc, job.body, result);
            }
        }
    }
    panic!("the rebuilds never settled");
}

/// A Boolean follows its tool body: made before the tool is built, it
/// waits for it, and a change to the tool rebuilds it.
#[test]
fn a_boolean_follows_its_tool_body() {
    let (mut doc, target, base) = setup(20.0, 20.0);
    doc.add_in(target, "Pad", pad(base, 10.0));
    let tool = doc.create_body(Some("Tool".into()));
    doc.add_in(
        target,
        "Boolean",
        DesignFeature::BodyBoolean {
            more_tools: Vec::new(),
            refine: false,
            boolean_tolerance: None,
            tool_body: tool,
            kind: kernel_api::BoolKind::Cut,
        },
    );
    let circle = doc.add_in(
        tool,
        "circle",
        circle_sketch_on(wb_sketch::sketch::SketchPlane::default(), 10.0, 10.0, 5.0),
    );
    let cylinder = doc.add_in(tool, "Cylinder", pad(circle, 10.0));

    // As a document opens: everything to build.
    wb_design::mark_all_design_features_dirty(&mut doc);
    settle(&mut doc);
    let pi = std::f64::consts::PI;
    let expect = |depth: f64| 4000.0 - pi * 25.0 * depth;
    let got = body_volume(&doc, target);
    assert!((got - expect(10.0)).abs() < 1e-3, "{got}");
    assert!(
        doc.feature_tree()
            .all_nodes()
            .all(|(_, n)| n.error.is_none()),
        "nothing failed on the way"
    );

    doc.set_feature(cylinder, pad(circle, 5.0));
    doc.mark_feature_dirty(cylinder);
    settle(&mut doc);
    let got = body_volume(&doc, target);
    assert!((got - expect(5.0)).abs() < 1e-3, "{got}");
}

/// Two bodies that take each other as tools can never be built: the
/// Boolean says so rather than rebuilding them in turn forever.
#[test]
fn bodies_that_take_each_other_as_tools_fail_once() {
    let (mut doc, a, base) = setup(20.0, 20.0);
    doc.add_in(a, "Pad", pad(base, 10.0));
    let b = doc.create_body(Some("B".into()));
    let circle = doc.add_in(
        b,
        "circle",
        circle_sketch_on(wb_sketch::sketch::SketchPlane::default(), 10.0, 10.0, 5.0),
    );
    doc.add_in(b, "Cylinder", pad(circle, 10.0));
    for (body, tool) in [(a, b), (b, a)] {
        doc.add_in(
            body,
            "Boolean",
            DesignFeature::BodyBoolean {
                more_tools: Vec::new(),
                refine: false,
                boolean_tolerance: None,
                tool_body: tool,
                kind: kernel_api::BoolKind::Fuse,
            },
        );
    }
    wb_design::mark_all_design_features_dirty(&mut doc);
    settle(&mut doc);
    let error = wb_design::body_build_ops(&doc, a).unwrap_err();
    assert!(
        error.message.contains("as a tool in turn"),
        "{}",
        error.message
    );
}

/// A quartic drawn through points and closed by a line pads: the solid
/// reaches every point the curve was drawn through, and no further along x.
#[test]
fn a_spline_through_points_closed_by_a_line_pads() {
    use wb_sketch::sketch::BSpline;
    let clicks = [[0.0, 0.0], [4.0, 6.0], [9.0, 7.0], [14.0, 3.0], [18.0, 0.0]];
    let fit = sketch_solver::spline::interpolate(&clicks, 4, false).expect("a fit");
    let mut sketch = Sketch::new("spline");
    let fit_points: Vec<_> = clicks
        .iter()
        .map(|p| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(p[0], p[1])))))
        .collect();
    let control: Vec<_> = fit
        .control
        .iter()
        .enumerate()
        .map(|(i, p)| match i {
            0 => fit_points[0],
            4 => fit_points[4],
            _ => sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(p[0], p[1])))),
        })
        .collect();
    sketch.add_geometry(GeometryElement::BSpline(BSpline {
        degree: fit.degree,
        knots: fit.knots.clone(),
        fit_points: fit_points.clone(),
        fit_params: fit.params.clone(),
        ..BSpline::new(control, false)
    }));
    sketch.add_geometry(GeometryElement::Line(Line::new(
        fit_points[4],
        fit_points[0],
    )));
    let (lo, hi) = mesh_bounds(&padded(sketch, 3.0).0.mesh);
    assert!(
        lo[0].abs() < 1e-3 && (hi[0] - 18.0).abs() < 1e-2,
        "{lo:?}..{hi:?}"
    );
    assert!(hi[1] >= 7.0 - 1e-2 && hi[1] < 8.0, "reaches (9, 7): {hi:?}");
    assert!((hi[2] - lo[2] - 3.0).abs() < 1e-3);
}

/// A sketch of one conic arc from `start` to `end`, closed by a line, padded
/// `height`: the solid's volume.
fn padded_conic_volume(
    kind: wb_sketch::sketch::ConicKind,
    center: Vec2D,
    axis: Vec2D,
    minor: f64,
    start: Vec2D,
    end: Vec2D,
    height: f32,
) -> f64 {
    use wb_sketch::sketch::Conic;
    let mut sketch = Sketch::new("conic");
    let c = sketch.add_geometry(GeometryElement::Point(Point::new(center)));
    let s = sketch.add_geometry(GeometryElement::Point(Point::new(start)));
    let e = sketch.add_geometry(GeometryElement::Point(Point::new(end)));
    sketch.add_geometry(GeometryElement::Conic(Conic::new(
        kind, c, axis, minor, s, e,
    )));
    sketch.add_geometry(GeometryElement::Line(Line::new(e, s)));
    padded(sketch, height).volume()
}

/// A parabolic segment is two thirds of the rectangle round it, and a
/// hyperbolic one what its integral says: the arcs pad exactly.
#[test]
fn arcs_of_parabola_and_hyperbola_closed_by_a_line_pad_to_their_areas() {
    use wb_sketch::sketch::ConicKind;
    // y = x² / 8 from x = -4 to 4, cut off at y = 2.
    let volume = padded_conic_volume(
        ConicKind::Parabola,
        Vec2D::new(0.0, 0.0),
        Vec2D::new(0.0, 2.0),
        0.0,
        Vec2D::new(4.0, 2.0),
        Vec2D::new(-4.0, 2.0),
        3.0,
    );
    let expected = 2.0 / 3.0 * 8.0 * 2.0 * 3.0;
    assert!(
        (volume - expected).abs() < 1e-3 * expected,
        "volume {volume} vs {expected}"
    );

    // x = 3·sqrt(1 + y² / 1.5²), cut off at x = 5 (y = ±2).
    let volume = padded_conic_volume(
        ConicKind::Hyperbola,
        Vec2D::new(0.0, 0.0),
        Vec2D::new(3.0, 0.0),
        1.5,
        Vec2D::new(5.0, -2.0),
        Vec2D::new(5.0, 2.0),
        2.0,
    );
    let steps = 2000;
    let area: f64 = (0..steps)
        .map(|i| {
            let y = -2.0 + 4.0 * (i as f64 + 0.5) / steps as f64;
            (5.0 - 3.0 * (1.0 + y * y / 2.25).sqrt()) * 4.0 / steps as f64
        })
        .sum();
    let expected = area * 2.0;
    assert!(
        (volume - expected).abs() < 1e-3 * expected,
        "volume {volume} vs {expected}"
    );
}

/// A `size`×`size`×`height` block with one hole position at the middle of
/// its top face, and the hole `hole` makes from that sketch: the block's
/// volume once drilled.
fn drilled_block(
    size: f64,
    height: f32,
    hole: impl FnOnce(FeatureId) -> DesignFeature,
) -> Result<f64, String> {
    drilled_block_solid(size, height, hole).map(|drilled| drilled.volume())
}

/// As [`drilled_block`], the built solid itself.
fn drilled_block_solid(
    size: f64,
    height: f32,
    hole: impl FnOnce(FeatureId) -> DesignFeature,
) -> Result<Built, String> {
    let (mut doc, body, rect_id) = setup(size, size);
    doc.add_in(body, "Pad", pad(rect_id, height));
    let top_face = wb_sketch::sketch::SketchPlane::from_face(
        [size * 0.5, size * 0.5, f64::from(height)],
        [0.0, 0.0, 1.0],
    );
    let mut holes = Sketch::new("holes");
    holes.plane = top_face;
    holes.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(
        size * 0.5,
        size * 0.5,
    ))));
    let holes_id = doc.add_in(body, "holes", SketchFeature::new(holes, top_face));
    doc.add_in(body, "Hole", hole(holes_id));
    build(&doc, body)
}

/// A plain blind hole, `diameter` across and `depth` deep, for a test to
/// change.
fn plain_hole(sketch: FeatureId, diameter: f32, depth: f32) -> DesignFeature {
    DesignFeature::Hole {
        cosmetic_thread: false,
        start: Default::default(),
        clearance: None,
        thread_length: Default::default(),
        refine: false,
        boolean_tolerance: None,
        sketch,
        diameter,
        depth,
        through_all: false,
        cut: wb_design::HoleCut::None,
        thread: None,
        threaded: false,
        modeled_thread: false,
        thread_depth: 0.0,
        fit: wb_design::HoleFit::Normal,
        drill_point: wb_design::DrillPoint::Flat,
        point_in_depth: false,
        taper_deg: 0.0,
        nut_trap: None,
        reversed: false,
    }
}

/// An ISO metric coarse thread of `size`.
fn iso(size: &str) -> wb_design::ThreadSpec {
    wb_design::ThreadSpec::new(wb_design::ThreadStandard::IsoMetricCoarse, size)
}

/// A hole `depth` deep tapped for `spec`, its thread modeled that many mm
/// deep when `modeled` gives a depth.
fn tapped_hole(
    sketch: FeatureId,
    depth: f32,
    spec: wb_design::ThreadSpec,
    modeled: Option<f32>,
) -> DesignFeature {
    edited(plain_hole(sketch, 1.0, depth), |f| {
        if let DesignFeature::Hole {
            thread,
            threaded,
            modeled_thread,
            thread_depth,
            ..
        } = f
        {
            *thread = Some(spec);
            *threaded = true;
            *modeled_thread = modeled.is_some();
            *thread_depth = modeled.unwrap_or(0.0);
        }
    })
}

use std::f64::consts::PI;

/// A hole started 2 mm above the top face drills only the 2 mm of its
/// 4 mm depth that reach the material; started 3 mm in, all 4 mm, lower.
#[test]
fn a_hole_drills_from_where_its_start_puts_it() {
    let block = 20.0 * 20.0 * 10.0;
    let r = 3.0f64;
    let removed = |offset: f32| {
        let drilled = drilled_block(20.0, 10.0, |sketch| {
            edited(plain_hole(sketch, 6.0, 4.0), |f| {
                if let DesignFeature::Hole { start, .. } = f {
                    start.offset = offset;
                }
            })
        })
        .unwrap_or_else(|e| panic!("builds: {e}"));
        block - drilled
    };
    let above = removed(-2.0);
    assert!((above - PI * r * r * 2.0).abs() < 0.05, "{above}");
    let within = removed(3.0);
    assert!((within - PI * r * r * 4.0).abs() < 0.05, "{within}");
}

/// A 118° drill point below the depth adds its cone under the wall; within
/// the depth the wall stops short so the tip lands on the depth.
#[test]
fn an_angled_drill_point_cones_the_bottom_of_a_blind_hole() {
    let block = 20.0 * 20.0 * 10.0;
    let (r, depth) = (3.0f64, 7.0f64);
    let cone_height = r / 59f64.to_radians().tan();
    let cone = PI * r * r * cone_height / 3.0;
    for in_depth in [false, true] {
        let drilled = drilled_block(20.0, 10.0, |sketch| {
            edited(plain_hole(sketch, 6.0, 7.0), |f| {
                if let DesignFeature::Hole {
                    drill_point,
                    point_in_depth,
                    ..
                } = f
                {
                    *drill_point = wb_design::DrillPoint::Angled { angle_deg: 118.0 };
                    *point_in_depth = in_depth;
                }
            })
        })
        .unwrap_or_else(|e| panic!("builds: {e}"));
        let wall = if in_depth { depth - cone_height } else { depth };
        let want = block - PI * r * r * wall - cone;
        assert!(
            (drilled - want).abs() < 0.05,
            "point in depth {in_depth}: {} against {want}",
            drilled
        );
    }
}

/// A hole's cut and taper take what their shapes say from the 20 × 20 ×
/// 10 block: a counterdrill a wide bore to its depth, then a 90° cone
/// narrowing to the hole; a spotface a shallow seat around it; a taper a
/// frustum narrowing toward the bottom; and an ISO 4762 seat on an M6 hole
/// the DIN 974-1 counterbore, 11 mm across and 6.4 deep.
#[test]
fn a_hole_cuts_what_its_cut_and_taper_say() {
    use wb_design::HoleCut;
    let (r1, h) = (3.0f64, 8.0f64);
    let r2 = r1 - h * 5f64.to_radians().tan();
    /// What a row names, the hole's diameter and depth, the change to
    /// its cut, and the volume it takes.
    type Case = (&'static str, f32, f32, fn(&mut DesignFeature), f64);
    let cases: [Case; 4] = [
        (
            "counterdrill",
            4.0,
            8.0,
            |f| {
                if let DesignFeature::Hole { cut, .. } = f {
                    *cut = HoleCut::Counterdrill {
                        diameter: 8.0,
                        depth: 3.0,
                        angle_deg: 90.0,
                    };
                }
            },
            // Radius 4 to 3 deep, a cone from 4 to 2 over the next 2,
            // radius 2 to 8 deep.
            PI * 16.0 * 3.0 + PI * 2.0 / 3.0 * (16.0 + 8.0 + 4.0) + PI * 4.0 * 3.0,
        ),
        (
            "spotface",
            4.0,
            8.0,
            |f| {
                if let DesignFeature::Hole { cut, .. } = f {
                    *cut = HoleCut::Spotface {
                        diameter: 10.0,
                        depth: 0.5,
                    };
                }
            },
            PI * 25.0 * 0.5 + PI * 4.0 * 7.5,
        ),
        (
            "taper",
            6.0,
            8.0,
            |f| {
                if let DesignFeature::Hole { taper_deg, .. } = f {
                    *taper_deg = 5.0;
                }
            },
            PI * h / 3.0 * (r1 * r1 + r1 * r2 + r2 * r2),
        ),
        (
            "socket head seat",
            1.0,
            9.0,
            |f| {
                if let DesignFeature::Hole {
                    thread, cut, fit, ..
                } = f
                {
                    *thread = Some(iso("M6"));
                    *fit = wb_design::HoleFit::Normal;
                    *cut = HoleCut::Seat {
                        seat: wb_design::ScrewSeat::SocketHead,
                    };
                }
            },
            PI * 5.5 * 5.5 * 6.4 + PI * 3.3 * 3.3 * (9.0 - 6.4),
        ),
    ];
    for (what, diameter, depth, cut, removed) in cases {
        let drilled = drilled_block(20.0, 10.0, |sketch| {
            edited(plain_hole(sketch, diameter, depth), cut)
        })
        .unwrap_or_else(|e| panic!("{what}: {e}"));
        let want = 20.0 * 20.0 * 10.0 - removed;
        assert!(
            (drilled - want).abs() < 0.05,
            "{what}: {drilled} against {want}"
        );
    }
}

/// A tapped 1/4 NPT hole is drilled at the thread's minor diameter where
/// it opens and narrows 1:16 on the diameter toward its bottom. Its thread,
/// modeled, follows the taper: the groove cuts into the tapered wall and no
/// further out than the major diameter at the face.
#[test]
fn an_npt_hole_tapers_one_in_sixteen_and_its_thread_follows() {
    let npt = wb_design::ThreadStandard::Npt.size("1/4").unwrap();
    let built = |modeled| {
        let spec = wb_design::ThreadSpec::new(wb_design::ThreadStandard::Npt, "1/4");
        drilled_block(30.0, 15.0, |s| tapped_hole(s, 10.0, spec, modeled))
            .unwrap_or_else(|e| panic!("builds: {e}"))
    };
    let tapped = built(None);
    let r1 = npt.minor * 0.5;
    let r2 = r1 - 10.0 / 32.0;
    let want = 30.0 * 30.0 * 15.0 - PI * 10.0 / 3.0 * (r1 * r1 + r1 * r2 + r2 * r2);
    assert!((tapped - want).abs() < 0.05, "{tapped} against {want}");
    let threaded = built(Some(2.0));
    let outer = PI * (npt.major * 0.5).powi(2) * 7.0;
    assert!(
        threaded < tapped - 5.0 && threaded > tapped - outer,
        "the groove takes some of the wall, not all of it: {threaded} (tapped {tapped})"
    );
}

/// A left-hand modeled thread is the mirror of the right-hand one. A
/// quarter pitch of thread is the tail of a groove that enters the
/// material over its last turn, most of it past the turn's start, where
/// the section plane sits (+X). A right-hand thread turns by the
/// right-hand rule about the way it runs in (-Z), from +X toward -Y, so
/// its groove's wall lies on the -Y side of the axis; a left-hand one's on
/// the +Y side.
#[test]
fn a_left_hand_thread_turns_the_other_way() {
    let hole = |left_handed: bool| {
        move |sketch| {
            let mut spec = iso("M6");
            spec.left_handed = left_handed;
            tapped_hole(sketch, 8.0, spec, Some(0.25))
        }
    };
    // The mean offset from the axis of the mesh's points in the groove:
    // out past the drilled wall (radius 2.5) and below the face.
    let groove = |left_handed: bool| {
        let result = drilled_block_solid(20.0, 10.0, hole(left_handed))
            .unwrap_or_else(|e| panic!("builds: {e}"));
        let points: Vec<[f32; 3]> = result
            .0
            .mesh
            .positions
            .iter()
            .copied()
            .filter(|p| {
                let r = ((p[0] - 10.0).powi(2) + (p[1] - 10.0).powi(2)).sqrt();
                r > 2.6 && r < 3.5 && p[2] < 9.99
            })
            .collect();
        assert!(!points.is_empty(), "the groove is cut");
        let n = points.len() as f32;
        let mean = |k: usize| points.iter().map(|p| p[k] - 10.0).sum::<f32>() / n;
        [mean(0), mean(1)]
    };
    let (right, left) = (groove(false), groove(true));
    assert!(right[1] < -0.3, "right-hand runs toward -Y: {right:?}");
    assert!(left[1] > 0.3, "left-hand runs toward +Y: {left:?}");
    assert!(
        (right[0] - left[0]).abs() < 0.05 && (right[1] + left[1]).abs() < 0.05,
        "mirror images across the XZ plane: {right:?} {left:?}"
    );
}

/// A document written when a hole named its ISO metric size by
/// `metric_index` still loads, as that size, and builds the same drill.
#[test]
fn a_metric_index_hole_loads_as_its_iso_metric_size() {
    let (_, _, sketch) = setup(1.0, 1.0);
    let old = serde_json::json!({
        "Hole": {
            "sketch": sketch.0.to_string(),
            "diameter": 99.0,
            "depth": 4.0,
            "through_all": false,
            "cut": "None",
            "metric_index": 5,
            "threaded": true,
            "fit": "Normal",
            "reversed": false
        }
    });
    let feature: DesignFeature = serde_json::from_value(old).unwrap();
    let DesignFeature::Hole { thread, .. } = &feature else {
        panic!("a hole");
    };
    assert_eq!(
        thread.as_ref().map(|t| (t.standard, t.size.as_str())),
        Some((wb_design::ThreadStandard::IsoMetricCoarse, "M6"))
    );
    assert!((wb_design::hole_diameter(&feature) - 5.0).abs() < 1e-6);
    let drilled = drilled_block(20.0, 10.0, |sketch| {
        let mut json = serde_json::to_value(&feature).unwrap();
        json["Hole"]["sketch"] = serde_json::json!(sketch.0.to_string());
        serde_json::from_value(json).unwrap()
    })
    .unwrap_or_else(|e| panic!("builds: {e}"));
    let want = 20.0 * 20.0 * 10.0 - PI * 2.5 * 2.5 * 4.0;
    assert!((drilled - want).abs() < 0.01, "{}", drilled);
}

/// The plane z = `z`, sketched as the XY plane is.
fn plane_at_z(z: f64) -> wb_sketch::sketch::SketchPlane {
    wb_sketch::sketch::SketchPlane::from_frame([0.0, 0.0, z], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0])
}

/// The volume the body's features build, or why they do not.
fn built_volume(doc: &Document, body: BodyId) -> Result<f64, String> {
    build(doc, body).map(|solid| solid.volume())
}

fn assert_built_volume(doc: &Document, body: BodyId, want: f64, what: &str) {
    let got = built_volume(doc, body).unwrap_or_else(|e| panic!("{what}: {e}"));
    assert!(
        (got - want).abs() <= want * 1e-4,
        "{what}: volume {got}, want {want}"
    );
}

/// A 20 × 20 × 10 block, the body's first feature.
fn block() -> (Document, BodyId) {
    let (mut doc, body, sketch) = setup(20.0, 20.0);
    doc.add_in(body, "Block", pad(sketch, 10.0));
    (doc, body)
}

/// Set fields of a pad or pocket the way `design.set` does.
fn with(mut feature: DesignFeature, fields: serde_json::Value) -> DesignFeature {
    use core_document::WorkbenchFeature;
    let mut value = feature.to_json();
    let inner = value
        .as_object_mut()
        .and_then(|m| m.values_mut().next())
        .and_then(|v| v.as_object_mut())
        .unwrap();
    for (k, v) in fields.as_object().unwrap() {
        assert!(inner.contains_key(k), "no field {k}");
        inner.insert(k.clone(), v.clone());
    }
    feature = DesignFeature::from_json(&value).unwrap();
    feature
}

/// A pad runs along a vector or a picked edge as it is told, its length
/// measured along that way, and refuses one that lies in the sketch.
#[test]
fn a_pad_runs_along_a_custom_vector_or_a_picked_edge() {
    let (mut doc, body, sketch) = setup(10.0, 10.0);
    let slanted = 1000.0 * std::f64::consts::FRAC_1_SQRT_2;
    let pad_id = doc.add_in(
        body,
        "Pad",
        with(
            pad(sketch, 10.0),
            serde_json::json!({"direction": {"Custom": [0.0, 1.0, 1.0]}}),
        ),
    );
    assert_built_volume(&doc, body, slanted, "along a vector");
    let data = with(
        pad(sketch, 10.0),
        serde_json::json!({"direction": {"Edge": {"point": [0.0, 0.0, 0.0], "direction": [0.0, 1.0, 1.0]}}}),
    );
    doc.set_feature(pad_id, data);
    assert_built_volume(&doc, body, slanted, "along an edge");
    let data = with(
        pad(sketch, 10.0),
        serde_json::json!({"direction": {"Custom": [1.0, 0.0, 0.0]}}),
    );
    doc.set_feature(pad_id, data);
    let refused = built_volume(&doc, body).unwrap_err();
    assert!(refused.contains("plane"), "{refused}");
}

/// Each side of a two-sided pad ends its own way: up 5 from its sketch,
/// and down to the block's top (picked, or the first face it meets).
#[test]
fn each_side_of_a_two_sided_pad_ends_its_own_way() {
    for second in [
        serde_json::json!({"mode2": "UpToFace", "up_to_face2": {"point": [5.0, 5.0, 10.0], "normal": [0.0, 0.0, 1.0]}}),
        serde_json::json!({"mode2": "ToFirst"}),
        serde_json::json!({"mode": "TwoLengths", "mode2": "ToFirst"}),
    ] {
        let (mut doc, body) = block();
        let top = doc.add_in(body, "top", rect_at(plane_at_z(20.0), 0.0, 0.0, 10.0, 10.0));
        doc.add_in(body, "Pad", with(pad(top, 5.0), second.clone()));
        // The block, and 10 × 10 from z = 10 up to z = 25.
        assert_built_volume(&doc, body, 4000.0 + 1500.0, &second.to_string());
    }
}

/// A flat face of the solid is a profile: padded out of the material, or
/// pocketed into it, with no sketch.
#[test]
fn a_flat_face_of_the_solid_pads_and_pockets_with_no_sketch() {
    let top = serde_json::json!({"point": [10.0, 10.0, 10.0], "normal": [0.0, 0.0, 1.0]});
    let (mut doc, body) = block();
    let (_, _, any) = setup(1.0, 1.0);
    doc.add_in(
        body,
        "Pad",
        with(
            pad(any, 5.0),
            serde_json::json!({"sketch": null, "profile_face": top}),
        ),
    );
    assert_built_volume(&doc, body, 6000.0, "the top face padded 5");

    let (mut doc, body) = block();
    doc.add_in(
        body,
        "Pocket",
        with(
            pocket(any, 3.0),
            serde_json::json!({"sketch": null, "profile_face": top}),
        ),
    );
    assert_built_volume(&doc, body, 2800.0, "the top face pocketed 3");
}

/// Up to shape: a pad run down over the step's edge stops on the step's
/// top where it lies over the step and on the block's beside it.
#[test]
fn an_up_to_shape_pad_stops_where_each_line_first_meets_a_picked_face() {
    let (mut doc, body) = block();
    let step = doc.add_in(
        body,
        "step",
        rect_at(plane_at_z(10.0), 0.0, 0.0, 10.0, 20.0),
    );
    doc.add_in(body, "Step", pad(step, 5.0));
    let top = doc.add_in(body, "top", rect_at(plane_at_z(20.0), 4.0, 5.0, 16.0, 15.0));
    doc.add_in(
        body,
        "Pad",
        with(
            pad_feature(top, 5.0, true, false),
            serde_json::json!({
                "mode": "UpToShape",
                "up_to_shape": [
                    {"point": [5.0, 10.0, 15.0], "normal": [0.0, 0.0, 1.0]},
                    {"point": [15.0, 10.0, 10.0], "normal": [0.0, 0.0, 1.0]}
                ]
            }),
        ),
    );
    assert_built_volume(&doc, body, 4000.0 + 1000.0 + 300.0 + 600.0, "up to shape");
}

/// A 5 × 10 rectangle standing on the XZ plane at x in [5, 10], and a
/// wall x in [-20, 0], y in [0, 20], 10 high, beside the Z axis.
fn wall_and_profile() -> (Document, BodyId, FeatureId) {
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let wall = doc.add_in(
        body,
        "wall",
        rect_at(wb_sketch::sketch::SketchPlane::xy(), -20.0, 0.0, 0.0, 20.0),
    );
    doc.add_in(body, "Wall", pad(wall, 10.0));
    let profile = doc.add_in(
        body,
        "profile",
        rect_at(wb_sketch::sketch::SketchPlane::xz(), 5.0, 0.0, 10.0, 10.0),
    );
    (doc, body, profile)
}

fn revolution(sketch: FeatureId, fields: serde_json::Value) -> DesignFeature {
    with(
        DesignFeature::Revolution {
            regions: Vec::new(),
            refine: false,
            boolean_tolerance: None,
            sketch,
            angle_deg: 360.0,
            axis: wb_design::RevolveAxis::SketchY,
            project_axis: false,
            reversed: false,
            midplane: false,
            second_angle_deg: None,
            start_angle_deg: 0.0,
            mode: wb_design::RevolveMode::Angle,
            up_to_face: None,
        },
        fields,
    )
}

/// A revolution turns until it meets the wall: up to its first face a
/// quarter turn on, adding material there; a groove to its last face (or
/// that face picked), half a turn on, cuts the quarter through the wall.
#[test]
fn a_revolution_or_a_groove_turns_up_to_the_face_it_meets() {
    use core_document::WorkbenchFeature;
    let ring = 75.0 * std::f64::consts::PI * 10.0;
    let groove = |sketch, fields| {
        let json = revolution(sketch, fields).to_json();
        let inner = json.get("Revolution").cloned().unwrap();
        DesignFeature::from_json(&serde_json::json!({ "Groove": inner })).unwrap()
    };
    for (fields, cuts, change) in [
        (serde_json::json!({"mode": "ToFirst"}), false, ring / 4.0),
        (serde_json::json!({"mode": "ToLast"}), true, -ring / 4.0),
        (
            serde_json::json!({"mode": "UpToFace",
                "up_to_face": {"point": [-10.0, 0.0, 5.0], "normal": [0.0, -1.0, 0.0]}}),
            true,
            -ring / 4.0,
        ),
    ] {
        let (mut doc, body, profile) = wall_and_profile();
        let feature = if cuts {
            groove(profile, fields.clone())
        } else {
            revolution(profile, fields.clone())
        };
        doc.add_in(body, "Rev", feature);
        assert_built_volume(&doc, body, 4000.0 + change, &fields.to_string());
    }
}

/// A revolution turns about a line of its sketch, a datum line or a picked
/// edge as it does about the sketch's own axis; an edge across the sketch
/// plane is refused.
#[test]
fn a_revolution_turns_about_a_sketch_line_a_datum_line_or_a_picked_edge() {
    use core_document::{BasePlane, DatumAttachment, DatumFeature, DatumShape};
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let mut ring = rect_at(wb_sketch::sketch::SketchPlane::xy(), 5.0, 0.0, 8.0, 2.0);
    let a = ring
        .sketch
        .add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, -1.0))));
    let b = ring
        .sketch
        .add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 4.0))));
    let line = ring
        .sketch
        .add_geometry(GeometryElement::Line(Line::new(a, b)));
    ring.sketch.set_construction(line, true);
    let sketch = doc.add_in(body, "ring", ring);
    // A datum line along world Y: the YZ plane's x axis.
    let datum = doc.add_in(
        body,
        "Line",
        DatumFeature {
            shape: DatumShape::Line { length: 20.0 },
            attachment: DatumAttachment::BasePlane(BasePlane::YZ),
            offset: Default::default(),
        },
    );
    let want = std::f64::consts::PI * (64.0 - 25.0) * 2.0;
    let feature = doc.add_in(body, "Rev", revolution(sketch, serde_json::json!({})));
    let edge = |direction: [f32; 3]| {
        wb_design::RevolveAxis::Edge(wb_design::EdgePick {
            faces: [0, 0],
            point: [0.0, 3.0, 0.0],
            direction,
        })
    };
    for axis in [
        wb_design::RevolveAxis::SketchLine(line),
        wb_design::RevolveAxis::Datum(datum),
        edge([0.0, 1.0, 0.0]),
    ] {
        let data = revolution(
            sketch,
            serde_json::json!({"axis": serde_json::to_value(axis).unwrap()}),
        );
        doc.set_feature(feature, data);
        assert_built_volume(&doc, body, want, &format!("{axis:?}"));
    }
    let data = revolution(
        sketch,
        serde_json::json!({"axis": serde_json::to_value(edge([0.0, 0.0, 1.0])).unwrap()}),
    );
    doc.set_feature(feature, data);
    let refused = built_volume(&doc, body).unwrap_err();
    assert!(refused.contains("sketch plane"), "{refused}");
}

/// A 60 × 60 × 4 plate and a boss of radius 3, 6 high on its top at
/// (`x`, `y`): the document, the body, the plate's sketch and the boss.
fn plate_with_boss(x: f64, y: f64) -> (Document, BodyId, FeatureId, FeatureId) {
    let (mut doc, body, plate) = setup(60.0, 60.0);
    doc.add_in(body, "Plate", pad(plate, 4.0));
    let boss = doc.add_in(body, "boss", circle_sketch_on(plane_at_z(4.0), x, y, 3.0));
    let boss = doc.add_in(body, "Boss", pad(boss, 6.0));
    (doc, body, plate, boss)
}

/// The plate and `n` whole bosses.
fn plate_and_bosses(n: f64) -> f64 {
    60.0 * 60.0 * 4.0 + n * std::f64::consts::PI * 9.0 * 6.0
}

fn linear_pattern(original: FeatureId, fields: serde_json::Value) -> DesignFeature {
    with(
        DesignFeature::LinearPattern {
            skipped: Vec::new(),
            refine: false,
            boolean_tolerance: None,
            originals: vec![original],
            axis: wb_design::PatternAxis::X,
            length: 50.0,
            occurrences: 3,
            spacing_mode: false,
            spacings: Vec::new(),
            reversed: false,
        },
        fields,
    )
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.1
}

/// A linear pattern runs along a picked edge, a datum line or a sketch's
/// axis as it does along the body's Y axis: bosses at y = 10, 35 and 60,
/// the last overhanging the plate's far side.
#[test]
fn a_linear_pattern_runs_along_an_edge_a_datum_line_or_a_sketch_axis() {
    use core_document::{BasePlane, DatumAttachment, DatumFeature, DatumShape};
    let (mut doc, body, plate, boss) = plate_with_boss(10.0, 10.0);
    // The YZ plane's datum line runs along world Y.
    let datum = doc.add_in(
        body,
        "Line",
        DatumFeature {
            shape: DatumShape::Line { length: 20.0 },
            attachment: DatumAttachment::BasePlane(BasePlane::YZ),
            offset: Default::default(),
        },
    );
    let pattern = doc.add_in(body, "Pattern", linear_pattern(boss, serde_json::json!({})));
    let edge = wb_design::PatternAxis::Edge(wb_design::EdgePick {
        faces: [0, 0],
        point: [0.0, 30.0, 4.0],
        direction: [0.0, 1.0, 0.0],
    });
    let sketch_v = wb_design::PatternAxis::Sketch {
        sketch: plate,
        axis: wb_design::SketchAxis::Vertical,
    };
    for axis in [
        wb_design::PatternAxis::Y,
        edge,
        wb_design::PatternAxis::Datum(datum),
        sketch_v,
    ] {
        let data = linear_pattern(
            boss,
            serde_json::json!({"axis": serde_json::to_value(axis).unwrap()}),
        );
        doc.set_feature(pattern, data);
        let (volume, _, max) = built_body(&doc, body).unwrap();
        assert!(
            (volume - plate_and_bosses(3.0)).abs() < 1.0,
            "{axis:?}: volume {volume}"
        );
        assert!(
            close(max[1], 63.0) && close(max[0], 60.0),
            "{axis:?}: {max:?}"
        );
    }
}

/// Uneven spacing: the bosses 15 then 35 apart, the last overhanging the
/// plate's side; along the sketch's normal the copies stack up in z.
#[test]
fn an_uneven_linear_pattern_spaces_each_occurrence_its_own_way() {
    let (mut doc, body, _, boss) = plate_with_boss(10.0, 10.0);
    doc.add_in(
        body,
        "Pattern",
        linear_pattern(boss, serde_json::json!({"spacings": [15.0, 35.0]})),
    );
    let (volume, _, max) = built_body(&doc, body).unwrap();
    assert!((volume - plate_and_bosses(3.0)).abs() < 1.0, "{volume}");
    assert!(close(max[0], 63.0), "{max:?}");

    // Along the plate sketch's normal, the boss copied 6 and 20 up: one
    // column 6 + 6 high and one standing apart.
    let (mut doc, body, plate, boss) = plate_with_boss(10.0, 10.0);
    let normal = wb_design::PatternAxis::Sketch {
        sketch: plate,
        axis: wb_design::SketchAxis::Normal,
    };
    doc.add_in(
        body,
        "Pattern",
        linear_pattern(
            boss,
            serde_json::json!({
                "axis": serde_json::to_value(normal).unwrap(),
                "spacings": [6.0, 14.0]
            }),
        ),
    );
    let (volume, _, max) = built_body(&doc, body).unwrap();
    assert!((volume - plate_and_bosses(3.0)).abs() < 1.0, "{volume}");
    assert!(close(max[2], 30.0), "{max:?}");
}

/// A polar pattern by step about a round edge's axis: 90° a step, three
/// occurrences stand at 0°, 90° and 180° about the plate's centre, each
/// overhanging a side; the same angle as the overall one puts them at
/// 0°, 45° and 90°.
#[test]
fn a_polar_pattern_by_step_turns_each_occurrence_by_its_angle() {
    let axis = wb_design::PatternAxis::Edge(wb_design::EdgePick {
        faces: [0, 0],
        point: [30.0, 30.0, 10.0],
        direction: [0.0, 0.0, 1.0],
    });
    let polar = |boss, step_mode: bool, angles: &[f32]| DesignFeature::PolarPattern {
        skipped: Vec::new(),
        refine: false,
        boolean_tolerance: None,
        originals: vec![boss],
        axis,
        angle_deg: 90.0,
        occurrences: 3,
        reversed: false,
        step_mode,
        angles: angles.to_vec(),
    };
    let (mut doc, body, _, boss) = plate_with_boss(58.0, 30.0);
    let pattern = doc.add_in(body, "Pattern", polar(boss, true, &[]));
    let (volume, min, max) = built_body(&doc, body).unwrap();
    assert!((volume - plate_and_bosses(3.0)).abs() < 1.0, "{volume}");
    assert!(close(min[0], -1.0) && close(min[1], 0.0), "{min:?}");
    assert!(close(max[0], 61.0) && close(max[1], 61.0), "{max:?}");

    let data = polar(boss, false, &[]);
    doc.set_feature(pattern, data);
    let (_, min, _) = built_body(&doc, body).unwrap();
    assert!(close(min[0], 0.0), "overall 90°: {min:?}");

    // Uneven: 180° then 90° more stands the last at 270°, off the near side.
    let data = polar(boss, true, &[180.0, 90.0]);
    doc.set_feature(pattern, data);
    let (_, min, max) = built_body(&doc, body).unwrap();
    assert!(close(min[0], -1.0) && close(min[1], -1.0), "{min:?}");
    assert!(close(max[1], 60.0), "{max:?}");
}

/// Skipped occurrences are left out of the solid. Bosses at x = 1, 11
/// and 21, the first overhanging the plate's near side by 2: a copy
/// skipped leaves two bosses and the overhang, the original skipped takes
/// the overhang with it, all three leave the plate alone.
#[test]
fn a_pattern_leaves_its_skipped_occurrences_out() {
    let (mut doc, body, _, boss) = plate_with_boss(1.0, 10.0);
    let pattern = doc.add_in(
        body,
        "Pattern",
        linear_pattern(boss, serde_json::json!({"length": 20.0})),
    );
    let cases: [(&[u32], f64, f32); 4] = [
        (&[], 3.0, -2.0),
        (&[1], 2.0, -2.0),
        (&[0], 2.0, 0.0),
        (&[0, 1, 2], 0.0, 0.0),
    ];
    for (skipped, bosses, min_x) in cases {
        let data = linear_pattern(
            boss,
            serde_json::json!({"length": 20.0, "skipped": skipped}),
        );
        doc.set_feature(pattern, data);
        let (volume, min, max) = built_body(&doc, body).unwrap();
        assert!(
            (volume - plate_and_bosses(bosses)).abs() < 1.0,
            "{skipped:?}: volume {volume}"
        );
        assert!(close(min[0], min_x), "{skipped:?}: {min:?}");
        let height = if bosses > 0.0 { 10.0 } else { 4.0 };
        assert!(close(max[2], height), "{skipped:?}: {max:?}");
    }
}

/// A pocket patterned with its original skipped: the original's hole is
/// filled, the copies' cut. A mirror of the body's first solid cannot
/// leave that solid out, and says so.
#[test]
fn a_skipped_original_leaves_its_cut_unmade_and_the_first_solid_stays() {
    let (mut doc, body, plate) = setup(60.0, 20.0);
    let pad = doc.add_in(body, "Plate", pad(plate, 4.0));
    let at = doc.add_in(
        body,
        "hole",
        circle_sketch_on(plane_at_z(4.0), 10.0, 10.0, 2.0),
    );
    let hole = doc.add_in(body, "Hole", pocket(at, 4.0));
    doc.add_in(
        body,
        "Pattern",
        linear_pattern(hole, serde_json::json!({"length": 20.0, "skipped": [0]})),
    );
    let hole_volume = std::f64::consts::PI * 4.0 * 4.0;
    assert_built_volume(
        &doc,
        body,
        60.0 * 20.0 * 4.0 - 2.0 * hole_volume,
        "two holes",
    );

    doc.add_in(
        body,
        "Mirror",
        with(
            mirrored(pad, wb_design::MirrorPlane::YZ),
            serde_json::json!({"skipped": [0]}),
        ),
    );
    let refused = built_volume(&doc, body).unwrap_err();
    assert!(refused.contains("first solid"), "{refused}");
}

/// The block hollowed to walls of 1, open at the top, its walls joined as
/// `join` says; `inward` keeps the block's outside, else the walls grow
/// around it.
fn thickened_block(inward: bool, join: kernel_api::ThicknessJoin) -> Result<f64, String> {
    let (mut doc, body) = block();
    doc.add_in(
        body,
        "Thickness",
        DesignFeature::Thickness {
            both_sides: false,
            value: 1.0,
            faces: vec![wb_design::FacePick {
                name: 0,
                point: [10.0, 10.0, 10.0],
                normal: [0.0, 0.0, 1.0],
            }],
            inward,
            join,
        },
    );
    built_volume(&doc, body)
}

/// The block hollowed both ways and joined both ways. Inward on a convex
/// block either join leaves the block less an 18 × 18 × 9 cavity.
/// Outward, the intersection join runs the walls on to sharp corners (a
/// 22 × 22 × 11 box less the block), and the arc join rounds them about
/// every edge below the open top, radius 1: the block grown by a ball of
/// radius 1, cut flush at the top, less the block itself.
#[test]
fn a_thickness_joins_its_walls_by_intersection_or_arc() {
    use kernel_api::ThicknessJoin::{Arc, Intersection};
    let rounded = 1200.0 + 30.0 * PI + 2.0 / 3.0 * PI;
    for (inward, join, want) in [
        (true, Intersection, 1084.0),
        (false, Intersection, 1324.0),
        (true, Arc, 1084.0),
        (false, Arc, rounded),
    ] {
        let got = thickened_block(inward, join).unwrap();
        assert_near(got, want, 1e-4, &format!("inward {inward}, {join:?}"));
    }
}

/// The block with its vertical edge at x = y = 20 rounded to radius 3,
/// and a dress-up of the top edges on top of it.
fn rounded_corner_and(dress_up: DesignFeature) -> (Document, BodyId) {
    let (mut doc, body) = block();
    doc.add_in(
        body,
        "Corner",
        DesignFeature::Fillet {
            radius: 3.0,
            edges: wb_design::EdgeSel::Edges(vec![wb_design::EdgePick {
                faces: [0, 0],
                point: [20.0, 20.0, 5.0],
                direction: [0.0, 0.0, 1.0],
            }]),
            follow_tangent: false,
        },
    );
    doc.add_in(body, "Top", dress_up);
    (doc, body)
}

/// The volume of [`rounded_corner_and`] once built.
fn block_with_rounded_corner(dress_up: DesignFeature) -> Result<f64, String> {
    let (doc, body) = rounded_corner_and(dress_up);
    built_volume(&doc, body)
}

/// The top edges the rounded corner joins: the side at x = 20, the round,
/// and the side at y = 20.
fn top_chain() -> Vec<wb_design::EdgePick> {
    let d = 3.0 * std::f32::consts::FRAC_1_SQRT_2;
    vec![
        wb_design::EdgePick {
            faces: [0, 0],
            point: [20.0, 8.0, 10.0],
            direction: [0.0, 1.0, 0.0],
        },
        wb_design::EdgePick {
            faces: [0, 0],
            point: [17.0 + d, 17.0 + d, 10.0],
            direction: [-1.0, 1.0, 0.0],
        },
        wb_design::EdgePick {
            faces: [0, 0],
            point: [8.0, 20.0, 10.0],
            direction: [1.0, 0.0, 0.0],
        },
    ]
}

/// A fillet on one top edge beside the rounded corner takes the whole
/// tangent chain: the round and the side past it, as picking all three
/// does; the chain stops at the block's sharp corners. (A chamfer's chain
/// is `a_two_distance_chamfer_keeps_its_sides_along_a_tangent_chain`'s.)
#[test]
fn a_dress_up_on_one_edge_takes_its_tangent_chain() {
    let fillet = |edges: Vec<wb_design::EdgePick>, follow_tangent| DesignFeature::Fillet {
        radius: 1.0,
        edges: wb_design::EdgeSel::Edges(edges),
        follow_tangent,
    };
    let chain = top_chain();
    let one = vec![chain[0]];
    let all = block_with_rounded_corner(fillet(chain.clone(), false)).unwrap();
    let followed = block_with_rounded_corner(fillet(one.clone(), true)).unwrap();
    assert!(
        (all - followed).abs() < all * 1e-6,
        "the chain followed {followed}, picked edge by edge {all}"
    );
    let rounded_corner = 4000.0 - 9.0 * (1.0 - std::f64::consts::FRAC_PI_4) * 10.0;
    // Three edges' worth of material off: a round of radius 1 takes
    // (1 - pi/4) mm² of section along some 31 mm of edge.
    let taken = rounded_corner - followed;
    assert!(taken > 0.2146 * 25.0 && taken < 0.2146 * 40.0, "{taken}");
}

/// The area of the block's top face (z = 10) once built.
fn top_area(dress_up: DesignFeature) -> f64 {
    let (doc, body) = rounded_corner_and(dress_up);
    let solid = built(&doc, body);
    let mesh = &solid.0.mesh;
    mesh.indices
        .chunks(3)
        .map(|t| [0, 1, 2].map(|k| mesh.positions[t[k] as usize].map(f64::from)))
        .filter(|tri| tri.iter().all(|p| (p[2] - 10.0).abs() < 1e-4))
        .map(|[a, b, c]| {
            let (u, v) = ([0, 1].map(|k| b[k] - a[k]), [0, 1].map(|k| c[k] - a[k]));
            (u[0] * v[1] - u[1] * v[0]).abs() / 2.0
        })
        .sum()
}

/// A chamfer by two distances along a tangent chain keeps each distance
/// on its own side all the way: the side faces change from plane to
/// round to plane, and the top face shrinks by the same distance along
/// the whole chain, whichever side `flip` puts it on.
#[test]
fn a_two_distance_chamfer_keeps_its_sides_along_a_tangent_chain() {
    let chamfer = |flip| DesignFeature::Chamfer {
        size: 1.0,
        mode: wb_design::ChamferMode::TwoDistances,
        size2: 2.0,
        angle_deg: 45.0,
        flip,
        edges: wb_design::EdgeSel::Edges(vec![top_chain()[0]]),
        follow_tangent: true,
    };
    // The top inset by h along the chain: a (20 - h) square whose rounded
    // corner, about (17, 17), has radius 3 - h.
    let inset =
        |h: f64| (20.0 - h).powi(2) - (1.0 - std::f64::consts::FRAC_PI_4) * (3.0 - h).powi(2);
    let areas = [false, true].map(|flip| top_area(chamfer(flip)));
    let near = |a: f64, b: f64| (a - b).abs() < 1e-3 * b;
    assert!(
        (near(areas[0], inset(1.0)) && near(areas[1], inset(2.0)))
            || (near(areas[0], inset(2.0)) && near(areas[1], inset(1.0))),
        "top areas {areas:?}, want {} and {}",
        inset(1.0),
        inset(2.0)
    );
}

/// The volume and bounds of a body built from its features.
fn built_body(doc: &Document, body: BodyId) -> Result<(f64, [f32; 3], [f32; 3]), String> {
    let solid = build(doc, body)?;
    let (min, max) = solid.bounds();
    Ok((solid.volume(), min, max))
}

fn assert_near(got: f64, want: f64, rel: f64, what: &str) {
    assert!(
        (got - want).abs() <= want * rel,
        "{what}: {got}, want {want}"
    );
}

/// A sketch of one open chain of lines through `points`, on `plane`.
fn polyline_sketch(plane: wb_sketch::sketch::SketchPlane, points: &[[f64; 2]]) -> SketchFeature {
    let mut sketch = Sketch::new("path");
    sketch.plane = plane;
    let ids: Vec<_> = points
        .iter()
        .map(|p| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(p[0], p[1])))))
        .collect();
    for pair in ids.windows(2) {
        sketch.add_geometry(GeometryElement::Line(Line::new(pair[0], pair[1])));
    }
    SketchFeature::new(sketch, plane)
}

/// A quarter arc on the XY plane about (20, 0), radius 20, from the origin
/// (heading along -Y) to (20, -20): a path 10π long.
fn quarter_arc_sketch() -> SketchFeature {
    use wb_sketch::sketch::Arc;
    let plane = wb_sketch::sketch::SketchPlane::xy();
    let mut sketch = Sketch::new("arc");
    sketch.plane = plane;
    let center = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(20.0, 0.0))));
    let start = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
    let end = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(20.0, -20.0))));
    sketch.add_geometry(GeometryElement::Arc(Arc::new(center, start, end, 20.0)));
    SketchFeature::new(sketch, plane)
}

/// A body holding a pipe of `profile` along `path`, with the sketches the
/// pipe's options name added from `extra`.
fn pipe_body(
    profile: SketchFeature,
    path: SketchFeature,
    extra: &[SketchFeature],
    pipe: impl FnOnce(FeatureId, FeatureId, &[FeatureId]) -> DesignFeature,
) -> (Document, BodyId) {
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let profile = doc.add_in(body, "profile", profile);
    let path = doc.add_in(body, "path", path);
    let extra: Vec<FeatureId> = extra
        .iter()
        .map(|s| doc.add_in(body, "sketch", s.clone()))
        .collect();
    doc.add_in(body, "Pipe", pipe(profile, path, &extra));
    (doc, body)
}

fn pipe_of(
    profile: FeatureId,
    spine: FeatureId,
    orientation: wb_design::PipeOrientation,
    corner: wb_design::PipeCorner,
    sections: Vec<FeatureId>,
) -> DesignFeature {
    DesignFeature::Pipe {
        keep_inside: false,
        path_borrowed: Vec::new(),
        path_edges: Vec::new(),
        profile_face: None,
        refine: false,
        boolean_tolerance: None,
        profile,
        spine,
        orientation,
        corner,
        sections,
        subtractive: false,
    }
}

/// A 2 × 2 square on the XZ plane, the section a pipe carries.
fn square_section() -> SketchFeature {
    rect_at(wb_sketch::sketch::SketchPlane::xz(), 0.0, 0.0, 2.0, 2.0)
}

fn unit_circle_section() -> SketchFeature {
    circle_sketch_on(wb_sketch::sketch::SketchPlane::xz(), 0.0, 0.0, 1.0)
}

/// A binormal square to the plane of a bending path holds the section the
/// way the rotation-minimizing frame does: the same tube, which never
/// leans out of the plane.
#[test]
fn a_pipe_holding_the_binormal_of_its_paths_plane_builds() {
    use wb_design::{PipeCorner, PipeOrientation};
    for orientation in [
        PipeOrientation::Standard,
        PipeOrientation::Binormal {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    ] {
        let (doc, body) = pipe_body(square_section(), quarter_arc_sketch(), &[], |p, s, _| {
            pipe_of(p, s, orientation, PipeCorner::Transformed, Vec::new())
        });
        let (volume, min, max) = built_body(&doc, body).unwrap();
        assert_near(
            volume,
            4.0 * 10.0 * std::f64::consts::PI,
            5e-3,
            &format!("{orientation:?}"),
        );
        assert!(
            (max[2] - 1.0).abs() < 0.02 && (min[2] + 1.0).abs() < 0.02,
            "{orientation:?}: z from {} to {}",
            min[2],
            max[2]
        );
    }
    // Along a straight path any binormal off it holds.
    let straight = polyline_sketch(
        wb_sketch::sketch::SketchPlane::xy(),
        &[[0.0, 0.0], [0.0, 20.0]],
    );
    let (doc, body) = pipe_body(unit_circle_section(), straight, &[], |p, s, _| {
        let binormal = PipeOrientation::Binormal {
            x: 1.0,
            y: 0.0,
            z: 1.0,
        };
        pipe_of(p, s, binormal, PipeCorner::Transformed, Vec::new())
    });
    let (volume, ..) = built_body(&doc, body).unwrap();
    assert_near(volume, 20.0 * std::f64::consts::PI, 5e-3, "straight");
}

/// A binormal leaning off the plane of a bending path turns the section
/// about the path: the square that starts square to Z ends turned 45°
/// about the path, reaching √2 out of the plane.
#[test]
fn a_pipe_holding_a_leaning_binormal_turns_its_section() {
    use wb_design::{PipeCorner, PipeOrientation};
    let (doc, body) = pipe_body(square_section(), quarter_arc_sketch(), &[], |p, s, _| {
        let binormal = PipeOrientation::Binormal {
            x: 1.0,
            y: 0.0,
            z: 1.0,
        };
        pipe_of(p, s, binormal, PipeCorner::Transformed, Vec::new())
    });
    let (volume, _, max) = built_body(&doc, body).unwrap();
    assert_near(volume, 4.0 * 10.0 * std::f64::consts::PI, 5e-3, "volume");
    assert!(
        (max[2] - std::f32::consts::SQRT_2).abs() < 0.02,
        "{}",
        max[2]
    );
}

/// A section turned by an auxiliary path: the path runs beside a straight
/// spine from +X at its start round to +Z at its end, so the square turns a
/// quarter turn along the way, reaching √2 out along X halfway.
#[test]
fn a_pipe_oriented_by_an_auxiliary_path_turns_toward_it() {
    use wb_design::{PipeCorner, PipeOrientation};
    // The auxiliary line from (5, 0, 0) to (0, 20, 5), on the plane through
    // it square to (1, 0, 1).
    let plane = wb_sketch::sketch::SketchPlane::from_face([5.0, 0.0, 0.0], [1.0, 0.0, 1.0]);
    let at = |p: [f64; 3]| {
        let d = [
            p[0] - plane.origin[0],
            p[1] - plane.origin[1],
            p[2] - plane.origin[2],
        ];
        let dot = |a: [f64; 3]| d[0] * a[0] + d[1] * a[1] + d[2] * a[2];
        [dot(plane.x_axis), dot(plane.y_axis)]
    };
    let guide = polyline_sketch(plane, &[at([5.0, 0.0, 0.0]), at([0.0, 20.0, 5.0])]);
    let straight = polyline_sketch(
        wb_sketch::sketch::SketchPlane::xy(),
        &[[0.0, 0.0], [0.0, 20.0]],
    );
    let (doc, body) = pipe_body(square_section(), straight, &[guide], |p, s, extra| {
        let guided = PipeOrientation::Auxiliary { path: extra[0] };
        pipe_of(p, s, guided, PipeCorner::Transformed, Vec::new())
    });
    let (volume, _, max) = built_body(&doc, body).unwrap();
    assert_near(volume, 80.0, 5e-3, "volume");
    assert!(
        (max[0] - std::f32::consts::SQRT_2).abs() < 0.02,
        "{}",
        max[0]
    );
}

/// The path of the corner tests: an L of two 10 mm legs.
fn l_path() -> SketchFeature {
    polyline_sketch(
        wb_sketch::sketch::SketchPlane::xy(),
        &[[0.0, 0.0], [0.0, 10.0], [10.0, 10.0]],
    )
}

/// The unit circle round the L, from either end of it: transformed
/// corners mitre the path, the tube as long as it; from the far end the
/// profile sits there, so the path runs from there. Right corners run
/// each leg on past the corner by the section's radius and fuse the two:
/// two 11 mm cylinders less the bicylinder they share (16/3 for a unit
/// radius). Round corners end each leg square at the corner and turn the
/// section about it: the two 10 mm cylinders, less the quarter bicylinder
/// they share, and the quarter ball outside the corner.
#[test]
fn a_pipe_round_an_l_turns_its_corner_as_its_mode_says() {
    use wb_design::{PipeCorner, PipeOrientation};
    let far = circle_sketch_on(
        wb_sketch::sketch::SketchPlane::from_face([10.0, 10.0, 0.0], [0.0, -1.0, 0.0]),
        10.0,
        0.0,
        1.0,
    );
    let cases = [
        (
            "mitred",
            unit_circle_section(),
            PipeCorner::Transformed,
            20.0 * PI,
        ),
        (
            "mitred from the far end",
            far,
            PipeCorner::Transformed,
            20.0 * PI,
        ),
        (
            "right corner",
            unit_circle_section(),
            PipeCorner::Right,
            22.0 * PI - 16.0 / 3.0,
        ),
        (
            "round corner",
            unit_circle_section(),
            PipeCorner::Round,
            20.0 * PI - 4.0 / 3.0 + PI / 3.0,
        ),
    ];
    for (what, section, corner, want) in cases {
        let (doc, body) = pipe_body(section, l_path(), &[], |p, s, _| {
            pipe_of(p, s, PipeOrientation::Standard, corner, Vec::new())
        });
        assert_near(built(&doc, body).volume(), want, 5e-3, what);
    }
}

/// A path with no sharp corner turns none, whatever the corner mode.
#[test]
fn corner_modes_leave_a_smooth_path_alone() {
    use wb_design::{PipeCorner, PipeOrientation};
    for corner in PipeCorner::ALL {
        let (doc, body) = pipe_body(
            unit_circle_section(),
            quarter_arc_sketch(),
            &[],
            |p, s, _| pipe_of(p, s, PipeOrientation::Standard, corner, Vec::new()),
        );
        let (volume, ..) = built_body(&doc, body).unwrap();
        let pi = std::f64::consts::PI;
        assert_near(volume, 10.0 * pi * pi, 5e-3, &format!("{corner:?}"));
    }
}

/// A pipe through a second section: a circle of radius 2 at the start of a
/// straight 20 mm path growing to radius 3 at its end, the frustum between.
#[test]
fn a_pipe_through_two_sections_changes_its_shape_down_the_path() {
    use wb_design::{PipeCorner, PipeOrientation};
    let straight = polyline_sketch(
        wb_sketch::sketch::SketchPlane::xy(),
        &[[0.0, 0.0], [0.0, 20.0]],
    );
    let end = circle_sketch_on(
        wb_sketch::sketch::SketchPlane::from_face([0.0, 20.0, 0.0], [0.0, 1.0, 0.0]),
        0.0,
        0.0,
        3.0,
    );
    let start = circle_sketch_on(wb_sketch::sketch::SketchPlane::xz(), 0.0, 0.0, 2.0);
    let (doc, body) = pipe_body(start, straight, &[end], |p, s, extra| {
        let standard = PipeOrientation::Standard;
        pipe_of(p, s, standard, PipeCorner::Transformed, extra.to_vec())
    });
    let (volume, ..) = built_body(&doc, body).unwrap();
    let pi = std::f64::consts::PI;
    assert_near(volume, pi * 20.0 / 3.0 * (4.0 + 6.0 + 9.0), 1e-2, "frustum");
}

/// A 1 × 1 square 5 to 6 mm from the sketch's Y axis.
fn coil_section() -> SketchFeature {
    let mut sketch = Sketch::new("coil");
    let corners = [[5.0, 0.0], [6.0, 0.0], [6.0, 1.0], [5.0, 1.0]];
    let ids: Vec<_> = corners
        .iter()
        .map(|p| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(p[0], p[1])))))
        .collect();
    for i in 0..4 {
        sketch.add_geometry(GeometryElement::Line(Line::new(ids[i], ids[(i + 1) % 4])));
    }
    let plane = sketch.plane;
    SketchFeature::new(sketch, plane)
}

fn growing_helix(sketch: FeatureId, height: f32, turns: f32, growth: f32) -> DesignFeature {
    DesignFeature::Helix {
        refine: false,
        boolean_tolerance: None,
        sketch,
        axis: wb_design::RevolveAxis::SketchY,
        project_axis: false,
        mode: wb_design::HelixMode::HeightTurnsGrowth,
        pitch: 0.0,
        height,
        turns,
        left_handed: false,
        cone_angle_deg: 0.0,
        reversed: false,
        subtractive: false,
        growth,
        keep_inside: false,
    }
}

/// The volume the unit-area coil section sweeps turning `turns` times about
/// an axis in its plane, its centroid starting 5.5 mm out and moving
/// `growth` further out each turn: the area times the arc its centroid
/// turns through, however it climbs.
fn coil_volume(turns: f64, growth: f64) -> f64 {
    let pi = std::f64::consts::PI;
    2.0 * pi * turns * 5.5 + pi * growth * turns * turns
}

/// A helix given by height, turns and growth per turn widens as it
/// climbs: four turns out by a millimetre each end 6 + 4 out, 12 + 1 high.
/// Of no height it is a flat spiral one section high, each turn 2 mm
/// further out than the last.
#[test]
fn a_helix_grows_by_its_growth_per_turn_flat_or_climbing() {
    for (height, turns, growth, outer, tall, within) in [
        (12.0, 4.0, 1.0, 10.0, 13.0, 0.05),
        (0.0, 3.0, 2.0, 12.0, 1.0, 0.01),
    ] {
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Body".into()));
        let sketch = doc.add_in(body, "coil", coil_section());
        doc.add_in(body, "Helix", growing_helix(sketch, height, turns, growth));
        let (volume, min, max) = built_body(&doc, body).unwrap();
        let what = format!("height {height}");
        let want = coil_volume(f64::from(turns), f64::from(growth));
        assert_near(volume, want, 5e-3, &what);
        assert!((max[0] - outer).abs() < 0.1, "{what}: x to {}", max[0]);
        assert!(
            (max[1] - min[1] - tall).abs() < within,
            "{what}: {min:?}..{max:?}"
        );
    }
}

/// A 40 mm thick block from `x0` to 20 in X and -5 to 25 in Y, clear of the
/// coil's ends, and four turns of the coil 12 mm high cut from it, or kept
/// inside it.
fn block_and_coil(x0: f64, keep_inside: bool) -> (Document, BodyId) {
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let mut block = polyline_sketch(
        wb_sketch::sketch::SketchPlane::xy(),
        &[[x0, -5.0], [20.0, -5.0], [20.0, 25.0], [x0, 25.0]],
    );
    let corners: Vec<_> = block
        .sketch
        .geometry
        .iter()
        .filter_map(|e| match e {
            GeometryElement::Point(p) => Some(p.id),
            _ => None,
        })
        .collect();
    block
        .sketch
        .add_geometry(GeometryElement::Line(Line::new(corners[3], corners[0])));
    let block = doc.add_in(body, "block", block);
    doc.add_in(body, "Block", pad_feature(block, 40.0, false, true));
    let coil = doc.add_in(body, "coil", coil_section());
    let mut helix = growing_helix(coil, 12.0, 4.0, 0.0);
    if let DesignFeature::Helix {
        subtractive,
        keep_inside: keep,
        ..
    } = &mut helix
    {
        *subtractive = true;
        *keep = keep_inside;
    }
    doc.add_in(body, "Helix", helix);
    (doc, body)
}

/// A body's volume read off a fine mesh of it, by the divergence theorem,
/// and the build it was read from.
fn fine_mesh_volume(doc: &Document, body: BodyId) -> (f64, Built) {
    let ops = wb_design::body_build_ops(doc, body).unwrap().ops;
    let fine = TessellationSettings {
        linear_deflection_mode: kernel_api::LinearDeflectionMode::AbsoluteMm,
        chord_tolerance: 0.002,
        angular_tolerance_deg: 2.0,
        ..TessellationSettings::default()
    };
    let built = OgeomKernel::new().execute_solid_chain(&ops, &fine).unwrap();
    let mesh = &built.mesh;
    let mut volume = 0.0;
    for t in mesh.indices.chunks(3) {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize].map(f64::from));
        volume += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]))
            / 6.0;
    }
    (volume, Built(built))
}

/// What of the coil lies at X of at least `x0`: each point of the section,
/// `r` out, spends acos(x0 / r) of every half turn there, over four turns.
fn coil_beyond(x0: f64) -> f64 {
    let steps = 2000;
    (0..steps)
        .map(|i| {
            let r = 5.0 + (i as f64 + 0.5) / steps as f64;
            8.0 * r * (x0 / r).clamp(-1.0, 1.0).acos() / steps as f64
        })
        .sum()
}

/// A subtractive helix kept inside leaves only what it shares with the
/// body, which the kernel measures as its mesh does; not kept inside, it
/// cuts that away. On a block whose face holds its axis it keeps half.
#[test]
fn a_subtractive_helix_kept_inside_leaves_what_it_shares() {
    let shared = coil_beyond(-3.0);
    let (doc, body) = block_and_coil(-3.0, true);
    let (volume, kept) = fine_mesh_volume(&doc, body);
    assert_near(volume, shared, 2e-3, "kept inside");
    assert_near(kept.volume(), shared, 2e-3, "measured");
    let min = kept.bounds().0;
    assert!(min[0] > -3.01, "nothing past the block: {}", min[0]);

    let (doc, body) = block_and_coil(-3.0, false);
    let (volume, _) = fine_mesh_volume(&doc, body);
    assert_near(volume, 23.0 * 30.0 * 40.0 - shared, 1e-5, "cut away");

    // A block whose face holds the helix's axis keeps half of every turn.
    let (doc, body) = block_and_coil(0.0, true);
    let (volume, _) = fine_mesh_volume(&doc, body);
    assert_near(volume, coil_volume(4.0, 0.0) / 2.0, 2e-3, "half the coil");
}

/// A sketch standing through a box takes the line where the box's side
/// crosses its plane, fixed and out of profiles, and brings it up to the
/// box when the box has grown.
#[test]
fn an_intersection_reference_is_where_a_face_crosses_the_sketch_plane() {
    use core_document::{CommandArgs, Workbench, WorkbenchFeature, WorkbenchRuntimeContext};
    use kernel_api::PrimitiveKind;
    let cube = |height: f64| {
        let ops = [primitive(PrimitiveKind::Box {
            length: 10.0,
            width: 10.0,
            height,
        })];
        chain(&ops).brep_blob
    };
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    doc.set_imported_brep_data(body, cube(10.0), Vec::new());
    // Upright through the box's middle: x across, z up.
    let plane = wb_sketch::sketch::SketchPlane::from_frame(
        [0.0, 5.0, 0.0],
        [0.0, -1.0, 0.0],
        [1.0, 0.0, 0.0],
    );
    let sketch = doc.add_in(body, "s", SketchFeature::new(Sketch::new("s"), plane));
    let mut wb = wb_sketch::SketchWorkbench::default();
    let args: CommandArgs = serde_json::from_value(serde_json::json!({
        "sketch": sketch.0.to_string(),
        "faces": [{"body": body.0.to_string(), "point": [0.0, 5.0, 5.0], "normal": [-1.0, 0.0, 0.0]}],
    }))
    .unwrap();
    let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
    ctx.kernel = Some(&kernel_ogeom::QUERIES);
    let made = wb
        .run_command("sketch.intersection", &args, &mut ctx)
        .expect("the side crosses the plane");
    assert_eq!(made["elements"].as_array().unwrap().len(), 3, "{made}");
    let stored = |doc: &Document| {
        SketchFeature::from_json(doc.get_feature_data(sketch).unwrap())
            .unwrap()
            .sketch
    };
    let line_ends = |sketch: &Sketch| {
        let (_, source) = sketch.external.iter().next().unwrap();
        assert!(source.section);
        let line = sketch
            .geometry
            .iter()
            .find_map(|g| match g {
                GeometryElement::Line(l) => Some(l.clone()),
                _ => None,
            })
            .unwrap();
        let mut ends = [
            sketch.point_position(line.start).unwrap(),
            sketch.point_position(line.end).unwrap(),
        ];
        ends.sort_by(|a, b| a.y.total_cmp(&b.y));
        ends
    };
    let sketch_now = stored(&doc);
    let [low, high] = line_ends(&sketch_now);
    assert!(low.x.abs() < 1e-4 && low.y.abs() < 1e-4, "{low:?}");
    assert!(
        high.x.abs() < 1e-4 && (high.y - 10.0).abs() < 1e-4,
        "{high:?}"
    );
    assert!(wb_sketch::profile::extract_wires(&sketch_now).is_err());

    // The box grows: opening the sketch again brings the line up to it.
    doc.set_imported_brep_data(body, cube(16.0), Vec::new());
    let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
    ctx.kernel = Some(&kernel_ogeom::QUERIES);
    ctx.active_document_object = Some(sketch);
    wb.on_frame(0.016, &mut ctx);
    let [_, high] = line_ends(&stored(&doc));
    assert!((high.y - 16.0).abs() < 1e-4, "{high:?}");
}

/// A pocket made with the tip back on the pad goes in before the boss
/// that came after the pad: while the tip stands there the body is the pad
/// alone less the pocket, and with the tip back at the end the boss builds
/// on the pocketed pad.
#[test]
fn a_feature_made_at_an_earlier_point_builds_there() {
    let (mut doc, body, rect_id) = setup(20.0, 20.0);
    let pad_id = doc.add_in(body, "Pad", pad(rect_id, 6.0));
    let top = wb_sketch::sketch::SketchPlane::from_face([10.0, 10.0, 6.0], [0.0, 0.0, 1.0]);
    let boss_sketch = doc.add_in(body, "boss", circle_sketch_on(top, 10.0, 10.0, 3.0));
    let boss = doc.add_in(body, "Boss", pad(boss_sketch, 4.0));

    // Back at the pad, a pocket goes in: after the pad, before the boss.
    doc.set_body_tip(body, Some(pad_id));
    let hole = doc.add_in(body, "hole", circle_sketch_on(top, 10.0, 10.0, 5.0));
    let pocket = doc.add_in(body, "Pocket", pocket(hole, 6.0));
    let pi = std::f64::consts::PI;
    let plan = wb_design::body_build_ops(&doc, body).unwrap();
    assert_eq!(
        plan.op_features,
        [pad_id, pocket],
        "the boss waits past the tip"
    );
    let (volume, ..) = built_body(&doc, body).unwrap();
    assert_near(volume, 2400.0 - 150.0 * pi, 5e-3, "pad less pocket");

    // The tip back at the end: the boss builds on the pocketed pad.
    doc.set_body_tip(body, None);
    let plan = wb_design::body_build_ops(&doc, body).unwrap();
    assert_eq!(plan.op_features, [pad_id, pocket, boss]);
    let (volume, ..) = built_body(&doc, body).unwrap();
    assert_near(
        volume,
        2400.0 - 150.0 * pi + 36.0 * pi,
        5e-3,
        "with the boss",
    );
}

/// Builds `body` naming its faces after its features, as the app does.
fn built_named(doc: &Document, body: BodyId) -> Result<kernel_api::SolidBuildResult, String> {
    let plan = wb_design::body_build_ops(doc, body).map_err(|e| e.message)?;
    let tags: Vec<kernel_api::TopoName> = plan
        .op_features
        .iter()
        .map(|f| kernel_api::naming::name_of_id(f.0.as_bytes()))
        .collect();
    let mut kernel = OgeomKernel::new();
    kernel
        .execute_solid_chain_named(
            &plan.ops,
            &tags,
            &TessellationSettings::default(),
            None,
            &[],
        )
        .map_err(|e| e.to_string())
}

/// The outline edge of `mesh` whose every point satisfies `on`, picked as
/// a click picks it: its middle, its direction and its faces' names.
fn pick_edge(mesh: &kernel_api::TriMesh, on: impl Fn([f32; 3]) -> bool) -> wb_design::EdgePick {
    let edges = mesh.edge_ids.iter().copied().max().unwrap_or(0) + 1;
    for edge in 0..edges {
        let points: Vec<[f32; 3]> = mesh
            .edges
            .chunks(2)
            .zip(&mesh.edge_ids)
            .filter(|(_, id)| **id == edge)
            .flat_map(|(pair, _)| pair.iter().map(|i| mesh.positions[*i as usize]))
            .collect();
        if points.is_empty() || !points.iter().all(|p| on(*p)) {
            continue;
        }
        let (a, b) = (points[0], points[points.len() - 1]);
        let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        return wb_design::EdgePick {
            point: [
                (a[0] + b[0]) / 2.0,
                (a[1] + b[1]) / 2.0,
                (a[2] + b[2]) / 2.0,
            ],
            direction: d.map(|c| c / len),
            faces: mesh.edge_faces[edge as usize],
        };
    }
    panic!("no such edge")
}

/// A fillet on the pad's top edge follows the edge when the pad grows:
/// the edge is found by the faces it runs between, where the point it was
/// picked at is now 10 mm off it.
#[test]
fn a_fillet_follows_its_edge_by_name_when_the_pad_grows() {
    let (mut doc, body, sketch_id) = setup(20.0, 20.0);
    let pad = doc.add_in(body, "Pad", pad(sketch_id, 10.0));
    let first = built_named(&doc, body).unwrap();
    assert!(
        first.mesh.face_names.iter().all(|n| *n != 0),
        "every face is named"
    );
    // The top edge along X at y = 0.
    let pick = pick_edge(&first.mesh, |p| {
        (p[2] - 10.0).abs() < 1e-3 && p[1].abs() < 1e-3
    });
    assert!(pick.faces.iter().all(|n| *n != 0), "the pick keeps names");
    doc.add_in(
        body,
        "Fillet",
        DesignFeature::Fillet {
            radius: 2.0,
            edges: wb_design::EdgeSel::Edges(vec![pick]),
            follow_tangent: false,
        },
    );
    let round = 4.0 * (1.0 - std::f64::consts::FRAC_PI_4);
    let filleted = built_named(&doc, body).unwrap();
    assert_near(
        volume(&filleted.brep_blob),
        4000.0 - round * 20.0,
        5e-3,
        "at 10 mm",
    );

    // The pad grows to 20 mm: the named edge is at z = 20 now.
    let mut data = doc.get_feature_data(pad).unwrap().clone();
    data["Pad"]["length"] = serde_json::json!(20.0);
    doc.update_feature_data(pad, data).unwrap();
    let grown = built_named(&doc, body).unwrap();
    assert_near(
        volume(&grown.brep_blob),
        8000.0 - round * 20.0,
        5e-3,
        "at 20 mm",
    );

    // Without the names the point alone is too far from the edge.
    let mut unnamed = doc.clone();
    let fillet = unnamed
        .feature_tree()
        .all_nodes()
        .find(|(_, n)| n.name == "Fillet")
        .map(|(id, _)| *id)
        .unwrap();
    let mut data = unnamed.get_feature_data(fillet).unwrap().clone();
    data["Fillet"]["edges"]["Edges"][0]["faces"] = serde_json::json!([0, 0]);
    unnamed.update_feature_data(fillet, data).unwrap();
    assert!(
        built_named(&unnamed, body).is_err(),
        "the point alone misses it"
    );
}

/// Build `features` on a 10 × 20 rectangle at the XY plane in a new body,
/// after `datums`, and answer the solid's bounds and volume.
fn extrude_with(
    datums: &[core_document::DatumFeature],
    feature: impl Fn(FeatureId, &[FeatureId]) -> DesignFeature,
) -> (([f32; 3], [f32; 3]), f64) {
    let (mut doc, body, sketch) = setup(10.0, 20.0);
    let ids: Vec<FeatureId> = datums
        .iter()
        .map(|d| doc.add_in(body, "Datum", *d))
        .collect();
    doc.add_in(body, "Pad", feature(sketch, &ids));
    let solid = built(&doc, body);
    (solid.bounds(), solid.volume())
}

fn edited(mut pad: DesignFeature, set: impl FnOnce(&mut DesignFeature)) -> DesignFeature {
    set(&mut pad);
    pad
}

#[test]
fn a_pad_stops_on_a_datum_plane_or_a_base_plane() {
    use core_document::{AttachmentOffset, BasePlane, DatumAttachment, DatumFeature, DatumShape};
    let lid = DatumFeature {
        shape: DatumShape::Plane { size: 30.0 },
        attachment: DatumAttachment::BasePlane(BasePlane::XY),
        offset: AttachmentOffset {
            translation: [0.0, 0.0, 7.0],
            ..Default::default()
        },
    };
    let ((lo, hi), _) = extrude_with(&[lid], |sketch, datums| {
        edited(pad(sketch, 1.0), |f| {
            if let DesignFeature::Pad { mode, .. } = f {
                *mode = wb_design::ExtrudeMode::UpToPlane(wb_design::PlaneTarget::Datum {
                    datum: datums[0],
                    plane: None,
                });
            }
        })
    });
    assert!((hi[2] - lo[2] - 7.0).abs() < 1e-3, "{lo:?}..{hi:?}");
    // A sketch 10 above the XY plane, padded down to it.
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let raised = wb_sketch::sketch::SketchPlane::from_frame(
        [0.0, 0.0, 10.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
    );
    let sketch = doc.add_in(body, "s", rect_at(raised, 0.0, 0.0, 10.0, 20.0));
    let pad = edited(pad_feature(sketch, 1.0, true, false), |f| {
        if let DesignFeature::Pad { mode, .. } = f {
            *mode = wb_design::ExtrudeMode::UpToPlane(wb_design::PlaneTarget::Base(BasePlane::XY));
        }
    });
    doc.add_in(body, "Pad", pad);
    let (lo, hi) = built(&doc, body).bounds();
    assert!(
        lo[2].abs() < 1e-3 && (hi[2] - 10.0).abs() < 1e-3,
        "{lo:?}..{hi:?}"
    );
}

#[test]
fn a_pad_starts_away_from_its_profile_and_measures_a_slant_along_the_normal() {
    let ((lo, hi), _) = extrude_with(&[], |sketch, _| {
        edited(pad(sketch, 5.0), |f| {
            if let DesignFeature::Pad { extras, .. } = f {
                extras.start_offset = 3.0;
            }
        })
    });
    assert!(
        (lo[2] - 3.0).abs() < 1e-3 && (hi[2] - 8.0).abs() < 1e-3,
        "{lo:?}..{hi:?}"
    );
    let ((lo, hi), volume) = extrude_with(&[], |sketch, _| {
        edited(pad(sketch, 5.0), |f| {
            if let DesignFeature::Pad {
                extras, direction, ..
            } = f
            {
                *direction = wb_design::ExtrudeDirection::Custom([1.0, 0.0, 1.0]);
                extras.along_normal = true;
            }
        })
    });
    assert!((hi[2] - lo[2] - 5.0).abs() < 1e-3, "{lo:?}..{hi:?}");
    assert!(
        (volume - 1000.0).abs() < 1e-2,
        "a slanted prism as tall: {volume}"
    );
    let ((lo, hi), _) = extrude_with(&[], |sketch, _| {
        edited(pad(sketch, 4.0), |f| {
            if let DesignFeature::Pad { direction, .. } = f {
                *direction = wb_design::ExtrudeDirection::Axis(wb_design::BaseAxis::Z);
            }
        })
    });
    assert!((hi[2] - lo[2] - 4.0).abs() < 1e-3, "{lo:?}..{hi:?}");
}

#[test]
fn each_side_of_a_pad_takes_its_own_taper() {
    let two_sided = |taper2: Option<f32>| {
        extrude_with(&[], |sketch, _| {
            edited(pad(sketch, 5.0), |f| {
                if let DesignFeature::Pad {
                    mode,
                    length2,
                    taper_deg,
                    extras,
                    ..
                } = f
                {
                    *mode = wb_design::ExtrudeMode::TwoLengths;
                    *length2 = 5.0;
                    *taper_deg = -10.0;
                    extras.taper2_deg = taper2;
                }
            })
        })
        .1
    };
    let both = two_sided(None);
    let one = two_sided(Some(0.0));
    // The side with no taper is the plain prism, 200 × 5.
    assert!(
        (one - (both / 2.0 + 1000.0)).abs() < 1e-2,
        "{one} vs {both}"
    );
}

#[test]
fn a_pad_runs_along_a_datum_or_a_sketch_line() {
    use core_document::{AttachmentOffset, BasePlane, DatumAttachment, DatumFeature, DatumShape};
    // Square to a datum plane tilted 45° about x: a prism leaning in y.
    let leaning = DatumFeature {
        shape: DatumShape::Plane { size: 30.0 },
        attachment: DatumAttachment::BasePlane(BasePlane::XY),
        offset: AttachmentOffset {
            tilt: [45.0, 0.0],
            ..Default::default()
        },
    };
    let ((lo, hi), volume) = extrude_with(&[leaning], |sketch, datums| {
        edited(pad(sketch, 5.0), |f| {
            if let DesignFeature::Pad { direction, .. } = f {
                *direction = wb_design::ExtrudeDirection::Datum(datums[0]);
            }
        })
    });
    let rise = 5.0 * std::f32::consts::FRAC_1_SQRT_2;
    assert!((hi[2] - lo[2] - rise).abs() < 1e-3, "{lo:?}..{hi:?}");
    assert!((volume - 200.0 * f64::from(rise)).abs() < 1e-2, "{volume}");
    // Along the profile's own edge the direction lies in its plane.
    let (mut doc, body, sketch) = setup(10.0, 20.0);
    let edge = {
        let data = doc.feature_values(sketch).unwrap();
        let feature = <SketchFeature as core_document::WorkbenchFeature>::from_json(data).unwrap();
        feature
            .sketch
            .geometry
            .iter()
            .find_map(|g| match g {
                GeometryElement::Line(l) => Some(l.id),
                _ => None,
            })
            .unwrap()
    };
    let pad = edited(pad(sketch, 5.0), |f| {
        if let DesignFeature::Pad { direction, .. } = f {
            *direction = wb_design::ExtrudeDirection::SketchLine {
                sketch,
                element: edge,
            };
        }
    });
    doc.add_in(body, "Pad", pad);
    let error = wb_design::body_build_ops(&doc, body)
        .err()
        .map(|e| e.message);
    assert_eq!(
        error.as_deref(),
        Some("the extrusion direction lies in the profile's plane")
    );
}

fn build_one(
    sketch: SketchFeature,
    feature: impl Fn(FeatureId) -> serde_json::Value,
) -> (([f32; 3], [f32; 3]), f64) {
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let id = doc.add_in(body, "s", sketch);
    let made = <DesignFeature as core_document::WorkbenchFeature>::from_json(&feature(id)).unwrap();
    doc.add_in(body, "F", made);
    let solid = built(&doc, body);
    (solid.bounds(), solid.volume())
}

#[test]
fn a_revolution_turns_about_the_bodys_axis() {
    let (_, volume) = build_one(rect_at(SketchPlane::xz(), 5.0, 0.0, 7.0, 4.0), |sketch| {
        serde_json::json!({"Revolution": {
            "sketch": sketch.0.to_string(),
            "angle_deg": 360.0,
            "axis": {"Base": "Z"},
        }})
    });
    let ring = std::f64::consts::PI * (49.0 - 25.0) * 4.0;
    assert!((volume - ring).abs() < 1e-3 * ring, "{volume} vs {ring}");
}

#[test]
fn a_helix_climbs_about_its_sketchs_normal_with_the_profile_level() {
    let disc = circle_sketch_on(wb_sketch::sketch::SketchPlane::xy(), 10.0, 0.0, 1.0);
    let ((lo, hi), volume) = build_one(disc, |sketch| {
        serde_json::json!({"Helix": {
            "sketch": sketch.0.to_string(),
            "axis": "SketchNormal",
            "mode": "PitchHeight",
            "pitch": 10.0,
            "height": 10.0,
            "turns": 1.0,
            "left_handed": false,
            "cone_angle_deg": 0.0,
            "reversed": false,
            "subtractive": false,
        }})
    });
    assert!(
        lo[2].abs() < 1e-3 && (hi[2] - 10.0).abs() < 1e-2,
        "{lo:?}..{hi:?}"
    );
    // Every level cut is the disc: its area times the rise.
    let expected = std::f64::consts::PI * 10.0;
    assert!(
        (volume - expected).abs() < 1e-2 * expected,
        "{volume} vs {expected}"
    );
}

/// A fixed orientation carries the section along the path without turning
/// it: the square, upright in the XZ plane, stays so round the quarter arc,
/// sweeping its 2 mm width across the path's 20 mm run in y, 2 mm tall.
#[test]
fn a_fixed_pipe_carries_its_section_without_turning_it() {
    use wb_design::{PipeCorner, PipeOrientation};
    let (doc, body) = pipe_body(square_section(), quarter_arc_sketch(), &[], |p, s, _| {
        pipe_of(
            p,
            s,
            PipeOrientation::Fixed,
            PipeCorner::Transformed,
            Vec::new(),
        )
    });
    let (volume, ..) = built_body(&doc, body).unwrap();
    assert_near(volume, 2.0 * 2.0 * 20.0, 5e-3, "fixed");
}

fn loft_of(sections: Vec<wb_design::LoftSection>) -> DesignFeature {
    DesignFeature::Loft {
        keep_inside: false,
        sections,
        ruled: false,
        closed: false,
        subtractive: false,
        refine: false,
        boolean_tolerance: None,
    }
}

fn raised(z: f64) -> wb_sketch::sketch::SketchPlane {
    wb_sketch::sketch::SketchPlane::from_frame([0.0, 0.0, z], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0])
}

/// A rectangle lofted to a datum point is a pyramid; so is one lofted from
/// a sketch holding a single point.
#[test]
fn a_loft_closes_to_a_point() {
    use core_document::{AttachmentOffset, BasePlane, DatumAttachment, DatumFeature, DatumShape};
    use wb_design::LoftSection;
    let pyramid = 200.0 * 10.0 / 3.0;
    let apex = DatumFeature {
        shape: DatumShape::Point,
        attachment: DatumAttachment::BasePlane(BasePlane::XY),
        offset: AttachmentOffset {
            translation: [5.0, 10.0, 10.0],
            ..Default::default()
        },
    };
    let (_, volume) = extrude_with(&[apex], |sketch, datums| {
        loft_of(vec![
            LoftSection::Feature(sketch),
            LoftSection::Feature(datums[0]),
        ])
    });
    assert!((volume - pyramid).abs() < 1e-3 * pyramid, "{volume}");

    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let mut dot = Sketch::new("apex");
    dot.plane = raised(-10.0);
    dot.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, 10.0))));
    let plane = dot.plane;
    let apex = doc.add_in(body, "apex", SketchFeature::new(dot, plane));
    let base = doc.add_in(
        body,
        "base",
        rect_at(SketchPlane::xy(), 0.0, 0.0, 10.0, 20.0),
    );
    doc.add_in(
        body,
        "Loft",
        loft_of(vec![LoftSection::Feature(apex), LoftSection::Feature(base)]),
    );
    let (volume, min, _) = built_body(&doc, body).unwrap();
    assert!((volume - pyramid).abs() < 1e-3 * pyramid, "{volume}");
    assert!((min[2] + 10.0).abs() < 1e-3);
}

/// A loft from a flat face of the solid: the top of a block lofted up to a
/// rectangle above it, the same size, stands a prism on the block.
#[test]
fn a_loft_starts_from_a_face_of_the_solid() {
    use wb_design::LoftSection;
    let (mut doc, body, sketch) = setup(10.0, 20.0);
    doc.add_in(body, "Pad", pad(sketch, 5.0));
    let top = doc.add_in(body, "top", rect_at(raised(15.0), 0.0, 0.0, 10.0, 20.0));
    let face = wb_design::FacePick {
        point: [5.0, 10.0, 5.0],
        normal: [0.0, 0.0, 1.0],
        name: 0,
    };
    doc.add_in(
        body,
        "Loft",
        loft_of(vec![LoftSection::Face(face), LoftSection::Feature(top)]),
    );
    let (volume, _, max) = built_body(&doc, body).unwrap();
    assert!((volume - 3000.0).abs() < 1.0, "{volume}");
    assert!((max[2] - 15.0).abs() < 1e-3);
}

/// Through a middle section to a point: circles narrowing to an apex on
/// their axis.
#[test]
fn a_loft_through_sections_closes_to_a_point() {
    use wb_design::LoftSection;
    let circles = |r: f64, z: f64| circle_sketch_on(raised(z), 0.0, 0.0, r);
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let a = doc.add_in(body, "a", circles(5.0, 0.0));
    let b = doc.add_in(body, "b", circles(3.0, 5.0));
    let mut dot = Sketch::new("apex");
    dot.plane = raised(10.0);
    dot.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
    let plane = dot.plane;
    let c = doc.add_in(body, "apex", SketchFeature::new(dot, plane));
    doc.add_in(
        body,
        "Loft",
        loft_of(vec![
            LoftSection::Feature(a),
            LoftSection::Feature(b),
            LoftSection::Feature(c),
        ]),
    );
    let (_, _, max) = built_body(&doc, body).unwrap();
    assert!((max[2] - 10.0).abs() < 1e-3);
}

/// A tube swept along the block's front bottom edge: three quarters of it
/// stand outside the block.
#[test]
fn a_pipe_runs_along_an_edge_of_the_solid() {
    let (mut doc, body, sketch) = setup(10.0, 20.0);
    doc.add_in(body, "Pad", pad(sketch, 5.0));
    let ring = doc.add_in(
        body,
        "ring",
        circle_sketch_on(wb_sketch::sketch::SketchPlane::yz(), 0.0, 0.0, 1.0),
    );
    let spare = doc.add_in(
        body,
        "spare",
        rect_at(SketchPlane::xy(), 0.0, 0.0, 1.0, 1.0),
    );
    let pipe = edited(
        pipe_of(
            ring,
            spare,
            wb_design::PipeOrientation::Standard,
            wb_design::PipeCorner::Transformed,
            Vec::new(),
        ),
        |f| {
            if let DesignFeature::Pipe { path_edges, .. } = f {
                *path_edges = vec![wb_design::EdgePick {
                    faces: [0, 0],
                    point: [5.0, 0.0, 0.0],
                    direction: [1.0, 0.0, 0.0],
                }];
            }
        },
    );
    doc.add_in(body, "Pipe", pipe);
    let (volume, ..) = built_body(&doc, body).unwrap();
    let expected = 1000.0 + 0.75 * std::f64::consts::PI * 10.0;
    assert!(
        (volume - expected).abs() < 1e-2 * expected,
        "{volume} vs {expected}"
    );
}

/// The block's top face swept straight up 10 is a prism on it.
#[test]
fn a_pipe_sweeps_a_face_of_the_solid() {
    let (mut doc, body, sketch) = setup(10.0, 20.0);
    doc.add_in(body, "Pad", pad(sketch, 5.0));
    let rise = doc.add_in(
        body,
        "rise",
        polyline_sketch(
            wb_sketch::sketch::SketchPlane::from_frame(
                [0.0, 10.0, 0.0],
                [0.0, -1.0, 0.0],
                [1.0, 0.0, 0.0],
            ),
            &[[5.0, 5.0], [5.0, 15.0]],
        ),
    );
    let pipe = edited(
        pipe_of(
            rise,
            rise,
            wb_design::PipeOrientation::Standard,
            wb_design::PipeCorner::Transformed,
            Vec::new(),
        ),
        |f| {
            if let DesignFeature::Pipe { profile_face, .. } = f {
                *profile_face = Some(wb_design::FacePick {
                    point: [5.0, 10.0, 5.0],
                    normal: [0.0, 0.0, 1.0],
                    name: 0,
                });
            }
        },
    );
    doc.add_in(body, "Pipe", pipe);
    let (volume, _, max) = built_body(&doc, body).unwrap();
    assert!((volume - 3000.0).abs() < 1.0, "{volume}");
    assert!((max[2] - 15.0).abs() < 1e-3, "{max:?}");
}

/// A square piped straight up to a datum point above its centre is the
/// pyramid over it.
#[test]
fn a_pipe_closes_to_a_point() {
    use core_document::{AttachmentOffset, BasePlane, DatumAttachment, DatumFeature, DatumShape};
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let square = doc.add_in(
        body,
        "square",
        rect_at(wb_sketch::sketch::SketchPlane::xy(), 0.0, 0.0, 2.0, 2.0),
    );
    let path = doc.add_in(
        body,
        "path",
        polyline_sketch(
            wb_sketch::sketch::SketchPlane::xz(),
            &[[1.0, 0.0], [1.0, 10.0]],
        ),
    );
    let apex = doc.add_in(
        body,
        "apex",
        DatumFeature {
            shape: DatumShape::Point,
            attachment: DatumAttachment::BasePlane(BasePlane::XY),
            offset: AttachmentOffset {
                // At the path's end, where the pipe closes.
                translation: [1.0, 0.0, 10.0],
                ..Default::default()
            },
        },
    );
    doc.add_in(
        body,
        "Pipe",
        pipe_of(
            square,
            path,
            wb_design::PipeOrientation::Standard,
            wb_design::PipeCorner::Transformed,
            vec![apex],
        ),
    );
    let (volume, ..) = built_body(&doc, body).unwrap();
    let pyramid = 4.0 * 10.0 / 3.0;
    assert!((volume - pyramid).abs() < 1e-3 * pyramid, "{volume}");
}

/// A 10 × 20 × 5 block opened at its top with 1 mm walls on both sides of
/// its faces: the inward walls and the outward ones together.
#[test]
fn a_thickness_on_both_sides_stands_either_side_of_the_faces() {
    let (mut doc, body, sketch) = setup(10.0, 20.0);
    doc.add_in(body, "Pad", pad(sketch, 5.0));
    doc.add_in(
        body,
        "Shell",
        DesignFeature::Thickness {
            value: 1.0,
            faces: vec![wb_design::FacePick {
                point: [5.0, 10.0, 5.0],
                normal: [0.0, 0.0, 1.0],
                name: 0,
            }],
            inward: true,
            join: kernel_api::ThicknessJoin::Intersection,
            both_sides: true,
        },
    );
    let (volume, min, max) = built_body(&doc, body).unwrap();
    let inner = 1000.0 - 8.0 * 18.0 * 4.0;
    let outer = 12.0 * 22.0 * 6.0 - 1000.0;
    assert!((volume - (inner + outer)).abs() < 0.5, "{volume}");
    assert!(
        (min[2] + 1.0).abs() < 1e-3 && (max[2] - 5.0).abs() < 1e-3,
        "{min:?}..{max:?}"
    );
}

/// One boolean cutting two tool bodies out of a block.
#[test]
fn a_boolean_takes_several_tool_bodies() {
    let mut doc = Document::new("t");
    let target = doc.create_body(Some("Target".into()));
    let block = doc.add_in(
        target,
        "block",
        rect_at(SketchPlane::xy(), 0.0, 0.0, 10.0, 20.0),
    );
    doc.add_in(target, "Pad", pad(block, 5.0));
    let tool = |doc: &mut Document, name: &str, x: f64| {
        let body = doc.create_body(Some(name.into()));
        let sketch = doc.add_in(
            body,
            "tool",
            rect_at(
                wb_sketch::sketch::SketchPlane::from_frame(
                    [x, 0.0, 0.0],
                    [0.0, 0.0, 1.0],
                    [1.0, 0.0, 0.0],
                ),
                0.0,
                0.0,
                2.0,
                2.0,
            ),
        );
        doc.add_in(body, "Pad", pad(sketch, 5.0));
        body
    };
    let a = tool(&mut doc, "A", 1.0);
    let b = tool(&mut doc, "B", 6.0);
    doc.add_in(
        target,
        "Cut",
        DesignFeature::BodyBoolean {
            tool_body: a,
            kind: kernel_api::BoolKind::Cut,
            more_tools: vec![b],
            refine: false,
            boolean_tolerance: None,
        },
    );
    // The tools build first; then the target cuts them both.
    wb_design::mark_all_design_features_dirty(&mut doc);
    settle(&mut doc);
    let volume = body_volume(&doc, target);
    assert!((volume - (1000.0 - 2.0 * 20.0)).abs() < 1e-3, "{volume}");
}

/// Two circles of radius 10 with centres 10 apart, padded 5 taking the
/// regions `pick` chooses of the three they close: the solid's volume and
/// bounds.
fn padded_circle_regions(
    pick: impl Fn(&[wb_sketch::regions::Region]) -> Vec<String>,
) -> (f64, ([f32; 3], [f32; 3])) {
    let mut sketch = Sketch::new("venn");
    for x in [0.0, 10.0] {
        let center = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 0.0))));
        sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 10.0)));
    }
    let regions = wb_sketch::profile::regions(&sketch).expect("regions");
    assert_eq!(regions.len(), 3, "a lens and two crescents");
    let chosen = pick(&regions);
    let plane = sketch.plane;
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let sketch_id = doc.add_in(body, "sketch", SketchFeature::new(sketch, plane));
    let mut pad = pad(sketch_id, 5.0);
    if let DesignFeature::Pad { regions, .. } = &mut pad {
        *regions = chosen;
    }
    doc.add_in(body, "Pad", pad);
    let solid = built(&doc, body);
    (solid.volume(), mesh_bounds(&solid.0.mesh))
}

/// The area two circles of radius 10 with centres 10 apart share.
fn venn_lens_area() -> f64 {
    200.0 * (0.5f64).acos() - 5.0 * 300f64.sqrt()
}

/// The lens alone, the union of the circles when no region is chosen, and
/// the two crescents, which meet only where the circles cross.
#[test]
fn crossing_circles_pad_the_regions_chosen() {
    let (volume, (lo, hi)) = padded_circle_regions(|regions| {
        let lens = regions
            .iter()
            .find(|r| (r.inside[0] - 5.0).abs() < 1.0)
            .expect("the lens between the centres");
        vec![lens.name.clone()]
    });
    assert!(
        (volume - 5.0 * venn_lens_area()).abs() < 1e-3,
        "{volume} vs {}",
        5.0 * venn_lens_area()
    );
    // From one centre to the other, as high as the circles cross.
    assert!(
        lo[0].abs() < 1e-3 && (hi[0] - 10.0).abs() < 1e-3,
        "{lo:?}..{hi:?}"
    );
    let half = 75f32.sqrt();
    assert!((hi[1] - half).abs() < 1e-3 && (lo[1] + half).abs() < 1e-3);
    assert!((hi[2] - lo[2] - 5.0).abs() < 1e-3);

    let disc = std::f64::consts::PI * 100.0;
    let (union, _) = padded_circle_regions(|_| Vec::new());
    let expected = 5.0 * (2.0 * disc - venn_lens_area());
    assert!((union - expected).abs() < 1e-3, "{union} vs {expected}");
    // The two crescents, which meet only where the circles cross.
    let (crescents, (lo, hi)) = padded_circle_regions(|regions| {
        regions
            .iter()
            .filter(|r| (r.inside[0] - 5.0).abs() >= 1.0)
            .map(|r| r.name.clone())
            .collect()
    });
    let expected = 5.0 * 2.0 * (disc - venn_lens_area());
    assert!(
        (crescents - expected).abs() < 1e-3,
        "{crescents} vs {expected}"
    );
    assert!((lo[0] + 10.0).abs() < 1e-3 && (hi[0] - 20.0).abs() < 1e-3);
}

/// A pocket kept inside, sketched on the block's top, keeps the stretch of
/// the block its tool digs through: a disc as deep as the pocket.
#[test]
fn a_pocket_kept_inside_keeps_the_disc_it_digs_through() {
    let (mut doc, body, rect_id) = setup(40.0, 30.0);
    doc.add_in(body, "Pad", pad(rect_id, 12.0));
    let top = wb_sketch::sketch::SketchPlane {
        origin: [0.0, 0.0, 12.0],
        ..Default::default()
    };
    let disc = doc.add_in(body, "disc", circle_sketch_on(top, 20.0, 15.0, 6.0));
    let mut kept = pocket(disc, 4.0);
    *kept.keep_inside_mut().expect("a pocket offers it") = true;
    doc.add_in(body, "Pocket", kept);

    let kept = built(&doc, body);
    let (lo, hi) = kept.bounds();
    assert!(
        (lo[2] - 8.0).abs() < 1e-3 && (hi[2] - 12.0).abs() < 1e-3,
        "{lo:?}..{hi:?}"
    );
    let volume = kept.volume();
    let expected = std::f64::consts::PI * 36.0 * 4.0;
    assert!(
        (volume - expected).abs() < 1e-2 * expected,
        "volume {volume} vs {expected}"
    );
}

/// A Ø20 × 20 cylinder with `label` projected onto its side, or wrapped
/// round it, picked where it faces -Y, as `mode` says; the plain
/// cylinder's volume and the built result.
fn sketch_projected_onto_a_cylinder(
    label: SketchFeature,
    mode: kernel_api::ProjectMode,
    height: f32,
    wrap: bool,
) -> (f64, kernel_api::SolidBuildResult) {
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    doc.add_in(
        body,
        "Cylinder",
        DesignFeature::Primitive {
            boolean_tolerance: None,
            keep_inside: false,
            attached: None,
            kind: kernel_api::PrimitiveKind::Cylinder {
                radius: 10.0,
                height: 20.0,
                angle_deg: 360.0,
            },
            placement: kernel_api::Placement::default(),
            subtractive: false,
            refine: false,
        },
    );
    let label = doc.add_in(body, "label", label);
    doc.add_in(
        body,
        "Project",
        DesignFeature::Project {
            sketch: label,
            faces: vec![wb_design::FacePick {
                point: [0.0, -10.0, 10.0],
                normal: [0.0, -1.0, 0.0],
                name: 0,
            }],
            mode,
            height,
            wrap,
        },
    );
    let ops = wb_design::body_build_ops(&doc, body).unwrap().ops;
    let result = OgeomKernel::new()
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .unwrap();
    (std::f64::consts::PI * 100.0 * 20.0, result)
}

fn volume_and_soundness(result: &kernel_api::SolidBuildResult) -> (f64, bool) {
    (volume(&result.brep_blob), is_sound(&result.brep_blob))
}

/// "PC", 8 mm tall, sketched in front of the Ø20 cylinder's side: letters
/// of curves, with a counter and a hollow.
fn text_label() -> SketchFeature {
    let plane = wb_sketch::sketch::SketchPlane {
        origin: [0.0, -15.0, 10.0],
        normal: [0.0, -1.0, 0.0],
        x_axis: [1.0, 0.0, 0.0],
        y_axis: [0.0, 0.0, 1.0],
    };
    let mut sketch = Sketch::new("s");
    sketch.plane = plane;
    let spec = wb_sketch::text::TextSpec {
        text: "PC".into(),
        size: 8.0,
        ..Default::default()
    };
    wb_sketch::text::add(&mut sketch, Vec2D::new(-4.0, -3.0), &spec).unwrap();
    SketchFeature::new(sketch, plane)
}

/// The result's volume summed over a mesh of every face, and whether the
/// kernel's checker finds it sound. Two results whose sides are cut alike
/// carry alike meshing errors, which their difference cancels: at this
/// chord the raised-to-sunk ratios below sit within 1e-3 of what a chord
/// ten times finer gives.
fn meshed_volume(result: &kernel_api::SolidBuildResult) -> (f64, bool) {
    let (model, shape) = read_shape(&result.brep_blob);
    let tol = ogeom::core::Tolerances::millimetres();
    let fine = ogeom::mesh::Deflection {
        chord: 1e-2,
        angular: 0.05,
        ..ogeom::mesh::Deflection::default()
    };
    let mut volume = 0.0;
    for face in ogeom::topo::explore_unique(&model, &shape, ogeom::topo::ShapeType::Face).unwrap() {
        let mesh = ogeom::mesh::triangulate_face(&model, &face, fine, tol).unwrap();
        for t in &mesh.triangles {
            let [a, b, c] = t.map(|i| mesh.positions[i as usize] - ogeom::math::Point::ORIGIN);
            volume += a.dot(b.cross(c)) / 6.0;
        }
    }
    (volume, is_sound(&result.brep_blob))
}

/// The text label split onto, embossed on and debossed into the cylinder's
/// side, wrapped round it or carried straight: the three meshed volumes,
/// each built on a thread of its own, each result checked sound.
fn text_on_a_cylinder(wrap: bool) -> [f64; 3] {
    use kernel_api::ProjectMode::{Deboss, Emboss, Split};
    std::thread::scope(|scope| {
        [Split, Emboss, Deboss]
            .map(|mode| {
                scope.spawn(move || {
                    let built = sketch_projected_onto_a_cylinder(text_label(), mode, 1.0, wrap).1;
                    let (volume, sound) = meshed_volume(&built);
                    assert!(sound, "{mode:?} (wrapped {wrap}) checks sound");
                    volume
                })
            })
            .map(|build| build.join().expect("the build ran"))
    })
}

/// [`text_on_a_cylinder`] carried straight, built once for both tests that
/// read it.
fn straight_text_on_a_cylinder() -> [f64; 3] {
    static STRAIGHT: std::sync::OnceLock<[f64; 3]> = std::sync::OnceLock::new();
    *STRAIGHT.get_or_init(|| text_on_a_cylinder(false))
}

#[test]
fn projected_text_embosses_and_debosses_a_cylinder() {
    let [split, up, down] = straight_text_on_a_cylinder();
    let (gained, lost) = (up - split, split - down);
    // The letters' area on the side, raised and sunk 1 mm: the shells
    // between radii 10 and 11 and between 9 and 10 they fill are 21 to 19.
    assert!(gained > 10.0 && lost > 10.0, "{gained} and {lost}");
    assert!(
        (gained / lost - 21.0 / 19.0).abs() < 0.03,
        "{gained} raised against {lost} sunk"
    );
}

#[test]
fn wrapped_text_embosses_and_debosses_a_cylinder() {
    let [split, up, down] = text_on_a_cylinder(true);
    let (gained, lost) = (up - split, split - down);
    assert!(
        (gained / lost - 21.0 / 19.0).abs() < 0.03,
        "{gained} raised against {lost} sunk"
    );
    // Wrapped, the letters keep their widths; carried straight they
    // stretch round the side away from its middle, and raise more.
    let [plain, straight, _] = straight_text_on_a_cylinder();
    assert!(
        gained < straight - plain,
        "{gained} wrapped, {} straight",
        straight - plain
    );
}

#[test]
#[ignore = "kernel: volume_properties meshes the whole solid at the deflection given once one face has no closed form, so a Ø20 cylinder's side measures 0.6% short and embossed text reads as lost volume (ogeom-rs#209)"]
fn embossed_text_measures_what_it_adds() {
    let (plain, raised) =
        sketch_projected_onto_a_cylinder(text_label(), kernel_api::ProjectMode::Emboss, 1.0, false);
    let (measured, _) = volume_and_soundness(&raised);
    // The letters add some 17 mm³.
    assert!(measured > plain + 15.0, "{} added", measured - plain);
}

/// A sketch whose profile counts an edge projected from another body's
/// pad follows the pad when its length changes, without the sketch being
/// opened: evaluation projects the edge again, and the body padded from
/// the sketch grows with it. What the sketch stores is left as it was.
#[test]
fn a_counting_projected_edge_follows_its_pad_without_the_sketch_opened() {
    use core_document::{CommandArgs, Workbench, WorkbenchFeature, WorkbenchRuntimeContext};
    let mut registry = registry();
    registry.set_kernel(&kernel_ogeom::QUERIES);

    // A block 40 mm wide and 10 mm high, and in front of it a sketch
    // standing upright, x across and z up, in a body of its own.
    let (mut doc, block, base) = setup(40.0, 10.0);
    let pad_id = doc.add_in(block, "Pad", pad(base, 10.0));
    doc.mark_feature_stale(pad_id);
    settle_as_host(&registry, &mut doc);
    let other = doc.create_body(Some("Other".into()));
    let plane = wb_sketch::sketch::SketchPlane::from_frame(
        [0.0, -5.0, 0.0],
        [0.0, -1.0, 0.0],
        [1.0, 0.0, 0.0],
    );
    let sketch_id = doc.add_in(
        other,
        "front",
        SketchFeature::new(Sketch::new("front"), plane),
    );
    // The block's front top edge, counting, closed by three drawn lines.
    let mut sketcher = wb_sketch::SketchWorkbench::default();
    let args: CommandArgs = serde_json::from_value(serde_json::json!({
        "sketch": sketch_id.0.to_string(),
        "counts": true,
        "edges": [{"body": block.0.to_string(), "point": [20.0, 0.0, 10.0], "direction": [1.0, 0.0, 0.0]}],
    }))
    .unwrap();
    let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
    ctx.kernel = Some(&kernel_ogeom::QUERIES);
    sketcher
        .run_command("sketch.external", &args, &mut ctx)
        .expect("the edge projects");
    let mut feature = SketchFeature::from_json(doc.get_feature_data(sketch_id).unwrap()).unwrap();
    let edge = feature
        .sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Line(l) if feature.sketch.is_external(l.id) => Some(l.clone()),
            _ => None,
        })
        .expect("the projected edge");
    let x_of = |sketch: &Sketch, p| sketch.point_position(p).unwrap().x;
    let (left, right) = if x_of(&feature.sketch, edge.start) < x_of(&feature.sketch, edge.end) {
        (edge.start, edge.end)
    } else {
        (edge.end, edge.start)
    };
    let sketch = &mut feature.sketch;
    let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
    let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(40.0, 0.0))));
    for (s, e) in [(a, b), (b, right), (left, a)] {
        sketch.add_geometry(GeometryElement::Line(Line::new(s, e)));
    }
    doc.set_feature(sketch_id, feature);
    let wall = doc.add_in(other, "Wall", pad(sketch_id, 2.0));
    doc.mark_feature_stale(wall);
    settle_as_host(&registry, &mut doc);
    let height = |doc: &Document| {
        let (min, max) = mesh_bounds(&doc.imported_geometry(other).unwrap().mesh);
        max[2] - min[2]
    };
    assert!((height(&doc) - 10.0).abs() < 1e-3, "{}", height(&doc));

    // The block's pad grows to 15 mm: the edge, the sketch's profile and
    // the wall padded from it follow.
    let stored = doc.get_feature_data(sketch_id).unwrap().clone();
    doc.set_feature(pad_id, pad(base, 15.0));
    doc.mark_feature_dirty(pad_id);
    settle_as_host(&registry, &mut doc);
    let followed = SketchFeature::from_json(doc.feature_values(sketch_id).unwrap()).unwrap();
    let edge_height = followed.sketch.point_position(right).unwrap().y;
    assert!((edge_height - 15.0).abs() < 1e-6, "{edge_height}");
    assert!((height(&doc) - 15.0).abs() < 1e-3, "{}", height(&doc));
    assert_eq!(
        doc.get_feature_data(sketch_id),
        Some(&stored),
        "followed as derived values, the sketch's own data untouched"
    );
}

/// An 80 × 80 × 4 plate with a hole of radius 2 through it at (70, 40),
/// a quarter arc of radius 30 about the plate's middle starting at the
/// hole, and the circle the arc is part of.
fn plate_with_hole_and_paths() -> (Document, BodyId, FeatureId, FeatureId, FeatureId) {
    let (mut doc, body, plate) = setup(80.0, 80.0);
    doc.add_in(body, "Plate", pad(plate, 4.0));
    let at = doc.add_in(
        body,
        "hole",
        circle_sketch_on(plane_at_z(4.0), 70.0, 40.0, 2.0),
    );
    let hole = doc.add_in(body, "Hole", pocket(at, 4.0));
    let mut arc = Sketch::new("arc");
    arc.plane = plane_at_z(4.0);
    let centre = arc.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(40.0, 40.0))));
    let start = arc.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(70.0, 40.0))));
    let end = arc.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(40.0, 70.0))));
    arc.add_geometry(GeometryElement::Arc(wb_sketch::sketch::Arc::new(
        centre, start, end, 30.0,
    )));
    let arc = doc.add_in(body, "arc", SketchFeature::new(arc, plane_at_z(4.0)));
    let ring = doc.add_in(
        body,
        "ring",
        circle_sketch_on(plane_at_z(4.0), 40.0, 40.0, 30.0),
    );
    (doc, body, hole, arc, ring)
}

fn path_pattern(
    hole: FeatureId,
    path: wb_design::PatternPath,
    layout: wb_design::PathSpacing,
) -> DesignFeature {
    DesignFeature::PathPattern {
        originals: vec![hole],
        path,
        layout,
        refine: false,
        boolean_tolerance: None,
        skipped: Vec::new(),
    }
}

/// The volume of what the body builds and where its round faces of
/// radius 2 stand: one per hole.
fn holes_built(doc: &Document, body: BodyId) -> (f64, Vec<[f32; 3]>) {
    let solid = built(doc, body);
    let holes: Vec<[f32; 3]> = solid
        .0
        .mesh
        .face_surfaces
        .iter()
        .filter_map(|s| match s {
            kernel_api::FaceSurface::Cylinder { origin, radius, .. }
                if (radius - 2.0).abs() < 1e-3 =>
            {
                Some(*origin)
            }
            _ => None,
        })
        .collect();
    (solid.volume(), holes)
}

fn plate_less_holes(holes: f64) -> f64 {
    80.0 * 80.0 * 4.0 - holes * std::f64::consts::PI * 4.0 * 4.0
}

/// Holes along a quarter arc: five spread over it from the hole's place
/// to the arc's far end, each 22.5° on; spaced 15 mm along it, as many as
/// fit. Moved only, each stands on the arc.
#[test]
fn holes_patterned_along_an_arc_stand_on_it() {
    let (mut doc, body, hole, arc, _) = plate_with_hole_and_paths();
    let pattern = doc.add_in(
        body,
        "Along",
        path_pattern(
            hole,
            wb_design::PatternPath::Sketch {
                sketch: arc,
                chain: 0,
            },
            wb_design::PathSpacing {
                occurrences: 5,
                ..Default::default()
            },
        ),
    );
    let (volume, holes) = holes_built(&doc, body);
    assert!((volume - plate_less_holes(5.0)).abs() < 0.5, "{volume}");
    assert_eq!(holes.len(), 5, "{holes:?}");
    for at in &holes {
        let r = (at[0] - 40.0).hypot(at[1] - 40.0);
        assert!((r - 30.0).abs() < 1e-3, "off the arc: {at:?}");
    }
    assert!(
        holes.iter().any(|h| close(h[0], 40.0) && close(h[1], 70.0)),
        "one at the arc's far end: {holes:?}"
    );

    // 15 mm apart along an arc 47.1 mm long: four fit, a fifth is refused.
    let spaced = |occurrences| {
        path_pattern(
            hole,
            wb_design::PatternPath::Sketch {
                sketch: arc,
                chain: 0,
            },
            wb_design::PathSpacing {
                occurrences,
                spacing_mode: true,
                spacing: 15.0,
                ..Default::default()
            },
        )
    };
    doc.set_feature(pattern, spaced(4));
    let (volume, holes) = holes_built(&doc, body);
    assert!((volume - plate_less_holes(4.0)).abs() < 0.5, "{volume}");
    assert_eq!(holes.len(), 4);
    doc.set_feature(pattern, spaced(5));
    let refused = built_volume(&doc, body).unwrap_err();
    assert!(refused.contains("holds 4 occurrences"), "{refused}");
}

/// Round a closed circle six holes go evenly; turned with the path they
/// stay on it whichever way it is walked. A skipped one is left out.
#[test]
fn holes_patterned_round_a_closed_circle_go_evenly_round_it() {
    let (mut doc, body, hole, _, ring) = plate_with_hole_and_paths();
    let round = |reversed, skipped: Vec<u32>| DesignFeature::PathPattern {
        originals: vec![hole],
        path: wb_design::PatternPath::Sketch {
            sketch: ring,
            chain: 0,
        },
        layout: wb_design::PathSpacing {
            occurrences: 6,
            reversed,
            align: true,
            ..Default::default()
        },
        refine: false,
        boolean_tolerance: None,
        skipped,
    };
    let pattern = doc.add_in(body, "Round", round(false, Vec::new()));
    for (reversed, skipped, count) in [
        (false, vec![], 6u32),
        (true, vec![], 6),
        (false, vec![3], 5),
    ] {
        doc.set_feature(pattern, round(reversed, skipped));
        let (volume, holes) = holes_built(&doc, body);
        assert!(
            (volume - plate_less_holes(f64::from(count))).abs() < 0.5,
            "{reversed}: {volume}"
        );
        assert_eq!(holes.len(), count as usize, "{reversed}: {holes:?}");
        for at in &holes {
            let r = (at[0] - 40.0).hypot(at[1] - 40.0);
            assert!((r - 30.0).abs() < 1e-3, "off the circle: {at:?}");
        }
    }
}

/// Along an edge of the solid: the rim of a disc of radius 30, found on
/// the solid where the pattern stands, a hole 5 mm in from it carried
/// round with the rim's turn, eight times.
#[test]
fn holes_patterned_along_a_solid_edge_follow_it() {
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let disc = doc.add_in(
        body,
        "disc",
        circle_sketch_on(plane_at_z(0.0), 0.0, 0.0, 30.0),
    );
    doc.add_in(body, "Disc", pad(disc, 4.0));
    let at = doc.add_in(
        body,
        "hole",
        circle_sketch_on(plane_at_z(4.0), 25.0, 0.0, 2.0),
    );
    let hole = doc.add_in(body, "Hole", pocket(at, 4.0));
    let rim = wb_design::EdgePick {
        faces: [0, 0],
        point: [0.0, 30.0, 4.0],
        direction: [1.0, 0.0, 0.0],
    };
    doc.add_in(
        body,
        "Round the rim",
        path_pattern(
            hole,
            wb_design::PatternPath::Edge(rim),
            wb_design::PathSpacing {
                occurrences: 8,
                align: true,
                ..Default::default()
            },
        ),
    );
    let (volume, holes) = holes_built(&doc, body);
    let want = (std::f64::consts::PI * 900.0 - 8.0 * std::f64::consts::PI * 4.0) * 4.0;
    assert!((volume - want).abs() < 0.5, "{volume}");
    assert_eq!(holes.len(), 8, "{holes:?}");
    for at in &holes {
        assert!((at[0].hypot(at[1]) - 25.0).abs() < 1e-3, "{at:?}");
    }
}

/// A sketch on a pad's top face counts the top's right edge, projected
/// from its own body, in the profile a pocket cuts. The pocket takes that
/// edge away, yet the projection is of the solid where the sketch stands,
/// before the pocket: the rebuilds settle, and when the pad grows taller
/// and wider the sketch rises with the face, the edge follows and the
/// pocket is cut where the face and edge stand.
#[test]
fn an_edge_projected_from_the_sketchs_own_body_follows_it_through_a_pocket() {
    use core_document::{CommandArgs, Workbench, WorkbenchFeature, WorkbenchRuntimeContext};
    let mut registry = registry();
    registry.set_kernel(&kernel_ogeom::QUERIES);
    // A block 40 x 40, 10 high.
    let (mut doc, body, base) = setup(40.0, 40.0);
    let pad_id = doc.add_in(body, "Pad", pad(base, 10.0));
    doc.mark_feature_stale(pad_id);
    settle_as_host(&registry, &mut doc);

    // A sketch on the top face, following it.
    let plane = wb_sketch::sketch::SketchPlane::from_frame(
        [0.0, 0.0, 10.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
    );
    let mut on_top = SketchFeature::new(Sketch::new("top"), plane);
    on_top.face = Some(wb_sketch::FaceSupport {
        point: [20.0, 20.0, 10.0],
        normal: [0.0, 0.0, 1.0],
        name: 0,
        placed: plane,
        lent_by: None,
    });
    let sketch_id = doc.add_in(body, "top", on_top);
    // The top's right edge, counting, closed by three drawn lines into a
    // strip 10 mm wide along it.
    let mut sketcher = wb_sketch::SketchWorkbench::default();
    let args: CommandArgs = serde_json::from_value(serde_json::json!({
        "sketch": sketch_id.0.to_string(),
        "counts": true,
        "edges": [{"body": body.0.to_string(), "point": [40.0, 20.0, 10.0], "direction": [0.0, 1.0, 0.0]}],
    }))
    .unwrap();
    let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
    ctx.kernel = Some(&kernel_ogeom::QUERIES);
    sketcher
        .run_command("sketch.external", &args, &mut ctx)
        .expect("the edge projects");
    let mut feature = SketchFeature::from_json(doc.get_feature_data(sketch_id).unwrap()).unwrap();
    let edge = feature
        .sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Line(l) if feature.sketch.is_external(l.id) => Some(l.clone()),
            _ => None,
        })
        .expect("the projected edge");
    let y_of = |sketch: &Sketch, p| sketch.point_position(p).unwrap().y;
    let (near, far) = if y_of(&feature.sketch, edge.start) < y_of(&feature.sketch, edge.end) {
        (edge.start, edge.end)
    } else {
        (edge.end, edge.start)
    };
    let sketch = &mut feature.sketch;
    let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(30.0, 0.0))));
    let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(30.0, 40.0))));
    for (s, e) in [(a, near), (far, b), (b, a)] {
        sketch.add_geometry(GeometryElement::Line(Line::new(s, e)));
    }
    doc.set_feature(sketch_id, feature);
    let pocket = doc.add_in(body, "Pocket", pocket(sketch_id, 3.0));
    doc.mark_feature_stale(pocket);
    let passes = settle_as_host(&registry, &mut doc);
    assert!(passes <= 4, "settled in {passes} passes");
    let volume = body_volume(&doc, body);
    assert!(
        (volume - (40.0 * 40.0 * 10.0 - 10.0 * 40.0 * 3.0)).abs() < 1e-3,
        "{volume}"
    );

    // The pad grows to 50 wide and 15 high.
    let stored = doc.get_feature_data(sketch_id).unwrap().clone();
    let mut wider = SketchFeature::from_json(doc.get_feature_data(base).unwrap()).unwrap();
    for element in &mut wider.sketch.geometry {
        if let GeometryElement::Point(p) = element
            && p.position.x > 39.0
        {
            p.position.x = 50.0;
        }
    }
    doc.set_feature(base, wider);
    doc.set_feature(pad_id, pad(base, 15.0));
    doc.mark_feature_dirty(pad_id);
    let passes = settle_as_host(&registry, &mut doc);
    assert!(passes <= 6, "settled in {passes} passes");
    let followed = SketchFeature::from_json(doc.feature_values(sketch_id).unwrap()).unwrap();
    assert!(
        (followed.plane.origin[2] - 15.0).abs() < 1e-6,
        "{:?}",
        followed.plane
    );
    for end in [near, far] {
        let x = followed.sketch.point_position(end).unwrap().x;
        assert!((x - 50.0).abs() < 1e-6, "the edge's end at x = {x}");
    }
    let volume = body_volume(&doc, body);
    assert!(
        (volume - (50.0 * 40.0 * 15.0 - 20.0 * 40.0 * 3.0)).abs() < 1e-3,
        "{volume}"
    );
    let (min, max) = mesh_bounds(&doc.imported_geometry(body).unwrap().mesh);
    assert!(
        (max[0] - 50.0).abs() < 1e-3 && (max[2] - 15.0).abs() < 1e-3,
        "{min:?} {max:?}"
    );
    assert_eq!(
        doc.get_feature_data(sketch_id),
        Some(&stored),
        "followed as derived values, the sketch's own data untouched"
    );
    // Nothing is reported lost.
    let sketcher = wb_sketch::SketchWorkbench::default();
    assert_eq!(
        sketcher.feature_warning(&doc, Some(&kernel_ogeom::QUERIES), sketch_id),
        None
    );
}
