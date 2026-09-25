//! The checks a part gets before it prints, called as a script calls
//! them, through the real benches with the kernel answering.

use core_document::{
    CommandArgs, CommandError, CommandResult, CommandSpec, Document, DocumentService, WorkbenchId,
    WorkbenchRuntimeContext,
};
use scripting::ScriptEngine;

struct Benches {
    registry: DocumentService,
    document: Document,
}

impl scripting::Host for Benches {
    fn commands(&self) -> Vec<CommandSpec> {
        self.registry
            .commands()
            .into_iter()
            .map(|(_, c)| c.clone())
            .collect()
    }

    fn call(&mut self, id: &str, args: CommandArgs) -> CommandResult {
        let (bench, spec): (WorkbenchId, _) = self
            .registry
            .command(id)
            .ok_or_else(|| CommandError::Unknown(id.to_string()))?;
        spec.check(&args)?;
        let wb = self.registry.workbench_mut(&bench).unwrap();
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut self.document, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        ctx.kernel = Some(&kernel_ogeom::QUERIES);
        wb.run_command(id, &args, &mut ctx)
    }
}

fn benches() -> Benches {
    let mut registry = DocumentService::default();
    registry
        .register_workbench(Box::new(wb_sketch::SketchWorkbench::default()))
        .unwrap();
    registry
        .register_workbench(Box::new(wb_part::PartDesignWorkbench::default()))
        .unwrap();
    Benches {
        registry,
        document: Document::new("checks"),
    }
}

/// Run `script` and read back the globals it leaves, as numbers.
fn run(host: &mut Benches, script: &str, globals: &[&str]) -> Vec<f64> {
    let mut engine = ScriptEngine::new();
    let out = engine.run_script(script, "check.lua", host);
    assert_eq!(out.error, None);
    globals
        .iter()
        .map(|name| {
            let out = engine.run_script(&format!("print({name})"), "read.lua", host);
            assert_eq!(out.error, None);
            let text = out.printed.join("");
            text.trim()
                .parse()
                .unwrap_or_else(|_| panic!("{name} = {text:?}"))
        })
        .collect()
}

#[test]
fn a_script_measures_a_strip_and_an_l_by_their_thinnest_walls() {
    let mut host = benches();
    let got = run(
        &mut host,
        r#"
        local strip = pc.sketch.new{plane = "XY"}
        pc.sketch.rect{sketch = strip, x = 0, y = 0, width = 20, height = 2}
        local a = pc.sketch.wall_thickness{sketch = strip}
        strip_wall, strip_thin = a.thinnest, a.thin and 1 or 0
        strip_y = a["where"].y

        local l = pc.sketch.new{plane = "XY"}
        pc.sketch.polyline{sketch = l, closed = true, points = {
            {0, 0}, {30, 0}, {30, 3}, {1.5, 3}, {1.5, 25}, {0, 25}}}
        local b = pc.sketch.wall_thickness{sketch = l, minimum = 2}
        l_wall, l_thin = b.thinnest, b.thin and 1 or 0
        "#,
        &["strip_wall", "strip_thin", "strip_y", "l_wall", "l_thin"],
    );
    let [strip_wall, strip_thin, strip_y, l_wall, l_thin] = got[..] else {
        unreachable!()
    };
    assert!((strip_wall - 2.0).abs() < 1e-3, "{strip_wall}");
    assert_eq!(strip_thin, 0.0, "2 mm is over the 0.8 mm default");
    assert!((strip_y - 1.0).abs() < 1e-3, "{strip_y}");
    assert!((l_wall - 1.5).abs() < 1e-3, "{l_wall}");
    assert_eq!(l_thin, 1.0, "1.5 mm is under the 2 mm asked for");
}

