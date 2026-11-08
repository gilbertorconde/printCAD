//! The dimension editor: the task panel while a dimension is made or
//! edited, opened by a double click on its label or as a tool adds it.

use egui::RichText;

use core_document::{TaskRequest, WorkbenchRuntimeContext};
use ui_kit::tokens::*;
use ui_kit::widgets::{Note, check_row, note_card, secondary_button, select_field};
use ui_kit::{mono, sans};

use crate::SketchWorkbench;
use crate::sketch;
use crate::style::constraint_icon;

impl SketchWorkbench {
    /// The dimension editor opens from a label or a tool's candidate. The
    /// value is typed and Enter applies it; a variable's chip puts its
    /// name in instead, and "Save as variable" puts the value in a new
    /// variable the dimension then reads.
    pub(crate) fn dim_edit_window(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) {
        if self.dim_edit.is_none() {
            return;
        }
        let edited = self.dim_edit.as_ref().and_then(|edit| {
            let kind = self.dimension_kind()?.clone();
            let name = self
                .displayed_sketch(ctx)?
                .sketch
                .constraints
                .iter()
                .find(|c| c.id == edit.constraint)
                .and_then(|c| c.name.clone());
            Some((kind, name))
        });
        // The dimension went (an undo, a delete): so does its editor.
        let Some((kind, constraint_name)) = edited else {
            self.cancel_dim_edit();
            return;
        };
        let unit = sketch::dimension_unit(&kind);
        let dim = crate::params::formula_dim(&kind);
        let kind_label = sketch::constraint_label(&kind);
        let document: &core_document::Document = ctx.document;
        let choices = document.variables_of(dim);
        let sets: Vec<(core_document::FeatureId, String)> = document
            .variable_sets()
            .into_iter()
            .map(|(id, name, _)| (id, name))
            .collect();
        let length_unit = document.display_unit();
        // A new dimension on a line, two lines or two points may be made
        // again as the horizontal or vertical distance, or as the one the
        // Dimension tool picks: the tool it was made with, when it offers that.
        let axis = self.axis_choice(ctx, &kind);
        let mut remake: Option<&'static str> = None;
        let Some(edit) = self.dim_edit.as_mut() else {
            return;
        };
        let title = match constraint_name.filter(|n| !n.trim().is_empty()) {
            Some(name) => name,
            None if edit.new => format!("New {}", kind_label.to_lowercase()),
            None => kind_label.clone(),
        };
        let text_id = egui::Id::new("sketch_dim_edit_text");
        let name_id = egui::Id::new("sketch_dim_edit_name");
        let suffix = unit_suffix(unit);
        let mut commit = request.accept;
        let mut cancel = request.cancel;
        let mut switch: Option<bool> = None;
        let mut arc_switch: Option<bool> = None;
        ui.vertical(|ui| {
            ui.set_max_width(FIELD_WIDTH + SPACE_4);
            ui.spacing_mut().item_spacing.y = SPACE_1;
            ui.horizontal(|ui| {
                ui_kit::icon::draw(ui, constraint_icon(&kind), 14.0, TEXT2);
                ui.label(
                    RichText::new(&title)
                        .font(ui_kit::sans_medium(FONT_SM))
                        .color(TEXT1),
                );
                // A circle's size, as its radius or its diameter.
                let diameter = match kind {
                    sketch::ConstraintKind::Radius { .. } => Some(false),
                    sketch::ConstraintKind::Diameter { .. } => Some(true),
                    _ => None,
                };
                let angle = match kind {
                    sketch::ConstraintKind::ArcLength { .. } => Some(false),
                    sketch::ConstraintKind::ArcAngle { .. } => Some(true),
                    _ => None,
                };
                if let Some(angle) = angle {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        for (label, as_angle, hint) in [
                            ("°", true, "Dimension the arc's sweep, in degrees"),
                            ("L", false, "Dimension the arc's length"),
                        ] {
                            if ui
                                .selectable_label(
                                    angle == as_angle,
                                    RichText::new(label).font(mono(FONT_SM)),
                                )
                                .on_hover_text(hint)
                                .clicked()
                                && angle != as_angle
                            {
                                arc_switch = Some(as_angle);
                            }
                        }
                    });
                }
                if let Some(current) = axis {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        for (label, tool, hint) in [
                            (
                                "Auto",
                                "dimension",
                                "The dimension the Dimension tool gives",
                            ),
                            ("V", "distance_y", "The vertical distance"),
                            ("H", "distance_x", "The horizontal distance"),
                        ] {
                            if ui
                                .selectable_label(
                                    current == tool,
                                    RichText::new(label).font(mono(FONT_SM)),
                                )
                                .on_hover_text(hint)
                                .clicked()
                                && current != tool
                            {
                                remake = Some(tool);
                            }
                        }
                    });
                }
                if let Some(diameter) = diameter {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        for (label, as_diameter, hint) in [
                            ("Ø", true, "Dimension the diameter"),
                            ("R", false, "Dimension the radius"),
                        ] {
                            if ui
                                .selectable_label(
                                    diameter == as_diameter,
                                    RichText::new(label).font(mono(FONT_SM)),
                                )
                                .on_hover_text(hint)
                                .clicked()
                                && diameter != as_diameter
                            {
                                switch = Some(as_diameter);
                            }
                        }
                    });
                }
            });

            // The value, or a formula; names complete as they
            // are typed.
            let value_label = ui_kit::widgets::field_label(ui, "Value or formula");
            let completed = ui_kit::completion::completing_text_edit(
                ui,
                text_id,
                &mut edit.text,
                &|| core_document::formula_candidates(document),
                |e| {
                    e.desired_width(FIELD_WIDTH)
                        .font(mono(FONT_MD))
                        .hint_text("A value, or a formula")
                },
            );
            let response = completed.response.labelled_by(value_label.id);
            if edit.select_all {
                // The first key typed takes the whole value.
                response.request_focus();
                let n = edit.text.chars().count();
                select_chars(ui, text_id, 0, n);
                edit.select_all = false;
            } else if edit.save_as.is_none() && !response.has_focus() && !response.lost_focus() {
                // Focus stays here unless the name has it; asked
                // only while it is elsewhere, so the input
                // method's composition is left alone.
                response.request_focus();
            }
            let entered = response.lost_focus()
                && !completed.picked
                && ui.input(|i| i.key_pressed(egui::Key::Enter));

            // What it comes to, as typed.
            let typed = edit.text.trim().to_string();
            let typed = typed.as_str();
            let constant = core_document::expr::is_constant(typed);
            let preview = document.evaluate_formula(typed, Some(dim));
            let (line, color) = match &preview {
                Ok(q) if constant => (format!("= {}{suffix}", fmt_value(q.value)), TEXT3),
                Ok(q) => (
                    format!("ƒ = {}{suffix}", fmt_value(q.value)),
                    SKETCH_FORMULA,
                ),
                Err(_) if typed.is_empty() => (String::new(), TEXT3),
                Err(why) => (why.clone(), DANGER),
            };
            if !line.is_empty() {
                ui.label(RichText::new(line).font(sans(FONT_XS)).color(color));
            }

            // The variables it can read, one click each.
            let shown: Vec<_> = choices.iter().take(MAX_CHIPS).collect();
            if !shown.is_empty() {
                ui.add_space(SPACE_1);
                ui_kit::widgets::overline(ui, "Variables");
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(SPACE_1, SPACE_1);
                    for choice in &shown {
                        let label = if sets.len() > 1 {
                            &choice.reference
                        } else {
                            &choice.name
                        };
                        let chip = ui
                            .selectable_label(
                                typed == choice.reference,
                                RichText::new(format!(
                                    "{label}  {}",
                                    choice.value.display(length_unit, 3)
                                ))
                                .font(mono(FONT_XS)),
                            )
                            .on_hover_text(format!(
                                "Set by {}: follows it when it changes",
                                choice.reference
                            ));
                        if chip.clicked() {
                            edit.text = choice.reference.clone();
                            edit.save_as = None;
                            let n = edit.text.chars().count();
                            select_chars(ui, text_id, n, n);
                            ui.memory_mut(|m| m.request_focus(text_id));
                        }
                    }
                });
                if choices.len() > shown.len() {
                    ui.label(
                        RichText::new("Type a name for the rest")
                            .font(sans(FONT_XS))
                            .color(TEXT3),
                    );
                }
            }

            // A new variable, from what is typed.
            let reads_a_variable = choices.iter().any(|c| c.reference == edit.text.trim());
            let mut name_ok = true;
            if edit.save_as.is_none() && !reads_a_variable {
                ui.add_space(SPACE_1);
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("+ Save as variable")
                                .font(sans(FONT_XS))
                                .color(ACCENT),
                        )
                        .frame(false),
                    )
                    .on_hover_text("Keep this value in a variable, and have the dimension read it")
                    .clicked()
                {
                    let set = sets.last().map(|(id, _)| *id);
                    let base = variable_base_name(&kind_label);
                    edit.save_as = Some(crate::SaveAs {
                        name: document.unused_variable_name(set, &base),
                        set,
                    });
                    ui.memory_mut(|m| m.request_focus(name_id));
                }
            }
            if let Some(save) = edit.save_as.as_mut() {
                ui.add_space(SPACE_1);
                ui_kit::widgets::overline(ui, "Save as variable");
                let mut close = false;
                ui.horizontal(|ui| {
                    let name = ui.add(
                        egui::TextEdit::singleline(&mut save.name)
                            .id(name_id)
                            .desired_width(if sets.is_empty() {
                                FIELD_WIDTH - 24.0
                            } else {
                                FIELD_WIDTH * 0.5
                            })
                            .font(mono(FONT_SM))
                            .hint_text("name"),
                    );
                    if name.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        commit = true;
                    }
                    if !sets.is_empty() {
                        let mut options: Vec<(Option<core_document::FeatureId>, &str)> =
                            sets.iter().map(|(id, n)| (Some(*id), n.as_str())).collect();
                        options.push((None, "New set"));
                        select_field(
                            ui,
                            "sketch_dim_edit_set",
                            &mut save.set,
                            &options,
                            FIELD_WIDTH * 0.5 - 30.0,
                        );
                    }
                    if ui
                        .add(egui::Button::new(RichText::new("×").color(TEXT3)).frame(false))
                        .on_hover_text("Keep the value in the dimension only")
                        .clicked()
                    {
                        close = true;
                    }
                });
                let name = save.name.trim();
                let set_name = match save.set {
                    Some(set) => sets
                        .iter()
                        .find(|(id, _)| *id == set)
                        .map(|(_, n)| n.clone())
                        .unwrap_or_default(),
                    None => document.unused_object_name("Variables"),
                };
                let exists = choices
                    .iter()
                    .any(|c| Some(c.set) == save.set && c.name == name);
                let (note, color) = if !core_document::expr::is_valid_name(name) {
                    name_ok = false;
                    ("Give it a name".to_string(), DANGER)
                } else if exists {
                    (
                        format!("{set_name}.{name} exists: it takes this value"),
                        WARNING,
                    )
                } else if save.set.is_none() {
                    (
                        format!("Makes the set {set_name}; formulas read {set_name}.{name}"),
                        TEXT3,
                    )
                } else {
                    (format!("Formulas read it as {set_name}.{name}"), TEXT3)
                };
                ui.label(RichText::new(note).font(sans(FONT_XS)).color(color));
                if close {
                    edit.save_as = None;
                    ui.memory_mut(|m| m.request_focus(text_id));
                }
            }

            // A plain value can stand as a reference dimension;
            // one a formula sets drives.
            if constant && edit.save_as.is_none() {
                let response = check_row(ui, &mut edit.driving, "Driving")
                    .on_hover_text("Off = reference dimension (measured, not enforced)");
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::Checkbox,
                        ui.is_enabled(),
                        edit.driving,
                        "Driving",
                    )
                });
            }

            let valid = preview.is_ok() && name_ok;
            if entered && valid {
                commit = true;
            }
            if edit.new {
                ui.label(
                    RichText::new("Apply adds the dimension · Esc cancels")
                        .font(sans(FONT_XS))
                        .color(TEXT3),
                );
            }
            if commit && !valid {
                commit = false;
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel = true;
            }
        });
        if let Some(tool) = remake {
            self.remake_new_dimension(ctx, tool);
            return;
        }
        if let Some(diameter) = switch {
            self.switch_edited_dimension(ctx, diameter);
        }
        if let Some(angle) = arc_switch {
            self.switch_edited_arc_dimension(ctx, angle);
        }
        self.recalculate_dimension(ctx);
        let valid = self
            .session
            .dimension()
            .is_some_and(|s| s.draft.working.is_ok());
        if let Some(Err(reason)) = self.session.dimension().map(|s| &s.draft.working) {
            note_card(ui, Note::Warning, None, reason);
        }
        ui.horizontal(|ui| {
            commit |= ui
                .add_enabled(valid, |ui: &mut egui::Ui| {
                    ui_kit::widgets::primary_button(ui, "Apply")
                })
                .clicked();
            cancel |= secondary_button(ui, "Cancel").clicked();
        });
        if cancel {
            self.cancel_dim_edit();
        } else if commit && valid {
            self.commit_dim_edit(ctx);
        }
    }
}

/// How wide the dimension editor's value field is.
const FIELD_WIDTH: f32 = 220.0;

/// How many variables the dimension editor offers as chips.
const MAX_CHIPS: usize = 12;

/// A value as the dimension editor writes it: up to four places, no
/// trailing zeros.
fn fmt_value(value: f64) -> String {
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

/// What a variable made from a dimension of kind `label` is first called:
/// `Distance X` gives `distance_x`.
fn variable_base_name(label: &str) -> String {
    let name: String = label
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let name = name.trim_matches('_').to_string();
    if name.is_empty() {
        "value".to_string()
    } else {
        name
    }
}

/// Select characters `from..to` of text edit `id`.
fn select_chars(ui: &egui::Ui, id: egui::Id, from: usize, to: usize) {
    let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(from),
            egui::text::CCursor::new(to),
        )));
    state.store(ui.ctx(), id);
}

/// What follows a dimension's value when it is written out.
fn unit_suffix(unit: sketch::DimensionUnit) -> &'static str {
    match unit {
        sketch::DimensionUnit::Length => " mm",
        sketch::DimensionUnit::Angle => "°",
        sketch::DimensionUnit::Ratio => "",
    }
}
