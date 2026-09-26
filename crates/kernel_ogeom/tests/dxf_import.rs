//! A DXF drawing becomes a sketch through the sketcher's command, read by
//! the real kernel: the path File › Import and a script both take.

use core_document::{
    CommandArgs, CommandError, CommandResult, CommandSpec, Document, DocumentService, FeatureId,
    Recorded, WorkbenchFeature, WorkbenchId, WorkbenchRuntimeContext,
};
use scripting::ScriptEngine;
use serde_json::json;
use wb_sketch::SketchFeature;
use wb_sketch::sketch::{GeometryElement, Sketch};

struct Benches {
    registry: DocumentService,
    document: Document,
}

impl Benches {
    fn new() -> Self {
        let mut registry = DocumentService::default();
        registry
            .register_workbench(Box::new(wb_sketch::SketchWorkbench::default()))
            .unwrap();
        Self {
            registry,
            document: Document::new("drawing"),
        }
    }

    fn sketch(&self, id: &str) -> Sketch {
        SketchFeature::from_json(self.document.get_feature_data(feature(id)).unwrap())
            .unwrap()
            .sketch
    }
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

fn feature(id: &str) -> FeatureId {
    serde_json::from_value(json!(id)).unwrap()
}

/// A DXF as a file on disk, named `name` in a directory of its own.
fn dxf_file(name: &str, text: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("printcad-dxf-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

/// Group codes and values, one per line each.
fn dxf(entities: &[(&str, &str)]) -> String {
    let mut out = String::from("0\nSECTION\n2\nENTITIES\n");
    for (code, value) in entities {
        out.push_str(&format!("{code}\n{value}\n"));
    }
    out.push_str("0\nENDSEC\n0\nEOF\n");
    out
}

/// A square drawn as one polyline closing on its first vertex, a line on
/// its own beside it, and a hidden centre line.
fn square_line_and_hidden() -> String {
    let mut e: Vec<(&str, &str)> = vec![("0", "POLYLINE"), ("8", "0"), ("66", "1"), ("70", "0")];
    for (x, y) in [
        ("0.0", "0.0"),
        ("20.0", "0.0"),
        ("20.0", "20.0"),
        ("0.0", "20.0"),
        ("0.0", "0.0"),
    ] {
        e.extend([("0", "VERTEX"), ("8", "0"), ("10", x), ("20", y)]);
    }
    e.push(("0", "SEQEND"));
    e.extend([
        ("0", "LINE"),
        ("8", "0"),
        ("10", "30.0"),
        ("20", "0.0"),
        ("11", "30.0"),
        ("21", "20.0"),
    ]);
    e.extend([
        ("0", "LINE"),
        ("8", "HIDDEN"),
        ("10", "10.0"),
        ("20", "-5.0"),
        ("11", "10.0"),
        ("21", "25.0"),
    ]);
    dxf(&e)
}

fn lines(sketch: &Sketch) -> Vec<&wb_sketch::sketch::Line> {
    sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Line(l) => Some(l),
            _ => None,
        })
        .collect()
}

#[test]
fn a_drawing_imports_as_a_sketch_whose_outline_shares_its_corners() {
    let path = dxf_file("bracket.dxf", &square_line_and_hidden());
    let mut host = Benches::new();
    // File › Import finds the command by the file's extension.
    let (_, import) = host.registry.file_import_for(&path).expect("an importer");
    assert_eq!(import.command, "sketch.import_dxf");
    let mut engine = ScriptEngine::new();
    let out = engine.run_script(
        &format!(
            "sketch = pc.sketch.import_dxf{{path = {:?}}}\nprint(sketch)",
            path.display().to_string()
        ),
        "import.lua",
        &mut host,
    );
    assert_eq!(out.error, None);
    let id = out.printed.concat();

    let node = host.document.get_feature_meta(feature(&id)).unwrap();
    assert_eq!(node.name, "bracket", "named after the file");
    assert!(node.body.is_some(), "in a body");

    let sketch = host.sketch(&id);
    let all = lines(&sketch);
    assert_eq!(all.len(), 6, "four sides, the lone line, the hidden one");
    let construction: Vec<_> = all
        .iter()
        .filter(|l| sketch.is_construction(l.id))
        .collect();
    assert_eq!(construction.len(), 1, "the hidden line is construction");

    // The square's four sides stand on four points, each shared by two.
    let sides: Vec<_> = all
        .iter()
        .filter(|l| {
            let a = sketch.point_position(l.start).unwrap();
            let b = sketch.point_position(l.end).unwrap();
            a.x <= 20.0 && b.x <= 20.0 && !sketch.is_construction(l.id)
        })
        .collect();
    assert_eq!(sides.len(), 4);
    let mut corners: Vec<_> = sides.iter().flat_map(|l| [l.start, l.end]).collect();
    corners.sort();
    corners.dedup();
    assert_eq!(corners.len(), 4, "consecutive sides share their corner");

    // The lone line leaves the outline open, so the profile is the square
    // alone once the line is construction too.
    let mut only_square = sketch.clone();
    for l in lines(&sketch) {
        if !sides.iter().any(|s| s.id == l.id) {
            only_square.set_construction(l.id, true);
        }
    }
    let wires = wb_sketch::profile::extract_wires(&only_square).expect("a closed square");
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0].segments.len(), 4);
}

