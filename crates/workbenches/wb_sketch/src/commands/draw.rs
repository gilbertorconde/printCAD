//! Drawing by numbers: points, lines, polylines, rectangles, circles, arcs,
//! any tool's clicks, drags, arrays and pasted geometry.

use core_document::{
    Args, CommandError, CommandResult, CommandSpec, CommandTable, FeatureId, ParamKind,
    WorkbenchRuntimeContext,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::sketch::{Arc, Circle, GeometryElement, Sketch, Vec2D};

use super::editing;
use super::{
    draw, ids, ids_of, line, made_since, point_at, points, polyline, positive, sketch, vec,
};
use crate::SketchWorkbench;
use crate::feature::SketchFeature;

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<SketchWorkbench>) {
    t.add_free(
        sketch(CommandSpec::new("sketch.point", "Add a point"))
            .param("x", ParamKind::Number, "")
            .param("y", ParamKind::Number, "")
            .returns("the point's id"),
        editing(point),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.line",
            "Add a line from (x1, y1) to (x2, y2)",
        ))
        .param("x1", ParamKind::Number, "")
        .param("y1", ParamKind::Number, "")
        .param("x2", ParamKind::Number, "")
        .param("y2", ParamKind::Number, "")
        .returns("the line's id"),
        editing(line_command),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.polyline",
            "Add lines through a list of points, each ending where the next starts; a level \
             or upright one is held so",
        ))
        .param("points", ParamKind::List, "Points as {x, y} pairs")
        .optional(
            "closed",
            ParamKind::Bool,
            "Join the last point to the first",
        )
        .returns("the lines' ids"),
        editing(polyline_command),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.rect",
            "Add a rectangle from its corner (x, y), its width and its height, its sides held \
             level and upright",
        ))
        .param("x", ParamKind::Number, "")
        .param("y", ParamKind::Number, "")
        .param("width", ParamKind::Number, "")
        .param("height", ParamKind::Number, "")
        .returns("the four lines' ids"),
        editing(rect),
    );
    t.add_free(
        sketch(CommandSpec::new("sketch.circle", "Add a circle"))
            .param("x", ParamKind::Number, "The centre")
            .param("y", ParamKind::Number, "The centre")
            .param("radius", ParamKind::Number, "")
            .returns("the circle's id"),
        editing(circle),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.arc",
            "Add an arc, counter-clockwise from the start angle to the end angle",
        ))
        .param("x", ParamKind::Number, "The centre")
        .param("y", ParamKind::Number, "The centre")
        .param("radius", ParamKind::Number, "")
        .param(
            "start",
            ParamKind::Number,
            "Degrees from the sketch's X axis",
        )
        .param("end", ParamKind::Number, "Degrees from the sketch's X axis")
        .returns("the arc's id"),
        editing(arc),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.draw",
            "Run a drawing or editing tool over points of the sketch, as clicks there would",
        ))
        .param(
            "tool",
            ParamKind::String,
            "line, polyline, rect, rect_center, rect_rounded, rect3, rect_center3, rect_frame, \
             circle, circle3, arc, arc3, \
             ellipse, ellipse3, ellipse_arc, parabola, hyperbola, bspline, polygon, slot, arc_slot, point, fillet, \
             chamfer, trim, extend, split, bspline_knot, offset, translate, rotate, scale or mirror",
        )
        .param(
            "points",
            ParamKind::List,
            "The clicks, each {x, y}, or {x = , y = , typed = {length = 20}, constrain = true} \
             with values typed at it, or for the line tool {x = , y = , arc = true}: an arc \
             there, tangent to what ends where it draws from; \"arc\" and \"line\" switch a \
             polyline, \"finish\" ends a spline",
        )
        .optional(
            "tolerance",
            ParamKind::Number,
            "How close a click snaps onto points and curves, mm (0.001)",
        )
        .optional(
            "params",
            ParamKind::Any,
            "Tool settings: polygon_sides, slot_width, fillet_radius, chamfer_length, corner_keep, \
             offset_distance, offset_round, offset_both, offset_delete, offset_linked, copies, \
             copies_linked, copies_total, copies_symmetric, rotate_full_turn, \
             bspline_periodic, bspline_degree, bspline_interpolate, auto_constraints, \
             arc_helpers, mirror_keep, mirror_linked, \
             mirror_center, cut_at_axes",
        )
        .optional(
            "construction",
            ParamKind::Bool,
            "What it makes is construction geometry",
        )
        .optional(
            "avoid_redundant",
            ParamKind::Bool,
            "Drop auto constraints that add nothing (true)",
        )
        .optional(
            "selection",
            ParamKind::List,
            "The elements offset, translate, rotate, scale and mirror act on",
        )
        .returns("{elements, constraints}: what it made"),
        editing(draw_command),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.drag",
            "Drag elements by a step, the rest of the sketch following its constraints",
        ))
        .param("items", ParamKind::List, "The elements to drag")
        .param("by", ParamKind::List, "The step, {x, y}"),
        editing(drag),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.array",
            "Repeat elements in rows and columns",
        ))
        .param("items", ParamKind::List, "The elements to repeat")
        .param("rows", ParamKind::Integer, "")
        .param("cols", ParamKind::Integer, "")
        .param("dx", ParamKind::Number, "The step between columns, mm")
        .param("dy", ParamKind::Number, "The step between rows, mm")
        .optional(
            "linked",
            ParamKind::Bool,
            "Copies stay the originals' size, spaced by one pitch along the rows and one \
             down the columns (false)",
        )
        .returns("{elements}: what it made"),
        editing(array),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.paste",
            "Add geometry held as a sketch of its own, moved by a step",
        ))
        .param(
            "clipboard",
            ParamKind::Any,
            "The geometry, as a sketch's fields (what copying in the sketcher holds)",
        )
        .param("by", ParamKind::List, "The step, {x, y}")
        .returns("{elements}: what it made"),
        editing(paste_command),
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "draw.md", include_str!("draw.md"), &[])
}

