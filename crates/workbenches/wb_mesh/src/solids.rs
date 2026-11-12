//! The steps that take or make solids (a boolean with a bounded solid, a
//! shell), a solid body trimmed to a scan's break, and the read-only cut
//! preview and export.
use super::*;
use kernel_api::{BooleanOp, MeshOperand, MeshSide, TrimRequest};

/// `p` moved by the row-major matrix, a direction when `w` is 0.
fn moved(rows: &[[f64; 4]; 4], p: [f64; 3], w: f64) -> [f64; 3] {
    std::array::from_fn(|r| {
        rows[r][0] * p[0] + rows[r][1] * p[1] + rows[r][2] * p[2] + w * rows[r][3]
    })
}

/// The operand a boolean takes, in the scan's frame, as it stands now:
/// the `operand` named (a solid body or a feature offering a bounded
/// solid), else the selected feature offering one, else the first other
/// body with a solid.
pub(super) fn operand(
    a: &Args,
    ctx: &WorkbenchRuntimeContext,
    scan: BodyId,
) -> Result<MeshOperand, CommandError> {
    let document = &*ctx.document;
    let id = a
        .opt_id("operand")?
        .or_else(|| {
            ctx.selected_features
                .iter()
                .find(|id| document.feature_solid(**id).is_some())
                .map(|id| id.0)
        })
        .or_else(|| {
            document
                .bodies()
                .iter()
                .find(|b| b.id != scan && document.imported_brep_blob_arc(b.id).is_some())
                .map(|b| b.id.0)
        })
        .ok_or_else(|| {
            CommandError::bad("operand", "select a bounded element or name a solid body")
        })?;
    let into = document.body_placement(scan).inverse();
    if id == scan.0 {
        return Err(CommandError::bad("operand", "is the scan itself"));
    }
    if let Some(blob) = document.imported_brep_blob_arc(BodyId(id)) {
        let brep = String::from_utf8(blob.to_vec())
            .map_err(|_| CommandError::failed("the operand's solid is not in the native format"))?;
        let transform = into.after(&document.body_placement(BodyId(id))).rows();
        return Ok(MeshOperand::Shape { brep, transform });
    }
    let mut shape = document
        .feature_solid(FeatureId(id))
        .cloned()
        .ok_or_else(|| {
            CommandError::bad("operand", "is neither a solid body nor a bounded element")
        })?;
    let rows = into.rows();
    match &mut shape {
        MeshOperand::Primitive { placement, .. } => {
            placement.origin = moved(&rows, placement.origin, 1.0);
            placement.x_axis = moved(&rows, placement.x_axis, 0.0);
            placement.z_axis = moved(&rows, placement.z_axis, 0.0);
        }
        MeshOperand::Shape { transform, .. } => {
            let first = *transform;
            *transform = std::array::from_fn(|r| {
                std::array::from_fn(|c| (0..4).map(|k| rows[r][k] * first[k][c]).sum())
            });
        }
    }
    Ok(shape)
}

pub(super) fn operation(value: &str) -> Result<BooleanOp, CommandError> {
    match value {
        "subtract" => Ok(BooleanOp::Cut),
        "add" => Ok(BooleanOp::Fuse),
        "intersect" => Ok(BooleanOp::Common),
        _ => Err(CommandError::bad(
            "operation",
            "is subtract, add or intersect",
        )),
    }
}

pub(super) fn side(value: &str) -> Result<MeshSide, CommandError> {
    match value {
        "inward" => Ok(MeshSide::Inward),
        "outward" => Ok(MeshSide::Outward),
        "both" => Ok(MeshSide::Both),
        _ => Err(CommandError::bad("side", "is inward, outward or both")),
    }
}

