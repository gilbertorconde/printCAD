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
        .returns("a list of {id, kind, points, radius?, construction}"),
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
        .param("value", ParamKind::Number, "mm, or degrees for an angle"),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.constraints",
            "List the sketch's constraints",
        ))
        .returns("a list of {id, kind, items, value?}"),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.status",
            "How constrained the sketch is, and what conflicts",
        ))
        .returns("{dof, solved, redundant, conflicting}"),
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
            Value::Null
        }
        "sketch.delete" => {
            let items = ids(args.get("items"), "items", sketch)?;
            let (constraints, elements): (Vec<Uuid>, Vec<Uuid>) = items
                .into_iter()
                .partition(|id| sketch.constraints.iter().any(|c| c.id == *id));
            sketch.constraints.retain(|c| !constraints.contains(&c.id));
            sketch.remove_geometry_cascade(&elements);
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
    if let Some(on) = a.opt_id("on")? {
        let (frame, body) = datum_plane(ctx, FeatureId(on), a.opt_string("plane")?)?;
        plane = frame;
        datum_body = body;
    }
    if let Some(offset) = a.opt_number("offset")? {
        for (o, n) in plane.origin.iter_mut().zip(plane.normal) {
            *o += n * offset as f32;
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
    let id = ctx
        .document
        .add_feature_in_body(SketchFeature::new(sketch, plane), name, Some(body))
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

/// The plane of datum `id`: a datum plane's own, or one of a coordinate
/// system's three (`which`, XY when left out), with the body it is in.
fn datum_plane(
    ctx: &WorkbenchRuntimeContext,
    id: FeatureId,
    which: Option<&str>,
) -> Result<(SketchPlane, Option<BodyId>), CommandError> {
    let not_a_plane = || CommandError::bad("on", "is not a datum plane or coordinate system");
    let node = ctx.document.get_feature_meta(id).ok_or_else(not_a_plane)?;
    let datum = DatumFeature::from_json(&node.data).map_err(|_| not_a_plane())?;
    let frame = match datum.shape {
        DatumShape::Plane { .. } => datum.frame(),
        DatumShape::CoordinateSystem { .. } => {
            let which = which.unwrap_or("XY");
            datum
                .frame()
                .planes()
                .into_iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(which))
                .map(|(_, f)| f)
                .ok_or_else(|| CommandError::bad("plane", "must be XY, XZ or YZ"))?
        }
        _ => return Err(not_a_plane()),
    };
    let plane = SketchPlane {
        origin: frame.origin,
        normal: frame.normal,
        x_axis: frame.x_axis,
        y_axis: frame.y_axis(),
    };
    Ok((plane, node.body))
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
fn constrain(
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
