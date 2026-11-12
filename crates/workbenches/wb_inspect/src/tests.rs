use super::*;
use crate::construction_export::construction_of;
use crate::registration::tool_id;
use core_document::Workbench;
use std::sync::Arc;

/// A kernel that fits every request as a unit sphere and keeps all
/// points but the last.
struct UnitSphere;

impl kernel_api::KernelQueries for UnitSphere {
    fn project_edge(
        &self,
        _brep: &[u8],
        _near: [f64; 3],
        _plane: &kernel_api::ProfilePlane,
    ) -> kernel_api::KernelResult<kernel_api::ProjectedEdge> {
        Err(kernel_api::KernelError::Unsupported("project_edge".into()))
    }

    /// A 2 mm square, outlined by a circle of radius 2.
    fn mesh_shadow(
        &self,
        _mesh: &kernel_api::TriMesh,
        _request: &kernel_api::ShadowRequest,
    ) -> kernel_api::KernelResult<kernel_api::ShadowAnswer> {
        Ok(kernel_api::ShadowAnswer {
            silhouette: vec![vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]],
            outline: vec![kernel_api::ProfileWire::new(vec![
                kernel_api::ProfileSegment::Circle {
                    center: [0.0; 2],
                    radius: 2.0,
                },
            ])],
        })
    }

    /// A 2 mm square about the axis at each offset.
    fn mesh_sections(
        &self,
        _mesh: &kernel_api::TriMesh,
        origin: [f64; 3],
        _normal: [f64; 3],
        offsets: &[f64],
    ) -> kernel_api::KernelResult<Vec<Vec<kernel_api::SectionChain>>> {
        Ok(offsets
            .iter()
            .map(|z| {
                let z = origin[2] + z;
                vec![kernel_api::SectionChain {
                    points: vec![
                        [-1.0, -1.0, z],
                        [1.0, -1.0, z],
                        [1.0, 1.0, z],
                        [-1.0, 1.0, z],
                    ],
                    outward: vec![[0.0; 3]; 4],
                    closed: true,
                }]
            })
            .collect())
    }

    fn fit(&self, request: &FitRequest) -> kernel_api::KernelResult<kernel_api::FitAnswer> {
        let n = request.points.len();
        Ok(kernel_api::FitAnswer {
            shape: kernel_api::FittedShape::Sphere {
                centre: [0.0; 3],
                radius: 1.0,
            },
            sigma: 0.01,
            form: (-0.02, 0.03),
            used: (0..n).map(|i| i + 1 < n).collect(),
            residuals: vec![0.0; n],
            converged: true,
        })
    }
}

