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
        .read_only()
        .note(
            "The body must be built first (`pc.doc.rebuild()`). The two faces are its \
             ends, each given by a point on it and its outward normal.",
        )
        .see_also("doc.faces")
        .see_also("doc.measure")
        .example(
            "The centre line of a quarter bend",
            r#"
            local s = pc.sketch.new{plane = "XZ"}
            pc.sketch.circle{sketch = s, x = 10, y = 0, radius = 2}
            local bend = pc.design.revolve{sketch = s, angle_deg = 90}
            assert(#pc.doc.rebuild() == 0, "the bend builds")
            local body = pc.doc.feature{id = bend}.body
            local line = pc.design.centre_line{body = body,
              from_point = {10, 0, 0}, from_normal = {0, -1, 0},
              to_point = {0, 10, 0}, to_normal = {-1, 0, 0}}
            -- A quarter circle of radius 10.
            assert(math.abs(line.length - math.pi * 10 / 2) < 0.05, line.length)
            assert(not line.straight)
            "#,
        ),
        |args, ctx| crate::centre::command(&Args(args), ctx),
    );
}