/// A plane cut from the command's arguments, placed on `mesh`.
pub(super) fn cut(a: &Args, mesh: &TriMesh) -> Result<MeshOp, CommandError> {
    let normal = match a.opt_string("plane")?.unwrap_or("xy") {
        "xy" | "XY" => [0.0, 0.0, 1.0],
        "xz" | "XZ" => [0.0, 1.0, 0.0],
        "yz" | "YZ" => [1.0, 0.0, 0.0],
        _ => return Err(CommandError::bad("plane", "is xy, xz or yz")),
    };
    let (lo, hi) = mesh
        .bounds()
        .ok_or_else(|| CommandError::failed("the scan has no points"))?;
    let (lo, hi) = (lo.map(f64::from), hi.map(f64::from));
    let mut origin: [f64; 3] = std::array::from_fn(|k| (lo[k] + hi[k]) / 2.0);
    if let Some(at) = a.opt_number("at")? {
        for k in 0..3 {
            if normal[k] != 0.0 {
                origin[k] = at;
            }
        }
    }
    let keep_above = match a.opt_string("keep")?.unwrap_or("below") {
        "below" => false,
        "above" => true,
        _ => return Err(CommandError::bad("keep", "is below or above")),
    };
    Ok(MeshOp::Cut {
        origin,
        normal,
        keep_above,
        close: a.opt_bool("close")?.unwrap_or(true),
        bridge: a.opt_number("bridge")?.unwrap_or(diagonal(mesh) / 50.0),
    })
}

/// The cut `mesh.cut` would add, built on the mesh as it stands.
pub(super) fn preview_cut(a: &Args, ctx: &WorkbenchRuntimeContext, scan: BodyId) -> CommandResult {
    let now = drawn(ctx, scan)?;
    let op = cut(a, &now)?;
    let answer = kernel(ctx)?
        .build_mesh_chain(&now, &[op])
        .map_err(|e| CommandError::failed(e.to_string()))?;
    if let Some((_, why)) = answer.failed {
        return Err(CommandError::failed(why));
    }
    Ok(json!(answer.steps.first()))
}

/// The scan's mesh at its tip written to `path`.
pub(super) fn export(a: &Args, ctx: &WorkbenchRuntimeContext, scan: BodyId) -> CommandResult {
    let path = a.string("path")?;
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let format = a.opt_string("format")?.unwrap_or(extension);
    let now = drawn(ctx, scan)?;
    let bytes = kernel(ctx)?
        .write_mesh(&now, format)
        .map_err(|e| CommandError::bad("format", e.to_string()))?;
    std::fs::write(path, bytes).map_err(|e| CommandError::bad("path", e.to_string()))?;
    Ok(json!({"path": path, "triangles": now.indices.len() / 3}))
}

/// The solid body named `body` (else the first other body with a solid)
/// trimmed where `scan` breaks off. The result replaces the body's shape
/// when it is a shape from outside its history; a body built from its
/// features keeps them, hidden, and a new body beside it takes the result.
pub(super) fn trim(a: &Args, ctx: &mut WorkbenchRuntimeContext, scan: BodyId) -> CommandResult {
    let document = &*ctx.document;
    let body = match a.opt_id("body")? {
        Some(id) => BodyId(id),
        None => document
            .bodies()
            .iter()
            .find(|b| b.id != scan && document.imported_brep_blob_arc(b.id).is_some())
            .map(|b| b.id)
            .ok_or_else(|| CommandError::bad("body", "names no solid body, and there is none"))?,
    };
    if body == scan {
        return Err(CommandError::bad("body", "is the scan itself"));
    }
    let solid = document
        .imported_brep_blob_arc(body)
        .ok_or_else(|| CommandError::bad("body", "has no solid; rebuild first"))?;
    let mesh = drawn(ctx, scan)?;
    let into = document
        .body_placement(body)
        .inverse()
        .after(&document.body_placement(scan));
    let non_negative = |name: &str, default: f64| -> Result<f64, CommandError> {
        let n = a.opt_number(name)?.unwrap_or(default);
        if n >= 0.0 && n.is_finite() {
            Ok(n)
        } else {
            Err(CommandError::bad(name, "must be a finite distance"))
        }
    };
    let request = TrimRequest {
        solid: solid.to_vec(),
        scan: into.mesh(&mesh),
        depth: non_negative("depth", 0.3)?,
        gap: non_negative("gap", 0.1)?,
        away: match a.opt_string("side")?.unwrap_or("away") {
            "away" => true,
            "toward" => false,
            _ => return Err(CommandError::bad("side", "is away or toward")),
        },
    };
    let answer = kernel(ctx)?
        .trim_to_scan(&request)
        .map_err(|e| CommandError::failed(e.to_string()))?;
    let name = document
        .body(body)
        .map_or_else(|| "Piece".to_string(), |b| b.name.clone());
    let file = format!("{name} trimmed.brep");
    let landed = if ctx.document.can_replace_shape(body) {
        ctx.document.replace_body_shape(body, &file, answer.brep);
        body
    } else {
        let placement = ctx.document.body_placement(body);
        let new = ctx.document.create_body(Some(format!("{name} trimmed")));
        ctx.document.set_body_placement(new, placement);
        ctx.document.shape_body_from_file(new, &file, answer.brep);
        ctx.document.set_body_visible(body, false);
        new
    };
    Ok(json!({
        "body": landed.0.to_string(),
        "volume": answer.volume,
        "mating_area": answer.mating_area,
        "gap_median": answer.gap_median,
        "gap_range": answer.gap_range,
    }))
}