/// Every tool names an icon the set has. A fit takes the marked part of
/// a scan, keeps it as an element under the scan with the vertices it
/// rests on and what the kernel made of them, records itself as a
/// command, and the report lists it; with nothing marked the tool says
/// so and makes nothing. A dimension on it passes its band and goes
/// when the element is deleted. Datums square the scan, and picked
/// point pairs bring it where a reference has it.
#[test]
fn a_fit_of_the_marked_surface_becomes_an_element() {
    let mut context = WorkbenchContext::default();
    InspectionWorkbench::default().configure(&mut context);
    assert_eq!(
        context.tools().len(),
        FitKind::ALL.len() + DimKind::ALL.len() + Made::ALL.len() + 19
    );
    for tool in context.tools() {
        let icon = tool.icon.expect("an icon");
        assert!(ui_kit::icon::exists(icon), "{icon}");
    }
    assert!(ui_kit::icon::exists("measure"));

    let mut document = Document::new("Scan");
    let scan = document.create_body(Some("Scan".into()));
    let mesh = kernel_api::TriMesh {
        positions: vec![
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [-1.0, 0.0, 0.0],
        ],
        normals: vec![
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [-1.0, 0.0, 0.0],
        ],
        indices: vec![0, 1, 2],
        ..Default::default()
    };
    document.set_imported_geometry(
        scan,
        core_document::ImportedGeometry {
            mesh: Arc::new(mesh),
            source_asset: None,
            revision: 0,
            bounds_mm: None,
            brep_blob_path: None,
            mesh_path: None,
            face_colors_path: None,
            health: None,
        },
    );
    let mut bench = InspectionWorkbench::default();
    let mut ctx = core_document::testing::ctx(&mut document);
    ctx.kernel = Some(&UnitSphere);
    let tool = tool_id(FitKind::Sphere);
    assert!(!bench.is_tool_enabled(&tool, &ctx));
    bench.on_input(&WorkbenchInputEvent::ToolActivated, Some(&tool), &mut ctx);
    let outcome = core_document::HookOutcome::take(&mut ctx);
    assert!(outcome.recorded.is_empty(), "nothing marked, nothing made");

    ctx.marking = Some(core_document::MarkedSurface {
        body: scan,
        revision: 0,
        vertices: Arc::new(vec![true, true, false, true]),
    });
    // A marking of a mesh the scan no longer has fits nothing: the
    // tool arms instead.
    ctx.marking.as_mut().unwrap().revision += 1;
    bench.tab.armed = None;
    assert!(!bench.is_tool_enabled(&tool, &ctx));
    bench.on_input(&WorkbenchInputEvent::ToolActivated, Some(&tool), &mut ctx);
    assert!(
        core_document::HookOutcome::take(&mut ctx)
            .recorded
            .is_empty()
    );
    assert!(bench.tab.armed.take().is_some());
    ctx.marking.as_mut().unwrap().revision -= 1;
    assert!(bench.is_tool_enabled(&tool, &ctx));
    bench.on_input(&WorkbenchInputEvent::ToolActivated, Some(&tool), &mut ctx);
    assert_eq!(
        core_document::HookOutcome::take(&mut ctx).recorded[0].id,
        FIT_COMMAND
    );
    let report = bench
        .run_command(REPORT_COMMAND, &CommandArgs::new(), &mut ctx)
        .unwrap();
    let first = &report["elements"][0];
    assert_eq!(first["name"], "Sphere 1");
    assert_eq!(
        (first["points"].clone(), first["used"].clone()),
        (json!(3), json!(2))
    );
    assert!((first["form"].as_f64().unwrap() - 0.05).abs() < 1e-12);
    let id = core_document::FeatureId(
        uuid::Uuid::parse_str(first["element"].as_str().unwrap()).unwrap(),
    );
    let data = ctx.document.feature_values(id).unwrap();
    let element = Element::from_json(data).unwrap();
    assert_eq!(
        (element.scan, element.region.clone()),
        (scan, vec![[0, 2], [3, 4]])
    );
    assert_eq!(element.result.lines().edges.len() / 2, 3 * 48);

    let mut args = CommandArgs::new();
    args.insert("kind".into(), json!("diameter"));
    args.insert("elements".into(), json!([first["element"]]));
    args.insert("plus".into(), json!(0.1));
    let dim = bench
        .run_command(DIMENSION_COMMAND, &args, &mut ctx)
        .unwrap();
    assert_eq!(
        (dim["value"].clone(), dim["pass"].clone()),
        (json!(2.0), json!(true))
    );
    let report = bench
        .run_command(REPORT_COMMAND, &CommandArgs::new(), &mut ctx)
        .unwrap();
    assert_eq!(
        (report["passed"].clone(), report["failed"].clone()),
        (json!(1), json!(0))
    );
    assert!(bench.delete_feature(&mut ctx, id));
    let report = bench
        .run_command(REPORT_COMMAND, &CommandArgs::new(), &mut ctx)
        .unwrap();
    assert_eq!(report["dimensions"], json!([]), "gone with its element");

    let asset = ctx.document.add_asset(core_document::AssetReference::new(
        "assets/scan.stl",
        core_document::AssetType::Stl,
        json!({}),
    ));
    let mut geometry = ctx.document.imported_geometry(scan).unwrap().clone();
    geometry.source_asset = Some(asset);
    ctx.document.set_imported_geometry(scan, geometry);
    assert!(ctx.document.set_body_scan(scan, Some(Default::default())));
    let before = align::by_numbers([5.0, 8.0, 3.0], [10.0, 20.0, 30.0]);
    ctx.document.set_body_placement(scan, before);
    let mut ids = Vec::new();
    for normal in [[0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        let mut datum = element.clone();
        datum.kind = FitKind::Plane;
        datum.result.shape = kernel_api::FittedShape::Plane {
            origin: [1.0, 2.0, 3.0],
            normal,
        };
        ids.push(
            ctx.document
                .add_feature_in_body(datum, "Datum".into(), Some(scan))
                .unwrap(),
        );
    }
    ctx.selected_features = ids.clone();
    assert!(bench.is_tool_enabled(align_commands::DATUMS, &ctx));
    bench.on_input(
        &WorkbenchInputEvent::ToolActivated,
        Some(align_commands::DATUMS),
        &mut ctx,
    );
    let outcome = core_document::HookOutcome::take(&mut ctx);
    assert_eq!(outcome.recorded.last().unwrap().id, align_commands::DATUMS);
    let (_, node) = ctx
        .document
        .feature_tree()
        .all_nodes()
        .find(|(_, n)| n.workbench_id.as_str() == align::KIND)
        .unwrap();
    let alignment = align::Alignment::from_json(&node.data).unwrap();
    assert_eq!(alignment.before, before);
    assert_eq!(alignment.after.point([1.0, 2.0, 3.0]), [0.0; 3]);
    assert!(
        matches!(alignment.how, align::How::Datums { primary, secondary, zero, .. }
        if [primary, secondary, zero] == ids.as_slice())
    );
    ctx.selected_features.swap(0, 1);
    assert!(bench.is_tool_enabled(align_commands::DATUMS, &ctx));
    ctx.selected_features[1] = ctx.selected_features[0];
    assert!(!bench.is_tool_enabled(align_commands::DATUMS, &ctx));

    // Points picked on the scan where it stands and where they are on
    // a reference bring it exactly where the reference has it.
    let now = ctx.document.body_placement(scan);
    let target = align::by_numbers([-4.0, 2.0, 7.0], [15.0, -30.0, 60.0]);
    let picked = [
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 0.0],
        [0.0, 5.0, 2.0],
        [3.0, 3.0, 8.0],
    ];
    let pairs: Vec<Value> = picked
        .iter()
        .map(|p| json!({"scan": now.point(*p), "reference": target.point(*p)}))
        .collect();
    let mut args = CommandArgs::new();
    args.insert("scan".into(), json!(scan.0.to_string()));
    args.insert("pairs".into(), json!(pairs));
    let answer = bench
        .run_command(align_commands::POINTS, &args, &mut ctx)
        .unwrap();
    assert!(answer["rms"].as_f64().unwrap() < 1e-4);
    let placed = ctx.document.body_placement(scan);
    for p in picked {
        let (a, b) = (placed.point(p), target.point(p));
        assert!((0..3).all(|k| (a[k] - b[k]).abs() < 1e-3), "{a:?} {b:?}");
    }
    assert_eq!(answer["name"], "Alignment 2");

    // Another scan's alignment is named apart from the first scan's.
    let other = ctx.document.create_body(Some("Other".into()));
    let geometry = ctx.document.imported_geometry(scan).unwrap().clone();
    ctx.document.set_imported_geometry(other, geometry);
    assert!(ctx.document.set_body_scan(other, Some(Default::default())));
    let mut args = CommandArgs::new();
    args.insert("scan".into(), json!(other.0.to_string()));
    let answer = bench.run_command(MOVE_COMMAND, &args, &mut ctx).unwrap();
    assert_eq!(answer["name"], "Alignment 3");
}

