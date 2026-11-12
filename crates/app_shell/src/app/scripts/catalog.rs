//! Host command specifications and file/key command contracts.

use crate::PrintCadApp;
use crate::ui::keymap;
use core_document::{Args, CommandError, CommandProse, CommandSpec, ParamKind};
use std::sync::OnceLock;

/// The notes, related commands and examples of the commands made here.
pub(super) fn prose() -> &'static CommandProse {
    static PROSE: OnceLock<CommandProse> = OnceLock::new();
    CommandProse::once(&PROSE, "catalog.md", include_str!("catalog.md"), &[])
}

/// `specs` with `catalog.md`'s prose.
fn explained(specs: Vec<CommandSpec>) -> Vec<CommandSpec> {
    specs.into_iter().map(|s| prose().explain(s)).collect()
}

/// The application's own commands beyond the keyboard's.
pub(crate) fn doc_commands() -> Vec<CommandSpec> {
    explained(vec![
        CommandSpec::new("doc.info", "The document's name, file and unit")
            .returns("{name, file, unit, modified}")
            .read_only(),
        CommandSpec::new(
            "doc.agent_rules",
            "The rules an AI agent working on this document keeps to, beside the ones in \
             Preferences for every document",
        )
        .returns("the rules, as text")
        .read_only(),
        CommandSpec::new(
            "doc.set_agent_rules",
            "Set the rules an AI agent working on this document keeps to; they are saved with \
             the document",
        )
        .param("rules", ParamKind::String, "The rules, as plain text")
        .agent_never("only the user sets the rules an agent keeps to"),
        CommandSpec::new("doc.bodies", "List the bodies")
            .returns(
                "a list of {id, name, visible, frozen, scan, selectable, material, array, features}",
            )
            .read_only(),
        CommandSpec::new("doc.features", "List the features in build order")
            .optional("body", ParamKind::Id, "Only this body's")
            .returns("a list of {id, name, kind, body, visible, suppressed, error}")
            .read_only(),
        CommandSpec::new("doc.feature", "A feature with its fields")
            .param("id", ParamKind::Id, "")
            .returns(
                "{id, name, kind, body, visible, suppressed, error, fields, unset, values}: \
                 unset names the fields holding no value, which fields leaves out; values \
                 is the data as the feature is built now (formulas worked out, a plane \
                 following what it stands on), given when it differs from fields",
            )
            .read_only(),
        CommandSpec::new("doc.selection", "What is selected")
            .returns("{item, body, feature}, each an id or nil")
            .read_only(),
        CommandSpec::new(
            "doc.select",
            "Select a body or a feature, as a click on its row",
        )
        .param("id", ParamKind::Id, ""),
        CommandSpec::new(
            "doc.coupon",
            "Make a calibration coupon to print",
        )
        .optional(
            "diameters",
            ParamKind::List,
            "The holes' and pins' diameters, mm; {3, 4, 5, 6, 8}",
        )
        .optional(
            "gaps",
            ParamKind::List,
            "The ladder's slot widths, mm; {0.3, 0.4, 0.5, 0.6, 0.8}",
        )
        .optional(
            "overhangs",
            ParamKind::List,
            "The fan's overhangs, degrees past vertical; {30, 45, 60, 70}, {} for none",
        )
        .returns("{body, width, depth, holes, pins, gaps, overhangs}"),
        CommandSpec::new("doc.new_body", "Make an empty body")
            .optional("name", ParamKind::String, "")
            .returns("the body's id"),
        CommandSpec::new("doc.rename", "Rename a body, a feature or a note (its text)")
            .param("id", ParamKind::Id, "")
            .param("name", ParamKind::String, "")
            .optional(
                "author",
                ParamKind::String,
                "Who asks: user (the default) or agent; an agent's calls are always the \
                 agent's, and an agent may not reword a user's note",
            ),
        CommandSpec::new(
            "doc.set_body",
            "Change a body: its colour, how much shows through, material, placement, whether \
             it is frozen, a scan, or clicks pick it",
        )
        .param("body", ParamKind::Id, "")
        .optional(
            "color",
            ParamKind::Any,
            "{r, g, b} from 0 to 1, or nil for the colour it came with",
        )
        .optional("opacity", ParamKind::Number, "1 solid, less to see through")
        .optional(
            "material",
            ParamKind::Any,
            "{name, density} with the density in g/cm³, or nil for none",
        )
        .optional(
            "frozen",
            ParamKind::Bool,
            "Keep it as it stands: its features are not rebuilt until it thaws",
        )
        .optional(
            "shading",
            ParamKind::String,
            "A mesh body's shading: as_meshed (the default), sharp (every triangle flat) or \
             smooth (across triangles turning less than `crease`)",
        )
        .optional("crease", ParamKind::Number, "Smooth shading's crease angle, degrees (30)")
        .optional(
            "selectable",
            ParamKind::Bool,
            "false lets clicks pass through it",
        )
        .optional(
            "scan",
            ParamKind::Bool,
            "A mesh body marked as a scan, the measured part that inspection and mesh \
             editing work on; false makes it an ordinary mesh again",
        )
        .optional(
            "face_colors",
            ParamKind::List,
            "An empty list gives every face the body's colour again",
        )
        .optional(
            "translation",
            ParamKind::Any,
            "{x, y, z}: where its origin goes; bodies moving as one with it follow",
        )
        .optional(
            "rotation",
            ParamKind::Any,
            "{x, y, z, w}: its turn as a quaternion",
        ),
        CommandSpec::new(
            "doc.set_textures",
            "Set the textures pressed into a body's faces for printing",
        )
        .param("body", ParamKind::Id, "")
        .param(
            "textures",
            ParamKind::List,
            "Each {texture = {pattern, projection, tile_mm, depth_mm, rotation_deg, inward, \
             keep_flat_deg}, faces = {...}}: pattern Knurl, Ribs, Dots, Hex, Bricks, Waves, \
             Noise or Crosshatch; projection Triplanar, {Planar = \"Z\"}, {Cylindrical = \"Z\"} \
             or Spherical; faces as doc.faces numbers them, none for every face; an empty list \
             takes them all away",
        ),
        CommandSpec::new(
            "doc.set_face_color",
            "Colour one face of a body, over the body's colour",
        )
        .param("body", ParamKind::Id, "")
        .param(
            "face",
            ParamKind::Integer,
            "The face, as doc.faces numbers it from 0",
        )
        .optional(
            "color",
            ParamKind::Any,
            "{r, g, b} from 0 to 1, or nil for the body's colour",
        ),
        CommandSpec::new(
            "doc.linked_copy",
            "A linked copy of a body beside it: the same shape, following every change",
        )
        .param("body", ParamKind::Id, "")
        .returns("the copy's id"),
        CommandSpec::new(
            "doc.array",
            "Repeat a body along a line, around an axis, along a path or at points, each \
             repeat a linked copy",
        )
        .optional(
            "body",
            ParamKind::Id,
            "The body repeated, the array's source; or give `component`",
        )
        .optional(
            "component",
            ParamKind::Id,
            "A component repeated in place of a body: every body in it copied, each \
             occurrence's copies held together in a rigid component of their own",
        )
        .optional(
            "count",
            ParamKind::Integer,
            "Occurrences, the source counted: 3 is the source and two copies; for points, \
             every point when left out",
        )
        .optional(
            "kind",
            ParamKind::String,
            "\"linear\" (along a line, the default), \"polar\" (around an axis), \"path\" \
             (along a sketch's curves or an edge) or \"points\" (at a sketch's points)",
        )
        .optional(
            "sketch",
            ParamKind::Id,
            "path: the sketch whose curves the copies run along; points: the sketch whose \
             points (those no curve ends or centres on) they stand at",
        )
        .optional(
            "chain",
            ParamKind::Integer,
            "path: which chain of the sketch's joined curves, from 1; the first when left out",
        )
        .optional(
            "edge",
            ParamKind::Any,
            "path: {body, point = {x, y, z}}, the edge of that body's solid nearest the \
             point (where the body sits, as `doc.edges` gives it), in place of a sketch",
        )
        .optional(
            "align",
            ParamKind::Bool,
            "path: true turns each copy as the path turns; false (the default) keeps the \
             source's orientation",
        )
        .optional(
            "direction",
            ParamKind::Any,
            "linear: {x, y, z}, the way the row runs; along X when left out",
        )
        .optional(
            "spacing",
            ParamKind::Number,
            "linear: millimetres between neighbours; 10 mm clear of each other when left out. \
             path: millimetres along the path; spread over the whole of it when left out",
        )
        .optional(
            "axis",
            ParamKind::Any,
            "polar: {x, y, z}, the axis turned about; Z when left out",
        )
        .optional(
            "center",
            ParamKind::Any,
            "polar: {x, y, z}, a point on the axis; the origin when left out",
        )
        .optional(
            "angle",
            ParamKind::Number,
            "polar: degrees between neighbours; a whole turn shared out when left out",
        )
        .optional(
            "skip",
            ParamKind::List,
            "Occurrences left out, numbered from 1 (0 is the source)",
        )
        .returns(
            "{id, instances, components}: the array's id, the copies of its source body in \
             order and, for a component, the component each occurrence's copies sit in",
        ),
        CommandSpec::new(
            "doc.set_array",
            "Change an array of linked copies: its count, spacing, angle, axis, path, points \
             or skipped occurrences",
        )
        .param("array", ParamKind::Id, "")
        .optional(
            "count",
            ParamKind::Integer,
            "Occurrences, the source counted; for points, every point when left out",
        )
        .optional(
            "kind",
            ParamKind::String,
            "\"linear\", \"polar\", \"path\" or \"points\"; the other kind's values start \
             afresh",
        )
        .optional("direction", ParamKind::Any, "linear: {x, y, z}")
        .optional(
            "spacing",
            ParamKind::Number,
            "linear: millimetres; path: millimetres along it",
        )
        .optional("axis", ParamKind::Any, "polar: {x, y, z}")
        .optional("center", ParamKind::Any, "polar: {x, y, z}")
        .optional("angle", ParamKind::Number, "polar: degrees between neighbours")
        .optional("sketch", ParamKind::Id, "path or points: the sketch")
        .optional("chain", ParamKind::Integer, "path: the sketch's chain, from 1")
        .optional("edge", ParamKind::Any, "path: {body, point = {x, y, z}}")
        .optional(
            "align",
            ParamKind::Bool,
            "path: true turns the copies as the path turns",
        )
        .optional(
            "skip",
            ParamKind::List,
            "Occurrences left out, numbered from 1; replaces the list it had",
        )
        .optional(
            "remove",
            ParamKind::Bool,
            "true takes the array away with every copy it owns; the source stays",
        )
        .returns("{id, instances}, as the array stands after the change"),
        CommandSpec::new(
            "doc.move_after",
            "Move a feature in its body's history to just after another",
        )
        .param("id", ParamKind::Id, "The feature")
        .optional(
            "after",
            ParamKind::Id,
            "The feature it goes after; first in its body when left out",
        ),
        CommandSpec::new("doc.recompute", "Build a body again from its history")
            .param("body", ParamKind::Id, ""),
        CommandSpec::new("doc.set_visible", "Show or hide a body, a feature or a note")
            .param("id", ParamKind::Id, "")
            .param("visible", ParamKind::Bool, ""),
        CommandSpec::new("doc.delete", "Delete a body, a feature or a note")
            .param("id", ParamKind::Id, "")
            .optional(
                "author",
                ParamKind::String,
                "Who asks: user (the default) or agent; an agent's calls are always the \
                 agent's, and an agent may not delete a user's note",
            ),
        CommandSpec::new(
            "doc.repair",
            "Repair the shapes the kernel's checker calls broken",
        )
        .param("bodies", ParamKind::List, "The bodies")
        .returns("nothing; pc.doc.rebuild() waits for the repair"),
        CommandSpec::new("doc.convert_to_solid", "Turn mesh bodies into solids")
            .param("bodies", ParamKind::List, "The mesh bodies")
            .returns("nothing; pc.doc.rebuild() waits for the conversion"),
        CommandSpec::new(
            "doc.refine",
            "Rebuild converted solids' facets on the cylinders, cones, spheres and tori they \
             approximate",
        )
        .param("bodies", ParamKind::List, "The converted bodies")
        .returns("nothing; pc.doc.rebuild() waits for the refine"),
        CommandSpec::new(
            "doc.replace_shape",
            "Read a body's shape from another file: its first solid becomes the shape the \
             body's features build on",
        )
        .param(
            "body",
            ParamKind::Id,
            "An imported or converted body, or one with a base shape",
        )
        .param("path", ParamKind::String, "A STEP, IGES or mesh file")
        .returns("nothing; pc.doc.rebuild() waits for the new shape")
        .agent_always_asks(),
        CommandSpec::new(
            "doc.suppress",
            "Leave a feature out of its body's solid, or back in",
        )
        .param("id", ParamKind::Id, "The feature")
        .optional("suppressed", ParamKind::Bool, "true (the default) or false"),
        CommandSpec::new("doc.move", "Move a feature one step in its body's history")
            .param("id", ParamKind::Id, "The feature")
            .param("up", ParamKind::Bool, "true: earlier, false: later")
            .returns(
                "true; a move past the end of the history, or past a feature one of the two \
                 is built from, fails saying so",
            ),
        CommandSpec::new(
            "doc.set_tip",
            "Build a body only up to a feature, or all of it again",
        )
        .param("id", ParamKind::Id, "A feature of the body")
        .optional(
            "clear",
            ParamKind::Bool,
            "true: build the whole history again",
        ),
        CommandSpec::new(
            "doc.rebuild",
            "Rebuild every solid that changed, repair or convert what was asked, and wait",
        )
        .optional("timeout", ParamKind::Number, "Seconds to wait at most (60)")
        .returns("a list of {feature, error} for every feature that failed")
        .read_only(),
        CommandSpec::new(
            "doc.note_add",
            "Put a numbered note on a face, an edge or a point of a body: a leader from the \
             point to a label, drawn in the view and written to STEP",
        )
        .param("body", ParamKind::Id, "")
        .param(
            "text",
            ParamKind::String,
            "What the label says; starting with = it shows what a formula comes to \
             (=Pad.length)",
        )
        .optional(
            "point",
            ParamKind::Any,
            "{x, y, z} where the body sits: the note goes to the point of the body nearest \
             it; without one, to the top of the body's middle",
        )
        .optional(
            "on",
            ParamKind::String,
            "face (the default): the face there; edge: the edge nearest the point; point: \
             the point alone, following nothing but the body",
        )
        .optional(
            "label",
            ParamKind::Any,
            "{x, y, z} where the label stands; without one, out from the face the point is on",
        )
        .optional(
            "asks",
            ParamKind::Bool,
            "A question for the user to answer in place, about what the note points at, \
             rather than a remark",
        )
        .optional(
            "author",
            ParamKind::String,
            "user (the default) or agent; an agent's notes are always the agent's",
        )
        .returns("the note's id"),
        CommandSpec::new(
            "doc.note_set",
            "Change a note's text, label or visibility, resolve it or answer its question",
        )
        .optional("id", ParamKind::Id, "The note")
        .optional("number", ParamKind::Integer, "The note by its number, in place of id")
        .optional("text", ParamKind::String, "What the label says")
        .optional(
            "label",
            ParamKind::Any,
            "{x, y, z} where the label stands, where the body sits",
        )
        .optional("visible", ParamKind::Bool, "false keeps it off the view")
        .optional(
            "done",
            ParamKind::String,
            "Resolve it, saying what was done (an answer, or the change made); it draws \
             grey from then on",
        )
        .optional("reopen", ParamKind::Bool, "true takes a resolved note back to open")
        .optional(
            "author",
            ParamKind::String,
            "Who asks for the change: user (the default) or agent; an agent's calls are \
             always the agent's",
        ),
        CommandSpec::new("doc.notes_open", "The notes not yet resolved")
            .returns("a list of notes as doc.notes gives them")
            .read_only(),
        CommandSpec::new("doc.note_remove", "Take a note away")
            .optional("id", ParamKind::Id, "The note")
            .optional("number", ParamKind::Integer, "The note by its number, in place of id")
            .agent_never(
                "a note is the user's to keep or take away; resolve it with doc.note_set \
                 instead",
            ),
        CommandSpec::new("doc.notes", "List the notes on bodies")
            .optional("body", ParamKind::Id, "Only this body's")
            .returns(
                "a list of {id, number, body, body_name, text, shows, author, point, label, \
                 visible, on, on_face, lost, asks, resolved, done, from_file, surface, feature, \
                 feature_name, edge}: point and label where the body sits, as built now; shows \
                 is what the label says",
            )
            .read_only(),
        CommandSpec::new("doc.faces", "The faces of a body's solid, where it sits")
            .param("body", ParamKind::Id, "")
            .optional(
                "frame",
                ParamKind::String,
                "world (the default): where the body sits; body: in the body's own frame, \
                 as features take faces and edges",
            )
            .returns(
                "a list of {index, kind, point, area, normal?, axis?, radius?, half_angle?, \
                 major_radius?, minor_radius?, name?}: point lies on the face, normal is a \
                 flat face's outward one, axis a turned face's {point, direction}, \
                 half_angle a cone's wall off its axis in degrees, major_radius and \
                 minor_radius a torus's",
            )
            .read_only(),
        CommandSpec::new("doc.edges", "The edges of a body's solid, where it sits")
            .param("body", ParamKind::Id, "")
            .optional(
                "frame",
                ParamKind::String,
                "world (the default): where the body sits; body: in the body's own frame, \
                 as features take faces and edges",
            )
            .returns(
                "a list of {index, kind, point, direction, length, faces, names?, centre?, \
                 normal?, radius?}: kind is line, circle or other; point lies halfway along \
                 the edge and direction is its way there, in world space, as an edge pick \
                 takes them ({point = e.point, direction = e.direction}); faces are the \
                 indices doc.faces gives the two faces it runs between, names theirs as \
                 strings; a circle's centre, normal and radius",
            )
            .read_only(),
        CommandSpec::new(
            "doc.measure",
            "A body's volume, surface area, centre and bounds",
        )
        .param("body", ParamKind::Id, "")
        .optional(
            "about",
            ParamKind::Any,
            "What the inertia is also taken about: \"origin\", a point {x, y, z}, a line \
             {point = {x, y, z}, direction = {x, y, z}}, or the id of a datum point, \
             coordinate system or line",
        )
        .returns("{volume, area, centre, min, max, approximate, inertia?}")
        .read_only(),
        CommandSpec::new(
            "doc.print_layout",
            "Where the print layout puts each copy: every part flat on its resting face, \
             as many as the parts list prints, packed on the bed",
        )
        .optional(
            "bed",
            ParamKind::Any,
            "{x, y, z}: the bed's width, depth and build height, mm; Preferences › 3D printing's \
             when left out",
        )
        .optional(
            "gap",
            ParamKind::Number,
            "Between copies, mm; Preferences › 3D printing's (5) when left out",
        )
        .returns(
            "{plates, pieces = {{body, name, plate, min = {x, y, z}, max = {x, y, z}, \
             transform}}, too_big, too_tall, unbuilt}",
        )
        .read_only(),
        CommandSpec::new(
            "doc.report_pdf",
            "Write a printable report of what the workbenches measured, as a PDF",
        )
        .param("path", ParamKind::String, "Where to write the PDF")
        .optional("title", ParamKind::String, "The report's title; the document's name")
        .returns("{path, pages, sections}")
        .agent_always_asks(),
        doc_render::proof::with_arguments(
            CommandSpec::new(
                "doc.picture",
                "Write a PNG of the bodies, the same for the same arguments whatever the \
                 user's camera (except view \"current\")",
            )
            .param("path", ParamKind::String, "Where to write the PNG"),
        )
        .returns("{path, width, height, renderer}")
        .agent_always_asks(),
        CommandSpec::new(
            "doc.parameters",
            "A feature's numbers that formulas set and read",
        )
        .param("id", ParamKind::Id, "The feature")
        .returns(
            "a list of {name, key, label, kind, value, text, formula, error}: name is what \
             formulas call it (nil when they cannot), value in mm or degrees",
        )
        .read_only(),
        CommandSpec::new(
            "doc.set_formula",
            "Set one of a feature's numbers by a formula, or take the formula away",
        )
        .param("id", ParamKind::Id, "The feature")
        .param(
            "parameter",
            ParamKind::String,
            "Its name or key, as doc.parameters lists them",
        )
        .optional(
            "formula",
            ParamKind::String,
            "Such as \"Printer.wall * 2\"; nil takes it away",
        )
        .returns("{value, text, error}: what it comes to"),
        CommandSpec::new(
            "doc.set_value",
            "Set one of a feature's numbers, taking away any formula on it",
        )
        .param("id", ParamKind::Id, "The feature")
        .param(
            "parameter",
            ParamKind::String,
            "Its name or key, as doc.parameters lists them",
        )
        .param("value", ParamKind::Number, "In millimetres or degrees"),
        CommandSpec::new("var.new", "Make a variable set")
            .param(
                "name",
                ParamKind::String,
                "What formulas call it: Printer.nozzle",
            )
            .returns("the set's id"),
        CommandSpec::new("var.set", "Set a variable to a formula, adding it when new")
            .param("set", ParamKind::String, "The set, by name or id")
            .param("name", ParamKind::String, "")
            .param(
                "formula",
                ParamKind::String,
                "Such as \"0.4 mm\" or \"3 * Printer.nozzle\"",
            )
            .optional("comment", ParamKind::String, "")
            .returns("{value, text, error}: what it comes to"),
        CommandSpec::new("var.remove", "Take a variable out of its set")
            .param("set", ParamKind::String, "The set, by name or id")
            .param("name", ParamKind::String, ""),
        CommandSpec::new(
            "var.rename",
            "Rename a variable, and every formula that reads it",
        )
        .param("set", ParamKind::String, "The set, by name or id")
        .param("name", ParamKind::String, "")
        .param("to", ParamKind::String, ""),
        CommandSpec::new(
            "var.list",
            "The variable sets and what each variable comes to",
        )
        .optional("set", ParamKind::String, "Only this set, by name or id")
        .returns(
            "a list of {id, name, variables}, each variable {name, formula, value, text, \
                 kind, error, comment}",
        )
        .read_only(),
        CommandSpec::new(
            "config.list",
            "The configurations: the variables they set, each row, and which is in effect",
        )
        .returns("{columns, rows: [{name, values, left_out}], active}")
        .read_only(),
        CommandSpec::new("config.new", "Add a configuration")
            .param("name", ParamKind::String, "Such as \"Large\"")
            .optional(
                "like",
                ParamKind::String,
                "Start from this configuration's values",
            ),
        CommandSpec::new("config.remove", "Remove a configuration")
            .param("name", ParamKind::String, ""),
        CommandSpec::new("config.rename", "Rename a configuration")
            .param("name", ParamKind::String, "")
            .param("to", ParamKind::String, ""),
        CommandSpec::new(
            "config.add_variable",
            "Let the configurations set a variable: it becomes a column",
        )
        .param(
            "variable",
            ParamKind::String,
            "As formulas read it: Size.width",
        ),
        CommandSpec::new("config.remove_variable", "Take a variable's column away")
            .param(
                "variable",
                ParamKind::String,
                "As formulas read it: Size.width",
            ),
        CommandSpec::new(
            "config.set",
            "What a configuration gives a variable: a formula, or empty for its own",
        )
        .param("name", ParamKind::String, "The configuration")
        .param(
            "variable",
            ParamKind::String,
            "As formulas read it: Size.width",
        )
        .param("value", ParamKind::String, "Such as \"60 mm\""),
        CommandSpec::new(
            "config.leave_out",
            "The bodies a configuration leaves out: not drawn, picked, exported or checked",
        )
        .param("name", ParamKind::String, "The configuration")
        .param(
            "bodies",
            ParamKind::List,
            "The bodies' ids; an empty list leaves none out",
        ),
        CommandSpec::new("config.activate", "Put a configuration in effect")
            .optional("name", ParamKind::String, "Nil leaves every variable its own"),
        CommandSpec::new("var.eval", "What a formula comes to in this document")
            .param("formula", ParamKind::String, "")
            .returns("{value, kind, text}: value in mm or degrees")
            .read_only(),
    ])
    .into_iter()
    .chain(crate::app::section_commands::doc_section_commands())
    .collect()
}

