//! The Variables panel at the bottom: one tab per variable set, and in
//! it the set's variables with their formulas, what each comes to, and
//! their comments.
//!
//! A cell edits on a click and keeps on Enter or a click away; Escape
//! leaves it. The row at the end adds a variable. The panel reads the
//! document and answers with commands.

use core_document::{Document, FeatureId};
use egui::RichText;
use ui_kit::tokens::*;
use ui_kit::widgets::{Tab, small_secondary_button, tab_plus};
use ui_kit::{mono, sans};

use super::UiCommand;

/// Which cell of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Name,
    Formula,
    Comment,
}

/// The panel's own state.
#[derive(Debug, Default)]
pub struct VariablesState {
    pub open: bool,
    /// The set on screen.
    active: Option<FeatureId>,
    /// The cell being edited: its set, variable, column and text.
    editing: Option<(FeatureId, String, Column, String)>,
    /// What the adding row holds.
    new_name: String,
    new_formula: String,
    /// The Configurations tab is on screen rather than a set.
    configurations: bool,
    /// The configuration cell being edited: its row, the column (`None`
    /// for the row's name) and the text.
    config_editing: Option<(String, Option<String>, String)>,
    /// A new configuration's name, as typed.
    new_configuration: String,
}

impl VariablesState {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Open on `set`.
    pub fn show_set(&mut self, set: FeatureId) {
        self.open = true;
        self.active = Some(set);
        self.configurations = false;
    }

    /// Open on the configurations.
    pub fn show_configurations(&mut self) {
        self.open = true;
        self.configurations = true;
    }
}

pub fn draw_variables(
    ui: &mut egui::Ui,
    state: &mut VariablesState,
    document: &Document,
    commands: &mut Vec<UiCommand>,
) {
    if !state.open {
        return;
    }
    let sets = document.variable_sets();
    if !state
        .active
        .is_some_and(|id| sets.iter().any(|(s, ..)| *s == id))
    {
        state.active = sets.first().map(|(id, ..)| *id);
    }
    egui::Panel::bottom("variables_panel")
        .resizable(true)
        .default_size(220.0)
        .min_size(120.0)
        .frame(
            egui::Frame::new()
                .fill(BG1)
                .inner_margin(egui::Margin::symmetric(10, 6)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (id, name, _) in &sets {
                    let on = !state.configurations && state.active == Some(*id);
                    let tab = Tab::new(name, on).show(ui);
                    if tab.selected {
                        state.active = Some(*id);
                        state.configurations = false;
                        state.editing = None;
                    }
                }
                if Tab::new("Configurations", state.configurations)
                    .show(ui)
                    .selected
                {
                    state.configurations = true;
                    state.config_editing = None;
                }
                if tab_plus(ui, TAB_BAR - 8.0)
                    .on_hover_text("New variable set")
                    .clicked()
                {
                    commands.push(UiCommand::NewVariableSet);
                }
                ui.add_space(SPACE_3);
                active_selector(ui, document, commands);
                ui.label(
                    RichText::new(
                        "Formulas read a variable as Set.name · units: mm, in, deg · \
                         click a cell to edit",
                    )
                    .font(sans(FONT_XS))
                    .color(TEXT3),
                );
            });
            ui.separator();
            if state.configurations {
                draw_configurations(ui, state, document, commands);
                return;
            }
            let Some((set, set_name, variables)) = sets
                .iter()
                .find(|(id, ..)| Some(*id) == state.active)
                .cloned()
            else {
                ui.add_space(SPACE_3);
                ui.label(
                    RichText::new(
                        "No variables yet. A variable set holds named values, such as a \
                         printer's nozzle or a part's wall, that any number in the model \
                         can follow.",
                    )
                    .font(sans(FONT_SM))
                    .color(TEXT2),
                );
                if small_secondary_button(ui, "New variable set").clicked() {
                    commands.push(UiCommand::NewVariableSet);
                }
                return;
            };
            let slots = document.evaluated_slots(set);
            let unit = document.display_unit();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new(("variables", set))
                        .num_columns(5)
                        .spacing([SPACE_4, SPACE_1])
                        .striped(true)
                        .show(ui, |ui| {
                            for title in ["Name", "Formula", "Value", "Comment", ""] {
                                ui.label(RichText::new(title).font(sans(FONT_XS)).color(TEXT3));
                            }
                            ui.end_row();
                            for v in &variables.variables {
                                let result = slots
                                    .iter()
                                    .find(|s| s.key == v.name)
                                    .map(|s| s.result.clone());
                                cell(
                                    ui,
                                    state,
                                    set,
                                    &v.name,
                                    Column::Name,
                                    &v.name,
                                    commands,
                                    &v.formula,
                                );
                                cell(
                                    ui,
                                    state,
                                    set,
                                    &v.name,
                                    Column::Formula,
                                    &v.formula,
                                    commands,
                                    &v.formula,
                                );
                                match result {
                                    Some(Ok(q)) => {
                                        ui.label(
                                            RichText::new(q.display(unit, 4))
                                                .font(mono(FONT_SM))
                                                .color(TEXT1),
                                        )
                                        .on_hover_text(
                                            format!(
                                                "{}.{}",
                                                core_document::expr::quote_name(&set_name),
                                                core_document::expr::quote_name(&v.name)
                                            ),
                                        );
                                    }
                                    Some(Err(why)) => {
                                        ui.label(
                                            RichText::new("error")
                                                .font(sans(FONT_SM))
                                                .color(DANGER),
                                        )
                                        .on_hover_text(why);
                                    }
                                    None => {
                                        ui.label("");
                                    }
                                }
                                cell(
                                    ui,
                                    state,
                                    set,
                                    &v.name,
                                    Column::Comment,
                                    &v.comment,
                                    commands,
                                    &v.formula,
                                );
                                if ui
                                    .add(
                                        egui::Button::new(RichText::new("×").font(sans(FONT_MD)))
                                            .frame(false),
                                    )
                                    .on_hover_text("Remove this variable")
                                    .clicked()
                                {
                                    commands.push(UiCommand::RemoveVariable {
                                        set,
                                        name: v.name.clone(),
                                    });
                                }
                                ui.end_row();
                            }
                        });
                    ui.add_space(SPACE_2);
                    adding_row(ui, state, set, commands);
                });
        });
}

