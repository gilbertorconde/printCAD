//! Constraints: adding them, setting a dimension's value, switching a
//! constraint's flags and removing axis alignments.

use core_document::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, FeatureId, ParamKind,
    WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::sketch::{ConstraintKind, Sketch};

use super::{constrain, ids, load, sketch};

/// Whether `id` is one of these commands.
pub(super) fn owns(id: &str) -> bool {
    matches!(
        id,
        "sketch.constrain"
            | "sketch.set_value"
            | "sketch.set_constraint"
            | "sketch.remove_axis_alignment"
    )
}

/// These commands' specs.
pub(super) fn configure(specs: &mut Vec<CommandSpec>) {
    specs.push(
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
        .returns("the new constraints' ids")
        .note(
            "Points are elements of their own: a line's ends are the point ids at its \
             `start` and `end` in `pc.doc.feature{id = s}.fields.sketch.geometry`, the line's \
             own id is the line. A point made with `sketch.point` before the line is the same \
             id as the end drawn on it.",
        )
        .note(
            "Without `value` a dimension takes what it measures on the sketch as it stands. \
             Angles are degrees, a diameter the diameter, a radius the radius.",
        )
        .note(
            "\"distance\" on one line is its length (listed as Length), on two points the \
             distance between them, on a point and a curve or two curves the gap. \
             \"dimension\" picks as the toolbar does: a line's length, a circle's diameter, \
             an arc's radius, two lines' angle (their distance when parallel).",
        )
        .note(
            "Nothing stays put until constrained: a dimension on free geometry moves every \
             item it names, so tie a corner to \"origin\" first to keep it where it was \
             drawn.",
        )
        .note(
            "\"lock\" holds a point by its distances along X and Y from the origin, each \
             the size of a coordinate (a point at x = -5 takes 5 and stays at -5).",
        )
        .note(
            "\"group\" makes its items one rigid unit, keeping the shape they have now: it \
             moves and turns as a whole, three degrees of freedom, and a drag of any member \
             carries all of it. Constraints among the members alone that say nothing of the \
             axes rest while the group holds; of those that do (horizontal, vertical, an angle \
             to an axis), the first turns the whole group and the rest rest.",
        )
        .note(
            "A constraint that contradicts others is still added; `pc.sketch.status` names \
             the conflict. While it stands the sketch does not solve, gives no profile, and \
             what is built from it fails at `pc.doc.rebuild()` (\"the sketch does not solve: \
             its constraints conflict\"). `remove_redundant` takes away only older \
             constraints the new one repeats, never one it contradicts.",
        )
        .note(
            "A kind that does not fit the items is refused (\"the ... constraint does not \
             fit these items\"), one that is no kind at all is refused with the list of \
             kinds, and so is a `value` for a kind that takes none.",
        )
        .see_also("sketch.status")
        .see_also("sketch.set_value")
        .see_also("sketch.constraints")
        .example(
            "A plate fully constrained from its corner on the origin",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local corner = pc.sketch.point{sketch = s, x = 1, y = 1}
            local sides = pc.sketch.rect{sketch = s, x = 1, y = 1, width = 20, height = 10}
            pc.sketch.constrain{sketch = s, kind = "coincident", items = {corner, "origin"}}
            pc.sketch.constrain{sketch = s, kind = "distance", items = {sides[1]}, value = 40}
            pc.sketch.constrain{sketch = s, kind = "distance", items = {sides[2]}, value = 25}
            assert(pc.sketch.status{sketch = s}.dof == 0, "fully constrained")
            local pad = pc.design.pad{sketch = s, length = 2}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - 40 * 25 * 2) < 1e-3)
            assert(math.abs(m.min[1]) < 1e-6 and math.abs(m.min[2]) < 1e-6, "its corner on the origin")
            "#,
        )
        .example(
            "A rectangle grouped and dragged as one",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local sides = pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            pc.sketch.constrain{sketch = s, kind = "group", items = sides}
            local status = pc.sketch.status{sketch = s}
            assert(status.dof == 2, "square to the axes, free to slide")
            assert(#status.redundant == 0, "its sides' alignments rest in the group")
            pc.sketch.drag{sketch = s, items = {sides[1]}, by = {5, 3}}
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - 20 * 10) < 1e-2, "the same rectangle")
            assert(math.abs(m.min[1] - 5) < 1e-4 and math.abs(m.min[2] - 3) < 1e-4, "moved whole")
            "#,
        ),
    );
    specs.push(
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
        )
        .note(
            "`constraint` is an id `sketch.constrain` returned; an element's id, or a \
             constraint that is not a dimension, is refused.",
        )
        .note("Angles are degrees; a diameter takes the diameter, a radius the radius.")
        .note(
            "To bind a dimension to a formula, `doc.set_formula` takes it by the key \
             `doc.parameters` lists for the sketch, which is the constraint's id.",
        )
        .see_also("sketch.constrain")
        .see_also("doc.parameters")
        .see_also("doc.set_formula")
        .example(
            "A circle's diameter changed after it was dimensioned",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local c = pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 5}
            local d = pc.sketch.constrain{sketch = s, kind = "diameter", items = {c}}
            pc.sketch.set_value{sketch = s, constraint = d[1], value = 20}
            assert(pc.sketch.constraints{sketch = s}[1].value == 20)
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - math.pi * 10 ^ 2) < 0.01, m.volume)
            "#,
        ),
    );
    specs.push(
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
        )
        .note(
            "`items` are constraint ids; an element's id among them is refused. Only the \
             flags given change, and `driving = false` is refused for a constraint that is \
             not a dimension.",
        )
        .note(
            "A dimension that is not driving measures and conflicts with nothing; a \
             constraint that is not active is kept but left out of solving.",
        )
        .note(
            "`as` turns a radius into a diameter or back, keeping the constraint and the size \
             it holds: its value doubles or halves, as does what a formula on it gives.",
        )
        .see_also("sketch.set_value")
        .see_also("sketch.status")
        .example(
            "A repeated dimension made a reference",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local sides = pc.sketch.rect{sketch = s, x = 0, y = 0, width = 30, height = 15}
            pc.sketch.constrain{sketch = s, kind = "distance", items = {sides[1]}, value = 30}
            local top = pc.sketch.constrain{sketch = s, kind = "distance", items = {sides[3]}}
            assert(#pc.sketch.status{sketch = s}.redundant > 0, "the top's length says the bottom's again")
            pc.sketch.set_constraint{sketch = s, items = top, driving = false}
            assert(#pc.sketch.status{sketch = s}.redundant == 0, "a reference only measures")
            "#,
        ),
    );
    specs.push(
        sketch(CommandSpec::new(
            "sketch.remove_axis_alignment",
            "Turn the horizontal and vertical constraints of lines into parallel and \
             perpendicular ones among them, so the group keeps its shape and turns as a whole",
        ))
        .param("items", ParamKind::List, "The lines")
        .returns("how many constraints changed")
        .note(
            "It returns how many horizontal and vertical constraints went; one fewer \
             parallel or perpendicular constraint takes their place, so the group gains the \
             freedom to turn. Lines with none answer 0.",
        )
        .see_also("sketch.constrain")
        .see_also("sketch.status")
        .example(
            "A rectangle freed to turn",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local sides = pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            assert(pc.sketch.status{sketch = s}.dof == 4)
            assert(pc.sketch.remove_axis_alignment{sketch = s, items = sides} == 4)
            for _, c in ipairs(pc.sketch.constraints{sketch = s}) do
              assert(c.kind == "Parallel" or c.kind == "Perpendicular", c.kind)
            end
            assert(pc.sketch.status{sketch = s}.dof == 5, "still a rectangle, free to turn")
            "#,
        ),
    );
}

