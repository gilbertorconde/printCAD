//! Parameters and commands for filling, smoothing, offsetting and reducing.
use super::*;
use kernel_api::MeshFairing;

pub(super) const TOOLS: [(&str, &str, &str); 4] = [
    ("mesh.fill", "Fill holes", "triangle"),
    ("mesh.smooth", "Smooth", "workbench-mesh"),
    ("mesh.offset", "Offset", "workbench-mesh"),
    ("mesh.reduce", "Reduce", "triangle"),
];

/// Each undirected geometric edge counts once, including split corners.
fn mean_edge(mesh: &TriMesh) -> f64 {
    let mut seen = std::collections::HashSet::new();
    let mut sum = 0.0;
    for t in mesh.indices.as_chunks::<3>().0 {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let (a, b) = (mesh.positions[a as usize], mesh.positions[b as usize]);
            let (ka, kb) = (a.map(f32::to_bits), b.map(f32::to_bits));
            if ka != kb && seen.insert(if ka < kb { (ka, kb) } else { (kb, ka) }) {
                sum += (0..3)
                    .map(|k| (f64::from(a[k]) - f64::from(b[k])).powi(2))
                    .sum::<f64>()
                    .sqrt();
            }
        }
    }
    sum / seen.len().max(1) as f64
}

fn rounded(value: f64) -> f64 {
    let decade = 10.0_f64.powf(value.max(1e-12).log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|s| s * decade)
        .min_by(|a, b| (a - value).abs().total_cmp(&(b - value).abs()))
        .unwrap()
}

pub(super) fn make(
    id: &str,
    a: &Args,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    mesh: &TriMesh,
) -> Result<MeshOp, CommandError> {
    let d = diagonal(mesh);
    let positive = |name: &str, default: f64| -> Result<f64, CommandError> {
        let n = a.opt_number(name)?.unwrap_or(default);
        if n > 0.0 && n.is_finite() {
            Ok(n)
        } else {
            Err(CommandError::bad(name, "must be positive and finite"))
        }
    };
    Ok(match id {
        "mesh.fill" => MeshOp::Fill {
            holes_up_to: positive("holes_up_to", d / 20.0)?,
            near: a
                .0
                .get("near")
                .map(|v| {
                    serde_json::from_value(v.clone())
                        .map_err(|_| CommandError::bad("near", "is {x, y, z} in the body's frame"))
                })
                .transpose()?,
            fairing: match a.opt_string("fairing")?.unwrap_or("curved") {
                "curved" | "Curved" => MeshFairing::Curved,
                "flat" | "Flat" => MeshFairing::Flat,
                _ => return Err(CommandError::bad("fairing", "is curved or flat")),
            },
        },
        "mesh.smooth" => MeshOp::Smooth {
            region: region(a, ctx, body, true)?,
            bumps_up_to: positive("bumps_up_to", (d / 125.0).max(2.5 * mean_edge(mesh)))?,
            move_at_most: a.opt_number("move_at_most")?.unwrap_or(d / 1000.0),
            keep_edges_deg: if a.opt_bool("keep_edges")?.unwrap_or(true) {
                Some(a.opt_number("edge_angle")?.unwrap_or(30.0))
            } else {
                None
            },
        },
        "mesh.offset" => MeshOp::Offset {
            region: region(a, ctx, body, true)?,
            distance: a.opt_number("distance")?.unwrap_or(0.1),
        },
        "mesh.reduce" => {
            let limit = positive("triangle_limit", 50_000.0)?;
            if limit.fract() != 0.0 || limit > u32::MAX as f64 {
                return Err(CommandError::bad(
                    "triangle_limit",
                    "must be a positive integer within u32",
                ));
            }
            MeshOp::Reduce {
                tolerance: positive("tolerance", rounded(d / 4000.0))?,
                triangle_limit: limit as usize,
            }
        }
        _ => return Err(CommandError::Unknown(id.into())),
    })
}

