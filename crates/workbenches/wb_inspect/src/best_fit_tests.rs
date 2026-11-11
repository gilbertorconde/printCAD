use super::*;
use core_document::Workbench;
use core_document::history::OpJournal;
use std::sync::Arc;

/// A scan and a reference beside it, both with a triangle.
fn scene() -> (Document, BodyId, BodyId) {
    let mut document = Document::new("Fit");
    let mut body = |name: &str| {
        let id = document.create_body(Some(name.into()));
        let mesh = kernel_api::TriMesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        document.set_imported_geometry(
            id,
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
        id
    };
    let (scan, reference) = (body("Scan"), body("Reference"));
    let asset = document.add_asset(core_document::AssetReference::new(
        "assets/scan.stl",
        core_document::AssetType::Stl,
        serde_json::json!({}),
    ));
    let mut geometry = document.imported_geometry(scan).unwrap().clone();
    geometry.source_asset = Some(asset);
    document.set_imported_geometry(scan, geometry);
    assert!(document.set_body_scan(scan, Some(Default::default())));
    (document, scan, reference)
}

fn answer(dx: f64) -> best_fit::Answer {
    let fit = kernel_api::Registration {
        transform: [
            [1.0, 0.0, 0.0, dx],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
        rms: 0.01,
        inliers: 3,
        sampled: 3,
        converged: true,
        free: Vec::new(),
    };
    Ok((align::by_numbers([dx, 0.0, 0.0], [0.0; 3]), fit))
}

fn alignments(document: &Document) -> usize {
    document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == align::KIND)
        .count()
}

/// A best fit lands only while the scan and reference are as it fitted
/// them, in the document that asked, unless stopped or superseded; once
/// landed, the Alignment and the move are one undo step.
#[test]
fn a_best_fit_lands_only_on_the_inputs_and_tab_it_was_asked_for() {
    let frame = |bench: &mut InspectionWorkbench, document: &mut Document| {
        let mut ctx = WorkbenchRuntimeContext::new(document, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        bench.on_frame(0.0, &mut ctx);
        core_document::HookOutcome::take(&mut ctx).recorded.len()
    };
    let moved = align::by_numbers([0.0, 3.0, 0.0], [0.0; 3]);

    // Completion with nothing changed: one Alignment, the scan placed,
    // the command recorded; a scan moved or given new geometry while it
    // ran drops the answer.
    let placed = align::by_numbers([2.0, 0.0, 0.0], [0.0; 3]);
    for edit in 0..3 {
        let (mut document, scan, reference) = scene();
        let mut bench = InspectionWorkbench::default();
        let slot = bench.tab.fitting.begin(&document, scan, reference, true);
        assert_eq!(frame(&mut bench, &mut document), 0, "still running");
        match edit {
            1 => document.set_body_placement(scan, moved),
            2 => {
                let mut geometry = document.imported_geometry(scan).unwrap().clone();
                geometry.mesh = Arc::new((*geometry.mesh).clone());
                document.set_imported_geometry(scan, geometry);
            }
            _ => {}
        }
        let at = document.body_placement(scan);
        slot.fill(answer(2.0));
        let recorded = frame(&mut bench, &mut document);
        assert!(!bench.busy());
        let want = usize::from(edit == 0);
        assert_eq!((recorded, alignments(&document)), (want, want), "{edit}");
        let at = if edit == 0 { placed } else { at };
        assert_eq!(document.body_placement(scan), at);
    }

    // An undo back to the inputs it fitted lets it land; the landing is
    // one step, so an undo takes both the Alignment and the move.
    let (mut document, scan, reference) = scene();
    let mut journal = OpJournal::new(16);
    journal.note(&mut document);
    let mut bench = InspectionWorkbench::default();
    let slot = bench.tab.fitting.begin(&document, scan, reference, true);
    document.set_body_placement(scan, moved);
    journal.note(&mut document);
    journal.undo(&mut document);
    slot.fill(answer(2.0));
    assert_eq!(frame(&mut bench, &mut document), 1);
    journal.note(&mut document);
    journal.undo(&mut document);
    assert_eq!(alignments(&document), 0);
    assert!(document.body_placement(scan).is_identity());
    journal.redo(&mut document);
    assert_eq!(alignments(&document), 1);

    // Escape and the tool's second click stop it; a later answer is lost.
    for stop in [
        (
            WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Escape,
            },
            None,
        ),
        (WorkbenchInputEvent::ToolActivated, Some(BEST_FIT)),
    ] {
        let (mut document, scan, reference) = scene();
        let mut bench = InspectionWorkbench::default();
        let slot = bench.tab.fitting.begin(&document, scan, reference, true);
        let mut ctx = WorkbenchRuntimeContext::new(&mut document, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        assert!(bench.tool_toggled(BEST_FIT) && bench.is_tool_enabled(BEST_FIT, &ctx));
        assert!(bench.on_input(&stop.0, stop.1, &mut ctx).consumed);
        assert!(!bench.busy(), "stopped, and no new fit started");
        slot.fill(answer(2.0));
        assert_eq!(frame(&mut bench, &mut document), 0);
        assert_eq!(alignments(&document), 0);
    }

    // A newer request supersedes the first, whose answer is never applied.
    let (mut document, scan, reference) = scene();
    let mut bench = InspectionWorkbench::default();
    let first = bench.tab.fitting.begin(&document, scan, reference, true);
    let second = bench.tab.fitting.begin(&document, scan, reference, true);
    first.fill(answer(2.0));
    assert_eq!(frame(&mut bench, &mut document), 0);
    second.fill(answer(5.0));
    assert_eq!(frame(&mut bench, &mut document), 1);
    assert_eq!(
        document.body_placement(scan),
        align::by_numbers([5.0, 0.0, 0.0], [0.0; 3])
    );

    // The fit goes with its tab: another tab's frames never see it, its
    // own tab's do once it is back, and Inspection need not be the bench
    // on screen.
    let (mut asking, scan, reference) = scene();
    let (mut other, ..) = scene();
    let mut bench = InspectionWorkbench::default();
    let slot = bench.tab.fitting.begin(&asking, scan, reference, true);
    let parked = bench.suspend_session();
    bench.resume_session(None);
    slot.fill(answer(2.0));
    assert!(!bench.busy());
    assert_eq!(frame(&mut bench, &mut other), 0);
    bench.resume_session(parked);
    assert!(bench.busy());
    let mut ctx = WorkbenchRuntimeContext::new(&mut asking, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
    bench.on_background_frame(&mut ctx);
    assert_eq!(core_document::HookOutcome::take(&mut ctx).recorded.len(), 1);
    assert_eq!((alignments(&asking), alignments(&other)), (1, 0));
}
