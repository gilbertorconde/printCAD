//! The fields a circular or path pattern shares with its multi-transform
//! step: rings about an axis, and a path with the spacing along it; and a
//! fill pattern's region and lattice.

use bench_api::{ButtonStyle, Tone};
use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};

use super::transforms::pattern_axis_editor;
use super::{
    Bounds, Formulas, Kind, choice_shown, coords, count_drag, deg_drag, mm_drag, mono_value,
    picked_edges, picked_face, quantity, regions_rows, set_f32, small_button, taper_note,
    use_button,
};
use crate::feature::{
    EdgePick, FacePick, FillArea, FillKind, PathSpacing, PatternAxis, PatternPath,
};
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
    changed |= mm_drag(f, fx, spacing, ("Ring spacing:", "ring_spacing"));
    let mut drop = None;
    for (ring, count) in counts.iter_mut().enumerate() {
        f.flow("", false, |f| {
            let label = format!("Ring {}:", ring + 1);
            let named = rings_named
                .then(|| {
                    let name = format!("ring{}", ring + 1);
                    let bounds = super::Bounds::within(1.0, 1000.0);
                    fx.show_named_within(f, &label, &name, f64::from(*count), bounds)
                })
                .flatten();
            match named {
                Some((edited, v)) => {
                    if edited {
                        *count = v.round().clamp(1.0, 1000.0) as u32;
                    }
                    changed |= edited;
                }
                None => changed |= count_drag(f, fx, count, (&label, "")),
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
    changed |= count_drag(
        f,
        fx,
        &mut layout.occurrences,
        ("Occurrences:", "occurrences"),
    );
    changed |= f.check(
        &mut layout.spacing_mode,
        "Spaced apart",
        "On: the distance between neighbours along the path. \
         Off: the occurrences spread over the whole path",
    );
    if layout.spacing_mode {
        changed |= mm_drag(f, fx, &mut layout.spacing, ("Spacing:", "spacing"));
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

/// A fill pattern's numbers and switches, beside its region.
pub(super) struct FillFields<'a> {
    pub lattice: &'a mut FillKind,
    pub spacing: &'a mut f32,
    pub row_spacing: &'a mut f32,
    pub angle_deg: &'a mut f32,
    pub margin: &'a mut f32,
    pub clip: &'a mut bool,
}

/// What a fill pattern covers (a sketch's area, its regions chosen, or a
/// face picked in the view) and the lattice it fills it with.
pub(super) fn fill_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    (body, feature_id): (BodyId, FeatureId),
    region: &mut FillArea,
    fields: FillFields,
) -> bool {
    let mut changed = false;
    let current = match region {
        FillArea::Sketch { sketch, .. } => Some(*sketch),
        FillArea::Face(_) => None,
    };
    let sketches = crate::build::sketch_choices(ctx.document, body, feature_id, current);
    let face = picked_face(ctx).map(FacePick::of);
    let shown = match region {
        FillArea::Sketch { sketch, .. } => sketches
            .iter()
            .find(|(id, _)| id == sketch)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| "(pick)".into()),
        FillArea::Face(_) => region.label().into(),
    };
    let mut options: Vec<String> = sketches.iter().map(|(_, n)| n.clone()).collect();
    options.push("Picked face".to_string());
    let at = match region {
        FillArea::Face(_) => Some(sketches.len()),
        FillArea::Sketch { .. } => sketches.iter().position(|(id, _)| current == Some(*id)),
    };
    match choice_shown(f, "Region", shown, options, at) {
        Some(i) if i < sketches.len() => {
            *region = FillArea::Sketch {
                sketch: sketches[i].0,
                regions: Vec::new(),
            };
            changed = true;
        }
        Some(_) => {
            *region = FillArea::Face(face.unwrap_or(FacePick {
                point: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
                name: Default::default(),
            }));
            changed = true;
        }
        None => {}
    }
    match region {
        FillArea::Sketch { sketch, regions } => {
            changed |= regions_rows(f, ctx, *sketch, regions);
        }
        FillArea::Face(pick) => {
            f.flow("Face", true, |f| {
                mono_value(f, &coords(pick.point), true);
                if use_button(
                    f,
                    "Use selected face",
                    face.is_some(),
                    "Click a flat face of the solid in the viewport, then press this",
                ) && let Some(picked) = face
                {
                    *pick = picked;
                    changed = true;
                }
            });
        }
    }
    let kinds: Vec<String> = FillKind::ALL
        .iter()
        .map(|k| k.label().to_string())
        .collect();
    let now = FillKind::ALL.iter().position(|k| k == fields.lattice);
    if let Some(i) = choice_shown(f, "Lattice", fields.lattice.label().into(), kinds, now) {
        *fields.lattice = FillKind::ALL[i];
        changed = true;
    }
    match fields.lattice {
        FillKind::Grid => {
            changed |= mm_drag(f, fx, fields.spacing, ("Spacing:", "spacing"));
            changed |= mm_drag(f, fx, fields.row_spacing, ("Row spacing:", "row_spacing"));
        }
        FillKind::Honeycomb => {
            changed |= mm_drag(f, fx, fields.spacing, ("Spacing:", "spacing"));
        }
    }
    changed |= deg_drag(f, fx, fields.angle_deg, ("Angle:", "angle"), -360.0..=360.0);
    changed |= margin_drag(f, fx, fields.margin);
    changed |= f.check(
        fields.clip,
        "Trim at the edge",
        "On: copies the region's edge passes through are trimmed to it. \
         Off: only copies lying whole inside are made",
    );
    taper_note(
        f,
        &format!(
            "The lattice passes through the original, which holds one of its places. At \
             most {} copies.",
            kernel_api::MAX_FILL_COPIES
        ),
    );
    changed
}

/// The margin: a length from zero up.
fn margin_drag(f: &mut Form, fx: &mut Formulas, margin: &mut f32) -> bool {
    let value = f64::from(*margin);
    let set = match fx.show_within(f, ("Margin:", "margin"), value, Bounds::within(0.0, 1.0e6)) {
        Some((true, v)) => Some(v),
        Some((false, _)) => None,
        None => {
            let mut q = quantity("Margin:", value, Kind::Mm, 0.1);
            q.min = Some(0.0);
            f.quantity(q)
        }
    };
    set_f32(margin, set.map(|v| v.max(0.0)))
}
