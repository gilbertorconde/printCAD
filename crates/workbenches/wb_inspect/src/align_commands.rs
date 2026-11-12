//! Command and selection inputs for scan-only alignments.

use crate::{align, element_at, record_alignment, selected_elements};
use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandFn, CommandResult, CommandSpec, CommandTable,
    FeatureId, ParamKind, ToolDescriptor, WorkbenchContext, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};
use std::sync::OnceLock;

pub const DATUMS: &str = "inspect.align_datums";
pub const AUTO: &str = "inspect.auto_align";
pub const SYMMETRY: &str = "inspect.use_symmetry";
pub const POINTS: &str = "inspect.align_points";

/// The alignment commands, each spec beside what runs it.
fn table() -> &'static CommandTable {
    static TABLE: OnceLock<CommandTable> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = CommandTable::default();
        specs(&mut t);
        t
    })
}

/// The table the bench's own includes, these commands among them.
pub fn family() -> &'static CommandTable {
    table()
}

/// Whether `id` is one of the alignment commands.
pub fn owns(id: &str) -> bool {
    table().owns(id)
}

/// Run alignment command `id`.
pub fn run(id: &str, args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    table().run(id, args, ctx)
}

pub fn configure(ctx: &mut WorkbenchContext) {
    for (id, title, icon) in [
        (DATUMS, "3-2-1 datums", "datum-plane"),
        (AUTO, "Auto-align", "joint-align"),
        (SYMMETRY, "Use symmetry", "datum-plane"),
    ] {
        ctx.register_tool(ToolDescriptor::new_action(id, title, Some("align")).icon(icon));
    }
    ctx.register_tool(
        ToolDescriptor::new_action(POINTS, "Align by points", Some("align")).icon("datum-point"),
    );
}

