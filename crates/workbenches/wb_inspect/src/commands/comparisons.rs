//! What a comparison reads: hot spots, missing and excess material, and its
//! distances written to a file.

use core_document::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, ParamKind,
    WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};

use crate::{
    Comparison, HOT_SPOTS, InspectionWorkbench, MAP_EXPORT, SPOT_LEAST, VOLUME_ERRORS, compare,
    comparison_values, export, hotspots, longest_side, volume,
};

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<InspectionWorkbench>) {
    t.add_on(
        CommandSpec::new(
            HOT_SPOTS,
            "Where a comparison is out of tolerance, as patches",
        )
        .param("comparison", ParamKind::Id, "The comparison")
        .optional(
            "tolerance",
            ParamKind::Number,
            "± in mm; the comparison's own when left out",
        )
        .optional(
            "least",
            ParamKind::Integer,
            "Leave out patches of fewer vertices; 3",
        )
        .returns(
            "a list, largest first, of {side, count, area, centre, min, max, mean, \
                     extreme, at, kind}",
        )
        .read_only(),
        hot_spots,
    );
    t.add_on(
        CommandSpec::new(VOLUME_ERRORS, "Missing and excess material of a comparison")
            .param(
                "comparison",
                ParamKind::Id,
                "The comparison; its reference a solid",
            )
            .optional(
                "cell",
                ParamKind::Number,
                "The grid's cell in mm; a two-hundredth of the scan's longest side",
            )
            .optional("off", ParamKind::Bool, "Stop reading them; false")
            .returns(
                "{cell, regions}: a list, largest first, of {side, form, volume, depth, \
                     centre, min, max, unsure}",
            ),
        volume_errors,
    );
    t.add_on(
        CommandSpec::new(MAP_EXPORT, "Write a comparison's distances to a file")
            .param("comparison", ParamKind::Id, "The comparison")
            .param("path", ParamKind::String, "Where to write")
            .optional(
                "format",
                ParamKind::String,
                "csv, ply (binary, a deviation per vertex) or npy; from the path's \
                     extension when left out",
            )
            .returns("{path, points}"),
        map_export,
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(
        &PROSE,
        "comparisons.md",
        include_str!("comparisons.md"),
        &[],
    )
}

fn hot_spots(
    bench: &mut InspectionWorkbench,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let id = core_document::FeatureId(a.id("comparison")?);
    let (c, inputs, values) =
        comparison_values(ctx, &bench.tab.maps, id).map_err(CommandError::failed)?;
    let tolerance = a
        .opt_number("tolerance")?
        .or(c.tolerance)
        .ok_or_else(|| CommandError::bad("tolerance", "is needed: the comparison has none"))?;
    let least = a
        .opt_number("least")?
        .map_or(SPOT_LEAST, |n| n.max(1.0) as usize);
    Ok(json!(hotspots::find(
        inputs.scan_mesh(),
        &values,
        tolerance,
        least
    )))
}

fn volume_errors(
    bench: &mut InspectionWorkbench,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let id = core_document::FeatureId(a.id("comparison")?);
    let mut c = ctx
        .document
        .feature_values(id)
        .filter(|_| {
            ctx.document
                .feature_tree()
                .get_node(id)
                .is_some_and(|n| n.workbench_id.as_str() == compare::KIND)
        })
        .and_then(|d| Comparison::from_json(d).ok())
        .ok_or_else(|| CommandError::bad("comparison", "is not a comparison"))?;
    if a.opt_bool("off")?.unwrap_or(false) {
        c.volume_cell = None;
        ctx.document
            .update_feature_data(id, c.to_json())
            .map_err(|e| CommandError::failed(e.to_string()))?;
        return Ok(json!({ "cell": Value::Null, "regions": [] }));
    }
    let inputs = compare::inputs(ctx.document, &c).map_err(CommandError::failed)?;
    let cell = match a.opt_number("cell")? {
        Some(cell) if cell.is_finite() && cell > 0.0 => cell,
        Some(_) => return Err(CommandError::bad("cell", "is a positive distance")),
        None => volume::default_cell(longest_side(inputs.scan_mesh())),
    };
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("no kernel to measure with"))?;
    let regions = volume::measure(kernel, &inputs, cell).map_err(CommandError::failed)?;
    let answer = volume::answer(&regions, cell);
    if c.volume_cell != Some(cell) {
        c.volume_cell = Some(cell);
        ctx.document
            .update_feature_data(id, c.to_json())
            .map_err(|e| CommandError::failed(e.to_string()))?;
    }
    let key = compare::inputs(ctx.document, &c)
        .map_err(CommandError::failed)?
        .key;
    bench
        .tab
        .maps
        .store_volumes(id, key, cell, std::sync::Arc::new(regions));
    Ok(answer)
}

fn map_export(
    bench: &mut InspectionWorkbench,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let id = core_document::FeatureId(a.id("comparison")?);
    let path = std::path::PathBuf::from(a.string("path")?);
    let format = match a.opt_string("format")? {
        Some(f) => f.to_lowercase(),
        None => path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("csv")
            .to_lowercase(),
    };
    let (_, inputs, values) =
        comparison_values(ctx, &bench.tab.maps, id).map_err(CommandError::failed)?;
    let bytes = export::write(inputs.scan_mesh(), &values, &format)
        .map_err(|why| CommandError::bad("format", why))?;
    std::fs::write(&path, bytes)
        .map_err(|e| CommandError::failed(format!("{}: {e}", path.display())))?;
    Ok(json!({"path": path.display().to_string(), "points": values.len()}))
}
