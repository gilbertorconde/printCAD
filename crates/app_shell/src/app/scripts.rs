//! Scripts and the console: the Lua engine run against every command the
//! application and the workbenches offer.
//!
//! The application's own commands are the document's queries and edits
//! (`doc.*`) and every command a key can run (`file.save`, `view.front`,
//! ...). A workbench's command runs in the workbench through
//! `Workbench::run_command`, with the same context its tools get, so the
//! host never needs to know what the command does. Every call's arguments
//! are checked against its spec before it runs.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, FeatureId, ParamKind,
};
use serde_json::{Value, json};
use uuid::Uuid;
use winit::event_loop::ActiveEventLoop;

use crate::PrintCadApp;
use crate::app::workbench_host::HookSite;
use crate::console::{self, LineKind};
use crate::ui::TreeItemId;
use crate::ui::keymap::{self, HostOutcome, HostState};

/// The application's own commands beyond the keyboard's.
pub(crate) fn doc_commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("doc.info", "The document's name, file and unit")
            .returns("{name, file, unit, modified}")
            .read_only()
            .note(
                "`file` is nil until the document is saved, and `modified` turns true with \
                 the first edit. `unit` is the unit numbers are shown in (\"mm\"); commands \
                 take and give lengths in millimetres whatever it is.",
            )
            .example(
                "A new document, edited",
                r#"
                local info = pc.doc.info()
                assert(info.unit == "mm" and info.file == nil and not info.modified)
                pc.sketch.new{plane = "XY"}
                assert(pc.doc.info().modified, "an edit marks it modified")
                "#,
            ),
        CommandSpec::new(
            "doc.agent_rules",
            "The rules an AI agent working on this document keeps to, beside the ones in \
             Preferences for every document",
        )
        .returns("the rules, as text")
        .read_only()
        .note("It needs the app's window: a `printcad --script` run refuses it.")
        .see_also("doc.set_agent_rules"),
        CommandSpec::new(
            "doc.set_agent_rules",
            "Set the rules an AI agent working on this document keeps to; they are saved with \
             the document",
        )
        .param("rules", ParamKind::String, "The rules, as plain text")
        .agent_never("only the user sets the rules an agent keeps to")
        .note(
            "The text replaces the rules the document had; an empty text takes them away. \
             The rules in Preferences, for every document, are kept apart and stay.",
        )
        .note("It needs the app's window: a `printcad --script` run refuses it.")
        .see_also("doc.agent_rules"),
        CommandSpec::new("doc.bodies", "List the bodies")
            .returns("a list of {id, name, visible, frozen, selectable, material, features}")
            .read_only()
            .note(
                "`features` are the body's feature ids in build order; `material` is nil \
                 until `pc.doc.set_body` gives it one.",
            )
            .note(
                "A sketch made without a `body` starts a body of its own, so one sketch \
                 and its pad are one body.",
            )
            .see_also("doc.features")
            .see_also("doc.set_body")
            .example(
                "A padded block's one body",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
                local pad = pc.design.pad{sketch = s, length = 5}
                local body = pc.doc.feature{id = pad}.body
                pc.doc.set_body{body = body, material = {name = "PLA", density = 1.24}}
                local bodies = pc.doc.bodies()
                assert(#bodies == 1 and bodies[1].id == body and bodies[1].visible)
                assert(bodies[1].features[1] == s and bodies[1].features[2] == pad)
                assert(bodies[1].material.name == "PLA")
                assert(math.abs(bodies[1].material.density - 1.24) < 1e-6)
                "#,
            ),
        CommandSpec::new("doc.features", "List the features in build order")
            .optional("body", ParamKind::Id, "Only this body's")
            .returns("a list of {id, name, kind, body, visible, suppressed, error}")
            .read_only()
            .note(
                "`error` is set only on a feature the last `pc.doc.rebuild()` could not \
                 build. Variable sets and the configurations table are listed too, with \
                 no body.",
            )
            .note(
                "A `body` that is not a body of the document gives an empty list, not an \
                 error.",
            )
            .see_also("doc.feature")
            .see_also("doc.bodies")
            .example(
                "A pad and its sketch, in order",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
                local pad = pc.design.pad{sketch = s, length = 5}
                local body = pc.doc.feature{id = pad}.body
                local list = pc.doc.features{body = body}
                assert(#list == 2)
                assert(list[1].id == s and list[1].kind == "Sketch")
                assert(list[2].id == pad and list[2].kind == "Pad" and list[2].error == nil)
                assert(not list[1].visible, "a pad hides its sketch")
                "#,
            ),
        CommandSpec::new("doc.feature", "A feature with its fields")
            .param("id", ParamKind::Id, "")
            .returns(
                "{id, name, kind, body, visible, suppressed, error, fields, unset, values}: \
                 unset names the fields holding no value, which fields leaves out; values \
                 is the data as the feature is built now (formulas worked out, a plane \
                 following what it stands on), given when it differs from fields",
            )
            .read_only()
            .note(
                "`fields` keep the plain numbers a formula stands over: read `values` for \
                 what the feature is built from. A body's id is refused (\"is not a \
                 feature of this document\").",
            )
            .see_also("doc.parameters")
            .see_also("doc.features")
            .example(
                "A pad's length, kept and worked out",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
                local pad = pc.design.pad{sketch = s, length = 5}
                local f = pc.doc.feature{id = pad}
                assert(f.kind == "Pad" and f.fields.Pad.length == 5 and f.fields.Pad.sketch == s)
                assert(f.values == nil, "no formula: values is fields")
                pc.doc.set_formula{id = pad, parameter = "length", formula = "2 * 4 mm"}
                f = pc.doc.feature{id = pad}
                assert(f.fields.Pad.length == 5 and f.values.Pad.length == 8)
                "#,
            ),
        CommandSpec::new("doc.selection", "What is selected")
            .returns("{item, body, feature}, each an id or nil")
            .read_only()
            .note(
                "`item` is the tree row selected, `body` the body being worked on and \
                 `feature` the feature being worked on. It needs the app's window: a \
                 `printcad --script` run refuses it.",
            )
            .see_also("doc.select"),
        CommandSpec::new(
            "doc.select",
            "Select a body or a feature, as a click on its row",
        )
        .param("id", ParamKind::Id, "")
        .note(
            "It takes a body, a feature, an imported part or a component; any other id is \
             refused (\"is not in this document\"). Selecting a feature moves its body's \
             tip to it, as a click on its row does.",
        )
        .note("It needs the app's window: a `printcad --script` run refuses it.")
        .see_also("doc.selection")
        .see_also("doc.set_tip"),
        CommandSpec::new("doc.new_body", "Make an empty body")
            .optional("name", ParamKind::String, "")
            .returns("the body's id")
            .note(
                "Without a name it is called `body`, `body_1` and so on. Features go into \
                 it only when given its id as `body`: `pc.sketch.new` without one starts a \
                 body of its own.",
            )
            .note(
                "It has no solid until a feature in it builds: `pc.doc.measure` refuses it \
                 (\"the body has no solid yet\").",
            )
            .see_also("doc.bodies")
            .example(
                "A named body with a pad in it",
                r#"
                local lid = pc.doc.new_body{name = "Lid"}
                local s = pc.sketch.new{plane = "XY", body = lid}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
                local pad = pc.design.pad{sketch = s, length = 2}
                assert(pc.doc.feature{id = pad}.body == lid)
                assert(#pc.doc.rebuild() == 0)
                assert(math.abs(pc.doc.measure{body = lid}.volume - 400) < 1e-6)
                assert(pc.doc.bodies()[1].name == "Lid")
                "#,
            ),
        CommandSpec::new("doc.rename", "Rename a body or a feature")
            .param("id", ParamKind::Id, "")
            .param("name", ParamKind::String, "")
            .note(
                "Every formula that reads a renamed feature is rewritten to the new name. \
                 Two features may share a name; an empty name leaves the name as it was.",
            )
            .note("A variable set is a feature: this renames it too.")
            .see_also("var.rename")
            .example(
                "A formula follows a renamed pad",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
                local pad = pc.design.pad{sketch = s, length = 5}
                pc.doc.set_formula{id = pad, parameter = "length2", formula = "Pad.length * 2"}
                pc.doc.rename{id = pad, name = "Base"}
                assert(pc.doc.feature{id = pad}.name == "Base")
                assert(pc.doc.parameters{id = pad}[2].formula == "Base.length * 2")
                pc.doc.rename{id = pc.doc.feature{id = pad}.body, name = "Bracket"}
                assert(pc.doc.bodies()[1].name == "Bracket")
                "#,
            ),
        CommandSpec::new(
            "doc.set_body",
            "Change a body: its colour, how much shows through, material, placement, whether \
             it is frozen or clicks pick it",
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
            "selectable",
            ParamKind::Bool,
            "false lets clicks pass through it",
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
        )
        .note(
            "Only what is given changes. `translation` and `rotation` set the placement \
             outright, not added to the one it has; the turn is about the body's own \
             origin, and either may be a list ({100, 0, 0}) as well as named parts.",
        )
        .note(
            "Colour parts are kept between 0 and 1 and `opacity` between 0.05 and 1. A \
             `material` without a density above 0 is refused; its name defaults to \
             \"Material\".",
        )
        .note(
            "While `frozen`, edits to its features build nothing and `pc.doc.rebuild()` \
             reports nothing for it; thawing builds what changed meanwhile.",
        )
        .see_also("doc.set_face_color")
        .see_also("doc.measure")
        .example(
            "A block moved and turned",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            assert(#pc.doc.rebuild() == 0)
            pc.doc.set_body{body = body, translation = {x = 100, y = 0, z = 0}, color = {r = 0.9, g = 0.4, b = 0.1}}
            local m = pc.doc.measure{body = body}
            assert(math.abs(m.min[1] - 100) < 1e-4 and math.abs(m.max[1] - 120) < 1e-4)
            -- A quarter turn about Z, about the body's origin.
            local h = math.sqrt(0.5)
            pc.doc.set_body{body = body, rotation = {x = 0, y = 0, z = h, w = h}}
            m = pc.doc.measure{body = body}
            assert(math.abs(m.min[1] - 90) < 1e-4 and math.abs(m.max[2] - 20) < 1e-4)
            assert(math.abs(m.volume - 1000) < 1e-6)
            "#,
        ),
        CommandSpec::new(
            "doc.set_textures",
            "Set every surface texture pressed into a body's faces for printing: drawn in the \
             view, baked into STL and 3MF files and what goes to the slicer",
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
        )
        .note(
            "Each call replaces every texture the body had. A texture needs `pattern`, \
             `projection`, `tile_mm` and `depth_mm`; `rotation_deg`, `inward` and \
             `keep_flat_deg` default to 0 and false.",
        )
        .note(
            "The solid keeps its shape: `pc.doc.measure` and `pc.doc.faces` read it \
             untextured. The texture is pressed into the mesh STL and 3MF exports write.",
        )
        .note(
            "Faces given by number need the body built (\"the body has no solid yet\"); \
             a texture with no faces needs none.",
        )
        .see_also("doc.faces")
        .see_also("file.export")
        .example(
            "Ribs on the top face, in the exported mesh",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 6, height = 6}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            assert(#pc.doc.rebuild() == 0)
            local top
            for _, face in ipairs(pc.doc.faces{body = body}) do
              if face.kind == "plane" and face.normal[3] > 0.99 then top = face.index end
            end
            local path = os.tmpname()
            local function triangles()
              local out = pc.file.export{path = path, format = "stl"}
              os.remove(out.path)
              return out.triangles
            end
            local plain = triangles()
            pc.doc.set_textures{body = body, textures = {{
              texture = {pattern = "Ribs", projection = {Planar = "Z"}, tile_mm = 3, depth_mm = 0.5},
              faces = {top},
            }}}
            assert(triangles() > 100 * plain, "the ribs are pressed into the exported mesh")
            assert(math.abs(pc.doc.measure{body = body}.volume - 180) < 1e-6, "the solid keeps its shape")
            pc.doc.set_textures{body = body, textures = {}}
            assert(triangles() == plain)
            os.remove(path)
            "#,
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
        )
        .note(
            "It needs the body built: before `pc.doc.rebuild()` it is refused (\"the body \
             has no solid yet\"), and a face number past the last is refused too.",
        )
        .note(
            "The colour can be a list ({1, 0, 0}) as well as named parts. \
             `pc.doc.set_body{body = ..., face_colors = {}}` takes every face's colour \
             away at once.",
        )
        .see_also("doc.faces")
        .see_also("doc.set_body")
        .example(
            "A pad's top painted red in a picture, then the body's colour again",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            assert(#pc.doc.rebuild() == 0)
            local top
            for _, face in ipairs(pc.doc.faces{body = body}) do
              if face.kind == "plane" and face.normal[3] > 0.99 then top = face.index end
            end
            local path = os.tmpname()
            local function picture()
              pc.doc.picture{path = path, view = "top", size = {64, 64}}
              local file = io.open(path, "rb")
              local bytes = file:read("a")
              file:close()
              return bytes
            end
            local plain = picture()
            pc.doc.set_face_color{body = body, face = top, color = {r = 1, g = 0, b = 0}}
            local red = picture()
            pc.doc.set_face_color{body = body, face = top}
            assert(red ~= plain and picture() == plain, "red, then the body's colour again")
            os.remove(path)
            "#,
        ),
        CommandSpec::new(
            "doc.linked_copy",
            "A linked copy of a body beside it: the same shape, following every change",
        )
        .param("body", ParamKind::Id, "")
        .returns("the copy's id")
        .note(
            "The copy stands 10 mm clear of the original along X, placed by its own \
             placement (`pc.doc.set_body`). It follows the original's changes once they \
             are built, and takes no features of its own.",
        )
        .see_also("doc.set_body")
        .example(
            "A copy that follows its original",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            assert(#pc.doc.rebuild() == 0)
            local copy = pc.doc.linked_copy{body = body}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = copy}
            assert(math.abs(m.volume - 1000) < 1e-6)
            assert(math.abs(m.min[1] - 30) < 1e-4, "10 mm clear of the original along X")
            pc.doc.set_value{id = pad, parameter = "length", value = 8}
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = copy}.volume - 1600) < 1e-6, "it follows")
            "#,
        ),
        CommandSpec::new(
            "doc.move_after",
            "Move a feature in its body's history to just after another",
        )
        .param("id", ParamKind::Id, "The feature")
        .optional(
            "after",
            ParamKind::Id,
            "The feature it goes after; first in its body when left out",
        )
        .note(
            "It refuses to carry a feature past one it is built from or one built from \
             it (\"depend on each other and keep their order\"), and two features of \
             different bodies (\"not in one body\"). The whole move is checked before \
             anything moves: a refused one leaves the history as it was. Moving a feature \
             after itself does nothing.",
        )
        .note(
            "Whether the feature still builds in its new place is told by \
             `pc.doc.rebuild()`, not here.",
        )
        .see_also("doc.move")
        .example(
            "A sketch brought forward, its pocket kept after the pad",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            local top = pc.sketch.new{plane = "XY", offset = 5, body = body}
            pc.sketch.circle{sketch = top, x = 10, y = 5, radius = 2}
            local hole = pc.design.pocket{sketch = top, depth = 3}
            local ok, why = pcall(pc.doc.move_after, {id = hole, after = s})
            assert(not ok and tostring(why):find("depend on each other"), tostring(why))
            pc.doc.move_after{id = top, after = s}
            local order = pc.doc.features{body = body}
            assert(order[1].id == s and order[2].id == top and order[3].id == pad and order[4].id == hole)
            assert(#pc.doc.rebuild() == 0)
            "#,
        ),
        CommandSpec::new("doc.recompute", "Build a body again from its history")
            .param("body", ParamKind::Id, "")
            .note(
                "It builds nothing itself: it marks every feature of the body for building \
                 from the start, and `pc.doc.rebuild()` builds them and waits. Edits mark \
                 what they change already, so a script needs it only to build a body \
                 afresh.",
            )
            .note("A feature's id is refused (\"is not a body of this document\").")
            .note(
                "A frozen body is refused (\"is frozen\"): it keeps the solid it has. \
                 Thawed with `pc.doc.set_body{body = ..., frozen = false}`, it builds what \
                 changed meanwhile.",
            )
            .see_also("doc.rebuild")
            .see_also("doc.set_body")
            .example(
                "A body built again from its sketch",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
                local pad = pc.design.pad{sketch = s, length = 5}
                local body = pc.doc.feature{id = pad}.body
                assert(#pc.doc.rebuild() == 0)
                pc.doc.recompute{body = body}
                assert(#pc.doc.rebuild() == 0, "built again from the sketch")
                assert(math.abs(pc.doc.measure{body = body}.volume - 1000) < 1e-6)
                pc.doc.set_body{body = body, frozen = true}
                local ok, why = pcall(pc.doc.recompute, {body = body})
                assert(not ok and tostring(why):find("is frozen"), tostring(why))
                assert(math.abs(pc.doc.measure{body = body}.volume - 1000) < 1e-6, "kept")
                "#,
            ),
        CommandSpec::new("doc.set_visible", "Show or hide a body or a feature")
            .param("id", ParamKind::Id, "")
            .param("visible", ParamKind::Bool, "")
            .note(
                "Hiding changes what is drawn and picked, not what is built: a hidden body \
                 still builds and measures, and a hidden feature stays in its body's solid \
                 (`pc.doc.suppress` takes it out).",
            )
            .note(
                "`pc.doc.picture` leaves hidden bodies out unless they are named in its \
                 `bodies`.",
            )
            .see_also("doc.suppress")
            .example(
                "A sketch shown, a body hidden",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
                local pad = pc.design.pad{sketch = s, length = 5}
                local body = pc.doc.feature{id = pad}.body
                pc.doc.set_visible{id = s, visible = true}
                assert(pc.doc.feature{id = s}.visible)
                pc.doc.set_visible{id = body, visible = false}
                assert(not pc.doc.bodies()[1].visible)
                assert(#pc.doc.rebuild() == 0)
                assert(math.abs(pc.doc.measure{body = body}.volume - 1000) < 1e-6, "hidden, still built")
                "#,
            ),
        CommandSpec::new("doc.delete", "Delete a body or a feature")
            .param("id", ParamKind::Id, "")
            .note("A body goes with every feature in it.")
            .note(
                "A feature goes alone: deleting a pad keeps its sketch, and deleting a sketch \
                 a later feature uses is not refused; that feature then fails at \
                 `pc.doc.rebuild()` (\"references a missing sketch\").",
            )
            .see_also("doc.suppress")
            .example(
                "A fillet taken away again",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 20}
                local pad = pc.design.pad{sketch = s, length = 5}
                local body = pc.doc.feature{id = pad}.body
                local round = pc.design.fillet{body = body, radius = 1}
                pc.doc.delete{id = round}
                assert(#pc.doc.rebuild() == 0)
                assert(#pc.doc.faces{body = body} == 6, "the block is square again")
                assert(#pc.doc.features{body = body} == 2, "the sketch and the pad")
                "#,
            ),
        CommandSpec::new(
            "doc.repair",
            "Repair the shapes the kernel's checker calls broken",
        )
        .param("bodies", ParamKind::List, "The bodies")
        .returns("nothing; pc.doc.rebuild() waits for the repair")
        .note(
            "It asks for the repair of imported, converted and based solids; any other \
             body, or one whose repair is already asked for, is passed over without an \
             error. Asking clears the undo history, as an import does.",
        )
        .note("It needs the app's window: a `printcad --script` run refuses it.")
        .see_also("doc.rebuild"),
        CommandSpec::new("doc.convert_to_solid", "Turn mesh bodies into solids")
            .param("bodies", ParamKind::List, "The mesh bodies")
            .returns("nothing; pc.doc.rebuild() waits for the conversion")
            .note(
                "Only mesh bodies (imported STL, OBJ, 3MF and the like) are converted; any \
                 other body is passed over without an error. Asking clears the undo \
                 history, as an import does.",
            )
            .note(
                "Curved stretches stay facets: `pc.doc.refine` rebuilds them on true \
                 surfaces. It needs the app's window: a `printcad --script` run refuses it.",
            )
            .see_also("doc.refine")
            .see_also("doc.rebuild"),
        CommandSpec::new(
            "doc.refine",
            "Rebuild converted solids' facets on the cylinders, cones, spheres and tori they \
             approximate",
        )
        .param("bodies", ParamKind::List, "The converted bodies")
        .returns("nothing; pc.doc.rebuild() waits for the refine")
        .note(
            "Only a converted body still made of facets, with no base shape, is refined; \
             any other body is passed over without an error. Asking clears the undo \
             history.",
        )
        .note("It needs the app's window: a `printcad --script` run refuses it.")
        .see_also("doc.convert_to_solid"),
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
        .agent_always_asks()
        .note(
            "A body that builds its shape from its own history, a linked copy and a part \
             linked from another file are refused; a file that cannot be read is refused \
             before anything changes. The file is kept in the document, and the change \
             clears the undo history, as an import does.",
        )
        .note("It needs the app's window: a `printcad --script` run refuses it.")
        .see_also("doc.rebuild"),
        CommandSpec::new(
            "doc.suppress",
            "Leave a feature out of its body's solid, or back in",
        )
        .param("id", ParamKind::Id, "The feature")
        .optional("suppressed", ParamKind::Bool, "true (the default) or false")
        .note(
            "The feature stays in the history with its data and is built again once \
             unsuppressed; `pc.doc.rebuild()` builds the body without it. A body's id is \
             refused (\"is not a feature of this document\").",
        )
        .see_also("doc.set_visible")
        .see_also("doc.set_tip")
        .see_also("doc.delete")
        .example(
            "A pocket left out and put back",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            local top = pc.sketch.new{plane = "XY", offset = 5, body = body}
            pc.sketch.circle{sketch = top, x = 10, y = 5, radius = 2}
            local hole = pc.design.pocket{sketch = top, depth = 3}
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = body}.volume - (1000 - math.pi * 4 * 3)) < 1e-3)
            pc.doc.suppress{id = hole}
            assert(pc.doc.feature{id = hole}.suppressed)
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = body}.volume - 1000) < 1e-6, "the plain block")
            pc.doc.suppress{id = hole, suppressed = false}
            assert(#pc.doc.rebuild() == 0)
            assert(pc.doc.measure{body = body}.volume < 1000, "cut again")
            "#,
        ),
        CommandSpec::new("doc.move", "Move a feature one step in its body's history")
            .param("id", ParamKind::Id, "The feature")
            .param("up", ParamKind::Bool, "true: earlier, false: later")
            .returns(
                "true; a move past the end of the history, or past a feature one of the two \
                 is built from, fails saying so",
            )
            .note(
                "It changes the order the features build in; `pc.doc.set_tip` changes how \
                 far the body builds and leaves the order alone.",
            )
            .note(
                "Only what one feature is built from is checked: a fillet moved above the \
                 pad it rounds is let through, and `pc.doc.rebuild()` then reports it \
                 (\"needs existing material\").",
            )
            .see_also("doc.move_after")
            .see_also("doc.set_tip")
            .example(
                "A pad stays after its sketch; a fillet moved before it fails",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
                local pad = pc.design.pad{sketch = s, length = 5}
                local body = pc.doc.feature{id = pad}.body
                local round = pc.design.fillet{body = body, radius = 1}
                local ok, why = pcall(pc.doc.move, {id = pad, up = true})
                assert(not ok and tostring(why):find("built from"), tostring(why))
                assert(pc.doc.move{id = round, up = true})
                assert(pc.doc.features{body = body}[2].id == round)
                local failed = pc.doc.rebuild()
                assert(#failed == 2 and failed[1].feature == round, "a fillet before the pad has nothing to round")
                "#,
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
        )
        .note(
            "The features after the tip stay in the history, left out of the solid until \
             the tip moves past them; `clear = true` takes the tip away, whichever of the \
             body's features `id` names.",
        )
        .note(
            "A feature made while the tip is set goes in right after it, and the tip moves \
             to the new feature.",
        )
        .see_also("doc.move")
        .see_also("doc.suppress")
        .example(
            "A body built only up to its pad",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            local top = pc.sketch.new{plane = "XY", offset = 5, body = body}
            pc.sketch.circle{sketch = top, x = 10, y = 5, radius = 2}
            local hole = pc.design.pocket{sketch = top, depth = 3}
            pc.doc.set_tip{id = pad}
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = body}.volume - 1000) < 1e-6, "built up to the pad")
            pc.doc.set_tip{id = pad, clear = true}
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = body}.volume - (1000 - math.pi * 12)) < 1e-3)
            "#,
        ),
        CommandSpec::new(
            "doc.rebuild",
            "Rebuild every solid that changed, repair or convert what was asked, and wait",
        )
        .optional("timeout", ParamKind::Number, "Seconds to wait at most (60)")
        .returns("a list of {feature, error} for every feature that failed")
        .read_only()
        .note(
            "Commands that make or change features build nothing; this builds them, and is \
             where a feature that cannot build is told. An empty list means every feature \
             built: check `#pc.doc.rebuild() == 0` before measuring.",
        )
        .note(
            "A body whose feature failed keeps the solid of the history before that feature, \
             so a measurement after a failure measures the earlier solid.",
        )
        .see_also("doc.measure")
        .see_also("doc.faces")
        .example(
            "A profile that does not close is told here",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.polyline{sketch = s, points = {{0, 0}, {10, 0}, {10, 10}}}
            local pad = pc.design.pad{sketch = s, length = 5}
            local failed = pc.doc.rebuild()
            assert(#failed == 1 and failed[1].feature == pad, "the pad failed")
            assert(failed[1].error:find("not closed"), failed[1].error)
            "#,
        ),
        CommandSpec::new("doc.faces", "The faces of a body's solid, where it sits")
            .param("body", ParamKind::Id, "")
            .optional(
                "frame",
                ParamKind::String,
                "world (the default): where the body sits; body: in the body's own frame, \
                 as features take faces and edges",
            )
            .returns(
                "a list of {index, kind, point, area, normal?, axis?, radius?, name?}: \
                 point lies on the face, normal is a flat face's outward one, \
                 axis a turned face's {point, direction}",
            )
            .read_only()
            .note(
                "It reads the built solid: run `pc.doc.rebuild()` first; a body not yet \
                 built is refused (\"the body has no solid yet\").",
            )
            .note(
                "Points, normals and axes are in world space, where the body sits; features \
                 take faces in the body's own frame, the same unless the body was moved: \
                 for a moved body ask with frame = \"body\" to pass a face to a feature.",
            )
            .note(
                "Every rebuild numbers the faces afresh: find a face by its kind, normal \
                 and point in the same script rather than keep its index. `name` is a \
                 string.",
            )
            .see_also("doc.measure")
            .example(
                "A moved block's top, where it sits and as a feature takes it",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
                local body = pc.doc.feature{id = pc.design.pad{sketch = s, length = 4}}.body
                pc.asm.place{body = body, translation = {0, 0, 50}}
                assert(#pc.doc.rebuild() == 0)
                local function top(faces)
                  for _, face in ipairs(faces) do
                    if face.kind == "plane" and face.normal[3] > 0.99 then return face end
                  end
                end
                assert(math.abs(top(pc.doc.faces{body = body}).point[3] - 54) < 1e-6)
                local own = top(pc.doc.faces{body = body, frame = "body"})
                assert(math.abs(own.point[3] - 4) < 1e-6, "4 in the body's own frame")
                "#,
            )
            .example(
                "The top face of a block found by its normal",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
                local pad = pc.design.pad{sketch = s, length = 4}
                assert(#pc.doc.rebuild() == 0)
                local faces = pc.doc.faces{body = pc.doc.feature{id = pad}.body}
                assert(#faces == 6)
                local top
                for _, face in ipairs(faces) do
                  if face.kind == "plane" and face.normal[3] > 0.99 then top = face end
                end
                assert(top and math.abs(top.point[3] - 4) < 1e-6, "the top is at z = 4")
                assert(math.abs(top.area - 200) < 1e-3)
                assert(type(top.name) == "string")
                "#,
            ),
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
            .read_only()
            .note(
                "It reads the built solid: run `pc.doc.rebuild()` first; a body not yet \
                 built is refused (\"the body has no solid yet\").",
            )
            .note(
                "`length` is measured along the drawn outline, a little under a curved \
                 edge's true length (31.40 for a 5 mm circle's 31.42). The seam of a \
                 turned face is an edge with one face in `faces`.",
            )
            .note(
                "Every rebuild numbers the edges afresh, as it does the faces: find an \
                 edge by its kind, point and faces in the same script.",
            )
            .note(
                "As doc.faces, world space unless frame = \"body\", which is what a \
                 fillet or chamfer of a moved body takes.",
            )
            .see_also("doc.faces")
            .example(
                "A cylinder's rims and seam",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 5}
                local pad = pc.design.pad{sketch = s, length = 10}
                assert(#pc.doc.rebuild() == 0)
                local body = pc.doc.feature{id = pad}.body
                local rims, seams = 0, 0
                for _, edge in ipairs(pc.doc.edges{body = body}) do
                  if edge.kind == "circle" then
                    rims = rims + 1
                    assert(math.abs(edge.radius - 5) < 1e-6 and #edge.faces == 2)
                    assert(math.abs(edge.length - 2 * math.pi * 5) < 0.05, "measured along the outline")
                  elseif #edge.faces == 1 then
                    seams = seams + 1
                    assert(edge.kind == "line" and math.abs(edge.length - 10) < 1e-6)
                  end
                end
                assert(rims == 2 and seams == 1, "the top and bottom rims and the side's seam")
                "#,
            ),
        CommandSpec::new(
            "doc.measure",
            "A body's volume, surface area, centre and bounds",
        )
        .param("body", ParamKind::Id, "")
        .returns("{volume, area, centre, min, max, approximate}")
        .read_only()
        .note(
            "It reads the built solid: run `pc.doc.rebuild()` first; a body not yet built is \
             refused (\"the body has no solid yet\").",
        )
        .note(
            "Volume is in mm³ and area in mm², the bounds and centre in world space. \
             `approximate` is true when some face had no closed form and the figures were \
             summed over its triangles: close (a fraction of a percent) rather than exact. \
             A mesh body is refused; it has no solid to measure.",
        )
        .see_also("doc.rebuild")
        .see_also("doc.faces")
        .example(
            "A cylinder's volume and bounds",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 5}
            local pad = pc.design.pad{sketch = s, length = 10}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - math.pi * 25 * 10) < 1e-3, m.volume)
            assert(math.abs(m.max[3] - 10) < 1e-6 and math.abs(m.min[1] + 5) < 1e-6)
            assert(math.abs(m.centre[3] - 5) < 1e-6)
            "#,
        ),
        CommandSpec::new(
            "doc.print_layout",
            "Where the print layout puts each copy: every part flat on its resting face, \
             as many as the parts list prints, packed on the bed",
        )
        .optional(
            "bed",
            ParamKind::Any,
            "{x, y, z}: the bed's width, depth and build height, mm; Preferences › Printing's \
             when left out",
        )
        .optional(
            "gap",
            ParamKind::Number,
            "Between copies, mm; Preferences › Printing's (5) when left out",
        )
        .returns(
            "{plates, pieces = {{body, name, plate, min = {x, y, z}, max = {x, y, z}, \
             transform}}, too_big, too_tall, unbuilt}",
        )
        .read_only()
        .note(
            "Nothing moves: the layout is where export (`file.export{layout = true}`) and \
             the slicer put the copies. `transform` is the rigid row-major 4×4 matrix from \
             the body's own frame to the bed; `min` and `max` are the copy's bounds there.",
        )
        .note(
            "A part rests on its largest flat face that has the whole part on one side; \
             among faces of nearly that area, the one leaving it lowest. Parts too wide for \
             the bed go on a plate of their own and are named in `too_big`; plates after the \
             first lie beside it along X.",
        )
        .see_also("asm.parts")
        .see_also("asm.part")
        .see_also("file.export")
        .example(
            "Three posts laid down on the bed",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 20}
            local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 60}}.body
            assert(#pc.doc.rebuild() == 0)
            pc.asm.part{body = post, print = 3}
            local layout = pc.doc.print_layout{bed = {x = 200, y = 200, z = 200}, gap = 5}
            assert(layout.plates == 1 and #layout.pieces == 3)
            for _, p in ipairs(layout.pieces) do
              assert(math.abs(p.min[3]) < 1e-3, "on the bed")
              assert(math.abs(p.max[3] - 10) < 1e-3, "lying on a 20 × 60 side")
              assert(p.min[1] >= 2.5 - 1e-3 and p.max[1] <= 197.5 + 1e-3)
            end
            "#,
        ),
        crate::proof::with_arguments(
            CommandSpec::new(
                "doc.picture",
                "Write a PNG of the bodies, the same for the same arguments whatever the \
                 user's camera (except view \"current\")",
            )
            .param("path", ParamKind::String, "Where to write the PNG"),
        )
        .returns("{path, width, height}")
        .read_only()
        .note(
            "It draws the solids as they stand: run `pc.doc.rebuild()` first. With no \
             visible body it is refused (\"Nothing is visible to draw\"); hidden bodies \
             are drawn only when named in `bodies`.",
        )
        .note(
            "Give a `view`: the default, \"current\", follows the user's view direction. A \
             body named in `bodies` or `highlight` that does not exist, or a face past the \
             body's last, is refused.",
        )
        .note(
            "Each side of `size` is kept between 16 and 2048 pixels. Folders missing from \
             `path` are made.",
        )
        .see_also("doc.faces")
        .see_also("doc.edges")
        .example(
            "A small isometric picture",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            assert(#pc.doc.rebuild() == 0)
            local path = os.tmpname()
            local shot = pc.doc.picture{path = path, view = "iso", size = {160, 120}}
            assert(shot.path == path and shot.width == 160 and shot.height == 120)
            local file = io.open(path, "rb")
            assert(file:read(4) == "\137PNG")
            file:close()
            os.remove(path)
            "#,
        ),
        CommandSpec::new(
            "doc.parameters",
            "A feature's numbers that formulas set and read",
        )
        .param("id", ParamKind::Id, "The feature")
        .returns(
            "a list of {name, key, label, kind, value, text, formula, error}: name is what \
             formulas call it (nil when they cannot), value in mm or degrees",
        )
        .read_only()
        .note(
            "`name` or `key` is what `pc.doc.set_formula` and `pc.doc.set_value` take; \
             `label` is only for showing. `value` is what the number comes to now, \
             formulas worked out, and `text` shows it in the document's unit.",
        )
        .note(
            "A sketch lists only its named dimensions, so a sketch without any lists \
             none. An id that is not a feature is refused (\"no such feature\").",
        )
        .see_also("doc.set_formula")
        .see_also("doc.set_value")
        .example(
            "A pad's length as formulas see it",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local length
            for _, p in ipairs(pc.doc.parameters{id = pad}) do
              if p.name == "length" then length = p end
            end
            assert(length.key == "/Pad/length" and length.kind == "length")
            assert(length.value == 5 and length.text == "5 mm" and length.formula == nil)
            "#,
        ),
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
        .returns("{value, text, error}: what it comes to")
        .note(
            "A formula that does not parse is refused. One that parses but does not work \
             out (a missing name, a loop, an angle where a length is wanted) is kept: the \
             answer has `error` and no `value`, the feature builds from the plain number \
             its fields keep, and `pc.doc.rebuild()` does not list it. Check the answer's \
             `error`.",
        )
        .note(
            "A bare number takes the parameter's unit: \"12\" is 12 mm for a length and \
             12 degrees for an angle. The fields keep their plain number; \
             `pc.doc.feature`'s `values` show what the formula gives.",
        )
        .note(
            "Taking the formula away keeps the number as it stands: the fields take what \
             the formula came to. A formula that did not work out leaves the plain number \
             they had.",
        )
        .note(
            "It sets one feature's number; `pc.var.set` defines a variable that formulas \
             read.",
        )
        .see_also("doc.parameters")
        .see_also("doc.set_value")
        .see_also("var.set")
        .example(
            "A pad's length following a variable",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            pc.var.new{name = "Printer"}
            pc.var.set{set = "Printer", name = "nozzle", formula = "0.4 mm"}
            local got = pc.doc.set_formula{id = pad, parameter = "length", formula = "Printer.nozzle * 10"}
            assert(math.abs(got.value - 4) < 1e-9 and got.error == nil)
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = body}.volume - 800) < 1e-6)
            pc.var.set{set = "Printer", name = "nozzle", formula = "0.6 mm"}
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = body}.volume - 1200) < 1e-6, "the pad follows")
            local kept = pc.doc.set_formula{id = pad, parameter = "length"}
            assert(math.abs(kept.value - 6) < 1e-9 and kept.formula == nil, "the 6 mm it came to stays")
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = body}.volume - 1200) < 1e-6)
            local bad = pc.doc.set_formula{id = pad, parameter = "length", formula = "30 deg"}
            assert(bad.value == nil and bad.error:find("angle"), bad.error)
            "#,
        ),
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
        .param("value", ParamKind::Number, "In millimetres or degrees")
        .note(
            "The value is not checked against what the feature accepts: a negative pad \
             length is taken, and `pc.doc.rebuild()` reports it (\"length must be \
             positive\").",
        )
        .see_also("doc.set_formula")
        .see_also("doc.parameters")
        .example(
            "A length set by hand over a formula",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            pc.doc.set_formula{id = pad, parameter = "length", formula = "2 * 4 mm"}
            pc.doc.set_value{id = pad, parameter = "length", value = 7}
            local length = pc.doc.parameters{id = pad}[1]
            assert(length.name == "length" and length.value == 7 and length.formula == nil)
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = body}.volume - 1400) < 1e-6)
            "#,
        ),
        CommandSpec::new("var.new", "Make a variable set")
            .param(
                "name",
                ParamKind::String,
                "What formulas call it: Printer.nozzle",
            )
            .returns("the set's id")
            .note(
                "A name a feature or another set already has is refused (\"something is \
                 already called ...\"), and so is an empty one. A name with spaces is read \
                 in backticks: `` `My set`.width ``.",
            )
            .note("The set is a feature with no body: `pc.doc.rename` renames it.")
            .see_also("var.set")
            .see_also("var.list")
            .example(
                "A set and one variable",
                r#"
                local set = pc.var.new{name = "Printer"}
                pc.var.set{set = set, name = "nozzle", formula = "0.4 mm"}
                local sets = pc.var.list()
                assert(#sets == 1 and sets[1].id == set and sets[1].name == "Printer")
                assert(pc.var.eval{formula = "Printer.nozzle"}.value == 0.4)
                local ok, why = pcall(pc.var.new, {name = "Printer"})
                assert(not ok and tostring(why):find("already called"), tostring(why))
                "#,
            ),
        CommandSpec::new("var.set", "Set a variable to a formula, adding it when new")
            .param("set", ParamKind::String, "The set, by name or id")
            .param("name", ParamKind::String, "")
            .param(
                "formula",
                ParamKind::String,
                "Such as \"0.4 mm\" or \"3 * Printer.nozzle\"",
            )
            .optional("comment", ParamKind::String, "")
            .returns("{value, text, error}: what it comes to")
            .note(
                "A formula that does not parse is refused; one that does not work out (a \
                 missing name, a loop, a length added to an angle) is kept, and the answer \
                 has `error` and no `value`.",
            )
            .note(
                "A bare number is a plain number, not a length: write the unit (\"0.4 \
                 mm\"). Setting a variable again keeps its comment unless a new one is \
                 given.",
            )
            .note(
                "Every formula reading it follows; `pc.doc.rebuild()` builds what moved. \
                 `pc.doc.set_formula` is what puts a formula on a feature's number.",
            )
            .see_also("var.eval")
            .see_also("doc.set_formula")
            .see_also("config.set")
            .example(
                "A wall that follows the nozzle",
                r#"
                pc.var.new{name = "Printer"}
                pc.var.set{set = "Printer", name = "nozzle", formula = "0.4 mm"}
                local wall = pc.var.set{set = "Printer", name = "wall", formula = "3 * Printer.nozzle"}
                assert(math.abs(wall.value - 1.2) < 1e-9 and wall.kind == "length")
                pc.var.set{set = "Printer", name = "nozzle", formula = "0.6 mm", comment = "hardened steel"}
                assert(math.abs(pc.var.eval{formula = "Printer.wall"}.value - 1.8) < 1e-9, "wall follows")
                local loop = pc.var.set{set = "Printer", name = "loop", formula = "Printer.loop + 1"}
                assert(loop.value == nil and loop.error:find("loop"), "kept, with its error")
                "#,
            ),
        CommandSpec::new("var.remove", "Take a variable out of its set")
            .param("set", ParamKind::String, "The set, by name or id")
            .param("name", ParamKind::String, "")
            .note(
                "It is not refused while formulas read it: they fail from then on \
                 (\"Printer has no nozzle\"). A configurations column naming it stays, and \
                 applies again to one added under that name.",
            )
            .see_also("var.rename")
            .see_also("config.remove_variable")
            .example(
                "A variable taken away from under a formula",
                r#"
                pc.var.new{name = "Printer"}
                pc.var.set{set = "Printer", name = "nozzle", formula = "0.4 mm"}
                pc.var.set{set = "Printer", name = "wall", formula = "3 * Printer.nozzle"}
                pc.var.remove{set = "Printer", name = "nozzle"}
                local vars = pc.var.list{set = "Printer"}[1].variables
                assert(#vars == 1 and vars[1].name == "wall")
                assert(vars[1].value == nil and vars[1].error:find("no nozzle"), "what read it fails")
                "#,
            ),
        CommandSpec::new(
            "var.rename",
            "Rename a variable, and every formula that reads it",
        )
        .param("set", ParamKind::String, "The set, by name or id")
        .param("name", ParamKind::String, "")
        .param("to", ParamKind::String, "")
        .note(
            "A name the set does not have, or a new name it already has, is refused. The \
             configurations column follows the new name too.",
        )
        .note("`pc.doc.rename` renames the set itself.")
        .see_also("doc.rename")
        .example(
            "A pad's formula follows the new name",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            pc.var.new{name = "Size"}
            pc.var.set{set = "Size", name = "h", formula = "8 mm"}
            pc.doc.set_formula{id = pad, parameter = "length", formula = "Size.h"}
            pc.var.rename{set = "Size", name = "h", to = "height"}
            assert(pc.doc.parameters{id = pad}[1].formula == "Size.height")
            assert(pc.doc.parameters{id = pad}[1].value == 8)
            "#,
        ),
        CommandSpec::new(
            "var.list",
            "The variable sets and what each variable comes to",
        )
        .optional("set", ParamKind::String, "Only this set, by name or id")
        .returns(
            "a list of {id, name, variables}, each variable {name, formula, value, text, \
                 kind, error, comment}",
        )
        .read_only()
        .note(
            "`formula` is the variable's own; `value` is what it comes to now, which is \
             the configuration's value while one in effect sets it. A variable whose \
             formula does not work out has `error` and no `value`, `text` or `kind`.",
        )
        .note("A `set` that does not exist is refused (\"no variable set is called ...\").")
        .see_also("var.eval")
        .see_also("config.list")
        .example(
            "Each variable's formula, value and comment",
            r#"
            pc.var.new{name = "Printer"}
            pc.var.set{set = "Printer", name = "nozzle", formula = "0.4 mm", comment = "brass"}
            pc.var.set{set = "Printer", name = "angle", formula = "45"}
            local vars = pc.var.list{set = "Printer"}[1].variables
            assert(vars[1].name == "nozzle" and vars[1].formula == "0.4 mm" and vars[1].comment == "brass")
            assert(vars[1].value == 0.4 and vars[1].kind == "length" and vars[1].text == "0.4 mm")
            assert(vars[2].kind == "number" and vars[2].value == 45, "a bare number has no unit")
            "#,
        ),
        CommandSpec::new(
            "config.list",
            "The configurations: the variables they set, each row, and which is in effect",
        )
        .returns("{columns, rows: [{name, values, left_out}], active}")
        .read_only()
        .note(
            "Each row's `values` line up with `columns`; an empty string leaves that \
             variable its own formula. `left_out` lists the ids of the bodies the row \
             leaves out, and `active` is nil while no configuration is in effect.",
        )
        .see_also("config.activate")
        .see_also("var.list")
        .example(
            "Two sizes, the large one in effect",
            r#"
            pc.var.new{name = "Size"}
            pc.var.set{set = "Size", name = "width", formula = "40 mm"}
            pc.config.add_variable{variable = "Size.width"}
            pc.config.new{name = "Small"}
            pc.config.set{name = "Small", variable = "Size.width", value = "30 mm"}
            pc.config.new{name = "Large", like = "Small"}
            pc.config.set{name = "Large", variable = "Size.width", value = "60 mm"}
            pc.config.activate{name = "Large"}
            local t = pc.config.list()
            assert(t.columns[1] == "Size.width" and t.active == "Large")
            assert(t.rows[1].name == "Small" and t.rows[1].values[1] == "30 mm")
            assert(t.rows[2].name == "Large" and t.rows[2].values[1] == "60 mm")
            assert(pc.var.eval{formula = "Size.width"}.value == 60)
            "#,
        ),
        CommandSpec::new("config.new", "Add a configuration")
            .param("name", ParamKind::String, "Such as \"Large\"")
            .optional(
                "like",
                ParamKind::String,
                "Start from this configuration's values",
            )
            .note(
                "The first one makes the document's configurations table. A new \
                 configuration is not put in effect (`pc.config.activate` does that), and \
                 without `like` it leaves every variable its own.",
            )
            .note("A name already used, or a `like` that does not exist, is refused.")
            .see_also("config.set")
            .see_also("config.activate")
            .example(
                "A configuration copied from another",
                r#"
                pc.var.new{name = "Size"}
                pc.var.set{set = "Size", name = "width", formula = "40 mm"}
                pc.config.add_variable{variable = "Size.width"}
                pc.config.new{name = "Large"}
                pc.config.set{name = "Large", variable = "Size.width", value = "60 mm"}
                pc.config.new{name = "Large, thin", like = "Large"}
                local rows = pc.config.list().rows
                assert(#rows == 2 and rows[2].values[1] == "60 mm", "copied from Large")
                assert(pc.config.list().active == nil, "a new configuration is not put in effect")
                "#,
            ),
        CommandSpec::new("config.remove", "Remove a configuration")
            .param("name", ParamKind::String, "")
            .note(
                "Removing the one in effect leaves none in effect: every variable goes \
                 back to its own formula. A name that does not exist is refused.",
            )
            .see_also("config.activate")
            .example(
                "The configuration in effect removed",
                r#"
                pc.var.new{name = "Size"}
                pc.var.set{set = "Size", name = "width", formula = "40 mm"}
                pc.config.add_variable{variable = "Size.width"}
                pc.config.new{name = "Large"}
                pc.config.set{name = "Large", variable = "Size.width", value = "60 mm"}
                pc.config.activate{name = "Large"}
                pc.config.remove{name = "Large"}
                assert(#pc.config.list().rows == 0 and pc.config.list().active == nil)
                assert(pc.var.eval{formula = "Size.width"}.value == 40, "the variable's own formula again")
                "#,
            ),
        CommandSpec::new("config.rename", "Rename a configuration")
            .param("name", ParamKind::String, "")
            .param("to", ParamKind::String, "")
            .note(
                "The configuration in effect stays in effect under its new name. A name \
                 that does not exist, or a new name already used, is refused.",
            )
            .example(
                "The one in effect renamed",
                r#"
                pc.config.new{name = "Big"}
                pc.config.activate{name = "Big"}
                pc.config.rename{name = "Big", to = "Large"}
                local t = pc.config.list()
                assert(t.rows[1].name == "Large" and t.active == "Large")
                "#,
            ),
        CommandSpec::new(
            "config.add_variable",
            "Let the configurations set a variable: it becomes a column",
        )
        .param(
            "variable",
            ParamKind::String,
            "As formulas read it: Size.width",
        )
        .note(
            "The variable must exist, written as `Set.name` (\"write it as Set.name\"); a \
             column already there is refused. Every configuration starts with an empty \
             value in it, leaving the variable its own formula.",
        )
        .see_also("config.set")
        .see_also("var.set")
        .example(
            "A column, empty in every row",
            r#"
            pc.var.new{name = "Size"}
            pc.var.set{set = "Size", name = "width", formula = "40 mm"}
            pc.config.new{name = "Large"}
            pc.config.add_variable{variable = "Size.width"}
            local t = pc.config.list()
            assert(t.columns[1] == "Size.width" and t.rows[1].values[1] == "", "empty: the variable's own")
            local ok, why = pcall(pc.config.add_variable, {variable = "width"})
            assert(not ok and tostring(why):find("Set.name"), tostring(why))
            "#,
        ),
        CommandSpec::new("config.remove_variable", "Take a variable's column away")
            .param(
                "variable",
                ParamKind::String,
                "As formulas read it: Size.width",
            )
            .note(
                "The variable stays in its set and goes back to its own formula. A \
                 variable that is not a column is refused.",
            )
            .see_also("config.add_variable")
            .see_also("var.remove")
            .example(
                "The column gone, the variable its own again",
                r#"
                pc.var.new{name = "Size"}
                pc.var.set{set = "Size", name = "width", formula = "40 mm"}
                pc.config.add_variable{variable = "Size.width"}
                pc.config.new{name = "Large"}
                pc.config.set{name = "Large", variable = "Size.width", value = "60 mm"}
                pc.config.activate{name = "Large"}
                assert(pc.var.eval{formula = "Size.width"}.value == 60)
                pc.config.remove_variable{variable = "Size.width"}
                assert(#pc.config.list().columns == 0)
                assert(pc.var.eval{formula = "Size.width"}.value == 40, "its own formula again")
                "#,
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
        .param("value", ParamKind::String, "Such as \"60 mm\"")
        .note(
            "The variable must be a column (`pc.config.add_variable`). The value is \
             checked only for its syntax: one that reads the same variable is a loop, \
             which shows as the variable's `error` once the configuration is in effect.",
        )
        .note(
            "It changes the variable's value only while the configuration is in effect; \
             `pc.var.set` changes the variable's own formula.",
        )
        .see_also("config.add_variable")
        .see_also("config.activate")
        .see_also("var.set")
        .example(
            "A value given, then left to the variable",
            r#"
            pc.var.new{name = "Size"}
            pc.var.set{set = "Size", name = "width", formula = "40 mm"}
            pc.config.add_variable{variable = "Size.width"}
            pc.config.new{name = "Large"}
            pc.config.set{name = "Large", variable = "Size.width", value = "1.5 * 40 mm"}
            pc.config.activate{name = "Large"}
            assert(pc.var.eval{formula = "Size.width"}.value == 60)
            pc.config.set{name = "Large", variable = "Size.width", value = ""}
            assert(pc.var.eval{formula = "Size.width"}.value == 40, "empty: the variable's own")
            "#,
        ),
        CommandSpec::new(
            "config.leave_out",
            "The bodies a configuration leaves out: not drawn, picked, exported or checked",
        )
        .param("name", ParamKind::String, "The configuration")
        .param(
            "bodies",
            ParamKind::List,
            "The bodies' ids; an empty list leaves none out",
        )
        .note(
            "The list replaces the one the configuration had. An id that is not a body of \
             the document is refused, and so is a configuration that does not exist.",
        )
        .note(
            "The bodies are left out only while the configuration is in effect; they stay \
             in the document.",
        )
        .see_also("config.list")
        .see_also("config.activate")
        .example(
            "A lid left out of one configuration",
            r#"
            local function box(z)
              local s = pc.sketch.new{plane = "XY", offset = z}
              pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
            end
            local base, lid = box(0), box(5)
            assert(#pc.doc.rebuild() == 0)
            pc.config.new{name = "Open"}
            pc.config.leave_out{name = "Open", bodies = {lid}}
            assert(pc.config.list().rows[1].left_out[1] == lid)
            pc.config.leave_out{name = "Open", bodies = {}}
            assert(#pc.config.list().rows[1].left_out == 0, "an empty list leaves none out")
            local ok, why = pcall(pc.config.leave_out, {name = "Closed", bodies = {lid}})
            assert(not ok, "no configuration is called Closed")
            "#,
        ),
        CommandSpec::new("config.activate", "Put a configuration in effect")
            .optional("name", ParamKind::String, "Nil leaves every variable its own")
            .note(
                "Every variable the configuration gives a value takes it, and what reads \
                 them follows once `pc.doc.rebuild()` builds it. A name that does not \
                 exist is refused.",
            )
            .see_also("config.list")
            .see_also("config.set")
            .example(
                "A pad's length switched by configuration",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
                local pad = pc.design.pad{sketch = s, length = 5}
                local body = pc.doc.feature{id = pad}.body
                pc.var.new{name = "Size"}
                pc.var.set{set = "Size", name = "height", formula = "5 mm"}
                pc.doc.set_formula{id = pad, parameter = "length", formula = "Size.height"}
                pc.config.add_variable{variable = "Size.height"}
                pc.config.new{name = "Tall"}
                pc.config.set{name = "Tall", variable = "Size.height", value = "12 mm"}
                pc.config.activate{name = "Tall"}
                assert(#pc.doc.rebuild() == 0)
                assert(math.abs(pc.doc.measure{body = body}.volume - 2400) < 1e-6)
                pc.config.activate{}
                assert(#pc.doc.rebuild() == 0)
                assert(math.abs(pc.doc.measure{body = body}.volume - 1000) < 1e-6, "its own 5 mm again")
                "#,
            ),
        CommandSpec::new("var.eval", "What a formula comes to in this document")
            .param("formula", ParamKind::String, "")
            .returns("{value, kind, text}: value in mm or degrees")
            .read_only()
            .note(
                "It changes nothing. A formula that does not work out is refused with the \
                 reason (\"nothing is called Nope\"), where `pc.var.set` would keep it.",
            )
            .note(
                "`kind` is such as length, angle, number or area; a bare number is a plain \
                 number, and trigonometry takes degrees. It reads the configuration in \
                 effect.",
            )
            .see_also("var.set")
            .see_also("var.list")
            .example(
                "Units worked out",
                r#"
                pc.var.new{name = "Printer"}
                pc.var.set{set = "Printer", name = "nozzle", formula = "0.4 mm"}
                local wall = pc.var.eval{formula = "3 * Printer.nozzle + 1 in"}
                assert(math.abs(wall.value - 26.6) < 1e-9 and wall.kind == "length")
                local area = pc.var.eval{formula = "2 mm * 3 mm"}
                assert(area.kind == "area" and area.value == 6)
                assert(math.abs(pc.var.eval{formula = "sin(30)"}.value - 0.5) < 1e-12, "degrees")
                "#,
            ),
    ]
}

/// What a quantity is called in a command's answer.
fn kind_name(dim: core_document::expr::Dim) -> String {
    use core_document::expr::Dim;
    match dim {
        Dim::LENGTH => "length".into(),
        Dim::ANGLE => "angle".into(),
        Dim::NUMBER => "number".into(),
        Dim::AREA => "area".into(),
        Dim::VOLUME => "volume".into(),
        other => other.unit_text("mm"),
    }
}

/// A slot's value as a command answers it.
fn slot_json(
    result: &Result<core_document::expr::Quantity, String>,
    unit: core_document::Unit,
) -> Value {
    match result {
        Ok(q) => json!({
            "value": q.value,
            "kind": kind_name(q.dim),
            "text": q.display(unit, 4),
            "error": Value::Null,
        }),
        Err(why) => json!({"value": Value::Null, "text": Value::Null, "error": why}),
    }
}

/// A variable set named or given by id.
fn set_arg(document: &core_document::Document, a: &Args) -> Result<FeatureId, CommandError> {
    let given = a.string("set")?;
    let by_id = Uuid::parse_str(given).ok().map(FeatureId).filter(|id| {
        document
            .get_feature_meta(*id)
            .is_some_and(|n| n.workbench_id.as_str() == core_document::VARIABLES_KIND)
    });
    by_id
        .or_else(|| {
            document.object_named(given).filter(|id| {
                document
                    .get_feature_meta(*id)
                    .is_some_and(|n| n.workbench_id.as_str() == core_document::VARIABLES_KIND)
            })
        })
        .ok_or_else(|| CommandError::failed(format!("no variable set is called {given}")))
}

/// What slot `key` of feature `id` comes to now.
fn slot_answer(
    document: &mut core_document::Document,
    registry: &core_document::DocumentService,
    id: FeatureId,
    key: &str,
) -> Value {
    registry.evaluate(document);
    let unit = document.display_unit();
    document
        .evaluated_slots(id)
        .iter()
        .find(|s| s.key == key)
        .map(|s| slot_json(&s.result, unit))
        .unwrap_or(Value::Null)
}

/// The commands a key can run that make sense without a key: all but
/// those that open a window of the interface's own.
fn key_commands() -> impl Iterator<Item = (CommandSpec, keymap::HostAction)> {
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
        .returns("nothing; pc.doc.rebuild() waits for the import"),
        Export => path(spec, "Where to write; the dialog when left out")
            .optional(
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
            .returns("{path, written, skipped, triangles}"),
        SendToSlicer => spec.optional(
            "layout",
            ParamKind::Bool,
            "The print layout rather than the bodies where they sit; Preferences › Printing \
             says when left out",
        ),
        _ => spec,
    }
}

/// The application's commands that are not a key's.
fn app_commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("app.workbenches", "The workbenches, in the order they load")
            .returns("a list of {id, label, active}")
            .read_only(),
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
    ]
}

/// The commands a script can call, by the application and then the
/// workbenches in registration order.
fn all_commands(app: &PrintCadApp) -> Vec<CommandSpec> {
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

impl PrintCadApp {
    /// Run command `id` for a script, its arguments checked against its
    /// spec: the application's own here, a workbench's in the workbench.
    fn execute_command(
        &mut self,
        id: &str,
        args: CommandArgs,
        event_loop: &ActiveEventLoop,
    ) -> CommandResult {
        if let Some(spec) = doc_commands().into_iter().find(|c| c.id == id) {
            spec.check(&args)?;
            return self.run_doc_command(id, &args, event_loop);
        }
        if let Some(spec) = app_commands().into_iter().find(|c| c.id == id) {
            spec.check(&args)?;
            return self.run_app_command(id, &args);
        }
        if let Some((spec, action)) = key_commands().find(|(c, _)| c.id == id) {
            spec.check(&args)?;
            if Args(&args).has("path") {
                return self.run_file_command(action, &args);
            }
            if let (keymap::HostAction::SendToSlicer, Some(layout)) =
                (action, Args(&args).opt_bool("layout")?)
            {
                let was = self.user_settings.printing.slicer_layout;
                self.user_settings.printing.slicer_layout = layout;
                self.send_to_slicer(false);
                self.user_settings.printing.slicer_layout = was;
                return Ok(Value::Null);
            }
            return self.run_key_command(action, event_loop);
        }
        let Some((bench, spec)) = self.registry.command(id) else {
            return Err(CommandError::Unknown(id.to_string()));
        };
        spec.check(&args)?;
        // A bench reads features as their formulas leave them: worked out
        // for the calls before this one, as a headless run has them.
        self.registry.evaluate(&mut self.session.document);
        let params = self.interaction_ctx_params();
        let (result, outcome) = self
            .with_workbench_ctx(&bench, params, |wb, ctx| wb.run_command(id, &args, ctx))
            .ok_or_else(|| CommandError::Unknown(id.to_string()))?;
        self.apply_hook_outcome(outcome, HookSite::Interaction);
        result
    }

    /// Run one line typed in the console on the script thread.
    pub(crate) fn run_console_line(&mut self, line: &str) {
        console::push(LineKind::Input, line);
        self.submit_script(scripting::Job::Line(line.to_string()), RunKind::Console);
    }

    pub(crate) fn submit_script(&mut self, job: scripting::Job, kind: RunKind) {
        let tab = self.session.tab;
        self.submit_script_in(job, kind, tab);
    }

    /// Run `job` on the document of `tab`, whichever tab is on screen.
    pub(crate) fn submit_script_in(&mut self, job: scripting::Job, kind: RunKind, tab: uuid::Uuid) {
        let commands = command_specs(&self.registry);
        self.script_runs.push_back(ScriptRun {
            tab,
            kind,
            label: match &job {
                scripting::Job::Line(_) => "a console line".to_string(),
                scripting::Job::Script { name, .. } => name.clone(),
                scripting::Job::Command { id, .. } => id.clone(),
            },
        });
        self.script_thread.submit(job, commands);
        self.redraw_needed = true;
    }

    fn run_app_command(&mut self, id: &str, args: &CommandArgs) -> CommandResult {
        let a = Args(args);
        match id {
            "app.workbenches" => Ok(Value::Array(
                self.registry
                    .ids()
                    .iter()
                    .map(|bench| {
                        json!({
                            "id": bench.as_str(),
                            "label": self.registry.descriptor(bench).map(|d| d.label.clone()),
                            "active": *bench == self.session.active_workbench.0,
                        })
                    })
                    .collect(),
            )),
            "app.workbench" => {
                let bench = self.bench_arg(a.string("id")?)?;
                self.switch_workbench_for_flow(bench);
                Ok(Value::Null)
            }
            "app.tools" => {
                let bench = match a.opt_string("workbench")? {
                    Some(id) => self.bench_arg(id)?,
                    None => self.session.active_workbench.0.clone(),
                };
                let tools = self.registry.tools_for(&bench).unwrap_or_default();
                Ok(Value::Array(
                    tools
                        .iter()
                        .filter(|t| t.planned.is_none())
                        .map(|t| {
                            json!({
                                "id": t.id,
                                "label": t.label,
                                "keys": t.shortcuts.iter().map(ToString::to_string).collect::<Vec<_>>(),
                            })
                        })
                        .collect(),
                ))
            }
            "app.tool" => {
                let id = a.string("id")?;
                let bench = self
                    .registry
                    .ids()
                    .iter()
                    .find(|bench| {
                        self.registry.tools_for(bench).is_ok_and(|tools| {
                            tools.iter().any(|t| t.id == id && t.planned.is_none())
                        })
                    })
                    .cloned()
                    .ok_or_else(|| CommandError::bad("id", "is not a tool"))?;
                if bench != self.session.active_workbench.0 {
                    self.switch_workbench_for_flow(bench.clone());
                }
                let tools = self.registry.tools_for(&bench).unwrap_or_default().to_vec();
                if let Some(tool) = tools.iter().find(|t| t.id == id) {
                    crate::ui::toolbar::activate_tool(
                        &mut self.session.active_tool,
                        &tools,
                        tool,
                        id,
                    );
                }
                Ok(Value::Null)
            }
            _ => Err(CommandError::Unknown(id.to_string())),
        }
    }

    fn bench_arg(&self, id: &str) -> Result<core_document::WorkbenchId, CommandError> {
        self.registry
            .ids()
            .iter()
            .find(|b| b.as_str() == id)
            .cloned()
            .ok_or_else(|| {
                CommandError::bad("id", "is not a workbench; app.workbenches lists them")
            })
    }

    /// A file command given its file: no dialog.
    fn run_file_command(
        &mut self,
        action: keymap::HostAction,
        args: &CommandArgs,
    ) -> CommandResult {
        use keymap::HostAction::*;
        let a = Args(args);
        let path = std::path::PathBuf::from(a.string("path")?);
        match action {
            Open => {
                self.open_document_at(path);
                Ok(Value::Null)
            }
            SaveAs => {
                self.save_document_at(&path)
                    .map_err(|e| CommandError::failed(e.to_string()))?;
                Ok(Value::Null)
            }
            Import => {
                if !path.is_file() {
                    return Err(CommandError::bad("path", "is not a file"));
                }
                let detail = self.last_step_import_detail.clone();
                self.import_step_at(&path, detail);
                Ok(Value::Null)
            }
            Export => {
                let format = export_format(a.opt_string("format")?, &path)?;
                let bodies = body_list(args.get("bodies"))?;
                let tolerance = a.opt_number("tolerance")?.map(|t| t as f32);
                let layout =
                    (a.opt_bool("layout")? == Some(true)).then(|| self.current_print_layout());
                let (path, exported) = crate::app::export::export_document(
                    &self.session.document,
                    path,
                    format,
                    bodies,
                    tolerance,
                    layout.as_ref(),
                )
                .map_err(CommandError::failed)?;
                Ok(json!({
                    "path": path.display().to_string(),
                    "written": exported.written,
                    "skipped": exported.skipped,
                    "triangles": exported.triangles,
                }))
            }
            _ => Err(CommandError::failed("this command takes no file")),
        }
    }

    /// Run a script file on the script thread. What it prints and the error
    /// that stops it open the console.
    pub(crate) fn run_script_file(&mut self, path: &std::path::Path) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let read = crate::platform::read(path).and_then(|bytes| {
            String::from_utf8(bytes)
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
        });
        match read {
            Ok(source) => {
                console::push(LineKind::Input, format!("run {name}"));
                self.submit_script(scripting::Job::Script { source, name }, RunKind::File);
            }
            Err(err) => {
                console::push(LineKind::Error, format!("{name}: {err}"));
                self.console_attention = true;
            }
        }
    }

    /// Stop the running script: at its next instruction, or at once when
    /// it waits on a rebuild.
    pub(crate) fn stop_script(&mut self) {
        self.script_thread.stop();
        if let Some(wait) = self.script_rebuild.take() {
            let _ = wait
                .reply
                .send(Err(CommandError::failed(scripting::STOPPED)));
        }
    }

    /// Answer the script thread: run the commands it asks for, show what it
    /// prints, and close a run's undo step when it ends. Commands keep
    /// coming within a few milliseconds a frame, so a script runs at speed
    /// while the window keeps drawing.
    pub(crate) fn drive_scripts(&mut self, event_loop: &ActiveEventLoop) {
        const BUDGET: std::time::Duration = std::time::Duration::from_millis(8);
        let started = web_time::Instant::now();
        loop {
            if self.script_rebuild.is_some() {
                self.answer_rebuild();
                if self.script_rebuild.is_some() {
                    return;
                }
            }
            let wait = if self.script_thread.busy() && started.elapsed() < BUDGET {
                std::time::Duration::from_millis(1)
            } else {
                std::time::Duration::ZERO
            };
            let Some(event) = self.script_thread.next_event(wait) else {
                return;
            };
            self.script_event(event, event_loop);
            if started.elapsed() > BUDGET {
                self.redraw_needed = true;
                return;
            }
        }
    }

    fn script_event(&mut self, event: scripting::Event, event_loop: &ActiveEventLoop) {
        use scripting::Event;
        let kind = self.script_runs.front().map(|r| r.kind.clone());
        let from_file = matches!(kind, Some(RunKind::File));
        let agent = matches!(kind, Some(RunKind::Agent { .. }));
        match event {
            Event::Started { label } => {
                let step = match kind {
                    Some(RunKind::File) => format!("Run {label}"),
                    Some(RunKind::Agent { .. }) => format!("Agent: {label}"),
                    _ => "Console".to_string(),
                };
                self.in_script_tab(|app| {
                    app.close_gesture();
                    app.session.journal.label_next(step);
                    app.session.journal.hold(true);
                });
            }
            Event::Printed(line) => {
                // An agent's script answers the agent: its output goes back
                // with it when it ends.
                if agent {
                    return;
                }
                console::push(LineKind::Printed, line);
                if from_file {
                    self.console_attention = true;
                }
            }
            Event::Call { id, args, reply } => {
                // A script or recording may use a command's former name.
                let id = core_document::renamed::command(&id).into_owned();
                if let Some(RunKind::Agent { single, .. }) = &kind
                    && let Err(refused) = self.agent_may_run(&id, *single)
                {
                    let _ = reply.send(Err(CommandError::failed(refused)));
                    return;
                }
                if id == "doc.rebuild" {
                    self.start_rebuild(args, reply);
                    return;
                }
                let answer = self
                    .in_script_tab(|app| app.execute_command(&id, args, event_loop))
                    .unwrap_or_else(|| {
                        Err(CommandError::failed("the script's document was closed"))
                    });
                let _ = reply.send(answer);
            }
            Event::Finished { label, mut output } => {
                let failed = output.error.is_some();
                let rolled = self.in_script_tab(|app| {
                    app.session.journal.hold(false);
                    // A run that stops with an error leaves nothing behind.
                    let rolled = failed.then(|| {
                        let rolled = app.session.journal.roll_back(&mut app.session.document);
                        if rolled.0 > 0 {
                            app.after_history_jump();
                        }
                        rolled
                    });
                    app.close_gesture();
                    rolled
                });
                if let (Some(error), Some(Some((undone, barrier)))) = (&mut output.error, rolled)
                    && undone > 0
                {
                    error.push_str(&rolled_back_note(barrier));
                }
                self.script_runs.pop_front();
                if let Some(RunKind::Agent { reply, single }) = kind {
                    let answer = if single {
                        agent_answer(output)
                    } else {
                        script_answer(output)
                    };
                    let _ = reply.send(answer);
                    self.redraw_needed = true;
                    return;
                }
                match (output.value, &output.returned) {
                    (Some(value), _) => console::push(LineKind::Value, value),
                    (None, Some(returned)) => console::push(LineKind::Value, short_json(returned)),
                    (None, None) => {}
                }
                match output.error {
                    Some(error) => {
                        console::push(LineKind::Error, error.clone());
                        if from_file {
                            self.console_attention = true;
                            crate::app_log::warn(format!("Script {label} stopped: {error}"));
                        }
                    }
                    None if from_file => crate::app_log::info(format!("Ran {label}")),
                    None => {}
                }
                self.redraw_needed = true;
            }
        }
    }

    /// Run `f` with the tab the running script started in on screen, for
    /// as long as `f` takes. `None` when that tab has closed.
    fn in_script_tab<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        let tab = self.script_runs.front().map(|r| r.tab)?;
        if tab == self.session.tab {
            return Some(f(self));
        }
        let index = self.tab_index_of(tab)?;
        Some(self.with_tab(index, f))
    }

    /// `doc.rebuild`: send every changed body to the kernel now, and answer
    /// once the kernel has nothing left to do.
    fn start_rebuild(&mut self, args: CommandArgs, reply: std::sync::mpsc::Sender<CommandResult>) {
        let spec = doc_commands().into_iter().find(|c| c.id == "doc.rebuild");
        if let Some(Err(err)) = spec.map(|s| s.check(&args)) {
            let _ = reply.send(Err(err));
            return;
        }
        let timeout = Args(&args)
            .opt_number("timeout")
            .ok()
            .flatten()
            .unwrap_or(60.0);
        self.in_script_tab(|app| {
            app.drive_part_recompute();
            app.drive_shape_repairs();
            app.drive_shape_refinements();
            app.drive_shape_replacements();
            app.drive_mesh_solids();
            app.drive_mirrored_copies();
            app.drive_links();
        });
        self.script_rebuild = Some(RebuildWait {
            reply,
            deadline: web_time::Instant::now()
                + std::time::Duration::from_secs_f64(timeout.max(0.0)),
        });
    }

    /// Answer the waiting `doc.rebuild` once the kernel is idle: the
    /// features that failed.
    fn answer_rebuild(&mut self) {
        let Some(wait) = &self.script_rebuild else {
            return;
        };
        let answer = if self.kernel_worker.in_flight() == 0 {
            let errors = self
                .in_script_tab(|app| rebuild_failures(&app.session.document))
                .unwrap_or_default();
            Ok(Value::Array(errors))
        } else if web_time::Instant::now() > wait.deadline {
            Err(CommandError::failed(
                "the kernel was still working at the timeout",
            ))
        } else {
            return;
        };
        if let Some(wait) = self.script_rebuild.take() {
            let _ = wait.reply.send(answer);
        }
    }

    /// Keep `calls` in the recording, when one is on and no script is
    /// running (a script's own calls are already a script).
    pub(crate) fn record_calls(&mut self, calls: Vec<core_document::Recorded>) {
        if !self.script_runs.is_empty() {
            return;
        }
        if let Some(recorder) = self.recording.as_mut() {
            for call in &calls {
                recorder.push(call);
            }
        }
    }

    /// Start a recording, or stop the one on and save it as a new script
    /// in the scripts folder, where the Scripts menu lists it.
    pub(crate) fn toggle_recording(&mut self) {
        let Some(recorder) = self.recording.take() else {
            self.recording = Some(scripting::Recorder::default());
            crate::app_log::info("Recording: what you do now is written as a script when you stop");
            return;
        };
        if recorder.is_empty() {
            crate::app_log::info("Recording stopped; nothing was recorded");
            return;
        }
        let Some(dir) = settings::scripts_dir() else {
            crate::app_log::warn("The system names no configuration folder for scripts");
            return;
        };
        let path = crate::script_library::fresh_name(&dir, "recording");
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().replace('_', " "))
            .unwrap_or_default();
        let text = recorder.script(&format!("A recording ({name})"));
        match std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, text)) {
            Ok(()) => {
                self.script_library_read = None;
                console::push(
                    LineKind::Printed,
                    format!("Recording saved as {}", path.display()),
                );
                crate::app_log::info(format!("Recording saved as {}", path.display()));
            }
            Err(err) => {
                crate::app_log::error(format!("Could not write {}: {err}", path.display()));
            }
        }
    }

    /// Every command's id, for the console's completion.
    pub(crate) fn script_command_ids(&self) -> Vec<String> {
        all_commands(self).into_iter().map(|c| c.id).collect()
    }

    /// Read the scripts folder again, every couple of seconds while frames
    /// run, so a script saved in an editor shows up without a restart.
    pub(crate) fn refresh_script_library(&mut self) {
        let due = self
            .script_library_read
            .is_none_or(|at| at.elapsed() > std::time::Duration::from_secs(2));
        if !due {
            return;
        }
        self.script_library_read = Some(web_time::Instant::now());
        self.script_library = settings::scripts_dir()
            .map(|dir| crate::script_library::scan(&dir))
            .unwrap_or_default();
    }

    /// Make a new script in the scripts folder, from `runs` (what the
    /// console ran) or else the template, and open it in the system's
    /// editor.
    pub(crate) fn new_script(&mut self, runs: Option<Vec<String>>) {
        // A page has no scripts folder: the script is a download.
        if crate::platform::ON_PAGE {
            let text = match runs {
                Some(runs) => crate::script_library::from_runs(&runs),
                None => crate::script_library::TEMPLATE.to_string(),
            };
            if let Err(err) =
                crate::platform::write(std::path::Path::new("script.lua"), text.as_bytes())
            {
                crate::app_log::error(format!("Could not save the script: {err}"));
            }
            return;
        }
        let Some(dir) = settings::scripts_dir() else {
            crate::app_log::warn("The system names no configuration folder for scripts");
            return;
        };
        let path = crate::script_library::fresh_path(&dir);
        let text = match runs {
            Some(runs) => crate::script_library::from_runs(&runs),
            None => crate::script_library::TEMPLATE.to_string(),
        };
        let written = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, text));
        match written {
            Ok(()) => {
                crate::app_log::info(format!("New script {}", path.display()));
                self.script_library_read = None;
                self.edit_script(Some(path));
            }
            Err(err) => crate::app_log::error(format!("Could not write {}: {err}", path.display())),
        }
    }

    /// Open `path` in the system's editor, or the scripts folder in its
    /// file manager.
    pub(crate) fn edit_script(&mut self, path: Option<std::path::PathBuf>) {
        let target = match path {
            Some(path) => path,
            None => {
                let Some(dir) = settings::scripts_dir() else {
                    return;
                };
                if let Err(err) = std::fs::create_dir_all(&dir) {
                    crate::app_log::error(format!("Could not make {}: {err}", dir.display()));
                    return;
                }
                dir
            }
        };
        if let Err(err) = local_ipc::open_with_system(&target) {
            crate::app_log::error(format!("Could not open {}: {err}", target.display()));
        }
    }

    fn run_key_command(
        &mut self,
        action: keymap::HostAction,
        event_loop: &ActiveEventLoop,
    ) -> CommandResult {
        let state = HostState {
            active_tab: Some(self.session.tab),
            section_on: self.session.section.is_some(),
            document: &self.session.document,
            tree_selection: self.session.tree_selection,
            editing: false,
        };
        match keymap::host_outcome(action, &state) {
            HostOutcome::Command(command) => {
                self.apply_ui_commands(vec![command], event_loop);
                Ok(Value::Null)
            }
            _ => Err(CommandError::failed("this command is not available here")),
        }
    }

    fn run_doc_command(
        &mut self,
        id: &str,
        args: &CommandArgs,
        event_loop: &ActiveEventLoop,
    ) -> CommandResult {
        if let Some(answer) = document_command(
            id,
            args,
            &mut self.session.document,
            &self.registry,
            self.session.current_file.as_deref(),
        ) {
            return answer;
        }
        let a = Args(args);
        match id {
            "doc.picture" => picture_command(args, |request| self.picture(request)),
            "doc.print_layout" => print_layout_command(
                args,
                &self.session.document,
                &self.registry,
                &self.user_settings.printing,
            ),
            "doc.selection" => Ok(json!({
                "item": self.session.tree_selection.and_then(item_id).map(|u| u.to_string()),
                "body": self.session.active_body_id.map(|b| b.0.to_string()),
                "feature": self.session.active_document_object.map(|f| f.0.to_string()),
            })),
            "doc.select" => {
                let item = tree_item(&self.session.document, a.id("id")?)?;
                self.apply_tree_selection(item);
                Ok(Value::Null)
            }
            "doc.set_visible" => {
                let visible = a.opt_bool("visible")?.unwrap_or(true);
                let command = match tree_item(&self.session.document, a.id("id")?)? {
                    TreeItemId::Body(body) => {
                        crate::ui::UiCommand::SetBodyVisible { body, visible }
                    }
                    TreeItemId::Feature(feature) => crate::ui::UiCommand::TreeFeature {
                        feature,
                        command: crate::ui::TreeFeatureCommand::SetVisible(visible),
                    },
                    TreeItemId::ImportedObject(node) => {
                        crate::ui::UiCommand::SetImportedVisibility { node, visible }
                    }
                    TreeItemId::Component(component) => {
                        let commands = self
                            .session
                            .document
                            .component_bodies(component)
                            .into_iter()
                            .map(|body| crate::ui::UiCommand::SetBodyVisible { body, visible })
                            .collect();
                        self.apply_ui_commands(commands, event_loop);
                        return Ok(Value::Null);
                    }
                    TreeItemId::DocumentRoot => {
                        return Err(CommandError::bad("id", "cannot be hidden"));
                    }
                };
                self.apply_ui_commands(vec![command], event_loop);
                Ok(Value::Null)
            }
            "doc.repair" | "doc.convert_to_solid" | "doc.refine" => {
                let bodies = body_list(args.get("bodies"))?.unwrap_or_default();
                let command = match id {
                    "doc.repair" => crate::ui::UiCommand::RepairShapes(bodies),
                    "doc.refine" => crate::ui::UiCommand::RefineShapes(bodies),
                    _ => crate::ui::UiCommand::ConvertToSolid(bodies),
                };
                self.apply_ui_commands(vec![command], event_loop);
                Ok(Value::Null)
            }
            "doc.delete" => {
                let item = tree_item(&self.session.document, a.id("id")?)?;
                self.apply_ui_commands(
                    vec![crate::ui::UiCommand::DeleteTreeItem(item)],
                    event_loop,
                );
                Ok(Value::Null)
            }
            _ => Err(CommandError::Unknown(id.to_string())),
        }
    }
}