/// Run command `id`, one of these.
pub(super) fn run(
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let sketch_id = FeatureId(a.id("sketch")?);
    let mut feature = load(ctx, sketch_id)?;
    let sketch = &mut feature.sketch;
    let answer = match id {
        "sketch.constrain" => {
            let kind = a.string("kind")?;
            let items = ids(args.get("items"), "items", sketch)?;
            let value = a.opt_number("value")?;
            let remove_redundant = a.opt_bool("remove_redundant")?.unwrap_or(false);
            let made = constrain(sketch, kind, &items, value)?;
            if remove_redundant {
                remove_superseded(sketch, &made);
            }
            json!(made)
        }
        "sketch.set_value" => {
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
            Value::Null
        }
        "sketch.set_constraint" => {
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
                if flags.driving == Some(false) && crate::sketch::dimension_value(&c.kind).is_none()
                {
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
            Value::Null
        }
        "sketch.remove_axis_alignment" => {
            let items: std::collections::HashSet<Uuid> = ids(args.get("items"), "items", sketch)?
                .into_iter()
                .collect();
            json!(crate::tools::remove_axis_alignment(sketch, &items))
        }
        _ => return Err(CommandError::Unknown(id.to_string())),
    };
    crate::solver::solve(sketch);
    ctx.document
        .update_feature_data(sketch_id, feature.to_json())
        .map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document.mark_feature_dirty(sketch_id);
    Ok(answer)
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
