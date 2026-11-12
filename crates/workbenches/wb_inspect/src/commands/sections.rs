//! Cuts of a scan: sections by a plane or a stack of planes, and the
//! shadow outline extruded into a block.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, ParamKind,
    WorkbenchRuntimeContext,
};

use crate::{
    InspectionWorkbench, SECTION, SHADOW, numbered, section, section_answer, section_plane, shadow,
    shadow_answer,
};

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<InspectionWorkbench>) {
    t.add(
            CommandSpec::new(SECTION, "Cut a scan by a plane, or a stack of planes")
                .optional(
                    "element",
                    ParamKind::Id,
                    "Cut across this plane, axis or circle",
                )
                .optional("scan", ParamKind::Id, "Without an element: the scan to cut")
                .optional(
                    "plane",
                    ParamKind::String,
                    "Without an element: xy, xz or yz through the scan's middle; xy",
                )
                .optional(
                    "offset",
                    ParamKind::Number,
                    "Move the first cut along the normal, mm; 0",
                )
                .optional("count", ParamKind::Number, "How many cuts; 1")
                .optional("step", ParamKind::Number, "Between cuts, mm; 5")
                .optional(
                    "through",
                    ParamKind::Bool,
                    "A half-section containing the element's axis",
                )
                .optional(
                    "angle",
                    ParamKind::Number,
                    "Of a half-section about the axis, degrees; 0",
                )
                .returns("{section, name, cuts = {{offset, chains, closed, length, area}}, tracks = {{cuts, from, to, tilt, draft, radius = {first, last}, roundness}}}"),
        section_command,
    );
    t.add(
        CommandSpec::new(
            SHADOW,
            "Outline a scan's shadow clear of it, and stand it up as a block",
        )
        .optional(
            "element",
            ParamKind::Id,
            "Look along this plane's normal or this axis",
        )
        .optional("scan", ParamKind::Id, "Without an element: the scan")
        .optional(
            "plane",
            ParamKind::String,
            "Without an element: look along the normal of xy, xz or yz; xy",
        )
        .optional(
            "play",
            ParamKind::Number,
            "The least clearance, mm; 1, 0 exact, negative a press fit",
        )
        .optional(
            "tolerance",
            ParamKind::Number,
            "Half the band the outline keeps to, mm; 0.1",
        )
        .optional(
            "round_below",
            ParamKind::Number,
            "Round corners turning less than this, degrees; 30",
        )
        .optional(
            "fill_below",
            ParamKind::Number,
            "Fill holes and notches no wider than this, mm, rounding inner corners to \
                 half of it; 0, none",
        )
        .optional(
            "block",
            ParamKind::Bool,
            "Stand the outline up as a block; true",
        )
        .optional(
            "height",
            ParamKind::Number,
            "The block's height, mm; the part's thickness",
        )
        .returns("{shadow, body, name, loops, area, band = {least, largest}, depth, error}"),
        shadow_command,
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "sections.md", include_str!("sections.md"), &[])
}

fn section_command(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let element = a.opt_id("element")?.map(core_document::FeatureId);
    let (scan, origin, normal, half) = section_plane(
        ctx.document,
        element,
        a.opt_id("scan")?.map(BodyId),
        a.opt_string("plane")?.unwrap_or("xy"),
        a.opt_bool("through")? == Some(true),
        a.opt_number("angle")?.unwrap_or(0.0),
    )?;
    let count = a.opt_number("count")?.unwrap_or(1.0);
    if !(1.0..=200.0).contains(&count) {
        return Err(CommandError::bad("count", "is 1 to 200"));
    }
    let s = section::Section {
        name: numbered(ctx.document, "Section"),
        scan,
        origin,
        normal,
        offset: a.opt_number("offset")?.unwrap_or(0.0),
        count: count.round() as u32,
        step: a.opt_number("step")?.unwrap_or(5.0),
        half,
    };
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("no kernel to cut with"))?;
    let (mesh, _) = ctx
        .document
        .local_geometry(scan)
        .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
    let cuts = section::cut(kernel, &mesh, &s).map_err(CommandError::failed)?;
    let id = ctx
        .document
        .add_feature_in_body(s.clone(), s.name.clone(), Some(scan))
        .map_err(|e| CommandError::failed(e.to_string()))?;
    Ok(section_answer(id, &s, &cuts))
}

fn shadow_command(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let (scan, _, normal, _) = section_plane(
        ctx.document,
        a.opt_id("element")?.map(core_document::FeatureId),
        a.opt_id("scan")?.map(BodyId),
        a.opt_string("plane")?.unwrap_or("xy"),
        false,
        0.0,
    )?;
    let (mesh, _) = ctx
        .document
        .local_geometry(scan)
        .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
    let (origin, x_axis) = shadow::foot(&mesh, normal)
        .ok_or_else(|| CommandError::failed("the scan has no points"))?;
    let tolerance = a.opt_number("tolerance")?.unwrap_or(0.1);
    if tolerance <= 0.0 {
        return Err(CommandError::bad("tolerance", "is above zero"));
    }
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("no kernel to outline with"))?;
    let s = shadow::Shadow {
        name: numbered(ctx.document, "Shadow"),
        scan,
        origin,
        normal,
        x_axis,
        play: a.opt_number("play")?.unwrap_or(1.0),
        tolerance,
        round_below_deg: a.opt_number("round_below")?.unwrap_or(30.0),
        fill_below: a.opt_number("fill_below")?.unwrap_or(0.0).max(0.0),
        block: a.opt_bool("block")?.unwrap_or(true),
        height: a.opt_number("height")?,
        made: None,
    };
    let made = shadow::make(ctx.document, kernel, &s);
    if let Some(why) = &made.error {
        return Err(CommandError::failed(why.clone()));
    }
    let body = ctx.document.create_body(Some(s.name.clone()));
    let id = ctx
        .document
        .add_feature_in_body(s.clone(), s.name.clone(), Some(body))
        .map_err(|e| CommandError::failed(e.to_string()))?;
    Ok(shadow_answer(id, body, &s, &made))
}
