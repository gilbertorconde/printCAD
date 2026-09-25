//! The sketcher's commands: make a sketch and draw in it by numbers, for
//! scripts and other callers that are not a click.
//!
//! Coordinates are the sketch's own, in millimetres. A drawn end that lands
//! exactly on a point the sketch already has takes that point, as a
//! snapped click does, so lines drawn end to end close a profile.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, DatumFeature, DatumShape,
    FeatureId, ParamKind, WorkbenchContext, WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::feature::DatumSupport;
use crate::feature::SketchFeature;
use crate::sketch::{Arc, Circle, GeometryElement, Line, Point, Sketch, SketchPlane, Vec2D};

/// Register every command this module runs.
pub fn register(context: &mut WorkbenchContext) {
    let sketch = |spec: CommandSpec| spec.param("sketch", ParamKind::Id, "The sketch to draw in");
    context.register_command(
        CommandSpec::new("sketch.new", "Make an empty sketch on a base plane")
            .optional(
                "body",
                ParamKind::Id,
                "The body it belongs to; the selected body, else a new one",
            )
            .optional("plane", ParamKind::String, "XY (the default), XZ or YZ")
            .optional(
                "offset",
                ParamKind::Number,
                "How far along the plane's normal it sits",
            )
            .optional("name", ParamKind::String, "Its name in the tree")
            .optional(
                "on",
                ParamKind::Id,
                "A datum plane, or a coordinate system whose XY, XZ or YZ plane (see plane) it takes",
            )
            .optional(
                "normal",
                ParamKind::List,
                "A plane of its own instead: its normal as {x, y, z}",
            )
            .optional(
                "origin",
                ParamKind::List,
                "With normal: where the plane's origin sits, {x, y, z}",
            )
            .optional(
                "x_axis",
                ParamKind::List,
                "With normal: the sketch's X direction, {x, y, z}",
            )
            .returns("the sketch's id"),
    );
    context.register_command(
        sketch(CommandSpec::new("sketch.point", "Add a point"))
            .param("x", ParamKind::Number, "")
            .param("y", ParamKind::Number, "")
            .returns("the point's id"),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.line",
            "Add a line from (x1, y1) to (x2, y2)",
        ))
        .param("x1", ParamKind::Number, "")
        .param("y1", ParamKind::Number, "")
        .param("x2", ParamKind::Number, "")
        .param("y2", ParamKind::Number, "")
        .returns("the line's id"),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.polyline",
            "Add lines through a list of points, each ending where the next starts",
        ))
        .param("points", ParamKind::List, "Points as {x, y} pairs")
        .optional(
            "closed",
            ParamKind::Bool,
            "Join the last point to the first",
        )
        .returns("the lines' ids"),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.rect",
            "Add a rectangle from its corner (x, y), its width and its height",
        ))
        .param("x", ParamKind::Number, "")
        .param("y", ParamKind::Number, "")
        .param("width", ParamKind::Number, "")
        .param("height", ParamKind::Number, "")
        .returns("the four lines' ids"),
    );
    context.register_command(
        sketch(CommandSpec::new("sketch.circle", "Add a circle"))
            .param("x", ParamKind::Number, "The centre")
            .param("y", ParamKind::Number, "The centre")
            .param("radius", ParamKind::Number, "")
            .returns("the circle's id"),
    );
    context.register_command(
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
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.geometry",
            "List the sketch's elements with their points",
        ))
        .returns("a list of {id, kind, points, radius?, construction}")
        .read_only(),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.constrain",
            "Constrain elements, as the constraint's toolbar button does for a selection",
        ))
        .param(
            "kind",
            ParamKind::String,
            "coincident, point_on_object, midpoint, horizontal, vertical, parallel, \
             perpendicular, tangent, equal, symmetric, block, lock, dimension, distance, \
             distance_x, distance_y, radius, diameter, angle, angle_x or angle_y",
        )
        .param(
            "items",
            ParamKind::List,
            "Element ids, or \"origin\", \"x_axis\" and \"y_axis\"",
        )
        .optional(
            "value",
            ParamKind::Number,
            "A dimension's value (mm, or degrees for an angle); the measured one when left out",
        )
        .returns("the new constraints' ids"),
    );
    context.register_command(
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
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.draw",
            "Run a drawing or editing tool over points of the sketch, as clicks there would",
        ))
        .param(
            "tool",
            ParamKind::String,
            "line, polyline, rect, rect_center, rect_rounded, circle, circle3, arc, arc3, \
             ellipse, ellipse3, ellipse_arc, bspline, polygon, slot, arc_slot, point, fillet, \
             chamfer, trim, extend, split, offset, translate, rotate, scale or mirror",
        )
        .param(
            "points",
            ParamKind::List,
            "The clicks, each {x, y}, or {x = , y = , typed = {length = 20}, constrain = true} \
             with values typed at it; \"arc\" and \"line\" switch a polyline, \"finish\" \
             ends a spline",
        )
        .optional(
            "tolerance",
            ParamKind::Number,
            "How close a click snaps onto points and curves, mm (0.001)",
        )
        .optional(
            "params",
            ParamKind::Any,
            "Tool settings: polygon_sides, slot_width, fillet_radius, chamfer_length, \
             offset_distance, copies, bspline_periodic, auto_constraints, array_rows, \
             array_cols, array_dx, array_dy",
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
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.drag",
            "Drag elements by a step, the rest of the sketch following its constraints",
        ))
        .param("items", ParamKind::List, "The elements to drag")
        .param("by", ParamKind::List, "The step, {x, y}"),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.set_plane",
            "Move the sketch onto another plane, its geometry kept in its own coordinates",
        ))
        .param("normal", ParamKind::List, "The plane's normal, {x, y, z}")
        .optional("origin", ParamKind::List, "Its origin, {x, y, z}")
        .optional(
            "x_axis",
            ParamKind::List,
            "The sketch's X direction, {x, y, z}",
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.array",
            "Repeat elements in rows and columns",
        ))
        .param("items", ParamKind::List, "The elements to repeat")
        .param("rows", ParamKind::Integer, "")
        .param("cols", ParamKind::Integer, "")
        .param("dx", ParamKind::Number, "The step between columns, mm")
        .param("dy", ParamKind::Number, "The step between rows, mm")
        .returns("{elements}: what it made"),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.set_constraint",
            "Make constraints driving or reference, active or not",
        ))
        .param("items", ParamKind::List, "The constraints")
        .optional(
            "driving",
            ParamKind::Bool,
            "false: a reference dimension that only measures",
        )
        .optional("active", ParamKind::Bool, "false: kept but not solved"),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.mirror_sketch",
            "A new sketch on the same plane: this one's geometry mirrored across its Y axis",
        ))
        .returns("the new sketch's id"),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.merge",
            "A new sketch holding this one's geometry and other sketches', mapped onto its plane",
        ))
        .param("with", ParamKind::List, "The other sketches")
        .returns("the new sketch's id"),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.carbon_copy",
            "Copy another sketch's geometry into this one, mapped onto its plane",
        ))
        .param("from", ParamKind::Id, "The sketch to copy")
        .returns("{elements, constraints}: what it made"),
    );
    context.register_command(
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
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.external",
            "Project edges of solids into the sketch as fixed references",
        ))
        .param(
            "edges",
            ParamKind::List,
            "Each {body, point, direction}: a point on the edge and its direction, \
             in the body's own frame",
        )
        .returns("{elements}: what it made"),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.constraints",
            "List the sketch's constraints",
        ))
        .returns("a list of {id, kind, items, value?}")
        .read_only(),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.wall_thickness",
            "How thin the sketch's closed profile gets, for printing",
        ))
        .optional(
            "minimum",
            ParamKind::Number,
            "The thinnest wall that prints, mm; the Sketcher preference when left out",
        )
        .returns(
            "{thinnest, where = {x, y}, minimum, thin, regions}: the thinnest wall in mm, \
             where it is, whether it is under the minimum, and each region's own",
        )
        .read_only(),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.status",
            "How constrained the sketch is, and what conflicts",
        ))
        .returns("{dof, solved, redundant, conflicting}")
        .read_only(),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.delete",
            "Delete elements or constraints, and what depends on them",
        ))
        .param("items", ParamKind::List, "Element or constraint ids"),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.construction",
            "Make elements construction geometry, or normal again",
        ))
        .param("items", ParamKind::List, "Element ids")
        .optional("on", ParamKind::Bool, "true (the default) or false"),
    );
}

