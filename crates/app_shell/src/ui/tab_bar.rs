//! The strip of open documents under the menu bar: one tab per document,
//! the one on screen lit, a dot for unsaved edits, a close on each and a
//! plus at the end.

use egui::{Sense, Ui, vec2};
use ui_kit::tokens::*;
use ui_kit::{sans, sans_medium};
use uuid::Uuid;

use super::UiCommand;

/// One tab as the host describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabInfo {
    pub tab: Uuid,
    pub name: String,
    pub dirty: bool,
    pub active: bool,
    /// Nothing has happened in the tab: it shows the start page and New
    /// or Open take it over instead of opening another.
    pub blank: bool,
}

const TAB_MIN: f32 = 96.0;
const TAB_MAX: f32 = 220.0;
const CLOSE: f32 = 16.0;

pub fn draw_tab_bar(ui: &mut Ui, tabs: &[TabInfo], commands: &mut Vec<UiCommand>) {
    egui::Panel::top("tab_bar")
        .exact_size(TAB_BAR)
        .frame(
            egui::Frame::new()
                .fill(BG1)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .inner_margin(egui::Margin::symmetric(6, 0)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let available = ui.available_width() - 32.0;
                let width = (available / tabs.len().max(1) as f32).clamp(TAB_MIN, TAB_MAX);
                for tab in tabs {
                    draw_tab(ui, tab, width, commands);
                }
                let (rect, plus) =
                    ui.allocate_exact_size(vec2(26.0, TAB_BAR - 8.0), Sense::click());
                if plus.hovered() {
                    ui.painter().rect_filled(rect, RADIUS_SM, BG2);
                }
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "+",
                    sans_medium(FONT_MD),
                    if plus.hovered() { TEXT1 } else { TEXT2 },
                );
                if plus.on_hover_text("New tab  (Ctrl+T)").clicked() {
                    commands.push(UiCommand::NewTab);
                }
            });
        });
}

fn draw_tab(ui: &mut Ui, tab: &TabInfo, width: f32, commands: &mut Vec<UiCommand>) {
    let (rect, response) = ui.allocate_exact_size(vec2(width, TAB_BAR - 4.0), Sense::click());
    let hovered = response.hovered();
    let painter = ui.painter();
    if tab.active {
        painter.rect_filled(rect, RADIUS_SM, BG0);
        painter.hline(
            rect.x_range(),
            rect.bottom() - 1.0,
            egui::Stroke::new(2.0, ACCENT),
        );
    } else if hovered {
        painter.rect_filled(rect, RADIUS_SM, BG2);
    }

    // Name on the left, then the dot, then the close on the far right.
    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 6.0 - CLOSE / 2.0, rect.center().y),
        vec2(CLOSE, CLOSE),
    );
    let mut text_right = close_rect.left() - 4.0;
    if tab.dirty {
        painter.circle_filled(egui::pos2(text_right - 4.0, rect.center().y), 3.0, WARNING);
        text_right -= 12.0;
    }
    let text_color = if tab.active { TEXT1 } else { TEXT2 };
    let font = if tab.active {
        sans_medium(FONT_SM)
    } else {
        sans(FONT_SM)
    };
    let max_width = (text_right - rect.left() - 10.0).max(0.0);
    let galley = painter.layout(tab.name.clone(), font, text_color, max_width);
    let clip = egui::Rect::from_min_max(rect.min, egui::pos2(text_right, rect.max.y));
    painter.with_clip_rect(clip).galley(
        egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0),
        galley,
        text_color,
    );

    let close = ui.interact(close_rect, response.id.with("close"), Sense::click());
    if hovered || tab.active || close.hovered() {
        if close.hovered() {
            ui.painter().rect_filled(close_rect, RADIUS_SM, BG2);
        }
        ui.painter().text(
            close_rect.center(),
            egui::Align2::CENTER_CENTER,
            "×",
            sans(FONT_MD),
            if close.hovered() { TEXT1 } else { TEXT3 },
        );
    }
    if close.on_hover_text("Close tab  (Ctrl+W)").clicked() || response.middle_clicked() {
        commands.push(UiCommand::CloseTab(tab.tab));
    } else if response.clicked() && !tab.active {
        commands.push(UiCommand::SelectTab(tab.tab));
    }
}