pub(super) fn register(context: &mut WorkbenchContext) {
    let scan =
        |spec: CommandSpec| spec.optional("scan", ParamKind::Id, "The mesh body; the selected one");
    let region = |spec: CommandSpec| {
        spec
        .optional("region", ParamKind::Any, "Explicit MeshRegion for replay")
        .optional("box", ParamKind::Any, "{min = {x, y, z}, max = {x, y, z}} in the body's frame")
        .optional("ball", ParamKind::Any, "{centre = {x, y, z}, radius = r} in the body's frame")
        .note("Without a box or a ball, uses the marking, or the whole mesh if nothing is marked.")
    };
    let example = |code: &str| {
        format!(
            r#"
        local s = pc.sketch.new{{plane = "XY"}}
        pc.sketch.rect{{sketch = s, x = 0, y = 0, width = 10, height = 10}}
        pc.design.pad{{sketch = s, length = 10}}
        assert(#pc.doc.rebuild() == 0)
        local path = os.tmpname() .. ".stl"
        pc.file.export{{path = path, format = "stl"}}
        pc.file.import{{path = path, scan = true}}
        os.remove(path)
        local scan
        for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
        {code}
    "#
        )
    };
    context.register_command(scan(CommandSpec::new("mesh.fill", "Fill mesh holes"))
        .optional("holes_up_to", ParamKind::Number, "Largest rim span, mm; diagonal / 20")
        .optional("near", ParamKind::Any, "A point {x, y, z} near one rim, in the body's frame; otherwise all holes")
        .optional("fairing", ParamKind::String, "curved or flat; curved")
        .returns("{feature, name}")
        .note("Only holes within the size limit are filled. With near, chooses the nearest rim before testing its size. Scan vertices never move. Rebuild before reading mesh.result.")
        .example("Close the open top of a scan", &example(r#"
            pc.mesh.delete{scan = scan, box = {min = {-1, -1, 9.99}, max = {11, 11, 11}}}
            assert(#pc.doc.rebuild() == 0)
            assert(pc.mesh.inspect{scan = scan}.open_edges > 0)
            local step = pc.mesh.fill{scan = scan, holes_up_to = 15, fairing = "flat"}
            assert(#pc.doc.rebuild() == 0)
            assert(pc.mesh.inspect{scan = scan}.open_edges == 0)
            assert(math.abs(pc.mesh.inspect{scan = scan}.volume - 1000) < 10)
            assert(pc.mesh.result{feature = step.feature}.faces > 0)
        "#)));
    context.register_command(region(scan(CommandSpec::new("mesh.smooth", "Smooth scan noise")))
        .optional("bumps_up_to", ParamKind::Number, "Neighbourhood radius, mm; max(diagonal / 125, 2.5 mean edge lengths)")
        .optional("move_at_most", ParamKind::Number, "Maximum point movement, mm; diagonal / 1000")
        .optional("keep_edges", ParamKind::Bool, "Preserve sharp edges; true")
        .optional("edge_angle", ParamKind::Number, "Normal turn limit, degrees; 30")
        .returns("{feature, name}")
        .note("Projects points onto local quadrics in two passes. A marking blends to zero at its edge. mesh.result reports maximum and RMS movement after rebuilding.")
        .example("Bound a smoothing step's movement", &example(r#"
            local step = pc.mesh.smooth{scan = scan, move_at_most = 0.01}
            assert(#pc.doc.rebuild() == 0)
            local r = pc.mesh.result{feature = step.feature}.report.Smooth
            assert(r.max <= 0.010001 and r.rms <= r.max)
        "#)));
    context.register_command(region(scan(CommandSpec::new("mesh.offset", "Offset mesh faces")))
        .optional("distance", ParamKind::Number, "Normal distance, mm; 0.1, negative shrinks")
        .returns("{feature, name}")
        .note("Offsets faces rather than scaling the mesh. A marked patch joins its neighbours with a wall.")
        .example("Expand a ten millimetre cube", &example(r#"
            pc.mesh.offset{scan = scan, distance = 0.1}
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.mesh.inspect{scan = scan}.volume - 10.2^3) < 0.1)
        "#)));
    context.register_command(scan(CommandSpec::new("mesh.reduce", "Reduce mesh triangles"))
        .optional("tolerance", ParamKind::Number, "Surface distance, mm; diagonal / 4000 rounded to a 1-2-5 step")
        .optional("triangle_limit", ParamKind::Number, "Maximum triangles; 50000")
        .returns("{feature, name}")
        .note("Verifies both ways at 99.9%. The triangle limit takes priority over tolerance; mesh.result reports the actual within-tolerance share, p999, furthest point and distance, RMS and whether the limit was reached. An impossible limit is reported, not hidden.")
        .example("Read a reduction's verified accuracy", &example(r#"
            local step = pc.mesh.reduce{scan = scan, tolerance = 0.01, triangle_limit = 50000}
            assert(#pc.doc.rebuild() == 0)
            local r = pc.mesh.result{feature = step.feature}
            assert(r.faces <= 50000)
            assert(r.report.Reduce.p999 <= 0.01)
            assert(r.report.Reduce.within_share >= 0.999)
        "#)));
    context.register_command(CommandSpec::new("mesh.result", "Read mesh step results")
        .param("feature", ParamKind::Id, "The mesh step")
        .returns("{faces, note, report}").read_only()
        .note("Rebuild first. Smooth reports {max, rms}; Reduce reports {within_share, p999, furthest, max, rms, target_met}. Distances are mm, furthest is in the body's frame. A changed or failed history has no result for an unbuilt step.")
        .example("Read a built step", &example(r#"
            local step = pc.mesh.smooth{scan = scan, move_at_most = 0.01}
            assert(#pc.doc.rebuild() == 0)
            local result = pc.mesh.result{feature = step.feature}
            assert(result.faces > 0 and result.report.Smooth.max <= 0.010001)
        "#)));
    context.register_command(CommandSpec::new("mesh.set", "Edit a mesh step")
        .param("feature", ParamKind::Id, "The mesh step")
        .optional("op", ParamKind::Any, "A MeshOp of the step's existing kind")
        .optional("offset", ParamKind::Number, "A cut's plane offset, mm")
        .returns("{feature, name}")
        .note("Edits the existing step; it does not create a step or open a task. Its region uses start-mesh vertex numbers.")
        .example("Change an offset in place", &example(r#"
            local step = pc.mesh.offset{scan = scan, distance = 0.1}
            pc.mesh.set{feature = step.feature, op = {Offset = {region = "All", distance = 0.2}}}
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.mesh.inspect{scan = scan}.volume - 10.4^3) < 0.1)
        "#)));
}
