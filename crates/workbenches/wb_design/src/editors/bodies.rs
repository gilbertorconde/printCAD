//! The task panels of features that start a body or make it from
//! another: the base shape, Clone, Split, Piece, borrowed geometry and
//! the body boolean.

use bench_api::{Select, Tone};
use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};

use crate::feature::{DesignFeature, MirrorPlane};
use crate::form::Form;

use super::borrow;
use super::{
    Formulas, LABEL_BLANK, choice_shown, mono_value, plane_editor, plane_references, signed_mm,
    small_button, taper_note,
};

/// The Base, Clone, Split, Piece, Borrow and Body boolean editors; `true`
/// when it changed the feature. Any other kind is left alone.
pub(super) fn body_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    feature_id: FeatureId,
    feature: &mut DesignFeature,
) -> bool {
    let mut changed = false;
    match feature {
        DesignFeature::Base {} => {
            let from = ctx.document.base_geometry(body);
            let health = from.and_then(|g| g.health.as_ref());
            taper_note(
                f,
                "The shape this body starts from, as it was imported or converted. \
                 The features after it change it.",
            );
            if let Some(health) = health.filter(|h| h.is_broken()) {
                taper_note(
                    f,
                    &format!(
                        "The checker found {} problem(s) in it; Repair shape mends it.",
                        health.broken
                    ),
                );
            }
        }
        DesignFeature::Clone { source } => {
            let bodies: Vec<(BodyId, String)> = ctx
                .document
                .bodies()
                .iter()
                .filter(|b| b.id != body)
                .map(|b| (b.id, b.name.clone()))
                .collect();
            let current = bodies
                .iter()
                .find(|(id, _)| id == source)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| "(pick body)".into());
            let at = bodies.iter().position(|(id, _)| id == source);
            let names = bodies.iter().map(|(_, n)| n.clone()).collect();
            if let Some(i) = choice_shown(f, "Source body", current, names, at) {
                *source = bodies[i].0;
                changed = true;
            }
        }
        DesignFeature::Split {
            plane,
            offset,
            sketch,
            other_side,
            split: _,
        } => {
            changed |= split_editor(f, ctx, fx, body, plane, offset, sketch, other_side);
        }
        DesignFeature::Piece { near } => {
            f.flow("Near", false, |f| {
                let near = format!("{:.2}, {:.2}, {:.2}", near[0], near[1], near[2]);
                f.caption(&near, Tone::Neutral, true, true);
            });
            taper_note(
                f,
                "Of the solid's pieces before it, this keeps the one nearest the point: the \
                 piece this body was separated as. Editing the body it came from rebuilds it.",
            );
        }
        DesignFeature::Borrow {
            source,
            frozen,
            options,
        } => {
            changed |= borrow::borrow_editor(f, ctx, body, feature_id, source, frozen, options);
        }
        DesignFeature::BodyBoolean {
            tool_body,
            kind,
            more_tools,
            refine: _,
            boolean_tolerance: _,
        } => {
            let bodies: Vec<(BodyId, String)> = ctx
                .document
                .bodies()
                .iter()
                .filter(|b| b.id != body)
                .map(|b| (b.id, b.name.clone()))
                .collect();
            let current = bodies
                .iter()
                .find(|(id, _)| id == tool_body)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| "(pick body)".into());
            let at = bodies.iter().position(|(id, _)| id == tool_body);
            let names = bodies.iter().map(|(_, n)| n.clone()).collect();
            if let Some(i) = choice_shown(f, "Tool body", current, names, at) {
                *tool_body = bodies[i].0;
                changed = true;
            }
            // Further tools, taken the same way in turn.
            let mut remove = None;
            for (i, tool) in more_tools.iter().enumerate() {
                let name = bodies
                    .iter()
                    .find(|(id, _)| id == tool)
                    .map(|(_, n)| n.clone())
                    .unwrap_or_else(|| "(gone)".into());
                f.flow(LABEL_BLANK, false, |f| {
                    mono_value(f, &name, true);
                    if small_button(f, "×", "") {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                more_tools.remove(i);
                changed = true;
            }
            let more: Vec<&(BodyId, String)> = bodies
                .iter()
                .filter(|(id, _)| id != tool_body && !more_tools.contains(id))
                .collect();
            let names = more.iter().map(|(_, n)| n.clone()).collect();
            if let Some(i) = choice_shown(f, "Add tool", "Another body…".into(), names, None) {
                more_tools.push(more[i].0);
                changed = true;
            }
            let kinds = [
                (kernel_api::BoolKind::Fuse, "Fuse"),
                (kernel_api::BoolKind::Cut, "Cut"),
                (kernel_api::BoolKind::Common, "Common"),
            ];
            let picked = f.select(
                "Operation",
                Select {
                    label: "Operation".into(),
                    options: kinds.iter().map(|(_, l)| l.to_string()).collect(),
                    selected: kinds
                        .iter()
                        .position(|(k, _)| k == kind)
                        .unwrap_or(kinds.len()),
                    segmented: true,
                    ..Default::default()
                },
            );
            if let Some(i) = picked {
                *kind = kinds[i].0;
                changed = true;
            }
        }
        _ => {}
    }
    changed
}

/// A split piece's cut: a plane of the body split (or a sketch of it
/// carried through the solid) and the side kept. The other pieces of the
/// split follow the cut (`split::sync`).
#[allow(clippy::too_many_arguments)]
fn split_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    plane: &mut MirrorPlane,
    offset: &mut f32,
    sketch: &mut Option<FeatureId>,
    other_side: &mut bool,
) -> bool {
    let mut changed = false;
    let source = crate::split::source_of(ctx.document, body);
    let sketches = source
        .map(|source| crate::build::sketches_of_body(ctx.document, source))
        .unwrap_or_default();
    let shown = sketch
        .and_then(|id| sketches.iter().find(|(s, _)| *s == id))
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| "A plane".to_string());
    let mut options = vec!["A plane".to_string()];
    options.extend(sketches.iter().map(|(_, n)| n.clone()));
    let at = match sketch {
        None => Some(0),
        Some(id) => sketches.iter().position(|(s, _)| s == id).map(|i| i + 1),
    };
    match choice_shown(f, "Cut by", shown, options, at) {
        Some(0) => {
            *sketch = None;
            changed = true;
        }
        Some(i) => {
            *sketch = Some(sketches[i - 1].0);
            changed = true;
        }
        None => {}
    }
    if sketch.is_none() {
        let references = source
            .map(|source| plane_references(ctx, source))
            .unwrap_or_default();
        changed |= plane_editor(f, ctx, plane, &references);
        changed |= signed_mm(f, fx, offset, "Offset", "offset");
    }
    let side = if sketch.is_some() {
        "Keep what lies outside the profile"
    } else {
        "Keep what lies behind the plane"
    };
    changed |= f.check(other_side, side, "");
    taper_note(
        f,
        "Every piece of the split cuts the same way and keeps its own side; a change \
         here moves the cut of them all. Editing the body split rebuilds the pieces.",
    );
    changed
}
