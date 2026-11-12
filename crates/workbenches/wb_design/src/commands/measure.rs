//! Measures of a body's solid: its centre line.

use core_document::{Args, CommandSpec, CommandTable, ParamKind};

use crate::DesignWorkbench;

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<DesignWorkbench>) {
    t.add(
        CommandSpec::new(
            "design.centre_line",
            "Measure the centre line of a tube-like solid between two of its faces",
        )
        .param(
            "body",
            ParamKind::Id,
            "The body whose solid it runs through",
        )
        .param(
            "from_point",
            ParamKind::List,
            "A point of the face it starts at, {x, y, z}, in the body's own frame",
        )
        .param(
            "from_normal",
            ParamKind::List,
            "That face's outward normal, {x, y, z}",
        )
        .param(
            "to_point",
            ParamKind::List,
            "A point of the face it ends at, {x, y, z}",
        )
        .param(
            "to_normal",
            ParamKind::List,
            "That face's outward normal, {x, y, z}",
        )
        .optional(
            "tolerance",
            ParamKind::Number,
            "How closely it follows the sections' centres, mm (0.02 when left out)",
        )
        .returns(
            "{length, points, deviation, straight}: its length in mm, points along it \
             in the body's frame, the largest distance measured from a section's centre \
             to it, and whether it is one straight segment",
        )
        .read_only(),
        |args, ctx| crate::centre::command(&Args(args), ctx),
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "measure.md", include_str!("measure.md"), &[])
}
