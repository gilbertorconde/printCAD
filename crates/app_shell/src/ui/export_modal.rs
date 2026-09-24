//! The export dialog: the format, which bodies, and for the mesh formats
//! how closely the triangles follow the surface. It edits the host's draft
//! in place; Export asks for a file name.

use egui::{Context, RichText};
use kernel_ogeom::export::ExportFormat;
use ui_kit::tokens::*;
use ui_kit::widgets::{
    Note, QtyField, check_row, note_card, primary_button, secondary_button, select_field,
};
use ui_kit::{sans, sans_semibold};

use super::StepImportDialogAction as DialogAction;
use crate::app::export::ExportDraft;

fn label(ui: &mut egui::Ui, text: &str) {
    ui.add_sized(
        [150.0, INPUT],
        egui::Label::new(RichText::new(text).font(sans(FONT_SM)).color(TEXT2)),
    );
}

/// `configurations` is how many the document has: with any, the dialog
/// offers a file for each.
pub fn draw_export_modal(
    ctx: &Context,
    draft: &mut ExportDraft,
    configurations: usize,
) -> DialogAction {
    let mut action = DialogAction::None;
    let frame = egui::Frame::new()
        .fill(BG1)
        .stroke(egui::Stroke::new(1.0, BORDER_STRONG))
        .corner_radius(RADIUS_LG as u8)
        .shadow(SHADOW_DIALOG)
        .inner_margin(0);
    egui::Modal::new(egui::Id::new("export_modal"))
        .frame(frame)
        .backdrop_color(egui::Color32::from_black_alpha(140))
        .show(ctx, |ui| {
            ui.set_width(460.0);
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_2;
                        ui_kit::icon::draw(ui, "export-stl", 18.0, ACCENT);
                        ui.label(RichText::new("Export").font(sans_semibold(FONT_LG)).color(TEXT1));
                    });
                });
            let r = ui.min_rect();
            ui.painter().hline(r.x_range(), r.bottom(), egui::Stroke::new(1.0, BORDER));

            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = SPACE_2;
                    ui.horizontal(|ui| {
                        label(ui, "Format");
                        let options: Vec<(ExportFormat, &str)> = ExportFormat::ALL
                            .iter()
                            .map(|f| {
                                (*f, match f {
                                    ExportFormat::Step => "STEP: exact shapes, for other CAD",
                                    ExportFormat::Stl => "STL: triangles, for any slicer",
                                    ExportFormat::ThreeMf => "3MF: named closed meshes, for slicers",
                                })
                            })
                            .collect();
                        select_field(ui, "export_format", &mut draft.format, &options, 260.0);
                    });
                    check_row(ui, &mut draft.selected_only, "Selected body only")
                        .on_hover_text("Otherwise every visible body");
                    if configurations > 0 {
                        check_row(
                            ui,
                            &mut draft.every_configuration,
                            &format!("Every configuration ({configurations} files)"),
                        )
                        .on_hover_text(
                            "One file per configuration, each built and named after it",
                        );
                    }
                    if draft.format.is_mesh() {
                        ui.horizontal(|ui| {
                            label(ui, "Chord tolerance");
                            draft.detail.linear_deflection_mode =
                                kernel_api::LinearDeflectionMode::AbsoluteMm;
                            QtyField::mm(&mut draft.detail.chord_tolerance)
                                .speed(0.001)
                                .range(0.001..=1.0)
                                .decimals(3)
                                .show(ui)
                        })
                        .response
                        .on_hover_text("How far a facet may stray from the true surface");
                        ui.horizontal(|ui| {
                            label(ui, "Angular tolerance");
                            QtyField::degrees(&mut draft.detail.angular_tolerance_deg)
                                .range(0.5..=45.0)
                                .show(ui)
                        })
                        .response
                        .on_hover_text("The most a curved surface turns across one facet");
                        note_card(
                            ui,
                            Note::Info,
                            None,
                            "A finer tolerance prints smoother curves and makes a larger file; \
                             a tenth of your nozzle width is plenty.",
                        );
                    } else {
                        note_card(
                            ui,
                            Note::Info,
                            None,
                            "STEP carries the exact shapes. Mesh bodies have none and are left out.",
                        );
                    }
                });

            let r = ui.min_rect();
            ui.painter().hline(r.x_range(), r.bottom(), egui::Stroke::new(1.0, BORDER));
            egui::Frame::new()
                .fill(BG2)
                .inner_margin(egui::Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_2;
                        if primary_button(ui, "Export…").clicked() {
                            action = DialogAction::Confirmed;
                        }
                        if secondary_button(ui, "Cancel").clicked() {
                            action = DialogAction::Cancelled;
                        }
                    });
                });
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                    action = DialogAction::Confirmed;
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                    action = DialogAction::Cancelled;
                }
            });
        });
    action
}