/// Run command `id`, or say it is not one of these.
pub fn run(id: &str, args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    if id == "sketch.new" {
        return new_sketch(&a, ctx);
    }
    let sketch_id = FeatureId(a.id("sketch")?);
    let mut feature = load(ctx, sketch_id)?;
    match id {
        "sketch.mirror_sketch" => {
            return mirror_sketch(ctx.document, sketch_id).map(|id| json!(id.0.to_string()));
        }
        "sketch.merge" => {
            let with = feature_ids(args.get("with"), "with")?;
            return merge(ctx.document, sketch_id, &with).map(|id| json!(id.0.to_string()));
        }
        "sketch.set_plane" => {
            let plane = custom_plane(&a)?;
            feature.plane = plane;
            feature.sketch.plane = plane;
            // A plane set outright leaves the datum the sketch followed.
            if feature.support.take().is_some() {
                ctx.document
                    .set_feature_dependencies(sketch_id, feature.dependencies());
            }
            return save(ctx, sketch_id, feature, Value::Null);
        }
        "sketch.carbon_copy" => {
            let from = FeatureId(a.id("from")?);
            let before = ids_of(&feature.sketch);
            carbon_copy(ctx.document, sketch_id, &mut feature.sketch, from)
                .map_err(CommandError::failed)?;
            let made = made_since(&feature.sketch, &before);
            return save(ctx, sketch_id, feature, made);
        }
        "sketch.external" => {
            let edges = external_sources(args.get("edges"))?;
            let before = ids_of(&feature.sketch);
            let placed = crate::placed_plane(
                &feature.plane,
                &crate::sketch_placement(ctx.document, sketch_id),
            );
            let added = add_external(ctx, &placed, &mut feature.sketch, &edges)
                .map_err(CommandError::failed)?;
            if added == 0 {
                return Err(CommandError::failed("no edge could be projected"));
            }
            let made = made_since(&feature.sketch, &before);
            return save(ctx, sketch_id, feature, made);
        }
        _ => {}
    }
    let sketch = &mut feature.sketch;
    let answer = match id {
        "sketch.point" => {
            let p = point_at(sketch, vec(a.number("x")?, a.number("y")?));
            json!(p.to_string())
        }
        "sketch.line" => {
            let from = vec(a.number("x1")?, a.number("y1")?);
            let to = vec(a.number("x2")?, a.number("y2")?);
            json!(line(sketch, from, to)?.to_string())
        }
        "sketch.polyline" => {
            let points = points(args.get("points"))?;
            let closed = a.opt_bool("closed")?.unwrap_or(false);
            json!(polyline(sketch, &points, closed)?)
        }
        "sketch.rect" => {
            let (x, y) = (a.number("x")?, a.number("y")?);
            let (w, h) = (a.number("width")?, a.number("height")?);
            if w == 0.0 || h == 0.0 {
                return Err(CommandError::failed(
                    "a rectangle needs a width and a height",
                ));
            }
            let corners = [vec(x, y), vec(x + w, y), vec(x + w, y + h), vec(x, y + h)];
            json!(polyline(sketch, &corners, true)?)
        }
        "sketch.circle" => {
            let radius = positive(&a, "radius")?;
            let centre = point_at(sketch, vec(a.number("x")?, a.number("y")?));
            let id = sketch.add_geometry(GeometryElement::Circle(Circle::new(centre, radius)));
            json!(id.to_string())
        }
        "sketch.arc" => {
            let radius = positive(&a, "radius")?;
            let (x, y) = (a.number("x")?, a.number("y")?);
            let at = |deg: f64| {
                let t = deg.to_radians();
                vec(
                    x + f64::from(radius) * t.cos(),
                    y + f64::from(radius) * t.sin(),
                )
            };
            let (start, end) = (a.number("start")?, a.number("end")?);
            let centre = point_at(sketch, vec(x, y));
            let s = point_at(sketch, at(start));
            let e = point_at(sketch, at(end));
            let id = sketch.add_geometry(GeometryElement::Arc(Arc::new(centre, s, e, radius)));
            json!(id.to_string())
        }
        "sketch.geometry" => return Ok(describe(sketch)),
        "sketch.constraints" => return Ok(describe_constraints(sketch)),
        "sketch.status" => return Ok(status(sketch)),
        "sketch.constrain" => {
            let kind = a.string("kind")?;
            let items = ids(args.get("items"), "items", sketch)?;
            let value = a.opt_number("value")?;
            json!(constrain(sketch, kind, &items, value)?)
        }
        "sketch.set_value" => {
            let id = a.id("constraint")?;
            let value = a.number("value")? as f32;
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
        "sketch.delete" => {
            let items = ids(args.get("items"), "items", sketch)?;
            delete_items(sketch, &items);
            Value::Null
        }
        "sketch.draw" => {
            let (elements, constraints) = draw(sketch, &a, args)?;
            json!({"elements": elements, "constraints": constraints})
        }
        "sketch.array" => {
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
                a.number("dx")? as f32,
                a.number("dy")? as f32,
            );
            if !effect.changed {
                return Err(CommandError::failed(
                    "an array needs elements and at least two rows or columns",
                ));
            }
            made_since(sketch, &before)
        }
        "sketch.set_constraint" => {
            let items = ids(args.get("items"), "items", sketch)?;
            let (driving, active) = (a.opt_bool("driving")?, a.opt_bool("active")?);
            set_constraints(sketch, &items, driving, active);
            Value::Null
        }
        "sketch.paste" => {
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
            made_since(sketch, &before)
        }
        "sketch.drag" => {
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
            Value::Null
        }
        "sketch.construction" => {
            let on = a.opt_bool("on")?.unwrap_or(true);
            for id in ids(args.get("items"), "items", sketch)? {
                sketch.set_construction(id, on);
            }
            Value::Null
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

fn new_sketch(a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let mut plane = match a.opt_string("plane")?.unwrap_or("XY") {
        p if p.eq_ignore_ascii_case("XY") => SketchPlane::xy(),
        p if p.eq_ignore_ascii_case("XZ") => SketchPlane::xz(),
        p if p.eq_ignore_ascii_case("YZ") => SketchPlane::yz(),
        _ => return Err(CommandError::bad("plane", "must be XY, XZ or YZ")),
    };
    if a.has("normal") {
        plane = custom_plane(a)?;
    }
    let mut datum_body = None;
    let offset = a.opt_number("offset")?.unwrap_or(0.0) as f32;
    let mut support = None;
    if let Some(on) = a.opt_id("on")? {
        let (on_datum, body) = datum_support(ctx, FeatureId(on), a.opt_string("plane")?, offset)?;
        plane = on_datum.1;
        support = Some(on_datum.0);
        datum_body = body;
    } else {
        for (o, n) in plane.origin.iter_mut().zip(plane.normal) {
            *o += n * offset;
        }
    }
    let body = match a.opt_id("body")?.or(datum_body.map(|b| b.0)) {
        Some(id) => {
            let body = BodyId(id);
            if !ctx.document.bodies().iter().any(|b| b.id == body) {
                return Err(CommandError::bad("body", "is not a body of this document"));
            }
            body
        }
        None => match ctx.selected_body_id {
            Some(id) => BodyId(id),
            None => ctx.document.create_body(None),
        },
    };
    let name = match a.opt_string("name")? {
        Some(name) => name.to_string(),
        None => crate::SketchWorkbench::next_sketch_name(ctx.document),
    };
    let mut sketch = Sketch::new(name.clone());
    sketch.plane = plane;
    let mut feature = SketchFeature::new(sketch, plane);
    feature.support = support;
    let id = ctx
        .document
        .add_feature_in_body(feature, name, Some(body))
        .map_err(|e| CommandError::failed(e.to_string()))?;
    Ok(json!(id.0.to_string()))
}

fn load(ctx: &WorkbenchRuntimeContext, id: FeatureId) -> Result<SketchFeature, CommandError> {
    let data = ctx
        .document
        .get_feature_data(id)
        .ok_or_else(|| CommandError::bad("sketch", "is not a feature of this document"))?;
    SketchFeature::from_json(data).map_err(|_| CommandError::bad("sketch", "is not a sketch"))
}

fn vec(x: f64, y: f64) -> Vec2D {
    Vec2D::new(x as f32, y as f32)
}

fn positive(a: &Args, name: &str) -> Result<f32, CommandError> {
    let value = a.number(name)?;
    if value > 0.0 {
        Ok(value as f32)
    } else {
        Err(CommandError::bad(name, "must be more than zero"))
    }
}

/// The point at `at`: one the sketch has there already, else a new one.
fn point_at(sketch: &mut Sketch, at: Vec2D) -> Uuid {
    let existing = sketch.geometry.iter().find_map(|g| match g {
        GeometryElement::Point(p) if (p.position - at).to_glam().length() < 1e-5 => Some(p.id),
        _ => None,
    });
    existing.unwrap_or_else(|| sketch.add_geometry(GeometryElement::Point(Point::new(at))))
}

fn line(sketch: &mut Sketch, from: Vec2D, to: Vec2D) -> Result<Uuid, CommandError> {
    if (to - from).to_glam().length() < 1e-5 {
        return Err(CommandError::failed("a line needs two different ends"));
    }
    let (a, b) = (point_at(sketch, from), point_at(sketch, to));
    Ok(sketch.add_geometry(GeometryElement::Line(Line::new(a, b))))
}

fn polyline(
    sketch: &mut Sketch,
    points: &[Vec2D],
    closed: bool,
) -> Result<Vec<String>, CommandError> {
    if points.len() < 2 {
        return Err(CommandError::bad("points", "needs at least two points"));
    }
    let mut ids = Vec::new();
    for pair in points.windows(2) {
        ids.push(line(sketch, pair[0], pair[1])?.to_string());
    }
    if closed && points.len() > 2 {
        ids.push(line(sketch, points[points.len() - 1], points[0])?.to_string());
    }
    Ok(ids)
}

/// A sketch's place on datum `id`: a datum plane, or one of a coordinate
/// system's three (`which`, XY when left out), `offset` along its normal;
/// with the plane that puts it on now and the body the datum is in.
#[allow(clippy::type_complexity)]
fn datum_support(
    ctx: &WorkbenchRuntimeContext,
    id: FeatureId,
    which: Option<&str>,
    offset: f32,
) -> Result<((DatumSupport, SketchPlane), Option<BodyId>), CommandError> {
    let not_a_plane = || CommandError::bad("on", "is not a datum plane or coordinate system");
    let node = ctx.document.get_feature_meta(id).ok_or_else(not_a_plane)?;
    let datum = DatumFeature::from_json(&node.data).map_err(|_| not_a_plane())?;
    let plane = match datum.shape {
        DatumShape::Plane { .. } => None,
        DatumShape::CoordinateSystem { .. } => {
            let which = which.unwrap_or("XY");
            if !["XY", "XZ", "YZ"]
                .iter()
                .any(|p| p.eq_ignore_ascii_case(which))
            {
                return Err(CommandError::bad("plane", "must be XY, XZ or YZ"));
            }
            Some(which.to_ascii_uppercase())
        }
        _ => return Err(not_a_plane()),
    };
    let support = DatumSupport {
        datum: id,
        plane,
        offset,
    };
    let values = ctx
        .document
        .feature_values(id)
        .cloned()
        .unwrap_or_else(|| node.data.clone());
    let at = support.plane_from(&values).ok_or_else(not_a_plane)?;
    Ok(((support, at), node.body))
}

/// A plane from `normal`, `origin` and `x_axis`, the last two optional.
fn custom_plane(a: &Args) -> Result<SketchPlane, CommandError> {
    let normal = unit(vector3(a.0.get("normal"), "normal")?, "normal")?;
    let origin = match a.0.get("origin") {
        Some(v) if !v.is_null() => vector3(Some(v), "origin")?,
        _ => [0.0; 3],
    };
    // Any direction square to the normal serves when none is given.
    let guess = if normal[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let wanted = match a.0.get("x_axis") {
        Some(v) if !v.is_null() => vector3(Some(v), "x_axis")?,
        _ => cross(guess, normal),
    };
    let along = dot(wanted, normal);
    let x_axis = unit(
        [
            wanted[0] - along * normal[0],
            wanted[1] - along * normal[1],
            wanted[2] - along * normal[2],
        ],
        "x_axis",
    )?;
    let y_axis = cross(normal, x_axis);
    let f = |v: [f64; 3]| v.map(|c| c as f32);
    Ok(SketchPlane {
        origin: f(origin),
        normal: f(normal),
        x_axis: f(x_axis),
        y_axis: f(y_axis),
    })
}

fn vector3(value: Option<&Value>, name: &str) -> Result<[f64; 3], CommandError> {
    let bad = || CommandError::bad(name, "must be {x, y, z}");
    let v = match value {
        Some(Value::Array(v)) if v.len() == 3 => [v[0].as_f64(), v[1].as_f64(), v[2].as_f64()],
        Some(Value::Object(m)) => ["x", "y", "z"].map(|k| m.get(k).and_then(Value::as_f64)),
        _ => return Err(bad()),
    };
    Ok([
        v[0].ok_or_else(bad)?,
        v[1].ok_or_else(bad)?,
        v[2].ok_or_else(bad)?,
    ])
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(v: [f64; 3], name: &str) -> Result<[f64; 3], CommandError> {
    let length = dot(v, v).sqrt();
    if length < 1e-9 {
        return Err(CommandError::bad(
            name,
            "must not be zero, or along the normal",
        ));
    }
    Ok(v.map(|c| c / length))
}

/// Ids named in a list: element and constraint ids, and the names of the
/// sketch's origin and axes.
fn ids(value: Option<&Value>, name: &str, sketch: &Sketch) -> Result<Vec<Uuid>, CommandError> {
    let list = value
        .and_then(Value::as_array)
        .ok_or_else(|| CommandError::bad(name, "must be a list of ids"))?;
    list.iter()
        .map(|v| {
            let text = v
                .as_str()
                .ok_or_else(|| CommandError::bad(name, "must be a list of ids"))?;
            let id = match text {
                "origin" => crate::sketch::ORIGIN_ID,
                "x_axis" => crate::sketch::X_AXIS_ID,
                "y_axis" => crate::sketch::Y_AXIS_ID,
                other => Uuid::parse_str(other)
                    .map_err(|_| CommandError::bad(name, format!("has `{other}`, not an id")))?,
            };
            let known = sketch.get_geometry(id).is_some()
                || sketch.constraints.iter().any(|c| c.id == id)
                || [
                    crate::sketch::ORIGIN_ID,
                    crate::sketch::X_AXIS_ID,
                    crate::sketch::Y_AXIS_ID,
                ]
                .contains(&id);
            if known {
                Ok(id)
            } else {
                Err(CommandError::bad(
                    name,
                    format!("has {id}, which is not in this sketch"),
                ))
            }
        })
        .collect()
}

/// Add the constraints `kind` makes for `items`, at `value` when given.
/// Store an edited sketch and answer `answer`.
fn save(
    ctx: &mut WorkbenchRuntimeContext,
    id: FeatureId,
    mut feature: SketchFeature,
    answer: Value,
) -> CommandResult {
    crate::solver::solve(&mut feature.sketch);
    ctx.document
        .update_feature_data(id, feature.to_json())
        .map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document.mark_feature_dirty(id);
    Ok(answer)
}

fn ids_of(
    sketch: &Sketch,
) -> (
    std::collections::HashSet<Uuid>,
    std::collections::HashSet<Uuid>,
) {
    (
        sketch.geometry.iter().map(GeometryElement::id).collect(),
        sketch.constraints.iter().map(|c| c.id).collect(),
    )
}

/// `{elements, constraints}`: what the sketch gained since `before`.
pub(crate) fn made_since(
    sketch: &Sketch,
    before: &(
        std::collections::HashSet<Uuid>,
        std::collections::HashSet<Uuid>,
    ),
) -> Value {
    let elements: Vec<String> = sketch
        .geometry
        .iter()
        .map(GeometryElement::id)
        .filter(|id| !before.0.contains(id))
        .map(|id| id.to_string())
        .collect();
    let constraints: Vec<String> = sketch
        .constraints
        .iter()
        .map(|c| c.id)
        .filter(|id| !before.1.contains(id))
        .map(|id| id.to_string())
        .collect();
    json!({"elements": elements, "constraints": constraints})
}

/// Sketch ids named in a list.
fn feature_ids(value: Option<&Value>, name: &str) -> Result<Vec<FeatureId>, CommandError> {
    value
        .and_then(Value::as_array)
        .ok_or_else(|| CommandError::bad(name, "must be a list of sketch ids"))?
        .iter()
        .map(|v| {
            v.as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .map(FeatureId)
                .ok_or_else(|| CommandError::bad(name, "must be a list of sketch ids"))
        })
        .collect()
}

/// Set the driving and active flags of `items`, where given.
pub(crate) fn set_constraints(
    sketch: &mut Sketch,
    items: &[Uuid],
    driving: Option<bool>,
    active: Option<bool>,
) {
    for c in &mut sketch.constraints {
        if items.contains(&c.id) {
            if let Some(driving) = driving {
                c.driving = driving;
            }
            if let Some(active) = active {
                c.active = active;
            }
        }
    }
}

/// Add `clip`'s geometry to `sketch`, moved by `by`.
pub(crate) fn paste(sketch: &mut Sketch, clip: &Sketch, by: Vec2D) -> usize {
    let all: std::collections::HashSet<Uuid> =
        clip.geometry.iter().map(GeometryElement::id).collect();
    crate::tools::copy_from(
        clip,
        sketch,
        &all,
        &crate::tools::Similarity::translation(glam::Vec2::new(by.x, by.y)),
    )
}

/// Every sketch's plane where its body sits.
fn placed(document: &core_document::Document, id: FeatureId) -> Option<SketchFeature> {
    let mut feature = crate::stored_sketch(document, id)?;
    let placement = crate::sketch_placement(document, id);
    feature.plane = crate::placed_plane(&feature.plane, &placement);
    Some(feature)
}

/// Copy sketch `from`'s geometry into `target`, the geometry of sketch
/// `target_id`, mapped from its plane onto the target's (both where their
/// bodies sit). How many elements and constraints came, and the map.
pub(crate) fn carbon_copy(
    document: &core_document::Document,
    target_id: FeatureId,
    target: &mut Sketch,
    from: FeatureId,
) -> Result<(usize, usize, crate::tools::Similarity), String> {
    let onto = placed(document, target_id).ok_or("the sketch is not in this document")?;
    let source = placed(document, from).ok_or("`from` is not a sketch of this document")?;
    if from == target_id {
        return Err("a sketch cannot copy itself".to_string());
    }
    let xf = crate::plane_map(&source.plane, &onto.plane)?;
    let (count, constraints) = crate::copy_sketch_into(&source.sketch, target, &xf);
    Ok((count, constraints, xf))
}

/// A new sketch on `sketch`'s plane and body holding its geometry and
/// `with`'s, each mapped onto the plane.
pub(crate) fn merge(
    document: &mut core_document::Document,
    sketch: FeatureId,
    with: &[FeatureId],
) -> Result<FeatureId, CommandError> {
    let base =
        placed(document, sketch).ok_or_else(|| CommandError::bad("sketch", "is not a sketch"))?;
    let mut merged = Sketch::new(format!("{} merged", base.sketch.name));
    crate::copy_sketch_into(
        &base.sketch,
        &mut merged,
        &crate::tools::Similarity::translation(glam::Vec2::ZERO),
    );
    for other in with {
        let from = placed(document, *other)
            .ok_or_else(|| CommandError::bad("with", "holds something that is not a sketch"))?;
        let xf = crate::plane_map(&from.plane, &base.plane).map_err(CommandError::failed)?;
        crate::copy_sketch_into(&from.sketch, &mut merged, &xf);
    }
    new_beside(document, sketch, merged)
}

/// A new sketch on `sketch`'s plane and body: its geometry mirrored across
/// the sketch's Y axis.
pub(crate) fn mirror_sketch(
    document: &mut core_document::Document,
    sketch: FeatureId,
) -> Result<FeatureId, CommandError> {
    let base = crate::stored_sketch(document, sketch)
        .ok_or_else(|| CommandError::bad("sketch", "is not a sketch"))?;
    let mut mirrored = Sketch::new(format!("{} mirror", base.sketch.name));
    let all: std::collections::HashSet<Uuid> = base
        .sketch
        .geometry
        .iter()
        .map(GeometryElement::id)
        .collect();
    let xf = crate::tools::Similarity::mirror_about(glam::Vec2::ZERO, glam::Vec2::Y);
    crate::tools::copy_from(&base.sketch, &mut mirrored, &all, &xf);
    new_beside(document, sketch, mirrored)
}

/// Add `made` as a new sketch on `beside`'s stored plane and in its body.
fn new_beside(
    document: &mut core_document::Document,
    beside: FeatureId,
    mut made: Sketch,
) -> Result<FeatureId, CommandError> {
    let stored = crate::stored_sketch(document, beside)
        .ok_or_else(|| CommandError::bad("sketch", "is not a sketch"))?;
    let body = document.get_feature_meta(beside).and_then(|n| n.body);
    made.plane = stored.plane;
    let name = made.name.clone();
    document
        .add_feature_in_body(SketchFeature::new(made, stored.plane), name, body)
        .map_err(|e| CommandError::failed(e.to_string()))
}

/// Edges named as `{body, point, direction}`, in each body's own frame.
fn external_sources(
    value: Option<&Value>,
) -> Result<Vec<crate::sketch::ExternalSource>, CommandError> {
    let bad = || CommandError::bad("edges", "must be a list of {body, point, direction}");
    let list = value.and_then(Value::as_array).ok_or_else(bad)?;
    list.iter()
        .map(|edge| {
            let body = edge
                .get("body")
                .and_then(Value::as_str)
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(bad)?;
            let v = |name: &str| -> Result<[f32; 3], CommandError> {
                let v = vector3(edge.get(name), "edges")?;
                Ok(v.map(|c| c as f32))
            };
            Ok(crate::sketch::ExternalSource {
                body,
                point: v("point")?,
                direction: v("direction")?,
            })
        })
        .collect()
}

/// Project `edges` onto `plane` (the sketch's, where its body sits) and add
/// what they come to as external geometry. How many elements came.
pub(crate) fn add_external(
    ctx: &WorkbenchRuntimeContext,
    plane: &SketchPlane,
    sketch: &mut Sketch,
    edges: &[crate::sketch::ExternalSource],
) -> Result<usize, String> {
    let mut added = 0;
    let mut last_error = None;
    for source in edges {
        match crate::project_source(ctx, plane, source) {
            Ok(projected) => added += crate::external::add(sketch, &projected, *source),
            Err(why) => last_error = Some(why),
        }
    }
    match (added, last_error) {
        (0, Some(why)) => Err(why),
        _ => Ok(added),
    }
}

/// Named arguments from a JSON object.
pub(crate) fn args(value: Value) -> CommandArgs {
    match value {
        Value::Object(map) => map,
        _ => CommandArgs::new(),
    }
}

/// Delete elements and constraints, and what hangs on them. The origin and
/// the axes are not the sketch's to delete. How many went.
pub(crate) fn delete_items(sketch: &mut Sketch, items: &[Uuid]) -> usize {
    let (constraints, elements): (Vec<Uuid>, Vec<Uuid>) = items
        .iter()
        .copied()
        .partition(|id| sketch.constraints.iter().any(|c| c.id == *id));
    let before = sketch.constraints.len();
    sketch.constraints.retain(|c| !constraints.contains(&c.id));
    let elements: Vec<Uuid> = elements
        .into_iter()
        .filter(|id| crate::sketch::Reference::of(*id).is_none())
        .collect();
    before - sketch.constraints.len() + sketch.remove_geometry_cascade(&elements).len()
}

/// `sketch.draw`: the tool run over the points as clicks, the same step a
/// click in the viewport takes. The ids of what it made.
fn draw(
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
        tol: a.opt_number("tolerance")?.unwrap_or(1e-3) as f32,
        params: step::params_from_json(args.get("params"))
            .map_err(|e| CommandError::bad("params", e))?,
        construction: a.opt_bool("construction")?.unwrap_or(false),
        avoid_redundant: a.opt_bool("avoid_redundant")?.unwrap_or(true),
    };
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
    for event in events {
        match event {
            Value::String(word) => match word.as_str() {
                "arc" | "line" => {
                    let want = word == "arc";
                    if let crate::tools::ToolState::PolylineFrom { arc, .. } = &state
                        && *arc != want
                    {
                        crate::tools::toggle_polyline_arc(&mut state);
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
                        format!("has `{other}`; a word there is arc, line or finish"),
                    ));
                }
            },
            click => {
                let (at, typed, constrain) = click_of(click)?;
                let outcome = step::click(
                    &mut state,
                    &mut capture,
                    &tool,
                    sketch,
                    at,
                    &typed,
                    constrain,
                    &settings,
                    &selected,
                );
                // The sketch settles after every click that changes it, as
                // it does between clicks in the viewport, so the next click
                // meets the same sketch.
                if outcome.changed || outcome.added > 0 {
                    crate::solver::solve(sketch);
                }
            }
        }
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
    "circle",
    "circle3",
    "arc",
    "arc3",
    "ellipse",
    "ellipse3",
    "ellipse_arc",
    "bspline",
    "polygon",
    "slot",
    "arc_slot",
    "fillet",
    "chamfer",
    "trim",
    "extend",
    "split",
    "offset",
    "translate",
    "rotate",
    "scale",
    "mirror",
];

/// One click of `sketch.draw`: where, the values typed at it, and whether
/// they become constraints.
type Click = (Vec2D, Vec<(crate::ovp::FieldKind, f32)>, bool);

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
                typed.push((kind, v as f32));
            }
        }
        constrain = fields
            .get("constrain")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    }
    Ok((at, typed, constrain))
}

