//! A Lua script builds a part through the workbenches' commands, and the
//! kernel builds the solid it describes: the path a console line takes,
//! with the registry standing in for the application.

use core_document::Document;
use scripting::ScriptEngine;

use super::common::{benches, chain, try_chain, volume};

/// A script written before the Design workbench was renamed, and the
/// document it made, still work: the old command names run the new
/// commands, and the old feature kind reads as the new one.
#[test]
fn old_command_names_and_feature_kinds_still_work() {
    let mut host = benches(Document::new("old"));
    let out = ScriptEngine::new().run_script(
        r#"
        local s = pc.sketch.new{plane = "XY"}
        pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
        local pad = pc.part.pad{sketch = s, length = 4}
        pc.part.set{feature = pad, length = 6}
        "#,
        "old.lua",
        &mut host,
    );
    assert_eq!(out.error, None);

    // The document as a file written before the rename would carry it.
    let json = serde_json::to_string(&host.document)
        .unwrap()
        .replace("\"wb.design\"", "\"wb.part\"");
    assert!(json.contains("\"wb.part\""));
    let old: Document = serde_json::from_str(&json).unwrap();
    let kinds: Vec<&str> = old
        .feature_tree()
        .all_nodes()
        .map(|(_, n)| n.workbench_id.as_str())
        .collect();
    assert!(kinds.contains(&"wb.design"), "{kinds:?}");
    assert!(!kinds.contains(&"wb.part"));
    let body = old.bodies()[0].id;
    let ops = wb_design::body_build_ops(&old, body).unwrap().ops;
    let result = chain(&ops);
    let top = result.mesh.bounds().expect("a solid").1[2];
    assert!((top - 6.0).abs() < 1e-3, "{top}");
}

#[test]
fn a_script_draws_and_pads_a_block() {
    let mut host = benches(Document::new("scripted"));
    let mut engine = ScriptEngine::new();
    let out = engine.run_script(
        r#"
        local s = pc.sketch.new{plane = "XY"}
        pc.sketch.rect{sketch = s, x = 0, y = 0, width = 30, height = 20}
        pad = pc.design.pad{sketch = s, length = 12}
        "#,
        "block.lua",
        &mut host,
    );
    assert_eq!(out.error, None);

    let body = host.document.bodies()[0].id;
    let ops = wb_design::body_build_ops(&host.document, body).unwrap().ops;
    let result = chain(&ops);
    let (min, max) = result.mesh.bounds().expect("a solid");
    let size: Vec<f32> = (0..3).map(|i| max[i] - min[i]).collect();
    assert!((size[0] - 30.0).abs() < 1e-3, "{size:?}");
    assert!((size[1] - 20.0).abs() < 1e-3, "{size:?}");
    assert!((size[2] - 12.0).abs() < 1e-3, "{size:?}");

    // A field changed from the script changes the solid.
    let out = engine.run_script(
        "pc.design.set{feature = pad, length = 5}",
        "edit.lua",
        &mut host,
    );
    assert_eq!(out.error, None);
    let ops = wb_design::body_build_ops(&host.document, body).unwrap().ops;
    let result = chain(&ops);
    let top = result.mesh.bounds().expect("a solid").1[2];
    assert!((top - 5.0).abs() < 1e-3, "{top}");
}

#[test]
fn a_script_sketches_on_a_datum_and_moves_it() {
    let mut host = benches(Document::new("datum"));
    let body = host.document.create_body(None);
    let mut engine = ScriptEngine::new();
    let out = engine.run_script(
        &format!(
            r#"
            local d = pc.design.datum{{kind = "plane", body = "{}", offset = {{0, 0, 10}}}}
            local s = pc.sketch.new{{on = d}}
            local c = pc.sketch.circle{{sketch = s, x = 0, y = 0, radius = 5}}
            pc.sketch.constrain{{sketch = s, kind = "radius", items = {{c}}, value = 4}}
            pc.design.pad{{sketch = s, length = 3}}
            datum = d
            "#,
            body.0
        ),
        "datum.lua",
        &mut host,
    );
    assert_eq!(out.error, None);
    let build = |doc: &Document| {
        let ops = wb_design::body_build_ops(doc, body).unwrap().ops;
        chain(&ops).mesh.bounds().expect("a solid")
    };
    let (min, max) = build(&host.document);
    assert!(
        (min[2] - 10.0).abs() < 1e-3 && (max[2] - 13.0).abs() < 1e-3,
        "{min:?} {max:?}"
    );
    assert!(
        (max[0] - min[0] - 8.0).abs() < 1e-2,
        "the radius constraint holds"
    );

    let out = engine.run_script(
        "pc.design.set{feature = datum, offset = {translation = {0, 0, 20}, rotation_deg = 0, flip = false}}",
        "move.lua",
        &mut host,
    );
    assert_eq!(out.error, None);
    let d = host
        .document
        .feature_tree()
        .all_nodes()
        .find(|(_, n)| n.workbench_id.as_str() == "core.datum")
        .map(|(_, n)| n.data.clone())
        .unwrap();
    assert_eq!(d["offset"]["translation"][2], serde_json::json!(20.0));
    host.registry.evaluate(&mut host.document);
    let (min, max) = build(&host.document);
    assert!(
        (min[2] - 20.0).abs() < 1e-3 && (max[2] - 23.0).abs() < 1e-3,
        "the pad follows its datum: {min:?} {max:?}"
    );
}

