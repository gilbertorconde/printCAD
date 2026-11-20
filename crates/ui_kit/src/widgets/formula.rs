//! The formula field: a quantity that also takes a formula, shown as its
//! value until it is edited.

use super::*;

/// What a [`FormulaField`] asks of whoever shows it: how typed text reads.
pub trait FormulaHost {
    /// The value `text` gives this field, in the field's own unit, or why
    /// it gives none.
    fn evaluate(&self, text: &str) -> Result<f64, String>;
    /// Whether `text` reads nothing but numbers: it becomes the value
    /// rather than a formula.
    fn is_constant(&self, text: &str) -> bool;
    /// The names a formula can read, as it writes them
    /// (`Printer.nozzle`), each with what it is, for completion.
    fn candidates(&self) -> Vec<crate::completion::Candidate>;
}

/// What a [`FormulaField`] was set to this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum FormulaEdit {
    /// A number: dragged, typed, or a formula taken away.
    Value(f64),
    /// A formula that reads something.
    Formula(String),
}

/// A number that a formula may set. Without one it drags and takes typed
/// quantities (`1 in`, `2 * 3 mm`); the `fx` beside it, or a click on a
/// bound field, opens the formula, where Enter keeps it, Escape leaves it,
/// Tab takes the first suggested name, and an empty formula leaves the
/// number as it stands.
pub struct FormulaField<'a> {
    id: egui::Id,
    value: f64,
    formula: Option<&'a str>,
    error: Option<&'a str>,
    unit: &'a str,
    speed: f64,
    decimals: usize,
    width: f32,
    /// The whole widget's width, the `fx` button included, when it fills a
    /// given space.
    fill: Option<f32>,
    /// What a drag or the wheel may set.
    drag_range: std::ops::RangeInclusive<f64>,
    host: &'a dyn FormulaHost,
}

impl<'a> FormulaField<'a> {
    pub fn new(id: egui::Id, value: f64, host: &'a dyn FormulaHost) -> Self {
        Self {
            id: id.with("formula_field"),
            value,
            formula: None,
            error: None,
            unit: "",
            speed: 0.1,
            decimals: 2,
            width: 140.0,
            fill: None,
            drag_range: f64::NEG_INFINITY..=f64::INFINITY,
            host,
        }
    }

    /// What a drag or the wheel may set: they stop at its ends, where a
    /// typed value or a formula may go past.
    pub fn drag_range(mut self, range: std::ops::RangeInclusive<f64>) -> Self {
        self.drag_range = range;
        self
    }

    pub fn formula(mut self, formula: Option<&'a str>) -> Self {
        self.formula = formula;
        self
    }

    pub fn error(mut self, error: Option<&'a str>) -> Self {
        self.error = error;
        self
    }

    pub fn unit(mut self, unit: &'a str) -> Self {
        self.unit = unit;
        self
    }

    pub fn speed(mut self, speed: f64) -> Self {
        self.speed = speed;
        self
    }

