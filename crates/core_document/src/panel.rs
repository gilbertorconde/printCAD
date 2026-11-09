//! Declared panels: a bench lists widgets as data (`bench_api::Widget`)
//! and the host draws them with the design system's controls, so a
//! package's task panel and Preferences page look and behave like the
//! built-in ones, formulas included.

use bench_api::{
    Bind, ButtonStyle, Callout, Cell, DiagramShape, DiagramStroke, Dim, Dimension, FilterField,
    Flow, Item, ItemList, NoteKind, PanelEvent, Tone, Widget,
};
use egui::{RichText, Ui};
use ui_kit::tokens::*;
use ui_kit::widgets;
use ui_kit::widgets::LabelColumn;

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
    show_section(ui, id, widgets, document, &mut out);
    out
}

/// One section's widgets: their labelled fields share one label column.
fn show_section(
    ui: &mut Ui,
    id: egui::Id,
    widgets: &[Widget],
    document: Option<&Document>,
    out: &mut PanelOutput,
) {
    let place = Place {
        column: LabelColumn::fitting(ui, labels_of(widgets)),
        fill: false,
    };
    for widget in widgets {
        show_one(ui, id, widget, place, document, out);
    }
}

/// Where a widget is drawn: its section's label column, and whether it
/// takes the rest of a row's width.
#[derive(Debug, Clone, Copy)]
struct Place {
    column: LabelColumn,
    fill: bool,
}

impl Place {
    /// A text or choice control's width: the rest of the row when it
    /// fills, else `width`.
    fn width(&self, ui: &Ui, width: f32) -> f32 {
        if self.fill {
            ui.available_width()
        } else {
            width.min(ui.available_width())
        }
    }
}

/// The labels of a section's fields, in its rows, stacks and flows too;
/// a group's, a section's and a [`Widget::Fields`]' are their own.
fn labels_of(widgets: &[Widget]) -> Vec<&str> {
    let mut labels = Vec::new();
    for widget in widgets {
        match widget {
            Widget::Number { label, .. }
            | Widget::Choice { label, .. }
            | Widget::TextField { label, .. }
            | Widget::Pick { label, .. }
            | Widget::Value { label, .. }
            | Widget::Slider { label, .. }
                if !label.is_empty() =>
            {
                labels.push(label.as_str());
            }
            Widget::PickChip(pick) if !pick.label.is_empty() => labels.push(&pick.label),
            Widget::Entry(entry) if !entry.label.is_empty() => labels.push(&entry.label),
            Widget::Row { children }
            | Widget::Stack(bench_api::Stack { children })
            | Widget::Flow(Flow { children, .. }) => labels.extend(labels_of(children)),
            Widget::Fill(fill) => labels.extend(labels_of(std::slice::from_ref(&*fill.child))),
            Widget::Hinted { widget, .. } | Widget::HintedFigure { widget, .. } => {
                labels.extend(labels_of(std::slice::from_ref(widget.as_ref())));
            }
            _ => {}
        }
    }
    labels
}

