//! Datums standing on a solid follow it through the real benches and the
//! kernel: a rebuild finds their references again where they stand in the
//! history, and what is built on them follows.

use core_document::{
    AttachmentOffset, BodyId, DatumAttachment, DatumFeature, DatumShape, Document, DocumentService,
    EdgeAnchor, FaceAnchor, FeatureId, ImportedGeometry, WorkbenchFeature,
};
use kernel_api::{Placement, PrimitiveKind, TessellationSettings};
use kernel_ogeom::OgeomKernel;
use wb_design::DesignFeature;
use wb_sketch::SketchFeature;
use wb_sketch::sketch::{GeometryElement, Line, Point, Sketch, Vec2D};

fn registry() -> DocumentService {
    let mut registry = DocumentService::default();
    registry
        .register_workbench(Box::new(wb_sketch::SketchWorkbench::default()))
        .unwrap();
    registry
        .register_workbench(Box::new(wb_design::DesignWorkbench::default()))
        .unwrap();
    registry
}

fn cylinder(radius: f64, height: f64) -> DesignFeature {
    DesignFeature::Primitive {
        attached: None,
        kind: PrimitiveKind::Cylinder {
            radius,
            height,
            angle_deg: 360.0,
        },
        placement: Placement::default(),
        subtractive: false,
        refine: false,
    }
}

fn datum(shape: DatumShape, attachment: DatumAttachment) -> DatumFeature {
    DatumFeature {
        shape,
        attachment,
        offset: AttachmentOffset::default(),
    }
}

/// Build every body that asks, taking what the builds find, until nothing
/// is left to build, as the app's recompute loop does. Answers the bounds
/// of `body`'s solid.
fn settle(registry: &DocumentService, doc: &mut Document, body: BodyId) -> ([f32; 3], [f32; 3]) {
    let mut kernel = OgeomKernel::new();
    for _ in 0..8 {
        registry.evaluate(doc);
        let jobs = registry.rebuild_jobs(doc);
        if jobs.is_empty() {
            break;
        }
        for job in jobs {
            let plan = job.plan.expect("the history translates");
            let asked: Vec<kernel_api::ChainProbe> = plan.probes.iter().map(|p| p.probe).collect();
            let tags: Vec<kernel_api::TopoName> = plan
                .op_features
                .iter()
                .map(|f| kernel_api::naming::name_of_id(f.0.as_bytes()))
                .collect();
            let result = kernel
                .execute_solid_chain_named(
                    &plan.ops,
                    &tags,
                    &TessellationSettings::default(),
                    None,
                    &asked,
                )
                .expect("the body builds");
            doc.store_probe_answers(&plan.probes, &result.probes);
            doc.set_imported_brep_data(job.body, result.brep_blob, Vec::new());
            doc.set_imported_geometry(
                job.body,
                ImportedGeometry {
                    mesh: std::sync::Arc::new(result.mesh),
                    source_asset: None,
                    revision: 0,
                    bounds_mm: result.bounds_mm,
                    brep_blob_path: None,
                    mesh_path: None,
                    face_colors_path: None,
                    health: None,
                },
            );
        }
    }
    assert!(registry.rebuild_jobs(doc).is_empty(), "the builds settle");
    doc.imported_geometry(body)
        .and_then(|g| g.bounds_mm)
        .expect("the body has a solid")
}

fn frame_of(doc: &Document, id: FeatureId) -> core_document::DatumFrame {
    DatumFeature::from_json(doc.feature_values(id).unwrap())
        .unwrap()
        .frame()
}

fn near(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3)
}

fn rect_sketch(width: f32, height: f32) -> SketchFeature {
    let mut sketch = Sketch::new("s");
    let corners = [(0.0, 0.0), (width, 0.0), (width, height), (0.0, height)]
        .map(|(x, y)| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y)))));
    for i in 0..4 {
        sketch.add_geometry(GeometryElement::Line(Line::new(
            corners[i],
            corners[(i + 1) % 4],
        )));
    }
    let plane = sketch.plane;
    SketchFeature::new(sketch, plane)
}

