//! What a comparison reads: hot spots, missing and excess material, and its
//! distances written to a file.

use core_document::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, ParamKind, WorkbenchContext,
    WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};

use crate::{
    Comparison, HOT_SPOTS, InspectionWorkbench, MAP_EXPORT, SPOT_LEAST, VOLUME_ERRORS, compare,
    comparison_values, export, hotspots, longest_side, volume,
};

pub(super) fn owns(id: &str) -> bool {
    matches!(id, HOT_SPOTS | VOLUME_ERRORS | MAP_EXPORT)
}

pub(super) fn configure(context: &mut WorkbenchContext) {
    context.register_command(
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
        .note(
            "Vertices beyond the tolerance are joined with neighbours on the same side \
                     across the mesh's edges. `side` is 1 outside the reference and -1 inside; \
                     `kind` is `feature` for a patch no more than 40 times as wide as it is deep, \
                     else `texture`. Positions are in the scan's frame.",
        )
        .read_only(),
    );
    context.register_command(
            CommandSpec::new(VOLUME_ERRORS, "Missing and excess material of a comparison")
                .param("comparison", ParamKind::Id, "The comparison; its reference a solid")
                .optional(
                    "cell",
                    ParamKind::Number,
                    "The grid's cell in mm; a two-hundredth of the scan's longest side",
                )
                .optional("off", ParamKind::Bool, "Stop reading them; false")
                .returns(
                    "{cell, regions}: a list, largest first, of {side, form, volume, depth, \
                     centre, min, max, unsure}",
                )
                .note(
                    "The scan is read as a solid on a grid of cubic cells and set against the \
                     reference. `side` is `missing` where the reference has material the scan \
                     lacks and `excess` where the scan has more; `form` is `layer` for a face \
                     standing off, `lump` for a boss or a pocket. `volume` is in mm³, `depth` in \
                     mm, how far apart the surfaces stand across the region. Holes in the scan \
                     read as the reference; a region on one, or shallower than two cells, is \
                     `unsure`, and one shallower than a cell is not reported. Positions are in \
                     the reference's frame. The comparison keeps the cell, and the largest \
                     regions are labelled on the part while the Inspection bench is active.",
                )
                .see_also(HOT_SPOTS)
                .example(
                    "A print two millimetres short of its part reads a missing layer",
                    r#"
                    local part = pc.doc.new_body{}
                    pc.design.primitive{body = part, kind = {Box = {length = 20, width = 20, height = 12}}}
                    local short = pc.doc.new_body{}
                    pc.design.primitive{body = short, kind = {Box = {length = 20, width = 20, height = 10}}}
                    assert(#pc.doc.rebuild() == 0)
                    local path = os.tmpname() .. ".stl"
                    pc.file.export{path = path, bodies = {short}}
                    pc.doc.set_visible{id = short, visible = false}
                    pc.file.import{path = path, scan = true}
                    pc.doc.rebuild()
                    os.remove(path)
                    local scan
                    for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
                    local c = pc.inspect.compare{scan = scan, reference = part}
                    local v = pc.inspect.volume_errors{comparison = c.comparison, cell = 0.5}
                    local r = v.regions[1]
                    assert(r.side == "missing" and r.form == "layer", r.side)
                    assert(math.abs(r.volume - 800) < 80 and math.abs(r.depth - 2) < 0.3, r.volume)
                    "#,
                ),
        );
    context.register_command(
        CommandSpec::new(MAP_EXPORT, "Write a comparison's distances to a file")
            .param("comparison", ParamKind::Id, "The comparison")
            .param("path", ParamKind::String, "Where to write")
            .optional(
                "format",
                ParamKind::String,
                "csv, ply (binary, a deviation per vertex) or npy; from the path's \
                     extension when left out",
            )
            .returns("{path, points}")
            .note(
                "Points are in the scan's own frame; an unmeasured point's distance is \
                       NaN, blank in CSV.",
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
        HOT_SPOTS => {
            let id = core_document::FeatureId(a.id("comparison")?);
            let (c, inputs, values) =
                comparison_values(ctx, &bench.tab.maps, id).map_err(CommandError::failed)?;
            let tolerance = a.opt_number("tolerance")?.or(c.tolerance).ok_or_else(|| {
                CommandError::bad("tolerance", "is needed: the comparison has none")
            })?;
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
        VOLUME_ERRORS => {
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
        MAP_EXPORT => {
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
        _ => Err(CommandError::Unknown(id.to_string())),
    }
}