/// A script drills a tapped UNC hole with a pointed bottom and a
/// counterdrill, naming the thread, the point and the cut as fields.
#[test]
fn a_script_drills_a_standard_hole() {
    let mut host = benches(Document::new("scripted"));
    let mut engine = ScriptEngine::new();
    let out = engine.run_script(
        r#"
        local s = pc.sketch.new{plane = "XY"}
        pc.sketch.rect{sketch = s, x = 0, y = 0, width = 30, height = 30}
        pc.design.pad{sketch = s, length = 12}
        "#,
        "plate.lua",
        &mut host,
    );
    assert_eq!(out.error, None);
    let body = host.document.bodies()[0].id;
    let out = engine.run_script(
        &format!(
            r#"
            local at = pc.sketch.new{{body = "{}", plane = "XY", offset = 12}}
            pc.sketch.circle{{sketch = at, x = 15, y = 15, radius = 2}}
            pc.design.hole{{
              sketch = at,
              depth = 8,
              thread = {{standard = "Unc", size = "1/4-20", class = "3B"}},
              threaded = true,
              drill_point = {{Angled = {{angle_deg = 118}}}},
              cut = {{Counterdrill = {{diameter = 9, depth = 2, angle_deg = 90}}}},
            }}
            "#,
            body.0
        ),
        "hole.lua",
        &mut host,
    );
    assert_eq!(out.error, None);
    let hole = host
        .document
        .feature_tree()
        .all_nodes()
        .find_map(|(id, node)| {
            let data = &node.data;
            data.get("Hole").map(|_| (*id, data.clone()))
        })
        .expect("a hole");
    let feature: wb_design::DesignFeature = serde_json::from_value(hole.1).unwrap();
    let wb_design::DesignFeature::Hole { thread, .. } = &feature else {
        panic!("a hole");
    };
    let thread = thread.as_ref().expect("a thread");
    assert_eq!(thread.designation(), "1/4-20 UNC-3B");
    // The tap drill of 1/4-20, #7.
    assert!((wb_design::hole_diameter(&feature) - 5.1054).abs() < 1e-4);

    let ops = wb_design::body_build_ops(&host.document, body).unwrap().ops;
    let result = try_chain(&ops).unwrap_or_else(|e| panic!("builds: {e}"));
    let volume = volume(&result.brep_blob);
    assert!(
        volume < 30.0 * 30.0 * 12.0 - std::f64::consts::PI * 4.5 * 4.5 * 2.0,
        "{volume}"
    );
}

/// A sketch with a closed rectangle and loose lines around it pads the
/// rectangle: what does not close is left out.
#[test]
fn a_pad_takes_the_closed_loop_and_leaves_loose_lines_out() {
    let mut host = benches(Document::new("loose"));
    let out = ScriptEngine::new().run_script(
        r#"
        local s = pc.sketch.new{plane = "XY"}
        pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 5}
        pc.sketch.line{sketch = s, x1 = 20, y1 = 0, x2 = 30, y2 = 8}
        pc.sketch.line{sketch = s, x1 = 10, y1 = 5, x2 = 14, y2 = 9}
        pc.design.pad{sketch = s, length = 2}
        "#,
        "loose.lua",
        &mut host,
    );
    assert_eq!(out.error, None);
    let body = host.document.bodies()[0].id;
    let ops = wb_design::body_build_ops(&host.document, body).unwrap().ops;
    let built = chain(&ops);
    let (lo, hi) = built.mesh.bounds().unwrap();
    assert!((hi[0] - lo[0] - 10.0).abs() < 1e-3 && (hi[1] - lo[1] - 5.0).abs() < 1e-3);
}
