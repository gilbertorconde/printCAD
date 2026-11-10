//! Declared panels: a bench lists widgets as data (`bench_api::Widget`)
//! and the host draws them with the design system's controls, so a
//! package's task panel and Preferences page look and behave like the
//! built-in ones, formulas included.

use bench_api::{
    Bind, ButtonStyle, Callout, Cell, DiagramShape, DiagramStroke, Dim, Dimension, FieldGrid,
    FilterField, Flow, GapSize, Item, ItemList, NoteKind, PanelEvent, Quantity, Tone, Widget,
};
use egui::{RichText, Ui};
use ui_kit::tokens::*;
use ui_kit::widgets;
use ui_kit::widgets::LabelColumn;

use crate::{Document, FeatureId};

mod diagram;
mod fields;
mod items;
mod sheet;

use diagram::*;
use fields::*;
use items::*;
use sheet::*;

/// What the user did to a declared panel this frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PanelOutput {
    /// For the bench, in the order they happened.
    pub events: Vec<PanelEvent>,
    /// Formulas set on bound numbers (`None` takes one away), for the host
    /// to write into the document: the bench never handles formulas.
    pub formulas: Vec<FormulaSet>,
    /// The item lists drawn this frame, by id.
    lists: Vec<String>,
    /// What the rows of a [`Widget::Settings`] are narrowed to, lowercase.
    filter: String,
}

/// A formula typed into a bound number, or taken off it.
#[derive(Debug, Clone, PartialEq)]
pub struct FormulaSet {
    /// The number's widget id.
    pub id: String,
    pub bind: Bind,
    /// `None` leaves the number as it stands, without a formula.
    pub formula: Option<String>,
}

/// Draw `widgets`. With a `document`, numbers bound to a feature's
/// parameter show and take formulas; without one they are plain numbers.
pub fn show(
    ui: &mut Ui,
    id: egui::Id,
    widgets: &[Widget],
    document: Option<&Document>,
) -> PanelOutput {
    show_filtered(ui, id, widgets, document, "")
}

/// [`show`], the rows of its [`Widget::Settings`] groups narrowed to those
/// whose label or hint holds `filter` (the Preferences search), a group
/// left with none not drawn.
pub fn show_filtered(
    ui: &mut Ui,
    id: egui::Id,
    widgets: &[Widget],
    document: Option<&Document>,
    filter: &str,
) -> PanelOutput {
    let mut out = PanelOutput {
        filter: filter.trim().to_lowercase(),
        ..Default::default()
    };
    ui.spacing_mut().item_spacing.y = SPACE_2;
    show_section(ui, id, widgets, document, &mut out);
    leave_undrawn_lists(ui, id, &mut out);
    out
}

/// A list the pointer was on that is not drawn this frame (its section
/// closed, the panel changed) is left: told so, as when the pointer moves
/// off it.
fn leave_undrawn_lists(ui: &Ui, id: egui::Id, out: &mut PanelOutput) {
    let key = id.with("hovered_lists");
    let hovered: Vec<String> = ui.data(|d| d.get_temp(key)).unwrap_or_default();
    for list in hovered.iter().filter(|l| !out.lists.contains(l)) {
        ui.data_mut(|d| {
            d.remove::<Option<usize>>(id.with(("items_hover", list)));
            d.remove::<Pointed>(id.with(("items_part", list)));
        });
        out.events.push(PanelEvent::Hover {
            id: list.clone(),
            index: None,
        });
        out.events.push(PanelEvent::HoverPart {
            id: list.clone(),
            index: None,
            tag: None,
            badge: None,
        });
    }
    let still: Vec<String> = out
        .lists
        .iter()
        .filter(|l| {
            ui.data(|d| d.get_temp::<Option<usize>>(id.with(("items_hover", l.as_str()))))
                .flatten()
                .is_some()
        })
        .cloned()
        .collect();
    ui.data_mut(|d| d.insert_temp(key, still));
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
        column: Column::Labels(LabelColumn::fitting(ui, labels_of(widgets))),
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
    column: Column,
    fill: bool,
}

/// Where a field's label goes.
#[derive(Debug, Clone, Copy)]
enum Column {
    /// The section's label column.
    Labels(LabelColumn),
    /// A [`Widget::Grid`]'s first column, the control in its second.
    Grid,
    /// The control alone: a [`Widget::Settings`] row draws the label.
    Control,
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
        labels.extend(widget.field_label());
        // Widgets laid out inline write their labels in this column;
        // sections, grids and cards keep columns of their own.
        if matches!(
            widget,
            Widget::Flow(_)
                | Widget::Row { .. }
                | Widget::Stack(_)
                | Widget::Fill(_)
                | Widget::Hinted { .. }
                | Widget::HintedFigure { .. }
        ) {
            for child in widget.children() {
                labels.extend(labels_of(std::slice::from_ref(child)));
            }
        }
    }
    labels
}

