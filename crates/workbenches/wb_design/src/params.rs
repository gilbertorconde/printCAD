//! The numbers of Design's features and of datums, as formulas set
//! and read them: `Pad.length`, `Pocket.depth`, `Hole.diameter`,
//! `Fillet.radius`, `Plane.offset_z`.
//!
//! A feature's JSON is its variant's name around its fields
//! (`{"Pad": {"length": 10}}`), so each number's key is its path there
//! (`/Pad/length`), which never moves while the feature is edited.

use core_document::expr::Dim;
use core_document::walk::Walker;
use core_document::{FeatureNode, Parameter};
use serde_json::Value;

const LENGTH: Dim = Dim::LENGTH;
const ANGLE: Dim = Dim::ANGLE;

/// A primitive shape's numbers, by the fields its JSON holds: lengths,
/// angles where a field ends in `_deg`, and a prism's count of sides.
pub(crate) fn primitive_kind(kind: &kernel_api::PrimitiveKind, walker: &mut Walker) {
    let Ok(Value::Object(kind)) = serde_json::to_value(kind) else {
        return;
    };
    for (shape, dims) in &kind {
        walker.at(shape, |walker| {
            for (field, value) in dims.as_object().into_iter().flatten() {
                let Some(number) = value.as_f64() else {
                    continue;
                };
                let (name, dim) = match field.strip_suffix("_deg") {
                    Some(stem) => (stem, Some(ANGLE)),
                    None if field == "sides" => (field.as_str(), None),
                    None => (field.as_str(), Some(LENGTH)),
                };
                walker.at(field, |walker| {
                    walker.number(&number, name, &label_of(name), dim)
                });
            }
        });
    }
}

/// A stated boolean tolerance: zero leaves it to the kernel, and is no
/// number to set.
pub(crate) fn boolean_tolerance(tolerance: &Option<f32>, walker: &mut Walker) {
    if tolerance.is_some_and(|t| t > 0.0) {
        walker.number(
            tolerance,
            "boolean_tolerance",
            "Boolean tolerance",
            Some(LENGTH),
        );
    }
}

/// A primitive's position: `x`, `y`, `z` of its origin.
pub(crate) fn primitive_placement(placement: &kernel_api::Placement, walker: &mut Walker) {
    walker.at("origin", |walker| {
        walker.axes(&placement.origin, "", "Position ", Some(LENGTH));
    });
}

/// The numbers of a Design feature, as its definition names them
/// (`#[walk(...)]` on its fields, `feature.rs`).
pub fn feature_parameters(node: &FeatureNode) -> Vec<Parameter> {
    use core_document::WorkbenchFeature;
    crate::feature::DesignFeature::from_json(&node.data)
        .map(|feature| Walker::of(&feature).parameters)
        .unwrap_or_default()
}

/// Every name Design gives a number, for checks.
#[cfg(test)]
pub(crate) fn every_name() -> Vec<&'static str> {
    use core_document::walk::Walk;
    let mut names = Vec::new();
    crate::feature::DesignFeature::names(&mut names);
    // A primitive's, named from its shape's fields, and a stated tolerance.
    names.extend(["x", "y", "z", "radius", "height", "length", "angle"]);
    names.push("boolean_tolerance");
    names.extend(
        datum_parameters()
            .into_iter()
            .filter_map(|p| p.name)
            .map(|n| &*Box::leak(n.into_boxed_str())),
    );
    names
}

/// The numbers of a datum: its offset from what it is attached to.
pub fn datum_parameters() -> Vec<Parameter> {
    vec![
        Parameter::new("offset_x", "Offset X", LENGTH, "/offset/translation/0"),
        Parameter::new("offset_y", "Offset Y", LENGTH, "/offset/translation/1"),
        Parameter::new(
            "offset_z",
            "Offset along the normal",
            LENGTH,
            "/offset/translation/2",
        ),
        Parameter::new("rotation", "Rotation", ANGLE, "/offset/rotation_deg"),
        Parameter::new("tilt_x", "Tilt about X", ANGLE, "/offset/tilt/0"),
        Parameter::new("tilt_y", "Tilt about Y", ANGLE, "/offset/tilt/1"),
    ]
}