/// The commands a key can run that make sense without a key: all but
/// those that open a window of the interface's own.
pub(super) fn key_commands() -> impl Iterator<Item = (CommandSpec, keymap::HostAction)> {
    use keymap::HostAction::*;
    keymap::host_actions()
        .filter(|(_, _, action)| {
            !matches!(
                action,
                Palette
                    | Preferences
                    | Console
                    | Assistant
                    | RunScript
                    | Record
                    | Delete
                    | ToggleVisibility
                    | PivotAtCursor
            )
        })
        .map(|(id, label, action)| {
            let spec = with_file_args(CommandSpec::new(id, label), action);
            // The view's commands move the camera, not the document.
            let spec = if id.starts_with("view.") {
                spec.read_only()
            } else {
                spec
            };
            (for_agents(spec, action), action)
        })
}

/// What an agent may do with a key's command. It runs inside the user's
/// session: what would end that session it never does, and what reaches
/// past the document it does only once the user allows it.
fn for_agents(spec: CommandSpec, action: keymap::HostAction) -> CommandSpec {
    use keymap::HostAction::*;
    match action {
        Quit => spec.agent_never(
            "it closes printCAD, the application the agent is running inside; only the user \
             quits it",
        ),
        CloseTab => spec.agent_never(
            "it closes a document's tab, and with it the chats working on that document; \
             only the user closes tabs",
        ),
        New | Open | SaveAs | Import | Export | SendToSlicer | Undo | Redo | NewTab | ReopenTab
        | NextTab | PreviousTab => spec.agent_always_asks(),
        _ => spec,
    }
}