/// The command a body menu's edit runs, and its arguments.
pub(crate) fn body_edit_call(body: BodyId, edit: &crate::ui::BodyEdit) -> (&'static str, Value) {
    use crate::ui::BodyEdit;
    let id = body.0.to_string();
    match edit {
        BodyEdit::Frozen(frozen) => ("doc.set_body", json!({"body": id, "frozen": frozen})),
        BodyEdit::Selectable(selectable) => (
            "doc.set_body",
            json!({"body": id, "selectable": selectable}),
        ),
        BodyEdit::LinkedCopy => ("doc.linked_copy", json!({"body": id})),
        BodyEdit::Recompute => ("doc.recompute", json!({"body": id})),
    }
}

/// What a host UI command does, as the command a recording says it with:
/// renaming, showing or hiding and deleting tree rows. The benches record
/// their own.
pub(crate) fn recorded_of(command: &crate::ui::UiCommand) -> Option<core_document::Recorded> {
    use crate::ui::{TreeFeatureCommand, UiCommand};
    let call = |id: &str, args: Value| core_document::Recorded {
        id: id.to_string(),
        args: match args {
            Value::Object(map) => map,
            _ => CommandArgs::new(),
        },
        result: Value::Null,
    };
    match command {
        UiCommand::SetDocumentAgentRules(rules) => {
            Some(call("doc.set_agent_rules", json!({"rules": rules})))
        }
        UiCommand::Config(crate::ui::ConfigEdit::NewTable) => None,
        UiCommand::Config(edit) => {
            let (id, args) = edit.command();
            Some(call(id, args))
        }
        UiCommand::SetVariable {
            set,
            name,
            formula,
            comment,
        } => {
            let mut args = json!({"set": set.0.to_string(), "name": name, "formula": formula});
            if let Some(comment) = comment {
                args["comment"] = json!(comment);
            }
            Some(call("var.set", args))
        }
        UiCommand::RemoveVariable { set, name } => Some(call(
            "var.remove",
            json!({"set": set.0.to_string(), "name": name}),
        )),
        UiCommand::RenameVariable { set, name, to } => Some(call(
            "var.rename",
            json!({"set": set.0.to_string(), "name": name, "to": to}),
        )),
        UiCommand::SetParameter {
            feature,
            parameter,
            edit,
        } => {
            let which = parameter
                .name
                .clone()
                .unwrap_or_else(|| parameter.key.clone());
            let id = feature.0.to_string();
            Some(match edit {
                ui_kit::widgets::FormulaEdit::Formula(text) => call(
                    "doc.set_formula",
                    json!({"id": id, "parameter": which, "formula": text}),
                ),
                ui_kit::widgets::FormulaEdit::Value(v) => call(
                    "doc.set_value",
                    json!({"id": id, "parameter": which, "value": v}),
                ),
            })
        }
        UiCommand::RenameTreeItem { item, name } => {
            let id = item_id(*item)?;
            Some(call(
                "doc.rename",
                json!({"id": id.to_string(), "name": name}),
            ))
        }
        UiCommand::BodyEdit { body, edit } => {
            let (id, args) = body_edit_call(*body, edit);
            Some(call(id, args))
        }
        UiCommand::SetBodyDisplay { body, display } => Some(call(
            "doc.set_body",
            match display {
                Some(d) => json!({
                    "body": body.0.to_string(),
                    "color": d.color,
                    "opacity": d.opacity,
                }),
                None => json!({"body": body.0.to_string(), "color": Value::Null}),
            },
        )),
        UiCommand::SetBodyVisible { body, visible } => Some(call(
            "doc.set_visible",
            json!({"id": body.0.to_string(), "visible": visible}),
        )),
        UiCommand::SetImportedVisibility { node, visible } => Some(call(
            "doc.set_visible",
            json!({"id": node.to_string(), "visible": visible}),
        )),
        UiCommand::TreeFeature {
            feature,
            command: TreeFeatureCommand::SetVisible(visible),
        } => Some(call(
            "doc.set_visible",
            json!({"id": feature.0.to_string(), "visible": visible}),
        )),
        UiCommand::DeleteTreeItem(item) => {
            let id = item_id(*item)?;
            Some(call("doc.delete", json!({"id": id.to_string()})))
        }
        UiCommand::RepairShapes(bodies)
        | UiCommand::ConvertToSolid(bodies)
        | UiCommand::RefineShapes(bodies) => {
            let id = match command {
                UiCommand::RepairShapes(_) => "doc.repair",
                UiCommand::RefineShapes(_) => "doc.refine",
                _ => "doc.convert_to_solid",
            };
            let bodies: Vec<String> = bodies.iter().map(|b| b.0.to_string()).collect();
            Some(call(id, json!({"bodies": bodies})))
        }
        UiCommand::TreeFeature { feature, command } => {
            let id = feature.0.to_string();
            match command {
                TreeFeatureCommand::Suppress(on) => {
                    Some(call("doc.suppress", json!({"id": id, "suppressed": on})))
                }
                TreeFeatureCommand::Delete => Some(call("doc.delete", json!({"id": id}))),
                TreeFeatureCommand::MoveUp => Some(call("doc.move", json!({"id": id, "up": true}))),
                TreeFeatureCommand::MoveDown => {
                    Some(call("doc.move", json!({"id": id, "up": false})))
                }
                TreeFeatureCommand::SetTip => Some(call("doc.set_tip", json!({"id": id}))),
                TreeFeatureCommand::ClearTip => {
                    Some(call("doc.set_tip", json!({"id": id, "clear": true})))
                }
                // Recorded where it lands, which only the document knows.
                TreeFeatureCommand::SetVisible(_) | TreeFeatureCommand::MoveNextTo { .. } => None,
            }
        }
        _ => None,
    }
}

