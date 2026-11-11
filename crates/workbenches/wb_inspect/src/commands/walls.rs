//! Maps of a body on its own: wall thickness, an element as a solid,
//! clearance and overhangs.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, ParamKind,
    WorkbenchContext, WorkbenchRuntimeContext,
};

use crate::{
    CLEARANCE, ELEMENT_SOLID, InspectionWorkbench, OVERHANG, THICKNESS, add_clearance,
    add_overhang, add_thickness, clearance, element_solid, overhang, thickness,
};

pub(super) fn owns(id: &str) -> bool {
    matches!(id, THICKNESS | ELEMENT_SOLID | CLEARANCE | OVERHANG)
}

pub(super) fn configure(context: &mut WorkbenchContext) {
    context.register_command(
            CommandSpec::new(THICKNESS, "Map how thin or thick a body's walls are")
                .param("body", ParamKind::Id, "Any body with geometry: a scan or a solid")
                .optional("sphere", ParamKind::Bool, "The largest ball that fits, in place of rays")
                .optional("min_wall", ParamKind::Number, "Count the walls thinner than this, mm")
                .optional("max", ParamKind::Number, "Walls thicker than this read unmeasured; a fifth of the body's diagonal when left out")
                .returns("{thickness, name, measured, unmeasured, mean, min, max, range, thinner}")
                .note(
                    "Read inward from every vertex of the body's mesh, along its normal; a \
                     solid is read from its exact surfaces. Surfaces within 60° of edge-on to a \
                     ray are passed over. The map paints thin walls hot while the Inspection \
                     bench is active.",
                ),
        );
    context.register_command(
        CommandSpec::new(
            ELEMENT_SOLID,
            "The solid primitive a fitted element stands for",
        )
        .param(
            "element",
            ParamKind::Id,
            "A fitted plane, cylinder, cone, sphere or torus",
        )
        .optional(
            "thickness",
            ParamKind::Number,
            "A plane's slab, under its fitted patch, mm; 2",
        )
        .returns("{kind, placement, name}: what pc.design.primitive takes, in the document's frame")
        .read_only()
        .note(
            "A cylinder or cone covers the length its points cover along its axis, a \
                     plane becomes a slab over the patch its points cover, sunk under it \
                     (against its normal), and a sphere or torus is whole. Hand `kind` and \
                     `placement` to pc.design.primitive in a new body to model from it; the \
                     Element as body tool does both. A circle or line has no solid.",
        )
        .see_also("design.primitive")
        .example(
            "A scanned cylinder fitted and made a body again",
            r#"
                    local drum = pc.doc.new_body{}
                    pc.design.primitive{body = drum, variant = "cylinder",
                      kind = {Cylinder = {radius = 5, height = 10, angle_deg = 360}}}
                    assert(#pc.doc.rebuild() == 0)
                    local path = os.tmpname() .. ".stl"
                    pc.file.export{path = path, bodies = {drum}}
                    pc.doc.set_visible{id = drum, visible = false}
                    pc.file.import{path = path, scan = true}
                    pc.doc.rebuild()
                    os.remove(path)
                    local scan
                    for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
                    local fit = pc.inspect.grow{kind = "cylinder", scan = scan, at = {5, 0, 5}}
                    local s = pc.inspect.element_solid{element = fit.element}
                    local copy = pc.doc.new_body{name = s.name}
                    pc.design.primitive{body = copy, kind = s.kind, placement = s.placement}
                    assert(#pc.doc.rebuild() == 0)
                    local v = pc.doc.measure{body = copy}.volume
                    assert(math.abs(v - math.pi * 25 * 10) < 0.02 * math.pi * 25 * 10, v)
                    "#,
        ),
    );
    context.register_command(
            CommandSpec::new(CLEARANCE, "Map a part's room in what it fits into")
                .param("part", ParamKind::Id, "The part the map paints: a scan or a solid")
                .param("housing", ParamKind::Id, "What it fits into: a solid or a scan")
                .optional("min_gap", ParamKind::Number, "Count the points with less room than this, mm")
                .optional("reach", ParamKind::Number, "Points farther than this from the housing read unmeasured, mm; 5 when left out")
                .returns("{clearance, name, measured, unmeasured, interfering, deepest, narrowest, too_tight}")
                .note(
                    "The two are measured as they sit now: place the part in the housing first \
                     (an alignment, the Assembly's joints or a placement). Each of the part's \
                     points reads its distance to the housing's surface, positive across a gap \
                     and negative inside the housing's material. `interfering` is the share of \
                     measured points inside it and `deepest` how far, `narrowest` the smallest \
                     gap when nothing interferes, and `too_tight` the share with less room than \
                     `min_gap`. The map paints interference red while the Inspection bench is \
                     active.",
                )
                .example(
                    "A 10 mm cube in a 12 mm square pocket has 1 mm all round; moved 1.5 mm, \
                     it presses 0.5 mm into one wall.",
                    "local box = pc.doc.new_body{}\n\
                     pc.design.primitive{body = box, kind = {Box = {length = 30, width = 30, height = 10}}}\n\
                     pc.design.subtractive_primitive{body = box, kind = {Box = {length = 12, width = 12, height = 20}},\n\
                       placement = {origin = {9, 9, 2}, x_axis = {1, 0, 0}, z_axis = {0, 0, 1}}}\n\
                     local part = pc.doc.new_body{}\n\
                     pc.design.primitive{body = part, kind = {Box = {length = 10, width = 10, height = 10}}}\n\
                     assert(#pc.doc.rebuild() == 0)\n\
                     pc.doc.set_body{body = part, translation = {10, 10, 4}}\n\
                     local fit = pc.inspect.clearance{part = part, housing = box, min_gap = 0.5}\n\
                     assert(fit.interfering == 0 and math.abs(fit.narrowest - 1) < 1e-6, fit.narrowest)\n\
                     pc.doc.set_body{body = part, translation = {11.5, 10, 4}}\n\
                     local tight = pc.inspect.clearance{part = part, housing = box}\n\
                     assert(tight.interfering > 0 and math.abs(tight.deepest - 0.5) < 1e-6, tight.deepest)",
                ),
        );
    context.register_command(
            CommandSpec::new(OVERHANG, "Map a body's overhangs as it would print")
                .param("body", ParamKind::Id, "Any body with geometry, standing as it would print (+Z up)")
                .optional("limit", ParamKind::Number, "The steepest overhang printed without support, degrees past vertical; 45 when left out")
                .returns("{overhang, name, limit, share, steepest}")
                .note(
                    "Each point of the mesh facing down reads how far it leans out past \
                     vertical: 0 a wall, 90 a ceiling. Surface facing up, and the face the body \
                     stands on at its lowest point, read unmeasured. `share` is the part of the \
                     body's area past the limit, which needs support; `steepest` the worst \
                     overhang, in degrees. The map paints while the Inspection bench is active.",
                )
                .example(
                    "A box standing on the bed needs no support; on its edge, half of it does.",
                    "local body = pc.doc.new_body{}\n\
                     pc.design.primitive{body = body}\n\
                     assert(#pc.doc.rebuild() == 0)\n\
                     local o = pc.inspect.overhang{body = body}\n\
                     assert(o.share == 0 and o.limit == 45, o.share)\n\
                     local q = math.pi / 8\n\
                     pc.doc.set_body{body = body, rotation = {x = math.sin(q), y = 0, z = 0, w = math.cos(q)}}\n\
                     local tilted = pc.inspect.overhang{body = body, limit = 40}\n\
                     assert(math.abs(tilted.steepest - 45) < 1e-3, tilted.steepest)",
                ),
        );
}

pub(super) fn run(
    bench: &InspectionWorkbench,
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    match id {
        THICKNESS => {
            let t = thickness::WallThickness {
                name: String::new(),
                body: BodyId(a.id("body")?),
                sphere: a.opt_bool("sphere")? == Some(true),
                min_wall: a.opt_number("min_wall")?,
                max: a.opt_number("max")?,
            };
            add_thickness(ctx, &bench.tab.maps, t).map_err(CommandError::failed)
        }
        ELEMENT_SOLID => {
            let id = core_document::FeatureId(a.id("element")?);
            let thickness = a.opt_number("thickness")?.unwrap_or(2.0);
            element_solid(ctx.document, id, thickness).map_err(CommandError::failed)
        }
        CLEARANCE => {
            let c = clearance::Clearance {
                name: String::new(),
                part: BodyId(a.id("part")?),
                housing: BodyId(a.id("housing")?),
                min_gap: a.opt_number("min_gap")?,
                reach: a.opt_number("reach")?.unwrap_or(clearance::DEFAULT_REACH),
            };
            add_clearance(ctx, &bench.tab.maps, c).map_err(CommandError::failed)
        }
        OVERHANG => {
            let o = overhang::Overhang {
                name: String::new(),
                body: BodyId(a.id("body")?),
                limit_deg: a.opt_number("limit")?.unwrap_or(45.0),
            };
            add_overhang(ctx, &bench.tab.maps, o).map_err(CommandError::failed)
        }
        _ => Err(CommandError::Unknown(id.to_string())),
    }
}