/// A plane tangent to a cylinder, a sketch on it and a pad on the sketch:
/// when the cylinder grows, the plane stays tangent and the pad goes with
/// it.
#[test]
fn a_tangent_plane_and_the_pad_on_it_follow_the_cylinder_s_radius() {
    let registry = registry();
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let base = doc
        .add_feature_in_body(cylinder(5.0, 10.0), "Cylinder".into(), Some(body))
        .unwrap();
    let tangent = doc
        .add_feature_in_body(
            datum(
                DatumShape::Plane { size: 20.0 },
                DatumAttachment::Face {
                    face: FaceAnchor {
                        name: 0,
                        point: [0.0, 5.0, 5.0],
                        normal: [0.0, 1.0, 0.0],
                        surface: None,
                        follows: true,
                    },
                },
            ),
            "Tangent".into(),
            Some(body),
        )
        .unwrap();
    // A 4 x 2 rectangle on the plane: along its x (the cylinder's axis)
    // and its y (across it), padded 3 out along its normal.
    let mut sketch = rect_sketch(4.0, 2.0);
    sketch.support = Some(wb_sketch::DatumSupport {
        datum: tangent,
        plane: None,
        offset: 0.0,
        shift: [0.0, 0.0],
        turn: 0.0,
    });
    let sketch = doc
        .add_feature_in_body(sketch, "Sketch".into(), Some(body))
        .unwrap();
    doc.add_feature_in_body(
        DesignFeature::Pad {
            profile_borrowed: None,
            extras: Default::default(),
            refine: false,
            sketch: Some(sketch),
            length: 3.0,
            reversed: false,
            symmetric: false,
            mode: wb_design::ExtrudeMode::Dimension,
            length2: 0.0,
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
        },
        "Pad".into(),
        Some(body),
    )
    .unwrap();

    let (_, max) = settle(&registry, &mut doc, body);
    let frame = frame_of(&doc, tangent);
    assert!(near(frame.origin, [0.0, 5.0, 5.0]), "{frame:?}");
    assert!(near(frame.normal, [0.0, 1.0, 0.0]), "{frame:?}");
    assert!((max[1] - 8.0).abs() < 1e-3, "the pad stands on it: {max:?}");

    doc.update_feature_data(base, cylinder(8.0, 10.0).to_json())
        .unwrap();
    doc.mark_feature_dirty(base);
    let (_, max) = settle(&registry, &mut doc, body);
    let frame = frame_of(&doc, tangent);
    assert!(near(frame.origin, [0.0, 8.0, 5.0]), "followed: {frame:?}");
    assert!(near(frame.normal, [0.0, 1.0, 0.0]), "{frame:?}");
    assert!(
        (max[1] - 11.0).abs() < 1e-3,
        "the pad followed the plane: {max:?}"
    );
    assert!(
        doc.get_feature_meta(tangent).unwrap().error.is_none(),
        "the face was found"
    );
}

