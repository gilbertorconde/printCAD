//! Surfaces through the real benches: a script sketches, the Surface
//! bench's commands make steps from the sketches, and the kernel builds
//! what the body's plan says.

use core_document::{BodyId, Document};
use ogeom::topo::ShapeType;
use scripting::ScriptEngine;

use super::common::{Benches, chain, count_shapes, with_benches};

fn benches() -> Benches {
    with_benches(
        Document::new("surfaces"),
        [
            Box::new(wb_sketch::SketchWorkbench::default()) as Box<dyn core_document::Workbench>,
            Box::new(wb_design::DesignWorkbench::default()),
            Box::new(wb_surface::SurfaceWorkbench::default()),
        ],
    )
}

fn run(host: &mut Benches, script: &str) {
    let out = ScriptEngine::new().run_script(script, "surface.lua", host);
    assert_eq!(out.error, None, "{:?}", out.printed);
    host.registry.evaluate(&mut host.document);
}

/// The body the surface steps went in, and how many faces and shells its
/// built shape has.
fn built(host: &Benches) -> (BodyId, usize, usize, usize) {
    let body = host
        .document
        .feature_tree()
        .all_nodes()
        .find(|(_, n)| n.workbench_id.as_str() == "wb.surface")
        .and_then(|(_, n)| n.body)
        .expect("a surface body");
    let plan = wb_surface::build::body_plan(&host.document, body).unwrap();
    let result = chain(&plan.ops);
    let count = |kind| count_shapes(&result.brep_blob, kind);
    (
        body,
        count(ShapeType::Face),
        count(ShapeType::Shell),
        count(ShapeType::Solid),
    )
}

#[test]
fn an_open_sketch_extrudes_into_a_surface_body() {
    let mut host = benches();
    run(
        &mut host,
        r#"
        local s = pc.sketch.new{plane = "XY"}
        pc.sketch.polyline{sketch = s, points = {{0, 0}, {10, 0}, {10, 10}}}
        pc.surface.extrude{sketches = {s}, length = 5}
        "#,
    );
    let (body, faces, _, solids) = built(&host);
    assert_eq!((faces, solids), (2, 0));
    let sketch_body = host
        .document
        .feature_tree()
        .all_nodes()
        .find(|(_, n)| n.workbench_id.as_str() == "wb.sketch")
        .and_then(|(_, n)| n.body);
    assert_eq!(
        sketch_body,
        Some(body),
        "the surface goes beside its sketch"
    );
}

/// A step made by a command takes the name it is given, as a Design
/// feature does.
#[test]
fn a_surface_command_names_its_step() {
    let mut host = benches();
    run(
        &mut host,
        r#"
        local s = pc.sketch.new{plane = "XY"}
        pc.sketch.polyline{sketch = s, points = {{0, 0}, {10, 0}, {10, 10}}}
        local wall = pc.surface.extrude{sketches = {s}, length = 5, name = "Wall"}
        pc.surface.sew{body = wall, name = "Joined"}
        "#,
    );
    let names: Vec<String> = host
        .document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == "wb.surface")
        .map(|(_, n)| n.name.clone())
        .collect();
    assert!(
        names.contains(&"Wall".to_string()) && names.contains(&"Joined".to_string()),
        "{names:?}"
    );
}

/// Walls and a floor sewn: an open box, one shell of five faces.
#[test]
fn walls_and_a_floor_sew_into_one_shell() {
    let mut host = benches();
    run(
        &mut host,
        r#"
        local s = pc.sketch.new{plane = "XY"}
        pc.sketch.polyline{sketch = s, points = {{0, 0}, {10, 0}, {10, 10}, {0, 10}, {0, 0}}}
        local walls = pc.surface.extrude{sketches = {s}, length = 5}
        pc.surface.planar{body = walls, sketches = {s}}
        pc.surface.sew{body = walls}
        "#,
    );
    let (_, faces, shells, solids) = built(&host);
    assert_eq!((faces, shells, solids), (5, 1, 0));
}

