//! DXF in, and DXF, SVG and CSV out.

use core_document::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, FeatureId, ParamKind,
    WorkbenchRuntimeContext,
};
use serde_json::json;

use crate::sketch::Sketch;

use super::{add_sketch, load, pictures, placing, sketch};

/// Whether `id` is one of these commands.
pub(super) fn owns(id: &str) -> bool {
    matches!(
        id,
        "sketch.import_dxf" | "sketch.export_dxf" | "sketch.export_svg" | "sketch.export_csv"
    )
}

/// These commands' specs.
pub(super) fn configure(specs: &mut Vec<CommandSpec>) {
    specs.push(
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
        .agent_always_asks()
        .note(
            "Left out, `scale` follows the drawing's own unit: a drawing in inches comes in \
             at 25.4 mm a unit. A drawing that names no unit comes in at 1 mm a unit.",
        )
        .note(
            "It is placed as `sketch.new` places a sketch: without `body` it goes in the \
             selected body, else a new one. It is named after the file unless `name` says \
             otherwise.",
        )
        .note(
            "A file that cannot be read or parsed is refused, and so is a drawing with no \
             curves.",
        )
        .see_also("sketch.new")
        .see_also("sketch.repair"),
    );
    specs.push(
        sketch(CommandSpec::new(
            "sketch.export_dxf",
            "Sketch as layered DXF",
        ))
        .param("path", ParamKind::String, "The file to write")
        .optional("units", ParamKind::String, "mm or inch (mm)")
        .returns("{path, units, curves, construction, edges, notes}")
        .agent_always_asks()
        .note(
            "At true scale, R2000, the unit declared. Lines, circles and arcs are written as themselves, \
             other curves as polylines within 0.01 mm, on layer ELEMENTS, construction and \
             projected guides dashed on CONSTRUCTION. The pictures' edges, thinned to \
             0.01 mm, go dashed on PICTURE_EDGES; text blocks as TEXT on NOTES, with a \
             traceability line under the drawing: the scale, each picture's mm per pixel \
             and where it came from, and the sketch's frame. Points are left out.",
        )
        .see_also("sketch.export_csv")
        .see_also("sketch.import_dxf")
        .example(
            "A circle in inches, read back",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 25.4}
            local path = os.tmpname()
            local out = pc.sketch.export_dxf{sketch = s, path = path, units = "inch"}
            assert(out.curves == 1 and out.notes == 1, out.notes)
            local file = io.open(path, "r")
            local text = file:read("a")
            file:close()
            os.remove(path)
            assert(text:find("$INSUNITS\n70\n1\n", 1, true), "declared in inches")
            assert(text:find("1:1 in inch", 1, true), "the traceability note")
            "#,
        ),
    );
    specs.push(
        sketch(CommandSpec::new(
            "sketch.export_svg",
            "Sketch as SVG",
        ))
        .param("path", ParamKind::String, "The file to write")
        .returns("{path, curves, construction}")
        .agent_always_asks()
        .note(
            "At true scale, one unit a millimetre, every curve a polyline within 0.01 mm of it; \
             construction and projected guides dashed. Points are left out; the \
             traceability line is written under the drawing.",
        ),
    );
    specs.push(
        sketch(CommandSpec::new(
            "sketch.export_csv",
            "Elements and dimensions as CSV",
        ))
        .returns("{csv, rows}")
        .read_only()
        .note(
            "A `#` traceability line (scale, its source, frame), a header, then a row per \
             element (kind, id, drawn or construction, its points, radius and length) and \
             per dimension (kind, id, name, driving or reference, value and unit). Lengths \
             in mm, angles in degrees.",
        )
        .see_also("sketch.export_dxf")
        .example(
            "A line's length",
            r##"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 30, y2 = 40}
            local out = pc.sketch.export_csv{sketch = s}
            assert(out.csv:find(",50,,mm", 1, true), out.csv)
            assert(out.csv:sub(1, 2) == "# ", "the traceability line first")
            "##,
        ),
    );
}

/// Run command `id`, one of these.
pub(super) fn run(
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    if id == "sketch.import_dxf" {
        return import_dxf(&a, ctx);
    }
    Err(CommandError::Unknown(id.to_string()))
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

/// `sketch.export_dxf`, `sketch.export_svg` and `sketch.export_csv`.
pub(crate) fn export(
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
    edges: &crate::picture_edges::Edges,
) -> CommandResult {
    use crate::drawing::{self, LAYER_CONSTRUCTION, LAYER_EDGES, LAYER_ELEMENTS};
    let a = Args(args);
    let feature = load(ctx, FeatureId(a.id("sketch")?))?;
    let sketch = &feature.sketch;
    if id == "sketch.export_csv" {
        let trace =
            drawing::traceability(sketch, &feature.plane, &pictures(sketch, ctx, edges), "mm");
        let (csv, rows) = drawing::csv(sketch, &trace);
        return Ok(json!({"csv": csv, "rows": rows}));
    }
    let path = a.string("path")?;
    let (drawn, guides) = drawing::polylines(sketch);
    if drawn.is_empty() && guides.is_empty() {
        return Err(CommandError::failed("the sketch has no curves to write"));
    }
    let write = |text: String| {
        std::fs::write(path, text).map_err(|e| CommandError::failed(format!("{path}: {e}")))
    };
    if id == "sketch.export_svg" {
        let trace =
            drawing::traceability(sketch, &feature.plane, &pictures(sketch, ctx, edges), "mm");
        write(drawing::svg(&drawn, &guides, &trace))?;
        return Ok(json!({"path": path, "curves": drawn.len(), "construction": guides.len()}));
    }
    let inches = match a.opt_string("units")? {
        None | Some("mm") => false,
        Some("inch") => true,
        Some(_) => return Err(CommandError::bad("units", "is mm or inch")),
    };
    let unit = if inches { "inch" } else { "mm" };
    let pictures = pictures(sketch, ctx, edges);
    let trace = drawing::traceability(sketch, &feature.plane, &pictures, unit);
    let sheet = drawing::sheet(sketch, &pictures, &trace, inches);
    let text = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("there is no kernel to write with"))?
        .write_dxf_drawing(&sheet)
        .map_err(|e| CommandError::failed(e.to_string()))?;
    write(text)?;
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