/// A point at a rim's centre and a coordinate system on the centre of mass
/// follow the cylinder they stand on as it changes.
#[test]
fn centres_follow_the_solid_they_are_found_on() {
    let registry = registry();
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let base = doc
        .add_feature_in_body(cylinder(5.0, 10.0), "Cylinder".into(), Some(body))
        .unwrap();
    let rim = doc
        .add_feature_in_body(
            datum(
                DatumShape::Point,
                DatumAttachment::CurveCentre {
                    edge: EdgeAnchor {
                        along: None,
                        faces: [0, 0],
                        point: [5.0, 0.0, 10.0],
                        direction: [0.0, 1.0, 0.0],
                        ends: None,
                        middle: None,
                        circle: None,
                        follows: true,
                    },
                },
            ),
            "Rim centre".into(),
            Some(body),
        )
        .unwrap();
    let mass = doc
        .add_feature_in_body(
            datum(
                DatumShape::CoordinateSystem { size: 10.0 },
                DatumAttachment::Inertia {
                    centre: [0.0; 3],
                    axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                },
            ),
            "Centre of mass".into(),
            Some(body),
        )
        .unwrap();

    settle(&registry, &mut doc, body);
    assert!(near(frame_of(&doc, rim).origin, [0.0, 0.0, 10.0]));
    let centre = frame_of(&doc, mass);
    assert!(near(centre.origin, [0.0, 0.0, 5.0]), "{centre:?}");
    assert!(
        centre.x_axis[2].abs() > 1.0 - 1e-4,
        "this cylinder turns most easily about its own axis: {centre:?}"
    );

    // Wider: the rim's centre is found on the new rim, at the same place.
    doc.update_feature_data(base, cylinder(8.0, 10.0).to_json())
        .unwrap();
    doc.mark_feature_dirty(base);
    settle(&registry, &mut doc, body);
    assert!(
        near(frame_of(&doc, rim).origin, [0.0, 0.0, 10.0]),
        "{:?}",
        frame_of(&doc, rim)
    );
    assert!(doc.get_feature_meta(rim).unwrap().error.is_none());

    // Flatter: the centre of mass drops, and the axis of the least moment
    // turns from the cylinder's own to a diameter.
    doc.update_feature_data(base, cylinder(5.0, 2.0).to_json())
        .unwrap();
    doc.mark_feature_dirty(base);
    settle(&registry, &mut doc, body);
    let centre = frame_of(&doc, mass);
    assert!(near(centre.origin, [0.0, 0.0, 1.0]), "{centre:?}");
    assert!(
        centre.x_axis[2].abs() < 1e-4,
        "a disc turns most easily about a diameter: {centre:?}"
    );
}

fn volume(doc: &Document, body: BodyId) -> f64 {
    let blob = doc.imported_brep_blob(body).expect("the body has a solid");
    OgeomKernel::new()
        .physical_properties(blob)
        .expect("the solid measures")
        .volume_mm3
        .expect("the solid is closed")
}

/// A revolution turns about a datum line along a cylinder's rim, which is
/// the cylinder's axis; when the cylinder moves, the line and the ring
/// turned about it go with it.
#[test]
fn a_revolution_turns_about_a_datum_line_that_follows_a_rim() {
    use std::f64::consts::PI;
    let registry = registry();
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let base = doc
        .add_feature_in_body(cylinder(2.0, 10.0), "Cylinder".into(), Some(body))
        .unwrap();
    // The rim as a pick brings it: its circle known, as `design.datum` and
    // the task fill it in from the body's solid.
    let axis = doc
        .add_feature_in_body(
            datum(
                DatumShape::Line { length: 20.0 },
                DatumAttachment::AlongEdge {
                    edge: EdgeAnchor {
                        along: None,
                        faces: [0, 0],
                        point: [2.0, 0.0, 10.0],
                        direction: [0.0, 1.0, 0.0],
                        ends: None,
                        middle: None,
                        circle: Some(core_document::AnchorCircle {
                            center: [0.0, 0.0, 10.0],
                            normal: [0.0, 0.0, 1.0],
                            radius: 2.0,
                        }),
                        follows: true,
                    },
                },
            ),
            "Axis".into(),
            Some(body),
        )
        .unwrap();
    // A 3 x 2 rectangle on XZ, 5 to 8 out along x, turned about the axis.
    let mut ring = rect_sketch(3.0, 2.0);
    let plane = wb_sketch::sketch::SketchPlane::xz();
    ring.sketch.plane = plane;
    ring.plane = plane;
    for element in ring.sketch.geometry.iter_mut() {
        if let GeometryElement::Point(p) = element {
            p.position.x += 5.0;
        }
    }
    let ring = doc
        .add_feature_in_body(ring, "Ring".into(), Some(body))
        .unwrap();
    doc.add_feature_in_body(
        DesignFeature::Revolution {
            refine: false,
            sketch: ring,
            angle_deg: 360.0,
            axis: wb_design::RevolveAxis::Datum(axis),
            reversed: false,
            midplane: false,
            second_angle_deg: None,
            mode: wb_design::RevolveMode::Angle,
            up_to_face: None,
        },
        "Revolution".into(),
        Some(body),
    )
    .unwrap();

    settle(&registry, &mut doc, body);
    let frame = frame_of(&doc, axis);
    assert!(near(frame.origin, [0.0, 0.0, 10.0]), "{frame:?}");
    let cylinder_volume = PI * 4.0 * 10.0;
    let want = cylinder_volume + PI * (64.0 - 25.0) * 2.0;
    let got = volume(&doc, body);
    assert!((got - want).abs() < want * 1e-4, "{got} against {want}");

    // Moved 1 along x: the ring turns about x = 1, from 4 to 7 out.
    let mut moved = cylinder(2.0, 10.0);
    if let DesignFeature::Primitive { placement, .. } = &mut moved {
        placement.origin = [1.0, 0.0, 0.0];
    }
    doc.update_feature_data(base, moved.to_json()).unwrap();
    doc.mark_feature_dirty(base);
    settle(&registry, &mut doc, body);
    let frame = frame_of(&doc, axis);
    assert!(
        near(frame.origin, [1.0, 0.0, 10.0]),
        "followed: {frame:?} {:?} {:?}",
        doc.get_feature_meta(axis).unwrap().error,
        doc.probed_references(axis)
    );
    let want = cylinder_volume + PI * (49.0 - 16.0) * 2.0;
    let got = volume(&doc, body);
    assert!((got - want).abs() < want * 1e-4, "{got} against {want}");
}