/// One cell of a variable's row: its text, or its editor while it is the
/// one being edited.
#[allow(clippy::too_many_arguments)]
fn cell(
    ui: &mut egui::Ui,
    state: &mut VariablesState,
    set: FeatureId,
    variable: &str,
    column: Column,
    text: &str,
    commands: &mut Vec<UiCommand>,
    formula: &str,
) {
    let font = if column == Column::Formula {
        mono(FONT_SM)
    } else {
        sans(FONT_SM)
    };
    let editing_here = matches!(
        &state.editing,
        Some((s, v, c, _)) if *s == set && v == variable && *c == column
    );
    if !editing_here {
        let shown = if text.is_empty() && column == Column::Comment {
            RichText::new("add a comment").font(font).color(TEXT3)
        } else {
            RichText::new(text)
                .font(font)
                .color(if column == Column::Comment {
                    TEXT2
                } else {
                    TEXT1
                })
        };
        if ui
            .add(egui::Label::new(shown).sense(egui::Sense::click()))
            .on_hover_cursor(egui::CursorIcon::Text)
            .clicked()
        {
            state.editing = Some((set, variable.to_string(), column, text.to_string()));
        }
        return;
    }
    let Some((_, _, _, buffer)) = &mut state.editing else {
        return;
    };
    let id = egui::Id::new(("variable_cell", set, variable, column as u8));
    let resp = ui.add(
        egui::TextEdit::singleline(buffer)
            .id(id)
            .font(font)
            .desired_width(if column == Column::Formula {
                220.0
            } else {
                140.0
            }),
    );
    if !resp.has_focus() && !resp.lost_focus() {
        resp.request_focus();
    }
    if resp.lost_focus() {
        let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let typed = buffer.trim().to_string();
        state.editing = None;
        if escaped || typed == text {
            return;
        }
        commands.push(match column {
            Column::Name => UiCommand::RenameVariable {
                set,
                name: variable.to_string(),
                to: typed,
            },
            Column::Formula => UiCommand::SetVariable {
                set,
                name: variable.to_string(),
                formula: typed,
                comment: None,
            },
            Column::Comment => UiCommand::SetVariable {
                set,
                name: variable.to_string(),
                formula: formula.to_string(),
                comment: Some(typed),
            },
        });
    }
}