/// The block an older widget is drawn as, so each control has one
/// drawing: a number is a [`Widget::Quantity`], a choice a
/// [`Widget::Select`], a toggle a [`Widget::Check`] and a group a
/// [`Widget::Section`]. Their events stay what they were.
fn lowered(widget: &Widget) -> Option<Widget> {
    Some(match widget {
        Widget::Number {
            id,
            label,
            value,
            dim,
            bind,
            min,
            max,
            decimals,
            error,
        } => Quantity {
            id: id.clone(),
            label: label.clone(),
            value: *value,
            dim: *dim,
            bind: bind.clone(),
            min: *min,
            max: *max,
            decimals: *decimals,
            error: error.clone(),
            ..Default::default()
        }
        .into(),
        Widget::Choice {
            id,
            label,
            options,
            selected,
        } => bench_api::Select {
            id: id.clone(),
            label: label.clone(),
            options: options.clone(),
            selected: *selected,
            ..Default::default()
        }
        .into(),
        Widget::Toggle { id, label, on } => bench_api::Check {
            id: id.clone(),
            text: label.clone(),
            on: *on,
            ..Default::default()
        }
        .into(),
        Widget::Group {
            title,
            open,
            children,
        } => bench_api::PanelSection {
            title: title.clone(),
            open: *open,
            children: children.clone(),
            ..Default::default()
        }
        .into(),
        _ => return None,
    })
}

