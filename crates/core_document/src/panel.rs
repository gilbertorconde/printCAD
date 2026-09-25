//! Declared panels: a bench lists widgets as data (`bench_api::Widget`)
//! and the host draws them with the design system's controls, so a
//! package's task panel and Preferences page look and behave like the
//! built-in ones, formulas included.

use bench_api::{Bind, ButtonStyle, Dim, NoteKind, PanelEvent, Widget};
use egui::{RichText, Ui};
use ui_kit::tokens::*;
use ui_kit::widgets;

use crate::{Document, FeatureId};

/// What the user did to a declared panel this frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PanelOutput {
    /// For the bench, in the order they happened.
    pub events: Vec<PanelEvent>,
    /// Formulas set on bound numbers (`None` takes one away), for the host
    /// to write into the document: the bench never handles formulas.
    pub formulas: Vec<(Bind, Option<String>)>,
}

/// Draw `widgets`. With a `document`, numbers bound to a feature's
/// parameter show and take formulas; without one they are plain numbers.
pub fn show(
    ui: &mut Ui,
    id: egui::Id,
    widgets: &[Widget],
    document: Option<&Document>,
) -> PanelOutput {
    let mut out = PanelOutput::default();
    ui.spacing_mut().item_spacing.y = SPACE_2;
    for widget in widgets {
        show_one(ui, id, widget, document, &mut out);
    }
    out
}

