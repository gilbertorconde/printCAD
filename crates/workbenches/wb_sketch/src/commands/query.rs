//! Reading a sketch: its geometry, constraints, regions, status and walls.

use core_document::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, FeatureId,
    ParamKind, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::SketchWorkbench;
use crate::sketch::{GeometryElement, Sketch};

use super::{load, sketch};

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<SketchWorkbench>) {
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.geometry",
            "List the sketch's elements with their points",
        ))
        .returns("a list of {id, kind, points, radius?, construction}")
        .read_only()
        .note(
            "A line's points are its start and end, an arc's its centre, start and end, a \
             circle's and an ellipse's their centre; positions are as last solved, with the \
             values formulas give its dimensions (what builds, which \
             `pc.doc.feature{id = s}.fields` may not yet be). Each element also says whether \
             it is `external`.",
        )
        .note(
            "Ends and centres are listed again as elements of kind point. A spline, a \
             parabola or a hyperbola is kind \"other\" with no points: its control points \
             are in `pc.doc.feature{id = s}.fields.sketch.geometry`.",
        )
        .see_also("sketch.constraints")
        .see_also("doc.feature")
        .example(
            "Ends and centres are points of their own",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local l = pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
            pc.sketch.circle{sketch = s, x = 5, y = 5, radius = 2}
            local count = {}
            for _, e in ipairs(pc.sketch.geometry{sketch = s}) do
              count[e.kind] = (count[e.kind] or 0) + 1
              if e.id == l then assert(e.points[2][1] == 10 and e.points[2][2] == 0) end
            end
            assert(count.line == 1 and count.circle == 1)
            assert(count.point == 3, "the line's ends and the centre are points of their own")
            "#,
        ),
        evaluated(|s| Ok(describe(s))),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.constraints",
            "List the sketch's constraints",
        ))
        .returns("a list of {id, kind, items, value?}")
        .read_only()
        .note(
            "`kind` is the stored name (Coincident, Horizontal, Length, Diameter, Angle, \
             ...): \"distance\" on a line lists as Length. `value` is a dimension's, angles \
             in degrees, as its formula sets it when it has one.",
        )
        .note(
            "`items` are the element ids it ties. The origin and the axes show as fixed ids \
             ending in 0001 (origin), 0002 (x axis) and 0003 (y axis), not by name.",
        )
        .see_also("sketch.status")
        .see_also("sketch.set_constraint")
        .example(
            "An angle listed in degrees",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local a = pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
            local b = pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 10, y2 = 10}
            local made = pc.sketch.constrain{sketch = s, kind = "angle", items = {a, b}}
            local listed = pc.sketch.constraints{sketch = s}
            assert(#listed == 1 and listed[1].id == made[1])
            assert(listed[1].kind == "Angle" and math.abs(listed[1].value - 45) < 1e-4, "degrees")
            assert(listed[1].items[1] == a and listed[1].items[2] == b)
            "#,
        ),
        evaluated(|s| Ok(describe_constraints(s))),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.regions",
            "List the regions the sketch's curves close",
        ))
        .returns("a list of {name, area, x, y, default}")
        .read_only()
        .note(
            "Curves split where they cross, touch or branch, and every closed area between \
             them is a region: two overlapping circles make three, the lens and two \
             crescents. Curves with a loose end are left out.",
        )
        .note(
            "`name` stays while the sketch's dimensions change; a pad, pocket, revolution \
             or groove takes regions by it in its `regions` field. `x`, `y` is a point \
             inside the region, `area` in mm² with its holes taken out. `default` is true \
             for the regions a feature takes when it names none: a region inside another \
             loop is a hole, one inside that is material again.",
        )
        .see_also("design.pad")
        .see_also("sketch.wall_thickness")
        .example(
            "Two crossing circles close three regions",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 10}
            pc.sketch.circle{sketch = s, x = 10, y = 0, radius = 10}
            local regions = pc.sketch.regions{sketch = s}
            assert(#regions == 3)
            local lens = 2 * 100 * math.acos(0.5) - 5 * math.sqrt(300)
            local found = false
            for _, r in ipairs(regions) do
              assert(r.default, "every region is material")
              if math.abs(r.x - 5) < 1 then
                found = true
                assert(math.abs(r.area - lens) < 0.01 * lens, r.area)
              end
            end
            assert(found, "the lens sits between the centres")
            "#,
        ),
        evaluated(describe_regions),
    );
    t.add_on(
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
        .read_only()
        .note(
            "It reads the sketch's closed profile, and refuses a sketch with none (\"profile \
             is not closed\").",
        )
        .note(
            "`minimum` is the Sketcher preference when left out (0.8 mm unless changed); \
             `thin` is true when the thinnest wall is under it.",
        )
        .see_also("sketch.status")
        .example(
            "A frame with 1 mm walls",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            pc.sketch.rect{sketch = s, x = 1, y = 1, width = 18, height = 8}
            local wall = pc.sketch.wall_thickness{sketch = s, minimum = 2.5}
            assert(math.abs(wall.thinnest - 1) < 1e-4, "the frame's wall is 1 mm")
            assert(wall.thin and wall.minimum == 2.5)
            assert(not pc.sketch.wall_thickness{sketch = s, minimum = 0.5}.thin)
            "#,
        ),
        |bench, args, ctx| bench.wall_thickness_command(args, ctx),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.status",
            "How constrained the sketch is, and what conflicts",
        ))
        .returns("{dof, solved, redundant, conflicting}")
        .read_only()
        .note(
            "`dof` is the freedom left: 0 is fully constrained. A free point has 2, a free \
             circle 3 (its centre and radius); a line's freedom is its two end points'.",
        )
        .note(
            "`solved` false with ids in `conflicting` means constraints contradict; every \
             constraint taking part is listed, not only the newest. `redundant` lists those \
             that say again what others say; the sketch still solves. While `solved` is \
             false, what is built from the sketch fails at `pc.doc.rebuild()`.",
        )
        .see_also("sketch.constrain")
        .see_also("sketch.constraints")
        .example(
            "A circle constrained, then over-constrained",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local c = pc.sketch.circle{sketch = s, x = 3, y = 4, radius = 5}
            assert(pc.sketch.status{sketch = s}.dof == 3, "a centre that moves and a radius")
            pc.sketch.constrain{sketch = s, kind = "radius", items = {c}, value = 5}
            local centre = pc.sketch.geometry{sketch = s}[1].id
            pc.sketch.constrain{sketch = s, kind = "lock", items = {centre}}
            local status = pc.sketch.status{sketch = s}
            assert(status.dof == 0 and status.solved and #status.conflicting == 0)
            local other = pc.sketch.circle{sketch = s, x = 20, y = 4, radius = 6}
            pc.sketch.constrain{sketch = s, kind = "equal", items = {c, other}}
            pc.sketch.constrain{sketch = s, kind = "diameter", items = {other}, value = 12}
            status = pc.sketch.status{sketch = s}
            assert(not status.solved and #status.conflicting == 3, "radius 5, equal, diameter 12")
            "#,
        ),
        evaluated(|s| Ok(status(s))),
    );
}

