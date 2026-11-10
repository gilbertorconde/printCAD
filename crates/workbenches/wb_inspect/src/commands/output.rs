//! What an inspection gives out: a print's compensation per axis, and
//! elements saved as STEP or IGES.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, ParamKind,
    WorkbenchContext, WorkbenchRuntimeContext,
};
use serde_json::json;

use crate::{
    COMPENSATION, EXPORT_IGES, EXPORT_STEP, InspectionWorkbench, compare, compensation,
    write_construction,
};

pub(super) fn owns(id: &str) -> bool {
    matches!(id, COMPENSATION | EXPORT_STEP | EXPORT_IGES)
}

pub(super) fn configure(context: &mut WorkbenchContext) {
    context.register_command(
            CommandSpec::new(COMPENSATION, "How a print came out per axis, and what scales it back")
                .param("scan", ParamKind::Id, "The scanned print, aligned onto its model (Best fit)")
                .param("reference", ParamKind::Id, "The body it was printed from")
                .optional("tie_xy", ParamKind::Bool, "One stretch for X and Y together, Z its own")
                .returns("{stretch = {x, y, z}, compensation = {x, y, z}, shift = {x, y, z}, holes, bosses, flare, warp, rms_before, rms_after, used, points}")
                .read_only()
                .note(
                    "Each scan point's distance to the model along its normal is explained as each \
                     axis stretching about the model's middle and a leftover shift, by least \
                     squares; points past three times the fit's RMS (support remains, defects) \
                     are left out. `stretch` is each axis's stretch less one in percent (-0.4 a \
                     print 0.4 % small); `compensation` the factors that scale the model so the \
                     next print comes out as drawn.",
                )
                .note(
                    "The model is read as it stood on the bed, +Z up. Scan points on the walls \
                     of the model's round holes and bosses add each kind's radius error beyond \
                     the stretch: `holes` and `bosses` are {radius_error, offset, points}, nil \
                     with too few points, `offset` the radius change to draw them with (a hole \
                     printing 0.1 mm small in radius reads offset 0.1). The side walls of the \
                     first 0.6 mm are left out of the fit; `flare` is how far they stand out \
                     past it (elephant's foot). `warp` reads the down-facing points within 3 mm \
                     of the bottom: {flatness, lift, at}, the bottom's peak-to-peak and how \
                     high its highest part stands over its middle height, and where.",
                )
                .see_also("inspect.best_fit"),
        );
    context.register_command(
        CommandSpec::new(
            EXPORT_STEP,
            "Write the elements as STEP construction geometry",
        )
        .param("path", ParamKind::String, "The file to write")
        .optional("scan", ParamKind::Id, "Only this scan's")
        .returns("{path, items}")
        .agent_always_asks()
        .note(
            "One part holding no solid, its construction geometry named as the tree \
                     names it: fitted and constructed planes as bounded planes, cylinders, cones \
                     and tori as their axes and rims, spheres and points as points, circles, \
                     lines, every section's chains and shadow outlines as curves, where the scan \
                     stands.",
        ),
    );
    context.register_command(
        CommandSpec::new(
            EXPORT_IGES,
            "Write the elements as IGES construction geometry",
        )
        .param("path", ParamKind::String, "The file to write")
        .optional("scan", ParamKind::Id, "Only this scan's")
        .returns("{path, items, warnings}")
        .agent_always_asks()
        .note(
            "Writes the same geometry as inspect.export_step, at the scan's placement, \
                 with the IGES construction flag (06) and no solid. `items` counts the items \
                 submitted to the writer; `warnings` lists any geometry left out or names \
                 carried only in part. Names containing non-ASCII characters, commas or \
                 semicolons keep only a short label.",
        )
        .see_also(EXPORT_STEP)
        .example(
            "A section saved as IGES with its naming warning",
            r#"
                    local drum = pc.doc.new_body{}
                    pc.design.primitive{body = drum, variant = "cylinder",
                      kind = {Cylinder = {radius = 5, height = 10, angle_deg = 360}}}
                    assert(#pc.doc.rebuild() == 0)
                    local path = os.tmpname() .. ".stl"
                    pc.file.export{path = path, bodies = {drum}}
                    pc.file.import{path = path, scan = true}
                    pc.doc.rebuild()
                    os.remove(path)
                    local scan
                    for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
                    local cut = pc.inspect.section{scan = scan}
                    pc.doc.rename{id = cut.section, name = "Section, centre"}
                    local out = os.tmpname() .. ".igs"
                    local saved = pc.inspect.export_iges{path = out, scan = scan}
                    os.remove(out)
                    assert(saved.path == out and saved.items == 1)
                    assert(#saved.warnings == 1)
                    assert(saved.warnings[1]:find("Section, centre", 1, true))
                "#,
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
        COMPENSATION => {
            let scan = BodyId(a.id("scan")?);
            let reference = BodyId(a.id("reference")?);
            let kernel = ctx
                .kernel
                .ok_or_else(|| CommandError::failed("no kernel to measure with"))?;
            let inputs =
                compare::inputs_for(ctx.document, scan, reference).map_err(CommandError::failed)?;
            let (points, normals) = inputs.points();
            if normals.len() != points.len() {
                return Err(CommandError::failed(
                    "the scan has no normals to measure along",
                ));
            }
            let (ref_mesh, _) = ctx
                .document
                .local_geometry(reference)
                .ok_or_else(|| CommandError::failed("the reference has no mesh"))?;
            let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
            for p in &ref_mesh.positions {
                for k in 0..3 {
                    lo[k] = lo[k].min(f64::from(p[k]));
                    hi[k] = hi[k].max(f64::from(p[k]));
                }
            }
            let diagonal = (0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f64>().sqrt();
            let distances = kernel
                .signed_distance(
                    inputs.target(),
                    &kernel_api::DistanceRequest {
                        points: points.clone(),
                        normals: normals.clone(),
                        max: Some(diagonal / 20.0),
                        facing_limit_deg: Some(60.0),
                    },
                )
                .map_err(|e| CommandError::failed(e.to_string()))?;
            let centre = std::array::from_fn(|k| (lo[k] + hi[k]) / 2.0);
            let roles = compensation::roles(&ref_mesh, &points, &normals, lo[2]);
            let fit = metrology::shrink::fit(
                &points,
                &normals,
                &distances,
                &roles,
                centre,
                a.opt_bool("tie_xy")? == Some(true),
            )
            .ok_or_else(|| {
                CommandError::failed(
                    "the scan's surface does not face enough ways to tell its axes apart",
                )
            })?;
            let xyz = |v: [f64; 3]| json!({"x": v[0], "y": v[1], "z": v[2]});
            let on = |role| roles.iter().filter(|r| **r == role).count();
            let round = |error: Option<f64>, role| {
                error.map(|e| json!({"radius_error": e, "offset": -e, "points": on(role)}))
            };
            let warp = compensation::warp(&points, &normals, lo[2]);
            Ok(json!({
                "stretch": xyz(fit.stretch.map(|e| e * 100.0)),
                "compensation": xyz(fit.compensation()),
                "shift": xyz(fit.shift),
                "holes": round(fit.hole, metrology::shrink::Role::Hole),
                "bosses": round(fit.boss, metrology::shrink::Role::Boss),
                "flare": compensation::flare(&fit, &points, &normals, &distances, &roles),
                "warp": warp.map(|w| json!({
                    "flatness": w.flatness,
                    "lift": w.lift,
                    "at": xyz(w.at),
                })),
                "rms_before": fit.rms[0],
                "rms_after": fit.rms[1],
                "used": fit.used,
                "points": fit.points,
            }))
        }
        EXPORT_STEP | EXPORT_IGES => {
            let items = bench.construction_items(ctx.document, a.opt_id("scan")?.map(BodyId));
            let kernel = ctx
                .kernel
                .ok_or_else(|| CommandError::failed("no kernel to write with"))?;
            let (bytes, warnings) = write_construction(kernel, &items, id == EXPORT_IGES)
                .map_err(|e| CommandError::failed(e.to_string()))?;
            let path = a.string("path")?;
            std::fs::write(path, bytes)
                .map_err(|e| CommandError::failed(format!("{path}: {e}")))?;
            let mut answer = json!({"path": path, "items": items.len()});
            if id == EXPORT_IGES {
                answer["warnings"] = json!(warnings);
            }
            Ok(answer)
        }
        _ => Err(CommandError::Unknown(id.to_string())),
    }
}
