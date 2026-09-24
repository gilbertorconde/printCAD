//! The numbers of Part Design's features and of datums, as formulas set
//! and read them: `Pad.length`, `Pocket.depth`, `Hole.diameter`,
//! `Fillet.radius`, `Plane.offset_z`.
//!
//! A feature's JSON is its variant's name around its fields
//! (`{"Pad": {"length": 10}}`), so each number's key is its path there
//! (`/Pad/length`), which never moves while the feature is edited.

use core_document::expr::Dim;
use core_document::{FeatureNode, Parameter};
use serde_json::Value;

const LENGTH: Dim = Dim::LENGTH;
const ANGLE: Dim = Dim::ANGLE;
const NUMBER: Dim = Dim::NUMBER;

/// A field: where it is under the variant, what formulas call it, its
/// label and kind. A kind of `None` is a count.
type Field = (&'static str, &'static str, &'static str, Option<Dim>);

/// Each variant's numbers.
fn fields(variant: &str) -> &'static [Field] {
    match variant {
        "Pad" => &[
            ("length", "length", "Length", Some(LENGTH)),
            ("length2", "length2", "Second length", Some(LENGTH)),
            ("taper_deg", "taper", "Taper angle", Some(ANGLE)),
            ("up_to_offset", "offset", "Offset", Some(LENGTH)),
        ],
        "Pocket" => &[
            ("depth", "depth", "Depth", Some(LENGTH)),
            ("depth2", "depth2", "Second depth", Some(LENGTH)),
            ("taper_deg", "taper", "Taper angle", Some(ANGLE)),
            ("up_to_offset", "offset", "Offset", Some(LENGTH)),
        ],
        "Revolution" | "Groove" => &[
            ("angle_deg", "angle", "Angle", Some(ANGLE)),
            ("second_angle_deg", "angle2", "Second angle", Some(ANGLE)),
        ],
        "Helix" => &[
            ("pitch", "pitch", "Pitch", Some(LENGTH)),
            ("height", "height", "Height", Some(LENGTH)),
            ("turns", "turns", "Turns", Some(NUMBER)),
            ("cone_angle_deg", "cone_angle", "Cone angle", Some(ANGLE)),
        ],
        "Hole" => &[
            ("diameter", "diameter", "Diameter", Some(LENGTH)),
            ("depth", "depth", "Depth", Some(LENGTH)),
            (
                "cut/Counterbore/diameter",
                "counterbore_diameter",
                "Counterbore diameter",
                Some(LENGTH),
            ),
            (
                "cut/Counterbore/depth",
                "counterbore_depth",
                "Counterbore depth",
                Some(LENGTH),
            ),
            (
                "cut/Countersink/diameter",
                "countersink_diameter",
                "Countersink diameter",
                Some(LENGTH),
            ),
            (
                "cut/Countersink/angle_deg",
                "countersink_angle",
                "Countersink angle",
                Some(ANGLE),
            ),
        ],
        "Fillet" => &[("radius", "radius", "Radius", Some(LENGTH))],
        "Chamfer" => &[
            ("size", "size", "Size", Some(LENGTH)),
            ("size2", "size2", "Second size", Some(LENGTH)),
            ("angle_deg", "angle", "Angle", Some(ANGLE)),
        ],
        "Draft" => &[("angle_deg", "angle", "Angle", Some(ANGLE))],
        "Thickness" => &[("value", "thickness", "Thickness", Some(LENGTH))],
        "LinearPattern" => &[
            ("length", "length", "Length", Some(LENGTH)),
            ("occurrences", "occurrences", "Occurrences", None),
        ],
        "PolarPattern" => &[
            ("angle_deg", "angle", "Angle", Some(ANGLE)),
            ("occurrences", "occurrences", "Occurrences", None),
        ],
        _ => &[],
    }
}

