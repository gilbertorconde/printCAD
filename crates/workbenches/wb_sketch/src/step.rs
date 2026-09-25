//! One step of a sketch tool, and one drag: the code a click in the
//! viewport and the `sketch.draw` / `sketch.drag` commands both run, so a
//! click and a script do the same thing.
//!
//! A click arrives here in sketch coordinates, after the viewport has
//! turned the cursor into a point on the plane (and the grid has snapped
//! it). What the click then does (snapping onto points and curves, typed
//! lengths, auto constraints, construction mode) happens here for both.

use std::collections::HashSet;

use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::ovp::{self, DimCapture, FieldKind};
use crate::sketch::{ConstraintKind, GeometryElement, Sketch, Vec2D};
use crate::tools::{self, ToolParams, ToolState};

/// How a click behaves beyond the tool itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct StepSettings {
    /// How close, in sketch units, a click snaps onto or picks geometry.
    pub tol: f32,
    pub params: ToolParams,
    /// What the tool makes is construction geometry.
    pub construction: bool,
    /// Auto constraints the solver would call redundant are dropped.
    pub avoid_redundant: bool,
}

/// What a click did.
pub(crate) struct StepOutcome {
    pub changed: bool,
    /// Constraints from typed values.
    pub added: usize,
    /// Auto constraints dropped as redundant.
    pub skipped: usize,
    pub log: Option<String>,
}

/// Run one click of `tool` at `cursor`, with `typed` values from the
/// keyboard (`constrain`: they become constraints too).
#[allow(clippy::too_many_arguments)]
pub(crate) fn click(
    state: &mut ToolState,
    capture: &mut DimCapture,
    tool: &str,
    sketch: &mut Sketch,
    cursor: Vec2D,
    typed: &[(FieldKind, f32)],
    constrain: bool,
    settings: &StepSettings,
    selected: &HashSet<Uuid>,
) -> StepOutcome {
    capture.sync(state);
    // A plain click lands where the snap cue said; typed values place it
    // themselves.
    let snapped = typed.is_empty() && crate::is_draw_tool(tool);
    let cursor = if !typed.is_empty() {
        ovp::override_cursor(state, sketch, cursor, typed)
    } else if snapped {
        tools::snap_at(state, sketch, cursor, settings.tol).pos
    } else {
        cursor
    };
    // Once snapped, the tool only finds again what the snap landed on: a
    // wider reach could pull the point on to something else.
    let tol = if snapped && settings.tol > 0.0 {
        (settings.tol * 0.01).max(1e-5)
    } else {
        settings.tol
    };
    // Which geometry existed, so construction mode flags everything a
    // drawing tool made (an id set: a tool may also remove a point). What
    // an edit makes of existing geometry (a fillet's arc, a trim's halves,
    // a copy) keeps its own kind.
    let before: Option<HashSet<Uuid>> = (settings.construction && crate::is_draw_tool(tool))
        .then(|| sketch.geometry.iter().map(GeometryElement::id).collect());
    let state_before = state.clone();
    let constraints_before: HashSet<Uuid> = sketch.constraints.iter().map(|c| c.id).collect();
    let effect = tools::handle_click(state, tool, sketch, cursor, tol, &settings.params, selected);
    if let Some(before) = before {
        let made: Vec<Uuid> = sketch
            .geometry
            .iter()
            .map(GeometryElement::id)
            .filter(|id| !before.contains(id))
            .collect();
        for id in made {
            sketch.set_construction(id, true);
        }
    }
    // A typed angle says what the segment's slant is: an auto horizontal
    // or vertical from the same click would only repeat it.
    if constrain && typed.iter().any(|(kind, _)| *kind == FieldKind::Angle) {
        sketch.constraints.retain(|c| {
            constraints_before.contains(&c.id)
                || !matches!(
                    c.kind,
                    ConstraintKind::Horizontal { .. } | ConstraintKind::Vertical { .. }
                )
        });
    }
    let before_typed: HashSet<Uuid> = sketch.constraints.iter().map(|c| c.id).collect();
    let added = ovp::apply_typed_constraints(
        capture,
        sketch,
        &state_before,
        state,
        effect.changed,
        typed,
        constrain,
    );
    // An auto constraint the solver calls redundant, typed values
    // included, adds nothing the sketch does not already enforce: it goes
    // before it lands. Typed values are what was asked for and always stay.
    let mut skipped = 0;
    let autos: Vec<Uuid> = sketch
        .constraints
        .iter()
        .map(|c| c.id)
        .filter(|id| !constraints_before.contains(id) && before_typed.contains(id))
        .collect();
    if settings.avoid_redundant && !autos.is_empty() {
        let diagnosis = crate::solver::diagnose(sketch);
        let redundant: Vec<Uuid> = autos
            .into_iter()
            .filter(|id| diagnosis.redundant.contains(id))
            .collect();
        skipped = redundant.len();
        sketch.constraints.retain(|c| !redundant.contains(&c.id));
    }
    StepOutcome {
        changed: effect.changed,
        added,
        skipped,
        log: effect.log,
    }
}