/// The handler of a command that reads a sketch as it builds: its
/// formulas' values in, solved.
fn evaluated(
    read: fn(&Sketch) -> CommandResult,
) -> impl Fn(&CommandArgs, &mut WorkbenchRuntimeContext) -> CommandResult + Send + Sync {
    move |args, ctx| {
        let sketch_id = FeatureId(Args(args).id("sketch")?);
        let feature = load(ctx, sketch_id)?;
        let evaluated = crate::stored_sketch(ctx.document, sketch_id).unwrap_or(feature);
        read(&evaluated.sketch)
    }
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

/// The regions the sketch's curves close, each `{name, area, x, y,
/// default}` with a point inside it.
fn describe_regions(sketch: &Sketch) -> Result<Value, CommandError> {
    let regions =
        crate::profile::regions(sketch).map_err(|e| CommandError::failed(e.to_string()))?;
    Ok(Value::Array(
        regions
            .iter()
            .map(|r| {
                json!({
                    "name": r.name,
                    "area": r.area,
                    "x": r.inside[0],
                    "y": r.inside[1],
                    "default": r.default,
                })
            })
            .collect(),
    ))
}

fn status(sketch: &Sketch) -> Value {
    let mut solved = sketch.clone();
    let outcome = crate::solver::solve(&mut solved);
    let diagnosis = crate::solver::diagnose(sketch);
    let list = |ids: &[Uuid]| ids.iter().map(Uuid::to_string).collect::<Vec<_>>();
    json!({
        "dof": diagnosis.dof,
        "solved": !matches!(outcome, crate::operations::SolveOutcome::NotConverged { .. }),
        "redundant": list(&diagnosis.redundant),
        "conflicting": list(&diagnosis.conflicting),
    })
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
