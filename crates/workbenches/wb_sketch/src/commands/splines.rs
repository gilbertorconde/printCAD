//! B-splines: converting curves, degree, knots, weights and joining curves
//! into one.

use core_document::{
    Args, CommandError, CommandResult, CommandSpec, CommandTable, FeatureId, ParamKind,
    WorkbenchRuntimeContext,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::sketch::Sketch;

use super::{editing, ids, ids_of, made_since, points, reading, sketch};
use crate::SketchWorkbench;
use crate::feature::SketchFeature;

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<SketchWorkbench>) {
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.to_bspline",
            "Make lines, arcs, circles, ellipses and conics into splines that are exactly them",
        ))
        .param("items", ParamKind::List, "The curves to make splines of")
        .returns("{elements}: what it made"),
        editing(to_bspline),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.spline_degree",
            "Raise or lower the degree of splines: raising keeps the curve, lowering fits the \
             nearest one",
        ))
        .param("items", ParamKind::List, "The splines")
        .param("by", ParamKind::Integer, "1 to raise, -1 to lower"),
        editing(spline_degree),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.insert_knot",
            "Insert a knot into a spline where it passes nearest a point, the curve unchanged",
        ))
        .param("spline", ParamKind::Id, "The spline")
        .param("at", ParamKind::List, "A point near the curve, {x, y}"),
        editing(insert_knot),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.knot_multiplicity",
            "Set how many times a spline's knot stands (1 up to the degree), or remove it with 0",
        ))
        .param("spline", ParamKind::Id, "The spline")
        .param(
            "knot",
            ParamKind::Number,
            "The knot's value, as sketch.spline_knots lists it",
        )
        .param("multiplicity", ParamKind::Integer, ""),
        editing(knot_multiplicity),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.spline_knots",
            "A spline's knots inside its ends and how many times each stands",
        ))
        .param("spline", ParamKind::Id, "The spline")
        .returns("{{knot, multiplicity}}")
        .read_only(),
        reading(spline_knots),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.spline_weight",
            "Weigh a spline's control point: more pulls the curve toward it",
        ))
        .param("spline", ParamKind::Id, "The spline")
        .param("point", ParamKind::Id, "One of its control points")
        .param("weight", ParamKind::Number, "More than 0; 1 is plain"),
        editing(spline_weight),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.join",
            "Merge curves that meet end to end into one B-spline following them",
        ))
        .param(
            "items",
            ParamKind::List,
            "The lines, arcs, arcs of ellipses, parabolas and hyperbolas, and open splines to merge",
        )
        .optional(
            "tolerance",
            ParamKind::Number,
            "How far the spline may stray from the curves, mm (0.01)",
        )
        .returns("{elements}: what it made"),
        editing(join),
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "splines.md", include_str!("splines.md"), &[])
}

fn to_bspline(
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
    let before = ids_of(sketch);
    let effect = crate::spline_edit::to_bspline(sketch, &items);
    if !effect.changed {
        return Err(CommandError::failed(
            effect
                .log
                .unwrap_or_else(|| "nothing to convert".to_string()),
        ));
    }
    Ok(made_since(sketch, &before))
}

fn spline_degree(
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
    let by = a.number("by")? as i32;
    if by == 0 {
        return Err(CommandError::bad("by", "must be 1 or -1"));
    }
    let effect = crate::spline_edit::change_degree(sketch, &items, by.signum());
    if !effect.changed {
        return Err(CommandError::failed(
            effect
                .log
                .unwrap_or_else(|| "no degree changed".to_string()),
        ));
    }
    Ok(Value::Null)
}

fn insert_knot(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let sketch = &mut feature.sketch;
    let spline = one_id(args, "spline", sketch)?;
    let at = points(Some(&json!([args
        .get("at")
        .cloned()
        .unwrap_or(Value::Null)])))
    .map_err(|_| CommandError::bad("at", "must be {x, y}"))?[0];
    let t = crate::spline_edit::param_near(sketch, spline, at)
        .ok_or_else(|| CommandError::bad("spline", "is not a spline"))?;
    let effect = crate::spline_edit::add_knot(sketch, spline, t);
    if !effect.changed {
        return Err(CommandError::failed(
            effect.log.unwrap_or_else(|| "no knot inserted".to_string()),
        ));
    }
    Ok(Value::Null)
}

fn knot_multiplicity(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let sketch = &mut feature.sketch;
    let spline = one_id(args, "spline", sketch)?;
    let knot = a.number("knot")?;
    let multiplicity = a.number("multiplicity")?;
    if multiplicity < 0.0 {
        return Err(CommandError::bad("multiplicity", "must be 0 or more"));
    }
    let effect = if multiplicity < 1.0 {
        crate::spline_edit::remove_knot(sketch, spline, knot)
    } else {
        crate::spline_edit::set_multiplicity(sketch, spline, knot, multiplicity as usize)
    };
    if !effect.changed {
        return Err(CommandError::failed(
            effect
                .log
                .unwrap_or_else(|| "the knot is unchanged".to_string()),
        ));
    }
    Ok(Value::Null)
}

fn spline_knots(
    a: &Args,
    _sketch_id: FeatureId,
    feature: SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let sketch = &feature.sketch;
    let spline = one_id(a.0, "spline", sketch)?;
    let knots = crate::spline_edit::knots_of(sketch, spline);
    Ok(json!(
        knots
            .iter()
            .map(|(k, m)| json!({"knot": k, "multiplicity": m}))
            .collect::<Vec<_>>()
    ))
}

fn spline_weight(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let sketch = &mut feature.sketch;
    let spline = one_id(args, "spline", sketch)?;
    let point = one_id(args, "point", sketch)?;
    let effect = crate::spline_edit::set_weight(sketch, spline, point, a.number("weight")?);
    if !effect.changed {
        return Err(CommandError::failed(
            effect
                .log
                .unwrap_or_else(|| "the weight is unchanged".to_string()),
        ));
    }
    Ok(Value::Null)
}

fn join(
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
    let tolerance = a
        .opt_number("tolerance")?
        .unwrap_or(crate::tools::JOIN_TOLERANCE);
    if tolerance <= 0.0 {
        return Err(CommandError::bad("tolerance", "must be more than 0"));
    }
    let before = ids_of(sketch);
    let effect = crate::tools::join(sketch, &items, tolerance);
    if !effect.changed {
        return Err(CommandError::failed(
            effect.log.unwrap_or_else(|| "nothing to join".to_string()),
        ));
    }
    Ok(made_since(sketch, &before))
}

/// The one id `args` gives as `name`.
fn one_id(
    args: &serde_json::Map<String, Value>,
    name: &str,
    sketch: &Sketch,
) -> Result<Uuid, CommandError> {
    let value = args
        .get(name)
        .cloned()
        .ok_or_else(|| CommandError::bad(name, "is missing"))?;
    Ok(ids(Some(&json!([value])), name, sketch)?[0])
}
