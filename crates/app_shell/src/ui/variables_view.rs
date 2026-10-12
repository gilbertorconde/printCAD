//! Variable sets and the configurations table in the property panel's
//! Data tab, shown when one is selected in the tree.
//!
//! A set lists its variables, one row each with what it comes to; a click
//! opens the row to edit its name, formula (with what it comes to as you
//! type) and comment, and the rows at the end add one. The configurations
//! list theirs with the one in effect marked; a click opens a
//! configuration's values. Their table shows every value at once,
//! configurations as rows or, switched, as columns. Everything answers
//! with commands.

use core_document::{Configuration, Configurations, Document, FeatureId, Variable};
use egui::RichText;
use ui_kit::tokens::*;
use ui_kit::widgets::{check_row, small_secondary_button};
use ui_kit::{mono, sans, sans_medium};

use super::{ConfigEdit, UiCommand};

/// What the Data tab's variable and configuration editors hold between
/// frames.
#[derive(Debug, Default)]
pub struct VariablesState {
    /// The variable open for editing: its set and name, and the drafts.
    open: Option<(FeatureId, String, Draft)>,
    /// The configuration open for editing, by name, with its drafts.
    open_configuration: Option<(String, ConfigDraft)>,
    new_name: String,
    new_formula: String,
    new_configuration: String,
    /// The configurations table drawn with configurations as columns and
    /// variables as rows.
    vertical: bool,
}

/// The configurations table as drawn: the names along the top, those
/// down the side, and the cell at each crossing (`cells[down][across]`).
#[derive(Debug, PartialEq)]
struct Grid<'a> {
    across: Vec<&'a str>,
    down: Vec<&'a str>,
    cells: Vec<Vec<Cell<'a>>>,
}

/// One crossing of the table: the variable it sets and the
/// configuration's value, empty where it keeps the variable's own.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Cell<'a> {
    variable: &'a str,
    value: &'a str,
}

/// `table` laid out with configurations down the side, or along the top
/// when `vertical`.
fn grid(table: &Configurations, vertical: bool) -> Grid<'_> {
    let configurations: Vec<&str> = table.rows.iter().map(|r| r.name.as_str()).collect();
    let variables: Vec<&str> = table.columns.iter().map(String::as_str).collect();
    let cell = |row: usize, column: usize| Cell {
        variable: variables[column],
        value: table.rows[row]
            .values
            .get(column)
            .map_or("", String::as_str),
    };
    if vertical {
        let cells = (0..variables.len())
            .map(|c| (0..configurations.len()).map(|r| cell(r, c)).collect())
            .collect();
        Grid {
            across: configurations,
            down: variables,
            cells,
        }
    } else {
        let cells = (0..configurations.len())
            .map(|r| (0..variables.len()).map(|c| cell(r, c)).collect())
            .collect();
        Grid {
            across: variables,
            down: configurations,
            cells,
        }
    }
}

/// An open variable's name, formula and comment, as typed.
#[derive(Debug, Clone, Default, PartialEq)]
struct Draft {
    name: String,
    formula: String,
    comment: String,
}

/// An open configuration's name and a value per column, as typed.
#[derive(Debug, Clone, Default, PartialEq)]
struct ConfigDraft {
    name: String,
    values: Vec<String>,
}

/// A group's title bar, as the property panel draws its groups.
fn group(ui: &mut egui::Ui, title: &str) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, BG0);
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1.0, BORDER),
    );
    ui.painter().text(
        egui::pos2(rect.left() + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        title.to_uppercase(),
        ui_kit::sans_semibold(FONT_XS),
        TEXT3,
    );
}

