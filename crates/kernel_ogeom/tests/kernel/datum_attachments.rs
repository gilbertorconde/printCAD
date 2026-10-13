//! Datums standing on a solid follow it through the real benches and the
//! kernel: a rebuild finds their references again where they stand in the
//! history, and what is built on them follows.

use core_document::{
    AttachmentOffset, BodyId, DatumAttachment, DatumFeature, DatumShape, Document, DocumentService,
    EdgeAnchor, FaceAnchor, FeatureId, WorkbenchFeature,
};
use kernel_api::{Placement, PrimitiveKind};
use wb_design::DesignFeature;
use wb_sketch::SketchFeature;
use wb_sketch::sketch::{GeometryElement, Point, Sketch, SketchPlane, Vec2D};

use super::common::{Features, body_volume, pad, pocket, rect_at, registry, settle_as_host};

fn cylinder(radius: f64, height: f64) -> DesignFeature {
    DesignFeature::Primitive {
        keep_inside: false,
        attached: None,
        kind: PrimitiveKind::Cylinder {
            radius,
            height,
            angle_deg: 360.0,
        },
        placement: Placement::default(),
        subtractive: false,
        refine: false,
        boolean_tolerance: None,
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
    settle_as_host(registry, doc);
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

/// A plane tangent to a cylinder, a sketch on it and a pad on the sketch:
/// when the cylinder grows, the plane stays tangent and the pad goes with
/// it.
#[test]
fn a_tangent_plane_and_the_pad_on_it_follow_the_cylinder_s_radius() {
    let registry = registry();
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let base = doc.add_in(body, "Cylinder", cylinder(5.0, 10.0));
    let tangent = doc.add_in(
        body,
        "Tangent",
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
    );
    // A 4 x 2 rectangle on the plane: along its x (the cylinder's axis)
    // and its y (across it), padded 3 out along its normal.
    let mut sketch = rect_at(SketchPlane::xy(), 0.0, 0.0, 4.0, 2.0);
    sketch.support = Some(wb_sketch::DatumSupport {
        datum: tangent,
        plane: None,
        offset: 0.0,
        shift: [0.0, 0.0],
        turn: 0.0,
    });
    let sketch = doc.add_in(body, "Sketch", sketch);
    doc.add_in(
        body,
        "Pad",
        DesignFeature::Pad {
            regions: Vec::new(),
            profile_borrowed: None,
            extras: Default::default(),
            refine: false,
            boolean_tolerance: None,
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
    );

    let (_, max) = settle(&registry, &mut doc, body);
    let frame = frame_of(&doc, tangent);
    assert!(near(frame.origin, [0.0, 5.0, 5.0]), "{frame:?}");
    assert!(near(frame.normal, [0.0, 1.0, 0.0]), "{frame:?}");
    assert!((max[1] - 8.0).abs() < 1e-3, "the pad stands on it: {max:?}");

    doc.set_feature(base, cylinder(8.0, 10.0));
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
    let base = doc.add_in(body, "Cylinder", cylinder(5.0, 10.0));
    let rim = doc.add_in(
        body,
        "Rim centre",
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
    );
    let mass = doc.add_in(
        body,
        "Centre of mass",
        datum(
            DatumShape::CoordinateSystem { size: 10.0 },
            DatumAttachment::Inertia {
                centre: [0.0; 3],
                axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            },
        ),
    );

    settle(&registry, &mut doc, body);
    assert!(near(frame_of(&doc, rim).origin, [0.0, 0.0, 10.0]));
    let centre = frame_of(&doc, mass);
    assert!(near(centre.origin, [0.0, 0.0, 5.0]), "{centre:?}");
    assert!(
        centre.x_axis[2].abs() > 1.0 - 1e-4,
        "this cylinder turns most easily about its own axis: {centre:?}"
    );

    // Wider: the rim's centre is found on the new rim, at the same place.
    doc.set_feature(base, cylinder(8.0, 10.0));
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
    doc.set_feature(base, cylinder(5.0, 2.0));
    doc.mark_feature_dirty(base);
    settle(&registry, &mut doc, body);
    let centre = frame_of(&doc, mass);
    assert!(near(centre.origin, [0.0, 0.0, 1.0]), "{centre:?}");
    assert!(
        centre.x_axis[2].abs() < 1e-4,
        "a disc turns most easily about a diameter: {centre:?}"
    );
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
    let base = doc.add_in(body, "Cylinder", cylinder(2.0, 10.0));
    // The rim as a pick brings it: its circle known, as `design.datum` and
    // the task fill it in from the body's solid.
    let axis = doc.add_in(
        body,
        "Axis",
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
    );
    // A 3 x 2 rectangle on XZ, 5 to 8 out along x, turned about the axis.
    let mut ring = rect_at(SketchPlane::xy(), 0.0, 0.0, 3.0, 2.0);
    let plane = wb_sketch::sketch::SketchPlane::xz();
    ring.sketch.plane = plane;
    ring.plane = plane;
    for element in ring.sketch.geometry.iter_mut() {
        if let GeometryElement::Point(p) = element {
            p.position.x += 5.0;
        }
    }
    let ring = doc.add_in(body, "Ring", ring);
    doc.add_in(
        body,
        "Revolution",
        DesignFeature::Revolution {
            regions: Vec::new(),
            refine: false,
            boolean_tolerance: None,
            sketch: ring,
            angle_deg: 360.0,
            axis: wb_design::RevolveAxis::Datum(axis),
            project_axis: false,
            reversed: false,
            midplane: false,
            second_angle_deg: None,
            start_angle_deg: 0.0,
            mode: wb_design::RevolveMode::Angle,
            up_to_face: None,
        },
    );

    settle(&registry, &mut doc, body);
    let frame = frame_of(&doc, axis);
    assert!(near(frame.origin, [0.0, 0.0, 10.0]), "{frame:?}");
    let cylinder_volume = PI * 4.0 * 10.0;
    let want = cylinder_volume + PI * (64.0 - 25.0) * 2.0;
    let got = body_volume(&doc, body);
    assert!((got - want).abs() < want * 1e-4, "{got} against {want}");

    // Moved 1 along x: the ring turns about x = 1, from 4 to 7 out.
    let mut moved = cylinder(2.0, 10.0);
    if let DesignFeature::Primitive { placement, .. } = &mut moved {
        placement.origin = [1.0, 0.0, 0.0];
    }
    doc.set_feature(base, moved);
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
    let got = body_volume(&doc, body);
    assert!((got - want).abs() < want * 1e-4, "{got} against {want}");
}

/// Set the pad `id`'s length as a panel edit does, marked to rebuild.
fn set_pad_length(doc: &mut Document, id: FeatureId, length: f32) {
    let mut data = doc.get_feature_data(id).unwrap().clone();
    data["Pad"]["length"] = serde_json::json!(length);
    doc.update_feature_data(id, data).unwrap();
    doc.mark_feature_dirty(id);
}

/// The flat face of `body`'s solid through `point` facing `normal`, as a
/// click picks it, named.
fn picked_face(
    doc: &Document,
    body: BodyId,
    point: [f32; 3],
    normal: [f32; 3],
) -> core_document::FaceRef {
    let mesh = &doc.imported_geometry(body).expect("a solid").mesh;
    let dot = |a: [f32; 3], b: [f32; 3]| (0..3).map(|i| a[i] * b[i]).sum::<f32>();
    let id = mesh
        .face_surfaces
        .iter()
        .position(|s| {
            matches!(s, kernel_api::FaceSurface::Plane { origin, normal: n }
                if dot(*n, normal) > 0.99
                    && dot(std::array::from_fn(|i| origin[i] - point[i]), normal).abs() < 1e-3)
        })
        .expect("the face");
    core_document::FaceRef {
        point,
        normal,
        surface: None,
        name: mesh.face_names[id],
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
    let base = doc.add_in(
        body,
        "base",
        rect_at(SketchPlane::xy(), 0.0, 0.0, 20.0, 20.0),
    );
    let pad_id = doc.add_in(body, "Pad", pad(base, 10.0));
    doc.mark_feature_dirty(pad_id);
    settle(&registry, &mut doc, body);

    // The top face, as a click picks it.
    let top = picked_face(&doc, body, [10.0, 10.0, 10.0], [0.0, 0.0, 1.0]);
    assert_ne!(top.name, 0, "the top is named");
    let plane = wb_sketch::sketch::SketchPlane::from_face(
        wb_sketch::boundary::widen3(top.point),
        wb_sketch::boundary::widen3(top.normal),
    );
    let mut hole = Sketch::new("hole");
    hole.plane = plane;
    let centre = hole.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 10.0))));
    hole.add_geometry(GeometryElement::Circle(wb_sketch::sketch::Circle::new(
        centre, 3.0,
    )));
    let mut hole = SketchFeature::new(hole, plane);
    hole.face = Some(wb_sketch::FaceSupport::on(&top, plane));
    let hole = doc.add_in(body, "hole", hole);
    let pocket_id = doc.add_in(body, "Pocket", pocket(hole, 3.0));
    doc.mark_feature_dirty(pocket_id);
    settle(&registry, &mut doc, body);
    assert_eq!(
        upward_floors(&doc, body),
        [7.0, 10.0],
        "the pocket's floor and the top"
    );

    // The pad grows: the top rises to 20, and the pocket with it.
    set_pad_length(&mut doc, pad_id, 20.0);
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
    let base = doc.add_in(
        body,
        "base",
        rect_at(SketchPlane::xy(), 0.0, 0.0, 20.0, 20.0),
    );
    let pad_id = doc.add_in(body, "Pad", pad(base, 10.0));
    doc.mark_feature_dirty(pad_id);
    settle(&registry, &mut doc, body);

    let side = picked_face(&doc, body, [20.0, 10.0, 5.0], [1.0, 0.0, 0.0]);
    let plane = wb_sketch::sketch::SketchPlane::from_face(
        wb_sketch::boundary::widen3(side.point),
        wb_sketch::boundary::widen3(side.normal),
    );
    let mut sketch = Sketch::new("side");
    sketch.plane = plane;
    let mut sketch = SketchFeature::new(sketch, plane);
    sketch.face = Some(wb_sketch::FaceSupport::on(&side, plane));
    let id = doc.add_in(body, "side", sketch);
    settle(&registry, &mut doc, body);

    let placed = SketchFeature::from_json(doc.feature_values(id).unwrap())
        .unwrap()
        .plane;
    let near = |a: [f64; 3], b: [f64; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3);
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
    let base = doc.add_in(b, "base", rect_at(SketchPlane::xy(), 0.0, 0.0, 20.0, 20.0));
    let block = doc.add_in(b, "Block", pad(base, 10.0));
    doc.mark_feature_dirty(block);
    settle(&registry, &mut doc, b);

    let top = picked_face(&doc, b, [10.0, 10.0, 10.0], [0.0, 0.0, 1.0]);
    let a = doc.create_body(Some("A".into()));
    let lent = doc.add_in(
        a,
        "Borrowed",
        DesignFeature::Borrow {
            options: Default::default(),
            source: wb_design::BorrowSource::Solid {
                body: b,
                faces: vec![wb_design::FacePick {
                    point: [10.0, 10.0, 10.0],
                    normal: [0.0, 0.0, 1.0],
                    name: top.name,
                }],
                edges: Vec::new(),
            },
            frozen: None,
        },
    );
    let plane = wb_sketch::sketch::SketchPlane::from_face(
        wb_sketch::boundary::widen3(top.point),
        wb_sketch::boundary::widen3(top.normal),
    );
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
    let post = doc.add_in(a, "post", post);
    let post_pad = doc.add_in(a, "Post", pad(post, 5.0));
    doc.mark_feature_dirty(post_pad);
    let (low, high) = settle(&registry, &mut doc, a);
    assert!(
        (low[2] - 10.0).abs() < 1e-3 && (high[2] - 15.0).abs() < 1e-3,
        "{low:?} {high:?}"
    );

    // B's block grows: its top rises to 20, and A's post with it.
    set_pad_length(&mut doc, block, 20.0);
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
    let base = doc.add_in(
        body,
        "base",
        rect_at(SketchPlane::xy(), 0.0, 0.0, 20.0, 20.0),
    );
    let pad_id = doc.add_in(body, "Pad", pad(base, 10.0));
    doc.mark_feature_dirty(pad_id);
    settle(&registry, &mut doc, body);

    let top = picked_face(&doc, body, [10.0, 10.0, 10.0], [0.0, 0.0, 1.0]);
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
    let hole = doc.add_in(body, "hole", hole);
    let pocket_id = doc.add_in(body, "Pocket", pocket(hole, 3.0));
    doc.mark_feature_dirty(pocket_id);
    settle(&registry, &mut doc, body);
    assert_eq!(upward_floors(&doc, body), [7.0, 10.0]);

    set_pad_length(&mut doc, pad_id, 20.0);
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
    let base = doc.add_in(
        body,
        "base",
        rect_at(SketchPlane::xy(), 0.0, 0.0, 20.0, 20.0),
    );
    let pad_id = doc.add_in(body, "Pad", pad(base, 10.0));
    doc.mark_feature_dirty(pad_id);
    settle(&registry, &mut doc, body);
    let top = picked_face(&doc, body, [10.0, 10.0, 10.0], [0.0, 0.0, 1.0]);
    let cube = DesignFeature::Primitive {
        keep_inside: false,
        kind: kernel_api::PrimitiveKind::Box {
            length: 4.0,
            width: 4.0,
            height: 4.0,
        },
        placement: kernel_api::Placement::default(),
        subtractive: false,
        refine: false,
        boolean_tolerance: None,
        attached: Some(Box::new(wb_design::Attached {
            attachment: core_document::DatumAttachment::Face {
                face: core_document::attach::face_anchor(&top, true),
            },
            offset: AttachmentOffset::default(),
        })),
    };
    let cube_id = doc.add_in(body, "Cube", cube);
    doc.mark_feature_dirty(cube_id);
    let (_, hi) = settle(&registry, &mut doc, body);
    assert!((hi[2] - 14.0).abs() < 1e-3, "on the top: {hi:?}");

    set_pad_length(&mut doc, pad_id, 20.0);
    let (_, hi) = settle(&registry, &mut doc, body);
    assert!((hi[2] - 24.0).abs() < 1e-3, "went up with the top: {hi:?}");
}

