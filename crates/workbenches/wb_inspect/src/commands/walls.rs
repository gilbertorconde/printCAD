//! Maps of a body on its own: wall thickness, an element as a solid,
//! clearance and overhangs.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, ParamKind,
    WorkbenchRuntimeContext,
};

use crate::{
    CLEARANCE, ELEMENT_SOLID, InspectionWorkbench, OVERHANG, THICKNESS, add_clearance,
    add_overhang, add_thickness, clearance, element_solid, overhang, thickness,
};

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<InspectionWorkbench>) {
    t.add_on(
            CommandSpec::new(THICKNESS, "Map how thin or thick a body's walls are")
                .param("body", ParamKind::Id, "Any body with geometry: a scan or a solid")
                .optional("sphere", ParamKind::Bool, "The largest ball that fits, in place of rays")
                .optional("min_wall", ParamKind::Number, "Count the walls thinner than this, mm")
                .optional("max", ParamKind::Number, "Walls thicker than this read unmeasured; a fifth of the body's diagonal when left out")
                .returns("{thickness, name, measured, unmeasured, mean, min, max, range, thinner}"),
        thickness_command,
    );
    t.add(
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
        .read_only(),
        element_solid_command,
    );
    t.add_on(
            CommandSpec::new(CLEARANCE, "Map a part's room in what it fits into")
                .param("part", ParamKind::Id, "The part the map paints: a scan or a solid")
                .param("housing", ParamKind::Id, "What it fits into: a solid or a scan")
                .optional("min_gap", ParamKind::Number, "Count the points with less room than this, mm")
                .optional("reach", ParamKind::Number, "Points farther than this from the housing read unmeasured, mm; 5 when left out")
                .returns("{clearance, name, measured, unmeasured, interfering, deepest, narrowest, too_tight}"),
        clearance_command,
    );
    t.add_on(
            CommandSpec::new(OVERHANG, "Map a body's overhangs as it would print")
                .param("body", ParamKind::Id, "Any body with geometry, standing as it would print (+Z up)")
                .optional("limit", ParamKind::Number, "The steepest overhang printed without support, degrees past vertical; 45 when left out")
                .returns("{overhang, name, limit, share, steepest}"),
        overhang_command,
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "walls.md", include_str!("walls.md"), &[])
}

fn thickness_command(
    bench: &mut InspectionWorkbench,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let t = thickness::WallThickness {
        name: String::new(),
        body: BodyId(a.id("body")?),
        sphere: a.opt_bool("sphere")? == Some(true),
        min_wall: a.opt_number("min_wall")?,
        max: a.opt_number("max")?,
    };
    add_thickness(ctx, &bench.tab.maps, t).map_err(CommandError::failed)
}

fn element_solid_command(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let id = core_document::FeatureId(a.id("element")?);
    let thickness = a.opt_number("thickness")?.unwrap_or(2.0);
    element_solid(ctx.document, id, thickness).map_err(CommandError::failed)
}

fn clearance_command(
    bench: &mut InspectionWorkbench,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let c = clearance::Clearance {
        name: String::new(),
        part: BodyId(a.id("part")?),
        housing: BodyId(a.id("housing")?),
        min_gap: a.opt_number("min_gap")?,
        reach: a.opt_number("reach")?.unwrap_or(clearance::DEFAULT_REACH),
    };
    add_clearance(ctx, &bench.tab.maps, c).map_err(CommandError::failed)
}

fn overhang_command(
    bench: &mut InspectionWorkbench,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let o = overhang::Overhang {
        name: String::new(),
        body: BodyId(a.id("body")?),
        limit_deg: a.opt_number("limit")?.unwrap_or(45.0),
    };
    add_overhang(ctx, &bench.tab.maps, o).map_err(CommandError::failed)
}
