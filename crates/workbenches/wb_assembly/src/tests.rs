use super::*;
use crate::test_support::{activate, context, imported, joint};
use core_document::{Document, FaceRef, TriMesh};
use kernel_api::FaceSurface;
use serde_json::json;

/// Two bodies, each a flat square facing up at its own height.
pub(crate) fn scene() -> (Document, BodyId, BodyId) {
    let mut doc = Document::new("t");
    let base = doc.create_body(Some("Base".into()));
    let part = doc.create_body(Some("Part".into()));
    for body in [base, part] {
        let square = TriMesh {
            positions: vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
            ..TriMesh::default()
        };
        doc.set_imported_geometry(body, imported(square, None));
    }
    doc.set_body_placement(
        part,
        BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(30.0, 0.0, 40.0)),
    );
    (doc, base, part)
}

pub(crate) fn face_up(z: f32) -> FaceRef {
    FaceRef {
        name: 0,
        point: [2.0, 2.0, z],
        normal: [0.0, 0.0, 1.0],
        surface: Some(FaceSurface::Plane {
            origin: [0.0, 0.0, z],
            normal: [0.0, 0.0, 1.0],
        }),
    }
}

pub(crate) fn frame(
    wb: &mut AssemblyWorkbench,
    doc: &mut Document,
    pick: Option<(BodyId, FaceRef)>,
) {
    let mut ctx = context(doc);
    ctx.selected_body_id = pick.map(|(b, _)| b.0);
    ctx.selected_face = pick.map(|(_, f)| f);
    wb.on_frame(0.016, &mut ctx);
}

/// One frame of the task panel, as the host runs it; what it recorded.
pub(crate) fn task_frame(
    wb: &mut AssemblyWorkbench,
    doc: &mut Document,
    request: core_document::TaskRequest,
) -> Vec<core_document::Recorded> {
    use bench_panel::BenchUi;
    let egui_ctx = egui::Context::default();
    ui_kit::apply_theme(&egui_ctx);
    let mut recorded = Vec::new();
    let mut output = egui_ctx.run_ui(egui::RawInput::default(), |ui| {
        let mut ctx = context(doc);
        wb.ui_task_panel(ui, &mut ctx, request);
        recorded = core_document::HookOutcome::take(&mut ctx).recorded;
    });
    output.textures_delta.clear();
    recorded
}

/// A view from `eye` toward `target`, Y up, 60° high, flipped upright
/// as the host's is.
pub(crate) fn flipped_view_proj(eye: glam::Vec3, target: glam::Vec3) -> [[f32; 4]; 4] {
    use glam::{Mat4, Vec3};
    let proj = glam::camera::rh::proj::directx::perspective(
        60f32.to_radians(),
        800.0 / 600.0,
        0.1,
        1000.0,
    );
    let view = glam::camera::rh::view::look_at_mat4(eye, target, Vec3::Y);
    (Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0)) * proj * view).to_cols_array_2d()
}

/// Frames run while `working` holds, as the host runs them while the
/// bench's work is away from the window; at most 5 s.
fn until_done(
    wb: &mut AssemblyWorkbench,
    ctx: &mut WorkbenchRuntimeContext<'_>,
    working: impl Fn(&AssemblyWorkbench) -> bool,
) {
    let started = std::time::Instant::now();
    while working(wb) {
        assert!(started.elapsed().as_secs() < 5, "the work finishes");
        std::thread::sleep(std::time::Duration::from_millis(5));
        wb.on_frame(0.016, ctx);
    }
}

/// A joint shows under the body it holds to as well as its own; the
/// ground it made does not.
#[test]
fn a_joint_is_linked_to_the_body_it_holds_to() {
    let (mut doc, base, part) = scene();
    let mut wb = AssemblyWorkbench::default();
    let mut ctx = context(&mut doc);
    let anchor = Anchor::Plane {
        point: [0.0; 3],
        normal: [0.0, 0.0, 1.0],
    };
    wb.make_joint(
        &mut ctx,
        part,
        joint(
            JointKind::Mate {
                flip: false,
                offset: 0.0,
            },
            anchor,
            base,
            anchor,
        ),
    );
    drop(ctx);
    let mate = joints(&doc)
        .into_iter()
        .find(|j| j.feature.kind != JointKind::Ground)
        .unwrap();
    assert_eq!(wb.linked_features(&doc, base), [mate.id]);
    assert!(wb.linked_features(&doc, part).is_empty());
}

