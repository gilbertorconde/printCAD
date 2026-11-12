//! `surface.set`: a step's fields and curves changed.

use core_document::{
    CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, FeatureId, ParamKind,
    WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::Value;

use super::steps::{merge_fields, sketch_curves, unreadable};
use crate::feature::{KIND, SurfaceFeature};
use crate::{SET_COMMAND, SurfaceWorkbench, panel_free};

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<SurfaceWorkbench>) {
    t.add(set(), set_by_command);
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "set.md", include_str!("set.md"), &[])
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