/// A Design feature asked for on a surface body goes into a body of its
/// own; the surface body keeps building from its surfaces.
#[test]
fn a_design_feature_never_lands_in_a_surface_body() {
    let mut host = benches();
    run(
        &mut host,
        r#"
        local s = pc.sketch.new{plane = "XY"}
        pc.sketch.polyline{sketch = s, points = {{0, 0}, {10, 0}, {10, 10}}}
        pc.surface.extrude{sketches = {s}, length = 5}
        "#,
    );
    let (surface_body, ..) = built(&host);
    let design_in_it = host
        .document
        .feature_tree()
        .all_nodes()
        .any(|(_, n)| n.workbench_id.as_str() == "wb.design" && n.body == Some(surface_body));
    assert!(!design_in_it);
    assert!(wb_surface::SurfaceWorkbench::takes_surfaces(
        &host.document,
        surface_body
    ));
}

/// A step whose sketch is gone cannot be planned: the plan is the history
/// before it, the error is on it, and the step after it is left out.
#[test]
fn a_step_that_cannot_be_planned_leaves_the_history_before_it() {
    let mut host = benches();
    run(
        &mut host,
        r#"
        local a = pc.sketch.new{plane = "XY"}
        pc.sketch.polyline{sketch = a, points = {{0, 0}, {10, 0}}}
        local b = pc.sketch.new{plane = "XY", offset = 5}
        pc.sketch.polyline{sketch = b, points = {{0, 2}, {10, 2}}}
        local walls = pc.surface.extrude{sketches = {a}, length = 5}
        pc.surface.ruled{body = walls, sketches = {a, b}}
        pc.surface.mirror{body = walls}
        "#,
    );
    let named = |kind: &str| {
        host.document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == kind)
            .map(|(id, n)| (*id, n.seq, n.body))
            .collect::<Vec<_>>()
    };
    let mut steps = named("wb.surface");
    steps.sort_by_key(|(_, seq, _)| *seq);
    let body = steps[0].2.unwrap();
    let mut sketches = named("wb.sketch");
    sketches.sort_by_key(|(_, seq, _)| *seq);
    host.document.remove_feature(sketches[1].0).unwrap();

    let plan = wb_surface::build::plan_until_failure(&host.document, body).unwrap();
    assert_eq!(plan.ops.len(), 1, "the extrusion before it");
    let failed = plan.failed.expect("the ruled surface fails");
    assert_eq!(failed.feature, Some(steps[1].0));
    assert_eq!(plan.unbuilt, vec![steps[2].0], "the mirror after it");
    assert!(wb_surface::build::body_plan(&host.document, body).is_err());
}

/// A step missing what it builds from is refused by its command, naming
/// the field.
#[test]
fn a_step_with_nothing_to_build_from_is_refused() {
    let mut host = benches();
    let out =
        ScriptEngine::new().run_script("pc.surface.extrude{length = 5}", "empty.lua", &mut host);
    let error = out.error.expect("refused");
    assert!(error.contains("`sketches`"), "{error}");
}

/// `surface.set` changes a step's fields after it is made, its curves too.
#[test]
fn surface_set_changes_a_step() {
    let mut host = benches();
    run(
        &mut host,
        r#"
        local a = pc.sketch.new{plane = "XY"}
        pc.sketch.polyline{sketch = a, points = {{0, 0}, {10, 0}}}
        local b = pc.sketch.new{plane = "XY"}
        pc.sketch.polyline{sketch = b, points = {{0, 0}, {10, 0}, {10, 10}}}
        local walls = pc.surface.extrude{sketches = {a}, length = 5}
        pc.surface.set{feature = walls, length = 8, sketches = {b}}
        "#,
    );
    let (body, faces, _, _) = built(&host);
    assert_eq!(faces, 2, "built from the second sketch");
    let plan = wb_surface::build::body_plan(&host.document, body).unwrap();
    let result = chain(&plan.ops);
    let top = result.bounds_mm.unwrap().1[2];
    assert!((top - 8.0).abs() < 1e-3, "{top}");
}
