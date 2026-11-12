//! Reading a scan: a hole's or a thread's size, a face snapped onto it, its
//! symmetry plane and how a body lies against it.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, ParamKind,
    WorkbenchRuntimeContext,
};
use serde_json::{Map, json};

use super::features::datum;
use crate::DesignWorkbench;

/// The largest RMS mirror residual, mm, a scan's symmetry plane is taken
/// at.
pub(crate) const SYMMETRY_ACCEPT: f64 = 0.1;

/// The name a scan's symmetry plane is given, by which Mirror finds it.
pub(crate) const SYMMETRY_NAME: &str = "Scan symmetry";

/// The plane `scan` is most nearly symmetric about, kept as a datum plane
/// of `body` when its RMS mirror residual is within `accept`.
pub(crate) fn scan_symmetry(
    ctx: &mut WorkbenchRuntimeContext,
    scan: BodyId,
    body: BodyId,
    accept: f64,
) -> CommandResult {
    if ctx.document.body_scan(scan).is_none() {
        return Err(CommandError::bad("scan", "is not a scan"));
    }
    let (mesh, _) = ctx
        .document
        .local_geometry(scan)
        .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("no kernel to read the scan with"))?;
    let found = kernel
        .symmetry_plane(&mesh)
        .map_err(|e| CommandError::failed(e.to_string()))?;
    if found.rms > accept {
        return Err(CommandError::failed(format!(
            "the scan is not symmetric: its best plane mirrors it to {:.3} mm RMS, over {accept}",
            found.rms
        )));
    }
    // From the scan's frame to the body's.
    let to = ctx
        .document
        .body_placement(body)
        .inverse()
        .after(&ctx.document.body_placement(scan));
    let point = to.point(found.point.map(|c| c as f32)).map(f64::from);
    let normal = to.direction(found.normal.map(|c| c as f32)).map(f64::from);
    let mut args = Map::new();
    args.insert("kind".into(), json!("plane"));
    args.insert("body".into(), json!(body.0.to_string()));
    args.insert("face_point".into(), json!(point));
    args.insert("face_normal".into(), json!(normal));
    args.insert("name".into(), json!(SYMMETRY_NAME));
    let datum_id = datum(&Args(&args), ctx)?;
    Ok(json!({
        "datum": datum_id,
        "rms": found.rms,
        "point": {"x": point[0], "y": point[1], "z": point[2]},
        "normal": {"x": normal[0], "y": normal[1], "z": normal[2]},
    }))
}

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<DesignWorkbench>) {
    t.add(
        CommandSpec::new(
            "design.read_hole",
            "A round's size off a scan, and the nearest standard",
        )
        .param("scan", ParamKind::Id, "The scan")
        .param(
            "at",
            ParamKind::Any,
            "{x, y, z}: a point on the round's wall, in the document's frame",
        )
        .returns("{diameter, depth, hole, sigma, start, axis, sweep, fillet, design, nearest}")
        .read_only(),
        read_hole,
    );
    t.add_on(
        CommandSpec::new(
            "design.snap_to_scan",
            "Move a face of the body onto a scan",
        )
        .param("body", ParamKind::Id, "The body")
        .param("face_point", ParamKind::Any, "{x, y, z}: a point of the face, in the body's own frame")
        .param("face_normal", ParamKind::Any, "{x, y, z}: the face's outward normal there")
        .param("scan", ParamKind::Id, "The scan")
        .param("at", ParamKind::Any, "{x, y, z}: where on the scan the face should lie, in the document's frame")
        .optional("name", ParamKind::String, "The feature's name in the tree")
        .returns("{feature, moved, angle, sigma} for a flat face; {feature, radius, fitted, design, angle, axis_offset, sigma} for a round one"),
        snap_to_scan,
    );
    t.add(
        CommandSpec::new(
            "design.scan_symmetry",
            "A scan's symmetry plane as a datum, for Mirror",
        )
        .param("scan", ParamKind::Id, "The scan")
        .param("body", ParamKind::Id, "The body the datum plane goes in")
        .optional(
            "accept",
            ParamKind::Number,
            "The largest RMS mirror residual taken, mm; 0.1",
        )
        .returns("{datum, rms, point, normal}: the plane in the body's frame"),
        scan_symmetry_command,
    );
    t.add(
        CommandSpec::new(
            "design.read_thread",
            "A thread's pitch, size and hand off a scan",
        )
        .param("scan", ParamKind::Id, "The scan")
        .param(
            "at",
            ParamKind::Any,
            "{x, y, z}: a point on the thread, in the document's frame",
        )
        .returns(
            "{pitch, major, minor, right_handed, internal, clarity, length, start, axis, nearest}",
        )
        .read_only(),
        read_thread,
    );
    t.add(
        CommandSpec::new("design.scan_deviation", "How a body lies against a scan")
            .param("body", ParamKind::Id, "The body being modelled")
            .param("scan", ParamKind::Id, "The scan it is modelled from")
            .optional(
                "range",
                ParamKind::Number,
                "On the surface within this, mm; 0.1",
            )
            .returns("{within, inside, outside, rms, measured}")
            .read_only(),
        scan_deviation,
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "scan.md", include_str!("scan.md"), &[])
}

