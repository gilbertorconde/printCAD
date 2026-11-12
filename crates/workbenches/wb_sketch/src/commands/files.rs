//! DXF in, and DXF, SVG and CSV out.

use core_document::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, FeatureId,
    ParamKind, WorkbenchRuntimeContext,
};
use serde_json::json;

use crate::SketchWorkbench;
use crate::drawing::{self, LAYER_CONSTRUCTION, LAYER_EDGES, LAYER_ELEMENTS, Lines};
use crate::feature::SketchFeature;
use crate::sketch::Sketch;

use super::{add_sketch, load, pictures, placing, sketch};

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<SketchWorkbench>) {
    t.add(
        placing(CommandSpec::new(
            "sketch.import_dxf",
            "Make a sketch of a DXF drawing: its lines, arcs, circles, ellipses and polylines \
             as sketch curves, splines as lines through points on them, hidden ones as \
             construction, ends that meet sharing one point",
        ))
        .param("path", ParamKind::String, "The DXF file")
        .optional(
            "scale",
            ParamKind::Number,
            "Millimetres per drawing unit; the drawing's own unit when left out, else 1",
        )
        .returns("the sketch's id")
        .agent_always_asks(),
        |args, ctx| import_dxf(&Args(args), ctx),
    );
    t.add_on(
        sketch(CommandSpec::new(
            "sketch.export_dxf",
            "Sketch as layered DXF",
        ))
        .param("path", ParamKind::String, "The file to write")
        .optional("units", ParamKind::String, "mm or inch (mm)")
        .returns("{path, units, curves, construction, edges, notes}")
        .agent_always_asks(),
        export_dxf,
    );
    t.add_on(
        sketch(CommandSpec::new("sketch.export_svg", "Sketch as SVG"))
            .param("path", ParamKind::String, "The file to write")
            .returns("{path, curves, construction}")
            .agent_always_asks(),
        export_svg,
    );
    t.add_on(
        sketch(CommandSpec::new(
            "sketch.export_csv",
            "Elements and dimensions as CSV",
        ))
        .returns("{csv, rows}")
        .read_only(),
        export_csv,
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "files.md", include_str!("files.md"), &[])
}

/// A sketch of the DXF drawing at `path`, placed as `sketch.new` places
/// one and named after the file unless `name` says otherwise.
fn import_dxf(a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let path = std::path::Path::new(a.string("path")?);
    let given_scale = a.opt_number("scale")?;
    if given_scale.is_some_and(|s| !(s > 0.0 && s.is_finite())) {
        return Err(CommandError::bad("scale", "must be more than zero"));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| CommandError::bad("path", format!("could not be read: {e}")))?;
    // DXF text is often Latin-1 in its strings; the geometry is ASCII.
    let text = String::from_utf8_lossy(&bytes);
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("there is no kernel to read the drawing"))?;
    let drawing = kernel
        .read_dxf(&text)
        .map_err(|e| CommandError::failed(e.to_string()))?;
    let name = match a.opt_string("name")? {
        Some(name) => name.to_string(),
        None => path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| crate::SketchWorkbench::next_sketch_name(ctx.document)),
    };
    // The scale given, else the drawing's own unit, else millimetres.
    let scale = given_scale.or(drawing.unit_mm).unwrap_or(1.0);
    let mut sketch = Sketch::new(name);
    let added = crate::dxf::add_drawing(&mut sketch, &drawing, scale);
    if added.curves() == 0 {
        return Err(CommandError::failed(
            "the drawing has no curves to bring in",
        ));
    }
    crate::solver::solve(&mut sketch);
    let id = add_sketch(a, ctx, sketch, None)?;
    let mut parts: Vec<String> = [
        (added.lines, "lines"),
        (added.arcs, "arcs"),
        (added.circles, "circles"),
        (added.ellipses, "ellipses"),
    ]
    .iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}"))
    .collect();
    if added.construction > 0 {
        parts.push(format!("{} of them construction", added.construction));
    }
    if added.splines > 0 {
        parts.push(format!("{} spline(s) as lines through them", added.splines));
    }
    ctx.log_info(format!(
        "Imported {} at {scale} mm a unit: {}",
        path.display(),
        parts.join(", ")
    ));
    Ok(json!(id.0.to_string()))
}

fn export_csv(
    bench: &mut SketchWorkbench,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let feature = load(ctx, FeatureId(a.id("sketch")?))?;
    let sketch = &feature.sketch;
    let pictures = pictures(sketch, ctx, &bench.tab.picture_edges);
    let trace = drawing::traceability(sketch, &feature.plane, &pictures, "mm");
    let (csv, rows) = drawing::csv(sketch, &trace);
    Ok(json!({"csv": csv, "rows": rows}))
}

/// The sketch an export draws, the file it writes and the sketch's curves
/// as lines (drawn, then construction), refused when it has none.
fn drawing_of<'a>(
    a: &'a Args,
    ctx: &WorkbenchRuntimeContext,
) -> Result<(SketchFeature, &'a str, Lines, Lines), CommandError> {
    let feature = load(ctx, FeatureId(a.id("sketch")?))?;
    let path = a.string("path")?;
    let (drawn, guides) = drawing::polylines(&feature.sketch);
    if drawn.is_empty() && guides.is_empty() {
        return Err(CommandError::failed("the sketch has no curves to write"));
    }
    Ok((feature, path, drawn, guides))
}

/// Write `text` to `path`.
fn write_to(path: &str, text: String) -> Result<(), CommandError> {
    std::fs::write(path, text).map_err(|e| CommandError::failed(format!("{path}: {e}")))
}

fn export_svg(
    bench: &mut SketchWorkbench,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let (feature, path, drawn, guides) = drawing_of(&a, ctx)?;
    let sketch = &feature.sketch;
    let pictures = pictures(sketch, ctx, &bench.tab.picture_edges);
    let trace = drawing::traceability(sketch, &feature.plane, &pictures, "mm");
    write_to(path, drawing::svg(&drawn, &guides, &trace))?;
    Ok(json!({"path": path, "curves": drawn.len(), "construction": guides.len()}))
}

fn export_dxf(
    bench: &mut SketchWorkbench,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let (feature, path, _, _) = drawing_of(&a, ctx)?;
    let sketch = &feature.sketch;
    let inches = match a.opt_string("units")? {
        None | Some("mm") => false,
        Some("inch") => true,
        Some(_) => return Err(CommandError::bad("units", "is mm or inch")),
    };
    let unit = if inches { "inch" } else { "mm" };
    let pictures = pictures(sketch, ctx, &bench.tab.picture_edges);
    let trace = drawing::traceability(sketch, &feature.plane, &pictures, unit);
    let sheet = drawing::sheet(sketch, &pictures, &trace, inches);
    let text = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("there is no kernel to write with"))?
        .write_dxf_drawing(&sheet)
        .map_err(|e| CommandError::failed(e.to_string()))?;
    write_to(path, text)?;
    let on = |layer: &str| sheet.curves.iter().filter(|c| c.layer == layer).count();
    Ok(json!({
        "path": path,
        "units": unit,
        "curves": on(LAYER_ELEMENTS),
        "construction": on(LAYER_CONSTRUCTION),
        "edges": on(LAYER_EDGES),
        "notes": sheet.texts.len(),
    }))
}
