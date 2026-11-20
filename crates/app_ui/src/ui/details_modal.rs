//! A message shown at length: what a tree row's "!" says, in a window
//! where it can be selected and copied, to paste into a report.

use egui::{Context, RichText};
use ui_kit::tokens::*;
use ui_kit::widgets::{primary_button, secondary_button};
use ui_kit::{mono, sans, sans_semibold};

/// Draws the window; returns whether it stays open.
pub fn draw_details_modal(ctx: &Context, title: &str, text: &str) -> bool {
    let mut open = true;
    let frame = egui::Frame::new()
        .fill(BG1)
        .stroke(egui::Stroke::new(1.0, BORDER_STRONG))
        .corner_radius(RADIUS_LG as u8)
        .shadow(SHADOW_DIALOG)
        .inner_margin(0);
    egui::Modal::new(egui::Id::new("details_modal"))
        .frame(frame)
        .backdrop_color(egui::Color32::from_black_alpha(140))
        .show(ctx, |ui| {
            ui.set_width(560.0);
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_2;
                        ui_kit::icon::draw(ui, "warning", 18.0, DANGER);
                        ui.label(
                            RichText::new(title)
                                .font(sans_semibold(FONT_LG))
                                .color(TEXT1),
                        );
                    });
                });
            let r = ui.min_rect();
            ui.painter()
                .hline(r.x_range(), r.bottom(), egui::Stroke::new(1.0, BORDER));
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(360.0)
                        .show(ui, |ui| {
                            // Read-only, but selectable: a text edit over a
                            // copy of the message.
                            let mut shown = text.to_string();
                            ui.add(
                                egui::TextEdit::multiline(&mut shown)
                                    .font(mono(FONT_SM))
                                    .desired_width(f32::INFINITY)
                                    .interactive(true),
                            );
                        });
                });
            let r = ui.min_rect();
            ui.painter()
                .hline(r.x_range(), r.bottom(), egui::Stroke::new(1.0, BORDER));
            egui::Frame::new()
                .fill(BG2)
                .inner_margin(egui::Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_2;
                        if secondary_button(ui, "Close").clicked() {
                            open = false;
                        }
                        if primary_button(ui, "Copy").clicked() {
                            ui.ctx().copy_text(format!("{title}\n{text}"));
                        }
                        ui.label(
                            RichText::new("Select any part, or copy it all")
                                .font(sans(FONT_XS))
                                .color(TEXT3),
                        );
                    });
                });
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                open = false;
            }
        });
    open
}