fn show_one(
    ui: &mut Ui,
    id: egui::Id,
    widget: &Widget,
    place: Place,
    document: Option<&Document>,
    out: &mut PanelOutput,
) {
    if let Some(block) = lowered(widget) {
        return show_one(ui, id, &block, place, document, out);
    }
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
            widgets::note_card(ui, note_kind(*kind), title.as_deref(), text);
        }
        Widget::Number { .. }
        | Widget::Choice { .. }
        | Widget::Toggle { .. }
        | Widget::Group { .. } => {
            unreachable!("drawn as the block it is lowered into")
        }
        Widget::TextField {
            id: wid,
            label,
            value,
        } => row(ui, place, label, |ui| {
            let width = place.width(ui, FIELD_WIDTH);
            let entry = bench_api::Entry {
                value: value.clone(),
                ..Default::default()
            };
            if let Some(value) = text_edit(ui, id.with(("text", wid)), &entry, width, None).text {
                out.events.push(PanelEvent::Text {
                    id: wid.clone(),
                    value,
                });
            }
        }),
        Widget::Button { id: wid, .. } => {
            if button(ui, widget).clicked() {
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
        // A button takes its words itself, so it stays one item of a flow
        // that wraps, and says them while disabled too.
        Widget::Hinted { hint, widget } => match widget.as_ref() {
            Widget::Button {
                id: wid, enabled, ..
            } => {
                let response = button(ui, widget);
                let response = if *enabled {
                    response.on_hover_text(hint)
                } else {
                    response.on_disabled_hover_text(hint)
                };
                if response.clicked() {
                    out.events.push(PanelEvent::Button { id: wid.clone() });
                }
            }
            _ => {
                ui.scope(|ui| show_one(ui, id, widget, place, document, out))
                    .response
                    .on_hover_text(hint);
            }
        },
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
            let drawn = {
                let mut field = field_drawer(id, document, out);
                item_row(ui, id.with(("item", &item.id)), item, &mut field)
            };
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
        Widget::Items(list) => item_list(ui, id, list, document, out),
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
                if stack.tight {
                    ui.spacing_mut().item_spacing.y = SPACE_1;
                }
                if let Some(spacing) = stack.spacing {
                    ui.spacing_mut().item_spacing.y = spacing;
                }
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
        Widget::Fields(fields) => match fields.label_width {
            Some(width) => {
                let place = Place {
                    column: Column::Labels(LabelColumn::fixed(width)),
                    fill: false,
                };
                for child in &fields.children {
                    show_one(ui, id, child, place, document, out);
                }
            }
            None => show_section(ui, id, &fields.children, document, out),
        },
        Widget::Inline(inline) => {
            let place = Place {
                column: Column::Labels(LabelColumn::inline()),
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
        // With no label the field stands where it is, as a check box does,
        // so one alone is no taller than its text.
        Widget::Entry(entry) => row_or_bare(ui, place, &entry.label, |ui| {
            let width = place.width(ui, entry.width.unwrap_or(FIELD_WIDTH));
            let key = id.with(("entry", &entry.id));
            let typed = text_edit(ui, key, entry, width, document);
            if let Some(value) = typed.text {
                out.events.push(PanelEvent::Text {
                    id: entry.id.clone(),
                    value,
                });
            }
            if let Some((value, escaped)) = typed.left {
                out.events.push(PanelEvent::TextLeft {
                    id: entry.id.clone(),
                    value,
                    escaped,
                });
            }
        }),
        Widget::Quantity(q) => row(ui, place, &q.label, |ui| {
            quantity(ui, id, q, place.fill, document, out);
        }),
        Widget::Check(check) => row_or_bare(ui, place, &check.label, |ui| {
            let mut on = check.on;
            // An enabled box is added as it stands, so a flow of them wraps.
            let response = if check.enabled {
                widgets::check_row(ui, &mut on, &check.text)
            } else {
                ui.add_enabled_ui(false, |ui| widgets::check_row(ui, &mut on, &check.text))
                    .inner
            };
            let response = if check.hint.is_empty() {
                response
            } else if check.enabled {
                response.on_hover_text(&check.hint)
            } else {
                response.on_disabled_hover_text(&check.hint)
            };
            if response.changed() {
                out.events.push(PanelEvent::Toggle {
                    id: check.id.clone(),
                    on,
                });
            }
        }),
        Widget::Select(select) => row(ui, place, &select.label, |ui| {
            let width = place.width(ui, select.width.unwrap_or(FIELD_WIDTH));
            let key = id.with(("select", &select.id));
            let chosen = ui
                .add_enabled_ui(select.enabled, |ui| {
                    widgets::select_control(
                        ui,
                        key,
                        widgets::SelectOptions {
                            options: &select.options,
                            selected: select.selected,
                            text: select.text.as_deref(),
                            disabled: &select.disabled,
                            hints: &select.hints,
                            separated: &select.separated,
                            segmented: select.segmented,
                        },
                        width,
                    )
                })
                .inner;
            if let Some(index) = chosen {
                out.events.push(PanelEvent::Choice {
                    id: select.id.clone(),
                    index,
                });
            }
        }),
        Widget::Grid(grid) => field_grid(ui, id, grid, place, document, out),
        Widget::Alert(alert) => {
            let response = widgets::note_card(
                ui,
                note_kind(alert.kind),
                alert.title.as_deref(),
                &alert.text,
            );
            if let Some(wid) = &alert.id
                && response
                    .interact(egui::Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
            {
                out.events.push(PanelEvent::Button { id: wid.clone() });
            }
        }
        Widget::Card(card) => {
            let mut frame = widgets::Card::new();
            if let Some(padding) = card.padding {
                frame = frame.padding(padding);
            }
            frame.show(ui, |ui| {
                if card.icon.is_some() || !card.title.is_empty() {
                    ui.horizontal(|ui| {
                        if let Some(icon) = &card.icon {
                            ui_kit::icon::draw(ui, icon, card.icon_size, ACCENT);
                        }
                        ui.label(
                            RichText::new(&card.title)
                                .font(ui_kit::theme::sans_semibold(FONT_MD))
                                .color(TEXT1),
                        );
                    });
                }
                show_section(ui, id, &card.children, document, out);
            });
        }
        Widget::Caption(caption) if caption.overline => {
            widgets::overline(ui, &caption.text);
        }
        Widget::Caption(caption) => {
            let size = if caption.small { FONT_XS } else { FONT_SM };
            let font = match (caption.strong, caption.mono, caption.medium) {
                (true, _, _) => ui_kit::theme::sans_semibold(size),
                (false, true, _) => ui_kit::theme::mono(size),
                (false, false, true) => ui_kit::theme::sans_medium(size),
                (false, false, false) => ui_kit::theme::sans(size),
            };
            let color = if caption.strong || caption.primary {
                TEXT1
            } else {
                tone_color(caption.tone)
            };
            ui.label(RichText::new(&caption.text).font(font).color(color));
        }
        Widget::Plot(plot) => self::plot(ui, plot),
        Widget::Gap(gap) => ui.add_space(match gap.size {
            GapSize::Small => SPACE_1,
            GapSize::Medium => SPACE_2,
            GapSize::Large => SPACE_4,
            GapSize::Section => SPACE_3,
        }),
        Widget::Swatches(set) => {
            if let Some(index) = swatches(ui, set) {
                out.events.push(PanelEvent::Choice {
                    id: set.id.clone(),
                    index,
                });
            }
        }
        Widget::Color(field) => row(ui, place, &field.label, |ui| {
            let mut color = field.color;
            if egui::color_picker::color_edit_button_rgb(ui, &mut color).changed() {
                out.events.push(PanelEvent::Color {
                    id: field.id.clone(),
                    color,
                });
            }
        }),
        Widget::Range(range) => row(ui, place, &range.label, |ui| {
            let mut v = range.value;
            let mut slider = egui::Slider::new(&mut v, range.min..=range.max)
                .suffix(&range.suffix)
                .fixed_decimals(range.decimals);
            if let Some(step) = range.step {
                slider = slider.step_by(step);
            }
            if ui.add_enabled(range.enabled, slider).changed() {
                out.events.push(PanelEvent::Number {
                    id: range.id.clone(),
                    value: v,
                });
            }
        }),
        Widget::RowButton(button) => {
            let text = RichText::new(&button.text).font(ui_kit::theme::sans(FONT_SM));
            let text = if button.current {
                text.color(ACCENT).strong()
            } else {
                text.color(TEXT1)
            };
            let response = ui.add_enabled(
                !button.current,
                egui::Button::new(text)
                    .frame(false)
                    .min_size(egui::Vec2::new(ui.available_width(), ROW_BUTTON_HEIGHT)),
            );
            if response.clicked() {
                out.events.push(PanelEvent::Button {
                    id: button.id.clone(),
                });
            }
        }
        Widget::Selectable(selectable) => {
            if ui
                .selectable_label(selectable.selected, &selectable.text)
                .clicked()
            {
                out.events.push(PanelEvent::Button {
                    id: selectable.id.clone(),
                });
            }
        }
        Widget::Gallery(gallery) => {
            if let Some(index) = self::gallery(ui, gallery) {
                out.events.push(PanelEvent::Choice {
                    id: gallery.id.clone(),
                    index,
                });
            }
        }
        Widget::Settings(group) => {
            let filter = out.filter.clone();
            // Each row's control reports into the one output.
            let shared = std::cell::RefCell::new(&mut *out);
            let rows: Vec<widgets::PrefRow<'_>> = group
                .children
                .iter()
                .map(|child| {
                    // A hint is the row's, under its label.
                    let (hint, child) = match child {
                        Widget::Hinted { hint, widget } => (Some(hint.as_str()), &**widget),
                        other => (None, other),
                    };
                    let label = match child {
                        Widget::Check(check) if check.label.is_empty() => check.text.as_str(),
                        other => labels_of(std::slice::from_ref(other))
                            .first()
                            .copied()
                            .unwrap_or(""),
                    };
                    let place = Place {
                        column: Column::Control,
                        fill: false,
                    };
                    let shared = &shared;
                    let row = widgets::PrefRow::new(label, move |ui| {
                        let mut out = shared.borrow_mut();
                        let before = out.events.len();
                        match child {
                            // A setting's switch, as the Preferences pages draw
                            // theirs, its words the row's label.
                            Widget::Check(check) if check.label.is_empty() => {
                                let mut on = check.on;
                                let changed = ui
                                    .add_enabled_ui(check.enabled, |ui| {
                                        widgets::toggle(ui, &mut on)
                                    })
                                    .inner
                                    .changed();
                                if changed {
                                    out.events.push(PanelEvent::Toggle {
                                        id: check.id.clone(),
                                        on,
                                    });
                                }
                            }
                            other => show_one(ui, id, other, place, document, &mut out),
                        }
                        out.events.len() > before
                    });
                    match hint {
                        Some(hint) if !hint.is_empty() => row.hint(hint),
                        _ => row,
                    }
                })
                .collect();
            widgets::pref_group(ui, &group.title, rows, &filter);
        }
        Widget::Action(action) => {
            let icon = action.icon.as_deref().unwrap_or("plus");
            if widgets::icon_button(ui, icon, &action.label).clicked() {
                out.events.push(PanelEvent::Button {
                    id: action.id.clone(),
                });
            }
        }
        Widget::Cells(cells) => {
            let columns = cells.columns.max(1);
            let place = Place {
                column: Column::Grid,
                ..place
            };
            egui::Grid::new(id.with(("cells", &cells.id)))
                .num_columns(columns)
                .spacing(egui::vec2(SPACE_3, SPACE_1))
                .show(ui, |ui| {
                    for (i, child) in cells.children.iter().enumerate() {
                        show_one(ui, id, child, place, document, out);
                        if (i + 1) % columns == 0 {
                            ui.end_row();
                        }
                    }
                });
        }
    }
}

fn note_kind(kind: NoteKind) -> widgets::Note {
    match kind {
        NoteKind::Info => widgets::Note::Info,
        NoteKind::Success => widgets::Note::Success,
        NoteKind::Warning => widgets::Note::Warning,
        NoteKind::Error => widgets::Note::Error,
    }
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
    let label = |ui: &mut Ui| {
        if !flow.label.is_empty() {
            match place.column {
                Column::Labels(column) => column.cell(ui, &flow.label),
                Column::Grid => {
                    widgets::field_label(ui, &flow.label);
                }
                Column::Control => {}
            }
        }
    };
    let Some(fill) = fill else {
        let mut each = |ui: &mut Ui| {
            label(ui);
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
        label(ui);
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
        Tone::Muted => TEXT3,
        other => note_of(other).color(),
    }
}

/// An sRGB colour a bench gave, else its tone's.
fn color_or(color: Option<[u8; 3]>, tone: Tone) -> egui::Color32 {
    match color {
        Some([r, g, b]) => egui::Color32::from_rgb(r, g, b),
        None => tone_color(tone),
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

/// Draw a [`Widget::Button`] in its style. The system's own button is
/// added as it stands, so a flow of them wraps from one line to the next.
fn button(ui: &mut Ui, widget: &Widget) -> egui::Response {
    let Widget::Button {
        label,
        style,
        enabled,
        ..
    } = widget
    else {
        return ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover());
    };
    let plain = match style {
        ButtonStyle::Plain => Some(RichText::new(label)),
        ButtonStyle::PlainSmall => Some(RichText::new(label).font(ui_kit::theme::sans(FONT_XS))),
        _ => None,
    };
    if let Some(text) = plain {
        return ui.add_enabled(
            *enabled,
            egui::Button::new(text).wrap_mode(egui::TextWrapMode::Extend),
        );
    }
    // Added in place rather than in a child, so a flow wraps it whole: its
    // label stays on one line, and in a row too full for it, it moves to
    // the next line rather than wrap a letter or two a line.
    let wrap = ui.style().wrap_mode;
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
    let response = ui.add_enabled(*enabled, |ui: &mut Ui| match style {
        ButtonStyle::Primary => widgets::primary_button(ui, label),
        ButtonStyle::Secondary => widgets::secondary_button(ui, label),
        ButtonStyle::Destructive => widgets::destructive_button(ui, label),
        ButtonStyle::Outline => widgets::accent_outline_button(ui, label),
        _ => widgets::small_secondary_button(ui, label),
    });
    ui.style_mut().wrap_mode = wrap;
    response
}

impl PanelOutput {
    /// Write the formulas typed into bound numbers into the document,
    /// marking their features to rebuild; a feature that is not there is
    /// logged as a warning. Returns the ids of
    /// the numbers a formula now sets: their `Number` events carry the
    /// formula's value, which a bench keeping plain numbers in its data
    /// leaves unwritten (a package is told it all the same).
    pub fn write_formulas(&self, ctx: &mut crate::WorkbenchRuntimeContext) -> Vec<String> {
        for FormulaSet { bind, formula, .. } in &self.formulas {
            let Ok(uuid) = uuid::Uuid::parse_str(&bind.feature) else {
                ctx.log_warn(format!("A formula names no feature: {}", bind.feature));
                continue;
            };
            let feature = FeatureId(uuid);
            match ctx
                .document
                .set_feature_formula(feature, &bind.key, formula.clone())
            {
                Ok(()) => ctx.document.mark_feature_dirty(feature),
                Err(why) => ctx.log_warn(why.to_string()),
            }
        }
        self.formulas
            .iter()
            .filter(|set| set.formula.is_some())
            .map(|set| set.id.clone())
            .collect()
    }

    /// The events a bench keeping plain numbers takes, after
    /// [`Self::write_formulas`]: those but the values a formula set.
    pub fn events_besides(&self, by_formula: &[String]) -> impl Iterator<Item = &PanelEvent> {
        self.events.iter().filter(move |event| {
            !(matches!(event, PanelEvent::Number { .. })
                && by_formula.iter().any(|id| id == event.id()))
        })
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
            // The blocks of the native panels.
            bench_api::Plot {
                height: 72.0,
                series: vec![bench_api::PlotSeries {
                    points: vec![[0.0, 0.0], [5.0, 2.0], [10.0, 1.0]],
                    tone: Tone::Warning,
                }],
            }
            .into(),
            bench_api::Fields {
                label_width: Some(110.0),
                children: vec![
                    bench_api::Quantity {
                        id: "torque".into(),
                        label: "Torque".into(),
                        unit: Some("N·m".into()),
                        ..Default::default()
                    }
                    .into(),
                ],
            }
            .into(),
            bench_api::Card {
                icon: Some("sketch".into()),
                title: "Card".into(),
                children: vec![
                    bench_api::Caption {
                        text: "small".into(),
                        tone: Tone::Muted,
                        small: true,
                        overline: true,
                        ..Default::default()
                    }
                    .into(),
                    bench_api::Gap::default().into(),
                ],
                ..Default::default()
            }
            .into(),
            bench_api::Alert {
                id: Some("alert".into()),
                kind: NoteKind::Error,
                title: Some("Conflict".into()),
                text: "Click".into(),
            }
            .into(),
            bench_api::FieldGrid {
                id: "grid".into(),
                roomy: true,
                apart: false,
                children: vec![
                    bench_api::Quantity {
                        id: "q".into(),
                        label: "Bound".into(),
                        bind: Some(Bind {
                            feature: feature.into(),
                            key: "/length".into(),
                        }),
                        speed: Some(0.5),
                        width: Some(90.0),
                        ..Default::default()
                    }
                    .into(),
                    bench_api::Quantity {
                        id: "steps".into(),
                        label: "Degree".into(),
                        value: 3.0,
                        stepper: Some(bench_api::Stepper {
                            down: true,
                            ..Default::default()
                        }),
                        ..Default::default()
                    }
                    .into(),
                    bench_api::Check {
                        id: "c".into(),
                        label: "Corner".into(),
                        text: "Keep it".into(),
                        enabled: false,
                        hint: "why".into(),
                        ..Default::default()
                    }
                    .into(),
                    bench_api::Select {
                        id: "s".into(),
                        label: "Font".into(),
                        options: vec!["A".into(), "B".into()],
                        width: Some(160.0),
                        ..Default::default()
                    }
                    .into(),
                    bench_api::Entry {
                        id: "e".into(),
                        label: "Text".into(),
                        multiline: true,
                        mono: true,
                        live: true,
                        ..Default::default()
                    }
                    .into(),
                ],
            }
            .into(),
            Widget::Button {
                id: "plain".into(),
                label: "Plain".into(),
                style: ButtonStyle::PlainSmall,
                enabled: true,
            },
            bench_api::ItemList {
                id: "rows".into(),
                items: vec![
                    Item {
                        label: "Line".into(),
                        related: true,
                        color: Some([10, 20, 30]),
                        value_tone: Tone::Accent,
                        flag: Some(bench_api::StatusLine {
                            text: "lost".into(),
                            tone: Tone::Warning,
                            detail: "gone".into(),
                        }),
                        badges: vec![bench_api::Badge {
                            icon: "constraint-horizontal".into(),
                            lit: true,
                            color: Some([200, 100, 0]),
                            ..Default::default()
                        }],
                        reveal: true,
                        ..Default::default()
                    },
                    Item {
                        label: "Length".into(),
                        field: Some(Box::new(
                            bench_api::Quantity {
                                id: "len".into(),
                                value: 4.0,
                                ..Default::default()
                            }
                            .into(),
                        )),
                        ..Default::default()
                    },
                    Item {
                        label: "named".into(),
                        editing: true,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }
            .into(),
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

    /// A panel of lists: a state, a folded section with a summary, one
    /// filter with a kind selector over two lists (counts, values, badges,
    /// tags, an action), and a folded section of numbers.
    fn lists_panel() -> Vec<Widget> {
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
                    ..Default::default()
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
                    ..Default::default()
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
                    ..Default::default()
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
    fn blocks_tell_the_bench_what_the_pointer_did() {
        let panel = Driven::new(lists_panel());
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

    /// The blocks the application's own tasks are made of, a tight stack
    /// of them, each told apart by its words.
    fn task_blocks() -> Vec<Widget> {
        let picture = bench_api::Picture {
            key: "grey".into(),
            width: 2,
            height: 2,
            rgba: vec![128; 16],
        };
        vec![Widget::Stack(bench_api::Stack {
            tight: true,
            spacing: None,
            children: vec![
                bench_api::Caption {
                    text: "Colours".into(),
                    strong: true,
                    ..Default::default()
                }
                .into(),
                bench_api::Swatches {
                    id: "colours".into(),
                    swatches: vec![
                        bench_api::Swatch::default(),
                        bench_api::Swatch {
                            color: [200, 40, 40],
                            chosen: true,
                        },
                    ],
                    keys: false,
                }
                .into(),
                bench_api::ColorField {
                    id: "own".into(),
                    color: [0.2, 0.4, 0.6],
                    ..Default::default()
                }
                .into(),
                bench_api::Range {
                    id: "through".into(),
                    max: 90.0,
                    suffix: " %".into(),
                    ..Default::default()
                }
                .into(),
                bench_api::Gap {
                    size: GapSize::Section,
                }
                .into(),
                bench_api::RowButton {
                    id: "here".into(),
                    text: "1. Where it is".into(),
                    current: true,
                }
                .into(),
                bench_api::RowButton {
                    id: "after".into(),
                    text: "2. After it".into(),
                    current: false,
                }
                .into(),
                bench_api::Selectable {
                    id: "second".into(),
                    text: "Second".into(),
                    selected: false,
                }
                .into(),
                bench_api::Gallery {
                    id: "patterns".into(),
                    tiles: vec![bench_api::Tile {
                        label: "Grey".into(),
                        picture,
                    }],
                    selected: Some(0),
                }
                .into(),
                bench_api::FieldGrid {
                    id: "apart".into(),
                    apart: true,
                    children: vec![
                        bench_api::Entry {
                            id: "formula".into(),
                            label: "Formula".into(),
                            formulas: true,
                            large: true,
                            live: true,
                            ..Default::default()
                        }
                        .into(),
                    ],
                    ..Default::default()
                }
                .into(),
                bench_api::SettingsGroup {
                    title: "Settings".into(),
                    children: vec![
                        bench_api::Check {
                            id: "deep".into(),
                            label: "Deep".into(),
                            text: "Go deep".into(),
                            ..Default::default()
                        }
                        .into(),
                    ],
                }
                .into(),
            ],
        })]
    }

    /// Each block of the tasks tells what was clicked: a swatch and a
    /// tile their place, a row button and a selectable their id (the
    /// current row none), a settings row's control its own event.
    #[test]
    fn the_task_blocks_tell_their_clicks() {
        let panel = Driven::new(task_blocks());
        panel.frame(Vec::new());
        let (texts, _) = panel.frame(Vec::new());
        let at = |text: &str| drawn(&texts, text)[0].center();
        let title = drawn(&texts, "Colours")[0];
        let second = egui::pos2(
            title.left() + SWATCH * 1.5 + 2.0,
            title.bottom() + SPACE_1 + 11.0,
        );
        assert_eq!(
            panel.click(second),
            vec![PanelEvent::Choice {
                id: "colours".into(),
                index: 1
            }]
        );
        assert!(
            panel.click(at("1. Where it is")).is_empty(),
            "the current row"
        );
        for (text, event) in [
            ("2. After it", PanelEvent::Button { id: "after".into() }),
            (
                "Second",
                PanelEvent::Button {
                    id: "second".into(),
                },
            ),
            (
                "Grey",
                PanelEvent::Choice {
                    id: "patterns".into(),
                    index: 0,
                },
            ),
            (
                "Go deep",
                PanelEvent::Toggle {
                    id: "deep".into(),
                    on: true,
                },
            ),
        ] {
            assert_eq!(panel.click(at(text)), vec![event], "{text}");
        }
        assert!(!drawn(&texts, "Deep").is_empty(), "the row's label");
    }

    /// Choices side by side tell the option picked, never the one
    /// chosen; a choice shows its own text closed; a labelled flow starts
    /// its children at the label column's end; a bare number writes its
    /// unit as given; a live entry tells each change and how it was left.
    #[test]
    fn choices_flows_and_live_entries_tell_what_they_did() {
        let panel = Driven::new(vec![Widget::Fields(bench_api::Fields {
            label_width: Some(96.0),
            children: vec![
                bench_api::Select {
                    id: "op".into(),
                    label: "Operation".into(),
                    options: vec!["Fuse".into(), "Cut".into()],
                    segmented: true,
                    ..Default::default()
                }
                .into(),
                bench_api::Select {
                    id: "edges".into(),
                    label: "Edges".into(),
                    options: vec!["All edges".into()],
                    text: Some("3 edge(s)".into()),
                    ..Default::default()
                }
                .into(),
                Flow {
                    label: "Edge".into(),
                    children: vec![
                        bench_api::Caption {
                            text: "(1.0, 2.0, 3.0)".into(),
                            ..Default::default()
                        }
                        .into(),
                    ],
                    ..Default::default()
                }
                .into(),
                Quantity {
                    id: "sides".into(),
                    label: "Sides".into(),
                    value: 6.0,
                    decimals: 0,
                    unit: Some(" faces".into()),
                    bare: true,
                    ..Default::default()
                }
                .into(),
                bench_api::Entry {
                    id: "name".into(),
                    label: "Name".into(),
                    value: "Pad".into(),
                    live: true,
                    ..Default::default()
                }
                .into(),
            ],
        })]);
        panel.frame(Vec::new());
        let (texts, _) = panel.frame(Vec::new());
        assert!(!drawn(&texts, "3 edge(s)").is_empty());
        assert!(!drawn(&texts, " faces").is_empty(), "{texts:?}");
        let label = drawn(&texts, "Edge")[0];
        let value = drawn(&texts, "(1.0, 2.0, 3.0)")[0];
        assert!(value.left() >= label.left() + 96.0, "{label:?} {value:?}");
        let cut = PanelEvent::Choice {
            id: "op".into(),
            index: 1,
        };
        assert_eq!(panel.click(drawn(&texts, "Cut")[0].center()), vec![cut]);
        assert!(panel.click(drawn(&texts, "Fuse")[0].center()).is_empty());

        panel.click(drawn(&texts, "Pad")[0].center());
        let (_, typed) = panel.frame(vec![egui::Event::Text("X".into())]);
        assert!(
            matches!(&typed[..], [PanelEvent::Text { id, value }] if id == "name" && value.contains('X')),
            "each change is told: {typed:?}"
        );
        let escape = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let (_, left) = panel.frame(vec![escape]);
        assert!(
            left.contains(&PanelEvent::TextLeft {
                id: "name".into(),
                value: "Pad".into(),
                escaped: true
            }),
            "{left:?}"
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
        let mut widgets = every_widget(&feature.0.to_string());
        widgets.extend(task_blocks());
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
        let out = PanelOutput {
            formulas: vec![FormulaSet {
                id: "bound".into(),
                bind: Bind {
                    feature: feature.0.to_string(),
                    key: "/length".into(),
                },
                formula: Some("2 * 3".into()),
            }],
            events: vec![
                PanelEvent::Number {
                    id: "bound".into(),
                    value: 6.0,
                },
                PanelEvent::Number {
                    id: "other".into(),
                    value: 1.0,
                },
            ],
            ..Default::default()
        };
        let mut ctx =
            crate::WorkbenchRuntimeContext::new(&mut document, [0.0; 3], [0.0; 3], (0, 0, 8, 8));
        let set = out.write_formulas(&mut ctx);
        assert_eq!(set, ["bound"]);
        assert_eq!(
            out.events_besides(&set).map(|e| e.id()).collect::<Vec<_>>(),
            ["other"]
        );
        let node = document.get_feature_meta(feature).unwrap();
        assert_eq!(
            node.formulas.get("/length").map(String::as_str),
            Some("2 * 3")
        );
        assert!(node.dirty);
    }

    /// The words drawn by `widgets` under the Preferences search `filter`.
    fn drawn_words(widgets: &[Widget], filter: &str) -> String {
        fn texts(shape: &egui::Shape, out: &mut String) {
            match shape {
                egui::Shape::Text(text) => {
                    out.push_str(text.galley.text());
                    out.push('\n');
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        ui_kit::theme::apply_theme(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            show_filtered(ui, egui::Id::new("settings_test"), widgets, None, filter);
        });
        output.textures_delta.clear();
        let mut words = String::new();
        for clipped in &output.shapes {
            texts(&clipped.shape, &mut words);
        }
        words
    }

    #[test]
    fn settings_rows_follow_the_preferences_search_by_label_and_hint() {
        let settings = [Widget::Settings(bench_api::SettingsGroup {
            title: "Look".into(),
            children: vec![
                Widget::Hinted {
                    hint: "Lines across the plane".into(),
                    widget: Box::new(Widget::Check(bench_api::Check {
                        id: "grid".into(),
                        text: "Grid".into(),
                        ..Default::default()
                    })),
                },
                Widget::Quantity(bench_api::Quantity {
                    id: "size".into(),
                    label: "Label size".into(),
                    value: 12.0,
                    ..Default::default()
                }),
            ],
        })];
        let all = drawn_words(&settings, "");
        assert!(all.contains("Grid") && all.contains("Label size"), "{all}");
        let by_hint = drawn_words(&settings, "ACROSS");
        assert!(
            by_hint.contains("Grid") && !by_hint.contains("Label size"),
            "{by_hint}"
        );
        let by_label = drawn_words(&settings, "label");
        assert!(
            !by_label.contains("Grid") && by_label.contains("Label size"),
            "{by_label}"
        );
        assert!(!drawn_words(&settings, "nothing").contains("Look"));
    }
}