/// The row that adds a variable: its name and formula, then Enter or Add.
fn adding_row(
    ui: &mut egui::Ui,
    state: &mut VariablesState,
    set: FeatureId,
    commands: &mut Vec<UiCommand>,
) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.new_name)
                .hint_text("name")
                .font(sans(FONT_SM))
                .desired_width(120.0),
        );
        let formula = ui.add(
            egui::TextEdit::singleline(&mut state.new_formula)
                .hint_text("formula, such as 0.4 mm")
                .font(mono(FONT_SM))
                .desired_width(220.0),
        );
        let ready = !state.new_name.trim().is_empty() && !state.new_formula.trim().is_empty();
        let entered = formula.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let add = ui
            .add_enabled(
                ready,
                egui::Button::new(RichText::new("Add").font(sans(FONT_SM))),
            )
            .clicked();
        if ready && (entered || add) {
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

/// Which configuration is in effect, to switch from any tab.
fn active_selector(ui: &mut egui::Ui, document: &Document, commands: &mut Vec<UiCommand>) {
    let Some((_, table)) = document.configurations() else {
        return;
    };
    if table.rows.is_empty() {
        return;
    }
    let shown = table.active.clone().unwrap_or_else(|| "None".to_string());
    egui::ComboBox::from_id_salt("active_configuration")
        .selected_text(RichText::new(format!("Configuration: {shown}")).font(sans(FONT_SM)))
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(table.active.is_none(), "None")
                .clicked()
            {
                commands.push(UiCommand::Config(super::ConfigEdit::Activate(None)));
            }
            for row in &table.rows {
                let on = table.active.as_deref() == Some(row.name.as_str());
                if ui.selectable_label(on, &row.name).clicked() && !on {
                    commands.push(UiCommand::Config(super::ConfigEdit::Activate(Some(
                        row.name.clone(),
                    ))));
                }
            }
        });
}

/// The configurations: a row per configuration, a column per variable it
/// sets, the one in effect marked.
fn draw_configurations(
    ui: &mut egui::Ui,
    state: &mut VariablesState,
    document: &Document,
    commands: &mut Vec<UiCommand>,
) {
    use super::ConfigEdit;
    let table = document
        .configurations()
        .map(|(_, t)| t)
        .unwrap_or_default();
    let own = |column: &str| -> String {
        let Ok(refs) = core_document::expr::references(column) else {
            return String::new();
        };
        let Some(r) = refs.first() else {
            return String::new();
        };
        document
            .variable_sets()
            .into_iter()
            .find(|(_, name, _)| *name == r.object)
            .and_then(|(_, _, set)| set.variable(&r.property).map(|v| v.formula.clone()))
            .unwrap_or_default()
    };
    if table.rows.is_empty() && table.columns.is_empty() {
        ui.label(
            RichText::new(
                "Configurations are versions of the model, a small and a large one, each \
                 giving some variables values of its own. Add a configuration, then the \
                 variables it sets.",
            )
            .font(sans(FONT_SM))
            .color(TEXT2),
        );
        ui.add_space(SPACE_2);
    }
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("configurations")
                .spacing([SPACE_4, SPACE_1])
                .striped(true)
                .show(ui, |ui| {
                    ui.label("");
                    ui.label(
                        RichText::new("Configuration")
                            .font(sans(FONT_XS))
                            .color(TEXT3),
                    );
                    for column in &table.columns {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(column).font(mono(FONT_XS)).color(TEXT2));
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("×").font(sans(FONT_SM)))
                                        .frame(false),
                                )
                                .on_hover_text("Stop configuring this variable")
                                .clicked()
                            {
                                commands.push(UiCommand::Config(ConfigEdit::RemoveVariable(
                                    column.clone(),
                                )));
                            }
                        });
                    }
                    ui.menu_button(RichText::new("+ variable").font(sans(FONT_XS)), |ui| {
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
                                    commands.push(UiCommand::Config(ConfigEdit::AddVariable(
                                        reference,
                                    )));
                                    ui.close();
                                }
                            }
                        }
                        if !any {
                            ui.label(
                                RichText::new("No variables left: add some in a variable set")
                                    .font(sans(FONT_SM))
                                    .color(TEXT3),
                            );
                        }
                    });
                    ui.end_row();

                    for row in &table.rows {
                        let active = table.active.as_deref() == Some(row.name.as_str());
                        if ui
                            .radio(active, "")
                            .on_hover_text("Put this configuration in effect")
                            .clicked()
                        {
                            commands.push(UiCommand::Config(ConfigEdit::Activate(
                                (!active).then(|| row.name.clone()),
                            )));
                        }
                        config_cell(ui, state, &row.name, None, &row.name, "", commands);
                        for (i, column) in table.columns.iter().enumerate() {
                            let value = row.values.get(i).cloned().unwrap_or_default();
                            config_cell(
                                ui,
                                state,
                                &row.name,
                                Some(column),
                                &value,
                                &own(column),
                                commands,
                            );
                        }
                        if ui
                            .add(
                                egui::Button::new(RichText::new("×").font(sans(FONT_MD)))
                                    .frame(false),
                            )
                            .on_hover_text("Remove this configuration")
                            .clicked()
                        {
                            commands.push(UiCommand::Config(ConfigEdit::Remove(row.name.clone())));
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(SPACE_2);
            ui.horizontal(|ui| {
                let field = ui.add(
                    egui::TextEdit::singleline(&mut state.new_configuration)
                        .hint_text("new configuration, such as Large")
                        .font(sans(FONT_SM))
                        .desired_width(200.0),
                );
                let name = state.new_configuration.trim().to_string();
                let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let add = ui
                    .add_enabled(
                        !name.is_empty(),
                        egui::Button::new(RichText::new("Add").font(sans(FONT_SM))),
                    )
                    .clicked();
                if !name.is_empty() && (entered || add) {
                    commands.push(UiCommand::Config(ConfigEdit::New {
                        like: table.active.clone(),
                        name,
                    }));
                    state.new_configuration.clear();
                }
            });
        });
}

