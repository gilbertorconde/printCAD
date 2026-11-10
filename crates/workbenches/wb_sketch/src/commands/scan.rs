//! Scans and measured sizes: candidate values, design values and fitting a
//! scan's section.

use core_document::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, FeatureId, ParamKind,
    WorkbenchRuntimeContext,
};
use serde_json::{Value, json};

use super::{ids_of, load, made_since, save, sketch};

/// Whether `id` is one of these commands.
pub(super) fn owns(id: &str) -> bool {
    matches!(
        id,
        "sketch.candidates" | "sketch.design_values" | "sketch.fit_scan"
    )
}

/// These commands' specs.
pub(super) fn configure(specs: &mut Vec<CommandSpec>) {
    specs.push(
        sketch(CommandSpec::new(
            "sketch.candidates",
            "What the sketch's geometry was probably designed at, and the relations it nearly keeps",
        ))
        .returns("{sizes = {{element, kind, measured, design}}, relations = {{kind, elements, offset}}}")
        .read_only()
        .note(
            "Each unsized line's length and arc's or circle's radius comes with its probable \
             design value: halves under 10 mm, whole millimetres above, radii to 0.5 mm. \
             Relations are horizontal, vertical, parallel, perpendicular, equal and concentric, \
             each within 2° or 1 % (at least 0.05 mm) and not yet constrained; `offset` is how \
             far from keeping it, in degrees or mm. Nothing is applied: `sketch.design_values` \
             holds sizes, `sketch.constrain` adds a relation.",
        )
        .see_also("sketch.design_values")
        .see_also("sketch.fit_scan"),
    );
    specs.push(
        sketch(CommandSpec::new(
            "sketch.design_values",
            "Hold lines and rounds at their probable design values with driving dimensions",
        ))
        .optional(
            "elements",
            ParamKind::List,
            "The lines, arcs and circles; every unsized one when left out",
        )
        .returns("{constraints}: the dimensions added")
        .see_also("sketch.candidates"),
    );
    specs.push(
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
        .returns("{chains, lines, arcs, circles, failed, elements}")
        .note(
            "Each chain of the section becomes lines and arcs end to end, closed where the \
             slice closes, ends that meet sharing one point, so a closed slice is a profile. \
             Chains shorter than 1 mm are left out. The scan's section shows as a dashed \
             guide while the sketch is open.",
        )
        .see_also("sketch.intersection"),
    );
}

/// Run command `id`, one of these.
pub(super) fn run(
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let sketch_id = FeatureId(a.id("sketch")?);
    let mut feature = load(ctx, sketch_id)?;
    match id {
        "sketch.candidates" => {
            let s = &feature.sketch;
            let id = |u: &uuid::Uuid| json!(u.to_string());
            return Ok(json!({
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
            }));
        }
        "sketch.design_values" => {
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
            let answer =
                json!({"constraints": added.iter().map(|u| u.to_string()).collect::<Vec<_>>()});
            return save(ctx, sketch_id, feature, answer);
        }
        "sketch.fit_scan" => {
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
            let (added, failed) =
                crate::scan::fit_into(&mut feature.sketch, kernel, &chains, fitting);
            let made = made_since(&feature.sketch, &before);
            let answer = json!({
                "chains": chains.len(),
                "lines": added.lines,
                "arcs": added.arcs,
                "circles": added.circles,
                "failed": failed,
                "elements": made.get("elements").cloned().unwrap_or(Value::Null),
            });
            return save(ctx, sketch_id, feature, answer);
        }
        _ => {}
    }
    Err(CommandError::Unknown(id.to_string()))
}