/// A stack of sections across a scan is kept in the scan's frame, reads
/// each cut's loops, length and area, draws every chain, and cuts again
/// when its count changes, and offers its first cut to sketch on. A
/// shadow reads its area and clearance, its
/// outline is derived once per change of what it is made from, and its
/// body's plan stands the outline up the scan's height; a STEP export
/// holds them all, a cylinder as its axis and rims.
#[test]
fn sections_and_shadows_read_and_draw_the_scan() {
    let mut document = Document::new("Scan");
    let scan = document.create_body(Some("Scan".into()));
    document.set_imported_geometry(
        scan,
        core_document::ImportedGeometry {
            mesh: Arc::new(kernel_api::TriMesh {
                positions: vec![[-1.0, -1.0, 0.0], [1.0, 1.0, 10.0], [1.0, -1.0, 0.0]],
                indices: vec![0, 1, 2],
                ..Default::default()
            }),
            source_asset: None,
            revision: 0,
            bounds_mm: None,
            brep_blob_path: None,
            mesh_path: None,
            face_colors_path: None,
            health: None,
        },
    );
    let mut bench = InspectionWorkbench::default();
    let mut ctx = core_document::testing::ctx(&mut document);
    ctx.kernel = Some(&UnitSphere);
    let mut args = CommandArgs::new();
    args.insert("scan".into(), json!(scan.0.to_string()));
    args.insert("offset".into(), json!(-4.0));
    args.insert("count".into(), json!(3));
    args.insert("step".into(), json!(4.0));
    let answer = bench.run_command(SECTION, &args, &mut ctx).unwrap();
    let cuts = answer["cuts"].as_array().unwrap();
    assert_eq!(cuts.len(), 3);
    for (cut, offset) in cuts.iter().zip([-4.0, 0.0, 4.0]) {
        assert_eq!(cut["offset"], json!(offset));
        assert_eq!(
            (cut["chains"].clone(), cut["closed"].clone()),
            (json!(1), json!(1))
        );
        assert!((cut["length"].as_f64().unwrap() - 8.0).abs() < 1e-12);
        assert!((cut["area"].as_f64().unwrap() - 4.0).abs() < 1e-12);
    }
    let id = core_document::FeatureId(answer["section"].as_str().unwrap().parse().unwrap());
    let (_, node) = document
        .feature_tree()
        .all_nodes()
        .find(|(f, _)| **f == id)
        .unwrap();
    let drawn = bench
        .passive_geometry(&document, id, &node.clone())
        .unwrap();
    assert_eq!(drawn.mesh.edges.len(), 3 * 4 * 2);
    assert_eq!(
        drawn.mesh.positions[0][2], 1.0,
        "the middle of the scan, less 4"
    );
    let offered = bench.feature_plane(&document, id, &node.clone()).unwrap();
    assert_eq!(
        (offered.origin[2], offered.normal),
        (1.0, [0.0, 0.0, 1.0]),
        "its first cut"
    );

    let mut args = CommandArgs::new();
    args.insert("scan".into(), json!(scan.0.to_string()));
    let mut ctx = core_document::testing::ctx(&mut document);
    ctx.kernel = Some(&UnitSphere);
    let shadow = bench.run_command(SHADOW, &args, &mut ctx).unwrap();
    assert_eq!(
        (shadow["loops"].clone(), shadow["area"].clone()),
        (json!(1), json!(4.0))
    );
    let band = shadow["band"].as_array().unwrap();
    assert!((band[0].as_f64().unwrap() - (2.0 - 2f64.sqrt())).abs() < 0.01);
    assert!((band[1].as_f64().unwrap() - 1.0).abs() < 0.01);
    assert_eq!(
        shadow["depth"],
        json!(10.0),
        "the scan's height above its foot"
    );
    let block = BodyId(shadow["body"].as_str().unwrap().parse().unwrap());
    let sid = core_document::FeatureId(shadow["shadow"].as_str().unwrap().parse().unwrap());
    let (_, node) = document
        .feature_tree()
        .all_nodes()
        .find(|(f, _)| **f == sid)
        .unwrap();
    let (node, mut values) = (node.clone(), node.data.clone());
    assert!(bench.derive_on_geometry(&node, &mut values, &document, Some(&UnitSphere)));
    assert!(!bench.derive_on_geometry(&node, &mut values, &document, Some(&UnitSphere)));
    let made = shadow::Shadow::from_json(&values).unwrap();
    match shadow::ops(&made).as_slice() {
        [
            kernel_api::SolidOp::Sweep {
                profile,
                kind: kernel_api::SweepKind::Extrude { termination, .. },
                ..
            },
        ] => {
            assert_eq!(profile.wires.len(), 1);
            assert_eq!(
                *termination,
                kernel_api::ExtrudeTermination::Blind { distance: 10.0 }
            );
            assert_eq!(profile.plane.normal, [0.0, 0.0, 1.0]);
        }
        other => panic!("{other:?}"),
    }
    // What a STEP export of the inspection holds: every cut's chain
    // (the shadow's outline joins once evaluation works it out).
    let items = bench.construction_items(&document, None);
    let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Section 1 -4.00 #1",
            "Section 1 +0.00 #1",
            "Section 1 +4.00 #1"
        ]
    );
    let e = Element {
        method: Default::default(),
        name: "Cylinder 1".into(),
        scan,
        kind: FitKind::Cylinder,
        cutoff: None,
        region: Vec::new(),
        revision: 0,
        result: Fitted {
            zone: None,
            shape: kernel_api::FittedShape::Cylinder {
                point: [0.0; 3],
                axis: [0.0, 0.0, 2.0],
                radius: 5.0,
            },
            sigma: 0.0,
            form: (0.0, 0.0),
            used: 0,
            points: 0,
            span: element::Span::Axis([1.0, 9.0]),
        },
    };
    let shapes = construction_of(&e);
    assert_eq!(shapes.len(), 3, "its axis and two rims");
    assert!(matches!(
        shapes[2].1,
        kernel_api::ConstructionShape::Circle {
            centre: [0.0, 0.0, 9.0],
            radius: 5.0,
            ..
        }
    ));
    bench.invalidate_body(&mut document, block);
    let jobs = bench.rebuild_jobs(&mut document);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].body, block);

    let mut data = document.feature_values(id).unwrap().clone();
    data["count"] = json!(1);
    document.update_feature_data(id, data).unwrap();
    let (_, node) = document
        .feature_tree()
        .all_nodes()
        .find(|(f, _)| **f == id)
        .unwrap();
    let drawn = bench
        .passive_geometry(&document, id, &node.clone())
        .unwrap();
    assert_eq!(drawn.mesh.edges.len(), 4 * 2);
}

