//! A sketch keeps its model in double precision: what is typed or solved
//! is what is saved, and documents written in single precision read back
//! with the numbers they hold.

use core_document::{Compression, Document, WorkbenchFeature};
use wb_sketch::SketchFeature;
use wb_sketch::sketch::{ConstraintKind, GeometryElement, Line, Point, Sketch, SketchPlane, Vec2D};

/// No model-space field of the sketch is single precision.
#[test]
fn the_sketch_model_holds_no_single_precision() {
    let single = concat!("f", "32");
    let source = include_str!("../../src/sketch.rs");
    let found: Vec<(usize, &str)> = source
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            line.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|word| word == single || word.ends_with(single))
        })
        .map(|(n, line)| (n + 1, line.trim()))
        .collect();
    assert!(found.is_empty(), "single precision in sketch.rs: {found:?}");
}

fn line_sketch(length: f64) -> SketchFeature {
    let mut sketch = Sketch::new("Dimensioned");
    let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
    let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(length, 0.0))));
    let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
    sketch.add_constraint(ConstraintKind::Length { line, length });
    let plane = SketchPlane::from_frame(
        [1e4 + 0.123456789, -2e3, 0.5],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
    );
    SketchFeature::new(sketch, plane)
}

fn saved_and_loaded(feature: SketchFeature) -> SketchFeature {
    let mut doc = Document::new("precision");
    let body = doc.create_body(None);
    let id = doc
        .add_feature_in_body(feature, "Dimensioned".into(), Some(body))
        .unwrap();
    let bytes = doc.save_to_bytes(Compression::None).unwrap();
    let loaded = Document::load_from_bytes(bytes).unwrap();
    SketchFeature::from_json(loaded.get_feature_data(id).unwrap()).unwrap()
}

/// A dimension, the point it places and the plane far off the origin come
/// back from a save exactly as they went.
#[test]
fn a_typed_dimension_survives_save_and_load_exactly() {
    let length = 100.000003;
    let before = line_sketch(length);
    let after = saved_and_loaded(before.clone());
    let dimension = after.sketch.constraints.iter().find_map(|c| match c.kind {
        ConstraintKind::Length { length, .. } => Some(length),
        _ => None,
    });
    assert_eq!(dimension.map(f64::to_bits), Some(length.to_bits()));
    let far = after.sketch.geometry.iter().find_map(|g| match g {
        GeometryElement::Point(p) if p.position.x != 0.0 => Some(p.position.x),
        _ => None,
    });
    assert_eq!(far.map(f64::to_bits), Some(length.to_bits()));
    assert_eq!(
        after.plane.origin.map(f64::to_bits),
        before.plane.origin.map(f64::to_bits)
    );
    assert_eq!(
        serde_json::to_string(&after.sketch).unwrap(),
        serde_json::to_string(&before.sketch).unwrap()
    );
}

/// Sketch JSON written from single-precision numbers, as documents were
/// saved before the model was double, reads as those numbers: each value
/// narrows back to the single it was, bit for bit.
#[test]
fn single_precision_json_reads_as_the_numbers_it_held() {
    let singles: [f32; 6] = [1.99997, 0.1, 33.3333, 7.77777, 100.000_003, 12.345678];
    let feature = line_sketch(1.0);
    let mut json = feature.to_json();
    let geometry = json["sketch"]["geometry"].as_array_mut().unwrap();
    // The second point, as serde wrote an `f32` into a JSON value (the
    // way a document's feature data was saved), and as text (the shortest
    // digits, the way an op log or a script carried it).
    let point = geometry
        .iter_mut()
        .filter_map(|g| g.get_mut("Point"))
        .nth(1)
        .unwrap();
    point["position"]["x"] = serde_json::to_value(singles[0]).unwrap();
    point["position"]["y"] = serde_json::from_str(&format!("{}", singles[1])).unwrap();
    json["plane"]["origin"] = serde_json::to_value([singles[2], singles[3], singles[5]]).unwrap();
    let constraint = json["sketch"]["constraints"][0]["kind"]["Length"]
        .as_object_mut()
        .unwrap();
    constraint.insert("length".into(), serde_json::to_value(singles[4]).unwrap());
    let text = serde_json::to_string(&json).unwrap();
    let read = SketchFeature::from_json(&serde_json::from_str(&text).unwrap()).unwrap();

    let narrow = |v: f64| (v as f32).to_bits();
    let point = read
        .sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some(p.position),
            _ => None,
        })
        .nth(1)
        .unwrap();
    assert_eq!(narrow(point.x), singles[0].to_bits());
    assert_eq!(
        point.x,
        f64::from(singles[0]),
        "a widened single stays that double"
    );
    assert_eq!(narrow(point.y), singles[1].to_bits());
    assert_eq!(point.y, 0.1, "the shortest digits read as their double");
    assert_eq!(
        read.plane.origin.map(narrow),
        [singles[2], singles[3], singles[5]].map(f32::to_bits)
    );
    let length = read.sketch.constraints.iter().find_map(|c| match c.kind {
        ConstraintKind::Length { length, .. } => Some(length),
        _ => None,
    });
    assert_eq!(length.map(narrow), Some(singles[4].to_bits()));
}