fn specs(t: &mut CommandTable) {
    t.add(
        CommandSpec::new(DATUMS, "Align by datums")
            .param(
                "primary",
                ParamKind::Id,
                "Plane, cylinder, cone or line element",
            )
            .param(
                "secondary",
                ParamKind::Id,
                "Plane normal or axis element on the same scan",
            )
            .param(
                "zero",
                ParamKind::Id,
                "Point, sphere, circle centre or plane element",
            )
            .optional(
                "primary_side",
                ParamKind::String,
                "+x, -x, +y, -y, +z or -z; default -z",
            )
            .optional(
                "secondary_along",
                ParamKind::String,
                "Signed world axis perpendicular to primary_side; default +x",
            )
            .optional(
                "zero_at",
                ParamKind::Any,
                "{x, y, z} or three numbers; default origin",
            )
            .optional(
                "centre",
                ParamKind::Bool,
                "Centre the scan along any free coordinate; default true",
            )
            .returns("{alignment, name, translation, rotation}"),
        by_datums,
    );
    t.add(
        CommandSpec::new(
            POINTS,
            "Align a scan by points picked on it and on a reference",
        )
        .param("scan", ParamKind::Id, "The scan")
        .param(
            "pairs",
            ParamKind::List,
            "Three or more {scan = {x, y, z}, reference = {x, y, z}}: a point on the scan \
                 where it stands now and the point on the reference it is",
        )
        .returns("{alignment, name, translation, rotation, rms}"),
        by_points,
    );
    let auto: CommandFn = automatic;
    let symmetric: CommandFn = by_symmetry;
    for (id, title, run) in [
        (AUTO, "Auto-align scan", auto),
        (SYMMETRY, "Use scan symmetry", symmetric),
    ] {
        t.add(
            CommandSpec::new(id, title)
                .param("scan", ParamKind::Id, "The scan")
                .returns("{alignment, name, translation, rotation}"),
            run,
        );
    }
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(
        &PROSE,
        "align_commands.md",
        include_str!("align_commands.md"),
        &[],
    )
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
    let zero_at = a.opt_vec3("zero_at")?.unwrap_or([0.0; 3]);
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

fn by_points(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let scan = BodyId(a.id("scan")?);
    let (how, after) = points(&a, ctx, scan)?;
    let mut answer =
        record_alignment(ctx, scan, how.clone(), after).map_err(CommandError::failed)?;
    if let align::How::Points { rms, .. } = how {
        answer["rms"] = json!(rms);
    }
    Ok(answer)
}

fn by_datums(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let (scan, how, after) = prepare_datums(&Args(args), ctx, false)?;
    record_alignment(ctx, scan, how, after).map_err(CommandError::failed)
}

/// The scan `args` names, with its mesh and the kernel to align it with.
fn scan_to_align(
    args: &CommandArgs,
    ctx: &WorkbenchRuntimeContext,
) -> Result<
    (
        BodyId,
        std::sync::Arc<kernel_api::TriMesh>,
        &'static dyn kernel_api::KernelQueries,
    ),
    CommandError,
> {
    let scan = BodyId(Args(args).id("scan")?);
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
    Ok((scan, mesh.0, kernel))
}

fn automatic(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let (scan, mesh, kernel) = scan_to_align(args, ctx)?;
    let proposal = kernel
        .orient(&mesh)
        .map_err(|e| CommandError::failed(e.to_string()))?;
    let after = align::automatic(&proposal);
    let how = align::How::Auto {
        said: proposal.said,
    };
    record_alignment(ctx, scan, how, after).map_err(CommandError::failed)
}

fn by_symmetry(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let (scan, mesh, kernel) = scan_to_align(args, ctx)?;
    let plane = kernel
        .symmetry_plane(&mesh)
        .map_err(|e| CommandError::failed(e.to_string()))?;
    let after =
        align::symmetry(ctx.document.body_placement(scan), &plane).map_err(CommandError::failed)?;
    record_alignment(ctx, scan, align::How::Symmetry { rms: plane.rms }, after)
        .map_err(CommandError::failed)
}

/// The placement bringing the scan's picked points onto the reference's.
fn points(
    a: &Args,
    ctx: &WorkbenchRuntimeContext,
    scan: BodyId,
) -> Result<(align::How, core_document::BodyPlacement), CommandError> {
    let pairs = match a.0.get("pairs") {
        Some(Value::Array(list)) => list.clone(),
        _ => return Err(CommandError::bad("pairs", "lists {scan, reference} pairs")),
    };
    let xyz = |v: Option<&Value>| {
        let v =
            v.ok_or_else(|| CommandError::bad("pairs", "each has a scan and a reference point"))?;
        core_document::command::vec3_of(Some(v), "pairs")
    };
    // The scan's points in its own frame: the placement found is where
    // that frame goes.
    let back = ctx.document.body_placement(scan).inverse().rows();
    let local = |p: [f64; 3]| -> [f64; 3] {
        std::array::from_fn(|r| {
            back[r][0] * p[0] + back[r][1] * p[1] + back[r][2] * p[2] + back[r][3]
        })
    };
    let mut from = Vec::new();
    let mut to = Vec::new();
    for pair in &pairs {
        from.push(local(xyz(pair.get("scan"))?));
        to.push(xyz(pair.get("reference"))?);
    }
    let fit = metrology::rigid::rigid(&from, &to).ok_or_else(|| {
        CommandError::bad(
            "pairs",
            "needs three or more pairs, the scan's points not on one line",
        )
    })?;
    let (m, t) = (fit.rotation, fit.translation);
    let after = core_document::BodyPlacement::from_rows(&[
        [m[0][0], m[0][1], m[0][2], t[0]],
        [m[1][0], m[1][1], m[1][2], t[1]],
        [m[2][0], m[2][1], m[2][2], t[2]],
        [0.0, 0.0, 0.0, 1.0],
    ])
    .ok_or_else(|| CommandError::failed("the fit is not a rigid motion"))?;
    Ok((
        align::How::Points {
            pairs: from.len(),
            rms: fit.rms,
        },
        after,
    ))
}
