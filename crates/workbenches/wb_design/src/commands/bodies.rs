//! Commands on a body's solid: its holes recognized, split, separated into
//! pieces and sectioned.

use core_document::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, ParamKind,
    WorkbenchRuntimeContext,
};
use serde_json::json;

use crate::DesignWorkbench;

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<DesignWorkbench>) {
    t.add(
        CommandSpec::new(
            "design.recognize_holes",
            "Make the round holes of a body's solid Hole features: their faces deleted, \
             and each set of alike holes drilled again from a sketch of their centres",
        )
        .param("body", ParamKind::Id, "The body")
        .returns(
            "{holes, left, features}: the holes made features, the bores left as they are \
             (counterbores, slots) and the features added",
        ),
        recognize_holes,
    );
    t.add(
        CommandSpec::new(
            "design.split",
            "Split a body in two along a plane or a sketch, each piece a body following it",
        )
        .optional(
            "body",
            ParamKind::Id,
            "The body to split; the selected one when left out",
        )
        .optional(
            "plane",
            ParamKind::Any,
            "The plane it cuts along, in the body's own frame: \"XY\", \"XZ\", \"YZ\", \
             {Datum = {datum = id}} or {Sketch = {sketch = id}}",
        )
        .optional(
            "offset",
            ParamKind::Number,
            "mm the plane moves along its normal; 0 with a plane given",
        )
        .optional(
            "sketch",
            ParamKind::Id,
            "A sketch of the body whose profile cuts, carried through the whole solid \
             along its normal, in place of the plane",
        )
        .optional(
            "name",
            ParamKind::String,
            "The pieces' names start with it; the body's name when left out",
        )
        .returns(
            "the pieces' body ids: first the side the plane's normal points to (inside the \
             sketch's profile), then the other",
        ),
        |args, ctx| crate::split::command(&Args(args), args, ctx),
    );
    t.add(
        CommandSpec::new(
            "design.separate",
            "Make each piece of a body's solid a body of its own, following it",
        )
        .optional(
            "body",
            ParamKind::Id,
            "The body; the selected one when left out",
        )
        .returns("the pieces' body ids, in the order the solid holds them"),
        |args, ctx| crate::separate::command(&Args(args), ctx),
    );
    t.add(
        CommandSpec::new(
            "design.section",
            "Cut a body by a plane and keep the outline as a sketch, or several parallel cuts",
        )
        .optional(
            "body",
            ParamKind::Id,
            "The body to cut; the selected one when left out",
        )
        .optional(
            "plane",
            ParamKind::Any,
            "The plane, in the body's own frame: \"XY\", \"XZ\", \"YZ\", {Datum = {datum = \
             id}} or {Sketch = {sketch = id}}; across Z through the middle of the solid when \
             left out",
        )
        .optional(
            "offset",
            ParamKind::Number,
            "mm the first cut moves along the plane's normal; 0 with a plane given",
        )
        .optional(
            "count",
            ParamKind::Integer,
            "How many cuts, each a sketch of its own; 1 when left out",
        )
        .optional(
            "spacing",
            ParamKind::Number,
            "mm between one cut and the next, along the normal",
        )
        .optional(
            "into",
            ParamKind::Id,
            "The body the sketches go in; a new body placed where the cut one stands when \
             left out",
        )
        .optional(
            "name",
            ParamKind::String,
            "The sketches' names start with it",
        )
        .returns("the sketches' ids, in order along the normal"),
        |args, ctx| crate::section::command(&Args(args), args, ctx),
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "bodies.md", include_str!("bodies.md"), &[])
}

fn recognize_holes(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let body = a.body(ctx.document, "body")?;
    let made = crate::recognize::recognize_holes(ctx, body).map_err(CommandError::failed)?;
    Ok(json!({
        "holes": made.holes,
        "left": made.left,
        "features": made.features.iter().map(|f| f.0.to_string()).collect::<Vec<_>>(),
    }))
}
