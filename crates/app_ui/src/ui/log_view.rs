//! The in-app log panel at the bottom of the window.

use egui::RichText;
use ui_kit::tokens::*;
use ui_kit::widgets::small_secondary_button;
use ui_kit::{mono, sans, sans_semibold};

pub fn draw_log_panel(ui: &mut egui::Ui, show: bool) {
    if !show {
        return;
    }
    let entries = app_log::entries();
    if entries.is_empty() {
        return;
    }

    egui::Panel::bottom("log_panel")
        .resizable(true)
        .default_size(160.0)
        .min_size(80.0)
        .frame(
            egui::Frame::new()
                .fill(BG1)
                .inner_margin(egui::Margin::symmetric(10, 6)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Log")
                        .font(sans_semibold(FONT_SM))
                        .color(TEXT1),
                );
                ui.add_space(SPACE_2);
                if small_secondary_button(ui, "Clear").clicked() {
                    app_log::clear();
                }
            });
            ui.add_space(SPACE_1);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for entry in entries {
                        let secs = entry.timestamp_secs % 86_400;
                        let time = format!(
                            "{:02}:{:02}:{:02}",
                            secs / 3600,
                            (secs % 3600) / 60,
                            secs % 60
                        );
                        let (label, color) = match entry.level {
                            app_log::LogLevel::Info => ("INFO", INFO),
                            app_log::LogLevel::Success => ("OK", SUCCESS),
                            app_log::LogLevel::Warn => ("WARN", WARNING),
                            app_log::LogLevel::Error => ("ERROR", DANGER),
                        };
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = SPACE_2;
                            ui.label(RichText::new(time).font(mono(FONT_XS)).color(TEXT3));
                            ui.label(RichText::new(label).font(mono(FONT_XS)).color(color));
                            // Messages are copied out into reports.
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&entry.message)
                                        .font(sans(FONT_SM))
                                        .color(TEXT2),
                                )
                                .selectable(true),
                            );
                        });
                    }
                });
        });
}
