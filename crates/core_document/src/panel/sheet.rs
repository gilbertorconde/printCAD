//! Sheets: rows of cells under column titles, some cells typed into.

use super::*;

/// Draw a [`Widget::Sheet`]: a striped grid under its column titles,
/// scrolled sideways when wider than the panel. An edited cell keeps its
/// draft until typing ends.
pub(super) fn sheet(
    ui: &mut Ui,
    key: egui::Id,
    wid: &str,
    columns: &[String],
    rows: &[Vec<Cell>],
    out: &mut PanelOutput,
) {
    egui::ScrollArea::horizontal()
        .id_salt(key.with("scroll"))
        .show(ui, |ui| {
            egui::Grid::new(key)
                .num_columns(columns.len())
                .striped(true)
                .spacing(egui::vec2(SPACE_3, SPACE_1))
                .show(ui, |ui| {
                    for column in columns {
                        ui.label(
                            RichText::new(column)
                                .font(ui_kit::theme::sans(FONT_XS))
                                .color(TEXT3),
                        );
                    }
                    ui.end_row();
                    for (r, cells) in rows.iter().enumerate() {
                        for (c, cell) in cells.iter().enumerate() {
                            sheet_cell(ui, key.with((r, c)), wid, (r, c), cell, out);
                        }
                        ui.end_row();
                    }
                });
        });
}

pub(super) fn sheet_cell(
    ui: &mut Ui,
    key: egui::Id,
    wid: &str,
    (row, column): (usize, usize),
    cell: &Cell,
    out: &mut PanelOutput,
) {
    match cell {
        Cell::Text { text, mono, strong } => {
            let font = match (mono, strong) {
                (true, _) => ui_kit::theme::mono(FONT_SM),
                (false, true) => ui_kit::theme::sans_semibold(FONT_SM),
                (false, false) => ui_kit::theme::sans(FONT_SM),
            };
            ui.label(RichText::new(text).font(font).color(TEXT1));
        }
        Cell::Link { text } => {
            let clicked = ui
                .add(
                    egui::Button::new(
                        RichText::new(text)
                            .font(ui_kit::theme::sans(FONT_SM))
                            .color(TEXT1),
                    )
                    .frame(false),
                )
                .clicked();
            if clicked {
                out.events.push(PanelEvent::Select {
                    id: wid.to_string(),
                    index: row,
                });
            }
        }
        Cell::Edit { text } => {
            let mut draft: String = ui.data(|d| d.get_temp(key)).unwrap_or_else(|| text.clone());
            let response = ui.add(
                egui::TextEdit::singleline(&mut draft)
                    .desired_width(90.0)
                    .font(ui_kit::theme::sans(FONT_SM)),
            );
            if response.has_focus() {
                ui.data_mut(|d| d.insert_temp(key, draft.clone()));
            }
            if response.lost_focus() {
                ui.data_mut(|d| d.remove::<String>(key));
                if draft != *text {
                    out.events.push(PanelEvent::CellText {
                        id: wid.to_string(),
                        row,
                        column,
                        value: draft,
                    });
                }
            }
        }
        Cell::Check { on, hint } => {
            let mut on = *on;
            let mut response = ui.checkbox(&mut on, "");
            if let Some(hint) = hint {
                response = response.on_hover_text(hint);
            }
            if response.changed() {
                out.events.push(PanelEvent::CellCheck {
                    id: wid.to_string(),
                    row,
                    column,
                    on,
                });
            }
        }
    }
}

/// An editable table cell: a text edit holding its draft while focused,
/// the text as left when focus goes and it changed.
pub(super) fn cell_edit(ui: &mut Ui, key: egui::Id, value: &str) -> Option<String> {
    let mut draft: String = ui
        .data(|d| d.get_temp(key))
        .unwrap_or_else(|| value.to_string());
    let response = ui.add(
        egui::TextEdit::singleline(&mut draft)
            .id(key)
            .font(ui_kit::theme::mono(FONT_SM))
            .desired_width(CELL_WIDTH),
    );
    if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(key, draft.clone()));
    }
    if response.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(key));
        if draft != value {
            return Some(draft);
        }
    }
    None
}

/// The width of an editable table cell.
pub(super) const CELL_WIDTH: f32 = 64.0;
