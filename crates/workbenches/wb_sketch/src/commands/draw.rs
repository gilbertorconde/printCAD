//! Drawing command and recording playback through the viewport's tool steps.

use super::{ids, points};
use crate::sketch::{GeometryElement, Sketch, Vec2D};
use core_document::{Args, CommandArgs, CommandError};
use serde_json::{Value, json};
use uuid::Uuid;

/// `sketch.draw`: the tool run over the points as clicks, the same step a
/// click in the viewport takes. The ids of what it made.
pub(super) fn run(
    sketch: &mut Sketch,
    a: &Args,
    args: &CommandArgs,
) -> Result<(Vec<String>, Vec<String>), CommandError> {
    use crate::step;
    let name = a.string("tool")?;
    let tool = if name.starts_with("sketch.") {
        name.to_string()
    } else {
        format!("sketch.{name}")
    };
    if !DRAW_TOOLS.contains(&tool.trim_start_matches("sketch.")) {
        return Err(CommandError::bad("tool", format!("has no tool `{name}`")));
    }
    let settings = step::StepSettings {
        tol: a.opt_number("tolerance")?.unwrap_or(1e-3),
        params: step::params_from_json(args.get("params"))
            .map_err(|e| CommandError::bad("params", e))?,
        construction: a.opt_bool("construction")?.unwrap_or(false),
        avoid_redundant: a.opt_bool("avoid_redundant")?.unwrap_or(true),
    };
    let corner = args
        .get("corner")
        .map(|value| serde_json::from_value::<crate::tools::modify::CornerTarget>(value.clone()))
        .transpose()
        .map_err(|e| CommandError::bad("corner", e.to_string()))?;
    if corner.is_some() && !matches!(tool.as_str(), "sketch.fillet" | "sketch.chamfer") {
        return Err(CommandError::bad("corner", "requires fillet or chamfer"));
    }
    let selected: std::collections::HashSet<Uuid> = match args.get("selection") {
        Some(v) if !v.is_null() => ids(Some(v), "selection", sketch)?.into_iter().collect(),
        _ => Default::default(),
    };
    let events = args
        .get("points")
        .and_then(Value::as_array)
        .ok_or_else(|| CommandError::bad("points", "must be a list of clicks"))?;
    let elements_before: std::collections::HashSet<Uuid> =
        sketch.geometry.iter().map(GeometryElement::id).collect();
    let constraints_before: std::collections::HashSet<Uuid> =
        sketch.constraints.iter().map(|c| c.id).collect();
    let mut state = crate::tools::ToolState::Idle;
    let mut capture = crate::ovp::DimCapture::default();
    let shared_line = a.opt_bool("shared_line")?.unwrap_or(false);
    let shared_polyline = a.opt_bool("shared_polyline")?.unwrap_or(false);
    let shared = shared_line || shared_polyline;
    let shared_point = a.opt_bool("shared_point")?.unwrap_or(false);
    let shared_circle = a.opt_bool("shared_circle")?.unwrap_or(false);
    let shared_rect = a.opt_bool("shared_rect")?.unwrap_or(false);
    let shared_conic = a.opt_bool("shared_conic")?.unwrap_or(false);
    let scale = a.opt_number("view_scale")?.unwrap_or(1.0 / 15.0);
    if shared
        && ((shared_line && tool != "sketch.line")
            || (shared_polyline && tool != "sketch.polyline")
            || !scale.is_finite()
            || scale <= 0.)
    {
        return Err(CommandError::bad(
            if shared_polyline {
                "shared_polyline"
            } else {
                "shared_line"
            },
            "requires the matching chain tool and a positive view_scale",
        ));
    }
    let mut line = crate::line_gesture::LineGesture::default();
    if shared_point && (tool != "sketch.point" || !scale.is_finite() || scale <= 0.) {
        return Err(CommandError::bad(
            "shared_point",
            "requires the point tool and a positive view_scale",
        ));
    }
    let mut point = crate::pick_projection::SelectPick::default();
    if shared_circle && (tool != "sketch.circle" || !scale.is_finite() || scale <= 0.) {
        return Err(CommandError::bad(
            "shared_circle",
            "requires the circle tool and a positive view_scale",
        ));
    }
    let mut circle = crate::circle_gesture::CircleGesture::default();
    if shared_rect
        && (!crate::rectangle_gesture::RectangleGesture::supports(&tool)
            || !scale.is_finite()
            || scale <= 0.)
    {
        return Err(CommandError::bad(
            "shared_rect",
            "requires a plain rectangle tool and a positive view_scale",
        ));
    }
    let mut rect = crate::rectangle_gesture::RectangleGesture::default();
    let rectangle = args
        .get("rectangle")
        .map(|v| serde_json::from_value::<crate::rect_session::RectangleParameters>(v.clone()))
        .transpose()
        .map_err(|e| CommandError::bad("rectangle", e.to_string()))?;
    if rectangle.is_some() && !shared_rect {
        return Err(CommandError::bad("rectangle", "requires shared_rect"));
    }
    rect.editable = rectangle.is_some();
    let rectangle_base = rectangle.map(|_| sketch.clone());
    if shared_conic
        && (!crate::conic_gesture::ConicGesture::supports(&tool)
            || !scale.is_finite()
            || scale <= 0.)
    {
        return Err(CommandError::bad(
            "shared_conic",
            "requires a supported conic tool and a positive view_scale",
        ));
    }
    let mut conic = crate::conic_gesture::ConicGesture::default();
    conic.tool.hyperbola_input = match a.opt_string("hyperbola_input")?.unwrap_or("focus") {
        "vertex" => sketch_runtime::conic_plan::HyperbolaInput::BranchVertex,
        "focus" => sketch_runtime::conic_plan::HyperbolaInput::Focus,
        _ => {
            return Err(CommandError::bad(
                "hyperbola_input",
                "expected vertex or focus",
            ));
        }
    };
    conic.tool.placement.snap_off = settings.tol <= 0.;
    rect.tool.placement_input.snap_off = settings.tol <= 0.;
    circle.tool.placement.snap_off = settings.tol <= 0.;
    line.tool.placement.snap_off = settings.tol <= 0.;
    for event in events.iter().filter(|_| corner.is_none()) {
        match event {
            Value::String(word) => match word.as_str() {
                w if crate::tools::PolySegment::of_word(w).is_some() => {
                    if let Some(segment) = crate::tools::PolySegment::of_word(w) {
                        if shared && line.tool.can_arc() {
                            use sketch_runtime::polyline::PolylineMode;
                            line.tool.mode = match segment {
                                crate::tools::PolySegment::Line => PolylineMode::Line,
                                crate::tools::PolySegment::Tangent => PolylineMode::Tangent,
                                crate::tools::PolySegment::Perpendicular => {
                                    PolylineMode::Perpendicular
                                }
                                crate::tools::PolySegment::Reverse => PolylineMode::Reverse,
                            };
                            line.update_state(&mut state);
                        } else {
                            crate::tools::set_polyline_segment(&mut state, segment);
                        }
                    }
                }
                "finish" => {
                    let effect =
                        crate::tools::finish_click_sequence(&mut state, sketch, &settings.params);
                    if effect.changed {
                        crate::solver::solve(sketch);
                    }
                }
                other => {
                    return Err(CommandError::bad(
                        "points",
                        format!(
                            "has `{other}`; a word there is line, arc, perpendicular_arc, \
                             reverse_arc or finish"
                        ),
                    ));
                }
            },
            click => {
                let (at, typed, constrain) = click_of(click)?;
                let picked = click
                    .get("snap")
                    .map(|value| crate::snap_record::read(value, sketch, &elements_before))
                    .transpose()
                    .map_err(|e| CommandError::bad("points", e))?;
                if picked.is_some() && (!typed.is_empty() || !crate::is_draw_tool(&tool)) {
                    return Err(CommandError::bad(
                        "points",
                        "snap sources belong to an untyped drawing click",
                    ));
                }
                let arc = click.get("arc").and_then(Value::as_bool) == Some(true);
                if shared && arc && line.tool.mode == sketch_runtime::polyline::PolylineMode::Line {
                    line.begin_arc(&mut state);
                }
                let outcome = if shared {
                    step::line_gesture_click(
                        &mut line,
                        &tool,
                        &mut state,
                        &mut capture,
                        sketch,
                        at,
                        &typed,
                        constrain,
                        &settings,
                        &selected,
                        scale,
                        picked,
                    )
                } else if shared_rect {
                    step::rectangle_gesture_click(
                        &mut rect,
                        &tool,
                        &mut state,
                        &mut capture,
                        sketch,
                        at,
                        &typed,
                        constrain,
                        &settings,
                        &selected,
                        scale,
                        picked,
                    )
                } else if shared_conic {
                    step::conic_gesture_click(
                        &mut conic,
                        &tool,
                        &mut state,
                        &mut capture,
                        sketch,
                        at,
                        &typed,
                        constrain,
                        &settings,
                        &selected,
                        scale,
                        picked,
                    )
                } else if shared_circle {
                    step::circle_gesture_click(
                        &mut circle,
                        &mut state,
                        &mut capture,
                        sketch,
                        at,
                        &typed,
                        constrain,
                        &settings,
                        &selected,
                        scale,
                        picked,
                    )
                } else if shared_point {
                    step::point_placement_click(
                        &mut point,
                        &mut state,
                        &mut capture,
                        sketch,
                        at,
                        &typed,
                        constrain,
                        &settings,
                        &selected,
                        scale,
                        sketch_runtime::placement::Placement {
                            snap_off: settings.tol <= 0.,
                            ..Default::default()
                        },
                        picked,
                    )
                } else if tool == "sketch.line"
                    && click.get("arc").and_then(Value::as_bool) == Some(true)
                {
                    line.leave();
                    step::line_arc_click(
                        &mut state,
                        &mut capture,
                        sketch,
                        at,
                        &typed,
                        constrain,
                        &settings,
                        &selected,
                        picked,
                    )
                } else {
                    step::coordinate_click(
                        &mut state,
                        &mut capture,
                        &tool,
                        sketch,
                        at,
                        &typed,
                        constrain,
                        &settings,
                        &selected,
                        picked,
                    )
                };
                // The sketch settles after every click that changes it, as
                // it does between clicks in the viewport, so the next click
                // meets the same sketch.
                if outcome.changed || outcome.added > 0 {
                    crate::solver::solve(sketch);
                    if shared {
                        line.committed(sketch, &mut state, None);
                    }
                }
            }
        }
    }
    if let Some(target) = corner {
        let bevel = tool == "sketch.chamfer";
        *sketch = crate::corner_session::calculate(
            sketch,
            &target,
            bevel,
            if bevel {
                settings.params.chamfer_length
            } else {
                settings.params.fillet_radius
            },
            settings.params.corner_keep,
        )
        .map_err(|why| CommandError::bad("corner", why))?;
    }
    if let (Some(parameters), Some(base)) = (rectangle, rectangle_base) {
        if rect.tool.draft().is_none() {
            return Err(CommandError::bad(
                "rectangle",
                "requires a completed rectangle gesture",
            ));
        }
        *sketch =
            crate::rect_session::calculate(&mut rect, &base, parameters, settings.construction)
                .map_err(|why| CommandError::bad("rectangle", why))?;
    }
    let elements = sketch
        .geometry
        .iter()
        .map(GeometryElement::id)
        .filter(|id| !elements_before.contains(id))
        .map(|id| id.to_string())
        .collect();
    let constraints = sketch
        .constraints
        .iter()
        .map(|c| c.id)
        .filter(|id| !constraints_before.contains(id))
        .map(|id| id.to_string())
        .collect();
    Ok((elements, constraints))
}