fn show_one(
    ui: &mut Ui,
    id: egui::Id,
    widget: &Widget,
    place: Place,
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
        } => row(ui, place, label, |ui| {
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
        } => row(ui, place, label, |ui| {
            let mut current = *selected;
            let labelled: Vec<(usize, &str)> = options
                .iter()
                .enumerate()
                .map(|(i, o)| (i, o.as_str()))
                .collect();
            let width = place.width(ui, FIELD_WIDTH);
            if widgets::select_field(ui, id.with(("choice", wid)), &mut current, &labelled, width) {
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
        } => row(ui, place, label, |ui| {
            let width = place.width(ui, FIELD_WIDTH);
            if let Some(value) = text_edit(ui, id.with(("text", wid)), value, "", width) {
                out.events.push(PanelEvent::Text {
                    id: wid.clone(),
                    value,
                });
            }
        }),
        Widget::Button {
            id: wid,
            label,
            style,
            enabled,
        } => {
            let clicked = ui
                .add_enabled_ui(*enabled, |ui| {
                    // A button's label stays on one line: in a row too full
                    // for it, it moves to the next line rather than wrap a
                    // letter or two a line.
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    match style {
                        ButtonStyle::Primary => widgets::primary_button(ui, label),
                        ButtonStyle::Secondary => widgets::secondary_button(ui, label),
                        ButtonStyle::Destructive => widgets::destructive_button(ui, label),
                        ButtonStyle::Small => widgets::small_secondary_button(ui, label),
                    }
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
        } => row(ui, place, label, |ui| {
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
            editable,
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
                            if editable.get(c).copied().unwrap_or(false) {
                                if let Some(value) =
                                    cell_edit(ui, id.with(("cell", wid, i, c)), cell)
                                {
                                    out.events.push(PanelEvent::Cell {
                                        id: wid.clone(),
                                        row: i,
                                        column: c,
                                        value,
                                    });
                                }
                                continue;
                            }
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
                    show_section(ui, id, children, document, out);
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
        Widget::Diagram {
            id: wid,
            width,
            height,
            shapes,
            dimensions,
            callouts,
        } => {
            let response = diagram(ui, *width, *height, shapes, dimensions, callouts);
            if let Some(press) = diagram_press(ui, wid, &response) {
                out.events.push(press);
            }
        }
        Widget::Header { icon, title } => header(ui, icon, title),
        Widget::Value { label, value, mono } => row(ui, place, label, |ui| {
            let font = if *mono {
                ui_kit::theme::mono(FONT_SM)
            } else {
                ui_kit::theme::sans(FONT_SM)
            };
            ui.label(RichText::new(value).font(font).color(TEXT1));
        }),
        Widget::Row { children } => {
            ui.horizontal_wrapped(|ui| {
                for child in children {
                    show_one(ui, id, child, place, document, out);
                }
            });
        }
        Widget::Slider {
            id: wid,
            label,
            value,
            min,
            max,
            step,
            decimals,
            show_value,
        } => row(ui, place, label, |ui| {
            let mut v = *value;
            let mut slider = egui::Slider::new(&mut v, *min..=*max)
                .fixed_decimals(*decimals)
                .show_value(*show_value);
            if let Some(step) = step {
                slider = slider.step_by(*step);
            }
            if ui.add(slider).changed() {
                out.events.push(PanelEvent::Number {
                    id: wid.clone(),
                    value: v,
                });
            }
        }),
        Widget::Sheet {
            id: wid,
            columns,
            rows,
        } => sheet(ui, id.with(("sheet", wid)), wid, columns, rows, out),
        Widget::Hinted { hint, widget } => {
            ui.scope(|ui| show_one(ui, id, widget, place, document, out))
                .response
                .on_hover_text(hint);
        }
        Widget::HintedFigure {
            hint,
            figure,
            widget,
        } => {
            ui.scope(|ui| show_one(ui, id, widget, place, document, out))
                .response
                .on_hover_ui(|ui| {
                    ui.set_max_width(HINT_FIGURE_WIDTH);
                    ui.label(RichText::new(hint).font(ui_kit::theme::sans(FONT_SM)));
                    // A figure explains; what it would report goes nowhere.
                    show_one(
                        ui,
                        id.with("hint"),
                        figure,
                        place,
                        None,
                        &mut PanelOutput::default(),
                    );
                });
        }
        Widget::Section(section) => {
            let key = if section.id.is_empty() {
                &section.title
            } else {
                &section.id
            };
            let actions: Vec<(&str, &str)> = section
                .actions
                .iter()
                .map(|a| (a.icon.as_deref().unwrap_or("plus"), a.label.as_str()))
                .collect();
            let (open, clicked) = widgets::section_fold_actions(
                ui,
                id.with(("section", key)),
                &section.title,
                section.count,
                section.summary.as_deref(),
                section.open,
                &actions,
            );
            if let Some(i) = clicked {
                out.events.push(PanelEvent::Button {
                    id: section.actions[i].id.clone(),
                });
            }
            if open {
                show_section(ui, id, &section.children, document, out);
            }
        }
        Widget::Item(item) => {
            let drawn = item_row(ui, id.with(("item", &item.id)), item);
            let index = 0;
            if drawn.response.clicked() {
                out.events.push(PanelEvent::Select {
                    id: item.id.clone(),
                    index,
                });
            }
            item_events(&item.id, index, item, &drawn, out);
            let hovered = ui
                .rect_contains_pointer(drawn.response.rect)
                .then_some(index);
            hover_changed(
                ui,
                id.with(("item_hover", &item.id)),
                &item.id,
                hovered,
                out,
            );
        }
        Widget::Items(list) => item_list(ui, id, list, out),
        Widget::Filter(filter) => filter_widget(ui, id, filter, out),
        Widget::Status(status) => {
            ui.horizontal(|ui| {
                widgets::status_pill(ui, note_of(status.tone), &status.text, &status.detail);
            });
        }
        Widget::TaskHeader(header) => {
            if !header.subtitle.is_empty() {
                ui.label(
                    RichText::new(&header.subtitle)
                        .font(ui_kit::theme::sans(FONT_SM))
                        .color(TEXT2),
                );
            }
            if let Some(status) = &header.status {
                ui.horizontal(|ui| {
                    widgets::status_pill(ui, note_of(status.tone), &status.text, &status.detail);
                });
            }
        }
        Widget::Stack(stack) => {
            ui.vertical(|ui| {
                for child in &stack.children {
                    show_one(ui, id, child, place, document, out);
                }
            });
        }
        Widget::Flow(flow) => show_flow(ui, id, flow, place, document, out),
        Widget::Fill(fill) => {
            let place = Place {
                fill: true,
                ..place
            };
            show_one(ui, id, &fill.child, place, document, out);
        }
        Widget::Fields(fields) => show_section(ui, id, &fields.children, document, out),
        Widget::Inline(inline) => {
            let place = Place {
                column: LabelColumn::inline(),
                ..place
            };
            show_one(ui, id, &inline.child, place, document, out);
        }
        Widget::PickChip(pick) => row(ui, place, &pick.label, |ui| {
            let placeholder = if pick.armed {
                "Click in the view…"
            } else if pick.placeholder.is_empty() {
                "Nothing picked"
            } else {
                &pick.placeholder
            };
            let outcome = widgets::pick_chip(ui, pick.value.as_deref(), placeholder, pick.armed);
            if outcome.arm {
                out.events.push(PanelEvent::Pick {
                    id: pick.id.clone(),
                });
            }
            if outcome.clear {
                out.events.push(PanelEvent::Clear {
                    id: pick.id.clone(),
                });
            }
        }),
        Widget::Entry(entry) => row(ui, place, &entry.label, |ui| {
            let width = place.width(ui, FIELD_WIDTH);
            let key = id.with(("entry", &entry.id));
            if let Some(value) = text_edit(ui, key, &entry.value, &entry.placeholder, width) {
                out.events.push(PanelEvent::Text {
                    id: entry.id.clone(),
                    value,
                });
            }
        }),
    }
}

/// The width a text or choice field takes in its row.
const FIELD_WIDTH: f32 = 150.0;

/// A one-line text field holding its draft while focused: the text as
/// left, when focus goes and it changed. `placeholder` shows while empty.
fn text_edit(
    ui: &mut Ui,
    key: egui::Id,
    value: &str,
    placeholder: &str,
    width: f32,
) -> Option<String> {
    let mut draft: String = ui
        .data(|d| d.get_temp(key))
        .unwrap_or_else(|| value.to_string());
    let response = ui.add(
        egui::TextEdit::singleline(&mut draft)
            .font(ui_kit::theme::sans(FONT_SM))
            .hint_text(RichText::new(placeholder).color(TEXT3))
            .desired_width(width),
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

/// A [`Widget::Flow`]: side by side; a [`Widget::Fill`] among them takes
/// the width the ones after it leave.
fn show_flow(
    ui: &mut Ui,
    id: egui::Id,
    flow: &Flow,
    place: Place,
    document: Option<&Document>,
    out: &mut PanelOutput,
) {
    let fill = flow
        .children
        .iter()
        .position(|w| matches!(w, Widget::Fill(_)));
    let Some(fill) = fill else {
        let mut each = |ui: &mut Ui| {
            for child in &flow.children {
                show_one(ui, id, child, place, document, out);
            }
        };
        if flow.no_wrap {
            ui.horizontal(each);
        } else {
            ui.horizontal_wrapped(|ui| each(ui));
        }
        return;
    };
    ui.horizontal(|ui| {
        for child in &flow.children[..fill] {
            show_one(ui, id, child, place, document, out);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for child in flow.children[fill + 1..].iter().rev() {
                show_one(ui, id, child, place, document, out);
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                show_one(ui, id, &flow.children[fill], place, document, out);
            });
        });
    });
}

/// The note colour of a tone.
fn note_of(tone: Tone) -> widgets::Note {
    match tone {
        Tone::Good => widgets::Note::Success,
        Tone::Warning => widgets::Note::Warning,
        Tone::Bad => widgets::Note::Error,
        _ => widgets::Note::Info,
    }
}

/// A tone's colour on an icon or a badge; `Neutral` is the plain one.
fn tone_color(tone: Tone) -> egui::Color32 {
    match tone {
        Tone::Neutral => TEXT2,
        Tone::Accent => ACCENT,
        other => note_of(other).color(),
    }
}

/// The most badges an item's row shows before "+N".
const ITEM_BADGES: usize = 4;

/// What the user did to an item's row.
struct ItemDrawn {
    response: egui::Response,
    tag: Option<usize>,
}

/// One item as a panel list row: icon, name, dim words and tags, then its
/// value and badges at the end.
fn item_row(ui: &mut Ui, salt: egui::Id, item: &Item) -> ItemDrawn {
    let look = widgets::RowLook {
        selected: item.selected,
        problem: item.problem,
        dim: item.muted,
        ..Default::default()
    };
    let icon_color = if item.muted {
        TEXT3
    } else {
        tone_color(item.tone)
    };
    let mut tag = None;
    let response = widgets::list_row_with(
        ui,
        salt,
        look,
        (item.icon.as_deref().unwrap_or_default(), icon_color),
        &item.label,
        |ui| {
            widgets::row_detail(ui, item.secondary.as_deref().unwrap_or_default());
            let tags: Vec<(&str, widgets::ChipLook)> = item
                .tags
                .iter()
                .map(|t| {
                    let look = widgets::ChipLook {
                        selected: t.selected,
                        tone: (!matches!(t.tone, Tone::Neutral | Tone::Accent))
                            .then(|| note_of(t.tone)),
                        ..Default::default()
                    };
                    (t.text.as_str(), look)
                })
                .collect();
            for (i, response) in widgets::chips(ui, &tags).into_iter().enumerate() {
                if response.clicked() {
                    tag = Some(i);
                }
            }
        },
        |ui| {
            if let Some(value) = &item.value {
                if item.value_plain {
                    ui.label(
                        RichText::new(value)
                            .font(ui_kit::theme::sans(FONT_SM))
                            .color(TEXT2),
                    );
                } else {
                    widgets::row_value(ui, value, TEXT2);
                }
            }
            let badges: Vec<(&str, egui::Color32, bool)> = item
                .badges
                .iter()
                .map(|b| (b.icon.as_str(), tone_color(b.tone), false))
                .collect();
            for (response, badge) in widgets::badges(ui, &badges, ITEM_BADGES)
                .into_iter()
                .zip(&item.badges)
            {
                if !badge.hint.is_empty() {
                    response.on_hover_text(&badge.hint);
                }
            }
        },
    );
    let response = match &item.hint {
        Some(hint) => response.on_hover_text(hint),
        None => response,
    };
    ItemDrawn { response, tag }
}

/// An item's double click, tag clicks and menu, as events of `id`.
fn item_events(id: &str, index: usize, item: &Item, drawn: &ItemDrawn, out: &mut PanelOutput) {
    if drawn.response.double_clicked() {
        out.events.push(PanelEvent::Activate {
            id: id.to_string(),
            index,
        });
    }
    if let Some(tag) = drawn.tag {
        out.events.push(PanelEvent::Tag {
            id: id.to_string(),
            index,
            tag,
        });
    }
    if !item.menu.is_empty() {
        drawn.response.context_menu(|ui| {
            for action in &item.menu {
                let clicked = match &action.icon {
                    Some(icon) => ui
                        .horizontal(|ui| {
                            ui_kit::icon::draw(ui, icon, 14.0, TEXT2);
                            ui.button(&action.label)
                        })
                        .inner
                        .clicked(),
                    None => ui.button(&action.label).clicked(),
                };
                if clicked {
                    out.events.push(PanelEvent::ContextMenu {
                        id: id.to_string(),
                        index,
                        action: action.id.clone(),
                    });
                    ui.close();
                }
            }
        });
    }
}

/// Tell the bench when the item under the pointer changed since the last
/// frame drawn, kept under `key`.
fn hover_changed(ui: &Ui, key: egui::Id, id: &str, hovered: Option<usize>, out: &mut PanelOutput) {
    let before: Option<usize> = ui.data(|d| d.get_temp(key)).flatten();
    if before != hovered {
        ui.data_mut(|d| d.insert_temp(key, hovered));
        out.events.push(PanelEvent::Hover {
            id: id.to_string(),
            index: hovered,
        });
    }
}

/// What a [`Widget::Filter`] holds between frames: the text typed, the
/// kind chosen, and what the lists it narrows need to read them.
#[derive(Debug, Clone, Default)]
struct FilterState {
    text: String,
    kind: usize,
    kinds: Vec<String>,
    by_bench: bool,
}

fn filter_key(panel: egui::Id, filter: &str) -> egui::Id {
    panel.with(("filter", filter))
}

/// A [`Widget::Filter`]: the panel's filter field, its kinds chosen at
/// its end.
fn filter_widget(ui: &mut Ui, id: egui::Id, filter: &FilterField, out: &mut PanelOutput) {
    let key = filter_key(id, &filter.id);
    let mut state: FilterState = ui.data(|d| d.get_temp(key)).unwrap_or_else(|| FilterState {
        kind: filter.kind.min(filter.kinds.len().saturating_sub(1)),
        ..Default::default()
    });
    state.kinds.clone_from(&filter.kinds);
    state.by_bench = filter.by_bench;
    let mut kind = state.kind;
    let kinds: Vec<(usize, &str)> = filter
        .kinds
        .iter()
        .enumerate()
        .map(|(i, k)| (i, k.as_str()))
        .collect();
    let mut kind_changed = false;
    let hint = if filter.hint.is_empty() {
        "Filter"
    } else {
        &filter.hint
    };
    let typed = widgets::filter_field(ui, &mut state.text, hint, |ui| {
        if !kinds.is_empty() {
            kind_changed =
                widgets::select_field(ui, key.with("kind"), &mut kind, &kinds, FILTER_KIND_WIDTH);
        }
    });
    state.kind = kind;
    if (typed || kind_changed) && filter.by_bench {
        out.events.push(PanelEvent::Filter {
            id: filter.id.clone(),
            text: state.text.clone(),
            kind,
        });
    }
    ui.data_mut(|d| d.insert_temp(key, state));
}

/// The width of a filter's kind selector.
const FILTER_KIND_WIDTH: f32 = 104.0;

/// Whether `item` passes a filter's text (in its label, secondary words
/// or tags, any case) and kind (the first passing every item).
fn passes(item: &Item, state: &FilterState) -> bool {
    let wanted = state.text.trim().to_lowercase();
    let text_ok = wanted.is_empty()
        || item.label.to_lowercase().contains(&wanted)
        || item
            .secondary
            .as_ref()
            .is_some_and(|s| s.to_lowercase().contains(&wanted))
        || item
            .tags
            .iter()
            .any(|t| t.text.to_lowercase().contains(&wanted));
    let kind_ok = state.kind == 0
        || state
            .kinds
            .get(state.kind)
            .is_none_or(|kind| item.kind.as_ref() == Some(kind));
    text_ok && kind_ok
}

/// A [`Widget::Items`]: its rows as the filter it names passes them, a
/// click selecting (Ctrl adding, in a multiple list).
fn item_list(ui: &mut Ui, id: egui::Id, list: &ItemList, out: &mut PanelOutput) {
    let state: Option<FilterState> = list
        .filter
        .as_ref()
        .and_then(|f| ui.data(|d| d.get_temp(filter_key(id, f))));
    let shown: Vec<usize> = (0..list.items.len())
        .filter(|&i| {
            state
                .as_ref()
                .is_none_or(|s| s.by_bench || passes(&list.items[i], s))
        })
        .collect();
    if shown.is_empty()
        && let Some(empty) = &list.empty
    {
        ui.label(
            RichText::new(empty)
                .font(ui_kit::theme::sans(FONT_SM))
                .color(TEXT3),
        );
    }
    let mut hovered = None;
    let mut clicked = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for &i in &shown {
            let item = &list.items[i];
            let drawn = item_row(ui, id.with(("items", &list.id, i)), item);
            if ui.rect_contains_pointer(drawn.response.rect) {
                hovered = Some(i);
            }
            if drawn.response.clicked() {
                clicked = Some(i);
            }
            item_events(&list.id, i, item, &drawn, out);
        }
    });
    if let Some(i) = clicked {
        if list.multi {
            let adding = ui.input(|input| input.modifiers.command);
            let mut indices: Vec<usize> = if adding {
                (0..list.items.len())
                    .filter(|&k| list.items[k].selected != (k == i))
                    .collect()
            } else {
                vec![i]
            };
            indices.sort_unstable();
            out.events.push(PanelEvent::Selection {
                id: list.id.clone(),
                indices,
            });
        } else {
            out.events.push(PanelEvent::Select {
                id: list.id.clone(),
                index: i,
            });
        }
    }
    hover_changed(
        ui,
        id.with(("items_hover", &list.id)),
        &list.id,
        hovered,
        out,
    );
}

/// How wide a hint with a figure grows, in points.
const HINT_FIGURE_WIDTH: f32 = 260.0;

/// What the pointer did on a diagram this frame, in the diagram's own
/// coordinates: a press, each move of a drag begun on it, and where the
/// drag or click ended.
fn diagram_press(ui: &Ui, wid: &str, drawn: &DiagramResponse) -> Option<PanelEvent> {
    let response = &drawn.response;
    let pressed = response.is_pointer_button_down_on() && ui.input(|i| i.pointer.any_pressed());
    let released = response.drag_stopped() || response.clicked();
    let moved = response.dragged() && response.drag_delta() != egui::Vec2::ZERO;
    if !(pressed || released || moved) {
        return None;
    }
    let pos = response
        .interact_pointer_pos()
        .or_else(|| ui.input(|i| i.pointer.latest_pos()))?;
    let [x, y] = drawn.to_diagram(pos);
    Some(PanelEvent::DiagramPress {
        id: wid.to_string(),
        x,
        y,
        dragging: moved && !pressed,
        released,
    })
}

/// A drawn diagram: what the pointer did on it, and how its space maps
/// to the screen.
struct DiagramResponse {
    response: egui::Response,
    origin: egui::Pos2,
    scale: f32,
}

impl DiagramResponse {
    fn to_diagram(&self, pos: egui::Pos2) -> [f32; 2] {
        [
            (pos.x - self.origin.x) / self.scale,
            (self.origin.y - pos.y) / self.scale,
        ]
    }
}

/// Draw a [`Widget::Sheet`]: a striped grid under its column titles,
/// scrolled sideways when wider than the panel. An edited cell keeps its
/// draft until typing ends.
fn sheet(
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

fn sheet_cell(
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

/// A task panel's title: its icon in the accent colour and the title, on a
/// raised band the panel's width.
fn header(ui: &mut Ui, icon: &str, title: &str) {
    egui::Frame::new()
        .fill(BG2)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(RADIUS_MD)
        .inner_margin(egui::Margin::symmetric(SPACE_2 as i8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_2;
                ui_kit::icon::draw(ui, icon, 18.0, ACCENT);
                ui.label(
                    RichText::new(title)
                        .font(ui_kit::theme::sans_semibold(FONT_MD))
                        .color(TEXT1),
                );
            });
        });
}

/// The tallest a diagram grows, in pixels; a wide one takes the panel's
/// width instead.
const DIAGRAM_MAX_HEIGHT: f32 = 220.0;

/// Draw a [`Widget::Diagram`]: its space fitted to the panel's width,
/// strokes, arrows and text at their pixel sizes whatever the fit.
fn diagram(
    ui: &mut Ui,
    width: f32,
    height: f32,
    shapes: &[DiagramShape],
    dimensions: &[Dimension],
    callouts: &[Callout],
) -> DiagramResponse {
    if width <= 0.0 || height <= 0.0 {
        let (rect, response) = ui.allocate_exact_size(egui::Vec2::ZERO, egui::Sense::hover());
        return DiagramResponse {
            response,
            origin: rect.min,
            scale: 1.0,
        };
    }
    let available = ui.available_width().max(1.0);
    let scale = (available / width).min(DIAGRAM_MAX_HEIGHT / height);
    let size = egui::vec2(width * scale, height * scale);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(available, size.y), egui::Sense::click_and_drag());
    let origin = egui::pos2(rect.center().x - size.x / 2.0, rect.bottom());
    let at = |p: [f32; 2]| egui::pos2(origin.x + p[0] * scale, origin.y - p[1] * scale);
    let painter = ui.painter_at(rect);
    let stroke_of = |stroke: DiagramStroke| match stroke {
        DiagramStroke::Outline => egui::Stroke::new(1.5, TEXT1),
        DiagramStroke::Hidden => egui::Stroke::new(1.0, TEXT2),
        DiagramStroke::Axis | DiagramStroke::Thin => egui::Stroke::new(1.0, TEXT3),
        DiagramStroke::Accent => egui::Stroke::new(1.5, ACCENT),
    };
    let dashes = |stroke: DiagramStroke| match stroke {
        DiagramStroke::Hidden => Some((4.0, 3.0)),
        DiagramStroke::Axis => Some((10.0, 4.0)),
        _ => None,
    };
    for shape in shapes {
        match shape {
            DiagramShape::Path {
                points,
                closed,
                stroke,
                fill,
            } => {
                let mut points: Vec<egui::Pos2> = points.iter().map(|p| at(*p)).collect();
                if points.len() < 2 {
                    continue;
                }
                if *fill && points.len() >= 3 {
                    painter.add(egui::Shape::convex_polygon(
                        points.clone(),
                        BG3,
                        egui::Stroke::NONE,
                    ));
                }
                if *closed {
                    points.push(points[0]);
                }
                match dashes(*stroke) {
                    Some((dash, gap)) => {
                        painter.extend(egui::Shape::dashed_line(
                            &points,
                            stroke_of(*stroke),
                            dash,
                            gap,
                        ));
                    }
                    None => {
                        painter.add(egui::Shape::line(points, stroke_of(*stroke)));
                    }
                }
            }
            DiagramShape::Circle {
                center,
                radius,
                stroke,
                fill,
            } => {
                let fill = if *fill {
                    BG3
                } else {
                    egui::Color32::TRANSPARENT
                };
                painter.circle(at(*center), radius * scale, fill, stroke_of(*stroke));
            }
            DiagramShape::Text { at: p, text, mono } => {
                let font = if *mono {
                    ui_kit::theme::mono(FONT_XS)
                } else {
                    ui_kit::theme::sans(FONT_XS)
                };
                painter.text(at(*p), egui::Align2::CENTER_CENTER, text, font, TEXT2);
            }
        }
    }
    for dimension in dimensions {
        let from = at(dimension.from);
        let to = at(dimension.to);
        let run = to - from;
        if run.length() < 0.5 {
            continue;
        }
        let dir = run.normalized();
        // The drawing's left of `from → to` is the screen's right, as y
        // flips; the offset is in the drawing's space.
        let normal = egui::vec2(dir.y, -dir.x);
        let offset = normal * dimension.offset * scale;
        let (p1, p2) = (from + offset, to + offset);
        let color = if dimension.emphasis { ACCENT } else { TEXT2 };
        let stroke = egui::Stroke::new(1.0, color);
        let overshoot = normal * 4.0 * dimension.offset.signum();
        painter.line_segment([from, p1 + overshoot], egui::Stroke::new(1.0, TEXT3));
        painter.line_segment([to, p2 + overshoot], egui::Stroke::new(1.0, TEXT3));
        painter.line_segment([p1, p2], stroke);
        arrow(&painter, p1, dir, color);
        arrow(&painter, p2, -dir, color);
        pill(
            &painter,
            p1 + (p2 - p1) / 2.0,
            &dimension.text,
            dimension.emphasis,
        );
    }
    for callout in callouts {
        let anchor = at(callout.anchor);
        let label = at(callout.at);
        let color = if callout.emphasis { ACCENT } else { TEXT2 };
        painter.line_segment([anchor, label], egui::Stroke::new(1.0, color));
        painter.circle_filled(anchor, 2.5, color);
        pill(&painter, label, &callout.text, callout.emphasis);
    }
    DiagramResponse {
        response,
        origin,
        scale,
    }
}

/// An arrowhead at `tip`, pointing along `dir`.
fn arrow(painter: &egui::Painter, tip: egui::Pos2, dir: egui::Vec2, color: egui::Color32) {
    let side = egui::vec2(-dir.y, dir.x);
    let base = tip + dir * 7.0;
    painter.add(egui::Shape::convex_polygon(
        vec![tip, base + side * 2.5, base - side * 2.5],
        color,
        egui::Stroke::NONE,
    ));
}

/// Text centred at `center` on a rounded backing, so it reads over lines.
fn pill(painter: &egui::Painter, center: egui::Pos2, text: &str, emphasis: bool) {
    let color = if emphasis { ACCENT } else { TEXT1 };
    let font = if emphasis {
        ui_kit::theme::mono_medium(FONT_XS)
    } else {
        ui_kit::theme::mono(FONT_XS)
    };
    let galley = painter.layout_no_wrap(text.to_string(), font, color);
    let pad = egui::vec2(PILL_PAD[0], PILL_PAD[1]);
    let rect = egui::Rect::from_center_size(center, galley.size() + pad * 2.0);
    painter.rect_filled(rect, RADIUS_SM, if emphasis { ACCENT_DIM } else { BG2 });
    painter.galley(rect.min + pad, galley, color);
}

/// An editable table cell: a text edit holding its draft while focused,
/// the text as left when focus goes and it changed.
fn cell_edit(ui: &mut Ui, key: egui::Id, value: &str) -> Option<String> {
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
const CELL_WIDTH: f32 = 64.0;

/// A labelled row: the section's label column, then the control. An
/// empty label takes no column, for a control in a [`Widget::Row`].
fn row(ui: &mut Ui, place: Place, label: &str, control: impl FnOnce(&mut Ui)) {
    if label.is_empty() {
        ui.horizontal(control);
    } else {
        place.column.row(ui, label, control);
    }
}

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
                editable: vec![false, true],
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
            Widget::Diagram {
                id: "d".into(),
                width: 100.0,
                height: 50.0,
                shapes: vec![
                    DiagramShape::Path {
                        points: vec![[10.0, 10.0], [90.0, 10.0], [90.0, 40.0], [10.0, 40.0]],
                        closed: true,
                        stroke: DiagramStroke::Outline,
                        fill: true,
                    },
                    DiagramShape::Path {
                        points: vec![[0.0, 25.0], [100.0, 25.0]],
                        closed: false,
                        stroke: DiagramStroke::Axis,
                        fill: false,
                    },
                    DiagramShape::Circle {
                        center: [50.0, 25.0],
                        radius: 5.0,
                        stroke: DiagramStroke::Hidden,
                        fill: false,
                    },
                    DiagramShape::Text {
                        at: [50.0, 45.0],
                        text: "top".into(),
                        mono: false,
                    },
                ],
                dimensions: vec![Dimension {
                    from: [10.0, 10.0],
                    to: [90.0, 10.0],
                    offset: -6.0,
                    text: "L 80".into(),
                    emphasis: true,
                }],
                callouts: vec![Callout {
                    anchor: [50.0, 25.0],
                    at: [75.0, 45.0],
                    text: "bore".into(),
                    emphasis: false,
                }],
            },
            Widget::Header {
                icon: "gear".into(),
                title: "Task".into(),
            },
            Widget::Value {
                label: "Moves".into(),
                value: "Part".into(),
                mono: false,
            },
            Widget::Row {
                children: vec![
                    Widget::Number {
                        id: "by".into(),
                        label: String::new(),
                        value: 90.0,
                        dim: Dim::Angle,
                        bind: None,
                        min: None,
                        max: None,
                        decimals: 2,
                        error: None,
                    },
                    Widget::Button {
                        id: "remove".into(),
                        label: "Remove".into(),
                        style: ButtonStyle::Small,
                        enabled: true,
                    },
                ],
            },
            Widget::Slider {
                id: "s".into(),
                label: "Spread".into(),
                value: 1.0,
                min: 0.0,
                max: 3.0,
                step: Some(0.5),
                decimals: 2,
                show_value: false,
            },
            Widget::Sheet {
                id: "sheet".into(),
                columns: vec!["No.".into(), "Part".into(), "Bought".into(), "Maker".into()],
                rows: vec![
                    vec![
                        Cell::Text {
                            text: String::new(),
                            mono: false,
                            strong: false,
                        },
                        Cell::Text {
                            text: "Frame".into(),
                            mono: false,
                            strong: true,
                        },
                    ],
                    vec![
                        Cell::Text {
                            text: "1".into(),
                            mono: true,
                            strong: false,
                        },
                        Cell::Link {
                            text: "Bolt".into(),
                        },
                        Cell::Check {
                            on: true,
                            hint: Some("Bought".into()),
                        },
                        Cell::Edit {
                            text: "ACME".into(),
                        },
                    ],
                ],
            },
            Widget::Hinted {
                hint: "Why".into(),
                widget: Box::new(Widget::Toggle {
                    id: "h".into(),
                    label: "Hinted".into(),
                    on: false,
                }),
            },
            Widget::HintedFigure {
                hint: "The step between passes".into(),
                figure: Box::new(Widget::Diagram {
                    id: "stepover".into(),
                    width: 60.0,
                    height: 20.0,
                    shapes: Vec::new(),
                    dimensions: vec![Dimension {
                        from: [10.0, 10.0],
                        to: [30.0, 10.0],
                        offset: 4.0,
                        text: "stepover".into(),
                        emphasis: true,
                    }],
                    callouts: Vec::new(),
                }),
                widget: Box::new(Widget::Toggle {
                    id: "f".into(),
                    label: "Figured".into(),
                    on: false,
                }),
            },
        ]
    }

    /// A press on a diagram and the drag after it come back in the
    /// diagram's own coordinates, y up, its space scaled to the panel.
    #[test]
    fn a_press_and_drag_on_a_diagram_report_where_in_its_space() {
        let widgets = vec![Widget::Diagram {
            id: "timeline".into(),
            width: 100.0,
            height: 50.0,
            shapes: Vec::new(),
            dimensions: Vec::new(),
            callouts: Vec::new(),
        }];
        let ctx = egui::Context::default();
        ui_kit::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 600.0));
        let mut events = Vec::new();
        let mut frame = |input: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(screen),
                events: input,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                events.extend(show(ui, egui::Id::new("diagram"), &widgets, None).events);
            });
            output.textures_delta.clear();
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let (a, b) = (egui::pos2(150.0, 100.0), egui::pos2(190.0, 100.0));
        frame(Vec::new());
        frame(vec![egui::Event::PointerMoved(a)]);
        frame(vec![button(a, true)]);
        frame(vec![egui::Event::PointerMoved(b)]);
        frame(vec![button(b, false)]);
        let presses: Vec<(f32, f32, bool, bool)> = events
            .iter()
            .map(|e| match e {
                PanelEvent::DiagramPress {
                    id,
                    x,
                    y,
                    dragging,
                    released,
                } => {
                    assert_eq!(id, "timeline");
                    (*x, *y, *dragging, *released)
                }
                other => panic!("{other:?}"),
            })
            .collect();
        let [press, drag, release] = presses.as_slice() else {
            panic!("{presses:?}");
        };
        assert!(!press.2 && !press.3, "{press:?}");
        assert!(drag.2 && !drag.3, "{drag:?}");
        assert!(release.3, "{release:?}");
        // The 400 px panel fits the 100 wide diagram 4 to 1, under the
        // 220 px its height may grow to.
        let scale = 4.0;
        assert!(
            (drag.0 - press.0 - 40.0 / scale).abs() < 1e-3,
            "{presses:?}"
        );
        assert!((drag.1 - press.1).abs() < 1e-3);
        assert!(
            (0.0..=50.0).contains(&press.1),
            "y within the diagram: {press:?}"
        );
    }

    #[test]
    fn typing_in_an_editable_cell_reports_the_cell_once_it_is_left() {
        let table = vec![Widget::Table {
            id: "tools".into(),
            columns: vec!["Name".into(), "Diameter".into()],
            rows: vec![vec!["Flat".into(), "6".into()]],
            selected: None,
            editable: vec![false, true],
        }];
        let ctx = egui::Context::default();
        ui_kit::theme::apply_theme(&ctx);
        let panel = egui::Id::new("cells");
        let cell = panel.with(("cell", "tools", 0usize, 1usize));
        let frame = |events: Vec<egui::Event>| {
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut out = None;
            let mut output = ctx.run_ui(input, |ui| out = Some(show(ui, panel, &table, None)));
            output.textures_delta.clear();
            out.unwrap_or_default()
        };
        frame(Vec::new());
        ctx.memory_mut(|m| m.request_focus(cell));
        assert!(
            frame(Vec::new()).events.is_empty(),
            "focus alone says nothing"
        );
        let typed = frame(vec![egui::Event::Text("5".into())]);
        assert!(typed.events.is_empty(), "nothing while the cell is edited");
        let key = |pressed| egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        };
        let left = frame(vec![key(true), key(false)]);
        match left.events.as_slice() {
            [
                PanelEvent::Cell {
                    id,
                    row: 0,
                    column: 1,
                    value,
                },
            ] => {
                assert_eq!(id, "tools");
                assert!(value.contains('5') && value.contains('6'), "{value}");
            }
            other => panic!("one cell event: {other:?}"),
        }
    }

    /// A button after long text in a narrow row keeps its label on one
    /// line, at every width the right panel takes.
    #[test]
    fn a_button_in_a_full_row_keeps_its_label_on_one_line() {
        let widgets = vec![Widget::Row {
            children: vec![
                Widget::Text {
                    text: "Hinge 1 turning from 0 to 90 degrees over the motion".into(),
                    mono: false,
                },
                Widget::Button {
                    id: "remove".into(),
                    label: "Remove".into(),
                    style: ButtonStyle::Small,
                    enabled: true,
                },
            ],
        }];
        let ctx = egui::Context::default();
        ui_kit::theme::apply_theme(&ctx);
        for width in (200..=480).step_by(4) {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_max_width(width as f32);
                show(ui, egui::Id::new("rows"), &widgets, None);
            });
            output.textures_delta.clear();
            let rows: Vec<usize> = output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) if text.galley.job.text == "Remove" => {
                        Some(text.galley.rows.len())
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(rows, vec![1], "at {width} px");
        }
    }

    /// Where each labelled field's control starts: the leftmost shape on
    /// its label's line that is not the label.
    fn control_starts(shapes: &[egui::epaint::ClippedShape], labels: &[&str]) -> Vec<f32> {
        let flat: Vec<(egui::Rect, Option<String>)> = shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some((
                    text.visual_bounding_rect(),
                    Some(text.galley.job.text.clone()),
                )),
                egui::Shape::Rect(rect)
                    if rect.fill != egui::Color32::TRANSPARENT || rect.stroke.width > 0.0 =>
                {
                    Some((rect.rect, None))
                }
                _ => None,
            })
            .collect();
        labels
            .iter()
            .map(|label| {
                let (at, _) = flat
                    .iter()
                    .find(|(_, text)| text.as_deref() == Some(*label))
                    .unwrap_or_else(|| panic!("{label} drawn"));
                flat.iter()
                    .filter(|(rect, text)| {
                        text.as_deref() != Some(*label)
                            && rect.min.x > at.min.x
                            && rect.y_range().contains(at.center().y)
                    })
                    .map(|(rect, _)| rect.min.x)
                    .fold(f32::INFINITY, f32::min)
            })
            .collect()
    }

    /// Every labelled field of a section starts its control at the same
    /// x, whatever the widget and however long its label; a longer label
    /// widens the section's column rather than push its own control.
    #[test]
    fn the_fields_of_a_section_start_their_controls_at_one_x() {
        let number = |id: &str, label: &str| Widget::Number {
            id: id.into(),
            label: label.into(),
            value: 2.0,
            dim: Dim::Length,
            bind: None,
            min: None,
            max: None,
            decimals: 2,
            error: None,
        };
        let fields = vec![
            Widget::Choice {
                id: "stock".into(),
                label: "Stock".into(),
                options: vec!["Box".into(), "Cylinder".into()],
                selected: 0,
            },
            number("sides", "Margin, sides"),
            number("top", "Margin, top"),
            number("below", "Margin, below"),
            Widget::TextField {
                id: "material".into(),
                label: "Material".into(),
                value: "Aluminium".into(),
            },
            Widget::Pick {
                id: "zero".into(),
                label: "Work zero".into(),
                value: Some("Top face".into()),
                armed: false,
            },
            Widget::Value {
                label: "Volume".into(),
                value: "12 cm³".into(),
                mono: true,
            },
        ];
        let labels = [
            "Stock",
            "Margin, sides",
            "Margin, top",
            "Margin, below",
            "Material",
            "Work zero",
            "Volume",
        ];
        let ctx = egui::Context::default();
        ui_kit::theme::apply_theme(&ctx);
        for widgets in [
            fields.clone(),
            vec![Widget::Group {
                title: "Stock".into(),
                open: true,
                children: fields,
            }],
        ] {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_max_width(320.0);
                show(ui, egui::Id::new("aligned"), &widgets, None);
            });
            output.textures_delta.clear();
            let starts = control_starts(&output.shapes, &labels);
            assert!(
                starts
                    .iter()
                    .all(|x| x.is_finite() && (x - starts[0]).abs() < 0.5),
                "{:?}",
                labels.iter().zip(&starts).collect::<Vec<_>>()
            );
        }
    }

    fn row_item(label: &str, icon: &str, value: &str) -> Item {
        Item {
            label: label.into(),
            icon: Some(icon.into()),
            value: (!value.is_empty()).then(|| value.into()),
            ..Default::default()
        }
    }

    fn number(id: &str, label: &str, value: f64) -> Widget {
        Widget::Number {
            id: id.into(),
            label: label.into(),
            value,
            dim: Dim::Number,
            bind: None,
            min: None,
            max: None,
            decimals: 0,
            error: None,
        }
    }

    /// The sketcher's own task panel, said with the blocks alone: its
    /// state, a placement fold with a summary, one filter with a kind
    /// selector over the element and constraint lists (counts, values,
    /// badges, operand chips, an action), and the settings folded away
    /// with summaries. What it cannot say is a block missing.
    fn sketch_panel() -> Vec<Widget> {
        let tag = |text: &str| bench_api::Tag {
            text: text.into(),
            ..Default::default()
        };
        let elements = vec![
            Item {
                kind: Some("Lines".into()),
                badges: vec![bench_api::Badge {
                    icon: "constraint-horizontal".into(),
                    tone: Tone::Good,
                    hint: "Horizontal".into(),
                }],
                ..row_item("Line 1", "line", "89.92 mm")
            },
            Item {
                kind: Some("Arcs".into()),
                secondary: Some("construction".into()),
                muted: true,
                ..row_item("Arc 2", "arc", "R 12.00")
            },
            Item {
                kind: Some("Points".into()),
                ..row_item("Point 5", "point", "")
            },
        ];
        let constraints = vec![
            Item {
                tags: vec![tag("Line 1"), tag("Point 5")],
                ..row_item("Coincident", "constraint-coincident", "")
            },
            Item {
                tags: vec![tag("Line 1")],
                problem: true,
                menu: vec![bench_api::PanelAction {
                    id: "reference".into(),
                    label: "Make reference".into(),
                    icon: None,
                }],
                ..row_item("Distance", "constraint-distance", "89.92 mm")
            },
        ];
        vec![
            Widget::Status(bench_api::StatusLine {
                text: "2 degrees of freedom left".into(),
                tone: Tone::Warning,
                detail: "Drag a point to see what still moves".into(),
            }),
            Widget::Section(bench_api::PanelSection {
                title: "Placement".into(),
                summary: Some("no offset".into()),
                open: false,
                children: vec![number("normal", "Along normal", 0.0)],
                ..Default::default()
            }),
            Widget::Filter(FilterField {
                id: "find".into(),
                hint: "Filter".into(),
                kinds: vec!["All".into(), "Lines".into(), "Arcs".into()],
                ..Default::default()
            }),
            Widget::Section(bench_api::PanelSection {
                title: "Elements".into(),
                count: Some(elements.len()),
                children: vec![Widget::Items(ItemList {
                    id: "elements".into(),
                    items: elements,
                    multi: true,
                    filter: Some("find".into()),
                    empty: Some("No elements".into()),
                })],
                ..Default::default()
            }),
            Widget::Section(bench_api::PanelSection {
                title: "Constraints".into(),
                count: Some(constraints.len()),
                actions: vec![bench_api::PanelAction {
                    id: "solve".into(),
                    icon: Some("refresh".into()),
                    label: "Solve now".into(),
                }],
                children: vec![Widget::Items(ItemList {
                    id: "constraints".into(),
                    items: constraints,
                    filter: Some("find".into()),
                    ..Default::default()
                })],
                ..Default::default()
            }),
            Widget::Section(bench_api::PanelSection {
                title: "Solver".into(),
                summary: Some("auto · 1e-9".into()),
                open: false,
                children: vec![
                    number("steps", "Most steps", 100.0),
                    number("within", "Solved within", 9.0),
                ],
                ..Default::default()
            }),
        ]
    }

    /// A headless panel driven frame by frame: what each frame drew and
    /// what it reported.
    struct Driven {
        ctx: egui::Context,
        widgets: Vec<Widget>,
    }

    impl Driven {
        fn new(widgets: Vec<Widget>) -> Self {
            let ctx = egui::Context::default();
            ui_kit::theme::apply_theme(&ctx);
            Self { ctx, widgets }
        }

        fn frame(&self, events: Vec<egui::Event>) -> (Vec<(String, egui::Rect)>, Vec<PanelEvent>) {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(360.0, 900.0),
                )),
                events,
                ..Default::default()
            };
            let mut reported = Vec::new();
            let mut output = self.ctx.run_ui(raw, |ui| {
                reported = show(ui, egui::Id::new("blocks"), &self.widgets, None).events;
            });
            output.textures_delta.clear();
            let texts = output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => {
                        Some((text.galley.job.text.clone(), text.visual_bounding_rect()))
                    }
                    _ => None,
                })
                .collect();
            (texts, reported)
        }

        fn click(&self, at: egui::Pos2) -> Vec<PanelEvent> {
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            self.frame(vec![egui::Event::PointerMoved(at)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]).1
        }
    }

    fn drawn(texts: &[(String, egui::Rect)], text: &str) -> Vec<egui::Rect> {
        texts
            .iter()
            .filter(|(t, _)| t == text)
            .map(|(_, r)| *r)
            .collect()
    }

    #[test]
    fn the_sketch_panel_draws_from_blocks_alone() {
        let panel = Driven::new(sketch_panel());
        panel.frame(Vec::new());
        let (texts, events) = panel.frame(Vec::new());
        assert!(events.is_empty(), "{events:?}");
        for text in [
            "2 degrees of freedom left",
            "PLACEMENT",
            "no offset",
            "Filter",
            "All",
            "ELEMENTS",
            "3",
            "Line 1",
            "construction",
            "89.92 mm",
            "R 12.00",
            "CONSTRAINTS",
            "Coincident",
            "Point 5",
            "SOLVER",
            "auto · 1e-9",
        ] {
            assert!(!drawn(&texts, text).is_empty(), "{text} drawn");
        }
        // Folded sections keep their fields away; "Line 1" is an element
        // and two constraints' chips.
        assert!(drawn(&texts, "Most steps").is_empty());
        assert_eq!(drawn(&texts, "Line 1").len(), 3);
    }

    #[test]
    fn blocks_tell_the_bench_what_the_pointer_did() {
        let panel = Driven::new(sketch_panel());
        panel.frame(Vec::new());
        let (texts, _) = panel.frame(Vec::new());
        let element = drawn(&texts, "Line 1")[0].center();
        let chip = drawn(&texts, "Point 5")[1].center();

        let hovered = panel.frame(vec![egui::Event::PointerMoved(element)]).1;
        assert!(
            hovered.contains(&PanelEvent::Hover {
                id: "elements".into(),
                index: Some(0)
            }),
            "{hovered:?}"
        );
        let clicked = panel.click(element);
        assert!(
            clicked.contains(&PanelEvent::Selection {
                id: "elements".into(),
                indices: vec![0]
            }),
            "a multiple list tells its selection whole: {clicked:?}"
        );
        let moved = panel.frame(vec![egui::Event::PointerMoved(chip)]).1;
        for hover in [
            PanelEvent::Hover {
                id: "elements".into(),
                index: None,
            },
            PanelEvent::Hover {
                id: "constraints".into(),
                index: Some(0),
            },
        ] {
            assert!(moved.contains(&hover), "{moved:?}");
        }
        let tagged = panel.click(chip);
        assert!(
            tagged.contains(&PanelEvent::Tag {
                id: "constraints".into(),
                index: 0,
                tag: 1
            }),
            "{tagged:?}"
        );

        // Typing in the filter narrows both lists in the host, telling the
        // bench nothing.
        panel.click(drawn(&texts, "Filter")[0].center());
        let (_, typed) = panel.frame(vec![egui::Event::Text("arc".into())]);
        assert!(typed.is_empty(), "{typed:?}");
        let (texts, _) = panel.frame(Vec::new());
        assert!(!drawn(&texts, "Arc 2").is_empty());
        assert!(drawn(&texts, "Line 1").is_empty(), "nothing else holds arc");
        assert!(drawn(&texts, "Coincident").is_empty());
    }

    /// A picked chip arms on a click and clears on its cross; an entry
    /// shows its placeholder while empty.
    #[test]
    fn a_pick_chip_arms_and_clears() {
        let panel = Driven::new(vec![
            Widget::PickChip(bench_api::PickChip {
                id: "edge".into(),
                label: "Along".into(),
                value: Some("Edge 4".into()),
                ..Default::default()
            }),
            Widget::Entry(bench_api::Entry {
                id: "name".into(),
                label: "Name".into(),
                placeholder: "Untitled".into(),
                ..Default::default()
            }),
        ]);
        panel.frame(Vec::new());
        let (texts, _) = panel.frame(Vec::new());
        assert!(!drawn(&texts, "Untitled").is_empty());
        let chip = drawn(&texts, "Edge 4")[0];
        assert_eq!(
            panel.click(chip.center()),
            vec![PanelEvent::Pick { id: "edge".into() }]
        );
        let cross = egui::pos2(chip.right() + PILL_PAD[0] + CHIP / 2.0, chip.center().y);
        assert_eq!(
            panel.click(cross),
            vec![PanelEvent::Clear { id: "edge".into() }]
        );
    }

    #[test]
    fn every_widget_draws_with_and_without_a_document() {
        let mut document = Document::new("panel");
        let feature = document.add_feature_of_kind(
            crate::WorkbenchId::new("bench.thing"),
            "Thing".into(),
            None,
            Vec::new(),
            serde_json::json!({"length": 10.0}),
            Default::default(),
        );
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
        let node = document.get_feature_meta(feature).unwrap();
        assert_eq!(
            node.formulas.get("/length").map(String::as_str),
            Some("2 * 3")
        );
        assert!(node.dirty);
    }
}
