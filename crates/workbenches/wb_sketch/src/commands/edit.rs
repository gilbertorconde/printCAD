//! Whole-sketch edits: solver settings, repair, restore, deleting,
//! construction, internal geometry and the section view.

use core_document::{
    Args, CommandError, CommandResult, CommandSpec, CommandTable, FeatureId, ParamKind,
    WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::feature::SketchFeature;
use crate::sketch::Sketch;

use super::{editing, ids, reading, sketch};
use crate::SketchWorkbench;

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<SketchWorkbench>) {
    t.add_free(
        sketch(
            CommandSpec::new(
                "sketch.solver_settings",
                "How far the solver goes on this sketch",
            )
            .optional(
                "iterations",
                ParamKind::Number,
                "The most steps it takes (100 when never set)",
            )
            .optional(
                "tolerance",
                ParamKind::Number,
                "How small what is left must be, against the sketch's size (1e-9 when never \
                 set)",
            ),
        ),
        editing(solver_settings),
    );
    t.add_free(
        sketch(
            CommandSpec::new(
                "sketch.repair",
                "Join ends of curves that nearly meet, and remove curves of no size, doubled \
                 curves and constraints left naming nothing",
            )
            .optional(
                "tolerance",
                ParamKind::Number,
                "How near two ends must be to join, mm (0.01 when left out)",
            )
            .returns("what was repaired, in words"),
        ),
        editing(repair_command),
    );
    t.add_free(
        sketch(
            CommandSpec::new(
                "sketch.restore",
                "Put the sketch back as `data` holds it: an editing session cancelled",
            )
            .param(
                "data",
                ParamKind::Any,
                "The sketch as doc.feature lists its data",
            ),
        ),
        editing(restore),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.delete",
            "Delete elements or constraints, and what depends on them",
        ))
        .param("items", ParamKind::List, "Element or constraint ids"),
        editing(delete),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.construction",
            "Make elements construction geometry, or normal again",
        ))
        .param("items", ParamKind::List, "Element ids")
        .optional("on", ParamKind::Bool, "true (the default) or false"),
        editing(construction),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.internal_geometry",
            "Show or hide curves' internal geometry: an ellipse's axes and foci, a parabola's \
             or hyperbola's axis and focus, a B-spline's control polygon, as construction held \
             to its curve",
        ))
        .param(
            "items",
            ParamKind::List,
            "The curves, or pieces of their internal geometry",
        )
        .optional(
            "show",
            ParamKind::Bool,
            "true makes what is missing, false takes away the pieces nothing else holds; \
             left out, it shows when a piece is missing and hides otherwise",
        )
        .returns("{shown, elements}: whether it showed, and what it made or took away"),
        editing(internal_geometry),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.section_view",
            "Cut away everything on the viewer's side of the sketch plane while it is edited",
        ))
        .optional("on", ParamKind::Bool, "true (the default) or false"),
        reading(section_view),
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "edit.md", include_str!("edit.md"), &[])
}

fn solver_settings(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    if let Some(n) = a.opt_number("iterations")? {
        if n < 1.0 {
            return Err(CommandError::bad("iterations", "must be at least 1"));
        }
        feature.sketch.solver.max_iterations = n as u32;
    }
    if let Some(t) = a.opt_number("tolerance")? {
        if !(t > 0.0 && t < 1.0) {
            return Err(CommandError::bad(
                "tolerance",
                "must be above 0 and below 1",
            ));
        }
        feature.sketch.solver.tolerance = t;
    }
    crate::solver::solve(&mut feature.sketch);
    Ok(Value::Null)
}

fn repair_command(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let tolerance = a.opt_number("tolerance")?.unwrap_or(0.01);
    let done = crate::repair::repair(&mut feature.sketch, tolerance);
    crate::solver::solve(&mut feature.sketch);
    let words = done.describe();
    Ok(json!(words))
}

fn restore(
    a: &Args,
    sketch_id: FeatureId,
    feature: &mut SketchFeature,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let data = args
        .get("data")
        .ok_or_else(|| CommandError::bad("data", "is needed"))?;
    let restored =
        SketchFeature::from_json(data).map_err(|e| CommandError::bad("data", e.to_string()))?;
    ctx.document
        .set_feature_dependencies(sketch_id, restored.dependencies());
    *feature = restored;
    Ok(Value::Null)
}

fn section_view(
    a: &Args,
    sketch_id: FeatureId,
    mut feature: SketchFeature,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    feature.set_section(a.opt_bool("on")?.unwrap_or(true));
    ctx.document
        .update_feature_data(sketch_id, feature.to_json())
        .map_err(|e| CommandError::failed(e.to_string()))?;
    Ok(Value::Null)
}

fn delete(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let sketch = &mut feature.sketch;
    let items = ids(args.get("items"), "items", sketch)?;
    delete_items(sketch, &items);
    Ok(Value::Null)
}

fn internal_geometry(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let sketch = &mut feature.sketch;
    let items = ids(args.get("items"), "items", sketch)?;
    if crate::internal::curves_of(sketch, &items).is_empty() {
        return Err(CommandError::bad(
            "items",
            "names no ellipse, parabola, hyperbola or B-spline",
        ));
    }
    let (shown, changed) = crate::internal::toggle(sketch, &items, a.opt_bool("show")?);
    let changed: Vec<String> = changed.iter().map(Uuid::to_string).collect();
    Ok(json!({"shown": shown, "elements": changed}))
}

fn construction(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let sketch = &mut feature.sketch;
    let on = a.opt_bool("on")?.unwrap_or(true);
    for id in ids(args.get("items"), "items", sketch)? {
        sketch.set_construction(id, on);
    }
    Ok(Value::Null)
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