/// A row with `left` and `right` across the width, clickable.
fn row(ui: &mut egui::Ui, left: RichText, right: RichText, open: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), TREE_ROW),
        egui::Sense::click(),
    );
    if open {
        ui.painter().rect_filled(rect, 0.0, ACCENT_DIM);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, BG2);
    }
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1.0, BG2),
    );
    let split = rect.left() + rect.width() * 0.45;
    for (text, area) in [
        (
            left,
            egui::Rect::from_min_max(
                egui::pos2(rect.left() + 12.0, rect.top()),
                egui::pos2(split - 6.0, rect.bottom()),
            ),
        ),
        (
            right,
            egui::Rect::from_min_max(
                egui::pos2(split + 6.0, rect.top()),
                egui::pos2(rect.right() - 8.0, rect.bottom()),
            ),
        ),
    ] {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(area)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.add(egui::Label::new(text).truncate().selectable(false));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A labelled field of an open row's editor; answers whether Enter was
/// pressed in it.
fn labelled_field(ui: &mut egui::Ui, label: &str, text: &mut String, font: egui::FontId) -> bool {
    ui.horizontal(|ui| {
        ui.add_sized(
            [64.0, INPUT],
            egui::Label::new(RichText::new(label).font(sans(FONT_SM)).color(TEXT2)),
        );
        let field = ui.add(
            egui::TextEdit::singleline(text)
                .font(font)
                .desired_width(ui.available_width()),
        );
        field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
    })
    .inner
}

/// A labelled formula box that completes names as they are typed;
/// answers whether Enter kept what was typed.
fn labelled_formula(
    ui: &mut egui::Ui,
    label: &str,
    text: &mut String,
    document: &Document,
    id: egui::Id,
) -> bool {
    ui.horizontal(|ui| {
        ui.add_sized(
            [64.0, INPUT],
            egui::Label::new(RichText::new(label).font(sans(FONT_SM)).color(TEXT2)),
        );
        formula_box(ui, text, document, id, None)
    })
    .inner
}

/// A formula box filling the width, completing names; answers whether
/// Enter kept what was typed.
fn formula_box(
    ui: &mut egui::Ui,
    text: &mut String,
    document: &Document,
    id: egui::Id,
    hint: Option<&str>,
) -> bool {
    let width = ui.available_width();
    let edit = ui_kit::completion::completing_text_edit(
        ui,
        id,
        text,
        &|| core_document::formula_candidates(document),
        |edit| {
            let edit = edit.font(mono(FONT_SM)).desired_width(width);
            match hint {
                Some(hint) => edit.hint_text(RichText::new(hint).color(TEXT3)),
                None => edit,
            }
        },
    );
    !edit.picked && edit.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

/// What `formula` comes to in `document`, under a formula being typed.
fn preview(ui: &mut egui::Ui, document: &Document, formula: &str) {
    if formula.trim().is_empty() {
        return;
    }
    let (line, color) = match document.evaluate_formula(formula, None) {
        Ok(q) => (
            format!("= {}", q.display(document.display_unit(), 4)),
            TEXT3,
        ),
        Err(why) => (why, DANGER),
    };
    ui.horizontal(|ui| {
        ui.add_space(68.0);
        ui.add(egui::Label::new(RichText::new(line).font(sans(FONT_XS)).color(color)).wrap());
    });
}

/// A note in the list's own margin.
fn note(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.add(egui::Label::new(RichText::new(text).font(sans(FONT_SM)).color(TEXT3)).wrap());
    });
}

/// The commands that apply `draft` to `variable` of `set`.
fn variable_changes(set: FeatureId, variable: &Variable, draft: &Draft) -> Vec<UiCommand> {
    let mut out = Vec::new();
    let formula = draft.formula.trim();
    let comment = draft.comment.trim();
    if formula != variable.formula || comment != variable.comment {
        out.push(UiCommand::SetVariable {
            set,
            name: variable.name.clone(),
            formula: formula.to_string(),
            comment: Some(comment.to_string()),
        });
    }
    let name = draft.name.trim();
    if !name.is_empty() && name != variable.name {
        out.push(UiCommand::RenameVariable {
            set,
            name: variable.name.clone(),
            to: name.to_string(),
        });
    }
    out
}