fn pad(sketch: FeatureId, length: f32) -> DesignFeature {
    DesignFeature::Pad {
        profile_borrowed: None,
        extras: Default::default(),
        refine: false,
        sketch: Some(sketch),
        length,
        reversed: false,
        symmetric: false,
        mode: wb_design::ExtrudeMode::Dimension,
        length2: 0.0,
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

fn pocket(sketch: FeatureId, depth: f32) -> DesignFeature {
    DesignFeature::Pocket {
        profile_borrowed: None,
        extras: Default::default(),
        refine: false,
        sketch: Some(sketch),
        depth,
        reversed: false,
        symmetric: false,
        through_all: false,
        mode: wb_design::ExtrudeMode::Dimension,
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

/// The heights of the flat faces of `body`'s mesh that face up.
fn upward_floors(doc: &Document, body: BodyId) -> Vec<f32> {
    let mesh = &doc.imported_geometry(body).unwrap().mesh;
    let mut heights: Vec<f32> = mesh
        .face_surfaces
        .iter()
        .filter_map(|s| match s {
            kernel_api::FaceSurface::Plane { origin, normal } if normal[2] > 0.99 => {
                Some((origin[2] * 1000.0).round() / 1000.0)
            }
            _ => None,
        })
        .collect();
    heights.sort_by(f32::total_cmp);
    heights.dedup();
    heights
}

/// A pocket drawn on the pad's top stays at the top when the pad grows:
/// its sketch follows the face it was placed on, found by its name.
#[test]
fn a_sketch_on_a_face_follows_the_face_when_the_pad_grows() {
    let registry = registry();
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let base = doc
        .add_feature_in_body(rect_sketch(20.0, 20.0), "base".into(), Some(body))
        .unwrap();
    let pad_id = doc
        .add_feature_in_body(pad(base, 10.0), "Pad".into(), Some(body))
        .unwrap();
    doc.mark_feature_dirty(pad_id);
    settle(&registry, &mut doc, body);

    // The top face, as a click picks it.
    let mesh = doc.imported_geometry(body).unwrap().mesh.clone();
    let top_id = mesh
        .face_surfaces
        .iter()
        .position(|s| {
            matches!(s, kernel_api::FaceSurface::Plane { origin, normal }
                if normal[2] > 0.99 && (origin[2] - 10.0).abs() < 1e-3)
        })
        .expect("a top face");
    let top = core_document::FaceRef {
        point: [10.0, 10.0, 10.0],
        normal: [0.0, 0.0, 1.0],
        surface: None,
        name: mesh.face_names[top_id],
    };
    assert_ne!(top.name, 0, "the top is named");
    let plane = wb_sketch::sketch::SketchPlane::from_face(top.point, top.normal);
    let mut hole = Sketch::new("hole");
    hole.plane = plane;
    let centre = hole.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 10.0))));
    hole.add_geometry(GeometryElement::Circle(wb_sketch::sketch::Circle::new(
        centre, 3.0,
    )));
    let mut hole = SketchFeature::new(hole, plane);
    hole.face = Some(wb_sketch::FaceSupport::on(&top, plane));
    let hole = doc
        .add_feature_in_body(hole, "hole".into(), Some(body))
        .unwrap();
    let pocket_id = doc
        .add_feature_in_body(pocket(hole, 3.0), "Pocket".into(), Some(body))
        .unwrap();
    doc.mark_feature_dirty(pocket_id);
    settle(&registry, &mut doc, body);
    assert_eq!(
        upward_floors(&doc, body),
        [7.0, 10.0],
        "the pocket's floor and the top"
    );

    // The pad grows: the top rises to 20, and the pocket with it.
    let mut data = doc.get_feature_data(pad_id).unwrap().clone();
    data["Pad"]["length"] = serde_json::json!(20.0);
    doc.update_feature_data(pad_id, data).unwrap();
    doc.mark_feature_dirty(pad_id);
    settle(&registry, &mut doc, body);
    assert_eq!(
        upward_floors(&doc, body),
        [17.0, 20.0],
        "the pocket went up with the top"
    );
}