fn point(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let sketch = &mut feature.sketch;
    let p = point_at(sketch, vec(a.number("x")?, a.number("y")?));
    Ok(json!(p.to_string()))
}

fn line_command(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let sketch = &mut feature.sketch;
    let from = vec(a.number("x1")?, a.number("y1")?);
    let to = vec(a.number("x2")?, a.number("y2")?);
    Ok(json!(line(sketch, from, to)?.to_string()))
}

fn polyline_command(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let sketch = &mut feature.sketch;
    let points = points(args.get("points"))?;
    let closed = a.opt_bool("closed")?.unwrap_or(false);
    Ok(json!(polyline(sketch, &points, closed)?))
}

fn rect(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let sketch = &mut feature.sketch;
    let (x, y) = (a.number("x")?, a.number("y")?);
    let (w, h) = (a.number("width")?, a.number("height")?);
    if w == 0.0 || h == 0.0 {
        return Err(CommandError::failed(
            "a rectangle needs a width and a height",
        ));
    }
    let corners = [vec(x, y), vec(x + w, y), vec(x + w, y + h), vec(x, y + h)];
    Ok(json!(polyline(sketch, &corners, true)?))
}

fn circle(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let sketch = &mut feature.sketch;
    let radius = positive(a, "radius")?;
    let centre = point_at(sketch, vec(a.number("x")?, a.number("y")?));
    let id = sketch.add_geometry(GeometryElement::Circle(Circle::new(centre, radius)));
    Ok(json!(id.to_string()))
}

fn arc(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let sketch = &mut feature.sketch;
    let radius = positive(a, "radius")?;
    let (x, y) = (a.number("x")?, a.number("y")?);
    let at = |deg: f64| {
        let t = deg.to_radians();
        vec(x + radius * t.cos(), y + radius * t.sin())
    };
    let (start, end) = (a.number("start")?, a.number("end")?);
    let centre = point_at(sketch, vec(x, y));
    let s = point_at(sketch, at(start));
    let e = point_at(sketch, at(end));
    let id = sketch.add_geometry(GeometryElement::Arc(Arc::new(centre, s, e, radius)));
    Ok(json!(id.to_string()))
}

fn draw_command(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let sketch = &mut feature.sketch;
    let (elements, constraints) = draw(sketch, a, args)?;
    Ok(json!({"elements": elements, "constraints": constraints}))
}

fn array(
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
    let count = |name: &str| -> Result<u32, CommandError> {
        let n = a.number(name)?;
        if n >= 1.0 {
            Ok(n as u32)
        } else {
            Err(CommandError::bad(name, "must be at least 1"))
        }
    };
    let before = ids_of(sketch);
    let effect = crate::tools::array(
        sketch,
        &items,
        count("rows")?,
        count("cols")?,
        a.number("dx")?,
        a.number("dy")?,
        a.opt_bool("linked")?.unwrap_or(false),
    );
    if !effect.changed {
        return Err(CommandError::failed(
            "an array needs elements and at least two rows or columns",
        ));
    }
    Ok(made_since(sketch, &before))
}

fn paste_command(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let sketch = &mut feature.sketch;
    let clip: Sketch =
        serde_json::from_value(args.get("clipboard").cloned().unwrap_or(Value::Null))
            .map_err(|e| CommandError::bad("clipboard", e.to_string()))?;
    let by = points(Some(&json!([args
        .get("by")
        .cloned()
        .unwrap_or(Value::Null)])))
    .map_err(|_| CommandError::bad("by", "must be {x, y}"))?[0];
    let before = ids_of(sketch);
    paste(sketch, &clip, by);
    Ok(made_since(sketch, &before))
}

fn drag(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let sketch = &mut feature.sketch;
    let items = ids(args.get("items"), "items", sketch)?;
    let by = points(Some(&json!([args
        .get("by")
        .cloned()
        .unwrap_or(Value::Null)])))
    .map_err(|_| CommandError::bad("by", "must be {x, y}"))?[0];
    let targets = crate::step::drag_targets(sketch, &items);
    crate::step::drag(sketch, &targets, by);
    let held: Vec<Uuid> = targets.iter().map(|(id, _)| *id).collect();
    crate::solver::solve_holding(sketch, &held);
    Ok(Value::Null)
}

/// Add `clip`'s geometry to `sketch`, moved by `by`.
pub(crate) fn paste(sketch: &mut Sketch, clip: &Sketch, by: Vec2D) -> usize {
    let all: std::collections::HashSet<Uuid> =
        clip.geometry.iter().map(GeometryElement::id).collect();
    crate::tools::copy_from(
        clip,
        sketch,
        &all,
        &crate::tools::Similarity::translation(glam::DVec2::new(by.x, by.y)),
    )
}