    pub fn decimals(mut self, decimals: usize) -> Self {
        self.decimals = decimals;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Spans exactly `total`, border, padding and the `fx` button included,
    /// so a row of fields never reaches past the space it was given.
    pub fn fill(mut self, total: f32) -> Self {
        self.fill = Some(total);
        self
    }

    /// The field frame's border and padding, both sides.
    const FRAME: f32 = 2.0 * SPACE_2 + 2.0;

    /// The width inside the field's frame; `button` when the `fx` button
    /// sits beside it.
    fn inner_width(&self, ui: &Ui, button: bool) -> f32 {
        let Some(total) = self.fill else {
            return self.width;
        };
        let beside = if button {
            SPACE_1 + 12.0 + 2.0 * ui.spacing().button_padding.x
        } else {
            0.0
        };
        (total - Self::FRAME - beside).max(24.0)
    }

    pub fn show(self, ui: &mut Ui) -> Option<FormulaEdit> {
        self.show_focused(ui).0
    }

    /// [`Self::show`], and whether the field holds keyboard focus: its
    /// number being typed, or its formula.
    pub fn show_focused(self, ui: &mut Ui) -> (Option<FormulaEdit>, bool) {
        let editing: Option<String> = ui.data(|d| d.get_temp(self.id));
        match editing {
            Some(text) => self.show_editing(ui, text),
            None => self.show_value(ui),
        }
    }

    fn start_editing(&self, ui: &Ui, text: String) {
        ui.data_mut(|d| d.insert_temp(self.id, text));
        ui.memory_mut(|m| m.request_focus(self.id.with("text")));
    }

    fn stop_editing(&self, ui: &Ui) {
        ui.data_mut(|d| d.remove::<String>(self.id));
    }

    fn frame(&self, border: Color32) -> Frame {
        Frame::new()
            .fill(BG2)
            .stroke(Stroke::new(1.0, border))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(Margin::symmetric(SPACE_2 as i8, 0))
    }

    fn show_value(self, ui: &mut Ui) -> (Option<FormulaEdit>, bool) {
        let mut out = None;
        let mut focused = false;
        let border = match (self.error, self.formula) {
            (Some(_), _) => DANGER,
            (None, Some(_)) => ACCENT,
            (None, None) => BORDER,
        };
        let suffix = if self.unit.is_empty() {
            String::new()
        } else {
            format!(" {}", self.unit)
        };
        let inner = self.inner_width(ui, true);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_1;
            let field = self.frame(border).show(ui, |ui| {
                ui.set_min_size(Vec2::new(inner, INPUT - 2.0));
                ui.set_max_width(inner);
                // An input's height, as for the plain number field.
                ui.set_max_height(INPUT - 2.0);
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_1;
                    ui.style_mut().override_font_id = Some(mono(FONT_SM));
                    match self.formula {
                        Some(_) => {
                            icon::draw(ui, "expression", 12.0, ACCENT);
                            let text = format!("{:.*}{suffix}", self.decimals, self.value);
                            ui.label(
                                RichText::new(text)
                                    .font(mono(FONT_SM))
                                    .color(if self.error.is_some() { DANGER } else { TEXT1 }),
                            );
                        }
                        None => {
                            let visuals = ui.visuals_mut();
                            for w in [
                                &mut visuals.widgets.inactive,
                                &mut visuals.widgets.hovered,
                                &mut visuals.widgets.active,
                            ] {
                                w.bg_fill = Color32::TRANSPARENT;
                                w.weak_bg_fill = Color32::TRANSPARENT;
                                w.bg_stroke = Stroke::NONE;
                            }
                            visuals.override_text_color = Some(TEXT1);
                            let mut v = self.value;
                            let host = self.host;
                            let resp = ui.add(
                                egui::DragValue::new(&mut v)
                                    .speed(self.speed)
                                    .fixed_decimals(self.decimals)
                                    .suffix(suffix.clone())
                                    .custom_parser(move |text| {
                                        host.is_constant(text)
                                            .then(|| host.evaluate(text).ok())
                                            .flatten()
                                    }),
                            );
                            focused = resp.has_focus();
                            hold_dragged(ui, &resp, &mut v, &self.drag_range);
                            if let Some(w) =
                                wheel_step(ui, &resp, v, self.decimals, &self.drag_range)
                            {
                                out = Some(FormulaEdit::Value(w));
                            } else if resp.changed() {
                                out = Some(FormulaEdit::Value(v));
                            }
                        }
                    }
                });
            });
            let bound_click = self.formula.is_some()
                && ui
                    .interact(field.response.rect, self.id.with("open"), Sense::click())
                    .clicked();
            let button = match icon::image(ui.ctx(), "expression", 12.0, TEXT2) {
                Some(image) => egui::Button::image(image),
                None => egui::Button::new(RichText::new("fx").font(mono(FONT_XS))),
            };
            let fx = ui.add(button).on_hover_text(match self.formula {
                Some(f) => format!("Formula: {f}"),
                None => "Set by a formula".to_string(),
            });
            if bound_click || fx.clicked() {
                let text = self
                    .formula
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{:.*}", self.decimals, self.value));
                self.start_editing(ui, text);
            }
            if let Some(err) = self.error {
                field.response.on_hover_text(err);
            }
        });
        (out, focused)
    }

    fn show_editing(self, ui: &mut Ui, mut text: String) -> (Option<FormulaEdit>, bool) {
        let text_id = self.id.with("text");
        let host = self.host;
        let mut out = None;
        let mut focused = false;
        let mut done = false;
        let inner = self.inner_width(ui, false);
        // Filling a space, the formula stays inside it; a free field lets a
        // long formula reach wider than the number it replaces.
        let text_width = match self.fill {
            Some(_) => inner - 12.0 - ui.spacing().item_spacing.x,
            None => self.width * 1.6,
        };
        ui.vertical(|ui| {
            let mut preview = host.evaluate(&text);
            self.frame(if preview.is_err() { DANGER } else { ACCENT })
                .show(ui, |ui| {
                    ui.set_min_size(Vec2::new(inner, INPUT - 2.0));
                    if self.fill.is_some() {
                        ui.set_max_width(inner);
                    }
                    ui.horizontal_centered(|ui| {
                        icon::draw(ui, "expression", 12.0, ACCENT);
                        let edit = crate::completion::completing_text_edit(
                            ui,
                            text_id,
                            &mut text,
                            &|| host.candidates(),
                            |edit| {
                                edit.frame(Frame::NONE)
                                    .font(mono(FONT_SM))
                                    .desired_width(text_width)
                            },
                        );
                        let resp = edit.response;
                        focused = resp.has_focus();
                        // A name picked from the dropdown is not the end
                        // of the formula: the edit gets the keyboard back.
                        if resp.lost_focus() && !edit.picked {
                            let (enter, escape) = ui.input(|i| {
                                (
                                    i.key_pressed(egui::Key::Enter),
                                    i.key_pressed(egui::Key::Escape),
                                )
                            });
                            if escape {
                                done = true;
                            } else if enter || !resp.has_focus() {
                                let typed = text.trim();
                                if typed.is_empty() {
                                    if self.formula.is_some() {
                                        out = Some(FormulaEdit::Value(self.value));
                                    }
                                    done = true;
                                } else if host.is_constant(typed) {
                                    if let Ok(v) = host.evaluate(typed) {
                                        out = Some(FormulaEdit::Value(v));
                                        done = true;
                                    }
                                } else {
                                    out = Some(FormulaEdit::Formula(typed.to_string()));
                                    done = true;
                                }
                            }
                        }
                    });
                });
            preview = host.evaluate(&text);
            let (line, color) = match &preview {
                Ok(v) => (format!("= {v:.*} {}", self.decimals, self.unit), TEXT3),
                Err(why) => (why.clone(), DANGER),
            };
            ui.label(RichText::new(line).font(sans(FONT_XS)).color(color));
        });
        if done {
            self.stop_editing(ui);
        } else {
            ui.data_mut(|d| d.insert_temp(self.id, text));
        }
        (out, focused)
    }
}