/// A selected joint draws its two ends, their directions and the link,
/// and names itself between them; nothing is drawn with none selected.
#[test]
fn a_selected_joint_is_drawn_in_the_view() {
    let (mut doc, base, part) = scene();
    let mut wb = AssemblyWorkbench::default();
    let mut ctx = context(&mut doc);
    let eye = glam::Vec3::new(60.0, -80.0, 90.0);
    let view = glam::camera::rh::view::look_at_mat4(eye, glam::Vec3::ZERO, glam::Vec3::Z);
    let proj = glam::camera::rh::proj::directx::perspective(0.8, 800.0 / 600.0, 0.1, 1000.0);
    ctx.view_proj = Some((proj * view).to_cols_array_2d());
    let anchor = Anchor::Plane {
        point: [2.0, 2.0, 0.0],
        normal: [0.0, 0.0, 1.0],
    };
    wb.make_joint(
        &mut ctx,
        part,
        joint(
            JointKind::Mate {
                flip: false,
                offset: 0.0,
            },
            anchor,
            base,
            anchor,
        ),
    );
    let [(lines, dots, name)] = wb
        .joint_drawings(&ctx)
        .try_into()
        .expect("the new joint is drawn");
    assert_eq!(dots.len(), 2);
    assert_eq!(
        lines.len(),
        11,
        "a normal and a square at each end, and the link"
    );
    assert_eq!(name, "Mate 1");
    wb.tab.task = None;
    ctx.active_document_object = None;
    assert!(wb.joint_drawings(&ctx).is_empty());
}

/// A body's menus in the tree and the view offer to select its joints
/// when it has any; the entry selects them together and the view draws
/// each one selected.
#[test]
fn a_body_s_menus_select_its_joints_and_the_view_draws_them_all() {
    let (mut doc, base, part) = scene();
    let mut wb = AssemblyWorkbench::default();
    for scope in [
        core_document::MenuScope::TreeBody(part),
        core_document::MenuScope::ViewportBody(part),
    ] {
        assert!(
            !wb.menu_items(&scope, &doc)
                .iter()
                .any(|i| i.id == "asm.menu.select_joints"),
            "no joints yet"
        );
    }
    let anchor = Anchor::Plane {
        point: [2.0, 2.0, 0.0],
        normal: [0.0, 0.0, 1.0],
    };
    let to_base = |kind| joint(kind, anchor, base, anchor);
    let mate = doc
        .add_feature_in_body(
            to_base(JointKind::Mate {
                flip: false,
                offset: 0.0,
            }),
            "Mate 1".into(),
            Some(part),
        )
        .unwrap();
    let parallel = doc
        .add_feature_in_body(
            to_base(JointKind::Parallel),
            "Parallel 1".into(),
            Some(part),
        )
        .unwrap();
    for scope in [
        core_document::MenuScope::TreeBody(base),
        core_document::MenuScope::ViewportBody(base),
    ] {
        let items = wb.menu_items(&scope, &doc);
        let item = items
            .iter()
            .find(|i| i.id == "asm.menu.select_joints")
            .expect("offered");
        assert_eq!(item.label, "Select its 2 joints");
        let mut ctx = context(&mut doc);
        assert!(wb.on_command("asm.menu.select_joints", &scope, &mut ctx));
        let requests = core_document::HookOutcome::take(&mut ctx).requests;
        assert_eq!(
            requests,
            [HostRequest::SelectFeatures(vec![mate, parallel])]
        );
    }

    let mut ctx = context(&mut doc);
    let eye = glam::Vec3::new(60.0, -80.0, 90.0);
    let view = glam::camera::rh::view::look_at_mat4(eye, glam::Vec3::ZERO, glam::Vec3::Z);
    let proj = glam::camera::rh::proj::directx::perspective(0.8, 800.0 / 600.0, 0.1, 1000.0);
    ctx.view_proj = Some((proj * view).to_cols_array_2d());
    ctx.active_document_object = Some(mate);
    ctx.selected_features = vec![mate, parallel];
    let names: Vec<String> = wb
        .joint_drawings(&ctx)
        .into_iter()
        .map(|(_, _, name)| name)
        .collect();
    assert_eq!(names, ["Mate 1", "Parallel 1"]);
}