/// A script run submitted to the script thread: the tab it runs against
/// and who asked for it.
#[derive(Debug, Clone)]
pub(crate) struct ScriptRun {
    pub tab: Uuid,
    pub kind: RunKind,
    /// The script's name, the console line, or the command.
    pub label: String,
}

/// Who asked for a script run, and where its output goes.
#[derive(Debug, Clone)]
pub(crate) enum RunKind {
    /// A console line: its output shows there.
    Console,
    /// A script file: its output shows in the console, which opens for it.
    File,
    /// An agent's tool call: its output is the answer. `single` for a
    /// `call`, one command the user's approval covered; a `lua` script's
    /// commands are checked one by one as it calls them.
    Agent {
        reply: std::sync::mpsc::Sender<agents::mcp::ToolAnswer>,
        single: bool,
    },
}

/// What a failed run's error adds once its changes are taken back;
/// `barrier` when an import in it could not be.
fn rolled_back_note(barrier: bool) -> String {
    if barrier {
        " (its changes after its last import were undone; the import stays)".into()
    } else {
        " (its changes were undone)".into()
    }
}

/// What an agent's run answers: what it printed and came to, or why it
/// stopped.
fn agent_answer(output: scripting::RunOutput) -> agents::mcp::ToolAnswer {
    let mut text = output.printed.join("\n");
    let mut add = |part: &str| {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(part);
    };
    match (&output.error, &output.value) {
        (Some(error), _) => {
            add(error);
            agents::mcp::ToolAnswer::error(text)
        }
        (None, Some(value)) => {
            add(value);
            agents::mcp::ToolAnswer::text(text)
        }
        (None, None) => {
            if text.is_empty() {
                text = "done".to_string();
            }
            agents::mcp::ToolAnswer::text(text)
        }
    }
}

