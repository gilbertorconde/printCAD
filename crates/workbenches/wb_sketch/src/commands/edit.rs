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
        )
        .note(
            "`iterations` must be at least 1 and `tolerance` above 0 and below 1; either may \
             be left out to keep what the sketch has.",
        )
        .note(
            "The sketch keeps them (`fields.sketch.solver` in `pc.doc.feature`) and is \
             solved again with them at once.",
        )
        .see_also("sketch.status")
        .example(
            "Settings kept on the sketch",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 5}
            pc.sketch.solver_settings{sketch = s, iterations = 500, tolerance = 1e-6}
            local solver = pc.doc.feature{id = s}.fields.sketch.solver
            assert(solver.max_iterations == 500 and solver.tolerance == 1e-6)
            assert(pc.sketch.status{sketch = s}.solved)
            "#,
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
        )
        .note(
            "It is the answer to a profile that fails with \"profile is not closed\" because \
             ends miss by a hair, as a drawing brought in may. A Pad, Pocket, Revolution or \
             Groove made from such a sketch joins ends that near itself.",
        )
        .note(
            "It answers in words, such as \"2 end(s) joined, 1 duplicate curve(s) removed\", \
             or \"nothing to repair\".",
        )
        .see_also("sketch.import_dxf")
        .see_also("sketch.status")
        .example(
            "Ends that miss by microns joined",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
            pc.sketch.line{sketch = s, x1 = 10.005, y1 = 0, x2 = 0, y2 = 10}
            pc.sketch.line{sketch = s, x1 = 0, y1 = 10, x2 = 0, y2 = 0.003}
            local said = pc.sketch.repair{sketch = s}
            assert(said:find("2 end"), said)
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - 50) < 0.01, m.volume)
            assert(pc.sketch.repair{sketch = s} == "nothing to repair")
            "#,
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
        )
        .note(
            "`data` is the whole `fields` of `pc.doc.feature{id = s}`, plane included, taken \
             before the edits; a table that is not a sketch's data is refused.",
        )
        .see_also("doc.feature")
        .example(
            "Edits put back",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 5}
            local saved = pc.doc.feature{id = s}.fields
            pc.sketch.circle{sketch = s, x = 5, y = 2, radius = 1}
            assert(#pc.sketch.geometry{sketch = s} == 10)
            pc.sketch.restore{sketch = s, data = saved}
            assert(#pc.sketch.geometry{sketch = s} == 8, "the circle and its centre are gone")
            "#,
        ),
        editing(restore),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.delete",
            "Delete elements or constraints, and what depends on them",
        ))
        .param("items", ParamKind::List, "Element or constraint ids")
        .note(
            "Deleting a point takes the curves that end or centre on it and their \
             constraints. Deleting a curve leaves its end points behind as loose points.",
        )
        .note(
            "A constraint's id deletes only that constraint. The origin and the axes are \
             passed over; an id the sketch does not have is refused.",
        )
        .see_also("sketch.construction")
        .see_also("sketch.repair")
        .example(
            "A corner deleted with the two lines on it",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local corner = pc.sketch.point{sketch = s, x = 20, y = 10}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            assert(#pc.sketch.constraints{sketch = s} == 4)
            pc.sketch.delete{sketch = s, items = {corner}}
            local lines = 0
            for _, e in ipairs(pc.sketch.geometry{sketch = s}) do
              if e.kind == "line" then lines = lines + 1 end
            end
            assert(lines == 2, "the two lines ending on the corner went with it")
            assert(#pc.sketch.constraints{sketch = s} == 2, "and the constraints on them")
            "#,
        ),
        editing(delete),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.construction",
            "Make elements construction geometry, or normal again",
        ))
        .param("items", ParamKind::List, "Element ids")
        .optional("on", ParamKind::Bool, "true (the default) or false")
        .note(
            "Construction geometry is left out of profiles: a construction circle inside an \
             outline cuts no hole. Constraints on it still hold.",
        )
        .note(
            "`sketch.draw` makes construction geometry from the start with `construction = true`.",
        )
        .see_also("sketch.draw")
        .see_also("sketch.delete")
        .example(
            "A circle that guides, then cuts",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local guide = pc.sketch.circle{sketch = s, x = 10, y = 5, radius = 3}
            pc.sketch.construction{sketch = s, items = {guide}}
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - 200) < 1e-3, "a construction circle cuts no hole")
            pc.sketch.construction{sketch = s, items = {guide}, on = false}
            assert(#pc.doc.rebuild() == 0)
            m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - (200 - math.pi * 9)) < 0.01, m.volume)
            "#,
        ),
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
        .returns("{shown, elements}: whether it showed, and what it made or took away")
        .note(
            "Without `show` it switches: the same call twice makes the pieces and takes them \
             away again. Give `show = true` to be sure they are there.",
        )
        .note(
            "An ellipse gets its major and minor axes with their ends and its two foci, all \
             construction held to it. Items naming no ellipse, parabola, hyperbola or \
             B-spline are refused.",
        )
        .see_also("sketch.draw")
        .see_also("sketch.constrain")
        .example(
            "An ellipse's axes and foci shown and hidden",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local made = pc.sketch.draw{sketch = s, tool = "ellipse", points = {{0, 0}, {10, 0}, {0, 4}}}
            local ellipse = made.elements[#made.elements]
            local shown = pc.sketch.internal_geometry{sketch = s, items = {ellipse}}
            assert(shown.shown and #shown.elements == 8, "two axes with their ends, and two foci")
            for _, e in ipairs(pc.sketch.geometry{sketch = s}) do
              if e.kind == "line" then assert(e.construction) end
            end
            local hidden = pc.sketch.internal_geometry{sketch = s, items = {ellipse}}
            assert(not hidden.shown and #pc.sketch.geometry{sketch = s} == 2, "the same call again takes them away")
            "#,
        ),
        editing(internal_geometry),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.section_view",
            "Cut away everything on the viewer's side of the sketch plane while it is edited",
        ))
        .optional("on", ParamKind::Bool, "true (the default) or false")
        .note(
            "A view setting only: nothing built from the sketch changes, and it shows only \
             while the sketch is open for editing in the window.",
        )
        .note("The sketch's data carries `section_view = true` while it is on.")
        .example(
            "The setting kept on the sketch",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.section_view{sketch = s}
            assert(pc.doc.feature{id = s}.fields.section_view == true)
            pc.sketch.section_view{sketch = s, on = false}
            assert(pc.doc.feature{id = s}.fields.section_view == nil)
            "#,
        ),
        reading(section_view),
    );
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