#[test]
fn the_couple_tool_ties_two_hinges_records_as_the_command_and_follows_a_ratio() {
    let (mut doc, base, part) = scene();
    let third = doc.create_body(Some("Third".into()));
    let run = |doc: &mut Document, id: &str, args: serde_json::Value| {
        core_document::testing::call_checked(&mut AssemblyWorkbench::default(), doc, id, args)
            .unwrap()
    };
    run(&mut doc, "asm.ground", json!({"body": base.0.to_string()}));
    let axis = |x: f32| json!({"axis": {"point": [x, 0.0, 0.0], "direction": [0.0, 0.0, 1.0]}});
    let mut hinges = Vec::new();
    for (body, x) in [(part, 0.0), (third, 30.0)] {
        let placed = doc.body_placement(body).point([0.0; 3]);
        let id = run(
            &mut doc,
            "asm.hinge",
            json!({
                "body": body.0.to_string(),
                "face": {"axis": {"point": placed, "direction": [0.0, 0.0, 1.0]}},
                "other": base.0.to_string(),
                "other_face": axis(x),
            }),
        );
        hinges.push(FeatureId(
            uuid::Uuid::parse_str(id.as_str().unwrap()).unwrap(),
        ));
    }
    let before = doc.clone();

    let mut wb = AssemblyWorkbench::default();
    {
        let mut ctx = context(&mut doc);
        assert!(wb.is_tool_enabled("asm.couple", &ctx));
        activate(&mut wb, &mut ctx, "asm.couple");
    }
    let Some(Task::Coupling { id, .. }) = wb.tab.task.clone() else {
        panic!("the tool opens the coupling's settings: {:?}", wb.tab.task);
    };
    let coupling = Coupling::from_json(doc.get_feature_data(id).unwrap()).unwrap();
    assert_eq!(coupling.gearing, Gearing::Gears, "gears suit two hinges");
    let recorded = task_frame(
        &mut wb,
        &mut doc,
        core_document::TaskRequest {
            accept: true,
            cancel: false,
        },
    );
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(recorded[0].id, "asm.couple");
    let mut replay = before;
    let made = run(
        &mut replay,
        &recorded[0].id,
        recorded[0].args.clone().into(),
    );
    let made = FeatureId(uuid::Uuid::parse_str(made.as_str().unwrap()).unwrap());
    let again = Coupling::from_json(replay.get_feature_data(made).unwrap()).unwrap();
    assert_eq!(
        (again.gearing, again.driver, again.driven, again.ratio),
        (
            coupling.gearing,
            coupling.driver,
            coupling.driven,
            coupling.ratio
        )
    );

    run(
        &mut doc,
        "asm.set",
        json!({"joint": id.0.to_string(), "ratio": 0.5}),
    );
    run(
        &mut doc,
        "asm.set",
        json!({"joint": coupling.driver.0.to_string(), "drive": 60.0}),
    );
    let travel = |doc: &Document, joint: FeatureId| {
        let j = joints(doc).into_iter().find(|j| j.id == joint).unwrap();
        let at = |b: BodyId| -> Rigid { doc.body_placement(b).into() };
        j.feature
            .travel(&at(j.body), &at(j.feature.other_body))
            .unwrap()
    };
    let driven = travel(&doc, coupling.driven);
    assert!(
        (driven + 30.0).abs() < 1e-2,
        "half the turn, the other way: {driven}"
    );
    assert!(hinges.contains(&coupling.driver) && hinges.contains(&coupling.driven));
}

