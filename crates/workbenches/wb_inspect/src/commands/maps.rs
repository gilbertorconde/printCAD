//! Readings shown on a mapped scan: pins and the map against one element.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, ParamKind,
    WorkbenchRuntimeContext,
};
use serde_json::json;

use crate::{
    ELEMENT_MAP, InspectionWorkbench, PIN, READING_REACH, element_at, element_map, maps, numbered,
    pins,
};

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<InspectionWorkbench>) {
    t.add_on(
        CommandSpec::new(PIN, "Pin a map's reading on the part")
            .param("scan", ParamKind::Id, "The scan the map is shown on")
            .param(
                "at",
                ParamKind::Any,
                "{x, y, z}: where, in the scan's own frame",
            )
            .returns("{pin, name, value}"),
        pin_command,
    );
    t.add_on(
        CommandSpec::new(
            ELEMENT_MAP,
            "Map a scan's distance from one of its elements",
        )
        .param(
            "element",
            ParamKind::Id,
            "A plane, cylinder, cone or sphere",
        )
        .optional(
            "tolerance",
            ParamKind::Number,
            "± in mm, counted in the legend",
        )
        .optional("flip", ParamKind::Bool, "Count the other side positive")
        .optional(
            "facing",
            ParamKind::Number,
            "Leave points turned further than this many degrees from the shape unmeasured; 60",
        )
        .returns("{map, name, measured, mean, rms, min, max}"),
        element_map_command,
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "maps.md", include_str!("maps.md"), &[])
}

fn pin_command(
    bench: &mut InspectionWorkbench,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let scan = BodyId(a.id("scan")?);
    let at = a.vec3("at")?;
    let (map, values) = bench
        .shown_map(ctx.document, scan)
        .ok_or_else(|| CommandError::failed("no map is shown on that scan"))?;
    let (mesh, _) = ctx
        .document
        .local_geometry(scan)
        .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
    let value = pins::read(&mesh, &values, at, READING_REACH);
    let pin = pins::Pin {
        name: numbered(ctx.document, "Pin"),
        scan,
        map,
        at,
    };
    let id = ctx
        .document
        .add_feature_in_body(pin.clone(), pin.name.clone(), Some(scan))
        .map_err(|e| CommandError::failed(e.to_string()))?;
    Ok(json!({"pin": id.0.to_string(), "name": pin.name, "value": value}))
}

fn element_map_command(
    bench: &mut InspectionWorkbench,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let element = core_document::FeatureId(a.id("element")?);
    let e = element_at(ctx.document, element)
        .ok_or_else(|| CommandError::bad("element", "is not an element"))?;
    let m = element_map::ElementMap {
        name: String::new(),
        scan: e.scan,
        element,
        facing_limit_deg: a.opt_number("facing")?.unwrap_or(60.0),
        flip: a.opt_bool("flip")? == Some(true),
        tolerance: a.opt_number("tolerance")?,
    };
    let (id, m, values) = maps::add(ctx, &bench.tab.maps, m).map_err(CommandError::failed)?;
    let wide: Vec<f64> = values.iter().map(|v| f64::from(*v)).collect();
    let s = metrology::stats::summary(&wide);
    Ok(json!({
        "map": id.0.to_string(),
        "name": m.name,
        "measured": s.map_or(0, |s| s.count),
        "mean": s.map(|s| s.mean),
        "rms": s.map(|s| s.rms),
        "min": s.map(|s| s.min),
        "max": s.map(|s| s.max),
    }))
}