/// `radius1` as a label: "Radius 1".
fn label_of(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else if c.is_ascii_digit() && !out.ends_with(|p: char| p.is_ascii_digit()) {
            out.push(' ');
            out.push(c);
        } else if c == '_' {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feature::DesignFeature;
    use core_document::WorkbenchFeature;

    fn node(feature: &DesignFeature) -> FeatureNode {
        FeatureNode::new(core_document::FeatureId::new(), feature)
    }

    /// Every parameter a feature lists points at a number in its JSON.
    fn all_resolve(feature: &DesignFeature) -> Vec<String> {
        let n = node(feature);
        let params = feature_parameters(&n);
        for p in &params {
            let at = n.data.pointer(&p.pointer);
            assert!(
                at.is_some_and(Value::is_number),
                "{} points at {:?} in {}",
                p.pointer,
                at,
                n.data
            );
        }
        params.into_iter().filter_map(|p| p.name).collect()
    }

    const SKETCH: &str = "5c0e3a44-0000-4000-8000-000000000001";

    #[test]
    fn every_listed_number_is_where_it_says() {
        let feature = |json: Value| DesignFeature::from_json(&json).expect("a feature");
        let pad = feature(
            serde_json::json!({"Pad": {"sketch": SKETCH, "length": 10.0, "reversed": false}}),
        );
        assert_eq!(
            all_resolve(&pad),
            [
                "length",
                "length2",
                "taper",
                "offset",
                "offset2",
                "start_offset"
            ]
        );
        // A custom direction's components join them, only while it is one.
        let slanted = feature(serde_json::json!({"Pad": {
            "sketch": SKETCH, "length": 10.0, "reversed": false,
            "direction": {"Custom": [0.0, 1.0, 1.0]},
        }}));
        assert_eq!(
            all_resolve(&slanted),
            [
                "length",
                "length2",
                "taper",
                "offset",
                "direction_x",
                "direction_y",
                "direction_z",
                "offset2",
                "start_offset"
            ]
        );
        let hole = feature(serde_json::json!({"Hole": {
            "sketch": SKETCH, "diameter": 5.0, "depth": 8.0, "through_all": false,
            "cut": {"Counterbore": {"diameter": 9.0, "depth": 2.0}},
        }}));
        let names = all_resolve_present(&hole);
        assert!(
            names.contains(&"counterbore_depth".to_string()),
            "{names:?}"
        );
        assert!(
            !names.contains(&"countersink_angle".to_string()),
            "{names:?}"
        );
        let pattern = feature(
            serde_json::json!({"MultiTransform": {"originals": [], "steps": [
                {"Linear": {"axis": "X", "length": 20.0, "occurrences": 3}},
                {"Polar": {"axis": "Z", "angle_deg": 90.0, "occurrences": 4}},
            ]}}),
        );
        assert_eq!(
            all_resolve(&pattern),
            [
                "step1_length",
                "step1_occurrences",
                "step2_angle",
                "step2_occurrences"
            ]
        );
        let counts: Vec<bool> = feature_parameters(&node(&pattern))
            .iter()
            .map(|p| p.integer)
            .collect();
        assert_eq!(counts, [false, true, false, true]);
        assert_eq!(datum_parameters().len(), 6);
    }

    /// The listed numbers the feature has: a counterbore hole has no
    /// countersink.
    fn all_resolve_present(feature: &DesignFeature) -> Vec<String> {
        let n = node(feature);
        feature_parameters(&n)
            .into_iter()
            .filter(|p| n.data.pointer(&p.pointer).is_some_and(Value::is_number))
            .filter_map(|p| p.name)
            .collect()
    }

    #[test]
    fn a_primitive_lists_its_sizes_angles_and_place() {
        let json = serde_json::json!({"Primitive": {
            "kind": {"Cylinder": {"radius": 5.0, "height": 10.0, "angle_deg": 360.0}},
            "placement": {"origin": [0.0, 0.0, 0.0], "x_axis": [1.0, 0.0, 0.0], "z_axis": [0.0, 0.0, 1.0]},
            "subtractive": false,
        }});
        let feature = DesignFeature::from_json(&json).unwrap();
        let params = feature_parameters(&node(&feature));
        let names: Vec<(String, Dim)> = params
            .iter()
            .map(|p| (p.name.clone().unwrap(), p.dim))
            .collect();
        assert!(names.contains(&("radius".into(), LENGTH)));
        assert!(names.contains(&("angle".into(), ANGLE)));
        assert!(names.contains(&("z".into(), LENGTH)));
        assert_eq!(label_of("radius1"), "Radius 1");
    }
}
