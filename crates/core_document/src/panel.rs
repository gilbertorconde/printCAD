//! Declared panels: a bench lists widgets as data (`bench_api::Widget`)
//! and the host draws them with the design system's controls, so a
//! package's task panel and Preferences page look and behave like the
//! built-in ones, formulas included.

use bench_api::{
    Bind, ButtonStyle, Callout, Cell, DiagramShape, DiagramStroke, Dim, Dimension, NoteKind,
    PanelEvent, Widget,
};
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
        Widget::Diagram {
            width,
            height,
            shapes,
            dimensions,
            callouts,
            ..
        } => diagram(ui, *width, *height, shapes, dimensions, callouts),
        Widget::Header { icon, title } => header(ui, icon, title),
        Widget::Value { label, value, mono } => row(ui, label, |ui| {
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
                    show_one(ui, id, child, document, out);
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
        } => row(ui, label, |ui| {
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
            ui.scope(|ui| show_one(ui, id, widget, document, out))
                .response
                .on_hover_text(hint);
        }
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
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let available = ui.available_width().max(1.0);
    let scale = (available / width).min(DIAGRAM_MAX_HEIGHT / height);
    let size = egui::vec2(width * scale, height * scale);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(available, size.y), egui::Sense::hover());
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

/// A labelled row: the label column, then the control. An empty label
/// takes no column, for a control in a [`Widget::Row`].
fn row(ui: &mut Ui, label: &str, control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        if label.is_empty() {
            control(ui);
            return;
        }
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
        ]
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
