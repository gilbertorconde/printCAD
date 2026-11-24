//! Lofts between rounded rectangles (four lines and four corner arcs),
//! alone, as frames with an inner loop, and added to a body of padded
//! frames they touch or overlap: built from scripts the way an agent
//! builds them, and held to the volumes the sections bound.
//!
//! A ruled loft between rounded rectangles of one corner radius has a
//! rounded rectangle for every cross-section, its half-width running
//! linearly, so its area is quadratic in height and Simpson's rule gives
//! its volume exactly.

use core_document::{BodyId, Document};
use ogeom::topo::{NodeData, ShapeType, explore_unique};
use scripting::ScriptEngine;

use super::common::{benches, body_volume, is_sound, read_shape, settle_as_host};

/// The sketch helpers every script here starts with: `rr` draws a rounded
/// rectangle of half-width `half` and corner radius `r` centred on the
/// origin, `frame` pads a frame of two of them, its wall 2.5 thick.
const HELPERS: &str = r#"
local function rr(s, half, r)
  local h, k = half, half - r
  pc.sketch.line{sketch=s,x1=-k,y1=-h,x2=k,y2=-h}
  pc.sketch.line{sketch=s,x1=h,y1=-k,x2=h,y2=k}
  pc.sketch.line{sketch=s,x1=k,y1=h,x2=-k,y2=h}
  pc.sketch.line{sketch=s,x1=-h,y1=k,x2=-h,y2=-k}
  pc.sketch.arc{sketch=s,x=k,y=-k,radius=r,start=-90,["end"]=0}
  pc.sketch.arc{sketch=s,x=k,y=k,radius=r,start=0,["end"]=90}
  pc.sketch.arc{sketch=s,x=-k,y=k,radius=r,start=90,["end"]=180}
  pc.sketch.arc{sketch=s,x=-k,y=-k,radius=r,start=180,["end"]=270}
end
local function frame(z0, half, len, body)
  local s = pc.sketch.new{body=body, plane="XY", offset=z0}
  rr(s, half, 3); rr(s, half - 2.5, 2)
  pc.design.pad{sketch=s, length=len}
end
"#;

/// The area of a rounded rectangle of half-width `half`, corners `r`.
fn rounded(half: f64, r: f64) -> f64 {
    4.0 * half * half - (4.0 - std::f64::consts::PI) * r * r
}

/// The volume between z = `lo` and `hi` of a ruled loft whose half-width
/// runs linearly from `h0` at `z0` to `h1` at `z1`, corners `r`.
fn ruled_volume(h0: f64, z0: f64, h1: f64, z1: f64, r: f64, lo: f64, hi: f64) -> f64 {
    let half = |z: f64| h0 + (h1 - h0) * (z - z0) / (z1 - z0);
    let area = |z: f64| rounded(half(z), r);
    (hi - lo) / 6.0 * (area(lo) + 4.0 * area((lo + hi) / 2.0) + area(hi))
}

/// The two padded frames the body scripts build: 145 square at z 0..5 and
/// 135 square at z 40..45.
fn frames_volume() -> f64 {
    5.0 * (rounded(72.5, 3.0) - rounded(70.0, 2.0))
        + 5.0 * (rounded(67.5, 3.0) - rounded(65.0, 2.0))
}

/// Run `script` (after the helpers) on a document of two bodies, `BODY`
/// and `TUBE` in it standing for their ids, and build both as the host
/// does: the document and `BODY`.
fn settled(script: &str) -> (Document, BodyId) {
    let mut document = Document::new("rounded lofts");
    let body = document.create_body(None);
    let tube = document.create_body(None);
    let mut host = benches(document);
    let script = format!("{HELPERS}{script}")
        .replace("BODY", &body.0.to_string())
        .replace("TUBE", &tube.0.to_string());
    let out = ScriptEngine::new().run_script(&script, "loft.lua", &mut host);
    assert_eq!(out.error, None);
    settle_as_host(&host.registry, &mut host.document);
    let failures: Vec<String> = host
        .document
        .feature_tree()
        .all_nodes()
        .filter_map(|(id, _)| host.document.derived().error(*id).map(str::to_string))
        .collect();
    assert!(failures.is_empty(), "{failures:?}");
    (host.document, body)
}

fn assert_volume(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-6 * expected,
        "volume {actual}, the sections bound {expected}"
    );
}

/// The body's snapshot.
fn blob(doc: &Document, body: BodyId) -> Vec<u8> {
    doc.derived()
        .imported_brep_blob_arc(body)
        .expect("the body has a solid")
        .to_vec()
}

/// How many of the snapshot's faces lie on planes, of how many.
fn planar_faces(blob: &[u8]) -> (usize, usize) {
    let (model, shape) = read_shape(blob);
    let faces = explore_unique(&model, &shape, ShapeType::Face).unwrap();
    let planes = faces
        .iter()
        .filter(|f| {
            let Some(NodeData::Face(data)) = model.node(f).map(|n| n.data()) else {
                return false;
            };
            matches!(
                model.geometry().surface(data.surface),
                Some(ogeom::geom::SurfaceGeometry::Plane(_))
            )
        })
        .count();
    (planes, faces.len())
}

