//! Dimension quantity changes shared by task drafts and commands.

use crate::sketch::{ConstraintKind, Sketch};
use core_document::FeatureId;
use uuid::Uuid;

/// Turn constraint `id` into a diameter (or a radius) of the same circle,
/// its value converted. The factor its value was multiplied by; `None`
/// when it is no radius or diameter dimension. A dimension already of the
/// kind asked for stays as it is.
pub(crate) fn switch_circle_dimension(
    sketch: &mut Sketch,
    id: Uuid,
    diameter: bool,
) -> Option<f64> {
    let c = sketch.constraints.iter_mut().find(|c| c.id == id)?;
    let (kind, scale) = match (&c.kind, diameter) {
        (ConstraintKind::Radius { circle, radius }, true) => (
            ConstraintKind::Diameter {
                circle: *circle,
                diameter: radius * 2.0,
            },
            2.0,
        ),
        (ConstraintKind::Diameter { circle, diameter }, false) => (
            ConstraintKind::Radius {
                circle: *circle,
                radius: diameter * 0.5,
            },
            0.5,
        ),
        (ConstraintKind::Radius { .. }, false) | (ConstraintKind::Diameter { .. }, true) => {
            return Some(1.0);
        }
        _ => return None,
    };
    c.kind = kind;
    Some(scale)
}

/// [`switch_circle_dimension`] on sketch `sketch_id`'s `sketch`, the
/// formula on the dimension, if any, scaled with its value.
pub(crate) fn switch_dimension(
    document: &mut core_document::Document,
    sketch_id: FeatureId,
    sketch: &mut Sketch,
    id: Uuid,
    diameter: bool,
) -> Option<f64> {
    let scale = switch_circle_dimension(sketch, id, diameter)?;
    let key = id.to_string();
    if let Some(formula) = document.feature_formula(sketch_id, &key) {
        let scaled = scaled_formula(formula, scale);
        document
            .set_feature_formula(sketch_id, key, Some(scaled))
            .ok()?;
    }
    Some(scale)
}

/// Arc dimension `id` of sketch `sketch_id`'s `sketch` made to measure its
/// arc's sweep (`angle`) or its length, at what the arc measures now; a
/// formula on it is dropped, since it gave the other quantity. Whether it
/// is an arc's dimension.
pub(crate) fn switch_arc_dimension(
    document: &mut core_document::Document,
    sketch_id: FeatureId,
    sketch: &mut Sketch,
    id: Uuid,
    angle: bool,
) -> bool {
    let Some(at) = sketch.constraints.iter().position(|c| c.id == id) else {
        return false;
    };
    let kind = match sketch.constraints[at].kind {
        ConstraintKind::ArcLength { arc, .. } if angle => ConstraintKind::ArcAngle {
            arc,
            angle_rad: crate::measure::arc_sweep(sketch, arc).unwrap_or(0.0),
        },
        ConstraintKind::ArcAngle { arc, .. } if !angle => ConstraintKind::ArcLength {
            arc,
            length: crate::measure::arc_length(sketch, arc).unwrap_or(0.0),
        },
        ConstraintKind::ArcLength { .. } | ConstraintKind::ArcAngle { .. } => return true,
        _ => return false,
    };
    sketch.constraints[at].kind = kind;
    let key = id.to_string();
    if document.feature_formula(sketch_id, &key).is_some() {
        let _ = document.set_feature_formula(sketch_id, key, None);
    }
    true
}

/// `formula` giving `scale` times what it gave, undoing the opposite
/// switch rather than stacking on it.
pub(crate) fn scaled_formula(formula: &str, scale: f64) -> String {
    let formula = formula.trim();
    let (undo, apply) = if scale > 1.0 {
        (" / 2", " * 2")
    } else {
        (" * 2", " / 2")
    };
    if scale == 1.0 {
        return formula.to_string();
    }
    if let Some(inner) = formula
        .strip_suffix(undo)
        .and_then(|f| f.strip_prefix('('))
        .and_then(|f| f.strip_suffix(')'))
    {
        return inner.to_string();
    }
    format!("({formula}){apply}")
}
