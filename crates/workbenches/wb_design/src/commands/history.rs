//! Moving features through histories: borrowed geometry frozen or mapped,
//! a feature moved to another body or duplicated.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, FeatureId,
    ParamKind, WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::Value;

use crate::DesignWorkbench;
use crate::feature::DesignFeature;

/// `design.freeze`: a borrow takes its source as it is now, or follows it
/// again.
fn freeze(a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let id = FeatureId(a.id("feature")?);
    let not_a_borrow = || CommandError::bad("feature", "is not borrowed geometry");
    let body = ctx
        .document
        .get_feature_meta(id)
        .and_then(|n| n.body)
        .ok_or_else(not_a_borrow)?;
    let Some(DesignFeature::Borrow {
        source, options, ..
    }) = ctx
        .document
        .get_feature_data(id)
        .and_then(|d| DesignFeature::from_json(d).ok())
    else {
        return Err(not_a_borrow());
    };
    let frozen = if a.opt_bool("frozen")?.unwrap_or(true) {
        Some(
            crate::borrow::freeze(ctx.document, ctx.kernel, body, &source, &options)
                .map_err(CommandError::failed)?,
        )
    } else {
        None
    };
    let feature = DesignFeature::Borrow {
        source,
        frozen,
        options,
    };
    ctx.document
        .update_feature_data(id, feature.to_json())
        .map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document
        .set_feature_dependencies(id, feature.dependencies());
    ctx.document.mark_feature_dirty(id);
    Ok(Value::Null)
}

fn move_to_body(a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let id = FeatureId(a.id("feature")?);
    let body = BodyId(a.id("body")?);
    move_feature(ctx, id, body)
        .map(|moved| Value::from(moved.iter().map(|f| f.0.to_string()).collect::<Vec<_>>()))
        .map_err(CommandError::failed)
}

/// Copy `id` into `body`, its own when `None`.
pub(crate) fn duplicate(
    ctx: &mut WorkbenchRuntimeContext,
    id: FeatureId,
    body: Option<BodyId>,
) -> Result<Vec<FeatureId>, String> {
    let copied = crate::clipboard::Clipboard::copy(ctx.document, id)
        .ok_or("the feature belongs to no body")?;
    let body = match body {
        Some(body) if ctx.document.body(body).is_some() => body,
        Some(_) => return Err("no such body".into()),
        None => ctx
            .document
            .get_feature_meta(id)
            .and_then(|n| n.body)
            .ok_or("the feature belongs to no body")?,
    };
    Ok(copied.paste(ctx.document, body))
}

/// Move `id` into `body` and rebuild both bodies.
pub(crate) fn move_feature(
    ctx: &mut WorkbenchRuntimeContext,
    id: FeatureId,
    body: BodyId,
) -> Result<Vec<FeatureId>, String> {
    let from = ctx
        .document
        .get_feature_meta(id)
        .and_then(|n| n.body)
        .ok_or("the feature belongs to no body")?;
    let moved = ctx.document.move_feature_to_body(id, body)?;
    crate::invalidate_body(ctx.document, from);
    crate::invalidate_body(ctx.document, body);
    Ok(moved)
}

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<DesignWorkbench>) {
    t.add(
        CommandSpec::new(
            "design.freeze",
            "Freeze borrowed geometry as it is now, or let it follow its source again",
        )
        .param("feature", ParamKind::Id, "The borrow")
        .optional(
            "frozen",
            ParamKind::Bool,
            "true (the default) takes the source as it is now; false follows it again",
        )
        .returns("nothing"),
        |args, ctx| freeze(&Args(args), ctx),
    );
    t.add(
        CommandSpec::new(
            "design.map_sketch",
            "Put an existing sketch on a flat face its body borrows from another body; the \
             sketch then follows the face",
        )
        .param("sketch", ParamKind::Id, "The sketch")
        .param(
            "borrow",
            ParamKind::Id,
            "A borrow of faces in the sketch's body",
        )
        .optional(
            "index",
            ParamKind::Integer,
            "Which of the borrow's faces, from 0; its first flat one when left out",
        )
        .returns("nothing"),
        map_sketch,
    );
    t.add(
        CommandSpec::new(
            "design.move_to_body",
            "Move a feature into another body's history, with the sketch and datums only it uses",
        )
        .param("feature", ParamKind::Id, "The feature")
        .param("body", ParamKind::Id, "The body it goes to, in at its tip")
        .returns("the ids of the features moved, the given one last"),
        |args, ctx| move_to_body(&Args(args), ctx),
    );
    t.add(
        CommandSpec::new(
            "design.duplicate",
            "Make a copy of a feature, with its own copies of the sketches and datums it reads",
        )
        .param("feature", ParamKind::Id, "The feature")
        .optional(
            "body",
            ParamKind::Id,
            "The body the copy goes in, at its tip (the feature's own when left out)",
        )
        .returns("the ids of the features made, the copy of the given one last"),
        duplicate_command,
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "history.md", include_str!("history.md"), &[])
}

fn map_sketch(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let sketch = FeatureId(a.id("sketch")?);
    let borrow = FeatureId(a.id("borrow")?);
    let index = match a.opt_number("index")? {
        Some(i) if i < 0.0 => return Err(CommandError::bad("index", "must be 0 or more")),
        Some(i) => i as usize,
        None => crate::borrow::flat_faces_of_body(
            ctx.document,
            ctx.document
                .get_feature_meta(borrow)
                .and_then(|n| n.body)
                .ok_or_else(|| CommandError::bad("borrow", "is not a borrow"))?,
        )
        .into_iter()
        .find(|(r, _)| r.borrow == borrow)
        .map(|(r, _)| r.index)
        .ok_or_else(|| CommandError::bad("borrow", "lends no flat face"))?,
    };
    crate::borrow::map_sketch(
        ctx.document,
        sketch,
        crate::feature::BorrowedRef { borrow, index },
    )
    .map_err(CommandError::failed)?;
    Ok(Value::Null)
}

fn duplicate_command(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let feature = FeatureId(a.id("feature")?);
    let body = a.opt_id("body")?.map(BodyId);
    duplicate(ctx, feature, body)
        .map(|made| Value::from(made.iter().map(|f| f.0.to_string()).collect::<Vec<_>>()))
        .map_err(CommandError::failed)
}
