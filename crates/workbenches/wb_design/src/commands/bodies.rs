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
        )
        .note(
            "The body must be built first (`pc.doc.rebuild()`); a body with no round holes \
             gets nothing, `holes = 0`.",
        )
        .note(
            "Per set of alike holes it adds one Delete Faces, a sketch of their centres \
             and a Hole, the Hole last; `design.set` on that Hole resizes them all.",
        )
        .see_also("design.hole")
        .see_also("design.delete_faces")
        .example(
            "Two bores made a Hole feature and widened",
            r#"
            local body = pc.doc.new_body{}
            pc.design.primitive{body = body, kind = {Box = {length = 20, width = 10, height = 5}}}
            local at = pc.sketch.new{body = body, plane = "XY", offset = 5}
            pc.sketch.circle{sketch = at, x = 5, y = 5, radius = 2}
            pc.sketch.circle{sketch = at, x = 15, y = 5, radius = 2}
            pc.design.pocket{sketch = at, through_all = true}
            assert(#pc.doc.rebuild() == 0)
            local made = pc.design.recognize_holes{body = body}
            -- The two alike bores: one Delete Faces, one sketch of centres and one Hole.
            assert(made.holes == 2 and made.left == 0 and #made.features == 3)
            assert(#pc.doc.rebuild() == 0, "the holes drill again")
            local hole = made.features[3]
            assert(pc.doc.feature{id = hole}.fields.Hole.diameter == 4)
            pc.design.set{feature = hole, diameter = 6}
            assert(#pc.doc.rebuild() == 0)
            local volume = pc.doc.measure{body = body}.volume
            assert(math.abs(volume - (1000 - 2 * math.pi * 9 * 5)) < 1e-3, volume)
            "#,
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
        )
        .note(
            "Given neither a plane nor a sketch, it cuts across the middle of the solid, \
             square to its longest side: a part too long for the bed in two.",
        )
        .note(
            "Each piece starts with a Clone of the body split and a Split feature keeping \
             its side, so editing the body split rebuilds the pieces; the body split is \
             hidden. `design.set` of one piece's `plane`, `offset` or `sketch` moves the cut \
             of every piece, and its `other_side` swaps the sides the two keep.",
        )
        .note("The body must be built first (`pc.doc.rebuild()`); a mesh body is refused.")
        .see_also("design.set")
        .see_also("doc.measure")
        .example(
            "A plate split in two, and the pieces following an edit",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 40, height = 20}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            assert(#pc.doc.rebuild() == 0)
            local pieces = pc.design.split{body = body, plane = "YZ", offset = 10}
            assert(#pieces == 2 and #pc.doc.rebuild() == 0)
            local volume = function(b) return pc.doc.measure{body = b}.volume end
            assert(math.abs(volume(pieces[1]) - 3000) < 1e-3, volume(pieces[1]))
            assert(math.abs(volume(pieces[2]) - 1000) < 1e-3, volume(pieces[2]))
            -- The pieces follow the body split.
            pc.design.set{feature = pad, length = 10}
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(volume(pieces[1]) + volume(pieces[2]) - 8000) < 1e-3)
            -- One piece's cut moved moves the other's.
            local cut = pc.doc.features{body = pieces[1]}[2].id
            pc.design.set{feature = cut, offset = 20}
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(volume(pieces[2]) - 4000) < 1e-3, volume(pieces[2]))
            "#,
        )
        .example(
            "A plate split by a sketch's circle: the disc and the plate around it",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 40, height = 20}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            local cut = pc.sketch.new{body = body, plane = "XY"}
            pc.sketch.circle{sketch = cut, x = 20, y = 10, radius = 5}
            assert(#pc.doc.rebuild() == 0)
            local pieces = pc.design.split{body = body, sketch = cut}
            assert(#pc.doc.rebuild() == 0)
            local disc = math.pi * 25 * 5
            assert(math.abs(pc.doc.measure{body = pieces[1]}.volume - disc) < 1e-3)
            assert(math.abs(pc.doc.measure{body = pieces[2]}.volume - (4000 - disc)) < 1e-3)
            "#,
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
        .returns("the pieces' body ids, in the order the solid holds them")
        .note(
            "Each piece starts with a Clone of the body and a Piece keeping the solid \
             nearest a point on that piece, so editing the body rebuilds the pieces; the \
             body itself is hidden. A solid of one piece is refused.",
        )
        .note("The body must be built first (`pc.doc.rebuild()`).")
        .see_also("doc.measure")
        .example(
            "A plate pocketed through into two pieces, separated",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 40, height = 20}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            local slot = pc.sketch.new{body = body, plane = "XY", offset = 5}
            pc.sketch.rect{sketch = slot, x = 10, y = -1, width = 2, height = 22}
            pc.design.pocket{sketch = slot, through_all = true}
            assert(#pc.doc.rebuild() == 0)
            local pieces = pc.design.separate{body = body}
            assert(#pieces == 2 and #pc.doc.rebuild() == 0)
            local a = pc.doc.measure{body = pieces[1]}.volume
            local b = pc.doc.measure{body = pieces[2]}.volume
            assert(math.abs(a + b - 3800) < 1e-3, a + b)
            assert(math.abs(math.min(a, b) - 1000) < 1e-3, math.min(a, b))
            -- A body of one piece has nothing to separate.
            assert(not pcall(pc.design.separate, {body = pieces[1]}))
            "#,
        ),
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
        .optional("name", ParamKind::String, "The sketches' names start with it")
        .returns("the sketches' ids, in order along the normal")
        .note(
            "Each sketch holds the cut's outline on its plane: the outer boundary and every \
             hole, its curves exact (lines, arcs, circles, ellipses, splines) and their ends \
             joined, so a closed cut pads. They are taken once and do not follow the solid.",
        )
        .note(
            "The body must be built first (`pc.doc.rebuild()`). A plane that misses the solid \
             is refused; with several cuts, those that miss give empty sketches.",
        )
        .see_also("design.pad")
        .see_also("sketch.regions")
        .example(
            "A plate with a hole cut across its middle and the outline padded again",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 40, height = 20}
            pc.sketch.circle{sketch = s, x = 20, y = 10, radius = 5}
            local pad = pc.design.pad{sketch = s, length = 6}
            local body = pc.doc.feature{id = pad}.body
            assert(#pc.doc.rebuild() == 0)
            local cut = pc.design.section{body = body, plane = "XY", offset = 3}
            assert(#cut == 1)
            local again = pc.design.pad{sketch = cut[1], length = 2}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = again}.body}
            assert(math.abs(m.volume - 2 * (800 - math.pi * 25)) < 1e-3, m.volume)
            "#,
        )
        .example(
            "Three cuts 2 mm apart, each a sketch",
            r#"
            local body = pc.doc.new_body{}
            pc.design.primitive{body = body, kind = {Box = {length = 10, width = 10, height = 10}}}
            assert(#pc.doc.rebuild() == 0)
            local cuts = pc.design.section{body = body, plane = "XY", offset = 2, count = 3, spacing = 2}
            assert(#cuts == 3)
            "#,
        ),
        |args, ctx| crate::section::command(&Args(args), args, ctx),
    );
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
