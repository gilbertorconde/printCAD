//! STEP import options shown after the user picks a file, before the kernel
//! runs.

use std::path::Path;

use egui::{Context, RichText};
use kernel_api::{LinearDeflectionMode, TessellationSettings};
use ui_kit::tokens::*;
use ui_kit::widgets::{
    Note, QtyField, check_row, note_card, primary_button, secondary_button, select_field,
};
use ui_kit::{mono, sans, sans_semibold};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StepImportDialogAction {
    #[default]
    None,
    Confirmed,
    Cancelled,
}

fn label(ui: &mut egui::Ui, text: &str) {
    ui.add_sized(
        [150.0, INPUT],
        egui::Label::new(RichText::new(text).font(sans(FONT_SM)).color(TEXT2)),
    );
}

pub fn draw_step_import_modal(
    ctx: &Context,
    path: &Path,
    draft: &mut TessellationSettings,
) -> StepImportDialogAction {
    let mut action = StepImportDialogAction::None;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());

    let frame = egui::Frame::new()
        .fill(BG1)
        .stroke(egui::Stroke::new(1.0, BORDER_STRONG))
        .corner_radius(RADIUS_LG as u8)
        .shadow(SHADOW_DIALOG)
        .inner_margin(0);
    egui::Modal::new(egui::Id::new("step_import_modal"))
        .frame(frame)
        .backdrop_color(egui::Color32::from_black_alpha(140))
        .show(ctx, |ui| {
            ui.set_width(460.0);
            // Header.
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_2;
                        ui_kit::icon::draw(ui, "open", 18.0, ACCENT);
                        ui.label(RichText::new("Import STEP").font(sans_semibold(FONT_LG)).color(TEXT1));
                    });
                    ui.label(RichText::new(&file_name).font(mono(FONT_XS)).color(TEXT3))
                        .on_hover_text(path.display().to_string());
                });
            let r = ui.min_rect();
            ui.painter().hline(r.x_range(), r.bottom(), egui::Stroke::new(1.0, BORDER));

            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = SPACE_2;
                    ui.horizontal(|ui| {
                        label(ui, "Linear deflection");
                        let mut mode = draft.linear_deflection_mode;
                        if select_field(
                            ui,
                            "step_linear_mode",
                            &mut mode,
                            &[
                                (LinearDeflectionMode::BboxScaled, "Scaled by the bounding box"),
                                (LinearDeflectionMode::AbsoluteMm, "Absolute chord height"),
                            ],
                            220.0,
                        ) {
                            draft.linear_deflection_mode = mode;
                        }
                    });
                    match draft.linear_deflection_mode {
                        LinearDeflectionMode::BboxScaled => {
                            ui.horizontal(|ui| {
                                label(ui, "Mesh deviation");
                                QtyField::new(&mut draft.mesh_deviation)
                                    .speed(0.005)
                                    .range(0.01..=1.0)
                                    .decimals(3)
                                    .show(ui);
                            });
                        }
                        LinearDeflectionMode::AbsoluteMm => {
                            ui.horizontal(|ui| {
                                label(ui, "Chord tolerance");
                                QtyField::mm(&mut draft.chord_tolerance)
                                    .speed(0.01)
                                    .range(0.001..=5.0)
                                    .decimals(3)
                                    .show(ui);
                            });
                        }
                    }
                    ui.horizontal(|ui| {
                        label(ui, "Angular tolerance");
                        QtyField::degrees(&mut draft.angular_tolerance_deg)
                            .range(0.5..=90.0)
                            .show(ui);
                    });
                    check_row(ui, &mut draft.weld_cross_face, "Weld across faces")
                        .on_hover_text("Merge coplanar-adjacent vertices");
                    if draft.weld_cross_face {
                        ui.horizontal(|ui| {
                            label(ui, "Weld angle threshold");
                            QtyField::degrees(&mut draft.weld_angle_threshold_deg)
                                .range(0.0..=90.0)
                                .show(ui);
                        });
                    }
                    check_row(ui, &mut draft.persist_brep_snapshot, "Keep shape snapshots")
                        .on_hover_text("Serialize each body's shape and mesh it in the background; recommended for large files");
                    check_row(ui, &mut draft.generate_boundary_edges, "Boundary edges")
                        .on_hover_text("Draw face boundaries as edge lines");
                    if !draft.persist_brep_snapshot {
                        note_card(
                            ui,
                            Note::Warning,
                            None,
                            "Without shape snapshots every body is meshed inside the import itself, one long step with no rebuild later.",
                        );
                    }
                });

            // Footer.
            let r = ui.min_rect();
            ui.painter().hline(r.x_range(), r.bottom(), egui::Stroke::new(1.0, BORDER));
            egui::Frame::new()
                .fill(BG2)
                .inner_margin(egui::Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_2;
                        if primary_button(ui, "Import").clicked() {
                            action = StepImportDialogAction::Confirmed;
                        }
                        if secondary_button(ui, "Cancel").clicked() {
                            action = StepImportDialogAction::Cancelled;
                        }
                    });
                });
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                    action = StepImportDialogAction::Confirmed;
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                    action = StepImportDialogAction::Cancelled;
                }
            });
        });
    action
}