/// A plane midway between a pad's top and bottom, both found by name,
/// stays halfway up when the pad grows.
#[test]
fn a_midplane_stays_halfway_between_the_faces_it_was_made_from() {
    let registry = registry();
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let base = doc.add_in(
        body,
        "base",
        rect_at(SketchPlane::xy(), 0.0, 0.0, 20.0, 20.0),
    );
    let pad_id = doc.add_in(body, "Pad", pad(base, 10.0));
    doc.mark_feature_dirty(pad_id);
    settle(&registry, &mut doc, body);

    // The top and the bottom, as clicks pick them.
    let face_at = |up: f32, height: f32| {
        let face = picked_face(&doc, body, [5.0, 15.0, height], [0.0, 0.0, up]);
        assert_ne!(face.name, 0, "the face is named");
        core_document::PlaneAnchor::Face {
            face: FaceAnchor {
                name: face.name,
                point: [5.0, 15.0, height],
                normal: [0.0, 0.0, up],
                surface: None,
                follows: true,
            },
        }
    };
    let middle = doc.add_in(
        body,
        "Middle",
        datum(
            DatumShape::Plane { size: 20.0 },
            DatumAttachment::Midplane {
                planes: [face_at(1.0, 10.0), face_at(-1.0, 0.0)],
            },
        ),
    );
    settle(&registry, &mut doc, body);
    let frame = frame_of(&doc, middle);
    assert!(near(frame.origin, [5.0, 15.0, 5.0]), "{frame:?}");
    assert!(near(frame.normal, [0.0, 0.0, 1.0]), "{frame:?}");

    set_pad_length(&mut doc, pad_id, 20.0);
    settle(&registry, &mut doc, body);
    let frame = frame_of(&doc, middle);
    assert!(
        (frame.origin[2] - 10.0).abs() < 1e-3,
        "halfway up the taller pad: {frame:?}"
    );
    assert!(near(frame.normal, [0.0, 0.0, 1.0]), "{frame:?}");
    assert!(
        doc.get_feature_meta(middle).unwrap().error.is_none(),
        "both faces were found"
    );
}