/// What `file.import`'s `scan` says, the app's and the window-less host's.
pub(crate) const SCAN_IMPORT_DOC: &str = "true marks the meshes it brings as scans, the measured parts inspection and mesh \
     editing work on";

/// What `file.import`'s `units` says.
pub(crate) const UNITS_IMPORT_DOC: &str = "mm, cm, m or inch: what an STL file's numbers are in, scaled to millimetres as it \
     comes in; Preferences' STL units when left out. Other files say their own units";

/// The units `file.import` reads `path` in: an STL's from `units`, else
/// `default`; nothing for a file that says its own.
pub(crate) fn import_units(
    a: &Args,
    path: &std::path::Path,
    default: kernel_api::LengthUnit,
) -> Result<Option<kernel_api::LengthUnit>, CommandError> {
    let given = match a.opt_string("units")? {
        Some(key) => Some(
            crate::app::scans::unit_of_key(key)
                .ok_or_else(|| CommandError::bad("units", "is mm, cm, m or inch"))?,
        ),
        None => None,
    };
    if !crate::app::scans::has_no_units(path) {
        return Ok(None);
    }
    Ok(Some(given.unwrap_or(default)))
}

/// A file command takes the file by name, to run without its dialog.
fn with_file_args(spec: CommandSpec, action: keymap::HostAction) -> CommandSpec {
    use keymap::HostAction::*;
    let path = |spec: CommandSpec, doc: &str| spec.optional("path", ParamKind::String, doc);
    match action {
        Open => path(spec, "The document to open; the dialog when left out")
            .returns("nothing; the document opens in its tab after the script"),
        SaveAs => path(spec, "Where to save; the dialog when left out"),
        Import => path(
            spec,
            "The STEP, IGES, STL, OBJ, 3MF, PLY, glTF or VRML file; the dialog when left out",
        )
        .optional("scan", ParamKind::Bool, SCAN_IMPORT_DOC)
        .optional("units", ParamKind::String, UNITS_IMPORT_DOC)
        .returns("nothing; pc.doc.rebuild() waits for the import"),
        Export => export_options(path(spec, "Where to write; the dialog when left out")).note(
            "STL and 3MF make up for the printer as Preferences › Printing › Shrinkage \
                 says while it is on (app.print_compensation): round holes and bosses drawn \
                 larger or smaller, each body scaled about its middle. `printcad --script` \
                 writes the bodies as drawn.",
        ),
        SendToSlicer => spec.optional(
            "layout",
            ParamKind::Bool,
            "The print layout rather than the bodies where they sit; Preferences › 3D printing \
             says when left out",
        ),
        _ => spec,
    }
}