/// The commands that apply `draft` to `configuration`.
fn configuration_changes(
    table: &Configurations,
    configuration: &Configuration,
    draft: &ConfigDraft,
) -> Vec<UiCommand> {
    let mut out = Vec::new();
    for (i, (column, value)) in table.columns.iter().zip(&draft.values).enumerate() {
        let before = configuration.values.get(i).map_or("", String::as_str);
        if value.trim() != before {
            out.push(UiCommand::Config(ConfigEdit::Set {
                name: configuration.name.clone(),
                variable: column.clone(),
                value: value.trim().to_string(),
            }));
        }
    }
    let name = draft.name.trim();
    if !name.is_empty() && name != configuration.name {
        out.push(UiCommand::Config(ConfigEdit::Rename {
            name: configuration.name.clone(),
            to: name.to_string(),
        }));
    }
    out
}

/// A variable set's variables, editable.
pub fn variable_set_section(
    ui: &mut egui::Ui,
    state: &mut VariablesState,
    document: &Document,
    set: FeatureId,
    commands: &mut Vec<UiCommand>,
) {
    let Some((_, set_name, variables)) = document
        .variable_sets()
        .into_iter()
        .find(|(id, ..)| *id == set)
    else {
        return;
    };
    let slots = document.evaluated_slots(set);
    let unit = document.display_unit();
    group(ui, "Variables");
    if variables.variables.is_empty() {
        ui.add_space(SPACE_1);
        note(
            ui,
            &format!(
                "None yet. Formulas read a variable here as {}.name.",
                core_document::expr::quote_name(&set_name)
            ),
        );
    }
    for v in &variables.variables {
        let result = slots.iter().find(|s| s.key == v.name).map(|s| &s.result);
        let value = match result {
            Some(Ok(q)) => RichText::new(q.display(unit, 4))
                .font(mono(FONT_SM))
                .color(TEXT1),
            Some(Err(_)) => RichText::new("error").font(sans(FONT_SM)).color(DANGER),
            None => RichText::new(""),
        };
        let is_open = matches!(&state.open, Some((s, n, _)) if *s == set && *n == v.name);
        let response = row(
            ui,
            RichText::new(&v.name).font(sans(FONT_SM)).color(TEXT1),
            value,
            is_open,
        );
        let hover = match result {
            Some(Err(why)) => format!("{}\n{why}", v.formula),
            _ if v.comment.is_empty() => v.formula.clone(),
            _ => format!("{}\n{}", v.formula, v.comment),
        };
        if response.on_hover_text(hover).clicked() {
            state.open = (!is_open).then(|| {
                (
                    set,
                    v.name.clone(),
                    Draft {
                        name: v.name.clone(),
                        formula: v.formula.clone(),
                        comment: v.comment.clone(),
                    },
                )
            });
        }
        if is_open {
            variable_editor(ui, state, document, set, v, commands);
        }
    }

    ui.add_space(SPACE_2);
    group(ui, "Add a variable");
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 6))
        .show(ui, |ui| {
            let mut entered = labelled_field(ui, "Name", &mut state.new_name, sans(FONT_SM));
            entered |= labelled_formula(
                ui,
                "Formula",
                &mut state.new_formula,
                document,
                egui::Id::new(("new_variable_formula", set)),
            );
            preview(ui, document, &state.new_formula);
            let ready = !state.new_name.trim().is_empty() && !state.new_formula.trim().is_empty();
            let add = ui
                .add_enabled_ui(ready, |ui| small_secondary_button(ui, "Add"))
                .inner
                .clicked();
            if ready && (add || entered) {
                commands.push(UiCommand::SetVariable {
                    set,
                    name: state.new_name.trim().to_string(),
                    formula: state.new_formula.trim().to_string(),
                    comment: None,
                });
                state.new_name.clear();
                state.new_formula.clear();
            }
        });
}