/// Each click of an element in progress is pinned and numbered 1..n,
/// the same size on screen at any zoom: an armed fit's Shift-clicks
/// until Escape puts them away, an alignment's points by pair (the
/// reference's beside the scan's) until Enter makes it.
#[test]
fn clicks_in_progress_are_pinned_and_numbered_until_made() {
    let mut document = Document::new("Pins");
    let mesh = |dx: f32| kernel_api::TriMesh {
        positions: vec![[dx, 0.0, 0.0], [dx + 1.0, 0.0, 0.0], [dx, 1.0, 0.0]],
        indices: vec![0, 1, 2],
        ..Default::default()
    };
    let mut body = |name: &str, dx: f32| {
        let id = document.create_body(Some(name.into()));
        document.set_imported_geometry(
            id,
            core_document::ImportedGeometry {
                mesh: Arc::new(mesh(dx)),
                source_asset: None,
                revision: 0,
                bounds_mm: None,
                brep_blob_path: None,
                mesh_path: None,
                face_colors_path: None,
                health: None,
            },
        );
        id
    };
    let (scan, reference) = (body("Scan", 0.0), body("Reference", 5.0));
    let asset = document.add_asset(core_document::AssetReference::new(
        "assets/scan.stl",
        core_document::AssetType::Stl,
        json!({}),
    ));
    let mut geometry = document.imported_geometry(scan).unwrap().clone();
    geometry.source_asset = Some(asset);
    document.set_imported_geometry(scan, geometry);
    assert!(document.set_body_scan(scan, Some(Default::default())));
    let mut bench = InspectionWorkbench::default();
    let mut ctx = WorkbenchRuntimeContext::new(&mut document, [0.0; 3], [0.0; 3], (0, 0, 400, 400));
    ctx.pixels_per_point = 2.0;
    let zoom = |k: f32| {
        [
            [k, 0.0, 0.0, 0.0],
            [0.0, k, 0.0, 0.0],
            [0.0, 0.0, 0.1, 0.0],
            [0.0, 0.0, 0.5, 1.0],
        ]
    };
    ctx.view_proj = Some(zoom(0.05));
    let shown = |bench: &InspectionWorkbench, ctx: &WorkbenchRuntimeContext| {
        let numbers: Vec<String> = bench
            .screen_labels(ctx, None)
            .into_iter()
            .map(|l| l.text)
            .collect();
        let radii: Vec<f32> = bench
            .screen_marks(ctx, None)
            .into_iter()
            .map(|m| match m.kind {
                core_document::MarkKind::Dot { radius } => radius,
                _ => 0.0,
            })
            .collect();
        (numbers, radii)
    };
    let click = |bench: &mut InspectionWorkbench,
                 ctx: &mut WorkbenchRuntimeContext,
                 body: BodyId,
                 p: [f32; 3]| {
        ctx.hovered_body_id = Some(body.0);
        ctx.hovered_face = Some(core_document::FaceRef {
            point: p,
            normal: [0.0, 0.0, 1.0],
            surface: None,
            name: 0,
        });
        let press = WorkbenchInputEvent::MousePress {
            button: core_document::MouseButton::Left,
            viewport_pos: (0.0, 0.0),
        };
        assert!(bench.on_input(&press, None, ctx).consumed);
    };
    let key = |bench: &mut InspectionWorkbench, ctx: &mut WorkbenchRuntimeContext, key| {
        bench.on_input(&WorkbenchInputEvent::KeyPress { key }, None, ctx);
    };

    let plane = tool_id(FitKind::Plane);
    bench.on_input(&WorkbenchInputEvent::ToolActivated, Some(&plane), &mut ctx);
    ctx.shift_down = true;
    for x in [0.1, 0.4, 0.2] {
        click(&mut bench, &mut ctx, scan, [x, 0.2, 0.0]);
    }
    let (numbers, radii) = shown(&bench, &ctx);
    assert_eq!(numbers, ["1", "2", "3"]);
    ctx.view_proj = Some(zoom(4.0));
    assert_eq!(shown(&bench, &ctx).1, radii, "the same size zoomed in");
    assert!(radii.iter().all(|r| *r == radii[0] && *r > 0.0));
    key(&mut bench, &mut ctx, core_document::KeyCode::Escape);
    assert!(shown(&bench, &ctx).0.is_empty(), "Escape puts them away");

    ctx.shift_down = false;
    ctx.view_proj = Some(zoom(0.05));
    let tool = align_commands::POINTS;
    assert!(bench.is_tool_enabled(tool, &ctx));
    bench.on_input(&WorkbenchInputEvent::ToolActivated, Some(tool), &mut ctx);
    for (i, p) in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        .into_iter()
        .enumerate()
    {
        if i == 1 {
            // Both bodies move between the clicks (an undo, another
            // client): the pins go with them. They park with their
            // tab.
            let place = |t| align::by_numbers(t, [0.0; 3]);
            ctx.document
                .set_body_placement(scan, place([0.0, 3.0, 0.0]));
            ctx.document
                .set_body_placement(reference, place([0.0, 0.0, 1.0]));
            let parked = bench.suspend_session();
            bench.resume_session(None);
            assert!(shown(&bench, &ctx).0.is_empty() && !bench.tab.picks.aligning);
            bench.resume_session(parked);
            assert_eq!(shown(&bench, &ctx).0, ["1", "1"]);
        }
        let at = |body| ctx.document.body_placement(body);
        let (on_scan, on_reference) = (
            at(scan).point(p),
            at(reference).point([p[0] + 5.0, p[1], p[2]]),
        );
        click(&mut bench, &mut ctx, scan, on_scan);
        click(&mut bench, &mut ctx, reference, on_reference);
    }
    assert_eq!(shown(&bench, &ctx).0, ["1", "2", "3", "1", "2", "3"]);
    core_document::HookOutcome::take(&mut ctx);
    key(&mut bench, &mut ctx, core_document::KeyCode::Enter);
    let outcome = core_document::HookOutcome::take(&mut ctx);
    assert_eq!(outcome.recorded.len(), 1, "the alignment is made");
    assert!(shown(&bench, &ctx).0.is_empty());
    let moved = ctx.document.body_placement(scan).point([0.0, 0.0, 0.0]);
    let want = [5.0, 0.0, 1.0];
    assert!(
        (0..3).all(|k| (moved[k] - want[k]).abs() < 1e-3),
        "{moved:?}"
    );
}
