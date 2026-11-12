//! Scans and measured sizes: candidate values, design values and fitting a
//! scan's section.

use core_document::{
    Args, CommandError, CommandResult, CommandSpec, CommandTable, FeatureId, ParamKind,
    WorkbenchRuntimeContext,
};
use serde_json::{Value, json};

use super::{editing, ids_of, made_since, reading, sketch};
use crate::SketchWorkbench;
use crate::feature::SketchFeature;

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<SketchWorkbench>) {
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.candidates",
            "What the sketch's geometry was probably designed at, and the relations it nearly keeps",
        ))
        .returns("{sizes = {{element, kind, measured, design}}, relations = {{kind, elements, offset}}}")
        .read_only(),
        reading(candidates),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.design_values",
            "Hold lines and rounds at their probable design values with driving dimensions",
        ))
        .optional(
            "elements",
            ParamKind::List,
            "The lines, arcs and circles; every unsized one when left out",
        )
        .returns("{constraints}: the dimensions added"),
        editing(design_values),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.fit_scan",
            "Fit lines and arcs to where every visible scan crosses the sketch plane",
        ))
        .optional(
            "tolerance",
            ParamKind::Number,
            "Every scan point lies within this of its piece, mm; 0.1",
        )
        .optional(
            "corners",
            ParamKind::Bool,
            "Bring back the sharp corners the scan rounds off; true",
        )
        .optional(
            "axis_snap",
            ParamKind::Number,
            "Turn lines within this many degrees of the sketch's axes onto them; 3, 0 for none",
        )
        .optional(
            "min_bow",
            ParamKind::Number,
            "An arc bowing less than this from its chord is a line, mm; 0.2",
        )
        .optional(
            "construction",
            ParamKind::Bool,
            "Make them construction, guides to draw over; false",
        )
        .returns("{chains, lines, arcs, circles, failed, elements}"),
        editing(fit_scan),
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "scan.md", include_str!("scan.md"), &[])
}

fn candidates(
    _a: &Args,
    _sketch_id: FeatureId,
    feature: SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let s = &feature.sketch;
    let id = |u: &uuid::Uuid| json!(u.to_string());
    Ok(json!({
        "sizes": crate::candidates::sizes(s).iter().map(|z| json!({
            "element": id(&z.element),
            "kind": z.kind,
            "measured": z.measured,
            "design": z.design,
        })).collect::<Vec<_>>(),
        "relations": crate::candidates::relations(s).iter().map(|r| json!({
            "kind": r.kind,
            "elements": r.elements.iter().map(id).collect::<Vec<_>>(),
            "offset": r.offset,
        })).collect::<Vec<_>>(),
    }))
}

fn design_values(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let elements: Vec<uuid::Uuid> = match args.get("elements") {
        Some(Value::Array(list)) => list
            .iter()
            .map(|v| {
                v.as_str()
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| CommandError::bad("elements", "are element ids"))
            })
            .collect::<Result<_, _>>()?,
        _ => Vec::new(),
    };
    let added = crate::candidates::hold_at_design(&mut feature.sketch, &elements);
    if added.is_empty() {
        return Err(CommandError::failed(
            "no unsized line, arc or circle to hold",
        ));
    }
    let answer = json!({"constraints": added.iter().map(|u| u.to_string()).collect::<Vec<_>>()});
    Ok(answer)
}

fn fit_scan(
    a: &Args,
    sketch_id: FeatureId,
    feature: &mut SketchFeature,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("there is no kernel to fit with"))?;
    let placed = crate::placed_plane(
        &feature.plane,
        &crate::sketch_placement(ctx.document, sketch_id),
    );
    let chains = crate::scan::chains(ctx.document, kernel, &placed);
    if chains.is_empty() {
        return Err(CommandError::failed(
            "no visible scan crosses the sketch plane",
        ));
    }
    let tolerance = a.opt_number("tolerance")?.unwrap_or(0.1);
    if !(tolerance > 0.0 && tolerance.is_finite()) {
        return Err(CommandError::bad("tolerance", "must be more than zero"));
    }
    let snap = a.opt_number("axis_snap")?.unwrap_or(3.0);
    let fitting = crate::scan::Fitting {
        fit: kernel_api::ChainFit {
            tolerance,
            corners: a.opt_bool("corners")?.unwrap_or(true),
            axis_snap_deg: (snap > 0.0).then_some(snap.min(90.0)),
            min_bow: a.opt_number("min_bow")?.unwrap_or(0.2).max(0.0),
            closed: false,
        },
        construction: a.opt_bool("construction")?.unwrap_or(false),
    };
    let before = ids_of(&feature.sketch);
    let (added, failed) = crate::scan::fit_into(&mut feature.sketch, kernel, &chains, fitting);
    let made = made_since(&feature.sketch, &before);
    let answer = json!({
        "chains": chains.len(),
        "lines": added.lines,
        "arcs": added.arcs,
        "circles": added.circles,
        "failed": failed,
        "elements": made.get("elements").cloned().unwrap_or(Value::Null),
    });
    Ok(answer)
}
