//! The array task: a body repeated along a line or around an axis, each
//! repeat a linked copy the array owns. Every change applies at once, so
//! the view shows the copies as the numbers move; OK keeps the array as
//! one undo step, Cancel puts back the array as it was (or takes away the
//! one the task made).

use core_document::{ArrayId, ArrayPattern, BodyId, CopyArray, Document, Recorded};
use egui::{RichText, Ui};
use serde_json::{Value, json};
use ui_kit::sans;
use ui_kit::tokens::*;
use ui_kit::widgets::{QtyField, overline, secondary_button, select_field};

use super::host_tasks::HostTaskEnd;

/// Which way a row runs, or the axis a ring turns about.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Way {
    X,
    Y,
    Z,
    Other,
}

impl Way {
    fn of(v: [f32; 3]) -> Self {
        let n = glam::Vec3::from_array(v).normalize_or_zero();
        if (n - glam::Vec3::X).length() < 1e-6 {
            Self::X
        } else if (n - glam::Vec3::Y).length() < 1e-6 {
            Self::Y
        } else if (n - glam::Vec3::Z).length() < 1e-6 {
            Self::Z
        } else {
            Self::Other
        }
    }

    fn vector(self) -> Option<[f32; 3]> {
        match self {
            Self::X => Some([1.0, 0.0, 0.0]),
            Self::Y => Some([0.0, 1.0, 0.0]),
            Self::Z => Some([0.0, 0.0, 1.0]),
            Self::Other => None,
        }
    }
}

const WAYS: &[(Way, &str)] = &[
    (Way::X, "Along X"),
    (Way::Y, "Along Y"),
    (Way::Z, "Along Z"),
    (Way::Other, "Other direction"),
];

#[derive(Debug, Clone)]
pub struct ArrayTask {
    source: BodyId,
    /// The array, once there is one: the task makes it on its first frame
    /// when it was opened on a body without one.
    array: Option<ArrayId>,
    /// The array as it was when the task opened; `None` when the task
    /// made it.
    start: Option<CopyArray>,
    polar: bool,
    count: f32,
    direction: [f32; 3],
    spacing: f32,
    center: [f32; 3],
    axis: [f32; 3],
    angle: f32,
    skipped: Vec<u32>,
    refused: Option<String>,
}

impl ArrayTask {
    /// The task for `body`: the array that owns it or repeats it, or a
    /// new row of three of it.
    pub fn open(document: &Document, body: BodyId) -> Self {
        let existing = document
            .array_instance(body)
            .map(|(a, _)| a.id)
            .or_else(|| document.arrays_of_source(body).first().copied())
            .and_then(|id| document.copy_array(id).cloned());
        let mut task = Self {
            source: body,
            array: None,
            start: None,
            polar: false,
            count: 3.0,
            direction: [1.0, 0.0, 0.0],
            spacing: document.spacing_clear_of(body, [1.0, 0.0, 0.0]),
            center: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            angle: 120.0,
            skipped: Vec::new(),
            refused: None,
        };
        if let Some(array) = existing {
            task.source = array.source;
            task.array = Some(array.id);
            task.count = array.count as f32;
            task.skipped = array.skipped.clone();
            match array.pattern {
                ArrayPattern::Linear { direction, spacing } => {
                    task.direction = direction;
                    task.spacing = spacing;
                }
                ArrayPattern::Polar {
                    center,
                    axis,
                    angle,
                } => {
                    task.polar = true;
                    task.center = center;
                    task.axis = axis;
                    task.angle = angle;
                }
            }
            task.start = Some(array);
        }
        task
    }

    pub fn source(&self) -> BodyId {
        self.source
    }

    fn pattern(&self) -> ArrayPattern {
        if self.polar {
            ArrayPattern::Polar {
                center: self.center,
                axis: self.axis,
                angle: self.angle,
            }
        } else {
            ArrayPattern::Linear {
                direction: self.direction,
                spacing: self.spacing,
            }
        }
    }

    fn count(&self) -> u32 {
        self.count.round().clamp(1.0, 1000.0) as u32
    }

