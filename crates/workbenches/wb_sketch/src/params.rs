//! A sketch's dimensions as formulas set and read them.
//!
//! Every driving dimension can be set by a formula, kept under the
//! constraint's id so it stays with its dimension while the sketch is
//! edited. One the user has named can be read by other formulas as
//! `Sketch.width`. Once formulas have set the values, the sketch solves for
//! them ([`settle`]), and what builds and draws is that solved copy.

use core_document::expr::{self, Dim};
use core_document::{FeatureNode, Parameter, WorkbenchFeature};
use serde_json::Value;

use crate::feature::SketchFeature;
use crate::sketch::{ConstraintKind, constraint_label, dimension_value};

/// The field of a dimensional kind that holds its value, and its variant.
fn value_field(kind: &ConstraintKind) -> Option<(&'static str, &'static str)> {
    Some(match kind {
        ConstraintKind::Length { .. } => ("Length", "length"),
        ConstraintKind::Radius { .. } => ("Radius", "radius"),
        ConstraintKind::Diameter { .. } => ("Diameter", "diameter"),
        ConstraintKind::Distance { .. } => ("Distance", "distance"),
        ConstraintKind::DistanceX { .. } => ("DistanceX", "value"),
        ConstraintKind::DistanceY { .. } => ("DistanceY", "value"),
        ConstraintKind::Angle { .. } => ("Angle", "angle_rad"),
        ConstraintKind::AngleToAxis { .. } => ("AngleToAxis", "angle_rad"),
        _ => return None,
    })
}

/// The sketch's driving dimensions.
pub fn parameters(node: &FeatureNode) -> Vec<Parameter> {
    let Ok(feature) = SketchFeature::from_json(&node.data) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, c) in feature.sketch.constraints.iter().enumerate() {
        if !c.driving || dimension_value(&c.kind).is_none() {
            continue;
        }
        let Some((variant, field)) = value_field(&c.kind) else {
            continue;
        };
        let angular = crate::sketch::is_angular(&c.kind);
        let name = c.name.as_deref().filter(|n| expr::is_valid_name(n));
        let label = name
            .map(str::to_string)
            .unwrap_or_else(|| constraint_label(&c.kind));
        let pointer = format!("/sketch/constraints/{i}/kind/{variant}/{field}");
        let dim = if angular { Dim::ANGLE } else { Dim::LENGTH };
        let mut p =
            Parameter::new(name.unwrap_or(""), &label, dim, pointer).keyed(c.id.to_string());
        if name.is_none() {
            p = p.unnamed();
        }
        if angular {
            p = p.scaled(std::f64::consts::PI / 180.0);
        }
        out.push(p);
    }
    out
}

/// Solve the sketch for the values its formulas gave it.
pub fn settle(values: &mut Value) {
    let Ok(mut feature) = SketchFeature::from_json(values) else {
        return;
    };
    crate::solver::solve(&mut feature.sketch);
    *values = feature.to_json();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Constraint, GeometryElement, Line, Point, Sketch, Vec2D};

    fn node(sketch: Sketch) -> FeatureNode {
        FeatureNode::new(
            core_document::FeatureId::new(),
            &SketchFeature::from_sketch(sketch),
        )
    }

    #[test]
    fn named_driving_dimensions_are_parameters_where_their_value_is() {
        let mut sketch = Sketch::new("s");
        let a = Point::new(Vec2D::new(0.0, 0.0));
        let b = Point::new(Vec2D::new(10.0, 0.0));
        let line = Line::new(a.id, b.id);
        let line_id = line.id;
        sketch.geometry.push(GeometryElement::Point(a));
        sketch.geometry.push(GeometryElement::Point(b));
        sketch.geometry.push(GeometryElement::Line(line));
        let mut width = Constraint::new(ConstraintKind::Length {
            line: line_id,
            length: 10.0,
        });
        width.name = Some("width".into());
        let width_id = width.id;
        sketch.constraints.push(width);
        let mut reference = Constraint::new(ConstraintKind::Length {
            line: line_id,
            length: 10.0,
        });
        reference.driving = false;
        sketch.constraints.push(reference);
        sketch
            .constraints
            .push(Constraint::new(ConstraintKind::AngleToAxis {
                line: line_id,
                axis: crate::sketch::AxisDirection::Horizontal,
                angle_rad: 0.5,
            }));
        let n = node(sketch);
        let params = parameters(&n);
        assert_eq!(
            params.len(),
            2,
            "the reference dimension is measured, not set"
        );
        assert_eq!(params[0].name.as_deref(), Some("width"));
        assert_eq!(params[0].key, width_id.to_string());
        assert_eq!(
            n.data.pointer(&params[0].pointer),
            Some(&serde_json::json!(10.0))
        );
        assert_eq!(params[1].name, None, "unnamed: settable, not readable");
        assert_eq!(params[1].dim, Dim::ANGLE);
        assert!(
            n.data
                .pointer(&params[1].pointer)
                .is_some_and(Value::is_number)
        );
    }
}