/// An open variable: its name, formula and comment, kept by Apply or
/// Enter; Remove takes it away.
fn variable_editor(
    ui: &mut egui::Ui,
    state: &mut VariablesState,
    document: &Document,
    set: FeatureId,
    variable: &Variable,
    commands: &mut Vec<UiCommand>,
) {
    let Some((_, _, draft)) = &mut state.open else {
        return;
    };
    let mut apply = false;
    let mut close = false;
    egui::Frame::new()
        .fill(BG2)
        .inner_margin(egui::Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            apply |= labelled_field(ui, "Name", &mut draft.name, sans(FONT_SM));
            apply |= labelled_formula(
                ui,
                "Formula",
                &mut draft.formula,
                document,
                egui::Id::new(("variable_formula", set, &variable.name)),
            );
            preview(ui, document, &draft.formula);
            apply |= labelled_field(ui, "Comment", &mut draft.comment, sans(FONT_SM));
            ui.horizontal(|ui| {
                if small_secondary_button(ui, "Apply").clicked() {
                    apply = true;
                }
                if small_secondary_button(ui, "Remove")
                    .on_hover_text("Formulas that read it report it missing")
                    .clicked()
                {
                    commands.push(UiCommand::RemoveVariable {
                        set,
                        name: variable.name.clone(),
                    });
                    close = true;
                }
            });
        });
    if apply {
        commands.extend(variable_changes(set, variable, draft));
        close = true;
    }
    if close {
        state.open = None;
    }
}