/// Export arguments shared by windowed and headless command catalogs.
pub(crate) fn export_options(spec: CommandSpec) -> CommandSpec {
    spec.optional(
        "format",
        ParamKind::String,
        "step, step_nurbs (every surface a spline), stl or 3mf; from the path's \
                 extension when left out",
    )
    .optional(
        "bodies",
        ParamKind::List,
        "The bodies to write; every visible one when left out",
    )
    .optional(
        "tolerance",
        ParamKind::Number,
        "The mesh formats' distance to the true surface, mm (0.01)",
    )
    .optional(
        "layout",
        ParamKind::Bool,
        "The print layout (doc.print_layout) rather than the bodies where they sit",
    )
    .returns("{path, written, skipped, triangles}")
}

/// The application's commands that are not a key's.
pub(super) fn app_commands() -> Vec<CommandSpec> {
    explained(vec![
        CommandSpec::new("app.workbenches", "The workbenches, in the order they load")
            .returns("a list of {id, label, active}")
            .read_only(),
        CommandSpec::new(
            "app.print_compensation",
            "Set what mesh exports scale each body by to make up for the printer's shrinkage",
        )
        .optional(
            "x",
            ParamKind::Number,
            "The factor along X; 1 when left out",
        )
        .optional("y", ParamKind::Number, "Along Y; 1")
        .optional("z", ParamKind::Number, "Along Z; 1")
        .optional(
            "holes",
            ParamKind::Number,
            "How much larger round holes are drawn in radius, mm (the compensation's holes.offset); 0",
        )
        .optional("bosses", ParamKind::Number, "The same of round bosses and pins; 0")
        .optional(
            "from",
            ParamKind::String,
            "What they were measured from, for the record",
        )
        .optional(
            "on",
            ParamKind::Bool,
            "Whether exports apply them; true with factors, else as it is",
        )
        .returns("{x, y, z, from, on}, or {on = false} with none"),
        CommandSpec::new("app.workbench", "Switch to a workbench").param(
            "id",
            ParamKind::String,
            "As app.workbenches lists it",
        ),
        CommandSpec::new("app.tool", "Start a toolbar tool, as a click on it does").param(
            "id",
            ParamKind::String,
            "The tool's id, as Preferences › Keyboard lists it",
        ),
        CommandSpec::new("app.tools", "The tools of a workbench")
            .optional(
                "workbench",
                ParamKind::String,
                "The active one when left out",
            )
            .returns("a list of {id, label, keys}")
            .read_only(),
        CommandSpec::new(
            "scan.mark",
            "Mark part of a scan's surface, the part the tools that work on some of a scan take",
        )
        .param("body", ParamKind::Id, "The scan")
        .optional(
            "box",
            ParamKind::Any,
            "{min = {x, y, z}, max = {x, y, z}}: points inside it, where the scan sits",
        )
        .optional(
            "ball",
            ParamKind::Any,
            "{centre = {x, y, z}, radius = r}: points within it",
        )
        .optional(
            "facing",
            ParamKind::Any,
            "{x, y, z}: only points whose surface faces this way, within `limit`",
        )
        .optional(
            "limit",
            ParamKind::Number,
            "Degrees from `facing`; 60 when left out",
        )
        .optional("erase", ParamKind::Bool, "Unmark what the region takes")
        .optional("invert", ParamKind::Bool, "Swap marked and unmarked first")
        .optional("clear", ParamKind::Bool, "Unmark everything first")
        .returns("{points, triangles}: how many are marked now"),
        crate::app::section_commands::view_section_command(),
    ])
}

