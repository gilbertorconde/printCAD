//! The borrowed geometry task panel: what the body borrows (another body's
//! sketch, or faces and edges of another body's solid), and whether it
//! follows its source or keeps a frozen copy.

use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};
use egui::Ui;
use ui_kit::tokens::*;
use ui_kit::widgets::{
    Note, accent_outline_button, check_row, mono_label, note_card, small_secondary_button,
};

use super::label_cell;
use crate::borrow;
use crate::feature::{BorrowOptions, BorrowSource, EdgePick, FacePick, FrozenBorrow};

/// A body's name, for the panel.
fn body_name(ctx: &WorkbenchRuntimeContext, body: BodyId) -> String {
    ctx.document
        .bodies()
        .iter()
        .find(|b| b.id == body)
        .map(|b| b.name.clone())
        .unwrap_or_else(|| "a missing body".into())
}

/// Every sketch of another body than `body`, named with its body.
fn other_sketches(ctx: &WorkbenchRuntimeContext, body: BodyId) -> Vec<(FeatureId, String)> {
    let mut sketches: Vec<(u64, FeatureId, String)> = ctx
        .document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == "wb.sketch" && n.body != Some(body))
        .map(|(id, n)| {
            let home = n
                .body
                .map(|b| body_name(ctx, b))
                .unwrap_or_else(|| "Document".into());
            (n.seq, *id, format!("{home} › {}", n.name))
        })
        .collect();
    sketches.sort_by_key(|(seq, id, _)| (*seq, *id));
    sketches.into_iter().map(|(_, id, n)| (id, n)).collect()
}

/// What the source is called in the panel.
fn source_label(ctx: &WorkbenchRuntimeContext, source: &BorrowSource) -> String {
    match source {
        BorrowSource::Sketch(sketch) => ctx
            .document
            .get_feature_meta(*sketch)
            .map(|n| {
                let home = n
                    .body
                    .map(|b| body_name(ctx, b))
                    .unwrap_or_else(|| "Document".into());
                format!("{home} › {}", n.name)
            })
            .unwrap_or_else(|| "a missing sketch".into()),
        BorrowSource::Solid { body, .. } => format!("Faces and edges of {}", body_name(ctx, *body)),
    }
}

/// Where the borrowed geometry is moved and turned to from where the
/// bodies put it.
fn offset_editor(ui: &mut Ui, feature_id: FeatureId, options: &mut BorrowOptions) -> bool {
    use ui_kit::widgets::QtyField;
    let mut changed = false;
    egui::CollapsingHeader::new("Offset")
        .id_salt(("borrow_offset", feature_id))
        .default_open(!options.offset.eq(&Default::default()))
        .show(ui, |ui| {
            let o = &mut options.offset;
            let [x, y, z] = &mut o.translation;
            for (label, value) in [("Along X", x), ("Along Y", y), ("Along Z", z)] {
                ui.horizontal(|ui| {
                    label_cell(ui, label);
                    changed |= QtyField::offset(value).show(ui);
                });
            }
            let [tilt_x, tilt_y] = &mut o.tilt;
            for (label, value) in [
                ("Turn about Z", &mut o.rotation_deg),
                ("Tilt about X", tilt_x),
                ("Tilt about Y", tilt_y),
            ] {
                ui.horizontal(|ui| {
                    label_cell(ui, label);
                    changed |= QtyField::degrees(value).show(ui);
                });
            }
            changed |= check_row(ui, &mut o.flip, "Flip").changed();
        });
    changed
}

/// Why freezing failed, kept for the panel until the next try.
fn freeze_error_id(feature: FeatureId) -> egui::Id {
    egui::Id::new(("borrow_freeze_error", feature))
}