/// Linked copies go in a row beside the body, count as the same part,
/// and one with no joints is dragged straight across the view.
#[test]
fn copies_go_in_a_row_and_a_free_one_is_dragged() {
    let (mut doc, base, _) = scene();
    doc.set_imported_brep_data(base, b"ogeom base".to_vec(), Vec::new());
    let made = {
        let mut ctx = context(&mut doc);
        let args = serde_json::json!({"body": base.0.to_string(), "count": 2});
        commands::run("asm.copy", args.as_object().unwrap(), &mut ctx).unwrap()
    };
    let made: Vec<BodyId> = made
        .as_array()
        .unwrap()
        .iter()
        .map(|v| BodyId(uuid::Uuid::parse_str(v.as_str().unwrap()).unwrap()))
        .collect();
    assert_eq!(made.len(), 2);
    let x = |b: BodyId, doc: &Document| doc.body_placement(b).translation[0];
    assert!(
        (x(made[0], &doc) - 11.0).abs() < 1e-3,
        "a width and a tenth on"
    );
    assert!((x(made[1], &doc) - 22.0).abs() < 1e-3);
    let parts = parts_list(&doc, &[]);
    assert!(
        parts.iter().any(|p| p.bodies.len() == 3),
        "one part, three of it"
    );

    let copy = made[0];
    let vp = flipped_view_proj(glam::Vec3::new(0.0, 0.0, 100.0), glam::Vec3::ZERO);
    let screen =
        |p: [f32; 3]| core_document::runtime::world_to_viewport(vp, (0, 0, 800, 600), p).unwrap();
    let mut wb = AssemblyWorkbench::default();
    let mut send = |doc: &mut Document, event: WorkbenchInputEvent| {
        let mut ctx =
            WorkbenchRuntimeContext::new(doc, [0.0, 0.0, 100.0], [0.0; 3], (0, 0, 800, 600));
        ctx.view_proj = Some(vp);
        ctx.hovered_body_id = Some(copy.0);
        ctx.hovered_world_pos = Some([12.0, 2.0, 0.0]);
        wb.on_input(&event, None, &mut ctx);
        core_document::HookOutcome::take(&mut ctx).recorded
    };
    send(
        &mut doc,
        WorkbenchInputEvent::MousePress {
            button: core_document::MouseButton::Left,
            viewport_pos: screen([12.0, 2.0, 0.0]),
        },
    );
    send(
        &mut doc,
        WorkbenchInputEvent::MouseMove {
            viewport_pos: screen([32.0, 12.0, 0.0]),
        },
    );
    let recorded = send(
        &mut doc,
        WorkbenchInputEvent::MouseRelease {
            button: core_document::MouseButton::Left,
            viewport_pos: screen([32.0, 12.0, 0.0]),
        },
    );
    let t = doc.body_placement(copy).translation;
    assert!(
        (t[0] - 31.0).abs() < 1e-2 && (t[1] - 10.0).abs() < 1e-2,
        "{t:?}"
    );
    assert_eq!(recorded.len(), 1);
}

/// A kernel for which every pair shares a little.
struct AlwaysShared;

impl kernel_api::KernelQueries for AlwaysShared {
    fn project_edge(
        &self,
        _: &[u8],
        _: [f64; 3],
        _: &kernel_api::ProfilePlane,
    ) -> kernel_api::KernelResult<kernel_api::ProjectedEdge> {
        Err(kernel_api::KernelError::Unsupported("projection".into()))
    }

    fn overlap(
        &self,
        _: &[u8],
        _: &[u8],
        _: &[[f64; 4]; 4],
    ) -> kernel_api::KernelResult<Option<kernel_api::Overlap>> {
        Ok(Some(kernel_api::Overlap {
            volume_mm3: 2.0,
            centre_mm: [1.0, 1.0, 0.0],
            mesh: TriMesh {
                positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                normals: vec![[0.0, 0.0, 1.0]; 3],
                indices: vec![0, 1, 2],
                ..TriMesh::default()
            },
        }))
    }
}

static ALWAYS_SHARED: AlwaysShared = AlwaysShared;

/// A kernel whose every solid is 1000 mm³ about its own (1, 2, 3).
struct Litre;

impl kernel_api::KernelQueries for Litre {
    fn project_edge(
        &self,
        _: &[u8],
        _: [f64; 3],
        _: &kernel_api::ProfilePlane,
    ) -> kernel_api::KernelResult<kernel_api::ProjectedEdge> {
        Err(kernel_api::KernelError::Unsupported("projection".into()))
    }