/// A sketch placed on a side face stands on that face once the body is
/// built and the sketch's place worked out again: not on an origin plane.
#[test]
fn a_sketch_on_a_side_face_stays_on_that_face() {
    let registry = registry();
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let base = doc
        .add_feature_in_body(rect_sketch(20.0, 20.0), "base".into(), Some(body))
        .unwrap();
    let pad_id = doc
        .add_feature_in_body(pad(base, 10.0), "Pad".into(), Some(body))
        .unwrap();
    doc.mark_feature_dirty(pad_id);
    settle(&registry, &mut doc, body);

    let mesh = doc.imported_geometry(body).unwrap().mesh.clone();
    let side_id = mesh
        .face_surfaces
        .iter()
        .position(|s| {
            matches!(s, kernel_api::FaceSurface::Plane { origin, normal }
                if normal[0] > 0.99 && (origin[0] - 20.0).abs() < 1e-3)
        })
        .expect("the +X side");
    let side = core_document::FaceRef {
        point: [20.0, 10.0, 5.0],
        normal: [1.0, 0.0, 0.0],
        surface: None,
        name: mesh.face_names[side_id],
    };
    let plane = wb_sketch::sketch::SketchPlane::from_face(side.point, side.normal);
    let mut sketch = Sketch::new("side");
    sketch.plane = plane;
    let mut sketch = SketchFeature::new(sketch, plane);
    sketch.face = Some(wb_sketch::FaceSupport::on(&side, plane));
    let id = doc
        .add_feature_in_body(sketch, "side".into(), Some(body))
        .unwrap();
    settle(&registry, &mut doc, body);

    let placed = SketchFeature::from_json(doc.feature_values(id).unwrap())
        .unwrap()
        .plane;
    let near = |a: [f32; 3], b: [f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3);
    assert!(
        near(placed.normal, [1.0, 0.0, 0.0]),
        "normal {:?}",
        placed.normal
    );
    assert!(
        (placed.origin[0] - 20.0).abs() < 1e-3,
        "origin {:?}",
        placed.origin
    );
}

