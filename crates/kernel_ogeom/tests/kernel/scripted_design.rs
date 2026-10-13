//! Design commands run as a script, the body built by the kernel and
//! measured: what a script's `pc.doc.rebuild()` and `pc.doc.measure`
//! answer.

use core_document::{BodyId, Document};
use kernel_api::{FaceProbe, KernelQueries, PhysicalProperties};
use kernel_ogeom::OgeomKernel;
use scripting::ScriptEngine;

use super::common::{benches, try_chain};

/// Run `script` on a document with one body, `BODY` in it standing for
/// the body's id, and build the body: its snapshot.
fn built(script: &str) -> Result<Vec<u8>, String> {
    let mut document = Document::new("scripted");
    let body: BodyId = document.create_body(None);
    let mut host = benches(document);
    let out = ScriptEngine::new().run_script(
        &script.replace("BODY", &body.0.to_string()),
        "part.lua",
        &mut host,
    );
    assert_eq!(out.error, None);
    let ops = wb_design::body_build_ops(&host.document, body).unwrap().ops;
    try_chain(&ops)
        .map(|b| b.brep_blob)
        .map_err(|e| e.to_string())
}

fn measured(blob: &[u8]) -> PhysicalProperties {
    OgeomKernel::new().physical_properties(blob).unwrap()
}

const BORED_PAD: &str = r#"
    local b = "BODY"
    local s = pc.sketch.new{body = b, plane = "XY"}
    pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
    pc.design.pad{sketch = s, length = 5}
    local h = pc.sketch.new{body = b, plane = "XY", offset = 5}
    pc.sketch.circle{sketch = h, x = 10, y = 5, radius = 2}
    pc.design.pocket{sketch = h, through_all = true}
"#;

#[test]
fn a_bore_pocketed_through_a_pad_is_deleted() {
    let blob = built(&format!(
        "{BORED_PAD}
        pc.design.delete_faces{{body = b, face_point = {{12, 5, 2.5}}, face_normal = {{-1, 0, 0}}}}"
    ))
    .unwrap();
    let volume = measured(&blob).volume_mm3.unwrap();
    assert!((volume - 1000.0).abs() < 1e-3, "the plate whole: {volume}");
}

#[test]
fn a_bore_moves_sideways() {
    let blob = built(
        r#"
        local b = "BODY"
        pc.design.primitive{body = b, kind = {Box = {length = 20, width = 10, height = 5}}}
        local h = pc.sketch.new{body = b, plane = "XY", offset = 5}
        pc.sketch.circle{sketch = h, x = 10, y = 5, radius = 2}
        pc.design.pocket{sketch = h, through_all = true}
        pc.design.move_faces{body = b, face_point = {12, 5, 2.5}, face_normal = {-1, 0, 0},
          translation = {3, 0, 0}}
        "#,
    )
    .unwrap();
    let m = measured(&blob);
    let bore = std::f64::consts::PI * 4.0 * 5.0;
    let volume = m.volume_mm3.unwrap();
    assert!((volume - (1000.0 - bore)).abs() < 1e-3, "{volume}");
    // The bore's axis at x = 13 pulls the centre of what is left that way.
    let centre_x = (1000.0 * 10.0 - bore * 13.0) / (1000.0 - bore);
    assert!(
        (m.centre_mm[0] - centre_x).abs() < 1e-3,
        "{:?}",
        m.centre_mm
    );
}

#[test]
fn a_rod_with_a_sharp_corner_has_a_centre_line() {
    let blob = built(
        r#"
        local b = "BODY"
        local profile = pc.sketch.new{body = b, plane = "XY"}
        pc.sketch.circle{sketch = profile, x = 0, y = 0, radius = 2}
        local path = pc.sketch.new{body = b, plane = "XZ"}
        pc.sketch.polyline{sketch = path, points = {{0, 0}, {0, 20}, {15, 20}}}
        pc.design.pipe{sketch = profile}
        "#,
    )
    .unwrap();
    let probe = |point: [f64; 3], normal: [f64; 3]| FaceProbe {
        name: 0,
        point,
        normal,
    };
    let line = kernel_ogeom::QUERIES
        .centre_line(
            &blob,
            &probe([0.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
            &probe([15.0, 0.0, 20.0], [1.0, 0.0, 0.0]),
            0.02,
        )
        .unwrap();
    assert!(
        (line.length - 35.0).abs() < 0.05,
        "up 20, across 15: {}",
        line.length
    );
    assert!(
        line.points
            .iter()
            .any(|p| (p[0].powi(2) + p[1].powi(2) + (p[2] - 20.0).powi(2)).sqrt() < 0.05),
        "through the corner: {:?}",
        line.points
    );
}

#[test]
fn a_refined_bracket_with_a_hole_measures_exactly() {
    let blob = built(
        r#"
        local b = "BODY"
        local side = pc.sketch.new{body = b, plane = "XZ"}
        pc.sketch.polyline{sketch = side, closed = true,
          points = {{0, 0}, {40, 0}, {40, 5}, {5, 5}, {5, 30}, {0, 30}}}
        pc.design.pad{sketch = side, length = 30, reversed = true}
        pc.design.fillet{body = b, radius = 4,
          edges = {Edges = {{point = {5, 15, 5}, direction = {0, 1, 0}}}}}
        local foot = pc.sketch.new{body = b, plane = "XY", offset = 5}
        pc.sketch.circle{sketch = foot, x = 27, y = 15, radius = 1}
        pc.design.hole{sketch = foot, diameter = 5, through_all = true, refine = true}
        "#,
    )
    .unwrap();
    let m = measured(&blob);
    let pi = std::f64::consts::PI;
    let exact = 325.0 * 30.0 + (16.0 - 4.0 * pi) * 30.0 - pi * 6.25 * 5.0;
    assert!(!m.approximate, "measured on a tessellation");
    assert!((m.volume_mm3.unwrap() - exact).abs() < 1e-6, "{m:?}");
}

#[test]
fn a_helix_of_part_turns_measures_exactly() {
    for turns in [0.5, 0.75, 1.5] {
        let blob = built(&format!(
            r#"
            local b = "BODY"
            local s = pc.sketch.new{{body = b, plane = "XZ"}}
            pc.sketch.rect{{sketch = s, x = 9, y = 0, width = 2, height = 2}}
            pc.design.helix{{sketch = s, mode = "PitchTurns", pitch = 4, turns = {turns}}}
            "#
        ))
        .unwrap();
        let m = measured(&blob);
        // A 2 x 2 square about a radius of 10, swept `turns` times round.
        let exact = 4.0 * 2.0 * std::f64::consts::PI * 10.0 * turns;
        assert!(!m.approximate, "{turns} turns measured on a tessellation");
        assert!(
            (m.volume_mm3.unwrap() - exact).abs() < 1e-3,
            "{turns}: {m:?}"
        );
    }
}