#[test]
fn a_recorded_import_replays_as_the_same_sketch() {
    let path = dxf_file("plate.dxf", &square_line_and_hidden());
    let mut host = Benches::new();
    let args: CommandArgs = [
        ("path".to_string(), json!(path.display().to_string())),
        ("scale".to_string(), json!(2.0)),
    ]
    .into_iter()
    .collect();
    let result = scripting::Host::call(&mut host, "sketch.import_dxf", args.clone()).unwrap();
    let first = host.sketch(result.as_str().unwrap());

    let mut recorder = scripting::Recorder::default();
    recorder.push(&Recorded {
        id: "sketch.import_dxf".to_string(),
        args,
        result,
    });
    let script = recorder.script("an import");
    let mut replay = Benches::new();
    let out = ScriptEngine::new().run_script(
        &format!("{script}\nprint(sketch1)"),
        "replay.lua",
        &mut replay,
    );
    assert_eq!(out.error, None, "{script}");
    let again = replay.sketch(&out.printed.concat());

    let ends = |s: &Sketch| -> Vec<[f32; 4]> {
        lines(s)
            .iter()
            .map(|l| {
                let a = s.point_position(l.start).unwrap();
                let b = s.point_position(l.end).unwrap();
                [a.x, a.y, b.x, b.y]
            })
            .collect()
    };
    assert_eq!(ends(&first), ends(&again));
    assert!(ends(&first).iter().any(|e| e[2] == 40.0), "scaled by 2");
}

#[test]
fn a_file_that_is_not_a_drawing_makes_nothing() {
    let path = dxf_file("broken.dxf", "0\nSECTION\n2");
    let mut host = Benches::new();
    let args: CommandArgs = [("path".to_string(), json!(path.display().to_string()))]
        .into_iter()
        .collect();
    assert!(scripting::Host::call(&mut host, "sketch.import_dxf", args).is_err());
    assert!(host.document.bodies().is_empty(), "no body made for it");
}

#[test]
fn arcs_and_circles_import_as_sketch_arcs_and_circles() {
    let text = dxf(&[
        ("0", "CIRCLE"),
        ("8", "0"),
        ("10", "0.0"),
        ("20", "0.0"),
        ("40", "5.0"),
        ("0", "ARC"),
        ("8", "0"),
        ("10", "20.0"),
        ("20", "0.0"),
        ("40", "4.0"),
        ("50", "0.0"),
        ("51", "90.0"),
        // A closed LWPOLYLINE of two half-circle bulges: a slot end to end.
        ("0", "LWPOLYLINE"),
        ("8", "0"),
        ("90", "2"),
        ("70", "1"),
        ("10", "40.0"),
        ("20", "0.0"),
        ("42", "1.0"),
        ("10", "50.0"),
        ("20", "0.0"),
        ("42", "1.0"),
    ]);
    let path = dxf_file("round.dxf", &text);
    let mut host = Benches::new();
    let args: CommandArgs = [("path".to_string(), json!(path.display().to_string()))]
        .into_iter()
        .collect();
    let id = scripting::Host::call(&mut host, "sketch.import_dxf", args).unwrap();
    let sketch = host.sketch(id.as_str().unwrap());

    let circles: Vec<f32> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Circle(c) => Some(c.radius),
            _ => None,
        })
        .collect();
    assert_eq!(circles, vec![5.0]);
    let arcs: Vec<f32> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Arc(a) => Some(a.radius),
            _ => None,
        })
        .collect();
    assert_eq!(arcs.len(), 3, "the arc and the slot's two halves");
    assert!(arcs.iter().all(|r| *r == 4.0 || *r == 5.0), "{arcs:?}");
    // Circle, and the slot closed on its two bulges; the lone arc is open.
    let mut closed_only = sketch.clone();
    for g in &sketch.geometry {
        if let GeometryElement::Arc(a) = g
            && a.radius == 4.0
        {
            closed_only.set_construction(a.id, true);
        }
    }
    assert_eq!(
        wb_sketch::profile::extract_wires(&closed_only)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn a_polyline_header_point_is_not_a_vertex() {
    // An R12 POLYLINE carries a point of its own (its elevation in 30),
    // written as zeros; only the VERTEX entities are the curve.
    let mut e: Vec<(&str, &str)> = vec![
        ("0", "POLYLINE"),
        ("8", "0"),
        ("66", "1"),
        ("10", "0.0"),
        ("20", "0.0"),
        ("30", "0.0"),
        ("70", "0"),
    ];
    for (x, y) in [("5.0", "5.0"), ("15.0", "5.0")] {
        e.extend([("0", "VERTEX"), ("8", "0"), ("10", x), ("20", y)]);
    }
    e.push(("0", "SEQEND"));
    let path = dxf_file("header.dxf", &dxf(&e));
    let mut host = Benches::new();
    let args: CommandArgs = [("path".to_string(), json!(path.display().to_string()))]
        .into_iter()
        .collect();
    let id = scripting::Host::call(&mut host, "sketch.import_dxf", args).unwrap();
    let sketch = host.sketch(id.as_str().unwrap());
    assert_eq!(lines(&sketch).len(), 1, "one segment, (5, 5) to (15, 5)");
}
