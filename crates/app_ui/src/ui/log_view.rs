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
            super::panel_contents(ui, |ui| {
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
                // The entries take the room under the header, down to
                // nothing in a short panel, so the newest one stays in view.
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .min_scrolled_height(0.0)
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
            })
        });
}

#[cfg(test)]
mod tests {
    /// A log panel dragged down to its smallest keeps that height with
    /// its entries filling more than it shows.
    #[test]
    fn a_short_panel_keeps_its_height() {
        for i in 0..40 {
            app_log::info(format!("entry {i}"));
        }
        let ctx = egui::Context::default();
        ui_kit::apply_theme(&ctx);
        let id = egui::Id::new("log_panel");
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
        let mut heights = Vec::new();
        for frame in 0..10 {
            if frame == 1 {
                let outer_rect = egui::Rect::from_x_y_ranges(screen.x_range(), 720.0..=800.0);
                ctx.data_mut(|d| d.insert_persisted(id, egui::PanelState { outer_rect }));
            }
            let raw = egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| super::draw_log_panel(ui, true));
            output.textures_delta.clear();
            heights.extend(egui::PanelState::load(&ctx, id).map(|s| s.size().y));
        }
        assert!(
            heights[1..].iter().all(|h| (h - 80.0).abs() < 0.5),
            "{heights:?}"
        );
    }
}