/// What an agent's `lua` answers: `{returned, printed}` as JSON, the
/// script's return value and its printed lines as one text, or
/// `{error, printed}` as an error when something stopped it.
fn script_answer(output: scripting::RunOutput) -> agents::mcp::ToolAnswer {
    let printed = output.printed.join("\n");
    let text = |value: Value| serde_json::to_string_pretty(&value).unwrap_or_default();
    match output.error {
        Some(error) => {
            agents::mcp::ToolAnswer::error(text(json!({"error": error, "printed": printed})))
        }
        None => agents::mcp::ToolAnswer::text(text(json!({
            "returned": output.returned.unwrap_or(Value::Null),
            "printed": printed,
        }))),
    }
}

/// A script's return value as one console line: compact JSON, cut short
/// past a few hundred characters.
fn short_json(value: &Value) -> String {
    const MOST: usize = 300;
    let text = value.to_string();
    match text.char_indices().nth(MOST) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text,
    }
}

/// A `doc.rebuild` waiting on the kernel.
pub(crate) struct RebuildWait {
    reply: std::sync::mpsc::Sender<CommandResult>,
    deadline: web_time::Instant,
}

/// `doc.picture`: the request in `args` drawn by `draw` and written to
/// its `path`.
pub(crate) fn picture_command(
    args: &CommandArgs,
    draw: impl FnOnce(&crate::proof::Request) -> Result<Vec<u8>, String>,
) -> CommandResult {
    let path = std::path::PathBuf::from(Args(args).string("path")?);
    let request = crate::proof::parse(args).map_err(CommandError::failed)?;
    let png = draw(&request).map_err(CommandError::failed)?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| CommandError::failed(e.to_string()))?;
    }
    std::fs::write(&path, png).map_err(|e| CommandError::failed(e.to_string()))?;
    Ok(json!({
        "path": path.display().to_string(),
        "width": request.size.0,
        "height": request.size.1,
    }))
}