#[cfg(test)]
mod formula_field_tests {
    use super::*;

    /// A length field in a document with one variable, `Printer.x` = 2.
    struct Host;

    impl FormulaHost for Host {
        fn evaluate(&self, text: &str) -> Result<f64, String> {
            match text.trim() {
                "Printer.x * 2" => Ok(4.0),
                "1 in" => Ok(25.4),
                other => other.parse().map_err(|_| format!("cannot read {other}")),
            }
        }
        fn is_constant(&self, text: &str) -> bool {
            !text.contains('.') || text.trim().parse::<f64>().is_ok()
        }
        fn candidates(&self) -> Vec<crate::completion::Candidate> {
            ["Printer.x", "Printer.y"]
                .iter()
                .map(|t| crate::completion::Candidate {
                    text: t.to_string(),
                    detail: String::new(),
                })
                .collect()
        }
    }

    /// Open the field's formula with `typed` in it, then press `key`: what
    /// the field answers.
    fn type_and_press(formula: Option<&str>, typed: &str, key: egui::Key) -> Option<FormulaEdit> {
        let ctx = egui::Context::default();
        let id = egui::Id::new("f");
        let field_id = id.with("formula_field");
        ctx.data_mut(|d| d.insert_temp(field_id, typed.to_string()));
        ctx.memory_mut(|m| m.request_focus(field_id.with("text")));
        let mut answer = None;
        for events in [
            Vec::new(),
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            let raw = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                if let Some(edit) = FormulaField::new(id, 10.0, &Host)
                    .formula(formula)
                    .unit("mm")
                    .show(ui)
                {
                    answer = Some(edit);
                }
            });
            output.textures_delta.clear();
        }
        answer
    }

    #[test]
    fn a_formula_is_kept_a_quantity_becomes_the_value_and_escape_leaves_it() {
        assert_eq!(
            type_and_press(None, "Printer.x * 2", egui::Key::Enter),
            Some(FormulaEdit::Formula("Printer.x * 2".into()))
        );
        assert_eq!(
            type_and_press(None, "1 in", egui::Key::Enter),
            Some(FormulaEdit::Value(25.4))
        );
        assert_eq!(
            type_and_press(Some("Printer.x * 2"), "", egui::Key::Enter),
            Some(FormulaEdit::Value(10.0)),
            "an empty formula leaves the number"
        );
        assert_eq!(
            type_and_press(None, "Printer.x * 2", egui::Key::Escape),
            None
        );
    }
}