/// The borrow editor; `true` when it changed the feature.
pub(super) fn borrow_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    feature_id: FeatureId,
    source: &mut BorrowSource,
    frozen: &mut Option<FrozenBorrow>,
    options: &mut BorrowOptions,
) -> bool {
    let mut changed = false;
    let sketches = other_sketches(ctx, body);
    let solids: Vec<BodyId> = ctx
        .document
        .bodies()
        .iter()
        .filter(|b| b.id != body && ctx.document.imported_brep_blob(b.id).is_some())
        .map(|b| b.id)
        .collect();
    ui.horizontal(|ui| {
        label_cell(ui, "Source");
        egui::ComboBox::from_id_salt(("borrow_source", feature_id))
            .selected_text(source_label(ctx, source))
            .width(160.0)
            .show_ui(ui, |ui| {
                for (id, name) in &sketches {
                    let current = *source == BorrowSource::Sketch(*id);
                    if ui.selectable_label(current, name).clicked() && !current {
                        *source = BorrowSource::Sketch(*id);
                        changed = true;
                    }
                }
                for other in &solids {
                    let current =
                        matches!(source, BorrowSource::Solid { body, .. } if body == other);
                    let name = format!("Faces and edges of {}", body_name(ctx, *other));
                    if ui.selectable_label(current, name).clicked() && !current {
                        *source = BorrowSource::Solid {
                            body: *other,
                            faces: Vec::new(),
                            edges: Vec::new(),
                        };
                        changed = true;
                    }
                }
            });
    });
    if let BorrowSource::Solid {
        body: from,
        faces,
        edges,
    } = source
    {
        changed |= picks_editor(ui, ctx, body, from, faces, edges);
        changed |= check_row(ui, &mut options.fill, "Fill closed edges")
            .on_hover_text(
                "Edges that close into a loop in one plane lend the face they bound, as a profile",
            )
            .changed();
        changed |= check_row(ui, &mut options.whole, "Whole solid as reference")
            .on_hover_text("Draw all of the other body's edges here, to measure and stop on")
            .changed();
    } else if options.fill || options.whole {
        options.fill = false;
        options.whole = false;
        changed = true;
    }
    changed |= offset_editor(ui, feature_id, options);

    let mut is_frozen = frozen.is_some();
    let toggled = check_row(ui, &mut is_frozen, "Frozen")
        .on_hover_text("Keep the geometry as it is now; unticked, it follows its source")
        .changed();
    let refreeze = frozen.is_some()
        && (changed
            || small_secondary_button(ui, "Take it again")
                .on_hover_text("Freeze the source as it is now")
                .clicked());
    if toggled && !is_frozen {
        *frozen = None;
        changed = true;
    } else if (toggled && is_frozen) || refreeze {
        match borrow::freeze(ctx.document, ctx.kernel, body, source, options) {
            Ok(snapshot) => {
                *frozen = Some(snapshot);
                ui.data_mut(|d| d.remove::<String>(freeze_error_id(feature_id)));
            }
            Err(e) => {
                *frozen = None;
                ui.data_mut(|d| d.insert_temp(freeze_error_id(feature_id), e));
            }
        }
        changed = true;
    }
    let error: Option<String> = ui.data(|d| d.get_temp(freeze_error_id(feature_id)));
    match (&error, frozen.is_some()) {
        (Some(error), false) => {
            note_card(ui, Note::Error, Some("Could not freeze"), error);
        }
        (_, true) => {
            note_card(
                ui,
                Note::Info,
                None,
                "Frozen: the source may change or move; this body keeps what it took.",
            );
        }
        (None, false) => {
            note_card(
                ui,
                Note::Info,
                None,
                "Follows its source: what stands on it rebuilds as the source changes or moves.",
            );
        }
    }
    changed
}

/// The faces and edges borrowed from `from`, with rows to take the ones
/// picked in the viewport. A pick on yet another body starts the lists
/// again from that body.
fn picks_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    using: BodyId,
    from: &mut BodyId,
    faces: &mut Vec<FacePick>,
    edges: &mut Vec<EdgePick>,
) -> bool {
    let mut changed = false;
    let mut remove_face = None;
    for (i, face) in faces.iter().enumerate() {
        ui.horizontal(|ui| {
            label_cell(ui, if i == 0 { "Faces" } else { "" });
            mono_label(
                ui,
                format!(
                    "({:.1}, {:.1}, {:.1})",
                    face.point[0], face.point[1], face.point[2]
                ),
                FONT_XS,
                TEXT1,
            );
            if small_secondary_button(ui, "×").clicked() {
                remove_face = Some(i);
            }
        });
    }
    if let Some(i) = remove_face {
        faces.remove(i);
        changed = true;
    }
    let mut remove_edge = None;
    for (i, edge) in edges.iter().enumerate() {
        ui.horizontal(|ui| {
            label_cell(ui, if i == 0 { "Edges" } else { "" });
            mono_label(
                ui,
                format!(
                    "({:.1}, {:.1}, {:.1})",
                    edge.point[0], edge.point[1], edge.point[2]
                ),
                FONT_XS,
                TEXT1,
            );
            if small_secondary_button(ui, "×").clicked() {
                remove_edge = Some(i);
            }
        });
    }
    if let Some(i) = remove_edge {
        edges.remove(i);
        changed = true;
    }

    // The face picked now belongs to the body the host says was clicked.
    let face_body = ctx
        .selected_body_id
        .map(BodyId)
        .filter(|b| *b != using && ctx.selected_face.is_some());
    let edge_body = ctx
        .selected_edges
        .first()
        .map(|e| BodyId(e.body))
        .filter(|b| *b != using);
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled_ui(face_body.is_some(), |ui| {
                accent_outline_button(ui, "Add selected face")
            })
            .inner
            .on_hover_text("Click a face of another body in the viewport first, then press this")
            .clicked()
            && let Some(picked) = face_body
            && let Some(face) = ctx.selected_face_in(picked)
        {
            restart_from(from, picked, faces, edges);
            faces.push(FacePick {
                name: 0,
                point: face.point,
                normal: face.normal,
            });
            changed = true;
        }
        if ui
            .add_enabled_ui(edge_body.is_some(), |ui| {
                accent_outline_button(ui, "Add selected edges")
            })
            .inner
            .on_hover_text("Click edges of another body in the viewport first, then press this")
            .clicked()
            && let Some(picked) = edge_body
        {
            restart_from(from, picked, faces, edges);
            for edge in ctx
                .selected_edges_in(picked)
                .iter()
                .filter(|e| BodyId(e.body) == picked)
            {
                edges.push(EdgePick {
                    faces: [0, 0],
                    point: edge.point,
                    direction: edge.direction,
                });
            }
            changed = true;
        }
    });
    changed
}

/// Borrowing from `picked` instead of `from`: the lists start again.
fn restart_from(
    from: &mut BodyId,
    picked: BodyId,
    faces: &mut Vec<FacePick>,
    edges: &mut Vec<EdgePick>,
) {
    if picked != *from {
        *from = picked;
        faces.clear();
        edges.clear();
    }
}
