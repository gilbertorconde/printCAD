//! The fields a circular or path pattern shares with its multi-transform
//! step: rings about an axis, and a path with the spacing along it.

use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};
use egui::{RichText, Ui};
use ui_kit::sans;
use ui_kit::tokens::*;
use ui_kit::widgets::{
    accent_outline_button, check_row, mono_label, secondary_button, small_secondary_button,
};

use super::{Formulas, count_drag, label_cell, mm_drag, pattern_axis_editor, picked_edges};
use crate::feature::{EdgePick, PathSpacing, PatternAxis, PatternPath};

/// Concentric rings: the axis they go round, the direction they step out
/// along, the spacing between them and each ring's count. `rings_named`
/// gives each count its formula (`ring1`, …) where the feature lists one.
pub(super) fn rings_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    (body, feature_id): (BodyId, FeatureId),
    (axis, outward): (&mut PatternAxis, &mut PatternAxis),
    (spacing, counts): (&mut f32, &mut Vec<u32>),
    (salt, rings_named): (usize, bool),
) -> bool {
    let mut changed = pattern_axis_editor(
        ui,
        ctx,
        (body, feature_id),
        axis,
        ("rings_axis", feature_id, salt),
    );
    ui.label(
        RichText::new("Rings step out along")
            .font(sans(FONT_XS))
            .color(TEXT2),
    );
    changed |= pattern_axis_editor(
        ui,
        ctx,
        (body, feature_id),
        outward,
        ("rings_out", feature_id, salt),
    );
    changed |= mm_drag(ui, fx, spacing, "Ring spacing:");
    let mut drop = None;
    for (ring, count) in counts.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let label = format!("Ring {}:", ring + 1);
            let named = rings_named
                .then(|| fx.show_named(ui, &label, &format!("ring{}", ring + 1), f64::from(*count)))
                .flatten();
            match named {
                Some((edited, v)) => {
                    *count = v.round().clamp(1.0, 1000.0) as u32;
                    changed |= edited;
                }
                None => changed |= count_drag(ui, fx, count, &label),
            }
            if ring > 0 && small_secondary_button(ui, "×").clicked() {
                drop = Some(ring);
            }
        });
    }
    if let Some(ring) = drop {
        counts.remove(ring);
        changed = true;
    }
    if small_secondary_button(ui, "+ Ring").clicked() {
        let last = counts.last().copied().unwrap_or(6);
        counts.push(last + 6);
        changed = true;
    }
    changed
}

/// What a path pattern follows (a sketch's chain of curves, or an edge
/// picked in the view) and how its occurrences are spaced along it.
pub(super) fn path_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    (body, feature_id): (BodyId, FeatureId),
    path: &mut PatternPath,
    layout: &mut PathSpacing,
    salt: usize,
) -> bool {
    let mut changed = false;
    let sketches = crate::build::sketch_choices(ctx.document, body, feature_id, path.reference());
    let edge = picked_edges(ctx).first().map(EdgePick::of);
    ui.horizontal(|ui| {
        label_cell(ui, "Path");
        let shown = match path {
            PatternPath::Sketch { sketch, .. } => sketches
                .iter()
                .find(|(id, _)| id == sketch)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| "(pick)".into()),
            PatternPath::Edge(_) => "Picked edge".into(),
        };
        egui::ComboBox::from_id_salt(("path_source", feature_id, salt))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (id, name) in &sketches {
                    let current = path.reference() == Some(*id);
                    if ui.selectable_label(current, name).clicked() && !current {
                        *path = PatternPath::Sketch {
                            sketch: *id,
                            chain: 0,
                        };
                        changed = true;
                    }
                }
                let on_edge = matches!(path, PatternPath::Edge(_));
                if ui.selectable_label(on_edge, "Picked edge").clicked() && !on_edge {
                    *path = PatternPath::Edge(edge.unwrap_or(EdgePick {
                        faces: [0, 0],
                        point: [0.0; 3],
                        direction: [0.0; 3],
                    }));
                    changed = true;
                }
            });
    });
    match path {
        PatternPath::Sketch { sketch, chain } => {
            let chains = crate::build::sketch_chain_count(ctx.document, *sketch);
            if chains > 1 {
                let mut n = (*chain + 1) as u32;
                ui.horizontal(|ui| {
                    label_cell(ui, "Chain");
                    if ui
                        .add(egui::DragValue::new(&mut n).range(1..=chains as u32))
                        .on_hover_text("Which of the sketch's chains of curves to follow")
                        .changed()
                    {
                        *chain = n as usize - 1;
                        changed = true;
                    }
                });
            }
        }
        PatternPath::Edge(pick) => {
            ui.horizontal_wrapped(|ui| {
                label_cell(ui, "Edge:");
                mono_label(
                    ui,
                    format!(
                        "({:.1}, {:.1}, {:.1})",
                        pick.point[0], pick.point[1], pick.point[2]
                    ),
                    FONT_XS,
                    TEXT1,
                );
                if ui
                    .add_enabled_ui(edge.is_some(), |ui| {
                        accent_outline_button(ui, "Use selected edge")
                    })
                    .inner
                    .on_hover_text("Click an edge of the solid in the viewport, then press this")
                    .clicked()
                    && let Some(picked) = edge
                {
                    *pick = picked;
                    changed = true;
                }
            });
        }
    }
    changed |= count_drag(ui, fx, &mut layout.occurrences, "Occurrences:");
    changed |= check_row(ui, &mut layout.spacing_mode, "Spaced apart")
        .on_hover_text(
            "On: the distance between neighbours along the path. \
             Off: the occurrences spread over the whole path",
        )
        .changed();
    if layout.spacing_mode {
        changed |= mm_drag(ui, fx, &mut layout.spacing, "Spacing:");
    }
    changed |= check_row(ui, &mut layout.reversed, "From the path's end").changed();
    changed |= check_row(ui, &mut layout.align, "Turn with the path")
        .on_hover_text("Each copy turns as the path's direction does from its start")
        .changed();
    changed
}

/// The button adding a path step to a multi-transform: along the body's
/// first sketch with curves, else the selected edge.
pub(super) fn add_path_step(
    ui: &mut Ui,
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
    secondary_button(ui, "+ Path").clicked().then_some(path)
}