/// The points a drag of `elements` carries, where they are now: each
/// element's own points, external geometry left where its edge is.
pub(crate) fn drag_targets(sketch: &Sketch, elements: &[Uuid]) -> Vec<(Uuid, Vec2D)> {
    let external = sketch.external_ids();
    let mut points: Vec<(Uuid, Vec2D)> = Vec::new();
    for element in elements {
        for pid in crate::drag_point_ids(sketch, *element) {
            if external.contains(&pid) || points.iter().any(|(seen, _)| *seen == pid) {
                continue;
            }
            if let Some(pos) = sketch.point_position(pid) {
                points.push((pid, pos));
            }
        }
    }
    points
}

/// Carry `targets` (from [`drag_targets`]) by `delta` from where they
/// started. The caller solves, and whatever shares those points follows.
pub(crate) fn drag(sketch: &mut Sketch, targets: &[(Uuid, Vec2D)], delta: Vec2D) {
    for (id, original) in targets {
        if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(*id) {
            p.position = *original + delta;
        }
    }
}

/// A typed field's name, as `sketch.draw` takes it.
pub(crate) fn field_name(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Length => "length",
        FieldKind::Angle => "angle",
        FieldKind::Width => "width",
        FieldKind::Height => "height",
        FieldKind::Diameter => "diameter",
        FieldKind::Radius => "radius",
        FieldKind::MajorRadius => "major_radius",
        FieldKind::MinorRadius => "minor_radius",
    }
}

pub(crate) fn field_of(name: &str) -> Option<FieldKind> {
    Some(match name {
        "length" => FieldKind::Length,
        "angle" => FieldKind::Angle,
        "width" => FieldKind::Width,
        "height" => FieldKind::Height,
        "diameter" => FieldKind::Diameter,
        "radius" => FieldKind::Radius,
        "major_radius" => FieldKind::MajorRadius,
        "minor_radius" => FieldKind::MinorRadius,
        _ => return None,
    })
}

/// The tool settings that differ from the defaults, by name.
pub(crate) fn params_to_json(params: &ToolParams) -> Map<String, Value> {
    let d = ToolParams::default();
    let mut out = Map::new();
    let mut put = |name: &str, value: Value, default: Value| {
        if value != default {
            out.insert(name.to_string(), value);
        }
    };
    put(
        "polygon_sides",
        json!(params.polygon_sides),
        json!(d.polygon_sides),
    );
    put("slot_width", json!(params.slot_width), json!(d.slot_width));
    put(
        "fillet_radius",
        json!(params.fillet_radius),
        json!(d.fillet_radius),
    );
    put(
        "chamfer_length",
        json!(params.chamfer_length),
        json!(d.chamfer_length),
    );
    put(
        "offset_distance",
        json!(params.offset_distance),
        json!(d.offset_distance),
    );
    put("copies", json!(params.copies), json!(d.copies));
    put(
        "bspline_periodic",
        json!(params.bspline_periodic),
        json!(d.bspline_periodic),
    );
    put(
        "auto_constraints",
        json!(params.auto_constraints),
        json!(d.auto_constraints),
    );
    put("array_rows", json!(params.array_rows), json!(d.array_rows));
    put("array_cols", json!(params.array_cols), json!(d.array_cols));
    put("array_dx", json!(params.array_dx), json!(d.array_dx));
    put("array_dy", json!(params.array_dy), json!(d.array_dy));
    out
}

/// Tool settings from `value`, a table of names, the rest the defaults.
pub(crate) fn params_from_json(value: Option<&Value>) -> Result<ToolParams, String> {
    let mut p = ToolParams::default();
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(p);
    };
    let fields = value
        .as_object()
        .ok_or("params must be a table of tool settings")?;
    for (name, v) in fields {
        let number = || v.as_f64().ok_or(format!("params.{name} must be a number"));
        let flag = || {
            v.as_bool()
                .ok_or(format!("params.{name} must be a boolean"))
        };
        match name.as_str() {
            "polygon_sides" => p.polygon_sides = number()? as u32,
            "slot_width" => p.slot_width = number()? as f32,
            "fillet_radius" => p.fillet_radius = number()? as f32,
            "chamfer_length" => p.chamfer_length = number()? as f32,
            "offset_distance" => p.offset_distance = number()? as f32,
            "copies" => p.copies = number()? as u32,
            "bspline_periodic" => p.bspline_periodic = flag()?,
            "auto_constraints" => p.auto_constraints = flag()?,
            "array_rows" => p.array_rows = number()? as u32,
            "array_cols" => p.array_cols = number()? as u32,
            "array_dx" => p.array_dx = number()? as f32,
            "array_dy" => p.array_dy = number()? as f32,
            other => return Err(format!("params has no setting `{other}`")),
        }
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_settings_round_trip_through_their_names() {
        let params = ToolParams {
            polygon_sides: 8,
            fillet_radius: 3.5,
            auto_constraints: false,
            ..ToolParams::default()
        };
        let named = params_to_json(&params);
        assert_eq!(named.len(), 3, "only what differs: {named:?}");
        let back = params_from_json(Some(&Value::Object(named))).unwrap();
        assert_eq!(back, params);
        assert!(params_from_json(Some(&json!({"sides": 3}))).is_err());
    }
}