/// Lines pair with lines and arcs with arcs: the walls are four planes
/// and four cones, each named apart from every other face, so a pick on
/// one side never finds the corner beside it.
#[test]
fn a_ruled_loft_of_rounded_rectangles_has_a_wall_per_side_and_corner() {
    let (doc, body) = settled(
        r#"
        local body = "BODY"
        local a = pc.sketch.new{body=body, plane="XY", offset=5}
        rr(a, 72.5, 3)
        local b = pc.sketch.new{body=body, plane="XY", offset=40}
        rr(b, 67.5, 3)
        pc.design.loft{sketch=a, ruled=true, sections={a, b}}
        "#,
    );
    let snapshot = blob(&doc, body);
    assert!(is_sound(&snapshot));
    assert_eq!(
        planar_faces(&snapshot),
        (6, 10),
        "4 sides and 2 caps planar, 4 corners not"
    );
    assert_volume(
        body_volume(&doc, body),
        ruled_volume(72.5, 5.0, 67.5, 40.0, 3.0, 5.0, 40.0),
    );

    let mesh = &doc.derived().imported_geometry(body).expect("a mesh").mesh;
    let names: Vec<Vec<u64>> = (0..mesh.face_names.len())
        .map(|f| {
            let mut all = vec![mesh.face_names[f]];
            all.extend(mesh.face_other_names.get(f).into_iter().flatten());
            all
        })
        .collect();
    for (i, a) in names.iter().enumerate() {
        for b in &names[i + 1..] {
            assert!(
                a.iter().all(|n| !b.contains(n)),
                "two faces share a name: {names:?}"
            );
        }
    }
}

/// The outer loops pair and the inner loops pair: a hollow tapered tube,
/// ruled with a wall per line and arc of both loops, smooth just as
/// closed.
#[test]
fn a_loft_between_frames_is_a_hollow_tube() {
    let tube = ruled_volume(72.5, 0.0, 67.5, 35.0, 3.0, 0.0, 35.0)
        - ruled_volume(70.0, 0.0, 65.0, 35.0, 2.0, 0.0, 35.0);
    for ruled in [true, false] {
        let (doc, body) = settled(&format!(
            r#"
            local body = "BODY"
            local a = pc.sketch.new{{body=body, plane="XY"}}
            rr(a, 72.5, 3); rr(a, 70, 2)
            local b = pc.sketch.new{{body=body, plane="XY", offset=35}}
            rr(b, 67.5, 3); rr(b, 65, 2)
            pc.design.loft{{sketch=a, ruled={ruled}, sections={{a, b}}}}
            "#
        ));
        let snapshot = blob(&doc, body);
        assert!(is_sound(&snapshot), "ruled {ruled}");
        if ruled {
            assert_eq!(planar_faces(&snapshot), (10, 18));
        }
        let volume = body_volume(&doc, body);
        assert!(
            (volume - tube).abs() <= 1e-3 * tube,
            "ruled {ruled}: {volume}, the loops bound {tube}"
        );
    }
}

/// A loft added to a body of two frames, its caps on the frames' facing
/// faces or sunk 1 mm into each frame, fuses into one sound solid holding
/// both.
#[test]
fn a_loft_fuses_onto_the_frames_it_touches_or_enters() {
    for (lo, hi) in [(5.0, 40.0), (4.0, 41.0)] {
        let (doc, body) = settled(&format!(
            r#"
            local frames = "BODY"
            frame(0, 72.5, 5, frames); frame(40, 67.5, 5, frames)
            local a = pc.sketch.new{{body=frames, plane="XY", offset={lo}}}; rr(a, 72.5, 3)
            local b = pc.sketch.new{{body=frames, plane="XY", offset={hi}}}; rr(b, 67.5, 3)
            pc.design.loft{{sketch=a, ruled=true, sections={{a, b}}}}
            "#
        ));
        assert!(is_sound(&blob(&doc, body)), "{lo}..{hi}");
        let loft = |a: f64, b: f64| ruled_volume(72.5, lo, 67.5, hi, 3.0, a, b);
        // Sunk into the frames, the loft fills the lower frame's opening
        // inside its outline and covers the upper frame's ring whole.
        let shared = if lo < 5.0 {
            (loft(lo, 5.0) - (5.0 - lo) * rounded(70.0, 2.0))
                + (hi - 40.0) * (rounded(67.5, 3.0) - rounded(65.0, 2.0))
        } else {
            0.0
        };
        assert_volume(
            body_volume(&doc, body),
            frames_volume() + loft(lo, hi) - shared,
        );
    }
}

/// A hollow tube made in a body of its own and added to the frames' body
/// by a Boolean: the frames and the tube, which meet along the frames'
/// facing faces, as one solid.
#[test]
fn a_rounded_tube_body_joins_the_frames_by_a_boolean() {
    let (doc, frames) = settled(
        r#"
        local frames, tube = "BODY", "TUBE"
        frame(0, 72.5, 5, frames); frame(40, 67.5, 5, frames)
        local s = pc.sketch.new{body=tube, plane="XY", offset=5}
        rr(s, 72.5, 3)
        local c = pc.sketch.new{body=tube, plane="XY", offset=40}; rr(c, 67.5, 3)
        pc.design.loft{sketch=s, ruled=true, sections={s, c}}
        local s2 = pc.sketch.new{body=tube, plane="XY", offset=5}; rr(s2, 70, 2)
        local c2 = pc.sketch.new{body=tube, plane="XY", offset=40}; rr(c2, 65, 2)
        pc.design.subtractive_loft{sketch=s2, ruled=true, sections={s2, c2}}
        pc.design.boolean{body=frames, tool_body=tube}
        "#,
    );
    assert!(is_sound(&blob(&doc, frames)));
    let tube = ruled_volume(72.5, 5.0, 67.5, 40.0, 3.0, 5.0, 40.0)
        - ruled_volume(70.0, 5.0, 65.0, 40.0, 2.0, 5.0, 40.0);
    assert_volume(body_volume(&doc, frames), frames_volume() + tube);
}
