//! A sketch written over the data of an earlier write stores exactly what
//! writing it afresh stores, its unchanged elements and constraints kept
//! or walked again: over the app's corpus of saved documents and edits
//! made at random on their sketches.

use core_document::{Document, WorkbenchFeature};
use sketch_model::SketchFeature;
use sketch_model::feature::SketchPrints;
use sketch_model::sketch::{GeometryElement, Vec2D};

/// Every sketch of every document in the app's corpus, by release.
fn corpus_sketches() -> Vec<SketchFeature> {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app_shell/tests/corpus");
    let mut sketches = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(&root)
        .expect("corpus")
        .flatten()
        .flat_map(|release| {
            std::fs::read_dir(release.path())
                .expect("release")
                .flatten()
        })
        .map(|file| file.path())
        .filter(|path| path.extension().is_some_and(|e| e == "prtcad"))
        .collect();
    files.sort();
    for path in files {
        let doc = Document::load_from_bytes(std::fs::read(&path).expect("read")).expect("load");
        sketches.extend(
            doc.features_of::<SketchFeature>()
                .into_iter()
                .map(|(_, f)| f),
        );
    }
    sketches
}

/// A small change made at random, as an edit makes one: a point moved,
/// or put on an axis, a zero turned to the other sign, an element
/// removed with what hangs on it, one added from another sketch, two
/// swapped, construction toggled, a constraint's settings or the
/// sketch's own changed.
fn edit(feature: &mut SketchFeature, donor: &SketchFeature, roll: &mut impl FnMut(usize) -> usize) {
    let sketch = &mut feature.sketch;
    let n = sketch.geometry.len();
    match roll(8) {
        0 if n > 0 => {
            if let GeometryElement::Point(p) = &mut sketch.geometry[roll(n)] {
                p.position = Vec2D::new(p.position.x + 0.5, p.position.y - 0.25);
            }
        }
        7 if n > 0 => {
            if let GeometryElement::Point(p) = &mut sketch.geometry[roll(n)] {
                // A value equal to the last, written differently.
                p.position.x = if p.position.x == 0.0 {
                    -p.position.x
                } else {
                    0.0
                };
            }
        }
        1 if n > 0 => {
            let id = sketch.geometry[roll(n)].id();
            sketch.remove_geometry_cascade(&[id]);
        }
        2 if !donor.sketch.geometry.is_empty() => {
            let element = donor.sketch.geometry[roll(donor.sketch.geometry.len())].clone();
            sketch.geometry.insert(roll(n + 1), element);
        }
        3 if n > 1 => sketch.geometry.swap(roll(n), roll(n)),
        4 if n > 0 => {
            let id = sketch.geometry[roll(n)].id();
            let on = sketch.is_construction(id);
            sketch.set_construction(id, !on);
        }
        5 if !sketch.constraints.is_empty() => {
            let c = roll(sketch.constraints.len());
            let constraint = &mut sketch.constraints[c];
            constraint.driving = !constraint.driving;
            constraint.label_offset = Some(Vec2D::new(1.0, roll(9) as f64));
            constraint.name = (roll(2) == 0).then(|| format!("d{c}"));
        }
        _ => {
            sketch.name.push('x');
            feature.section_view = !feature.section_view;
            feature.plane.origin[2] += 1.0;
        }
    }
}

/// Each corpus sketch written over the one before, then edited at random
/// and written over its last write each time, stores the very text a
/// fresh write of it stores; so does each write over the last that keeps
/// the elements and constraints serialized alike.
#[test]
fn a_sketch_written_over_earlier_data_stores_what_a_fresh_write_does() {
    let sketches = corpus_sketches();
    assert!(sketches.len() > 10, "the corpus holds sketches");
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut roll = |n: usize| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        ((state >> 33) as usize) % n
    };
    let mut data = serde_json::Value::Null;
    let mut kept = (serde_json::Value::Null, SketchPrints::default());
    for (k, sketch) in sketches.iter().enumerate() {
        let donor = &sketches[(k + 1) % sketches.len()];
        let mut feature = sketch.clone();
        for _ in 0..12 {
            let fresh = feature.to_json().to_string();
            data = feature.to_json_over(data);
            assert_eq!(data.to_string(), fresh);
            let (over, then) = std::mem::take(&mut kept);
            let now = SketchPrints::of(&feature.sketch);
            let written = feature.to_json_over_kept(over, &then, &now);
            assert_eq!(written.to_string(), fresh, "kept");
            kept = (written, now);
            edit(&mut feature, donor, &mut roll);
        }
    }
}