/// A multi-transform step's numbers.
fn step_fields(kind: &str) -> &'static [Field] {
    match kind {
        "Linear" => &[
            ("length", "length", "Length", Some(LENGTH)),
            ("occurrences", "occurrences", "Occurrences", None),
        ],
        "Polar" => &[
            ("angle_deg", "angle", "Angle", Some(ANGLE)),
            ("occurrences", "occurrences", "Occurrences", None),
        ],
        "Scale" => &[
            ("factor", "factor", "Factor", Some(NUMBER)),
            ("occurrences", "occurrences", "Occurrences", None),
        ],
        _ => &[],
    }
}

fn parameter(pointer: String, name: &str, label: &str, dim: Option<Dim>) -> Parameter {
    match dim {
        Some(dim) => Parameter::new(name, label, dim, pointer),
        None => Parameter::count(name, label, pointer),
    }
}

/// The numbers of a Part Design feature.
pub fn feature_parameters(node: &FeatureNode) -> Vec<Parameter> {
    let Some((variant, body)) = node.data.as_object().and_then(|m| m.iter().next()) else {
        return Vec::new();
    };
    let mut out: Vec<Parameter> = fields(variant)
        .iter()
        .map(|(path, name, label, dim)| parameter(format!("/{variant}/{path}"), name, label, *dim))
        .collect();
    match variant.as_str() {
        "MultiTransform" => {
            let steps = body.get("steps").and_then(Value::as_array);
            for (i, step) in steps.into_iter().flatten().enumerate() {
                let Some(kind) = step.as_object().and_then(|m| m.keys().next()) else {
                    continue;
                };
                for (path, name, label, dim) in step_fields(kind) {
                    out.push(parameter(
                        format!("/MultiTransform/steps/{i}/{kind}/{path}"),
                        &format!("step{}_{name}", i + 1),
                        &format!("Step {} {}", i + 1, label.to_lowercase()),
                        *dim,
                    ));
                }
            }
        }
        "Primitive" => {
            if let Some((shape, dims)) = body
                .get("kind")
                .and_then(Value::as_object)
                .and_then(|m| m.iter().next())
            {
                for (field, value) in dims.as_object().into_iter().flatten() {
                    if !value.is_number() {
                        continue;
                    }
                    let (name, dim) = match field.strip_suffix("_deg") {
                        Some(stem) => (stem, Some(ANGLE)),
                        None if field == "sides" => (field.as_str(), None),
                        None => (field.as_str(), Some(LENGTH)),
                    };
                    out.push(parameter(
                        format!("/Primitive/kind/{shape}/{field}"),
                        name,
                        &label_of(name),
                        dim,
                    ));
                }
            }
            for (i, axis) in ["x", "y", "z"].iter().enumerate() {
                out.push(parameter(
                    format!("/Primitive/placement/origin/{i}"),
                    axis,
                    &format!("Position {}", axis.to_uppercase()),
                    Some(LENGTH),
                ));
            }
        }
        _ => {}
    }
    out
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
    use crate::feature::PartFeature;
    use core_document::WorkbenchFeature;

    fn node(feature: &PartFeature) -> FeatureNode {
        FeatureNode::new(core_document::FeatureId::new(), feature)
    }

    /// Every parameter a feature lists points at a number in its JSON.
    fn all_resolve(feature: &PartFeature) -> Vec<String> {
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
        let feature = |json: Value| PartFeature::from_json(&json).expect("a feature");
        let pad = feature(
            serde_json::json!({"Pad": {"sketch": SKETCH, "length": 10.0, "reversed": false}}),
        );
        assert_eq!(all_resolve(&pad), ["length", "length2", "taper", "offset"]);
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
        assert_eq!(datum_parameters().len(), 4);
    }

    /// The listed numbers the feature has: a counterbore hole has no
    /// countersink.
    fn all_resolve_present(feature: &PartFeature) -> Vec<String> {
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
        let feature = PartFeature::from_json(&json).unwrap();
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