    /// The document brought to the task's values: the array made, or
    /// changed.
    fn apply(&mut self, document: &mut Document) {
        let result = match self.array {
            None => document
                .create_copy_array(self.source, self.pattern(), self.count())
                .map(|id| {
                    self.array = Some(id);
                }),
            Some(id) => {
                document.set_copy_array(id, self.pattern(), self.count(), self.skipped.clone())
            }
        };
        self.refused = result.err().map(|e| e.to_string());
    }

    /// The calls a recording keeps for the array as it stands.
    fn recorded(&self) -> Vec<Recorded> {
        let mut args = match self.pattern() {
            ArrayPattern::Linear { direction, spacing } => json!({
                "kind": "linear",
                "direction": direction,
                "spacing": spacing,
            }),
            ArrayPattern::Polar {
                center,
                axis,
                angle,
            } => json!({
                "kind": "polar",
                "center": center,
                "axis": axis,
                "angle": angle,
            }),
        };
        args["count"] = json!(self.count());
        if !self.skipped.is_empty() {
            args["skip"] = json!(self.skipped);
        }
        let id = match (&self.start, self.array) {
            (Some(_), Some(array)) => {
                args["array"] = json!(array.0.to_string());
                "doc.set_array"
            }
            _ => {
                args["body"] = json!(self.source.0.to_string());
                "doc.array"
            }
        };
        vec![Recorded {
            id: id.to_string(),
            args: match args {
                Value::Object(map) => map,
                _ => Default::default(),
            },
            result: Value::Null,
        }]
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        document: &mut Document,
        accept: bool,
        cancel: bool,
    ) -> Option<HostTaskEnd> {
        if cancel {
            if let Some(id) = self.array {
                document.restore_copy_array(id, self.start.clone());
            }
            return Some(HostTaskEnd::Cancelled);
        }
        if self.array.is_none() {
            self.apply(document);
        }
        if accept {
            let name = self
                .array
                .and_then(|id| document.copy_array(id))
                .map(|a| a.name.clone())
                .unwrap_or_default();
            return Some(HostTaskEnd::Accepted(
                format!("Array {name}"),
                self.recorded(),
            ));
        }
        let mut changed = false;
        note(
            ui,
            "Copies of the body that follow every change to it, placed in a row or a ring \
             from where it sits. Moving the body carries them.",
        );
        heading(ui, "Kind");
        let mut polar = self.polar;
        if select_field(
            ui,
            "array_kind",
            &mut polar,
            &[(false, "Row (linear)"), (true, "Ring (polar)")],
            200.0,
        ) {
            self.polar = polar;
            if polar {
                self.angle = 360.0 / self.count() as f32;
            }
            changed = true;
        }
        heading(ui, "Count");
        ui.horizontal(|ui| {
            label(ui, "Occurrences");
            changed |= QtyField::new(&mut self.count)
                .range(1.0..=1000.0)
                .decimals(0)
                .speed(0.05)
                .show(ui);
        });
        note(ui, "The body itself counted: 3 is the body and two copies.");
        if self.polar {
            heading(ui, "Ring");
            ui.horizontal(|ui| {
                label(ui, "Angle");
                changed |= QtyField::degrees(&mut self.angle)
                    .range(-360.0..=360.0)
                    .show(ui);
            });
            if secondary_button(ui, "Spread over a whole turn").clicked() {
                self.angle = 360.0 / self.count() as f32;
                changed = true;
            }
            changed |= way(ui, "array_axis", &mut self.axis, "Axis");
            ui.add_space(SPACE_1);
            label(ui, "A point on the axis");
            changed |= vector(ui, "array_center", &mut self.center);
        } else {
            heading(ui, "Row");
            ui.horizontal(|ui| {
                label(ui, "Spacing");
                changed |= QtyField::offset(&mut self.spacing).show(ui);
            });
            if secondary_button(ui, "10 mm apart").clicked() {
                self.spacing = document.spacing_clear_of(self.source, self.direction);
                changed = true;
            }
            changed |= way(ui, "array_direction", &mut self.direction, "Direction");
        }
        if !self.skipped.is_empty() {
            heading(ui, "Skipped");
            let list: Vec<String> = self.skipped.iter().map(u32::to_string).collect();
            note(
                ui,
                &format!(
                    "Cop{} {} left out (deleting a copy skips it).",
                    if list.len() == 1 { "y" } else { "ies" },
                    list.join(", ")
                ),
            );
            if secondary_button(ui, "Bring them back").clicked() {
                self.skipped.clear();
                changed = true;
            }
        }
        if let Some(why) = &self.refused {
            ui.add_space(SPACE_2);
            ui.label(RichText::new(why).font(sans(FONT_SM)).color(DANGER));
        }
        if changed {
            self.apply(document);
        }
        None
    }
}