fn scan_symmetry_command(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let scan = BodyId(a.id("scan")?);
    let body = BodyId(a.id("body")?);
    let accept = a.opt_number("accept")?.unwrap_or(SYMMETRY_ACCEPT);
    scan_symmetry(ctx, scan, body, accept)
}

fn read_hole(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let scan = BodyId(a.id("scan")?);
    let at = a.vec3("at")?;
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("no kernel to fit with"))?;
    let r = crate::read_scan::read_round(ctx.document, kernel, scan, at)
        .map_err(CommandError::failed)?;
    let xyz = |v: [f64; 3]| json!({"x": v[0], "y": v[1], "z": v[2]});
    Ok(json!({
        "diameter": r.diameter,
        "depth": r.depth,
        "hole": r.hole,
        "sigma": r.sigma,
        "start": xyz(r.start),
        "axis": xyz(r.axis),
        "sweep": r.sweep_deg,
        "fillet": r.fillet().then(|| json!({
            "radius": r.diameter / 2.0,
            "design": metrology::design::radius_mm(r.diameter / 2.0),
            "concave": r.hole,
        })),
        "design": metrology::design::length_mm(r.diameter),
        "nearest": if r.fillet() {
            Vec::new()
        } else {
            crate::read_scan::nearest_sizes(r.diameter, r.hole, 3)
        },
    }))
}

fn snap_to_scan(
    bench: &mut DesignWorkbench,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let body = BodyId(a.id("body")?);
    let scan = BodyId(a.id("scan")?);
    let (point, normal, at) = (a.vec3("face_point")?, a.vec3("face_normal")?, a.vec3("at")?);
    let surface = crate::snap::face_surface(ctx.document, body, point)
        .ok_or_else(|| CommandError::failed("the body has no face there"))?;
    let kind = match surface {
        kernel_api::FaceSurface::Plane { .. } => kernel_api::FitKind::Plane,
        kernel_api::FaceSurface::Cylinder { .. } => kernel_api::FitKind::Cylinder,
        _ => {
            return Err(CommandError::failed(
                "only a flat or round face snaps to a scan",
            ));
        }
    };
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("no kernel to fit with"))?;
    let (fitted, sigma) = crate::snap::fit_on_scan(ctx.document, kernel, scan, at, kind, body)
        .map_err(CommandError::failed)?;
    let snap =
        crate::snap::snap_of(&surface, point, normal, &fitted).map_err(CommandError::failed)?;
    let mut call = Map::new();
    call.insert("body".into(), json!(body.0.to_string()));
    call.insert("face_point".into(), json!(point));
    call.insert("face_normal".into(), json!(normal));
    if let Some(name) = a.opt_string("name")? {
        call.insert("name".into(), json!(name));
    }
    Ok(match snap {
        crate::snap::Snap::Move {
            distance,
            angle_deg,
        } => {
            let n = {
                let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2])
                    .sqrt()
                    .max(1e-300);
                normal.map(|c| c / len * distance)
            };
            call.insert("translation".into(), json!(n));
            let feature = super::run(bench, "design.move_faces", &call, ctx)?;
            json!({"feature": feature, "moved": distance, "angle": angle_deg, "sigma": sigma})
        }
        crate::snap::Snap::Offset {
            distance,
            radius,
            fitted,
            angle_deg,
            axis_offset,
        } => {
            call.insert("distance".into(), json!(distance));
            let feature = super::run(bench, "design.offset_faces", &call, ctx)?;
            json!({
                "feature": feature,
                "radius": radius,
                "fitted": fitted,
                "design": metrology::design::radius_mm(fitted),
                "angle": angle_deg,
                "axis_offset": axis_offset,
                "sigma": sigma,
            })
        }
    })
}

fn read_thread(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let scan = BodyId(a.id("scan")?);
    let at = a.vec3("at")?;
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("no kernel to fit with"))?;
    let t = crate::read_scan::read_thread(ctx.document, kernel, scan, at)
        .map_err(CommandError::failed)?;
    let xyz = |v: [f64; 3]| json!({"x": v[0], "y": v[1], "z": v[2]});
    Ok(json!({
        "pitch": t.pitch,
        "major": t.major,
        "minor": t.minor,
        "right_handed": t.right_handed,
        "internal": t.internal,
        "clarity": t.clarity,
        "length": t.length,
        "start": xyz(t.start),
        "axis": xyz(t.axis),
        "nearest": crate::read_scan::nearest_threads(t.major, t.pitch, 3),
    }))
}

fn scan_deviation(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let body = BodyId(a.id("body")?);
    let scan = BodyId(a.id("scan")?);
    let range = a
        .opt_number("range")?
        .unwrap_or(crate::scan_deviation::RANGE);
    if range <= 0.0 {
        return Err(CommandError::bad("range", "is a distance above zero"));
    }
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("no kernel to measure with"))?;
    let inputs =
        crate::scan_deviation::inputs(ctx.document, scan, body).map_err(CommandError::failed)?;
    let values = crate::scan_deviation::measure(kernel, &inputs).map_err(CommandError::failed)?;
    Ok(crate::scan_deviation::reading(&values, range))
}