fn show_one(
    ui: &mut Ui,
    id: egui::Id,
    widget: &Widget,
    document: Option<&Document>,
    out: &mut PanelOutput,
) {
    match widget {
        Widget::Heading { text } => {
            ui.label(
                RichText::new(text)
                    .font(ui_kit::theme::sans_semibold(FONT_MD))
                    .color(TEXT1),
            );
        }
        Widget::Text { text, mono } => {
            let font = if *mono {
                ui_kit::theme::mono(FONT_SM)
            } else {
                ui_kit::theme::sans(FONT_SM)
            };
            ui.label(RichText::new(text).font(font).color(TEXT2));
        }
        Widget::Note { kind, title, text } => {
            let note = match kind {
                NoteKind::Info => widgets::Note::Info,
                NoteKind::Success => widgets::Note::Success,
                NoteKind::Warning => widgets::Note::Warning,
                NoteKind::Error => widgets::Note::Error,
            };
            widgets::note_card(ui, note, title.as_deref(), text);
        }
        Widget::Number {
            id: wid,
            label,
            value,
            dim,
            bind,
            min,
            max,
            decimals,
            error,
        } => row(ui, label, |ui| {
            let unit = match dim {
                Dim::Length => "mm",
                Dim::Angle => "°",
                Dim::Number => "",
            };
            let clamp = |v: f64| v.clamp(min.unwrap_or(f64::MIN), max.unwrap_or(f64::MAX));
            let bound = bind.as_ref().zip(document).and_then(|(bind, document)| {
                let feature = FeatureId(uuid::Uuid::parse_str(&bind.feature).ok()?);
                Some((bind, feature, document))
            });
            match bound {
                Some((bind, feature, document)) => {
                    let want = expr_dim(*dim);
                    let formula = document.feature_formula(feature, &bind.key);
                    let slot = document
                        .evaluated_slots(feature)
                        .iter()
                        .find(|s| s.key == bind.key);
                    let shown = match (formula, slot.map(|s| &s.result)) {
                        (Some(_), Some(Ok(q))) => q.value,
                        _ => *value,
                    };
                    let slot_error = slot.and_then(|s| s.result.as_ref().err());
                    let host = crate::DocumentFormulas {
                        document,
                        dim: want,
                    };
                    let edit = widgets::FormulaField::new(id.with(("number", wid)), shown, &host)
                        .formula(formula)
                        .error(error.as_deref().or(slot_error.map(String::as_str)))
                        .unit(unit)
                        .decimals(*decimals)
                        .speed(if *dim == Dim::Angle { 1.0 } else { 0.1 })
                        .show(ui);
                    match edit {
                        Some(widgets::FormulaEdit::Value(v)) => {
                            if formula.is_some() {
                                out.formulas.push((bind.clone(), None));
                            }
                            out.events.push(PanelEvent::Number {
                                id: wid.clone(),
                                value: clamp(v),
                            });
                        }
                        Some(widgets::FormulaEdit::Formula(text)) => {
                            let now = document.evaluate_formula(&text, Some(want));
                            out.formulas.push((bind.clone(), Some(text)));
                            if let Ok(q) = now {
                                out.events.push(PanelEvent::Number {
                                    id: wid.clone(),
                                    value: q.value,
                                });
                            }
                        }
                        None => {}
                    }
                }
                None => {
                    let mut v = *value as f32;
                    let mut field = widgets::QtyField::new(&mut v)
                        .unit(match dim {
                            Dim::Length => "mm",
                            Dim::Angle => "°",
                            Dim::Number => "",
                        })
                        .decimals(*decimals)
                        .error(error.as_deref());
                    if min.is_some() || max.is_some() {
                        field = field.range(min.unwrap_or(f64::MIN)..=max.unwrap_or(f64::MAX));
                    }
                    if field.show(ui) {
                        out.events.push(PanelEvent::Number {
                            id: wid.clone(),
                            value: clamp(f64::from(v)),
                        });
                    }
                }
            }
        }),
        Widget::Choice {
            id: wid,
            label,
            options,
            selected,
        } => row(ui, label, |ui| {
            let mut current = *selected;
            let labelled: Vec<(usize, &str)> = options
                .iter()
                .enumerate()
                .map(|(i, o)| (i, o.as_str()))
                .collect();
            if widgets::select_field(ui, id.with(("choice", wid)), &mut current, &labelled, 150.0) {
                out.events.push(PanelEvent::Choice {
                    id: wid.clone(),
                    index: current,
                });
            }
        }),
        Widget::Toggle { id: wid, label, on } => {
            let mut on = *on;
            if widgets::check_row(ui, &mut on, label).changed() {
                out.events.push(PanelEvent::Toggle {
                    id: wid.clone(),
                    on,
                });
            }
        }
        Widget::TextField {
            id: wid,
            label,
            value,
        } => row(ui, label, |ui| {
            let key = id.with(("text", wid));
            let mut draft: String = ui
                .data(|d| d.get_temp(key))
                .unwrap_or_else(|| value.clone());
            let response = ui.add(
                egui::TextEdit::singleline(&mut draft)
                    .font(ui_kit::theme::sans(FONT_SM))
                    .desired_width(150.0),
            );
            if response.has_focus() {
                ui.data_mut(|d| d.insert_temp(key, draft.clone()));
            }
            if response.lost_focus() {
                ui.data_mut(|d| d.remove::<String>(key));
                if draft != *value {
                    out.events.push(PanelEvent::Text {
                        id: wid.clone(),
                        value: draft,
                    });
                }
            }
        }),
        Widget::Button {
            id: wid,
            label,
            style,
            enabled,
        } => {
            let clicked = ui
                .add_enabled_ui(*enabled, |ui| match style {
                    ButtonStyle::Primary => widgets::primary_button(ui, label),
                    ButtonStyle::Secondary => widgets::secondary_button(ui, label),
                    ButtonStyle::Destructive => widgets::destructive_button(ui, label),
                })
                .inner
                .clicked();
            if clicked {
                out.events.push(PanelEvent::Button { id: wid.clone() });
            }
        }
        Widget::Pick {
            id: wid,
            label,
            value,
            armed,
        } => row(ui, label, |ui| {
            let text = match (value, armed) {
                (_, true) => "Click in the view…".to_string(),
                (Some(v), false) => v.clone(),
                (None, false) => "Nothing picked".to_string(),
            };
            let button = if *armed {
                widgets::accent_outline_button(ui, &text)
            } else {
                widgets::secondary_button(ui, &text)
            };
            if button.clicked() {
                out.events.push(PanelEvent::Pick { id: wid.clone() });
            }
        }),
        Widget::List {
            id: wid,
            items,
            selected,
        } => {
            for (i, item) in items.iter().enumerate() {
                let on = *selected == Some(i);
                let response = ui
                    .horizontal(|ui| {
                        if let Some(icon) = &item.icon {
                            ui_kit::icon::draw(ui, icon, 14.0, if on { ACCENT } else { TEXT2 });
                        }
                        let label = ui.selectable_label(
                            on,
                            RichText::new(&item.label).font(ui_kit::theme::sans(FONT_SM)),
                        );
                        if let Some(detail) = &item.detail {
                            widgets::mono_label(ui, detail, FONT_XS, TEXT3);
                        }
                        label
                    })
                    .inner;
                if response.clicked() {
                    out.events.push(PanelEvent::Select {
                        id: wid.clone(),
                        index: i,
                    });
                }
            }
        }
        Widget::Table {
            id: wid,
            columns,
            rows,
            selected,
        } => {
            egui::Grid::new(id.with(("table", wid)))
                .striped(true)
                .spacing(egui::vec2(SPACE_3, SPACE_1))
                .show(ui, |ui| {
                    for column in columns {
                        ui.label(
                            RichText::new(column)
                                .font(ui_kit::theme::sans_semibold(FONT_XS))
                                .color(TEXT3),
                        );
                    }
                    ui.end_row();
                    for (i, cells) in rows.iter().enumerate() {
                        let on = *selected == Some(i);
                        for (c, cell) in cells.iter().enumerate() {
                            let text = RichText::new(cell)
                                .font(ui_kit::theme::mono(FONT_SM))
                                .color(if on { ACCENT } else { TEXT1 });
                            let clicked = if c == 0 {
                                ui.selectable_label(on, text).clicked()
                            } else {
                                ui.label(text).clicked()
                            };
                            if clicked {
                                out.events.push(PanelEvent::Select {
                                    id: wid.clone(),
                                    index: i,
                                });
                            }
                        }
                        ui.end_row();
                    }
                });
        }
        Widget::Group {
            title,
            open,
            children,
        } => {
            if widgets::section_header(ui, id.with(("group", title)), title, None, *open) {
                ui.indent(id.with(("group_body", title)), |ui| {
                    for child in children {
                        show_one(ui, id, child, document, out);
                    }
                });
            }
        }
        Widget::Progress {
            label, fraction, ..
        } => {
            ui.label(
                RichText::new(label)
                    .font(ui_kit::theme::sans(FONT_SM))
                    .color(TEXT2),
            );
            match fraction {
                Some(f) => {
                    ui.add(egui::ProgressBar::new(f.clamp(0.0, 1.0)).desired_height(6.0));
                }
                None => {
                    ui.spinner();
                }
            }
        }
        Widget::Separator => {
            ui.separator();
        }
    }
}

