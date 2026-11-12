use super::*;
use core_document::{Document, WorkbenchInputEvent, WorkbenchRuntimeContext};

/// The box primitive at the origin, added to `body`.
fn add_box(doc: &mut Document, body: BodyId) -> FeatureId {
    let cube = DesignFeature::Primitive {
        keep_inside: false,
        attached: None,
        refine: false,
        boolean_tolerance: None,
        kind: primitive_preset("box").unwrap(),
        placement: kernel_api::Placement::default(),
        subtractive: false,
    };
    doc.add_feature_in_body(cube, "Box".into(), Some(body))
        .unwrap()
}

#[test]
fn the_body_tool_creates_a_body_and_asks_the_host_to_select_and_label_it() {
    let mut wb = DesignWorkbench::default();
    let mut doc = Document::new("t");
    let mut ctx = core_document::testing::ctx(&mut doc);
    let result = wb.on_input(
        &WorkbenchInputEvent::ToolActivated,
        Some("design.new_body"),
        &mut ctx,
    );
    assert!(result.consumed);
    let requests = ctx.take_requests();
    assert_eq!(doc.bodies().len(), 1);
    let body = doc.bodies()[0].id;
    assert_eq!(
        requests,
        vec![
            HostRequest::SelectBody(body),
            HostRequest::JournalLabel("Create body".to_string()),
        ]
    );
}
/// The Mirror tool with a face picked: a flat face is the plane it
/// starts with, a curved one leaves it on YZ.
#[test]
fn a_picked_flat_face_is_the_plane_a_new_mirror_starts_with() {
    let mirror_with = |surface: Option<kernel_api::FaceSurface>| {
        let mut wb = DesignWorkbench::default();
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let pad = add_box(&mut doc, body);
        let mut ctx = core_document::testing::ctx(&mut doc);
        ctx.selected_body_id = Some(body.0);
        ctx.active_document_object = Some(pad);
        ctx.selected_face = Some(core_document::FaceRef {
            name: 0,
            point: [10.0, 2.5, 4.0],
            normal: [1.0, 0.0, 0.0],
            surface,
        });
        wb.on_input(
            &WorkbenchInputEvent::ToolActivated,
            Some("design.mirror"),
            &mut ctx,
        );
        let mirrored = doc
            .feature_tree()
            .all_nodes()
            .find_map(|(_, n)| match DesignFeature::from_json(&n.data).ok()? {
                DesignFeature::Mirrored {
                    plane, originals, ..
                } => Some((plane, originals)),
                _ => None,
            })
            .expect("the tool made a mirror");
        assert_eq!(
            mirrored.1,
            vec![pad],
            "the selected feature is the original"
        );
        mirrored.0
    };
    let flat = kernel_api::FaceSurface::Plane {
        origin: [10.0, 0.0, 0.0],
        normal: [1.0, 0.0, 0.0],
    };
    assert_eq!(
        mirror_with(Some(flat)),
        MirrorPlane::Face(FacePick {
            name: 0,
            point: [10.0, 2.5, 4.0],
            normal: [1.0, 0.0, 0.0],
        })
    );
    let round = kernel_api::FaceSurface::Cylinder {
        origin: [0.0; 3],
        axis: [0.0, 0.0, 1.0],
        radius: 5.0,
    };
    assert_eq!(mirror_with(Some(round)), MirrorPlane::YZ);
}

