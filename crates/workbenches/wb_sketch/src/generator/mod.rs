//! The sketcher's side of generated profiles: the command that changes a
//! generated sketch's numbers and the panel that shows them. The numbers
//! and the curves made from them are [`sketch_model::generator`].

#[cfg(feature = "egui")]
pub(crate) mod panel;

use core_document::WorkbenchFeature;
use sketch_model::feature::SketchFeature;
pub use sketch_model::generator::*;

impl crate::SketchWorkbench {
    /// Whether the sketch open for editing is a generated one. Read off
    /// the JSON rather than the whole sketch: the toolbar asks every frame.
    pub(crate) fn editing_generated(&self, ctx: &core_document::WorkbenchRuntimeContext) -> bool {
        self.tab
            .active_sketch_id
            .and_then(|id| ctx.document.get_feature_data(id))
            .and_then(|data| data.get("generator"))
            .is_some_and(|g| !g.is_null())
    }
}

/// The command that changes a generated sketch's numbers.
pub(crate) fn table(t: &mut core_document::CommandTable<crate::SketchWorkbench>) {
    use core_document::{CommandSpec, ParamKind};
    t.add(
        CommandSpec::new(
            "sketch.generator",
            "Change the numbers a generated sketch is made from, or detach it into a plain \
             sketch: a gear, a sprocket, a shaft or a keyway",
        )
        .param("sketch", ParamKind::Id, "The generated sketch")
        .optional(
            "detach",
            ParamKind::Bool,
            "Keep the curves as they are and forget the numbers: a plain sketch to edit by hand",
        )
        .extra_args(
            "The numbers to change, such as teeth = 24 or module = 1.5; a shaft takes \
             sections = {{length = 20, diameter = 10, chamfer = 0.5, fillet = 0}, ...}",
        )
        .returns("what the numbers come to: its diameters, or its length"),
        command,
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "generator.md", include_str!("generator.md"), &[])
}

/// Run `sketch.generator`.
pub fn command(
    args: &core_document::CommandArgs,
    ctx: &mut core_document::WorkbenchRuntimeContext,
) -> core_document::CommandResult {
    use core_document::{Args, CommandError, FeatureId};
    let a = Args(args);
    let id = FeatureId(a.id("sketch")?);
    let data = ctx
        .document
        .get_feature_data(id)
        .ok_or_else(|| CommandError::bad("sketch", "is not a feature of this document"))?;
    let mut feature = SketchFeature::from_json(data)
        .map_err(|_| CommandError::bad("sketch", "is not a sketch"))?;
    let Some(generator) = feature.generator.as_mut() else {
        return Err(CommandError::bad("sketch", "is not a generated sketch"));
    };
    let fields: serde_json::Map<String, serde_json::Value> = args
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "sketch" | "detach"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    // The answer is what builds: the numbers formulas set, with these.
    let mut built = crate::stored_sketch(ctx.document, id)
        .and_then(|f| f.generator)
        .unwrap_or_else(|| generator.clone());
    generator.merge(&fields).map_err(CommandError::failed)?;
    built.merge(&fields).map_err(CommandError::failed)?;
    let answer = summary(&built);
    if a.opt_bool("detach")?.unwrap_or(false) {
        feature.generator = None;
    } else {
        regenerate(&mut feature).map_err(CommandError::failed)?;
    }
    ctx.document
        .update_feature_data(id, feature.to_json())
        .map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document.mark_feature_dirty(id);
    Ok(answer)
}
