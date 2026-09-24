//! The strip of open documents under the menu bar: one tab per document,
//! the one on screen lit, a dot for unsaved edits, a close on each and a
//! plus at the end.

use egui::Ui;
use ui_kit::tokens::*;
use ui_kit::widgets::{Tab, tab_plus};
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
                    let shown = Tab::new(&tab.name, tab.active)
                        .width(width)
                        .dirty(tab.dirty)
                        .closable("Close tab  (Ctrl+W)")
                        .show(ui);
                    if shown.closed {
                        commands.push(UiCommand::CloseTab(tab.tab));
                    } else if shown.selected {
                        commands.push(UiCommand::SelectTab(tab.tab));
                    }
                }
                if tab_plus(ui, TAB_BAR - 8.0)
                    .on_hover_text("New tab  (Ctrl+T)")
                    .clicked()
                {
                    commands.push(UiCommand::NewTab);
                }
            });
        });
}
