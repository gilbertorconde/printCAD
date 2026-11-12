//! Constraints: adding them, setting a dimension's value, switching a
//! constraint's flags and removing axis alignments.

use core_document::{
    Args, CommandError, CommandResult, CommandSpec, CommandTable, FeatureId, ParamKind,
    WorkbenchRuntimeContext,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::sketch::{ConstraintKind, Sketch};

use super::editing;
use super::{constrain, ids, sketch};
use crate::SketchWorkbench;
use crate::feature::SketchFeature;

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<SketchWorkbench>) {
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.constrain",
            "Constrain elements, as the constraint's toolbar button does for a selection",
        ))
        .param(
            "kind",
            ParamKind::String,
            "coincident, point_on_object, midpoint, horizontal, vertical, \
             horizontal_vertical, parallel, perpendicular, tangent, equal, symmetric, block, group, lock, dimension, distance, \
             distance_x, distance_y, gap, arc_length, radius, diameter, radius_diameter, \
             angle, angle_x, angle_y, angle_at_point, arc_angle, angle_three_points (items: \
             arm, corner, arm), ellipse_minor or refraction; radius on an ellipse is its \
             major radius, arc_length on a spline or conic its length",
        )
        .param(
            "items",
            ParamKind::List,
            "Element ids, or \"origin\", \"x_axis\" and \"y_axis\"",
        )
        .optional(
            "value",
            ParamKind::Number,
            "A dimension's value (mm, degrees for an angle, the ratio of indices for a \
             refraction); the measured one when left out",
        )
        .optional(
            "remove_redundant",
            ParamKind::Bool,
            "Take away the older constraints the new ones make redundant",
        )
        .returns("the new constraints' ids"),
        editing(constrain_command),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.set_value",
            "Change a dimension's value",
        ))
        .param("constraint", ParamKind::Id, "")
        .param("value", ParamKind::Number, "mm, or degrees for an angle")
        .optional(
            "driving",
            ParamKind::Bool,
            "false makes it a reference dimension that only measures",
        ),
        editing(set_value),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.set_constraint",
            "Make constraints driving or reference, active or not, parked or not",
        ))
        .param("items", ParamKind::List, "The constraints")
        .optional(
            "driving",
            ParamKind::Bool,
            "false: a reference dimension that only measures",
        )
        .optional("active", ParamKind::Bool, "false: kept but not solved")
        .optional(
            "as",
            ParamKind::String,
            "\"radius\" or \"diameter\": a circle's or arc's dimension measures that instead; \
             \"length\" or \"angle\": an arc's length or sweep dimension measures the other",
        )
        .optional(
            "parked",
            ParamKind::Bool,
            "true: its symbol moves to the parked layer, drawn only while that layer shows; \
             it still solves",
        ),
        editing(set_constraint),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.remove_axis_alignment",
            "Turn the horizontal and vertical constraints of lines into parallel and \
             perpendicular ones among them, so the group keeps its shape and turns as a whole",
        ))
        .param("items", ParamKind::List, "The lines")
        .returns("how many constraints changed"),
        editing(remove_axis_alignment),
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "constrain.md", include_str!("constrain.md"), &[])
}

fn constrain_command(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let sketch = &mut feature.sketch;
    let kind = a.string("kind")?;
    let items = ids(args.get("items"), "items", sketch)?;
    let value = a.opt_number("value")?;
    let remove_redundant = a.opt_bool("remove_redundant")?.unwrap_or(false);
    let made = constrain(sketch, kind, &items, value)?;
    if remove_redundant {
        remove_superseded(sketch, &made);
    }
    Ok(json!(made))
}

fn set_value(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let sketch = &mut feature.sketch;
    let id = a.id("constraint")?;
    let value = a.number("value")?;
    let constraint = sketch
        .constraints
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| CommandError::bad("constraint", "is not in this sketch"))?;
    if crate::sketch::dimension_value(&constraint.kind).is_none() {
        return Err(CommandError::bad("constraint", "is not a dimension"));
    }
    constraint.kind = crate::sketch::with_dimension_value(&constraint.kind, value);
    if let Some(driving) = a.opt_bool("driving")? {
        constraint.driving = driving;
    }
    Ok(Value::Null)
}