fn note(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(sans(FONT_SM)).color(TEXT3));
}

fn label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(sans(FONT_SM)).color(TEXT2));
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(SPACE_3);
    overline(ui, text);
    ui.add_space(SPACE_1);
}

/// X, Y, Z or a direction of its own, typed; whether it changed.
fn way(ui: &mut Ui, id: &str, value: &mut [f32; 3], title: &str) -> bool {
    let mut changed = false;
    let mut chosen = Way::of(*value);
    ui.add_space(SPACE_1);
    ui.horizontal(|ui| {
        label(ui, title);
        if select_field(ui, id, &mut chosen, WAYS, 160.0) {
            if let Some(v) = chosen.vector() {
                *value = v;
            } else {
                // A slant to start from, for the numbers to change.
                *value = [1.0, 1.0, 0.0];
            }
            changed = true;
        }
    });
    if chosen == Way::Other {
        changed |= vector(ui, id, value);
    }
    changed
}

/// Three numbers, X, Y and Z; whether one changed.
fn vector(ui: &mut Ui, id: &str, value: &mut [f32; 3]) -> bool {
    let mut changed = false;
    egui::Grid::new((id, "xyz"))
        .num_columns(2)
        .spacing([SPACE_3, SPACE_1])
        .show(ui, |ui| {
            for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
                label(ui, axis);
                changed |= QtyField::offset(&mut value[i]).show(ui);
                ui.end_row();
            }
        });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `f` in a frame of a panel, as the task panel would.
    fn in_frame(f: impl FnOnce(&mut Ui)) {
        let ctx = egui::Context::default();
        ui_kit::theme::apply_theme(&ctx);
        let mut f = Some(f);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            if let Some(f) = f.take() {
                f(ui);
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn the_task_makes_a_row_and_cancel_takes_it_away() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let mut task = ArrayTask::open(&doc, body);
        in_frame(|ui| {
            task.show(ui, &mut doc, false, false);
        });
        assert_eq!(doc.bodies().len(), 3, "a row of three to start");
        task.count = 5.0;
        task.apply(&mut doc);
        assert_eq!(doc.bodies().len(), 5);
        let mut end = None;
        in_frame(|ui| end = task.show(ui, &mut doc.clone(), true, false));
        let Some(HostTaskEnd::Accepted(_, ref calls)) = end else {
            panic!("accepted");
        };
        assert_eq!(calls[0].id, "doc.array");
        assert_eq!(calls[0].args["count"], json!(5));
        in_frame(|ui| end = task.show(ui, &mut doc, false, true));
        assert!(matches!(end, Some(HostTaskEnd::Cancelled)));
        assert_eq!(doc.bodies().len(), 1, "only the body is left");
        assert!(doc.copy_arrays().is_empty());
    }

    #[test]
    fn a_copy_opens_its_array_and_cancel_puts_it_back() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let row = ArrayPattern::Linear {
            direction: [0.0, 1.0, 0.0],
            spacing: 7.0,
        };
        let id = doc.create_copy_array(body, row, 3).unwrap();
        let before = doc.copy_array(id).cloned();
        let copy = before.as_ref().unwrap().instances[1].body;
        let mut task = ArrayTask::open(&doc, copy);
        assert_eq!(task.source(), body);
        assert_eq!(task.spacing, 7.0);
        task.count = 2.0;
        task.apply(&mut doc);
        assert_eq!(doc.bodies().len(), 2);
        let mut end = None;
        in_frame(|ui| end = task.show(ui, &mut doc, false, true));
        assert!(matches!(end, Some(HostTaskEnd::Cancelled)));
        assert_eq!(doc.copy_array(id).cloned(), before, "the same copies again");
    }
}
