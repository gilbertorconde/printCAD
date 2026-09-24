//! The sketcher's commands: make a sketch and draw in it by numbers, for
//! scripts and other callers that are not a click.
//!
//! Coordinates are the sketch's own, in millimetres. A drawn end that lands
//! exactly on a point the sketch already has takes that point, as a
//! snapped click does, so lines drawn end to end close a profile.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, FeatureId, ParamKind,
    WorkbenchContext, WorkbenchFeature, WorkbenchRuntimeContext,
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
    if let Some(offset) = a.opt_number("offset")? {
        for (o, n) in plane.origin.iter_mut().zip(plane.normal) {
            *o += n * offset as f32;
        }
    }
    let body = match a.opt_id("body")? {
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