/// A sketch placed on a face another body lends follows the face: body B's
/// top, borrowed by body A, rises when B's pad grows, and A's post drawn
/// on it rises with it.
#[test]
fn a_sketch_on_a_lent_face_follows_the_lender() {
    let registry = registry();
    let mut doc = Document::new("t");
    let b = doc.create_body(Some("B".into()));
    let base = doc
        .add_feature_in_body(rect_sketch(20.0, 20.0), "base".into(), Some(b))
        .unwrap();
    let block = doc
        .add_feature_in_body(pad(base, 10.0), "Block".into(), Some(b))
        .unwrap();
    doc.mark_feature_dirty(block);
    settle(&registry, &mut doc, b);

    let mesh = doc.imported_geometry(b).unwrap().mesh.clone();
    let top_id = mesh
        .face_surfaces
        .iter()
        .position(|s| {
            matches!(s, kernel_api::FaceSurface::Plane { origin, normal }
                if normal[2] > 0.99 && (origin[2] - 10.0).abs() < 1e-3)
        })
        .expect("a top face");
    let a = doc.create_body(Some("A".into()));
    let lent = doc
        .add_feature_in_body(
            DesignFeature::Borrow {
                options: Default::default(),
                source: wb_design::BorrowSource::Solid {
                    body: b,
                    faces: vec![wb_design::FacePick {
                        point: [10.0, 10.0, 10.0],
                        normal: [0.0, 0.0, 1.0],
                        name: mesh.face_names[top_id],
                    }],
                    edges: Vec::new(),
                },
                frozen: None,
            },
            "Borrowed".into(),
            Some(a),
        )
        .unwrap();
    let top = core_document::FaceRef {
        point: [10.0, 10.0, 10.0],
        normal: [0.0, 0.0, 1.0],
        surface: None,
        name: mesh.face_names[top_id],
    };
    let plane = wb_sketch::sketch::SketchPlane::from_face(top.point, top.normal);
    let mut post = Sketch::new("post");
    post.plane = plane;
    let centre = post.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 10.0))));
    post.add_geometry(GeometryElement::Circle(wb_sketch::sketch::Circle::new(
        centre, 3.0,
    )));
    let mut post = SketchFeature::new(post, plane);
    post.face = wb_sketch::FaceSupport::from_origin(
        &top,
        core_document::FaceOrigin::Lent {
            borrow: lent,
            index: 0,
        },
        plane,
    );
    let post = doc
        .add_feature_in_body(post, "post".into(), Some(a))
        .unwrap();
    let post_pad = doc
        .add_feature_in_body(pad(post, 5.0), "Post".into(), Some(a))
        .unwrap();
    doc.mark_feature_dirty(post_pad);
    let (low, high) = settle(&registry, &mut doc, a);
    assert!(
        (low[2] - 10.0).abs() < 1e-3 && (high[2] - 15.0).abs() < 1e-3,
        "{low:?} {high:?}"
    );

    // B's block grows: its top rises to 20, and A's post with it.
    let mut data = doc.get_feature_data(block).unwrap().clone();
    data["Pad"]["length"] = serde_json::json!(20.0);
    doc.update_feature_data(block, data).unwrap();
    doc.mark_feature_dirty(block);
    settle(&registry, &mut doc, b);
    let (low, high) = settle(&registry, &mut doc, a);
    assert!(
        (low[2] - 20.0).abs() < 1e-3 && (high[2] - 25.0).abs() < 1e-3,
        "{low:?} {high:?}"
    );
}

