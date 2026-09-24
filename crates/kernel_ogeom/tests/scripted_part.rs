//! A Lua script builds a part through the workbenches' commands, and the
//! kernel builds the solid it describes: the path a console line takes,
//! with the registry standing in for the application.

use core_document::{
    CommandArgs, CommandError, CommandResult, CommandSpec, Document, DocumentService, WorkbenchId,
    WorkbenchRuntimeContext,
};
use kernel_api::TessellationSettings;
use kernel_ogeom::OgeomKernel;
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
        wb.run_command(id, &args, &mut ctx)
    }
}

#[test]
fn a_script_draws_and_pads_a_block() {
    let mut registry = DocumentService::default();
    registry
        .register_workbench(Box::new(wb_sketch::SketchWorkbench::default()))
        .unwrap();
    registry
        .register_workbench(Box::new(wb_part::PartDesignWorkbench::default()))
        .unwrap();
    let mut host = Benches {
        registry,
        document: Document::new("scripted"),
    };
    let mut engine = ScriptEngine::new();
    let out = engine.run_script(
        r#"
        local s = pc.sketch.new{plane = "XY"}
        pc.sketch.rect{sketch = s, x = 0, y = 0, width = 30, height = 20}
        pad = pc.part.pad{sketch = s, length = 12}
        "#,
        "block.lua",
        &mut host,
    );
    assert_eq!(out.error, None);

    let body = host.document.bodies()[0].id;
    let ops = wb_part::body_build_ops(&host.document, body).unwrap().ops;
    let result = OgeomKernel::new()
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .unwrap();
    let (min, max) = result.mesh.bounds().expect("a solid");
    let size: Vec<f32> = (0..3).map(|i| max[i] - min[i]).collect();
    assert!((size[0] - 30.0).abs() < 1e-3, "{size:?}");
    assert!((size[1] - 20.0).abs() < 1e-3, "{size:?}");
    assert!((size[2] - 12.0).abs() < 1e-3, "{size:?}");

    // A field changed from the script changes the solid.
    let out = engine.run_script(
        "pc.part.set{feature = pad, length = 5}",
        "edit.lua",
        &mut host,
    );
    assert_eq!(out.error, None);
    let ops = wb_part::body_build_ops(&host.document, body).unwrap().ops;
    let result = OgeomKernel::new()
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .unwrap();
    let top = result.mesh.bounds().expect("a solid").1[2];
    assert!((top - 5.0).abs() < 1e-3, "{top}");
}
