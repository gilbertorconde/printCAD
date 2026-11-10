//! `surface.set`: a step's fields and curves changed.

use core_document::{
    CommandArgs, CommandError, CommandResult, CommandSpec, FeatureId, ParamKind, WorkbenchContext,
    WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::Value;

use super::steps::{merge_fields, sketch_curves, unreadable};
use crate::feature::{KIND, SurfaceFeature};
use crate::{SET_COMMAND, panel_free};

pub(super) fn owns(id: &str) -> bool {
    id == SET_COMMAND
}

pub(super) fn configure(context: &mut WorkbenchContext) {
    context.register_command(set());
}

pub(super) fn run(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    set_by_command(args, ctx)
}

/// `surface.set`.
pub fn set() -> CommandSpec {
    CommandSpec::new(SET_COMMAND, "Change fields of a surface step")
        .param("feature", ParamKind::Id, "The surface step to change")
        .extra_args(
            "The fields to change, by name (`length`, `continuity`, `plane`, `sketches`…), as \
             the step's own command takes them",
        )
        .returns("nothing")
        .note(
            "It takes the fields the step's command takes, by the same names; `sketches` \
             replaces the curves it is built from. A name the step does not have is refused \
             (\"is not a field of this surface\"); the kind of step stays.",
        )
        .note(
            "The step keeps its place in the body's history: everything after it builds again \
             on the change at `pc.doc.rebuild()`.",
        )
        .example(
            "A wall made taller",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
            local wall = pc.surface.extrude{sketches = {s}, length = 5}
            local body = pc.doc.feature{id = wall}.body
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = body}.area - 50) < 1e-6)
            pc.surface.set{feature = wall, length = 8}
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = body}.area - 80) < 1e-6)
            "#,
        )
}

/// `surface.set`: named fields merged into a step, `sketches` replacing its
/// curves, recorded as one edit.
fn set_by_command(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let id = FeatureId(core_document::command::Args(args).id("feature")?);
    let node = ctx
        .document
        .get_feature_meta(id)
        .ok_or_else(|| CommandError::bad("feature", "is not in this document"))?;
    if node.workbench_id.as_str() != KIND {
        return Err(CommandError::bad("feature", "is not a surface step"));
    }
    let mut feature = ctx
        .document
        .get_feature_data(id)
        .and_then(|data| SurfaceFeature::from_json(data).ok())
        .ok_or_else(|| CommandError::failed("the step's data does not read"))?;
    if args.contains_key("sketches") {
        let curves = sketch_curves(&feature, args, ctx.document)?;
        feature.clear_curves();
        panel_free::take_selection(&mut feature, &curves, &[]);
    }
    let mut fields = args.clone();
    fields.remove("feature");
    fields.remove("sketches");
    let mut data = feature.to_json();
    merge_fields(&mut data, &fields)?;
    let feature = SurfaceFeature::from_json(&data).map_err(unreadable)?;
    if let Some(missing) = feature.missing() {
        return Err(CommandError::failed(format!("it needs {missing}")));
    }
    ctx.document
        .update_feature_data(id, feature.to_json())
        .map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document.mark_feature_dirty(id);
    Ok(Value::Null)
}
