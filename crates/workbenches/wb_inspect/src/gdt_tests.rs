use super::*;
use core_document::{FeatureId, ImportedGeometry, MarkedSurface, Workbench};
use std::sync::Arc;
use tolerance::Characteristic as C;

#[test]
fn scanned_hole_position_bonus_and_offset_runout_reach_the_report() {
    let mut document = Document::new("Scanned plate");
    let scan = document.create_body(Some("Plate".into()));
    let mut mesh = kernel_api::TriMesh::default();
    for z in [0.0, 5.0, 10.0] {
        for i in 0..64 {
            let a = i as f64 * std::f64::consts::TAU / 64.0;
            mesh.positions
                .push([(0.1 + 5.1 * a.cos()) as f32, (5.1 * a.sin()) as f32, z]);
            mesh.normals.push([-a.cos() as f32, -a.sin() as f32, 0.0]);
        }
    }
    for ring in 0..2 {
        for i in 0..64 {
            let a = ring * 64 + i;
            let b = ring * 64 + (i + 1) % 64;
            mesh.indices.extend([a, b, a + 64, b, b + 64, a + 64]);
        }
    }
    let mut regions = vec![("cylinder", 0..192)];
    for (k, normal) in [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        .into_iter()
        .enumerate()
    {
        let start = mesh.positions.len();
        for a in [-10.0, 10.0] {
            for b in [-10.0, 10.0] {
                mesh.positions.push(match k {
                    0 => [a, b, 0.0],
                    1 => [0.0, a, b],
                    _ => [a, 0.0, b],
                });
                mesh.normals.push(normal);
            }
        }
        regions.push(("plane", start..start + 4));
    }
    let count = mesh.positions.len();
    document.set_imported_geometry(
        scan,
        ImportedGeometry {
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
    ctx.kernel = Some(&kernel_ogeom::QUERIES);
    let mut ids = Vec::new();
    let revision = ctx.document.imported_geometry(scan).unwrap().revision;
    for (kind, region) in regions {
        ctx.marking = Some(MarkedSurface {
            body: scan,
            revision,
            vertices: Arc::new((0..count).map(|i| region.contains(&i)).collect()),
        });
        let args = serde_json::from_value(json!({"kind": kind})).unwrap();
        let answer = bench.run_command(FIT_COMMAND, &args, &mut ctx).unwrap();
        ids.push(answer["element"].clone());
    }
    let mut args: CommandArgs = serde_json::from_value(json!({
        "characteristic": "position", "element": ids[0], "limit": 0.1,
        "datums": [{"element": ids[1]}, {"element": ids[2]}, {"element": ids[3]}],
        "basic": [0.0,0.0,0.0]
    }))
    .unwrap();
    let plain = bench.run_command(TOLERANCE, &args, &mut ctx).unwrap();
    assert!(
        (plain["value"].as_f64().unwrap() - 0.2).abs() < 1e-5,
        "{plain}"
    );
    assert_eq!(plain["pass"], false);
    args.insert("material".into(), json!("mmc"));
    args.insert("material_size".into(), json!(10.0));
    let bonus = bench.run_command(TOLERANCE, &args, &mut ctx).unwrap();
    assert!((bonus["bonus"].as_f64().unwrap() - 0.2).abs() < 1e-5);
    assert_eq!(bonus["pass"], true);
    let mut datum = element_at(
        ctx.document,
        FeatureId(ids[0].as_str().unwrap().parse().unwrap()),
    )
    .unwrap();
    if let kernel_api::FittedShape::Cylinder { point, .. } = &mut datum.result.shape {
        point[0] = 0.05;
    }
    let datum = ctx
        .document
        .add_feature_in_body(datum, "Runout axis".into(), Some(scan))
        .unwrap();
    for kind in ["circular_runout", "total_runout"] {
        let args = serde_json::from_value(json!({"characteristic":kind,"element":ids[0],"datum":datum,"limit":0.11,"section_width":0.1})).unwrap();
        let read = bench.run_command(TOLERANCE, &args, &mut ctx).unwrap();
        assert!(
            (read["value"].as_f64().unwrap() - 0.1).abs() < 1e-5,
            "{read}"
        );
        assert_eq!(read["pass"], true);
    }
    let report = bench
        .run_command(REPORT_COMMAND, &CommandArgs::new(), &mut ctx)
        .unwrap();
    assert_eq!(
        (report["passed"].clone(), report["failed"].clone()),
        (json!(3), json!(1))
    );
    assert!(report::text(&report, "Plate").contains("bonus 0.2000"));
    // The plain one's task panel takes the same material and size.
    let plain_id = FeatureId(plain["tolerance"].as_str().unwrap().parse().unwrap());
    bench.edit_feature(&mut ctx, plain_id);
    let reads = |bench: &InspectionWorkbench, ctx: &WorkbenchRuntimeContext| {
        let [bench_api::Widget::Stack(stack)] = &bench.task_widgets(ctx)[..] else {
            panic!("one stack")
        };
        let texts = stack.children.iter().filter_map(|w| match w {
            bench_api::Widget::Text { text, .. } => Some(text.as_str()),
            _ => None,
        });
        texts.collect::<Vec<_>>().join("\n")
    };
    assert!(
        reads(&bench, &ctx).contains("FAIL"),
        "{}",
        reads(&bench, &ctx)
    );
    for event in [
        bench_api::PanelEvent::Choice {
            id: "material".into(),
            index: 1,
        },
        bench_api::PanelEvent::Toggle {
            id: "material_size:on".into(),
            on: true,
        },
    ] {
        bench.task_event(&mut ctx, &event);
    }
    let edited =
        tolerance::Tolerance::from_json(ctx.document.get_feature_data(plain_id).unwrap()).unwrap();
    assert_eq!(
        (edited.material, edited.material_size),
        (Some(tolerance::Material::Mmc), Some(10.0))
    );
    assert!(
        reads(&bench, &ctx).contains("PASS"),
        "{}",
        reads(&bench, &ctx)
    );
    bench.task_event(
        &mut ctx,
        &bench_api::PanelEvent::Choice {
            id: "datum:2".into(),
            index: 0,
        },
    );
    let edited =
        tolerance::Tolerance::from_json(ctx.document.get_feature_data(plain_id).unwrap()).unwrap();
    assert_eq!(edited.references().len(), 2, "the tertiary datum taken out");
    let angularity = C::ALL.iter().position(|c| *c == C::Angularity).unwrap();
    for event in [
        bench_api::PanelEvent::Choice {
            id: "characteristic".into(),
            index: angularity,
        },
        bench_api::PanelEvent::Toggle {
            id: "angle:on".into(),
            on: true,
        },
    ] {
        bench.task_event(&mut ctx, &event);
    }
    let edited =
        tolerance::Tolerance::from_json(ctx.document.get_feature_data(plain_id).unwrap()).unwrap();
    assert_eq!(
        (edited.characteristic, edited.angle_deg),
        (C::Angularity, Some(90.0))
    );
    let old = json!({"name":"old","scan":scan,"characteristic":"flatness","element":ids[1],"datum":null,"limit":0.1});
    assert!(
        tolerance::Tolerance::from_json(&old)
            .unwrap()
            .datums
            .is_empty()
    );
}
