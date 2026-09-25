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