/// A sketch attached to the pad's top by the face mode, as a datum plane
/// would be, follows the top when the pad grows, and the pocket drawn on
/// it with it.
#[test]
fn a_sketch_attached_by_a_mode_follows_what_it_stands_on() {
    let registry = registry();
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let base = doc
        .add_feature_in_body(rect_sketch(20.0, 20.0), "base".into(), Some(body))
        .unwrap();
    let pad_id = doc
        .add_feature_in_body(pad(base, 10.0), "Pad".into(), Some(body))
        .unwrap();
    doc.mark_feature_dirty(pad_id);
    settle(&registry, &mut doc, body);

    let mesh = doc.imported_geometry(body).unwrap().mesh.clone();
    let top_id = mesh
        .face_surfaces
        .iter()
        .position(|s| {
            matches!(s, kernel_api::FaceSurface::Plane { origin, normal }
                if normal[2] > 0.99 && (origin[2] - 10.0).abs() < 1e-3)
        })
        .expect("a top face");
    let top = core_document::FaceRef {
        point: [10.0, 10.0, 10.0],
        normal: [0.0, 0.0, 1.0],
        surface: None,
        name: mesh.face_names[top_id],
    };
    let attached = wb_sketch::AttachedSupport {
        attachment: core_document::DatumAttachment::Face {
            face: core_document::attach::face_anchor(&top, true),
        },
        offset: AttachmentOffset::default(),
    };
    let plane = attached.plane();
    let mut hole = Sketch::new("hole");
    hole.plane = plane;
    // The attachment's frame: the circle is placed about its origin.
    let centre = hole.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
    hole.add_geometry(GeometryElement::Circle(wb_sketch::sketch::Circle::new(
        centre, 3.0,
    )));
    let mut hole = SketchFeature::new(hole, plane);
    hole.attached = Some(attached);
    let hole = doc
        .add_feature_in_body(hole, "hole".into(), Some(body))
        .unwrap();
    let pocket_id = doc
        .add_feature_in_body(pocket(hole, 3.0), "Pocket".into(), Some(body))
        .unwrap();
    doc.mark_feature_dirty(pocket_id);
    settle(&registry, &mut doc, body);
    assert_eq!(upward_floors(&doc, body), [7.0, 10.0]);

    let mut data = doc.get_feature_data(pad_id).unwrap().clone();
    data["Pad"]["length"] = serde_json::json!(20.0);
    doc.update_feature_data(pad_id, data).unwrap();
    doc.mark_feature_dirty(pad_id);
    settle(&registry, &mut doc, body);
    assert_eq!(
        upward_floors(&doc, body),
        [17.0, 20.0],
        "the attached sketch went up with the top"
    );
}

/// A box attached to the pad's top face sits on it, and goes up with the
/// top when the pad grows.
#[test]
fn a_primitive_attached_by_a_mode_follows_what_it_stands_on() {
    let registry = registry();
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let base = doc
        .add_feature_in_body(rect_sketch(20.0, 20.0), "base".into(), Some(body))
        .unwrap();
    let pad_id = doc
        .add_feature_in_body(pad(base, 10.0), "Pad".into(), Some(body))
        .unwrap();
    doc.mark_feature_dirty(pad_id);
    settle(&registry, &mut doc, body);
    let mesh = doc.imported_geometry(body).unwrap().mesh.clone();
    let top_id = mesh
        .face_surfaces
        .iter()
        .position(|s| {
            matches!(s, kernel_api::FaceSurface::Plane { origin, normal }
                if normal[2] > 0.99 && (origin[2] - 10.0).abs() < 1e-3)
        })
        .expect("a top face");
    let top = core_document::FaceRef {
        point: [10.0, 10.0, 10.0],
        normal: [0.0, 0.0, 1.0],
        surface: None,
        name: mesh.face_names[top_id],
    };
    let cube = DesignFeature::Primitive {
        kind: kernel_api::PrimitiveKind::Box {
            length: 4.0,
            width: 4.0,
            height: 4.0,
        },
        placement: kernel_api::Placement::default(),
        subtractive: false,
        refine: false,
        attached: Some(Box::new(wb_design::Attached {
            attachment: core_document::DatumAttachment::Face {
                face: core_document::attach::face_anchor(&top, true),
            },
            offset: AttachmentOffset::default(),
        })),
    };
    let cube_id = doc
        .add_feature_in_body(cube, "Cube".into(), Some(body))
        .unwrap();
    doc.mark_feature_dirty(cube_id);
    let (_, hi) = settle(&registry, &mut doc, body);
    assert!((hi[2] - 14.0).abs() < 1e-3, "on the top: {hi:?}");

    let mut data = doc.get_feature_data(pad_id).unwrap().clone();
    data["Pad"]["length"] = serde_json::json!(20.0);
    doc.update_feature_data(pad_id, data).unwrap();
    doc.mark_feature_dirty(pad_id);
    let (_, hi) = settle(&registry, &mut doc, body);
    assert!((hi[2] - 24.0).abs() < 1e-3, "went up with the top: {hi:?}");
}
