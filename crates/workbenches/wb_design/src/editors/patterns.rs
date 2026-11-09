//! The fields a circular or path pattern shares with its multi-transform
//! step: rings about an axis, and a path with the spacing along it.

use bench_api::{ButtonStyle, Tone};
use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};

use super::{
    Formulas, choice_shown, coords, count_drag, mm_drag, mono_value, pattern_axis_editor,
    picked_edges, small_button, use_button,
};
use crate::feature::{EdgePick, PathSpacing, PatternAxis, PatternPath};
use crate::form::Form;

/// Concentric rings: the axis they go round, the direction they step out
/// along, the spacing between them and each ring's count. `rings_named`
/// gives each count its formula (`ring1`, …) where the feature lists one.
pub(super) fn rings_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    (body, feature_id): (BodyId, FeatureId),
    (axis, outward): (&mut PatternAxis, &mut PatternAxis),
    (spacing, counts): (&mut f32, &mut Vec<u32>),
    rings_named: bool,
) -> bool {
    let mut changed = pattern_axis_editor(f, ctx, (body, feature_id), axis);
    f.caption("Rings step out along", Tone::Neutral, true, false);
    changed |= pattern_axis_editor(f, ctx, (body, feature_id), outward);
    changed |= mm_drag(f, fx, spacing, "Ring spacing:");
    let mut drop = None;
    for (ring, count) in counts.iter_mut().enumerate() {
        f.flow("", false, |f| {
            let label = format!("Ring {}:", ring + 1);
            let named = rings_named
                .then(|| fx.show_named(f, &label, &format!("ring{}", ring + 1), f64::from(*count)))
                .flatten();
            match named {
                Some((edited, v)) => {
                    if edited {
                        *count = v.round().clamp(1.0, 1000.0) as u32;
                    }
                    changed |= edited;
                }
                None => changed |= count_drag(f, fx, count, &label),
            }
            if ring > 0 && small_button(f, "×", "") {
                drop = Some(ring);
            }
        });
    }
    if let Some(ring) = drop {
        counts.remove(ring);
        changed = true;
    }
    if small_button(f, "+ Ring", "") {
        let last = counts.last().copied().unwrap_or(6);
        counts.push(last + 6);
        changed = true;
    }
    changed
}

/// What a path pattern follows (a sketch's chain of curves, or an edge
/// picked in the view) and how its occurrences are spaced along it.
pub(super) fn path_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    (body, feature_id): (BodyId, FeatureId),
    path: &mut PatternPath,
    layout: &mut PathSpacing,
) -> bool {
    let mut changed = false;
    let sketches = crate::build::sketch_choices(ctx.document, body, feature_id, path.reference());
    let edge = picked_edges(ctx).first().map(EdgePick::of);
    let shown = match path {
        PatternPath::Sketch { sketch, .. } => sketches
            .iter()
            .find(|(id, _)| id == sketch)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| "(pick)".into()),
        PatternPath::Edge(_) => "Picked edge".into(),
    };
    let mut options: Vec<String> = sketches.iter().map(|(_, n)| n.clone()).collect();
    options.push("Picked edge".to_string());
    let at = match path {
        PatternPath::Edge(_) => Some(sketches.len()),
        PatternPath::Sketch { .. } => sketches
            .iter()
            .position(|(id, _)| path.reference() == Some(*id)),
    };
    match choice_shown(f, "Path", shown, options, at) {
        Some(i) if i < sketches.len() => {
            *path = PatternPath::Sketch {
                sketch: sketches[i].0,
                chain: 0,
            };
            changed = true;
        }
        Some(_) => {
            *path = PatternPath::Edge(edge.unwrap_or(EdgePick {
                faces: [0, 0],
                point: [0.0; 3],
                direction: [0.0; 3],
            }));
            changed = true;
        }
        None => {}
    }
    match path {
        PatternPath::Sketch { sketch, chain } => {
            let chains = crate::build::sketch_chain_count(ctx.document, *sketch);
            if chains > 1 {
                let set = f.quantity_hinted(
                    bench_api::Quantity {
                        label: "Chain".into(),
                        value: (*chain + 1) as f64,
                        min: Some(1.0),
                        max: Some(chains as f64),
                        speed: Some(1.0),
                        decimals: 0,
                        bare: true,
                        ..Default::default()
                    },
                    "Which of the sketch's chains of curves to follow",
                );
                if let Some(n) = set {
                    *chain = (n.round() as usize).clamp(1, chains) - 1;
                    changed = true;
                }
            }
        }
        PatternPath::Edge(pick) => {
            f.flow("Edge", true, |f| {
                mono_value(f, &coords(pick.point), true);
                if use_button(
                    f,
                    "Use selected edge",
                    edge.is_some(),
                    "Click an edge of the solid in the viewport, then press this",
                ) && let Some(picked) = edge
                {
                    *pick = picked;
                    changed = true;
                }
            });
        }
    }
    changed |= count_drag(f, fx, &mut layout.occurrences, "Occurrences:");
    changed |= f.check(
        &mut layout.spacing_mode,
        "Spaced apart",
        "On: the distance between neighbours along the path. \
         Off: the occurrences spread over the whole path",
    );
    if layout.spacing_mode {
        changed |= mm_drag(f, fx, &mut layout.spacing, "Spacing:");
    }
    changed |= f.check(&mut layout.reversed, "From the path's end", "");
    changed |= f.check(
        &mut layout.align,
        "Turn with the path",
        "Each copy turns as the path's direction does from its start",
    );
    changed
}

/// The button adding a path step to a multi-transform: along the body's
/// first sketch with curves, else the selected edge.
pub(super) fn add_path_step(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
) -> Option<PatternPath> {
    let path = crate::build::sketches_of_body(ctx.document, body)
        .into_iter()
        .find(|(id, _)| crate::build::sketch_chain_count(ctx.document, *id) > 0)
        .map(|(sketch, _)| PatternPath::Sketch { sketch, chain: 0 })
        .or_else(|| {
            picked_edges(ctx)
                .first()
                .map(|e| PatternPath::Edge(EdgePick::of(e)))
        })?;
    f.button("+ Path", ButtonStyle::Secondary, true, "")
        .then_some(path)
}