/// The commands every host of a document answers the same way, with or
/// without a window: reading it, naming and adding bodies, and measuring.
/// `None` for any other command.
/// `doc.print_layout`: the layout of `document`'s parts on the bed the
/// arguments give, else on `printing`'s.
pub(crate) fn print_layout_command(
    args: &CommandArgs,
    document: &core_document::Document,
    registry: &core_document::DocumentService,
    printing: &settings::PrintingSettings,
) -> CommandResult {
    use crate::app::print_layout::{LayoutSettings, lay_out, parts_to_print, piece_mesh};
    let a = Args(args);
    let mut settings = LayoutSettings::of(printing);
    if let Some(gap) = a.opt_number("gap")? {
        if gap < 0.0 {
            return Err(CommandError::bad("gap", "must not be negative"));
        }
        settings.gap_mm = gap as f32;
    }
    if let Some(bed) = args.get("bed") {
        let size = |key: &str, at: usize| {
            bed.get(key)
                .or_else(|| bed.get(at))
                .and_then(Value::as_f64)
                .filter(|v| *v > 0.0)
                .ok_or_else(|| CommandError::bad("bed", "is {x, y, z}, each above 0"))
        };
        settings.bed_mm = [
            size("x", 0)? as f32,
            size("y", 1)? as f32,
            size("z", 2)? as f32,
        ];
    }
    let layout = lay_out(document, &parts_to_print(document, registry), &settings);
    let pieces: Vec<Value> = layout
        .pieces
        .iter()
        .map(|piece| {
            let bounds = piece_mesh(document, piece).and_then(|m| m.bounds());
            json!({
                "body": piece.body.0.to_string(),
                "name": piece.name,
                "plate": piece.plate + 1,
                "min": bounds.map(|b| b.0),
                "max": bounds.map(|b| b.1),
                "transform": piece.placement.rows(),
            })
        })
        .collect();
    Ok(json!({
        "plates": layout.plates,
        "pieces": pieces,
        "too_big": layout.too_big,
        "too_tall": layout.too_tall,
        "unbuilt": layout.unbuilt,
    }))
}