/// The tools `sketch.draw` runs, without the `sketch.` prefix.
const DRAW_TOOLS: &[&str] = &[
    "point",
    "line",
    "polyline",
    "rect",
    "rect_rounded",
    "rect_center",
    "rect3",
    "rect_center3",
    "rect_frame",
    "circle",
    "circle3",
    "arc",
    "arc3",
    "ellipse",
    "ellipse3",
    "ellipse_arc",
    "parabola",
    "hyperbola",
    "bspline",
    "polygon",
    "slot",
    "arc_slot",
    "fillet",
    "chamfer",
    "trim",
    "extend",
    "split",
    "bspline_knot",
    "offset",
    "translate",
    "rotate",
    "scale",
    "mirror",
];

/// One click of `sketch.draw`: where, the values typed at it, and whether
/// they become constraints.
type Click = (Vec2D, Vec<(crate::ovp::FieldKind, f64)>, bool);

/// Read one click of `sketch.draw`.
fn click_of(click: &Value) -> Result<Click, CommandError> {
    let at = points(Some(&json!([click])))
        .map_err(|_| CommandError::bad("points", "must hold {x, y} clicks"))?[0];
    let mut typed = Vec::new();
    let mut constrain = false;
    if let Value::Object(fields) = click {
        if let Some(Value::Object(values)) = fields.get("typed") {
            for (name, v) in values {
                let kind = crate::step::field_of(name).ok_or_else(|| {
                    CommandError::bad("points", format!("types `{name}`, which no tool asks for"))
                })?;
                let v = v
                    .as_f64()
                    .ok_or_else(|| CommandError::bad("points", "typed values are numbers"))?;
                typed.push((kind, v));
            }
        }
        constrain = fields
            .get("constrain")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    }
    Ok((at, typed, constrain))
}