    fn measure(&self, _: &[u8]) -> kernel_api::KernelResult<kernel_api::PhysicalProperties> {
        Ok(kernel_api::PhysicalProperties {
            volume_mm3: Some(1000.0),
            area_mm2: 600.0,
            centre_mm: [1.0, 2.0, 3.0],
            approximate: false,
            inertia: None,
        })
    }
}

static LITRE: Litre = Litre;

/// Mark `part` bought in the parts list, as `asm.part` does.
fn doc_with_bought_part(ctx: &mut WorkbenchRuntimeContext, part: BodyId) {
    let args = serde_json::json!({"body": part.0.to_string(), "bought": true});
    commands::run("asm.part", args.as_object().unwrap(), ctx).unwrap();
}

/// Two bodies weighed: the centre between their placed centres, the
/// mass at the density asked; the tool shows the same away from the
/// window.
#[test]
fn the_mass_tool_weighs_the_bodies_where_they_sit() {
    let (mut doc, base, part) = scene();
    // Two shapes, so the parts list counts two parts.
    doc.set_imported_brep_data(base, b"base".to_vec(), Vec::new());
    doc.set_imported_brep_data(part, b"part".to_vec(), Vec::new());
    let mut ctx = context(&mut doc);
    ctx.kernel = Some(&LITRE);
    let args = serde_json::json!({"density": 2.0});
    let got = commands::run("asm.mass", args.as_object().unwrap(), &mut ctx).unwrap();
    assert!((got["mass"].as_f64().unwrap() - 4.0).abs() < 1e-9, "{got}");
    // The part sits at (30, 0, 40): the centres are (1, 2, 3) and
    // (31, 2, 43).
    let centre: Vec<f64> = serde_json::from_value(got["centre"].clone()).unwrap();
    assert!((centre[0] - 16.0).abs() < 1e-4 && (centre[2] - 23.0).abs() < 1e-4);

    // A bought part weighs in unless left out.
    doc_with_bought_part(&mut ctx, part);
    let args = serde_json::json!({"density": 2.0});
    let got = commands::run("asm.mass", args.as_object().unwrap(), &mut ctx).unwrap();
    assert!((got["mass"].as_f64().unwrap() - 4.0).abs() < 1e-9, "{got}");
    let bought: Vec<bool> = got["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["bought"].as_bool().unwrap())
        .collect();
    assert_eq!(bought.iter().filter(|b| **b).count(), 1, "{got}");
    let args = serde_json::json!({"density": 2.0, "bought": false});
    let got = commands::run("asm.mass", args.as_object().unwrap(), &mut ctx).unwrap();
    assert!((got["mass"].as_f64().unwrap() - 2.0).abs() < 1e-9, "{got}");
    let centre: Vec<f64> = serde_json::from_value(got["centre"].clone()).unwrap();
    assert!((centre[0] - 1.0).abs() < 1e-4, "the base alone: {got}");

    let mut wb = AssemblyWorkbench::default();
    activate(&mut wb, &mut ctx, "asm.mass");
    assert!(wb.busy());
    until_done(&mut wb, &mut ctx, |wb| wb.tab.measuring.is_some());
    assert!(!wb.busy());
    let Some(Task::Mass {
        found: Some(report),
        ..
    }) = &wb.tab.task
    else {
        panic!("the report is shown")
    };
    assert_eq!(report.bodies.len(), 2);
}

#[test]
fn an_interference_check_runs_away_from_the_window_and_draws_what_is_shared() {
    let (mut doc, base, part) = scene();
    for body in [base, part] {
        doc.set_imported_brep_data(body, b"shape".to_vec(), Vec::new());
    }
    doc.set_body_placement(part, BodyPlacement::default());
    let mut wb = AssemblyWorkbench::default();
    let mut ctx = context(&mut doc);
    ctx.kernel = Some(&ALWAYS_SHARED);
    activate(&mut wb, &mut ctx, "asm.interference");
    assert!(matches!(
        wb.tab.task,
        Some(Task::Interference { found: None, .. })
    ));
    until_done(&mut wb, &mut ctx, |wb| wb.tab.checking.is_some());
    let Some(Task::Interference {
        found: Some(found), ..
    }) = &wb.tab.task
    else {
        panic!("the answer is shown")
    };
    assert_eq!(found.clashes.len(), 1);
    let drawn = wb.overlay_meshes(&ctx, None);
    assert_eq!(drawn.len(), 1);
    assert!(drawn[0].on_top && drawn[0].opacity < 1.0);
    wb.finish_editing(&mut ctx);
    assert!(wb.overlay_meshes(&ctx, None).is_empty());
}

/// With a body selected, the check asks only about the pairs it is in.
#[test]
fn a_selected_body_is_checked_against_the_others() {
    let (mut doc, base, part) = scene();
    let third = doc.create_body(Some("Third".into()));
    let geometry = doc.imported_geometry(base).cloned().unwrap();
    doc.set_imported_geometry(third, geometry);
    for body in [base, part, third] {
        doc.set_imported_brep_data(body, b"shape".to_vec(), Vec::new());
    }
    doc.set_body_placement(part, BodyPlacement::default());
    assert_eq!(interference::plan(&doc, None).pairs(), 3);
    assert_eq!(interference::plan(&doc, None).around(part).pairs(), 2);
    let mut wb = AssemblyWorkbench::default();
    let mut ctx = context(&mut doc);
    ctx.kernel = Some(&ALWAYS_SHARED);
    ctx.selected_body_id = Some(part.0);
    activate(&mut wb, &mut ctx, "asm.interference");
    assert!(matches!(
        wb.tab.task,
        Some(Task::Interference { around: Some(b), .. }) if b == part
    ));
    assert_eq!(wb.interference_progress().map(|(_, total)| total), Some(2));
}

/// A kernel for 10 mm cubes that are never turned: what two share is
/// where their boxes overlap.
struct Cubes;

impl kernel_api::KernelQueries for Cubes {
    fn project_edge(
        &self,
        _: &[u8],
        _: [f64; 3],
        _: &kernel_api::ProfilePlane,
    ) -> kernel_api::KernelResult<kernel_api::ProjectedEdge> {
        Err(kernel_api::KernelError::Unsupported("projection".into()))
    }

    fn overlap(
        &self,
        _: &[u8],
        _: &[u8],
        b_in_a: &[[f64; 4]; 4],
    ) -> kernel_api::KernelResult<Option<kernel_api::Overlap>> {
        let span = |k: usize| (10.0 - b_in_a[k][3].abs()).max(0.0);
        let volume = span(0) * span(1) * span(2);
        Ok((volume > 1e-6).then(|| kernel_api::Overlap {
            volume_mm3: volume,
            centre_mm: [0.0; 3],
            mesh: TriMesh::default(),
        }))
    }
}

static CUBES: Cubes = Cubes;

/// Two 10 mm cubes, the second 20 mm along X on a slider along X: the
/// document, the second cube and the slider.
fn cubes_on_a_slider() -> (Document, BodyId, FeatureId) {
    let mut doc = Document::new("t");
    let [base, part] = [doc.create_body(None), doc.create_body(None)];
    for body in [base, part] {
        let cube = TriMesh {
            positions: vec![[0.0; 3], [10.0, 0.0, 0.0], [0.0, 10.0, 10.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
            ..TriMesh::default()
        };
        doc.set_imported_geometry(body, imported(cube, Some(([0.0; 3], [10.0; 3]))));
        doc.set_imported_brep_data(body, b"cube".to_vec(), Vec::new());
    }
    doc.set_body_placement(
        part,
        BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(20.0, 0.0, 0.0)),
    );
    let rail = Anchor::Axis {
        point: [0.0, 5.0, 5.0],
        direction: [1.0, 0.0, 0.0],
    };
    let at = |b: BodyId| Rigid::from(doc.body_placement(b));
    let kind = JointTool::Slider.joint(&rail, &at(part), &rail, &at(base), 0.0);
    let slider = doc
        .add_feature_in_body(joint(kind, rail, base, rail), "Slider 1".into(), Some(part))
        .unwrap();
    (doc, part, slider)
}

/// A slider swept from where the cube stands (20 mm off) down to
/// nothing runs into the other cube for the steps under 10 mm.
#[test]
fn a_motion_is_checked_for_collisions_step_by_step() {
    let (mut doc, part, slider) = cubes_on_a_slider();
    let check = sweep_check::plan(&doc, slider, 20.0, 0.0, 5).expect("the part moves");
    let found = check
        .run(
            &CUBES,
            &std::sync::atomic::AtomicUsize::new(0),
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
    let at: Vec<f32> = found.iter().map(|c| c.at).collect();
    assert_eq!(at, [5.0, 0.0], "{found:?}");
    assert!((found[0].volume_mm3 - 500.0).abs() < 1e-6);
    assert_eq!(
        doc.body_placement(part).translation[0],
        20.0,
        "the document is not moved"
    );

    let mut wb = AssemblyWorkbench::default();
    let mut ctx = context(&mut doc);
    ctx.kernel = Some(&CUBES);
    wb.check_sweep(&mut ctx, slider, (20.0, 0.0));
    until_done(&mut wb, &mut ctx, |wb| wb.tab.sweeping.is_some());
    let Some((joint, Ok(found))) = &wb.tab.motion_clashes else {
        panic!("the answer is kept")
    };
    assert_eq!(*joint, slider);
    assert!(!found.is_empty());
}

/// Two 10 mm cubes, the second on a slider along X, 20 mm along; the
/// second dragged from its middle to `to`, and where it ends up.
fn slide_cube_to(to: [f32; 3], collisions_off: bool) -> f32 {
    let (mut doc, part, _) = cubes_on_a_slider();
    let vp = flipped_view_proj(
        glam::Vec3::new(10.0, 5.0, 100.0),
        glam::Vec3::new(10.0, 5.0, 0.0),
    );
    let screen =
        |p: [f32; 3]| core_document::runtime::world_to_viewport(vp, (0, 0, 800, 600), p).unwrap();
    let mut wb = AssemblyWorkbench::default();
    wb.tab.collisions_off = collisions_off;
    let grab = [25.0, 5.0, 10.0];
    let mut send = |doc: &mut Document, event: WorkbenchInputEvent| {
        let mut ctx = WorkbenchRuntimeContext::new(
            doc,
            [10.0, 5.0, 100.0],
            [10.0, 5.0, 0.0],
            (0, 0, 800, 600),
        );
        ctx.view_proj = Some(vp);
        ctx.kernel = Some(&CUBES);
        ctx.hovered_body_id = Some(part.0);
        ctx.hovered_world_pos = Some(grab);
        wb.on_input(&event, None, &mut ctx);
    };
    send(
        &mut doc,
        WorkbenchInputEvent::MousePress {
            button: core_document::MouseButton::Left,
            viewport_pos: screen(grab),
        },
    );
    for step in 1..=8 {
        let t = step as f32 / 8.0;
        let p = glam::Vec3::from_array(grab).lerp(glam::Vec3::from_array(to), t);
        send(
            &mut doc,
            WorkbenchInputEvent::MouseMove {
                viewport_pos: screen(p.to_array()),
            },
        );
    }
    send(
        &mut doc,
        WorkbenchInputEvent::MouseRelease {
            button: core_document::MouseButton::Left,
            viewport_pos: screen(to),
        },
    );
    doc.body_placement(part).translation[0]
}

#[test]
fn a_dragged_body_stops_where_it_would_collide() {
    let stopped = slide_cube_to([-5.0, 5.0, 10.0], false);
    assert!(
        (10.0..10.5).contains(&stopped),
        "against the other cube's face: {stopped}"
    );
    let through = slide_cube_to([-5.0, 5.0, 10.0], true);
    assert!(
        through < 0.0,
        "with collisions off it goes through: {through}"
    );
    let clear = slide_cube_to([40.0, 5.0, 10.0], false);
    assert!(clear > 30.0, "away from it nothing stops it: {clear}");
}

/// [`Cubes`], counting what it is asked.
struct Counted(std::sync::atomic::AtomicUsize);

impl kernel_api::KernelQueries for Counted {
    fn project_edge(
        &self,
        _: &[u8],
        _: [f64; 3],
        _: &kernel_api::ProfilePlane,
    ) -> kernel_api::KernelResult<kernel_api::ProjectedEdge> {
        Err(kernel_api::KernelError::Unsupported("projection".into()))
    }

    fn overlap(
        &self,
        a: &[u8],
        b: &[u8],
        b_in_a: &[[f64; 4]; 4],
    ) -> kernel_api::KernelResult<Option<kernel_api::Overlap>> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        CUBES.overlap(a, b, b_in_a)
    }
}

/// A cube pushed into one neighbour while it touches another: once
/// the first has stopped a move, the next move it stops is answered
/// by one boolean, the touching pair left unasked.
#[test]
fn a_collision_asks_the_pair_that_last_collided_first() {
    let mut doc = Document::new("t");
    let at = |x: f32, y: f32| BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(x, y, 0.0));
    let [_touching, _wall, moved] = [(12.0, 10.0), (20.0, 0.0), (10.0, 0.0)].map(|(x, y)| {
        let body = doc.create_body(None);
        doc.set_imported_geometry(
            body,
            imported(TriMesh::default(), Some(([0.0; 3], [10.0; 3]))),
        );
        doc.set_imported_brep_data(body, b"cube".to_vec(), Vec::new());
        doc.set_body_placement(body, at(x, y));
        body
    });
    let kernel = Counted(std::sync::atomic::AtomicUsize::new(0));
    let mut baseline = collide::Baseline::new(&[(moved, at(10.0, 0.0))]);
    for (x, asked) in [(12.0, 3), (12.5, 1)] {
        kernel.0.store(0, std::sync::atomic::Ordering::Relaxed);
        let hit = collide::collides(&doc, &kernel, &[(moved, at(x, 0.0))], &mut baseline);
        assert_eq!(hit, Ok(true), "{x} mm along it is in the wall");
        assert_eq!(
            kernel.0.load(std::sync::atomic::Ordering::Relaxed),
            asked,
            "at {x} mm"
        );
    }
}

#[test]
fn a_sweep_is_recorded_frame_by_frame_without_touching_the_document() {
    let mut doc = Document::new("t");
    let frame = doc.create_body(None);
    let door = doc.create_body(None);
    let pin = Anchor::Axis {
        point: [0.0; 3],
        direction: [0.0, 0.0, 1.0],
    };
    let at = Rigid::from(BodyPlacement::default());
    let hinge = doc
        .add_feature_in_body(
            joint(
                JointTool::Hinge.joint(&pin, &at, &pin, &at, 0.0),
                pin,
                frame,
                pin,
            ),
            "Hinge 1".into(),
            Some(door),
        )
        .unwrap();
    let seq = doc.mutation_seq();
    let frames = sweep_frames(&doc, hinge, -90.0, 90.0, 8);
    assert_eq!(frames.len(), 8);
    let angle = |frame: &Vec<(BodyId, BodyPlacement)>| {
        let (_, placed) = frame.iter().find(|(b, _)| *b == door).unwrap();
        let x = placed.direction([1.0, 0.0, 0.0]);
        x[1].atan2(x[0]).to_degrees()
    };
    assert!(
        (angle(&frames[0]) + 90.0).abs() < 0.1,
        "{}",
        angle(&frames[0])
    );
    assert!(
        (angle(&frames[4]) - 90.0).abs() < 0.1,
        "{}",
        angle(&frames[4])
    );
    assert_eq!(doc.mutation_seq(), seq, "the document is left alone");
    assert!(doc.body_placement(door).is_identity());
}

#[test]
fn an_exploded_view_spreads_the_bodies_and_puts_them_back() {
    let (mut doc, base, part) = scene();
    let before = [base, part].map(|b| doc.body_placement(b));
    let mut wb = AssemblyWorkbench::default();
    let mut ctx = context(&mut doc);
    activate(&mut wb, &mut ctx, "asm.explode");
    let apart = |ctx: &WorkbenchRuntimeContext| {
        let [a, b] = [base, part]
            .map(|body| glam::Vec3::from_array(ctx.document.body_placement(body).translation));
        a.distance(b)
    };
    let was = glam::Vec3::from_array(before[0].translation)
        .distance(glam::Vec3::from_array(before[1].translation));
    assert!((apart(&ctx) - 2.0 * was).abs() < 1e-3, "twice as far apart");
    wb.finish_editing(&mut ctx);
    assert_eq!([base, part].map(|b| ctx.document.body_placement(b)), before);
}