pub(crate) fn constrain(
    sketch: &mut Sketch,
    kind: &str,
    items: &[Uuid],
    value: Option<f64>,
) -> Result<Vec<String>, CommandError> {
    let selected: std::collections::HashSet<Uuid> = items.iter().copied().collect();
    let shape = crate::constrain::SelectionShape::of(sketch, &selected);
    let tool = if kind == "dimension" {
        crate::dimension_for(&shape).ok_or_else(|| {
            CommandError::failed("dimension takes a line, two points, a circle or two lines")
        })?
    } else {
        kind
    };
    let kinds = crate::constrain::kinds_for(tool, &shape, sketch).ok_or_else(|| {
        CommandError::failed(format!("the {tool} constraint does not fit these items"))
    })?;
    let mut made = Vec::new();
    for mut k in kinds {
        if let Some(value) = value {
            if crate::sketch::dimension_value(&k).is_none() {
                return Err(CommandError::bad("value", format!("{tool} takes no value")));
            }
            k = crate::sketch::with_dimension_value(&k, value as f32);
        }
        made.push(sketch.add_constraint(k).to_string());
    }
    Ok(made)
}

/// The name of a constraint kind, as it is stored.
fn kind_name(kind: &crate::sketch::ConstraintKind) -> String {
    match serde_json::to_value(kind) {
        Ok(Value::Object(m)) => m.keys().next().cloned().unwrap_or_default(),
        Ok(Value::String(s)) => s,
        _ => String::new(),
    }
}