/// The configurations: which is in effect, the variables they set, and
/// each one's values.
pub fn configurations_section(
    ui: &mut egui::Ui,
    state: &mut VariablesState,
    document: &Document,
    commands: &mut Vec<UiCommand>,
) {
    let table = document
        .configurations()
        .map(|(_, t)| t)
        .unwrap_or_default();
    // A variable's own formula, shown where a configuration leaves it.
    let own = |column: &str| -> String {
        core_document::expr::references(column)
            .ok()
            .and_then(|refs| refs.into_iter().next())
            .and_then(|r| {
                document
                    .variable_sets()
                    .into_iter()
                    .find(|(_, name, _)| *name == r.object)
                    .and_then(|(_, _, set)| set.variable(&r.property).map(|v| v.formula.clone()))
            })
            .unwrap_or_default()
    };

    group(ui, "In effect");
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let shown = table.active.clone().unwrap_or_else(|| "None".to_string());
        egui::ComboBox::from_id_salt("active_configuration")
            .selected_text(RichText::new(shown).font(sans(FONT_SM)))
            .width((ui.available_width() - 12.0).max(80.0))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(table.active.is_none(), "None")
                    .clicked()
                {
                    commands.push(UiCommand::Config(ConfigEdit::Activate(None)));
                }
                for r in &table.rows {
                    let on = table.active.as_deref() == Some(r.name.as_str());
                    if ui.selectable_label(on, &r.name).clicked() && !on {
                        commands.push(UiCommand::Config(ConfigEdit::Activate(Some(
                            r.name.clone(),
                        ))));
                    }
                }
            });
    });

    group(ui, "Variables they set");
    if table.columns.is_empty() {
        note(ui, "None yet: add the variables the configurations change.");
    }
    for column in &table.columns {
        let response = row(
            ui,
            RichText::new(column).font(mono(FONT_SM)).color(TEXT1),
            RichText::new(own(column)).font(mono(FONT_XS)).color(TEXT3),
            false,
        )
        .on_hover_text("Its own formula, used where a configuration leaves it empty. Right-click to stop configuring it.");
        response.context_menu(|ui| {
            if ui.button("Stop configuring it").clicked() {
                commands.push(UiCommand::Config(ConfigEdit::RemoveVariable(
                    column.clone(),
                )));
                ui.close();
            }
        });
    }
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.menu_button(RichText::new("+ Variable").font(sans(FONT_SM)), |ui| {
            let mut any = false;
            for (_, set_name, set) in document.variable_sets() {
                for v in &set.variables {
                    let reference = format!(
                        "{}.{}",
                        core_document::expr::quote_name(&set_name),
                        core_document::expr::quote_name(&v.name)
                    );
                    if table.columns.contains(&reference) {
                        continue;
                    }
                    any = true;
                    if ui
                        .button(RichText::new(&reference).font(mono(FONT_SM)))
                        .clicked()
                    {
                        commands.push(UiCommand::Config(ConfigEdit::AddVariable(reference)));
                        ui.close();
                    }
                }
            }
            if !any {
                ui.label(
                    RichText::new("Every variable is set here, or there are none yet")
                        .font(sans(FONT_SM))
                        .color(TEXT3),
                );
            }
        });
    });

    group(ui, "Configurations");
    if table.rows.is_empty() {
        note(
            ui,
            "None yet: a configuration is a version of the model, a small or a large one.",
        );
    }
    for r in &table.rows {
        let active = table.active.as_deref() == Some(r.name.as_str());
        let is_open = matches!(&state.open_configuration, Some((n, _)) if *n == r.name);
        let response = row(
            ui,
            RichText::new(&r.name)
                .font(if active {
                    sans_medium(FONT_SM)
                } else {
                    sans(FONT_SM)
                })
                .color(TEXT1),
            RichText::new(if active { "in effect" } else { "" })
                .font(sans(FONT_XS))
                .color(SUCCESS),
            is_open,
        );
        if response.clicked() {
            state.open_configuration = (!is_open).then(|| {
                (
                    r.name.clone(),
                    ConfigDraft {
                        name: r.name.clone(),
                        values: (0..table.columns.len())
                            .map(|i| r.values.get(i).cloned().unwrap_or_default())
                            .collect(),
                    },
                )
            });
        }
        if is_open {
            configuration_editor(ui, state, document, &table, r, &own, commands);
        }
    }

    if !table.rows.is_empty() && !table.columns.is_empty() {
        ui.add_space(SPACE_2);
        group(ui, "Table");
        table_view(ui, &mut state.vertical, &table, &own);
    }

    ui.add_space(SPACE_2);
    group(ui, "Add a configuration");
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 6))
        .show(ui, |ui| {
            let entered = labelled_field(ui, "Name", &mut state.new_configuration, sans(FONT_SM));
            let name = state.new_configuration.trim().to_string();
            let add = ui
                .add_enabled_ui(!name.is_empty(), |ui| small_secondary_button(ui, "Add"))
                .inner
                .clicked();
            if !name.is_empty() && (entered || add) {
                commands.push(UiCommand::Config(ConfigEdit::New {
                    like: table.active.clone(),
                    name,
                }));
                state.new_configuration.clear();
            }
            if table.active.is_some() {
                ui.label(
                    RichText::new("It starts with the values of the one in effect.")
                        .font(sans(FONT_XS))
                        .color(TEXT3),
                );
            }
        });
}

/// Every configuration's values at once, scrolling sideways when wider
/// than the panel; the switch puts the configurations along the top.
fn table_view(
    ui: &mut egui::Ui,
    vertical: &mut bool,
    table: &Configurations,
    own: &dyn Fn(&str) -> String,
) {
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        check_row(ui, vertical, "Configurations as columns")
            .on_hover_text("Lay the table out with a column per configuration, a row per variable");
    });
    let layout = grid(table, *vertical);
    let configuration_name = |name: &str| {
        let active = table.active.as_deref() == Some(name);
        RichText::new(name)
            .font(if active {
                sans_medium(FONT_SM)
            } else {
                sans(FONT_SM)
            })
            .color(if active { SUCCESS } else { TEXT1 })
    };
    let variable_name = |name: &str| RichText::new(name).font(mono(FONT_XS)).color(TEXT2);
    let header = |name: &str, configuration: bool| {
        if configuration {
            configuration_name(name)
        } else {
            variable_name(name)
        }
    };
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 6))
        .show(ui, |ui| {
            egui::ScrollArea::horizontal()
                .id_salt("configurations_table")
                .show(ui, |ui| {
                    egui::Grid::new("configurations_table_grid")
                        .striped(true)
                        .spacing(egui::vec2(SPACE_3, SPACE_1))
                        .show(ui, |ui| {
                            ui.label("");
                            for name in &layout.across {
                                ui.label(header(name, *vertical));
                            }
                            ui.end_row();
                            for (name, cells) in layout.down.iter().zip(&layout.cells) {
                                ui.label(header(name, !*vertical));
                                for cell in cells {
                                    let text = if cell.value.is_empty() {
                                        RichText::new(own(cell.variable))
                                            .font(mono(FONT_XS))
                                            .color(TEXT3)
                                    } else {
                                        RichText::new(cell.value).font(mono(FONT_SM)).color(TEXT1)
                                    };
                                    ui.label(text);
                                }
                                ui.end_row();
                            }
                        });
                });
        });
}

