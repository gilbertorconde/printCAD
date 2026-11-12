//! What an inspection gives out: a print's compensation per axis, and
//! elements saved as STEP or IGES.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, ParamKind,
    WorkbenchRuntimeContext,
};
use serde_json::json;

use crate::{
    COMPENSATION, EXPORT_IGES, EXPORT_STEP, InspectionWorkbench, compare, compensation,
    write_construction,
};

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<InspectionWorkbench>) {
    t.add(
            CommandSpec::new(COMPENSATION, "How a print came out per axis, and what scales it back")
                .param("scan", ParamKind::Id, "The scanned print, aligned onto its model (Best fit)")
                .param("reference", ParamKind::Id, "The body it was printed from")
                .optional("tie_xy", ParamKind::Bool, "One stretch for X and Y together, Z its own")
                .returns("{stretch = {x, y, z}, compensation = {x, y, z}, shift = {x, y, z}, holes, bosses, flare, warp, rms_before, rms_after, used, points}")
                .read_only(),
        compensation_command,
    );
    t.add_on_by_id(
        CommandSpec::new(
            EXPORT_STEP,
            "Write the elements as STEP construction geometry",
        )
        .param("path", ParamKind::String, "The file to write")
        .optional("scan", ParamKind::Id, "Only this scan's")
        .returns("{path, items}")
        .agent_always_asks(),
        export_step,
    );
    t.add_on_by_id(
        CommandSpec::new(
            EXPORT_IGES,
            "Write the elements as IGES construction geometry",
        )
        .param("path", ParamKind::String, "The file to write")
        .optional("scan", ParamKind::Id, "Only this scan's")
        .returns("{path, items, warnings}")
        .agent_always_asks(),
        export_step,
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "output.md", include_str!("output.md"), &[])
}

fn compensation_command(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
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
    let (lo, hi) = ref_mesh
        .bounds()
        .ok_or_else(|| CommandError::failed("the reference has no mesh"))?;
    let (lo, hi) = (lo.map(f64::from), hi.map(f64::from));
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
        CommandError::failed("the scan's surface does not face enough ways to tell its axes apart")
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

fn export_step(
    bench: &mut InspectionWorkbench,
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let items = bench.construction_items(ctx.document, a.opt_id("scan")?.map(BodyId));
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("no kernel to write with"))?;
    let (bytes, warnings) = write_construction(kernel, &items, id == EXPORT_IGES)
        .map_err(|e| CommandError::failed(e.to_string()))?;
    let path = a.string("path")?;
    std::fs::write(path, bytes).map_err(|e| CommandError::failed(format!("{path}: {e}")))?;
    let mut answer = json!({"path": path, "items": items.len()});
    if id == EXPORT_IGES {
        answer["warnings"] = json!(warnings);
    }
    Ok(answer)
}