/// The commands a script can call, by the application and then the
/// workbenches in registration order.
pub(super) fn all_commands(app: &PrintCadApp) -> Vec<CommandSpec> {
    command_specs(&app.registry)
}

/// Every command a script in the application can call: the application's,
/// then each workbench's in registration order.
pub(crate) fn command_specs(registry: &core_document::DocumentService) -> Vec<CommandSpec> {
    let mut out = doc_commands();
    out.extend(app_commands());
    out.extend(key_commands().map(|(spec, _)| spec));
    out.extend(registry.commands().into_iter().map(|(_, c)| c.clone()));
    out
}

/// The command reference `docs/SCRIPTING.md` carries: every command, by
/// the name before its first dot.
#[cfg(test)]
pub(crate) fn reference(commands: &[CommandSpec]) -> String {
    let mut groups: Vec<&str> = Vec::new();
    for c in commands {
        let group = c.id.split('.').next().unwrap_or("");
        if !groups.contains(&group) {
            groups.push(group);
        }
    }
    let mut out = String::new();
    for group in groups {
        out.push_str(&format!("\n### {group}\n"));
        for c in commands
            .iter()
            .filter(|c| c.id.split('.').next() == Some(group))
        {
            out.push_str(&format!(
                "\n`pc.{}`: {}.\n",
                c.id,
                c.summary.trim_end_matches('.')
            ));
            let mut lines: Vec<String> = c
                .params
                .iter()
                .map(|p| {
                    let optional = if p.required { "" } else { ", optional" };
                    let doc = if p.doc.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", p.doc)
                    };
                    format!("- `{}` ({}{optional}){doc}", p.name, p.kind.name())
                })
                .collect();
            if let Some(extra) = &c.extra_args {
                lines.push(format!("- Other arguments: {extra}"));
            }
            if c.returns != "nothing" {
                lines.push(format!("- Returns {}", c.returns));
            }
            if !lines.is_empty() {
                out.push('\n');
                out.push_str(&lines.join("\n"));
                out.push('\n');
            }
            if !c.notes.is_empty() {
                out.push_str("\nNotes:\n\n");
                for note in &c.notes {
                    out.push_str(&format!("- {note}\n"));
                }
            }
            if !c.see_also.is_empty() {
                let others: Vec<String> = c.see_also.iter().map(|s| format!("`pc.{s}`")).collect();
                out.push_str(&format!("\nSee also {}.\n", others.join(", ")));
            }
            if let Some(example) = c.examples.first() {
                out.push_str(&format!(
                    "\nExample: {}.\n\n```lua\n{}\n```\n",
                    example.title.trim_end_matches('.'),
                    example.script.trim()
                ));
            }
        }
    }
    out
}