fn describe_constraints(sketch: &Sketch) -> Value {
    Value::Array(
        sketch
            .constraints
            .iter()
            .map(|c| {
                let mut out = json!({
                    "id": c.id.to_string(),
                    "kind": kind_name(&c.kind),
                    "items": crate::sketch::constraint_refs(&c.kind).iter().map(Uuid::to_string).collect::<Vec<_>>(),
                });
                if let Some(v) = crate::sketch::dimension_value(&c.kind) {
                    out["value"] = json!(v);
                }
                out
            })
            .collect(),
    )
}

fn status(sketch: &Sketch) -> Value {
    let mut solved = sketch.clone();
    let outcome = crate::solver::solve(&mut solved);
    let diagnosis = crate::solver::diagnose(sketch);
    let list = |ids: &[Uuid]| ids.iter().map(Uuid::to_string).collect::<Vec<_>>();
    json!({
        "dof": diagnosis.dof,
        "solved": !matches!(outcome, crate::solver::SolveOutcome::NotConverged { .. }),
        "redundant": list(&diagnosis.redundant),
        "conflicting": list(&diagnosis.conflicting),
    })
}

/// A list of `{x, y}` pairs, as `{{0, 0}, {10, 0}}` or `{{x = 0, y = 0}}`.
fn points(value: Option<&Value>) -> Result<Vec<Vec2D>, CommandError> {
    let bad = || CommandError::bad("points", "must be a list of {x, y} pairs");
    let list = value.and_then(Value::as_array).ok_or_else(bad)?;
    list.iter()
        .map(|p| {
            let (x, y) = match p {
                Value::Array(xy) if xy.len() == 2 => (xy[0].as_f64(), xy[1].as_f64()),
                Value::Object(xy) => (
                    xy.get("x").and_then(Value::as_f64),
                    xy.get("y").and_then(Value::as_f64),
                ),
                _ => (None, None),
            };
            Ok(vec(x.ok_or_else(bad)?, y.ok_or_else(bad)?))
        })
        .collect()
}