/// Pad and Pocket take a flat face of the solid as their profile: with
/// one picked they are there to click, with a curved one or nothing
/// they are not, and the pad made has the face for its profile.
#[test]
fn a_picked_flat_face_is_a_pad_s_or_pocket_s_profile() {
    let mut wb = DesignWorkbench::default();
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    add_box(&mut doc, body);
    let top = |surface| core_document::FaceRef {
        name: 0,
        point: [5.0, 5.0, 10.0],
        normal: [0.0, 0.0, 1.0],
        surface,
    };
    let flat = kernel_api::FaceSurface::Plane {
        origin: [0.0, 0.0, 10.0],
        normal: [0.0, 0.0, 1.0],
    };
    let round = kernel_api::FaceSurface::Cylinder {
        origin: [0.0; 3],
        axis: [0.0, 0.0, 1.0],
        radius: 5.0,
    };
    let mut ctx = core_document::testing::ctx(&mut doc);
    ctx.selected_body_id = Some(body.0);
    let tools = |wb: &DesignWorkbench, ctx: &WorkbenchRuntimeContext| {
        (
            wb.is_tool_enabled("design.pad", ctx),
            wb.is_tool_enabled("design.pocket", ctx),
        )
    };
    assert_eq!(tools(&wb, &ctx), (false, false), "nothing picked");
    ctx.selected_face = Some(top(Some(round)));
    assert_eq!(
        tools(&wb, &ctx),
        (false, false),
        "a curved face has no plane"
    );
    ctx.selected_face = Some(top(Some(flat)));
    assert_eq!(tools(&wb, &ctx), (true, true), "a flat face is a profile");

    wb.on_input(
        &WorkbenchInputEvent::ToolActivated,
        Some("design.pad"),
        &mut ctx,
    );
    let profile = doc
        .feature_tree()
        .all_nodes()
        .find_map(|(_, n)| match DesignFeature::from_json(&n.data).ok()? {
            DesignFeature::Pad {
                sketch,
                profile_face,
                ..
            } => Some((sketch, profile_face)),
            _ => None,
        })
        .expect("the tool made a pad");
    assert_eq!(profile.0, None, "no sketch");
    assert_eq!(profile.1.map(|f| f.point), Some([5.0, 5.0, 10.0]));
}

#[test]
fn the_coordinate_system_tool_places_a_frame_on_the_body() {
    let mut wb = DesignWorkbench::default();
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    let mut ctx = core_document::testing::ctx(&mut doc);
    ctx.selected_body_id = Some(body.0);
    assert!(wb.is_tool_enabled("design.coordinate_system", &ctx));
    wb.on_input(
        &WorkbenchInputEvent::ToolActivated,
        Some("design.coordinate_system"),
        &mut ctx,
    );
    let datums = core_document::datums_of_body(&doc, body);
    assert_eq!(datums.len(), 1);
    assert!(matches!(
        datums[0].2.shape,
        core_document::DatumShape::CoordinateSystem { .. }
    ));
    assert!(!datum_mesh(&datums[0].2).edges.is_empty());
}

/// A click near a drawn datum's lines finds it; one far from them, or
/// on a datum of a body the view does not draw the datums of, does not.
#[test]
fn a_click_finds_a_drawn_datum_by_its_lines() {
    let mut wb = DesignWorkbench::default();
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    let mut ctx = core_document::testing::ctx(&mut doc);
    ctx.selected_body_id = Some(body.0);
    wb.on_input(
        &WorkbenchInputEvent::ToolActivated,
        Some("design.coordinate_system"),
        &mut ctx,
    );
    let (id, _, datum) = core_document::datums_of_body(ctx.document, body).remove(0);
    let core_document::DatumShape::CoordinateSystem { size } = datum.shape else {
        panic!("{datum:?}");
    };
    // World X and Y from -2 size to 2 size across a 400 px square.
    let scale = 1.0 / (2.0 * size);
    let pick = |x: f32, y: f32| core_document::ViewportPick {
        view_proj: glam::Mat4::from_scale(glam::Vec3::splat(scale)).to_cols_array_2d(),
        viewport: (0, 0, 400, 400),
        cursor: (x, y),
    };
    let node = ctx.document.get_feature_meta(id).unwrap().clone();
    let distance =
        |wb: &DesignWorkbench, x, y| wb.pick_feature(ctx.document, id, &node, &pick(x, y));
    assert_eq!(distance(&wb, 250.0, 200.0), None, "not drawn yet");
    wb.on_frame(0.0, &mut ctx);
    let node = ctx.document.get_feature_meta(id).unwrap().clone();
    let distance = |x, y| {
        wb.pick_feature(ctx.document, id, &node, &pick(x, y))
            .unwrap()
    };
    // Halfway along its X axis, and 3 px off it.
    assert!(distance(250.0, 200.0) < 1e-3);
    assert!((distance(250.0, 203.0) - 3.0).abs() < 1e-3);
    assert!(distance(30.0, 370.0) > 100.0, "far from every line");
}
