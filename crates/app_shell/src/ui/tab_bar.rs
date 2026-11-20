//! The strip of open documents under the menu bar: one tab per document,
//! the one on screen lit, a dot for unsaved edits, a close on each and a
//! plus at the end. A drag moves a tab along the strip; a right click
//! offers closing it or the others, and reverting it to its file.

use egui::Ui;
use ui_kit::tokens::*;
use ui_kit::widgets::{Tab, TabReorder, tab_plus};

use super::UiCommand;
use crate::contract::TabInfo;

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
                let mut reorder = TabReorder::default();
                for tab in tabs {
                    let mut shown = Tab::new(&tab.name, tab.active)
                        .width(width)
                        .dirty(tab.dirty)
                        .marked(tab.read_only.then_some(("lock", WARNING)))
                        .closable("Close tab  (Ctrl+W)")
                        .draggable()
                        .show(ui);
                    reorder.push(&shown);
                    if tab.read_only {
                        shown.response = shown
                            .response
                            .on_hover_text(crate::app::file_safety::READ_ONLY_TIP);
                    }
                    shown
                        .response
                        .context_menu(|ui| tab_menu(ui, tab, tabs.len() > 1, commands));
                    if shown.closed {
                        commands.push(UiCommand::CloseTab(tab.tab));
                    } else if shown.selected {
                        commands.push(UiCommand::SelectTab(tab.tab));
                    }
                }
                if let Some((from, to)) = reorder.finish(ui) {
                    commands.push(UiCommand::MoveTab {
                        tab: tabs[from].tab,
                        to,
                    });
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

/// A tab's right-click menu.
fn tab_menu(ui: &mut Ui, tab: &TabInfo, others: bool, commands: &mut Vec<UiCommand>) {
    let row = |ui: &mut Ui, label: &str, enabled: bool| {
        let text = egui::RichText::new(label).font(ui_kit::sans(FONT_SM));
        let clicked = ui.add_enabled(enabled, egui::Button::new(text)).clicked();
        if clicked {
            ui.close();
        }
        clicked
    };
    if row(ui, "Close tab", true) {
        commands.push(UiCommand::CloseTab(tab.tab));
    }
    if row(ui, "Close other tabs", others) {
        commands.push(UiCommand::CloseOtherTabs(tab.tab));
    }
    ui.separator();
    if row(ui, "Revert to saved", tab.has_file) {
        commands.push(UiCommand::RevertTab(tab.tab));
    }
}
