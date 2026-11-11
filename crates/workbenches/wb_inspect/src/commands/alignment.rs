//! Moving a scan: best fit to a reference, by numbers, and back.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, ParamKind,
    WorkbenchContext, WorkbenchRuntimeContext,
};
use serde_json::Value;

use crate::{
    BEST_FIT, MOVE_COMMAND, RESET_ALIGNMENT, align, best_fit_how, compare, record_alignment,
};

pub(super) fn owns(id: &str) -> bool {
    matches!(id, BEST_FIT | MOVE_COMMAND | RESET_ALIGNMENT)
}

pub(super) fn configure(context: &mut WorkbenchContext) {
    context.register_command(
        CommandSpec::new(BEST_FIT, "Move a scan onto a reference body, best fit")
            .param("scan", ParamKind::Id, "The scan; only it moves")
            .param("reference", ParamKind::Id, "The body it should match")
            .optional(
                "search",
                ParamKind::Bool,
                "Also try every way of matching the scan's principal axes with the \
                     reference's, for a scan nowhere near; true when left out",
            )
            .optional(
                "marked",
                ParamKind::Bool,
                "A local fine fit: only the scan's marked part is fitted (a datum face, a \
                     region the part should sit on), the search off unless asked",
            )
            .returns("{alignment, name, translation, rotation, rms, inliers, free}")
            .note(
                "Rigid and point to plane, pairs past three times the median distance left \
                     out. `free` counts the motions the fit could not pin down (a cylinder \
                     turning about its axis): above 0 the scan may sit anywhere along them. The \
                     scan's placement changes in one undoable step and an alignment row records \
                     how.",
            ),
    );
    context.register_command(
        CommandSpec::new(MOVE_COMMAND, "Move and turn a scan by numbers")
            .param("scan", ParamKind::Id, "The scan")
            .optional("translate", ParamKind::Any, "{x, y, z} in mm")
            .optional(
                "rotate",
                ParamKind::Any,
                "{x, y, z} in degrees: about X, then Y, then Z, through the origin",
            )
            .returns("{alignment, name, translation, rotation}")
            .note("The motion is applied to where the scan sits now."),
    );
    context.register_command(
        CommandSpec::new(
            RESET_ALIGNMENT,
            "Put a scan back where the scanner placed it",
        )
        .param("scan", ParamKind::Id, "The scan")
        .returns("{alignment, name, translation, rotation}"),
    );
}

pub(super) fn run(
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    match id {
        BEST_FIT => {
            let scan = BodyId(a.id("scan")?);
            let reference = BodyId(a.id("reference")?);
            let marked = a.opt_bool("marked")? == Some(true);
            let search = a.opt_bool("search")?.unwrap_or(!marked);
            let keep = if marked {
                let marking = ctx
                    .current_marking()
                    .filter(|m| m.body == scan)
                    .ok_or_else(|| CommandError::failed("mark the part of the scan to fit on"))?;
                Some(marking.vertices.clone())
            } else {
                None
            };
            let kernel = ctx
                .kernel
                .ok_or_else(|| CommandError::failed("no kernel to fit with"))?;
            let inputs =
                compare::inputs_for(ctx.document, scan, reference).map_err(CommandError::failed)?;
            let placed = ctx.document.body_placement(reference);
            let (after, fit) = align::best_fit_of(
                kernel,
                &inputs,
                placed,
                search,
                keep.as_deref().map(Vec::as_slice),
            )
            .map_err(CommandError::failed)?;
            let mut how = best_fit_how(reference, search, &fit);
            if let align::How::BestFit { marked: m, .. } = &mut how {
                *m = marked;
            }
            record_alignment(ctx, scan, how, after).map_err(CommandError::failed)
        }
        MOVE_COMMAND => {
            let scan = BodyId(a.id("scan")?);
            let triple = |name: &str| -> Result<[f64; 3], CommandError> {
                let v = a.0.get(name).cloned().unwrap_or(Value::Null);
                if v.is_null() {
                    return Ok([0.0; 3]);
                }
                let get = |k: &str, i: usize| {
                    v.get(k)
                        .or_else(|| v.get(i))
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0)
                };
                Ok([get("x", 0), get("y", 1), get("z", 2)])
            };
            let (translate, rotate_deg) = (triple("translate")?, triple("rotate")?);
            let after =
                align::by_numbers(translate, rotate_deg).after(&ctx.document.body_placement(scan));
            record_alignment(
                ctx,
                scan,
                align::How::Numbers {
                    translate,
                    rotate_deg,
                },
                after,
            )
            .map_err(CommandError::failed)
        }
        RESET_ALIGNMENT => {
            let scan = BodyId(a.id("scan")?);
            record_alignment(
                ctx,
                scan,
                align::How::Reset,
                core_document::BodyPlacement::IDENTITY,
            )
            .map_err(CommandError::failed)
        }
        _ => Err(CommandError::Unknown(id.to_string())),
    }
}