/// An open configuration: its name and a value per variable it sets,
/// kept by Apply or Enter; it can be put in effect or removed.
fn configuration_editor(
    ui: &mut egui::Ui,
    state: &mut VariablesState,
    document: &Document,
    table: &Configurations,
    configuration: &Configuration,
    own: &dyn Fn(&str) -> String,
    commands: &mut Vec<UiCommand>,
) {
    let active = table.active.as_deref() == Some(configuration.name.as_str());
    let Some((_, draft)) = &mut state.open_configuration else {
        return;
    };
    let mut apply = false;
    let mut close = false;
    egui::Frame::new()
        .fill(BG2)
        .inner_margin(egui::Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            apply |= labelled_field(ui, "Name", &mut draft.name, sans(FONT_SM));
            for (column, value) in table.columns.iter().zip(draft.values.iter_mut()) {
                ui.label(RichText::new(column).font(mono(FONT_XS)).color(TEXT2));
                apply |= formula_box(
                    ui,
                    value,
                    document,
                    egui::Id::new(("configuration_value", &configuration.name, column)),
                    Some(&own(column)),
                );
                preview(ui, document, value);
            }
            // The bodies it leaves out, each a tick.
            ui.menu_button(
                RichText::new(format!(
                    "Leaves out {} bod{}",
                    configuration.left_out.len(),
                    if configuration.left_out.len() == 1 {
                        "y"
                    } else {
                        "ies"
                    }
                ))
                .font(sans(FONT_SM)),
                |ui| {
                    for body in document.bodies() {
                        let mut out = configuration.left_out.contains(&body.id);
                        if ui.checkbox(&mut out, &body.name).changed() {
                            let mut bodies = configuration.left_out.clone();
                            if out {
                                bodies.push(body.id);
                            } else {
                                bodies.retain(|b| *b != body.id);
                            }
                            commands.push(UiCommand::Config(ConfigEdit::LeaveOut {
                                name: configuration.name.clone(),
                                bodies,
                            }));
                        }
                    }
                },
            )
            .response
            .on_hover_text(
                "Bodies this configuration leaves out: not drawn, picked, exported or checked \
                 while it is in effect",
            );
            ui.horizontal(|ui| {
                if small_secondary_button(ui, "Apply").clicked() {
                    apply = true;
                }
                if !active && small_secondary_button(ui, "Put in effect").clicked() {
                    commands.push(UiCommand::Config(ConfigEdit::Activate(Some(
                        configuration.name.clone(),
                    ))));
                }
                if small_secondary_button(ui, "Remove").clicked() {
                    commands.push(UiCommand::Config(ConfigEdit::Remove(
                        configuration.name.clone(),
                    )));
                    close = true;
                }
            });
        });
    if apply {
        commands.extend(configuration_changes(table, configuration, draft));
        close = true;
    }
    if close {
        state.open_configuration = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_open_variable_applies_only_what_changed() {
        let set = FeatureId::new();
        let v = Variable {
            name: "nozzle".into(),
            formula: "0.4 mm".into(),
            comment: String::new(),
        };
        let same = Draft {
            name: "nozzle".into(),
            formula: " 0.4 mm ".into(),
            comment: String::new(),
        };
        assert!(variable_changes(set, &v, &same).is_empty());
        let both = Draft {
            name: "bore".into(),
            formula: "0.6 mm".into(),
            comment: "wider".into(),
        };
        let changes = variable_changes(set, &v, &both);
        assert!(
            matches!(
                changes.as_slice(),
                [
                    UiCommand::SetVariable { name, formula, comment: Some(c), .. },
                    UiCommand::RenameVariable { to, .. },
                ] if name == "nozzle" && formula == "0.6 mm" && c == "wider" && to == "bore"
            ),
            "the formula under its old name, then the rename: {changes:?}"
        );
    }

    #[test]
    fn an_open_configuration_applies_its_changed_values_and_name() {
        let table = Configurations {
            columns: vec!["Size.width".into(), "Size.height".into()],
            rows: vec![Configuration {
                name: "S".into(),
                values: vec!["30 mm".into()],
                left_out: Vec::new(),
            }],
            active: None,
        };
        let draft = ConfigDraft {
            name: "Small".into(),
            values: vec!["30 mm".into(), "5 mm".into()],
        };
        let changes = configuration_changes(&table, &table.rows[0], &draft);
        assert!(
            matches!(
                changes.as_slice(),
                [
                    UiCommand::Config(ConfigEdit::Set { variable, value, .. }),
                    UiCommand::Config(ConfigEdit::Rename { to, .. }),
                ] if variable == "Size.height" && value == "5 mm" && to == "Small"
            ),
            "{changes:?}"
        );
    }

    #[test]
    fn the_table_turns_with_configurations_as_columns() {
        let table = Configurations {
            columns: vec!["Size.width".into(), "Size.height".into()],
            rows: vec![
                Configuration {
                    name: "S".into(),
                    values: vec!["30 mm".into()],
                    left_out: Vec::new(),
                },
                Configuration {
                    name: "L".into(),
                    values: vec!["90 mm".into(), "20 mm".into()],
                    left_out: Vec::new(),
                },
            ],
            active: None,
        };
        let rows = grid(&table, false);
        assert_eq!(rows.across, ["Size.width", "Size.height"]);
        assert_eq!(rows.down, ["S", "L"]);
        let columns = grid(&table, true);
        assert_eq!(columns.across, ["S", "L"]);
        assert_eq!(columns.down, ["Size.width", "Size.height"]);
        for (r, row) in rows.cells.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                assert_eq!(*cell, columns.cells[c][r], "the same cell either way");
            }
        }
        assert_eq!(
            rows.cells[0][1],
            Cell {
                variable: "Size.height",
                value: ""
            },
            "a value a configuration leaves out is empty, keeping the variable's own"
        );
        assert_eq!(columns.cells[1][1].value, "20 mm");
    }

    #[test]
    fn both_sections_draw_in_a_narrow_column() {
        let mut doc = Document::new("t");
        let set = doc.add_variable_set("Printer").unwrap();
        doc.set_variable(set, "nozzle", "0.4 mm", Some("the nozzle"))
            .unwrap();
        doc.add_configuration("Small", None).unwrap();
        doc.add_configuration_column("Printer.nozzle").unwrap();
        let mut state = VariablesState {
            open: Some((set, "nozzle".into(), Draft::default())),
            vertical: true,
            open_configuration: Some(("Small".into(), ConfigDraft::default())),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        ui_kit::apply_theme(&ctx);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(260.0, 900.0),
            )),
            ..Default::default()
        };
        let mut commands = Vec::new();
        let mut output = ctx.run_ui(raw, |ui| {
            variable_set_section(ui, &mut state, &doc, set, &mut commands);
            configurations_section(ui, &mut state, &doc, &mut commands);
        });
        output.textures_delta.clear();
        assert!(commands.is_empty());
    }
}
