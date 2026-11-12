//! Parameters and commands for filling, smoothing, offsetting, reducing,
//! booleans and shells.
use super::*;
use kernel_api::MeshFairing;

pub(super) const TOOLS: [(&str, &str, &str); 6] = [
    ("mesh.fill", "Fill holes", "triangle"),
    ("mesh.smooth", "Smooth", "workbench-mesh"),
    ("mesh.offset", "Offset", "workbench-mesh"),
    ("mesh.reduce", "Reduce", "triangle"),
    ("mesh.boolean", "Boolean with solids", "boolean"),
    ("mesh.thicken", "Shell / hollow", "thickness"),
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
        "mesh.boolean" => MeshOp::Boolean {
            operand: solids::operand(a, ctx, body)?,
            operation: solids::operation(a.opt_string("operation")?.unwrap_or("subtract"))?,
            chord: positive("chord", 0.01)?,
        },
        "mesh.thicken" => MeshOp::Shell {
            thickness: positive("thickness", d / 125.0)?,
            side: solids::side(a.opt_string("side")?.unwrap_or("both"))?,
        },
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

pub(super) fn table(t: &mut CommandTable<MeshWorkbench>) {
    let scan =
        |spec: CommandSpec| spec.optional("scan", ParamKind::Id, "The mesh body; the selected one");
    let region = |spec: CommandSpec| {
        spec.optional("region", ParamKind::Any, "Explicit MeshRegion for replay")
            .optional(
                "box",
                ParamKind::Any,
                "{min = {x, y, z}, max = {x, y, z}} in the body's frame",
            )
            .optional(
                "ball",
                ParamKind::Any,
                "{centre = {x, y, z}, radius = r} in the body's frame",
            )
    };
    solids::table(t);
    register_steps(t, scan, region);
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands, the
/// examples' setup as `{{scan}}`.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(
        &PROSE,
        "steps.md",
        include_str!("steps.md"),
        &[("scan", SCAN)],
    )
}

/// The examples' setup: a 10 mm cube exported and read back as `scan`.
pub(super) const SCAN: &str = r#"
    local s = pc.sketch.new{plane = "XY"}
    pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
    pc.design.pad{sketch = s, length = 10}
    assert(#pc.doc.rebuild() == 0)
    local path = os.tmpname() .. ".stl"
    pc.file.export{path = path, format = "stl"}
    pc.file.import{path = path, scan = true}
    os.remove(path)
    local scan
    for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
"#;

fn register_steps(
    t: &mut CommandTable<MeshWorkbench>,
    scan: impl Fn(CommandSpec) -> CommandSpec,
    region: impl Fn(CommandSpec) -> CommandSpec,
) {
    commands::on_scan(
        t,
        scan(CommandSpec::new("mesh.fill", "Fill holes"))
            .optional(
                "holes_up_to",
                ParamKind::Number,
                "Largest rim span, mm; diagonal / 20",
            )
            .optional(
                "near",
                ParamKind::Any,
                "A point {x, y, z} near one rim, in the body's frame; otherwise all holes",
            )
            .optional("fairing", ParamKind::String, "curved or flat; curved")
            .returns("{feature, name}"),
        commands::made,
    );
    commands::on_scan(
        t,
        region(scan(CommandSpec::new("mesh.smooth", "Smooth noise")))
            .optional(
                "bumps_up_to",
                ParamKind::Number,
                "Neighbourhood radius, mm; max(diagonal / 125, 2.5 mean edge lengths)",
            )
            .optional(
                "move_at_most",
                ParamKind::Number,
                "Maximum point movement, mm; diagonal / 1000",
            )
            .optional("keep_edges", ParamKind::Bool, "Preserve sharp edges; true")
            .optional(
                "edge_angle",
                ParamKind::Number,
                "Normal turn limit, degrees; 30",
            )
            .returns("{feature, name}"),
        commands::made,
    );
    commands::on_scan(
        t,
        region(scan(CommandSpec::new("mesh.offset", "Offset faces")))
            .optional(
                "distance",
                ParamKind::Number,
                "Normal distance, mm; 0.1, negative shrinks",
            )
            .returns("{feature, name}"),
        commands::made,
    );
    commands::on_scan(
        t,
        scan(CommandSpec::new("mesh.reduce", "Reduce triangles"))
            .optional(
                "tolerance",
                ParamKind::Number,
                "Surface distance, mm; diagonal / 4000 rounded to a 1-2-5 step",
            )
            .optional(
                "triangle_limit",
                ParamKind::Number,
                "Maximum triangles; 50000",
            )
            .returns("{feature, name}"),
        commands::made,
    );
    t.add_on_by_id(
        CommandSpec::new("mesh.result", "Step results")
            .param("feature", ParamKind::Id, "The mesh step")
            .returns("{faces, note, report}")
            .read_only(),
        |bench, id, args, ctx| bench.step_command(id, &Args(args), ctx),
    );
    t.add_on_by_id(
        CommandSpec::new("mesh.set", "Edit a step")
            .param("feature", ParamKind::Id, "The mesh step")
            .optional("op", ParamKind::Any, "A MeshOp of the step's existing kind")
            .optional("offset", ParamKind::Number, "A cut's plane offset, mm")
            .returns("{feature, name}"),
        |bench, id, args, ctx| bench.step_command(id, &Args(args), ctx),
    );
}
