//! Command and selection inputs for scan-only alignments.

use crate::{align, element_at, record_alignment, selected_elements};
use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, FeatureId, ParamKind,
    ToolDescriptor, WorkbenchContext, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};

pub const DATUMS: &str = "inspect.align_datums";
pub const AUTO: &str = "inspect.auto_align";
pub const SYMMETRY: &str = "inspect.use_symmetry";

pub fn owns(id: &str) -> bool {
    matches!(id, DATUMS | AUTO | SYMMETRY)
}

pub fn configure(ctx: &mut WorkbenchContext) {
    for (id, title, icon) in [
        (DATUMS, "3-2-1 datums", "datum-plane"),
        (AUTO, "Auto-align", "joint-align"),
        (SYMMETRY, "Use symmetry", "datum-plane"),
    ] {
        ctx.register_tool(ToolDescriptor::new_action(id, title, Some("align")).icon(icon));
    }
    ctx.register_command(CommandSpec::new(DATUMS, "Align by datums")
        .param("primary", ParamKind::Id, "Plane, cylinder, cone or line element")
        .param("secondary", ParamKind::Id, "Plane normal or axis element on the same scan")
        .param("zero", ParamKind::Id, "Point, sphere, circle centre or plane element")
        .optional("primary_side", ParamKind::String, "+x, -x, +y, -y, +z or -z; default -z")
        .optional("secondary_along", ParamKind::String, "Signed world axis perpendicular to primary_side; default +x")
        .optional("zero_at", ParamKind::Any, "{x, y, z} or three numbers; default origin")
        .optional("centre", ParamKind::Bool, "Centre the scan along any free coordinate; default true")
        .returns("{alignment, name, translation, rotation}")
        .note("The primary fixes its direction and zero, the secondary turns square to it and a plane fixes its remaining coordinate. The zero element fixes only coordinates still free. Parallel directions and mixed scans are refused."));
    for (id, title, note) in [
        (
            AUTO,
            "Auto-align scan",
            "Largest flat face or planar open rim down, long side along X, bounding-box centre on the standing face at zero. Round walls take their axis as Z; otherwise principal axes provide a fallback.",
        ),
        (
            SYMMETRY,
            "Use scan symmetry",
            "Shortest turn of the mirror plane's normal to its nearest signed world axis, then a shift normal to the plane through the origin. Mirror RMS above 0.5 mm is refused.",
        ),
    ] {
        ctx.register_command(
            CommandSpec::new(id, title)
                .param("scan", ParamKind::Id, "The scan")
                .returns("{alignment, name, translation, rotation}")
                .note(note),
        );
    }
}

pub fn selected_args(id: &str, ctx: &WorkbenchRuntimeContext) -> Option<CommandArgs> {
    let mut args = CommandArgs::new();
    if id == DATUMS {
        let ids = selected_elements(ctx);
        if ids.len() != 3 {
            return None;
        }
        for (key, id) in ["primary", "secondary", "zero"].into_iter().zip(ids) {
            args.insert(key.into(), json!(id.0.to_string()));
        }
        prepare_datums(&Args(&args), ctx, true).ok()?;
    } else {
        let selected = ctx.selected_body_id.map(BodyId).or_else(|| {
            selected_elements(ctx)
                .first()
                .and_then(|id| element_at(ctx.document, *id))
                .map(|e| e.scan)
        });
        let scan = selected
            .filter(|s| ctx.document.body_scan(*s).is_some())
            .or_else(|| {
                let mut scans = ctx
                    .document
                    .bodies()
                    .iter()
                    .filter(|b| ctx.document.body_scan(b.id).is_some());
                let scan = scans.next()?.id;
                scans.next().is_none().then_some(scan)
            })?;
        ctx.document.local_geometry(scan)?;
        ctx.kernel?;
        args.insert("scan".into(), json!(scan.0.to_string()));
    }
    Some(args)
}

fn prepare_datums(
    a: &Args<'_>,
    ctx: &WorkbenchRuntimeContext,
    check_only: bool,
) -> Result<(BodyId, align::How, core_document::BodyPlacement), CommandError> {
    let ids = [
        FeatureId(a.id("primary")?),
        FeatureId(a.id("secondary")?),
        FeatureId(a.id("zero")?),
    ];
    let elements = ids
        .iter()
        .map(|id| {
            element_at(ctx.document, *id)
                .ok_or_else(|| CommandError::failed("datum is not an element"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let scan = elements[0].scan;
    if elements.iter().any(|e| e.scan != scan) || ctx.document.body_scan(scan).is_none() {
        return Err(CommandError::failed("datums must belong to one scan"));
    }
    let primary_side = a.opt_string("primary_side")?.unwrap_or("-z");
    let secondary_along = a.opt_string("secondary_along")?.unwrap_or("+x");
    let mut zero_at = [0.0; 3];
    if let Some(v) = a.0.get("zero_at") {
        for (i, key) in ["x", "y", "z"].iter().enumerate() {
            zero_at[i] = v
                .get(*key)
                .or_else(|| v.get(i))
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite())
                .ok_or_else(|| CommandError::bad("zero_at", "needs three finite coordinates"))?;
        }
    }
    let mesh = ctx
        .document
        .local_geometry(scan)
        .ok_or_else(|| CommandError::failed("scan has no mesh"))?;
    let after = align::datums(
        &mesh.0,
        ctx.document.body_placement(scan),
        &align::Datums {
            shapes: [
                &elements[0].result.shape,
                &elements[1].result.shape,
                &elements[2].result.shape,
            ],
            primary_side,
            secondary_along,
            zero_at,
            centre: !check_only && a.opt_bool("centre")?.unwrap_or(true),
        },
    )
    .map_err(CommandError::failed)?;
    Ok((
        scan,
        align::How::Datums {
            primary: ids[0],
            secondary: ids[1],
            zero: ids[2],
            primary_side: primary_side.into(),
            secondary_along: secondary_along.into(),
        },
        after,
    ))
}

pub fn run(id: &str, args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let (scan, how, after) = if id == DATUMS {
        prepare_datums(&a, ctx, false)?
    } else {
        let scan = BodyId(a.id("scan")?);
        if ctx.document.body_scan(scan).is_none() {
            return Err(CommandError::failed("only a scan is aligned"));
        }
        let mesh = ctx
            .document
            .local_geometry(scan)
            .ok_or_else(|| CommandError::failed("scan has no mesh"))?;
        let kernel = ctx
            .kernel
            .ok_or_else(|| CommandError::failed("no kernel to align with"))?;
        let (how, after) = if id == AUTO {
            let proposal = kernel
                .orient(&mesh.0)
                .map_err(|e| CommandError::failed(e.to_string()))?;
            let after = align::automatic(&proposal);
            (
                align::How::Auto {
                    said: proposal.said,
                },
                after,
            )
        } else {
            let plane = kernel
                .symmetry_plane(&mesh.0)
                .map_err(|e| CommandError::failed(e.to_string()))?;
            let after = align::symmetry(ctx.document.body_placement(scan), &plane)
                .map_err(CommandError::failed)?;
            (align::How::Symmetry { rms: plane.rms }, after)
        };
        (scan, how, after)
    };
    record_alignment(ctx, scan, how, after).map_err(CommandError::failed)
}