/// A labelled row: the label column, then the control.
fn row(ui: &mut Ui, label: &str, control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(LABEL_COLUMN, INPUT), egui::Sense::hover());
        ui.put(rect, |ui: &mut Ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                widgets::field_label(ui, label)
            })
            .inner
        });
        control(ui);
    });
}

/// The width of a labelled row's label column.
const LABEL_COLUMN: f32 = 110.0;

fn expr_dim(dim: Dim) -> crate::expr::Dim {
    match dim {
        Dim::Length => crate::expr::Dim::LENGTH,
        Dim::Angle => crate::expr::Dim::ANGLE,
        Dim::Number => crate::expr::Dim::NUMBER,
    }
}

/// Write the formulas a panel set into the document, marking what they
/// move: a formula on a bound number, or `None` to leave the number as it
/// stands.
pub fn apply_formulas(document: &mut Document, formulas: Vec<(Bind, Option<String>)>) {
    for (bind, formula) in formulas {
        let Ok(uuid) = uuid::Uuid::parse_str(&bind.feature) else {
            continue;
        };
        let feature = FeatureId(uuid);
        if document
            .set_feature_formula(feature, bind.key, formula)
            .is_ok()
        {
            document.mark_feature_dirty(feature);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bench_api::{ListItem, NoteKind};

    fn every_widget(feature: &str) -> Vec<Widget> {
        vec![
            Widget::Heading { text: "H".into() },
            Widget::Text {
                text: "t".into(),
                mono: true,
            },
            Widget::Note {
                kind: NoteKind::Warning,
                title: Some("n".into()),
                text: "careful".into(),
            },
            Widget::Number {
                id: "bound".into(),
                label: "Length".into(),
                value: 10.0,
                dim: Dim::Length,
                bind: Some(Bind {
                    feature: feature.into(),
                    key: "/length".into(),
                }),
                min: None,
                max: None,
                decimals: 2,
                error: None,
            },
            Widget::Number {
                id: "plain".into(),
                label: "Count".into(),
                value: 3.0,
                dim: Dim::Number,
                bind: None,
                min: Some(1.0),
                max: Some(9.0),
                decimals: 0,
                error: Some("too few".into()),
            },
            Widget::Choice {
                id: "c".into(),
                label: "Pick".into(),
                options: vec!["a".into(), "b".into()],
                selected: 1,
            },
            Widget::Toggle {
                id: "t".into(),
                label: "On".into(),
                on: true,
            },
            Widget::TextField {
                id: "f".into(),
                label: "Name".into(),
                value: "x".into(),
            },
            Widget::Button {
                id: "b".into(),
                label: "Go".into(),
                style: ButtonStyle::Primary,
                enabled: false,
            },
            Widget::Pick {
                id: "p".into(),
                label: "Face".into(),
                value: None,
                armed: true,
            },
            Widget::List {
                id: "l".into(),
                items: vec![ListItem {
                    label: "one".into(),
                    detail: Some("1".into()),
                    icon: Some("gear".into()),
                }],
                selected: Some(0),
            },
            Widget::Table {
                id: "tb".into(),
                columns: vec!["A".into(), "B".into()],
                rows: vec![vec!["1".into(), "2".into()]],
                selected: None,
            },
            Widget::Group {
                title: "G".into(),
                open: true,
                children: vec![Widget::Separator],
            },
            Widget::Progress {
                label: "Working".into(),
                fraction: Some(0.5),
                job: None,
            },
        ]
    }

    #[test]
    fn every_widget_draws_with_and_without_a_document() {
        let mut document = Document::new("panel");
        let feature = crate::FeatureId::new();
        let widgets = every_widget(&feature.0.to_string());
        let ctx = egui::Context::default();
        ui_kit::theme::apply_theme(&ctx);
        for with_document in [true, false] {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let out = show(
                    ui,
                    egui::Id::new("panel_test"),
                    &widgets,
                    with_document.then_some(&document),
                );
                assert!(out.events.is_empty(), "nothing was touched");
                assert!(out.formulas.is_empty());
            });
            output.textures_delta.clear();
        }
        // Formulas land in the document as the bench's own would.
        apply_formulas(
            &mut document,
            vec![(
                Bind {
                    feature: feature.0.to_string(),
                    key: "/length".into(),
                },
                Some("2 * 3".into()),
            )],
        );
    }
}
