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
        .read_only(),
        evaluated(|s| Ok(describe(s))),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.constraints",
            "List the sketch's constraints",
        ))
        .returns("a list of {id, kind, items, value?}")
        .read_only(),
        evaluated(|s| Ok(describe_constraints(s))),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.regions",
            "List the regions the sketch's curves close",
        ))
        .returns("a list of {name, area, x, y, default}")
        .read_only(),
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
        .read_only(),
        |bench, args, ctx| bench.wall_thickness_command(args, ctx),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.status",
            "How constrained the sketch is, and what conflicts",
        ))
        .returns("{dof, solved, redundant, conflicting}")
        .read_only(),
        evaluated(|s| Ok(status(s))),
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "query.md", include_str!("query.md"), &[])
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