/// One cell of the table: a row's name (`column` is `None`) or its value
/// for a variable, empty showing the variable's own formula, dimmed.
fn config_cell(
    ui: &mut egui::Ui,
    state: &mut VariablesState,
    row: &str,
    column: Option<&str>,
    text: &str,
    own: &str,
    commands: &mut Vec<UiCommand>,
) {
    use super::ConfigEdit;
    let here = matches!(
        &state.config_editing,
        Some((r, c, _)) if r == row && c.as_deref() == column
    );
    let font = if column.is_some() {
        mono(FONT_SM)
    } else {
        sans(FONT_SM)
    };
    if !here {
        let shown = if text.is_empty() {
            RichText::new(own).font(font).color(TEXT3)
        } else {
            RichText::new(text).font(font).color(TEXT1)
        };
        let response = ui
            .add(egui::Label::new(shown).sense(egui::Sense::click()))
            .on_hover_cursor(egui::CursorIcon::Text);
        let response = if text.is_empty() && column.is_some() {
            response
                .on_hover_text("The variable's own formula: click to give this configuration one")
        } else {
            response
        };
        if response.clicked() {
            state.config_editing = Some((
                row.to_string(),
                column.map(str::to_string),
                text.to_string(),
            ));
        }
        return;
    }
    let Some((_, _, buffer)) = &mut state.config_editing else {
        return;
    };
    let resp = ui.add(
        egui::TextEdit::singleline(buffer)
            .id(egui::Id::new(("configuration_cell", row, column)))
            .font(font)
            .desired_width(140.0),
    );
    if !resp.has_focus() && !resp.lost_focus() {
        resp.request_focus();
    }
    if resp.lost_focus() {
        let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let typed = buffer.trim().to_string();
        state.config_editing = None;
        if escaped || typed == text {
            return;
        }
        commands.push(UiCommand::Config(match column {
            None => ConfigEdit::Rename {
                name: row.to_string(),
                to: typed,
            },
            Some(variable) => ConfigEdit::Set {
                name: row.to_string(),
                variable: variable.to_string(),
                value: typed,
            },
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_formula_cell_edits_and_enter_sets_the_variable() {
        let mut doc = Document::new("t");
        let set = doc.add_variable_set("Printer").unwrap();
        doc.set_variable(set, "nozzle", "0.4 mm", None).unwrap();
        let mut state = VariablesState {
            open: true,
            ..Default::default()
        };
        state.editing = Some((set, "nozzle".into(), Column::Formula, "0.6 mm".into()));
        let ctx = egui::Context::default();
        ui_kit::apply_theme(&ctx);
        let mut commands = Vec::new();
        for events in [
            Vec::new(),
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 700.0),
                )),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                draw_variables(ui, &mut state, &doc, &mut commands)
            });
            output.textures_delta.clear();
        }
        assert!(
            matches!(
                commands.as_slice(),
                [UiCommand::SetVariable { name, formula, comment: None, .. }]
                    if name == "nozzle" && formula == "0.6 mm"
            ),
            "{commands:?}"
        );
        assert!(state.editing.is_none());
    }
}