fn set_constraint(
    a: &Args,
    sketch_id: FeatureId,
    feature: &mut SketchFeature,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let sketch = &mut feature.sketch;
    let items = ids(args.get("items"), "items", sketch)?;
    let flags = ConstraintFlags {
        driving: a.opt_bool("driving")?,
        active: a.opt_bool("active")?,
        parked: a.opt_bool("parked")?,
    };
    for id in &items {
        let Some(c) = sketch.constraints.iter().find(|c| c.id == *id) else {
            return Err(CommandError::bad(
                "items",
                format!("has {id}, which is not a constraint"),
            ));
        };
        if flags.driving == Some(false) && crate::sketch::dimension_value(&c.kind).is_none() {
            return Err(CommandError::bad(
                "driving",
                format!("cannot be false for {id}: only a dimension can be a reference"),
            ));
        }
    }
    let (switch, arc_switch) = match a.opt_string("as")? {
        None => (None, None),
        Some("radius") => (Some(false), None),
        Some("diameter") => (Some(true), None),
        Some("length") => (None, Some(false)),
        Some("angle") => (None, Some(true)),
        Some(_) => {
            return Err(CommandError::bad(
                "as",
                "must be radius, diameter, length or angle",
            ));
        }
    };
    set_constraints(sketch, &items, flags);
    if let Some(angle) = arc_switch {
        for id in &items {
            if !switch_arc_dimension(ctx.document, sketch_id, sketch, *id, angle) {
                return Err(CommandError::bad(
                    "as",
                    format!("length and angle apply to an arc's dimension, not {id}"),
                ));
            }
        }
    }
    if let Some(diameter) = switch {
        let circular = |id: &Uuid| {
            sketch.constraints.iter().any(|c| {
                c.id == *id
                    && matches!(
                        c.kind,
                        ConstraintKind::Radius { .. } | ConstraintKind::Diameter { .. }
                    )
            })
        };
        if let Some(id) = items.iter().find(|id| !circular(id)) {
            return Err(CommandError::bad(
                "as",
                format!("applies to radius and diameter dimensions, not {id}"),
            ));
        }
        for id in &items {
            switch_dimension(ctx.document, sketch_id, sketch, *id, diameter);
        }
    }
    Ok(Value::Null)
}

fn remove_axis_alignment(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let sketch = &mut feature.sketch;
    let items: std::collections::HashSet<Uuid> = ids(args.get("items"), "items", sketch)?
        .into_iter()
        .collect();
    Ok(json!(crate::tools::remove_axis_alignment(sketch, &items)))
}

/// Set the driving, active and parked flags of `items`, where given.
pub(crate) fn set_constraints(sketch: &mut Sketch, items: &[Uuid], flags: ConstraintFlags) {
    for c in &mut sketch.constraints {
        if items.contains(&c.id) {
            if let Some(driving) = flags.driving {
                c.driving = driving;
            }
            if let Some(active) = flags.active {
                c.active = active;
            }
            if let Some(parked) = flags.parked {
                c.parked = parked;
            }
        }
    }
}

/// The flags `sketch.set_constraint` sets; `None` leaves one as it is.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ConstraintFlags {
    pub driving: Option<bool>,
    pub active: Option<bool>,
    pub parked: Option<bool>,
}

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

/// Take away the older constraints the constraints `made` (their ids, as
/// [`constrain`] returns them) made redundant; returns how many went.
pub(crate) fn remove_superseded(sketch: &mut Sketch, made: &[String]) -> usize {
    let new: Vec<Uuid> = made.iter().filter_map(|id| id.parse().ok()).collect();
    let gone = crate::solver::superseded(sketch, &new);
    sketch.constraints.retain(|c| !gone.contains(&c.id));
    gone.len()
}