pub(crate) fn document_command(
    id: &str,
    args: &CommandArgs,
    document: &mut core_document::Document,
    registry: &core_document::DocumentService,
    file: Option<&std::path::Path>,
) -> Option<CommandResult> {
    let a = Args(args);
    let answer = (|| match id {
        "doc.agent_rules" => Ok(json!(document.agent_rules())),
        "doc.set_agent_rules" => {
            document.set_agent_rules(a.string("rules")?);
            Ok(Value::Null)
        }
        "doc.info" => Ok(json!({
            "name": document.name(),
            "file": file.map(|p| p.display().to_string()),
            "unit": document.display_unit().short_label(),
            "modified": document.metadata().dirty(),
        })),
        "doc.bodies" => Ok(Value::Array(
            document
                .bodies()
                .iter()
                .map(|b| {
                    json!({
                        "id": b.id.0.to_string(),
                        "name": b.name,
                        "visible": !b.hidden,
                        "frozen": b.frozen,
                        "selectable": !b.unselectable,
                        "material": b.material.as_ref().map(|m| json!({
                            "name": m.name,
                            "density": m.density,
                        })),
                        "features": features_in_order(document, Some(b.id))
                            .iter()
                            .map(|n| n.id.0.to_string())
                            .collect::<Vec<_>>(),
                    })
                })
                .collect(),
        )),
        "doc.features" => {
            let body = a.opt_id("body")?.map(BodyId);
            Ok(Value::Array(
                features_in_order(document, None)
                    .into_iter()
                    .filter(|n| body.is_none() || n.body == body)
                    .map(|n| {
                        json!({
                            "id": n.id.0.to_string(),
                            "name": n.name,
                            "kind": kind_of(registry, n),
                            "body": n.body.map(|b| b.0.to_string()),
                            "visible": n.visible,
                            "suppressed": n.suppressed,
                            "error": n.error,
                        })
                    })
                    .collect(),
            ))
        }
        "doc.feature" => {
            let node = document
                .get_feature_meta(FeatureId(a.id("id")?))
                .ok_or_else(|| CommandError::bad("id", "is not a feature of this document"))?;
            Ok(json!({
                "id": node.id.0.to_string(),
                "name": node.name,
                "kind": kind_of(registry, node),
                "body": node.body.map(|b| b.0.to_string()),
                "visible": node.visible,
                "suppressed": node.suppressed,
                "error": node.error,
                "fields": node.data,
                "unset": unset_fields(&node.data),
                // What it is built from now, where that is not what is kept:
                // formulas worked out, a plane where what it stands on is.
                "values": document
                    .feature_values(node.id)
                    .filter(|v| **v != node.data)
                    .cloned(),
            }))
        }
        "doc.suppress" => {
            let feature = feature_arg(document, &a)?;
            suppress(document, feature, a.opt_bool("suppressed")?.unwrap_or(true));
            Ok(Value::Null)
        }
        "doc.move" => {
            let feature = feature_arg(document, &a)?;
            let up = a.opt_bool("up")?.unwrap_or(true);
            move_in_history(document, feature, up).map_err(CommandError::failed)?;
            Ok(json!(true))
        }
        "doc.set_tip" => {
            let feature = feature_arg(document, &a)?;
            let tip = (!a.opt_bool("clear")?.unwrap_or(false)).then_some(feature);
            set_tip(document, registry, feature, tip).map_err(CommandError::failed)?;
            Ok(Value::Null)
        }
        "doc.new_body" => {
            let body = document.create_body(None);
            if let Some(name) = a.opt_string("name")? {
                document.rename_body(body, name);
            }
            Ok(json!(body.0.to_string()))
        }
        "doc.set_body" => {
            let body = body_arg(document, &a)?;
            set_body(document, registry, body, &a)?;
            Ok(Value::Null)
        }
        "doc.set_textures" => {
            let body = body_arg(document, &a)?;
            let textures = textures_arg(document, body, a.0.get("textures"))?;
            document.set_body_textures(body, textures);
            Ok(Value::Null)
        }
        "doc.set_face_color" => {
            let body = body_arg(document, &a)?;
            let index = a.number("face")? as i64;
            let mesh = document
                .imported_geometry(body)
                .map(|g| std::sync::Arc::clone(&g.mesh))
                .ok_or_else(|| CommandError::failed("the body has no solid yet"))?;
            let faces = mesh.faces.iter().map(|f| *f as i64 + 1).max().unwrap_or(0);
            if index < 0 || index >= faces {
                return Err(CommandError::bad(
                    "face",
                    format!("is not one of its {faces} faces"),
                ));
            }
            let color = match a.0.get("color") {
                None | Some(Value::Null) => None,
                Some(v) => Some(color_arg(v, "color")?),
            };
            document.color_face(body, index as u32, color);
            Ok(Value::Null)
        }
        "doc.linked_copy" => {
            let body = body_arg(document, &a)?;
            let copy = document
                .create_linked_copy(body, None)
                .ok_or_else(|| CommandError::failed("that body cannot be copied"))?;
            // Beside the original, clear of it along X.
            let width = document
                .imported_geometry(body)
                .and_then(|g| g.bounds_mm.or_else(|| g.mesh.bounds()))
                .map_or(10.0, |(lo, hi)| hi[0] - lo[0]);
            let at = document.body_placement(body);
            document.set_body_placement(
                copy,
                core_document::BodyPlacement::new(
                    at.quat(),
                    at.offset() + glam::Vec3::X * (width + 10.0),
                ),
            );
            Ok(json!(copy.0.to_string()))
        }
        "doc.move_after" => {
            let feature = FeatureId(a.id("id")?);
            let after = a.opt_id("after")?.map(FeatureId);
            move_after(document, feature, after).map_err(CommandError::failed)?;
            Ok(Value::Null)
        }
        "doc.recompute" => {
            let body = body_arg(document, &a)?;
            if document.body_frozen(body) {
                return Err(CommandError::bad(
                    "body",
                    "is frozen: it keeps the solid it has until it is thawed \
                     (doc.set_body{frozen = false})",
                ));
            }
            registry.invalidate_body(document, body);
            Ok(Value::Null)
        }
        "doc.replace_shape" => {
            let body = body_arg(document, &a)?;
            let path = a.string("path")?;
            let bytes = crate::platform::read(std::path::Path::new(path))
                .map_err(|e| CommandError::bad("path", format!("could not be read: {e}")))?;
            if !document.replace_body_shape(body, path, bytes) {
                return Err(CommandError::bad(
                    "body",
                    "builds its shape from its history, or takes it from another body or \
                     file; only an imported or converted shape, or a base shape, is replaced",
                ));
            }
            Ok(Value::Null)
        }
        "doc.rename" => {
            let name = a.string("name")?.to_string();
            match tree_item(document, a.id("id")?)? {
                TreeItemId::Body(body) => document.rename_body(body, name),
                TreeItemId::Feature(feature) => document.rename_feature(feature, name),
                TreeItemId::Component(id) => {
                    if let Some(mut component) = document.component(id).cloned() {
                        component.name = name;
                        document
                            .update_component(component)
                            .map_err(|e| CommandError::failed(e.to_string()))?;
                    }
                }
                _ => return Err(CommandError::bad("id", "cannot be renamed")),
            }
            Ok(Value::Null)
        }
        "doc.faces" | "doc.edges" => {
            let body = body_arg(document, &a)?;
            let in_body = match a.opt_string("frame")? {
                None | Some("world") => false,
                Some("body") => true,
                Some(_) => return Err(CommandError::bad("frame", "must be world or body")),
            };
            let mesh = if in_body {
                document.local_geometry(body).map(|(mesh, _)| mesh)
            } else {
                document
                    .imported_geometry(body)
                    .map(|g| std::sync::Arc::clone(&g.mesh))
            }
            .ok_or_else(|| CommandError::failed("the body has no solid yet"))?;
            Ok(if id == "doc.faces" {
                faces_of(&mesh)
            } else {
                crate::app::edges::edges_of(&mesh)
            })
        }
        "doc.measure" => {
            let body = body_arg(document, &a)?;
            let (min, max) = document
                .imported_geometry(body)
                .and_then(|g| g.bounds_mm.or_else(|| g.mesh.bounds()))
                .ok_or_else(|| CommandError::failed("the body has no solid yet"))?;
            let blob = document.imported_brep_blob_arc(body).ok_or_else(|| {
                CommandError::failed("the body is a mesh; it has no solid to measure")
            })?;
            let props = kernel_ogeom::OgeomKernel::new()
                .physical_properties(&blob)
                .map_err(|e| CommandError::failed(e.to_string()))?;
            let c = props.centre_mm.map(|v| v as f32);
            let centre = document.body_placement(body).point(c);
            Ok(json!({
                "volume": props.volume_mm3,
                "area": props.area_mm2,
                "centre": centre,
                "min": min,
                "max": max,
                "approximate": props.approximate,
            }))
        }
        "doc.parameters" => {
            let feature = FeatureId(a.id("id")?);
            let node = document
                .get_feature_meta(feature)
                .ok_or_else(|| CommandError::failed("no such feature"))?
                .clone();
            registry.evaluate(document);
            let unit = document.display_unit();
            let slots = document.evaluated_slots(feature);
            Ok(Value::Array(
                registry
                    .parameters(&node)
                    .into_iter()
                    .map(|p| {
                        let mut row = slots
                            .iter()
                            .find(|s| s.key == p.key)
                            .map(|s| slot_json(&s.result, unit))
                            .unwrap_or_else(|| json!({}));
                        row["name"] = json!(p.name);
                        row["key"] = json!(p.key);
                        row["label"] = json!(p.label);
                        row["kind"] = json!(kind_name(p.dim));
                        row["formula"] = json!(node.formulas.get(&p.key));
                        row
                    })
                    .collect(),
            ))
        }
        "doc.set_formula" => {
            let feature = FeatureId(a.id("id")?);
            let parameter = registry
                .parameter_named(document, feature, a.string("parameter")?)
                .map_err(CommandError::failed)?;
            let formula = a.opt_string("formula")?.map(str::to_string);
            match &formula {
                Some(text) => core_document::expr::check_syntax(text)
                    .map_err(|e| CommandError::failed(e.message))?,
                // What the formula comes to now is the number kept.
                None => registry.evaluate(document),
            }
            document
                .set_feature_formula(feature, parameter.key.clone(), formula)
                .map_err(|e| CommandError::failed(e.to_string()))?;
            Ok(slot_answer(document, registry, feature, &parameter.key))
        }
        "doc.set_value" => {
            let feature = FeatureId(a.id("id")?);
            let parameter = registry
                .parameter_named(document, feature, a.string("parameter")?)
                .map_err(CommandError::failed)?;
            registry
                .set_parameter_value(document, feature, &parameter, a.number("value")?)
                .map(|()| Value::Null)
                .map_err(CommandError::failed)
        }
        "var.new" => document
            .add_variable_set(a.string("name")?)
            .map(|id| json!(id.0.to_string()))
            .map_err(CommandError::failed),
        "var.set" => {
            let set = set_arg(document, &a)?;
            let name = a.string("name")?.to_string();
            document
                .set_variable(set, &name, a.string("formula")?, a.opt_string("comment")?)
                .map_err(CommandError::failed)?;
            Ok(slot_answer(document, registry, set, &name))
        }
        "var.remove" => {
            let set = set_arg(document, &a)?;
            document
                .remove_variable(set, a.string("name")?)
                .map(|()| Value::Null)
                .map_err(CommandError::failed)
        }
        "var.rename" => {
            let set = set_arg(document, &a)?;
            document
                .rename_variable(set, a.string("name")?, a.string("to")?)
                .map(|()| Value::Null)
                .map_err(CommandError::failed)
        }
        "var.list" => {
            let only = match a.opt_string("set")? {
                Some(_) => Some(set_arg(document, &a)?),
                None => None,
            };
            registry.evaluate(document);
            let unit = document.display_unit();
            Ok(Value::Array(
                document
                    .variable_sets()
                    .into_iter()
                    .filter(|(id, ..)| only.is_none_or(|o| o == *id))
                    .map(|(id, name, set)| {
                        let slots = document.evaluated_slots(id);
                        let variables: Vec<Value> = set
                            .variables
                            .iter()
                            .map(|v| {
                                let mut row = slots
                                    .iter()
                                    .find(|s| s.key == v.name)
                                    .map(|s| slot_json(&s.result, unit))
                                    .unwrap_or_else(|| json!({}));
                                row["name"] = json!(v.name);
                                row["formula"] = json!(v.formula);
                                row["comment"] = json!(v.comment);
                                row
                            })
                            .collect();
                        json!({"id": id.0.to_string(), "name": name, "variables": variables})
                    })
                    .collect(),
            ))
        }
        "config.list" => {
            let table = document
                .configurations()
                .map(|(_, t)| t)
                .unwrap_or_default();
            Ok(json!({
                "columns": table.columns,
                "rows": table.rows.iter().map(|r| json!({
                    "name": r.name,
                    "values": r.values,
                    "left_out": r.left_out.iter().map(|b| b.0.to_string()).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "active": table.active,
            }))
        }
        "config.new" => document
            .add_configuration(a.string("name")?, a.opt_string("like")?)
            .map(|()| Value::Null)
            .map_err(CommandError::failed),
        "config.remove" => document
            .remove_configuration(a.string("name")?)
            .map(|()| Value::Null)
            .map_err(CommandError::failed),
        "config.rename" => document
            .rename_configuration(a.string("name")?, a.string("to")?)
            .map(|()| Value::Null)
            .map_err(CommandError::failed),
        "config.add_variable" => document
            .add_configuration_column(a.string("variable")?)
            .map(|()| Value::Null)
            .map_err(CommandError::failed),
        "config.remove_variable" => document
            .remove_configuration_column(a.string("variable")?)
            .map(|()| Value::Null)
            .map_err(CommandError::failed),
        "config.set" => document
            .set_configuration_value(a.string("name")?, a.string("variable")?, a.string("value")?)
            .map(|()| Value::Null)
            .map_err(CommandError::failed),
        "config.leave_out" => {
            let bodies =
                a.0.get("bodies")
                    .and_then(Value::as_array)
                    .ok_or_else(|| CommandError::bad("bodies", "must be a list of ids"))?
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .and_then(|t| Uuid::parse_str(t).ok())
                            .map(core_document::BodyId)
                            .ok_or_else(|| CommandError::bad("bodies", "must be a list of ids"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
            document
                .set_configuration_left_out(a.string("name")?, bodies)
                .map(|()| Value::Null)
                .map_err(CommandError::failed)
        }
        "config.activate" => document
            .activate_configuration(a.opt_string("name")?)
            .map(|()| Value::Null)
            .map_err(CommandError::failed),
        "var.eval" => {
            let q = registry
                .evaluate_formula(document, a.string("formula")?, None)
                .map_err(CommandError::failed)?;
            Ok(json!({
                "value": q.value,
                "kind": kind_name(q.dim),
                "text": q.display(document.display_unit(), 4),
            }))
        }
        _ => Err(CommandError::Unknown(id.to_string())),
    })();
    match answer {
        Err(CommandError::Unknown(_)) => None,
        answer => Some(answer),
    }
}

fn feature_arg(document: &core_document::Document, a: &Args) -> Result<FeatureId, CommandError> {
    let feature = FeatureId(a.id("id")?);
    if document.get_feature_meta(feature).is_some() {
        Ok(feature)
    } else {
        Err(CommandError::bad("id", "is not a feature of this document"))
    }
}

/// Leave `feature` out of its body's solid, or put it back.
pub(crate) fn suppress(document: &mut core_document::Document, feature: FeatureId, on: bool) {
    document.set_feature_suppressed(feature, on);
    document.mark_feature_dirty(feature);
}

/// Move `feature` a step earlier (`up`) or later, or say why it cannot go.
pub(crate) fn move_in_history(
    document: &mut core_document::Document,
    feature: FeatureId,
    up: bool,
) -> Result<(), String> {
    use core_document::MoveRefused;
    let name = |document: &core_document::Document, id: FeatureId| {
        document
            .get_feature_meta(id)
            .map(|n| n.name.clone())
            .unwrap_or_default()
    };
    document
        .try_move_feature_in_history(feature, up)
        .map_err(|refused| match refused {
            MoveRefused::NotFound => "no such feature".to_string(),
            MoveRefused::AtEnd if up => {
                format!("{} is already first in its body", name(document, feature))
            }
            MoveRefused::AtEnd => {
                format!("{} is already last in its body", name(document, feature))
            }
            MoveRefused::Dependency { neighbour } => {
                let (this, other) = (name(document, feature), name(document, neighbour));
                if up {
                    format!("{this} cannot go before {other}, which it is built from")
                } else {
                    format!("{this} cannot go after {other}, which is built from it")
                }
            }
        })
}

/// Build the body of `of` only up to `tip`, or all of it when `None`.
pub(crate) fn set_tip(
    document: &mut core_document::Document,
    registry: &core_document::DocumentService,
    of: FeatureId,
    tip: Option<FeatureId>,
) -> Result<(), &'static str> {
    let body = document
        .get_feature_meta(of)
        .and_then(|n| n.body)
        .ok_or("the feature belongs to no body")?;
    document.set_body_tip(body, tip);
    // The chain changes shape: rebuild from the first feature.
    registry.invalidate_body(document, body);
    Ok(())
}

/// A colour given as {r, g, b} (or a list of three), each 0 to 1.
fn color_arg(value: &Value, name: &str) -> Result<[f32; 3], CommandError> {
    let part = |key: &str, i: usize| {
        value
            .get(key)
            .or_else(|| value.get(i))
            .and_then(Value::as_f64)
            .map(|v| v.clamp(0.0, 1.0) as f32)
    };
    match (part("r", 0), part("g", 1), part("b", 2)) {
        (Some(r), Some(g), Some(b)) => Ok([r, g, b]),
        _ => Err(CommandError::bad(name, "is {r, g, b}, each from 0 to 1")),
    }
}

/// Numbers given as a table of named parts or a list, in `keys` order.
fn numbers_arg<const N: usize>(
    value: &Value,
    name: &str,
    keys: [&str; N],
) -> Result<[f32; N], CommandError> {
    let mut out = [0.0f32; N];
    for (i, key) in keys.iter().enumerate() {
        out[i] = value
            .get(*key)
            .or_else(|| value.get(i))
            .and_then(Value::as_f64)
            .ok_or_else(|| CommandError::bad(name, format!("is {{{}}}", keys.join(", "))))?
            as f32;
    }
    Ok(out)
}

/// What `doc.set_body` changes, each named field in turn.
fn set_body(
    document: &mut core_document::Document,
    registry: &core_document::DocumentService,
    body: BodyId,
    a: &Args,
) -> Result<(), CommandError> {
    let entry = document
        .bodies()
        .iter()
        .find(|b| b.id == body)
        .cloned()
        .ok_or_else(|| CommandError::bad("body", "is not a body of this document"))?;
    let color = a.0.get("color");
    let opacity = a.opt_number("opacity")?;
    if color.is_some() || opacity.is_some() {
        let display = match color {
            Some(Value::Null) if opacity.is_none() => None,
            _ => {
                let mut display = entry.display.unwrap_or_default();
                if let Some(v) = color.filter(|v| !v.is_null()) {
                    display.color = color_arg(v, "color")?;
                }
                if let Some(o) = opacity {
                    display.opacity = o.clamp(0.05, 1.0) as f32;
                }
                Some(display)
            }
        };
        document.set_body_display(body, display);
    }
    match a.0.get("material") {
        None => {}
        Some(Value::Null) => document.set_body_material(body, None),
        Some(v) => {
            let name = v
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Material")
                .to_string();
            let density = v
                .get("density")
                .and_then(Value::as_f64)
                .filter(|d| *d > 0.0)
                .ok_or_else(|| CommandError::bad("material", "needs a density above 0, g/cm³"))?;
            document.set_body_material(
                body,
                Some(core_document::Material {
                    name,
                    density: density as f32,
                }),
            );
        }
    }
    if let Some(frozen) = a.opt_bool("frozen")? {
        document.set_body_frozen(body, frozen);
        if !frozen && entry.frozen {
            // What changed while it was frozen builds now.
            registry.invalidate_body(document, body);
        }
    }
    if let Some(selectable) = a.opt_bool("selectable")? {
        document.set_body_selectable(body, selectable);
    }
    match a.0.get("face_colors") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) if list.is_empty() => {
            for c in &entry.face_colors {
                document.set_face_color(body, c.index, c.name, None);
            }
        }
        Some(_) => {
            return Err(CommandError::bad(
                "face_colors",
                "takes only an empty list; doc.set_face_color colours a face",
            ));
        }
    }
    let translation = a.0.get("translation").filter(|v| !v.is_null());
    let rotation = a.0.get("rotation").filter(|v| !v.is_null());
    if translation.is_some() || rotation.is_some() {
        let now = document.body_placement(body);
        let offset = match translation {
            Some(v) => glam::Vec3::from_array(numbers_arg(v, "translation", ["x", "y", "z"])?),
            None => now.offset(),
        };
        let turn = match rotation {
            Some(v) => glam::Quat::from_array(numbers_arg(v, "rotation", ["x", "y", "z", "w"])?)
                .normalize(),
            None => now.quat(),
        };
        document.place_with_unit(body, core_document::BodyPlacement::new(turn, offset));
    }
    Ok(())
}

/// The feature a drop next to `target` lands after: `target` itself, or
/// for a drop before it, the one before it in its body's history
/// (`Some(None)`: the start). `None` when `target` is in no history.
pub(crate) fn drop_place(
    document: &core_document::Document,
    target: FeatureId,
    before: bool,
) -> Option<Option<FeatureId>> {
    let order = document.body_history_of(target);
    let at = order.iter().position(|f| *f == target)?;
    Some(if before {
        at.checked_sub(1).map(|i| order[i])
    } else {
        Some(target)
    })
}

/// Move `feature` in its body's history to just after `after`, or first
/// for `None`, saying why a step is refused.
pub(crate) fn move_after(
    document: &mut core_document::Document,
    feature: FeatureId,
    after: Option<FeatureId>,
) -> Result<(), String> {
    use core_document::MoveRefused;
    let name = |document: &core_document::Document, id: FeatureId| {
        document
            .get_feature_meta(id)
            .map(|n| n.name.clone())
            .unwrap_or_default()
    };
    document
        .move_feature_after(feature, after)
        .map_err(|refused| match refused {
            MoveRefused::NotFound => "the two features are not in one body".to_string(),
            MoveRefused::AtEnd => "it can go no further".to_string(),
            MoveRefused::Dependency { neighbour } => format!(
                "{} and {} depend on each other and keep their order",
                name(document, feature),
                name(document, neighbour)
            ),
        })
}

/// A body's textures as a script gives them: each face a number (as
/// `doc.faces` has it) or `{name, index}` as a recording writes it.
fn textures_arg(
    document: &core_document::Document,
    body: BodyId,
    value: Option<&Value>,
) -> Result<Vec<core_document::FaceTexture>, CommandError> {
    let list = value
        .and_then(Value::as_array)
        .ok_or_else(|| CommandError::bad("textures", "must be a list"))?;
    let mesh = document
        .imported_geometry(body)
        .map(|g| std::sync::Arc::clone(&g.mesh));
    list.iter()
        .map(|item| {
            let texture: surface_texture::Texture =
                serde_json::from_value(item.get("texture").cloned().unwrap_or(Value::Null))
                    .map_err(|e| {
                        CommandError::bad("textures", format!("has a texture it cannot read: {e}"))
                    })?;
            let faces = item
                .get("faces")
                .and_then(Value::as_array)
                .map(|faces| {
                    faces
                        .iter()
                        .map(|face| match face.as_u64() {
                            Some(index) => mesh
                                .as_deref()
                                .map(|m| core_document::FaceKey::of(m, index as u32))
                                .ok_or_else(|| CommandError::failed("the body has no solid yet")),
                            None => serde_json::from_value(face.clone()).map_err(|e| {
                                CommandError::bad(
                                    "textures",
                                    format!("has a face it cannot read: {e}"),
                                )
                            }),
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?
                .unwrap_or_default();
            Ok(core_document::FaceTexture { texture, faces })
        })
        .collect()
}

fn body_arg(document: &core_document::Document, a: &Args) -> Result<BodyId, CommandError> {
    let body = BodyId(a.id("body")?);
    if document.bodies().iter().any(|b| b.id == body) {
        Ok(body)
    } else {
        Err(CommandError::bad("body", "is not a body of this document"))
    }
}

/// The names of a feature's fields that hold no value, which a script's
/// table cannot show (a nil is no entry): at the top of the data or inside
/// its one variant, where a feature kept as an enum holds its fields.
fn unset_fields(data: &Value) -> Vec<String> {
    let Value::Object(top) = data else {
        return Vec::new();
    };
    let fields = match top.values().next() {
        Some(Value::Object(inner)) if top.len() == 1 => inner,
        _ => top,
    };
    fields
        .iter()
        .filter(|(_, v)| v.is_null())
        .map(|(k, _)| k.clone())
        .collect()
}

/// What kind of feature `node` is, as its workbench names it.
fn kind_of(registry: &core_document::DocumentService, node: &core_document::FeatureNode) -> String {
    registry
        .feature_info(node)
        .map(|info| info.kind_label)
        .unwrap_or_else(|| node.workbench_id.as_str().to_string())
}

/// The tree row an id names: a body, a feature or an imported part.
pub(crate) fn tree_item(
    document: &core_document::Document,
    id: Uuid,
) -> Result<TreeItemId, CommandError> {
    if document.bodies().iter().any(|b| b.id.0 == id) {
        Ok(TreeItemId::Body(BodyId(id)))
    } else if document.get_feature_meta(FeatureId(id)).is_some() {
        Ok(TreeItemId::Feature(FeatureId(id)))
    } else if document.imported_object(id).is_some() {
        Ok(TreeItemId::ImportedObject(id))
    } else if document.component(core_document::ComponentId(id)).is_some() {
        Ok(TreeItemId::Component(core_document::ComponentId(id)))
    } else {
        Err(CommandError::bad("id", "is not in this document"))
    }
}

/// The format an export names, else the one its path's extension says.
pub(crate) fn export_format(
    name: Option<&str>,
    path: &std::path::Path,
) -> Result<kernel_ogeom::export::ExportFormat, CommandError> {
    use kernel_ogeom::export::ExportFormat;
    match name {
        Some(name) => match name.to_ascii_lowercase().as_str() {
            "step" | "stp" => Ok(ExportFormat::Step),
            "step_nurbs" => Ok(ExportFormat::StepNurbs),
            "stl" => Ok(ExportFormat::Stl),
            "3mf" => Ok(ExportFormat::ThreeMf),
            _ => Err(CommandError::bad(
                "format",
                "must be step, step_nurbs, stl or 3mf",
            )),
        },
        None => ExportFormat::of_path(path).ok_or_else(|| {
            CommandError::bad(
                "format",
                "is needed when the path has no .step, .stl or .3mf",
            )
        }),
    }
}

/// A list of body ids, when one is given.
pub(crate) fn body_list(value: Option<&Value>) -> Result<Option<Vec<BodyId>>, CommandError> {
    match value {
        Some(Value::Array(list)) => list
            .iter()
            .map(|v| {
                v.as_str()
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .map(BodyId)
                    .ok_or_else(|| CommandError::bad("bodies", "must be a list of body ids"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        _ => Ok(None),
    }
}

/// The features of `body` (or all of them) in build order.
/// What `doc.rebuild` answers: every feature that failed, in history
/// order, as `{feature, name, error}`: its build error, or the formula of
/// its that fails.
pub(crate) fn rebuild_failures(document: &core_document::Document) -> Vec<Value> {
    features_in_order(document, None)
        .into_iter()
        .filter_map(|n| {
            let error = n.error.clone().or_else(|| document.formula_error(n.id))?;
            Some(json!({"feature": n.id.0.to_string(), "name": n.name, "error": error}))
        })
        .collect()
}

fn features_in_order(
    document: &core_document::Document,
    body: Option<BodyId>,
) -> Vec<&core_document::FeatureNode> {
    let mut nodes: Vec<_> = document
        .feature_tree()
        .all_nodes()
        .map(|(_, n)| n)
        .filter(|n| body.is_none() || n.body == body)
        .collect();
    nodes.sort_by_key(|n| n.seq);
    nodes
}

/// Each face of a mesh: what surface it is, a point on it, its area, and a
/// flat face's normal or a turned face's axis.
fn faces_of(mesh: &kernel_api::TriMesh) -> Value {
    let count = mesh.face_surfaces.len().max(
        mesh.faces
            .iter()
            .map(|f| *f as usize + 1)
            .max()
            .unwrap_or(0),
    );
    // Per face: area, the area-weighted centre, and the triangle centre
    // nearest it (a point that is on the face even when it curves).
    let mut area = vec![0.0f32; count];
    let mut centre = vec![[0.0f32; 3]; count];
    let tri = |t: usize| {
        let p = |i: usize| glam::Vec3::from(mesh.positions[mesh.indices[3 * t + i] as usize]);
        (p(0), p(1), p(2))
    };
    for (t, face) in mesh.faces.iter().enumerate() {
        let (a, b, c) = tri(t);
        let w = (b - a).cross(c - a).length() / 2.0;
        let f = *face as usize;
        area[f] += w;
        let m = (a + b + c) / 3.0;
        for i in 0..3 {
            centre[f][i] += m[i] * w;
        }
    }
    let mut on_face = vec![None::<(f32, glam::Vec3)>; count];
    for (t, face) in mesh.faces.iter().enumerate() {
        let f = *face as usize;
        if area[f] <= 0.0 {
            continue;
        }
        let target = glam::Vec3::from(centre[f]) / area[f];
        let (a, b, c) = tri(t);
        let m = (a + b + c) / 3.0;
        let d = m.distance(target);
        if on_face[f].is_none_or(|(best, _)| d < best) {
            on_face[f] = Some((d, m));
        }
    }
    let list = (0..count)
        .filter_map(|f| {
            let (_, point) = on_face[f]?;
            let surface = mesh.face_surfaces.get(f).copied().unwrap_or_default();
            let kind = match surface {
                kernel_api::FaceSurface::Plane { .. } => "plane",
                kernel_api::FaceSurface::Cylinder { .. } => "cylinder",
                kernel_api::FaceSurface::Cone { .. } => "cone",
                kernel_api::FaceSurface::Sphere { .. } => "sphere",
                kernel_api::FaceSurface::Torus { .. } => "torus",
                kernel_api::FaceSurface::Other => "other",
            };
            let mut out = json!({
                "index": f,
                "kind": kind,
                "point": point.to_array(),
                "area": area[f],
            });
            match surface {
                kernel_api::FaceSurface::Plane { normal, .. } => out["normal"] = json!(normal),
                kernel_api::FaceSurface::Cylinder { radius, .. } => out["radius"] = json!(radius),
                kernel_api::FaceSurface::Sphere { radius, center } => {
                    out["radius"] = json!(radius);
                    out["centre"] = json!(center);
                }
                _ => {}
            }
            if let Some((p, d)) = surface.axis() {
                out["axis"] = json!({"point": p, "direction": d});
            }
            // What a rebuild finds the face by, where it has a name: a
            // string, since a script's numbers cannot hold every name.
            if let Some(name) = mesh.face_names.get(f).filter(|n| **n != 0) {
                out["name"] = json!(name.to_string());
            }
            Some(out)
        })
        .collect();
    Value::Array(list)
}

fn item_id(item: TreeItemId) -> Option<Uuid> {
    match item {
        TreeItemId::Body(b) => Some(b.0),
        TreeItemId::Feature(f) => Some(f.0),
        TreeItemId::ImportedObject(n) => Some(n),
        TreeItemId::Component(c) => Some(c.0),
        TreeItemId::DocumentRoot => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faces_are_listed_with_a_point_on_each_and_what_surface_it_is() {
        let mesh = kernel_api::TriMesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                [10.0, 10.0, 0.0],
                [0.0, 10.0, 0.0],
                [0.0, 0.0, 5.0],
                [1.0, 0.0, 5.0],
                [0.0, 1.0, 5.0],
            ],
            indices: vec![0, 1, 2, 0, 2, 3, 4, 5, 6],
            faces: vec![0, 0, 1],
            face_surfaces: vec![
                kernel_api::FaceSurface::Plane {
                    origin: [0.0; 3],
                    normal: [0.0, 0.0, -1.0],
                },
                kernel_api::FaceSurface::Cylinder {
                    origin: [0.0; 3],
                    axis: [0.0, 0.0, 1.0],
                    radius: 3.0,
                },
            ],
            ..Default::default()
        };
        let faces = faces_of(&mesh);
        let faces = faces.as_array().unwrap();
        assert_eq!(faces.len(), 2);
        assert_eq!(faces[0]["kind"], "plane");
        assert_eq!(faces[0]["normal"], json!([0.0, 0.0, -1.0]));
        assert!((faces[0]["area"].as_f64().unwrap() - 100.0).abs() < 1e-3);
        assert_eq!(faces[0]["point"][2], json!(0.0));
        assert_eq!(faces[1]["kind"], "cylinder");
        assert_eq!(faces[1]["axis"]["direction"], json!([0.0, 0.0, 1.0]));
        assert_eq!(faces[1]["radius"], json!(3.0));
    }

    /// An agent's script answers what it returned beside what it printed,
    /// or what stopped it.
    #[test]
    fn a_script_answers_the_agent_its_return_value_and_output() {
        let answer = |output: scripting::RunOutput| {
            let answer = script_answer(output);
            let agents::mcp::Content::Text(text) = &answer.content[0] else {
                panic!("text");
            };
            (
                answer.is_error,
                serde_json::from_str::<Value>(text).unwrap(),
            )
        };
        let (failed, body) = answer(scripting::RunOutput {
            printed: vec!["one".into(), "two".into()],
            returned: Some(json!({"pad": "f-1", "volume": 1000.0})),
            ..Default::default()
        });
        assert!(!failed);
        assert_eq!(
            body,
            json!({"returned": {"pad": "f-1", "volume": 1000.0}, "printed": "one\ntwo"})
        );
        let (_, body) = answer(scripting::RunOutput::default());
        assert_eq!(body, json!({"returned": null, "printed": ""}));
        let (failed, body) = answer(scripting::RunOutput {
            error: Some("design.pad: no sketch".into()),
            ..Default::default()
        });
        assert!(failed);
        assert_eq!(body["error"], "design.pad: no sketch");
    }

    /// Where the reference sits in `docs/SCRIPTING.md`.
    const BEGIN: &str = "<!-- commands: generated from the registered commands -->";
    const END: &str = "<!-- /commands -->";

    #[test]
    fn the_scripting_guide_lists_every_command_as_registered() {
        let mut registry = core_document::DocumentService::default();
        workbenches::register_all_workbenches(&mut registry).unwrap();
        let generated = reference(&command_specs(&registry));
        assert!(!generated.contains('\u{2014}'), "the docs use no long dash");
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/SCRIPTING.md");
        let text = std::fs::read_to_string(&path).unwrap();
        let (Some(start), Some(end)) = (text.find(BEGIN), text.find(END)) else {
            panic!("docs/SCRIPTING.md has no command reference markers");
        };
        let current = &text[start + BEGIN.len()..end];
        let wanted = format!("\n{}\n", generated.trim());
        if current != wanted {
            if std::env::var_os("PRINTCAD_WRITE_DOCS").is_some() {
                let updated = format!("{}{BEGIN}{wanted}{}", &text[..start], &text[end..]);
                std::fs::write(&path, updated).unwrap();
            } else {
                panic!(
                    "docs/SCRIPTING.md's command reference is out of date; \
                     PRINTCAD_WRITE_DOCS=1 cargo test -p app_shell scripting_guide rewrites it"
                );
            }
        }
    }

    /// The ```lua blocks of a markdown text.
    fn lua_blocks(text: &str) -> Vec<String> {
        let mut blocks = Vec::new();
        let mut open: Option<String> = None;
        for line in text.lines() {
            match open.as_mut() {
                None if line.trim_start() == "```lua" => open = Some(String::new()),
                Some(_) if line.trim_start() == "```" => blocks.extend(open.take()),
                Some(block) => {
                    block.push_str(line);
                    block.push('\n');
                }
                None => {}
            }
        }
        blocks
    }

    #[test]
    fn markdown_lua_blocks_are_found_whole() {
        let text =
            "Text\n```lua\nlocal a = 1\nassert(a)\n```\n```\nnot lua\n```\n```lua\nb()\n```\n";
        assert_eq!(lua_blocks(text), ["local a = 1\nassert(a)\n", "b()\n"]);
    }

    /// Every example of every command, and every Lua block of every recipe
    /// under `docs/recipes/`, runs from an empty document as `printcad
    /// --script` runs a file, and its asserts hold.
    #[test]
    fn every_command_example_and_recipe_runs() {
        let mut registry = core_document::DocumentService::default();
        workbenches::register_all_workbenches(&mut registry).unwrap();
        let mut runs: Vec<(String, String, String)> = command_specs(&registry)
            .into_iter()
            .flat_map(|c| {
                c.examples
                    .into_iter()
                    .map(move |e| (c.id.clone(), e.title, e.script))
            })
            .collect();
        let recipes = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/recipes");
        let mut files: Vec<_> = std::fs::read_dir(&recipes)
            .expect("docs/recipes")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "md"))
            .collect();
        files.sort();
        assert!(!files.is_empty(), "docs/recipes has recipes");
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            let name = format!(
                "docs/recipes/{}",
                file.file_name().unwrap().to_string_lossy()
            );
            assert!(
                !text.contains('\u{2014}'),
                "{name}: the docs use no long dash"
            );
            let blocks = lua_blocks(&text);
            assert!(!blocks.is_empty(), "{name} has a lua block");
            for (n, block) in blocks.into_iter().enumerate() {
                runs.push((name.clone(), format!("block {}", n + 1), block));
            }
        }
        let mut failed = Vec::new();
        for (owner, title, script) in &runs {
            if let Err(error) = crate::headless::run_in_empty_document(&mut registry, script, title)
            {
                failed.push(format!("{owner}, \"{title}\": {error}"));
            }
        }
        assert!(
            failed.is_empty(),
            "examples that fail:\n{}",
            failed.join("\n")
        );
    }

    #[test]
    fn host_ui_edits_record_as_the_document_s_commands() {
        use crate::ui::{TreeFeatureCommand, UiCommand};
        let body = BodyId(Uuid::new_v4());
        let feature = FeatureId(Uuid::new_v4());
        let rename = recorded_of(&UiCommand::RenameTreeItem {
            item: TreeItemId::Body(body),
            name: "Frame".into(),
        })
        .unwrap();
        assert_eq!(rename.id, "doc.rename");
        assert_eq!(rename.args["name"], json!("Frame"));
        let hide = recorded_of(&UiCommand::TreeFeature {
            feature,
            command: TreeFeatureCommand::SetVisible(false),
        })
        .unwrap();
        assert_eq!(
            (hide.id.as_str(), &hide.args["visible"]),
            ("doc.set_visible", &json!(false))
        );
        let delete = recorded_of(&UiCommand::DeleteTreeItem(TreeItemId::Feature(feature))).unwrap();
        assert_eq!(delete.args["id"], json!(feature.0.to_string()));
        assert!(recorded_of(&UiCommand::FitView).is_none());
        let mut more = Vec::new();
        for command in [
            TreeFeatureCommand::Suppress(true),
            TreeFeatureCommand::MoveUp,
            TreeFeatureCommand::ClearTip,
        ] {
            more.push(recorded_of(&UiCommand::TreeFeature { feature, command }).unwrap());
        }
        more.push(recorded_of(&UiCommand::RepairShapes(vec![body])).unwrap());
        more.push(recorded_of(&UiCommand::ConvertToSolid(vec![body])).unwrap());
        more.push(recorded_of(&UiCommand::RefineShapes(vec![body])).unwrap());
        assert_eq!(
            more.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            [
                "doc.suppress",
                "doc.move",
                "doc.set_tip",
                "doc.repair",
                "doc.convert_to_solid",
                "doc.refine"
            ]
        );
        // Every call a recording can hold is a command a script can call.
        for call in [rename, hide, delete].into_iter().chain(more) {
            assert!(doc_commands().iter().any(|c| c.id == call.id));
            let spec = doc_commands()
                .into_iter()
                .find(|c| c.id == call.id)
                .unwrap();
            spec.check(&call.args).unwrap();
        }
    }

    #[test]
    fn the_tree_s_history_edits_work_on_the_document_as_commands() {
        let mut registry = core_document::DocumentService::default();
        workbenches::register_all_workbenches(&mut registry).unwrap();
        let mut doc = core_document::Document::new("t");
        let body = doc.create_body(None);
        let datum = |doc: &mut core_document::Document, name: &str| {
            doc.add_feature_in_body(
                core_document::DatumFeature {
                    shape: core_document::DatumShape::Point,
                    attachment: core_document::DatumAttachment::BasePlane(
                        core_document::BasePlane::XY,
                    ),
                    offset: Default::default(),
                },
                name.to_string(),
                Some(body),
            )
            .unwrap()
        };
        let (a, b) = (datum(&mut doc, "a"), datum(&mut doc, "b"));
        let run = |doc: &mut core_document::Document, id: &str, args: Value| {
            let args = match args {
                Value::Object(map) => map,
                _ => CommandArgs::new(),
            };
            document_command(id, &args, doc, &registry, None).unwrap()
        };
        run(&mut doc, "doc.suppress", json!({"id": a.0.to_string()})).unwrap();
        assert!(doc.get_feature_meta(a).unwrap().suppressed);
        let moved = run(
            &mut doc,
            "doc.move",
            json!({"id": b.0.to_string(), "up": true}),
        )
        .unwrap();
        assert_eq!(moved, json!(true));
        let order: Vec<String> = features_in_order(&doc, Some(body))
            .iter()
            .map(|n| n.name.clone())
            .collect();
        assert_eq!(order, ["b", "a"]);
        let stuck = run(
            &mut doc,
            "doc.move",
            json!({"id": b.0.to_string(), "up": true}),
        )
        .unwrap_err();
        assert!(
            stuck.to_string().contains("already first"),
            "the first can go no earlier, and says so: {stuck}"
        );
        run(&mut doc, "doc.set_tip", json!({"id": b.0.to_string()})).unwrap();
        assert_eq!(doc.bodies()[0].tip, Some(b));
        run(
            &mut doc,
            "doc.set_tip",
            json!({"id": b.0.to_string(), "clear": true}),
        )
        .unwrap();
        assert_eq!(doc.bodies()[0].tip, None);
    }

    #[test]
    fn a_body_menu_s_edits_are_commands() {
        let registry = core_document::DocumentService::default();
        let mut doc = core_document::Document::new("t");
        let body = doc.create_body(None);
        let mesh = kernel_api::TriMesh {
            positions: vec![[0.0; 3], [4.0, 0.0, 0.0], [0.0, 4.0, 0.0], [4.0, 4.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            indices: vec![0, 1, 2, 1, 3, 2],
            faces: vec![0, 1],
            face_names: vec![11, 12],
            ..Default::default()
        };
        doc.set_imported_geometry(
            body,
            core_document::ImportedGeometry {
                mesh: std::sync::Arc::new(mesh),
                source_asset: None,
                revision: 0,
                bounds_mm: None,
                brep_blob_path: None,
                mesh_path: None,
                face_colors_path: None,
                health: None,
            },
        );
        let run = |doc: &mut core_document::Document, id: &str, args: Value| {
            let args = match args {
                Value::Object(map) => map,
                _ => CommandArgs::new(),
            };
            doc_commands()
                .into_iter()
                .find(|c| c.id == id)
                .unwrap()
                .check(&args)
                .unwrap();
            document_command(id, &args, doc, &registry, None).unwrap()
        };
        let id = body.0.to_string();
        run(
            &mut doc,
            "doc.set_body",
            json!({"body": id, "frozen": true, "selectable": false, "opacity": 0.5,
                   "material": {"name": "PETG", "density": 1.27},
                   "translation": [1, 2, 3]}),
        )
        .unwrap();
        let entry = doc.bodies()[0].clone();
        assert!(entry.frozen && entry.unselectable);
        assert_eq!(entry.display.unwrap().opacity, 0.5);
        assert_eq!(entry.material.unwrap().name, "PETG");
        assert_eq!(entry.placement.translation, [1.0, 2.0, 3.0]);
        run(
            &mut doc,
            "doc.set_face_color",
            json!({"body": id, "face": 1, "color": {"r": 1, "g": 0, "b": 0}}),
        )
        .unwrap();
        assert_eq!(doc.bodies()[0].face_colors[0].name, 12);
        assert!(
            run(
                &mut doc,
                "doc.set_face_color",
                json!({"body": id, "face": 5})
            )
            .is_err(),
            "a face the body lacks"
        );
        run(
            &mut doc,
            "doc.set_body",
            json!({"body": id, "face_colors": []}),
        )
        .unwrap();
        assert!(doc.bodies()[0].face_colors.is_empty());
        let copy = run(&mut doc, "doc.linked_copy", json!({"body": id})).unwrap();
        let copy = BodyId(uuid::Uuid::parse_str(copy.as_str().unwrap()).unwrap());
        assert_eq!(
            doc.bodies().iter().find(|b| b.id == copy).unwrap().copy_of,
            Some(body)
        );
        assert!(
            doc.body_placement(copy).translation[0] > 4.0,
            "beside the original"
        );
        // What the menus record is a call a script can make.
        for edit in [
            crate::ui::BodyEdit::Frozen(false),
            crate::ui::BodyEdit::Selectable(true),
            crate::ui::BodyEdit::Recompute,
        ] {
            let (id, args) = body_edit_call(body, &edit);
            run(&mut doc, id, args).unwrap();
        }
        assert!(!doc.bodies()[0].frozen);
    }

    #[test]
    fn features_move_after_another() {
        let mut registry = core_document::DocumentService::default();
        workbenches::register_all_workbenches(&mut registry).unwrap();
        let mut doc = core_document::Document::new("t");
        let body = doc.create_body(None);
        let ids: Vec<FeatureId> = ["a", "b", "c", "d"]
            .iter()
            .map(|name| {
                doc.add_feature_in_body(
                    core_document::DatumFeature {
                        shape: core_document::DatumShape::Point,
                        attachment: core_document::DatumAttachment::BasePlane(
                            core_document::BasePlane::XY,
                        ),
                        offset: Default::default(),
                    },
                    name.to_string(),
                    Some(body),
                )
                .unwrap()
            })
            .collect();
        let order = |doc: &core_document::Document| -> Vec<String> {
            features_in_order(doc, Some(body))
                .iter()
                .map(|n| n.name.clone())
                .collect()
        };
        move_after(&mut doc, ids[0], Some(ids[2])).unwrap();
        assert_eq!(order(&doc), ["b", "c", "a", "d"]);
        move_after(&mut doc, ids[3], Some(ids[1])).unwrap();
        assert_eq!(order(&doc), ["b", "d", "c", "a"]);
        move_after(&mut doc, ids[0], None).unwrap();
        assert_eq!(order(&doc), ["a", "b", "d", "c"]);

        // A drop above a row goes before it, below it after it.
        let place = |doc: &core_document::Document, target, before| {
            drop_place(doc, target, before).unwrap()
        };
        assert_eq!(
            place(&doc, ids[0], true),
            None,
            "above the first: the start"
        );
        assert_eq!(place(&doc, ids[3], true), Some(ids[1]), "above d: after b");
        assert_eq!(place(&doc, ids[3], false), Some(ids[3]));
        let after = place(&doc, ids[1], true);
        move_after(&mut doc, ids[2], after).unwrap();
        assert_eq!(order(&doc), ["a", "c", "b", "d"], "c dropped above b");
    }

    #[test]
    fn the_application_s_command_ids_are_unique() {
        let mut ids: Vec<String> = doc_commands().into_iter().map(|c| c.id).collect();
        ids.extend(app_commands().into_iter().map(|c| c.id));
        ids.extend(key_commands().map(|(c, _)| c.id));
        let count = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }
}