/// A tube 20 long along Z: a ring of 3 and 2 mm padded, its solid built.
fn padded_tube(host: &mut Benches) -> core_document::BodyId {
    run(
        host,
        r#"
        local s = pc.sketch.new{plane = "XY"}
        pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
        pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 2}
        pc.part.pad{sketch = s, length = 20}
        "#,
        &[],
    );
    let body = host.document.bodies()[0].id;
    let ops = wb_part::body_build_ops(&host.document, body).unwrap().ops;
    let built = kernel_ogeom::OgeomKernel::new()
        .execute_solid_chain(&ops, &kernel_api::TessellationSettings::default())
        .unwrap();
    host.document
        .set_imported_brep_data(body, built.brep_blob, Vec::new());
    body
}

#[test]
fn a_script_measures_a_padded_tubes_centre_line() {
    let mut host = benches();
    let body = padded_tube(&mut host);
    let got = run(
        &mut host,
        &format!(
            r#"
            local c = pc.part.centre_line{{body = "{}",
                from_point = {{2.5, 0, 0}}, from_normal = {{0, 0, -1}},
                to_point = {{0, 2.5, 20}}, to_normal = {{0, 0, 1}}}}
            length, straight, first_z = c.length, c.straight and 1 or 0, c.points[1][3]
            "#,
            body.0
        ),
        &["length", "straight", "first_z"],
    );
    assert!((got[0] - 20.0).abs() < 1e-3, "{got:?}");
    assert_eq!(got[1], 1.0, "a pad's centre line is straight");
    assert!(got[2].abs() < 1e-3, "it starts at the first face: {got:?}");
}

#[test]
fn the_centre_line_tool_takes_two_faces_and_records_what_it_measured() {
    use core_document::{FaceRef, HookOutcome, Workbench, WorkbenchInputEvent};

    let mut host = benches();
    let body = padded_tube(&mut host);
    let mut bench = wb_part::PartDesignWorkbench::default();
    // Looking down Z from above, a millimetre a fiftieth of the viewport.
    let view_proj = [
        [0.02, 0.0, 0.0, 0.0],
        [0.0, -0.02, 0.0, 0.0],
        [0.0, 0.0, -0.001, 0.0],
        [0.0, 0.0, 0.5, 1.0],
    ];
    let mut ctx =
        WorkbenchRuntimeContext::new(&mut host.document, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
    ctx.kernel = Some(&kernel_ogeom::QUERIES);
    ctx.view_proj = Some(view_proj);
    ctx.selected_body_id = Some(body.0);
    ctx.selected_face = Some(FaceRef {
        point: [2.5, 0.0, 0.0],
        normal: [0.0, 0.0, -1.0],
        surface: None,
    });
    bench.on_input(
        &WorkbenchInputEvent::ToolActivated,
        Some("part.centre_line"),
        &mut ctx,
    );
    assert_eq!(
        bench.task(&ctx).map(|t| t.title),
        Some("Centre line".to_string())
    );
    // The same pick seen again on the next frame is not a second face.
    bench.on_frame(0.016, &mut ctx);
    assert!(HookOutcome::take(&mut ctx).recorded.is_empty());

    ctx.selected_face = Some(FaceRef {
        point: [0.0, 2.5, 20.0],
        normal: [0.0, 0.0, 1.0],
        surface: None,
    });
    bench.on_frame(0.016, &mut ctx);
    let recorded = HookOutcome::take(&mut ctx).recorded;
    assert_eq!(recorded.len(), 1, "{:?}", ctx.drain_logs());
    assert_eq!(recorded[0].id, "part.centre_line");
    let length = recorded[0].result["length"].as_f64().unwrap();
    assert!((length - 20.0).abs() < 1e-3, "{length}");

    let labels: Vec<String> = bench
        .get_screen_space_labels(&ctx, None)
        .into_iter()
        .map(|l| l.text)
        .collect();
    assert_eq!(labels, vec!["20.00 mm".to_string()]);
    assert!(!bench.get_screen_space_overlays(&ctx, None).is_empty());
    let footer = bench.viewport_hud(&ctx).unwrap().footer;
    assert_eq!(footer, vec!["Centre line 20.00 mm".to_string()]);

    // Closing the panel puts the tool away.
    bench.finish_editing(&mut ctx);
    assert!(bench.task(&ctx).is_none());
    assert!(bench.get_screen_space_labels(&ctx, None).is_empty());
}