fn describe(sketch: &Sketch) -> Value {
    let at = |id: Uuid| {
        sketch
            .point_position(id)
            .map(|p| json!([p.x, p.y]))
            .unwrap_or(Value::Null)
    };
    let elements: Vec<Value> = sketch
        .geometry
        .iter()
        .map(|g| {
            let (kind, points, radius) = match g {
                GeometryElement::Point(p) => ("point", vec![at(p.id)], None),
                GeometryElement::Line(l) => ("line", vec![at(l.start), at(l.end)], None),
                GeometryElement::Circle(c) => ("circle", vec![at(c.center)], Some(c.radius)),
                GeometryElement::Arc(a) => (
                    "arc",
                    vec![at(a.center), at(a.start), at(a.end)],
                    Some(a.radius),
                ),
                GeometryElement::Ellipse(e) => ("ellipse", vec![at(e.center)], None),
                _ => ("other", Vec::new(), None),
            };
            let mut out = json!({
                "id": g.id().to_string(),
                "kind": kind,
                "points": points,
                "construction": sketch.construction.contains(&g.id()),
                "external": sketch.external.contains_key(&g.id()),
            });
            if let Some(r) = radius {
                out["radius"] = json!(r);
            }
            out
        })
        .collect();
    Value::Array(elements)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::{Document, WorkbenchRuntimeContext};

    fn call(doc: &mut Document, id: &str, args: Value) -> CommandResult {
        let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        run(id, args.as_object().unwrap(), &mut ctx)
    }

    /// `sketch.new{on = datum}` draws the sketch on the datum and keeps it
    /// there: the sketch records the datum and depends on it; a plane set
    /// outright later lets it go.
    #[test]
    fn a_sketch_made_on_a_datum_keeps_to_it() {
        use core_document::{
            AttachmentOffset, BasePlane, DatumAttachment, DatumFeature, DatumShape,
        };
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let datum = doc
            .add_feature_in_body(
                DatumFeature {
                    shape: DatumShape::CoordinateSystem { size: 10.0 },
                    attachment: DatumAttachment::BasePlane(BasePlane::XY),
                    offset: AttachmentOffset {
                        translation: [0.0, 0.0, 5.0],
                        rotation_deg: 0.0,
                        flip: false,
                    },
                },
                "Frame".into(),
                Some(body),
            )
            .unwrap();
        let made = call(
            &mut doc,
            "sketch.new",
            json!({"on": datum.0.to_string(), "plane": "xz", "offset": 2.0}),
        )
        .unwrap();
        let id = FeatureId(uuid::Uuid::parse_str(made.as_str().unwrap()).unwrap());
        let feature = SketchFeature::from_json(doc.get_feature_data(id).unwrap()).unwrap();
        assert_eq!(
            feature.support,
            Some(DatumSupport {
                datum,
                plane: Some("XZ".into()),
                offset: 2.0,
            })
        );
        assert_eq!(doc.feature_tree().dependencies(id), vec![datum]);
        assert_eq!(doc.get_feature_meta(id).unwrap().body, Some(body));

        call(
            &mut doc,
            "sketch.set_plane",
            json!({"sketch": made, "normal": [0, 0, 1]}),
        )
        .unwrap();
        let feature = SketchFeature::from_json(doc.get_feature_data(id).unwrap()).unwrap();
        assert_eq!(feature.support, None);
        assert!(doc.feature_tree().dependencies(id).is_empty());
    }

    #[test]
    fn a_scripted_rectangle_is_one_closed_profile() {
        let mut doc = Document::new("t");
        let sketch = call(&mut doc, "sketch.new", json!({"plane": "XZ"})).unwrap();
        let lines = call(
            &mut doc,
            "sketch.rect",
            json!({"sketch": sketch, "x": 0, "y": 0, "width": 20, "height": 10}),
        )
        .unwrap();
        assert_eq!(lines.as_array().unwrap().len(), 4);
        let id = FeatureId(Uuid::parse_str(sketch.as_str().unwrap()).unwrap());
        let feature = SketchFeature::from_json(doc.get_feature_data(id).unwrap()).unwrap();
        let points = feature
            .sketch
            .geometry
            .iter()
            .filter(|g| matches!(g, GeometryElement::Point(_)))
            .count();
        assert_eq!(points, 4, "corners are shared");
        assert_eq!(
            crate::profile::extract_wires(&feature.sketch)
                .unwrap()
                .len(),
            1
        );
        assert!(doc.get_feature_meta(id).unwrap().body.is_some());
    }

    #[test]
    fn lines_drawn_end_to_end_share_their_points() {
        let mut doc = Document::new("t");
        let sketch = call(&mut doc, "sketch.new", json!({})).unwrap();
        for (x1, y1, x2, y2) in [(0, 0, 10, 0), (10, 0, 0, 10), (0, 10, 0, 0)] {
            call(
                &mut doc,
                "sketch.line",
                json!({"sketch": sketch, "x1": x1, "y1": y1, "x2": x2, "y2": y2}),
            )
            .unwrap();
        }
        let listed = call(&mut doc, "sketch.geometry", json!({"sketch": sketch})).unwrap();
        let points = listed
            .as_array()
            .unwrap()
            .iter()
            .filter(|g| g["kind"] == "point")
            .count();
        assert_eq!(points, 3);
    }

    fn sketch_of(doc: &Document, id: &Value) -> Sketch {
        let id = FeatureId(Uuid::parse_str(id.as_str().unwrap()).unwrap());
        SketchFeature::from_json(doc.get_feature_data(id).unwrap())
            .unwrap()
            .sketch
    }

    fn line_length(sketch: &Sketch, id: &Value) -> f32 {
        let id = Uuid::parse_str(id.as_str().unwrap()).unwrap();
        let Some(GeometryElement::Line(l)) = sketch.get_geometry(id) else {
            panic!("not a line")
        };
        let (a, b) = (
            sketch.point_position(l.start).unwrap(),
            sketch.point_position(l.end).unwrap(),
        );
        (b - a).to_glam().length()
    }

    #[test]
    fn constraints_drive_the_geometry_and_the_status_counts_them() {
        let mut doc = Document::new("t");
        let s = call(&mut doc, "sketch.new", json!({})).unwrap();
        let line = call(
            &mut doc,
            "sketch.line",
            json!({"sketch": s, "x1": 1, "y1": 1, "x2": 9, "y2": 2}),
        )
        .unwrap();
        let free = call(&mut doc, "sketch.status", json!({"sketch": s})).unwrap()["dof"]
            .as_i64()
            .unwrap();
        call(
            &mut doc,
            "sketch.constrain",
            json!({"sketch": s, "kind": "horizontal", "items": [line]}),
        )
        .unwrap();
        let dim = call(
            &mut doc,
            "sketch.constrain",
            json!({"sketch": s, "kind": "dimension", "items": [line], "value": 25}),
        )
        .unwrap();
        let sketch = sketch_of(&doc, &s);
        assert!((line_length(&sketch, &line) - 25.0).abs() < 1e-3);
        let status = call(&mut doc, "sketch.status", json!({"sketch": s})).unwrap();
        assert_eq!(status["dof"].as_i64().unwrap(), free - 2);
        assert_eq!(status["solved"], json!(true));

        call(
            &mut doc,
            "sketch.set_value",
            json!({"sketch": s, "constraint": dim[0], "value": 40}),
        )
        .unwrap();
        assert!((line_length(&sketch_of(&doc, &s), &line) - 40.0).abs() < 1e-3);
        let listed = call(&mut doc, "sketch.constraints", json!({"sketch": s})).unwrap();
        assert_eq!(listed.as_array().unwrap().len(), 2);

        // The origin takes constraints by name.
        let start = sketch_of(&doc, &s)
            .geometry
            .iter()
            .find_map(|g| match g {
                GeometryElement::Line(l) => Some(l.start),
                _ => None,
            })
            .unwrap();
        call(
            &mut doc,
            "sketch.constrain",
            json!({"sketch": s, "kind": "coincident", "items": [start.to_string(), "origin"]}),
        )
        .unwrap();
        let sketch = sketch_of(&doc, &s);
        assert!(sketch.point_position(start).unwrap().to_glam().length() < 1e-3);

        call(
            &mut doc,
            "sketch.delete",
            json!({"sketch": s, "items": [dim[0]]}),
        )
        .unwrap();
        assert_eq!(sketch_of(&doc, &s).constraints.len(), 2);
        assert!(
            call(
                &mut doc,
                "sketch.constrain",
                json!({"sketch": s, "kind": "parallel", "items": [line]})
            )
            .is_err(),
            "parallel takes two lines"
        );
    }

    #[test]
    fn a_sketch_takes_a_plane_of_its_own() {
        let mut doc = Document::new("t");
        let s = call(
            &mut doc,
            "sketch.new",
            json!({"normal": [1, 0, 0], "origin": [5, 0, 0], "x_axis": [0, 1, 0]}),
        )
        .unwrap();
        let plane = sketch_of(&doc, &s).plane;
        assert_eq!(plane.origin, [5.0, 0.0, 0.0]);
        assert_eq!(plane.x_axis, [0.0, 1.0, 0.0]);
        assert_eq!(plane.y_axis, [0.0, 0.0, 1.0]);
        assert!(call(&mut doc, "sketch.new", json!({"normal": [0, 0, 0]})).is_err());
    }

    #[test]
    fn bad_arguments_are_refused() {
        let mut doc = Document::new("t");
        let sketch = call(&mut doc, "sketch.new", json!({})).unwrap();
        assert!(
            call(
                &mut doc,
                "sketch.circle",
                json!({"sketch": sketch, "x": 0, "y": 0, "radius": -1})
            )
            .is_err()
        );
        assert!(call(&mut doc, "sketch.new", json!({"plane": "AB"})).is_err());
        let not_a_sketch = Uuid::new_v4().to_string();
        assert!(
            call(
                &mut doc,
                "sketch.point",
                json!({"sketch": not_a_sketch, "x": 0, "y": 0})
            )
            .is_err()
        );
    }
}