fn drawn(
    ctx: &WorkbenchRuntimeContext,
    scan: BodyId,
) -> Result<std::sync::Arc<TriMesh>, CommandError> {
    ctx.document
        .local_geometry(scan)
        .map(|(mesh, _)| mesh)
        .ok_or_else(|| CommandError::failed("the scan has no mesh"))
}

fn kernel<'a>(
    ctx: &WorkbenchRuntimeContext<'a>,
) -> Result<&'a dyn kernel_api::KernelQueries, CommandError> {
    ctx.kernel.ok_or_else(|| CommandError::failed("no kernel"))
}

pub(super) fn table(t: &mut CommandTable<MeshWorkbench>) {
    let scan =
        |s: CommandSpec| s.optional("scan", ParamKind::Id, "The mesh body; the selected one");
    let example = steps::example;
    commands::on_scan(t, scan(CommandSpec::new("mesh.boolean", "Boolean a solid"))
        .optional("operand", ParamKind::Id, "A solid body, or a feature offering a bounded solid; the selected one")
        .optional("operation", ParamKind::String, "subtract, add or intersect; subtract")
        .optional("chord", ParamKind::Number, "Operand meshing chord, mm; 0.01")
        .returns("{feature, name}")
        .note("Takes the operand as it stands when the step is made: a body's solid where it sits, or an Inspection element's cylinder, cone or sphere over its drawn extent. The scan must be closed (fill holes first), and an operand it does not meet is refused when the step builds. mesh.result reports cut_triangles and the volume in mm³.")
        .example("Drill a scan", &example(r#"
            local pin = pc.doc.new_body{}
            pc.design.primitive{body = pin, kind = {Cylinder = {radius = 1, height = 12, angle_deg = 360}}, placement = {origin = {5, 5, -1}, x_axis = {1, 0, 0}, z_axis = {0, 0, 1}}}
            assert(#pc.doc.rebuild() == 0)
            local step = pc.mesh.boolean{scan = scan, operand = pin}
            assert(#pc.doc.rebuild() == 0)
            local volume = pc.mesh.result{feature = step.feature}.report.Boolean.volume
            assert(math.abs(volume - (1000 - math.pi * 10)) < 1)
        "#)), commands::made);
    commands::on_scan(t, scan(CommandSpec::new("mesh.thicken", "Thicken or hollow"))
        .optional("thickness", ParamKind::Number, "Wall thickness, mm; diagonal / 125")
        .optional("side", ParamKind::String, "An open sheet's side: inward, outward or both; both")
        .returns("{feature, name}")
        .note("A closed mesh keeps its outer surface and gains an inner wall around a sealed cavity. An open sheet gains a second layer joined to it by walls along its rims; both puts half the thickness to each side. mesh.result reports whether a cavity was made and the volume in mm³.")
        .example("Wall a cube in", &example(r#"
            local step = pc.mesh.thicken{scan = scan, thickness = 1}
            assert(#pc.doc.rebuild() == 0)
            local r = pc.mesh.result{feature = step.feature}.report.Shell
            assert(r.hollow and math.abs(r.volume - 488) < 10)
            local walled = pc.mesh.inspect{scan = scan}
            assert(walled.open_edges == 0 and math.abs(walled.volume - r.volume) < 1e-3)
        "#)), commands::made);
    commands::on_scan(t, scan(CommandSpec::new("mesh.trim_to_scan", "Trim a piece"))
        .optional("body", ParamKind::Id, "The solid body modelled into the break; the first other solid")
        .optional("depth", ParamKind::Number, "Break depth: how far inside the body the scan must lie, mm; 0.3")
        .optional("gap", ParamKind::Number, "Glue gap between break and piece, mm; 0.1")
        .optional("side", ParamKind::String, "away (from the scan) or toward; away")
        .returns("{body, volume, mating_area, gap_median, gap_range}")
        .agent_always_asks()
        .note("The break is the scan's surface lying deeper than depth inside the body; it is extended past the body, thickened by the gap to each side and cut away, keeping the side given. A body whose shape came from a file takes the result in place; a body built from its features is hidden and a new body takes it. Volume in mm³, mating area in mm², the gap's median and 90 % range in mm. Changing the shape clears the undo history, as an import does.")
        .example("Trim a cap to a broken top", &example(r#"
            local cap = pc.doc.new_body{}
            pc.design.primitive{body = cap, kind = {Box = {length = 12, width = 12, height = 8}}, placement = {origin = {-1, -1, 8}, x_axis = {1, 0, 0}, z_axis = {0, 0, 1}}}
            assert(#pc.doc.rebuild() == 0)
            local r = pc.mesh.trim_to_scan{scan = scan, body = cap, gap = 0.1}
            assert(math.abs(r.volume - 144 * 5.9) < 0.01)
            assert(math.abs(r.mating_area - 100) < 0.01 and math.abs(r.gap_median - 0.1) < 1e-6)
        "#)), |_, a, ctx, body, _| trim(a, ctx, body));
    commands::on_scan(
        t,
        scan(CommandSpec::new("mesh.preview_cut", "Preview a cut"))
            .optional("plane", ParamKind::String, "xy, xz or yz; xy")
            .optional(
                "at",
                ParamKind::Number,
                "Plane position, mm; the mesh's middle",
            )
            .optional("keep", ParamKind::String, "below or above; below")
            .optional("close", ParamKind::Bool, "Cap the cut flat; true")
            .optional(
                "bridge",
                ParamKind::Number,
                "Close rim gaps up to this, mm; diagonal / 50",
            )
            .returns("{faces, note, report = {Cut = {capped_loops, bridged, open_loops}}}")
            .read_only()
            .note("Builds the cut mesh.cut would add on the mesh as it stands, recording nothing.")
            .example(
                "Preview a cap",
                &example(
                    r#"
            local before = pc.mesh.inspect{scan = scan}.triangles
            local cut = pc.mesh.preview_cut{scan = scan, at = 5}
            assert(cut.faces > 0 and cut.report.Cut.capped_loops == 1)
            assert(cut.report.Cut.open_loops == 0)
            assert(pc.mesh.inspect{scan = scan}.triangles == before)
        "#,
                ),
            ),
        |_, a, ctx, body, _| preview_cut(a, ctx, body),
    );
    commands::on_scan(t, scan(CommandSpec::new("mesh.export", "Write a file"))
        .param("path", ParamKind::String, "The file to write")
        .optional("format", ParamKind::String, "stl, 3mf, ply or obj; the path's extension")
        .returns("{path, triangles}")
        .read_only()
        .agent_always_asks()
        .note("Writes the mesh at its history's tip, its triangles as they are, in the body's frame and millimetres.")
        .example("Export and read back", &example(r#"
            local path = os.tmpname() .. ".ply"
            local result = pc.mesh.export{scan = scan, path = path}
            pc.file.import{path = path, scan = true}
            os.remove(path)
            assert(result.triangles == pc.mesh.inspect{scan = scan}.triangles)
        "#)), |_, a, ctx, body, _| export(a, ctx, body));
}
