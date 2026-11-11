//! Fields: text, numbers (bound to formulas or plain), labelled rows and
//! grids, buttons, swatches and picture tiles.

use super::*;

/// The side of a [`Widget::Swatches`] square, points.
pub(super) const SWATCH: f32 = 22.0;

/// The height of a [`Widget::RowButton`], points.
pub(super) const ROW_BUTTON_HEIGHT: f32 = 22.0;

/// The side of a [`Widget::Gallery`] picture, points; its name goes under.
pub(super) const TILE: f32 = 52.0;

/// A readout field's width when the widget gives none: room for a short
/// formula and what it comes to.
const READOUT_WIDTH: f32 = 120.0;

/// A [`Widget::Swatches`] block: the squares side by side; the one
/// clicked.
pub(super) fn swatches(ui: &mut Ui, set: &bench_api::Swatches) -> Option<usize> {
    if set.keys {
        for swatch in &set.swatches {
            let (rect, _) =
                ui.allocate_exact_size(egui::Vec2::splat(SPACE_3), egui::Sense::hover());
            let [r, g, b] = swatch.color;
            ui.painter()
                .rect_filled(rect, RADIUS_SM, egui::Color32::from_rgb(r, g, b));
        }
        return None;
    }
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::splat(2.0);
        for (i, swatch) in set.swatches.iter().enumerate() {
            let (rect, response) =
                ui.allocate_exact_size(egui::Vec2::splat(SWATCH), egui::Sense::click());
            let painter = ui.painter();
            let [r, g, b] = swatch.color;
            painter.rect_filled(
                rect.shrink(2.0),
                RADIUS_SM,
                egui::Color32::from_rgb(r, g, b),
            );
            let stroke = if swatch.chosen {
                egui::Stroke::new(2.0, ACCENT)
            } else if response.hovered() {
                egui::Stroke::new(1.0, TEXT2)
            } else {
                egui::Stroke::new(1.0, BORDER)
            };
            painter.rect_stroke(
                rect.shrink(1.0),
                RADIUS_SM,
                stroke,
                egui::StrokeKind::Inside,
            );
            if response.clicked() {
                picked = Some(i);
            }
        }
    });
    picked
}

/// A [`Widget::Gallery`] block: the pictures side by side, each over its
/// name; the one clicked.
pub(super) fn gallery(ui: &mut Ui, gallery: &bench_api::Gallery) -> Option<usize> {
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::splat(SPACE_1);
        for (i, tile) in gallery.tiles.iter().enumerate() {
            let chosen = gallery.selected == Some(i);
            let (rect, response) =
                ui.allocate_exact_size(egui::Vec2::new(TILE, TILE + 14.0), egui::Sense::click());
            let image = egui::Rect::from_min_size(rect.min, egui::Vec2::splat(TILE));
            if let Some(texture) = picture(ui.ctx(), &tile.picture) {
                ui.painter().image(
                    texture.id(),
                    image,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            let stroke = if chosen {
                egui::Stroke::new(2.0, ACCENT)
            } else if response.hovered() {
                egui::Stroke::new(1.0, TEXT2)
            } else {
                egui::Stroke::new(1.0, BORDER)
            };
            ui.painter()
                .rect_stroke(image, RADIUS_SM, stroke, egui::StrokeKind::Inside);
            ui.painter().text(
                egui::pos2(image.center().x, image.bottom() + 2.0),
                egui::Align2::CENTER_TOP,
                &tile.label,
                ui_kit::theme::sans(FONT_XS),
                if chosen { TEXT1 } else { TEXT3 },
            );
            if response.clicked() {
                picked = Some(i);
            }
        }
    });
    picked
}

/// A [`bench_api::Picture`] on the screen, uploaded the first time its
/// key is shown; `None` for one whose pixels do not fill its size.
pub(super) fn picture(
    ctx: &egui::Context,
    picture: &bench_api::Picture,
) -> Option<egui::TextureHandle> {
    let id = egui::Id::new(("panel_picture", &picture.key));
    if let Some(handle) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(handle);
    }
    let size = [picture.width as usize, picture.height as usize];
    if size[0] * size[1] * 4 != picture.rgba.len() || picture.rgba.is_empty() {
        return None;
    }
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &picture.rgba);
    let handle = ctx.load_texture(
        format!("panel_picture::{}", picture.key),
        image,
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|d| d.insert_temp(id, handle.clone()));
    Some(handle)
}

/// What a text field reported this frame.
pub(super) struct Typed {
    /// The text to tell: at each change when live, else as left.
    pub(super) text: Option<String>,
    /// A live field left this frame: its text, and whether by Escape.
    pub(super) left: Option<(String, bool)>,
}

/// A one-line text field holding its draft while focused: the text as
/// left, when focus goes and it changed. `placeholder` shows while empty.
/// A live one tells each change, and when it is left. In a `card` a
/// one-line field holds its text against the right edge, as numbers stand.
pub(super) fn text_edit(
    ui: &mut Ui,
    key: egui::Id,
    entry: &bench_api::Entry,
    width: f32,
    card: bool,
    document: Option<&Document>,
) -> Typed {
    let align = if card && !entry.multiline {
        egui::Align::Max
    } else {
        egui::Align::Min
    };
    let value = entry.value.as_str();
    let mut draft: String = if entry.live {
        value.to_string()
    } else {
        ui.data(|d| d.get_temp(key))
            .unwrap_or_else(|| value.to_string())
    };
    let size = if entry.large { FONT_MD } else { FONT_SM };
    let font = if entry.mono {
        ui_kit::theme::mono(size)
    } else {
        ui_kit::theme::sans(size)
    };
    let hint = RichText::new(&entry.placeholder).color(TEXT3);
    let response = match document.filter(|_| entry.formulas && !entry.multiline) {
        Some(document) => {
            let candidates = || crate::formula_candidates(document);
            ui_kit::completion::completing_text_edit(ui, key, &mut draft, &candidates, |e| {
                e.font(font)
                    .hint_text(hint)
                    .desired_width(width)
                    .horizontal_align(align)
            })
            .response
        }
        None => {
            let edit = if entry.multiline {
                egui::TextEdit::multiline(&mut draft).desired_rows(1)
            } else {
                egui::TextEdit::singleline(&mut draft)
            };
            ui.add(
                edit.id(key)
                    .font(font)
                    .hint_text(hint)
                    .desired_width(width)
                    .horizontal_align(align),
            )
        }
    };
    if entry.live {
        let left = response.lost_focus().then(|| {
            let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
            (draft.clone(), escaped)
        });
        return Typed {
            text: response.changed().then_some(draft),
            left,
        };
    }
    if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(key, draft.clone()));
    }
    let mut text = None;
    if response.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(key));
        if draft != value {
            text = Some(draft);
        }
    }
    Typed { text, left: None }
}

/// A labelled row: the section's label column, then the control. An
/// empty label takes no column, for a control in a [`Widget::Row`].
pub(super) fn row(ui: &mut Ui, place: Place, label: &str, control: impl FnOnce(&mut Ui)) {
    match place.column {
        Column::Grid if !label.is_empty() => {
            widgets::field_label(ui, label);
            control(ui);
        }
        Column::Grid | Column::Control => control(ui),
        Column::Labels(_) if label.is_empty() => {
            ui.horizontal(control);
        }
        Column::Labels(column) => column.row(ui, label, control),
    }
}

/// A labelled row as [`row`] draws it; with no label, the control alone
/// where it stands, as a check box sits in a list of them.
pub(super) fn row_or_bare(ui: &mut Ui, place: Place, label: &str, control: impl FnOnce(&mut Ui)) {
    if label.is_empty() {
        control(ui);
    } else {
        row(ui, place, label, control);
    }
}

/// A [`Widget::Grid`]: a field's label and control fill a row, a widget
/// with no label one cell of it.
pub(super) fn field_grid(
    ui: &mut Ui,
    id: egui::Id,
    grid: &FieldGrid,
    place: Place,
    document: Option<&Document>,
    out: &mut PanelOutput,
) {
    let gap = if grid.roomy { SPACE_2 } else { SPACE_1 };
    let apart = if grid.apart { SPACE_3 } else { SPACE_2 };
    let place = Place {
        column: Column::Grid,
        ..place
    };
    egui::Grid::new(id.with(("grid", &grid.id)))
        .num_columns(2)
        .spacing([apart, gap])
        .show(ui, |ui| {
            let mut cells = 0;
            for child in &grid.children {
                show_one(ui, id, child, place, document, out);
                cells += if labels_of(std::slice::from_ref(child)).is_empty() {
                    1
                } else {
                    2
                };
                if cells >= 2 {
                    ui.end_row();
                    cells = 0;
                }
            }
            if cells > 0 {
                ui.end_row();
            }
        });
}

/// A [`Widget::Quantity`]: its control, and its lock after it when it
/// has one.
pub(super) fn quantity(
    ui: &mut Ui,
    id: egui::Id,
    q: &Quantity,
    fill: bool,
    document: Option<&Document>,
    out: &mut PanelOutput,
) {
    let Some(held) = q.held else {
        return quantity_control(ui, id, q, fill, document, out);
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_1;
        quantity_control(ui, id, q, false, document, out);
        if widgets::lock_toggle(ui, held).clicked() {
            out.events.push(PanelEvent::Toggle {
                id: q.id.clone(),
                on: !held,
            });
        }
    });
}

/// A [`Widget::Quantity`]'s control: the package's text in a box it does
/// not edit, a stepper, a field taking formulas when bound and drawn with
/// a document (spanning the rest of the row when it `fill`s), or a plain
/// number field.
fn quantity_control(
    ui: &mut Ui,
    id: egui::Id,
    q: &Quantity,
    fill: bool,
    document: Option<&Document>,
    out: &mut PanelOutput,
) {
    let unit = match (&q.unit, q.dim) {
        (Some(unit), _) => unit.as_str(),
        (None, Dim::Length) => "mm",
        (None, Dim::Angle) => "°",
        (None, Dim::Number) => "",
    };
    // A number box pads outside the width it is given, a text field
    // inside: in a card both stand [`CARD_FIELD_WIDTH`] wide.
    let width = q
        .width
        .or(out.card.then_some(CARD_FIELD_WIDTH - 2.0 * SPACE_2));
    if let Some(text) = &q.shown {
        let width = if fill {
            ui.available_width()
        } else {
            width.unwrap_or(READOUT_WIDTH)
        };
        widgets::qty_readout(ui, text, unit, q.focus, width);
        return;
    }
    let clamp = |v: f64| v.clamp(q.min.unwrap_or(f64::MIN), q.max.unwrap_or(f64::MAX));
    // What a drag, the wheel or a stepper's buttons may set; typing is held
    // only to `min` and `max`.
    let stops = widgets::intersect(
        &(q.min.unwrap_or(f64::NEG_INFINITY)..=q.max.unwrap_or(f64::INFINITY)),
        &(q.drag_min.unwrap_or(f64::NEG_INFINITY)..=q.drag_max.unwrap_or(f64::INFINITY)),
    );
    let stop = |v: f64| v.clamp(*stops.start(), *stops.end());
    let key = id.with(("quantity", &q.id));
    if q.focus {
        ui.memory_mut(|m| m.request_focus(key));
    }
    ui.add_enabled_ui(q.enabled, |ui| {
        if let Some(stepper) = &q.stepper {
            ui.horizontal(|ui| {
                let down = ui
                    .add_enabled(stepper.down, egui::Button::new("−"))
                    .on_hover_text(&stepper.down_hint);
                let text = if stepper.text.is_empty() {
                    format!("{:.*}", q.decimals, q.value)
                } else {
                    stepper.text.clone()
                };
                widgets::mono_label(ui, text, FONT_SM, TEXT1);
                let up = ui
                    .add_enabled(stepper.up, egui::Button::new("+"))
                    .on_hover_text(&stepper.up_hint);
                for (response, by) in [(down, -1.0), (up, 1.0)] {
                    if response.clicked() {
                        out.events.push(PanelEvent::Number {
                            id: q.id.clone(),
                            value: stop(q.value + by),
                        });
                    }
                }
            });
            return;
        }
        let bound = q.bind.as_ref().zip(document).and_then(|(bind, document)| {
            let feature = FeatureId(uuid::Uuid::parse_str(&bind.feature).ok()?);
            Some((bind, feature, document))
        });
        if let Some((bind, feature, document)) = bound {
            let want = expr_dim(q.dim);
            let formula = document.feature_formula(feature, &bind.key);
            let slot = document
                .evaluated_slots(feature)
                .iter()
                .find(|s| s.key == bind.key);
            let shown = match (formula, slot.map(|s| &s.result)) {
                (Some(_), Some(Ok(v))) => v.value,
                _ => q.value,
            };
            let slot_error = slot.and_then(|s| s.result.as_ref().err());
            let host = crate::DocumentFormulas {
                document,
                dim: want,
            };
            let mut field = widgets::FormulaField::new(key, shown, &host)
                .formula(formula)
                .error(q.error.as_deref().or(slot_error.map(String::as_str)))
                .unit(unit)
                .decimals(q.decimals)
                .drag_range(stops.clone())
                .speed(
                    q.speed
                        .unwrap_or(if q.dim == Dim::Angle { 1.0 } else { 0.1 }),
                );
            if fill {
                field = field.fill(ui.available_width());
            } else if let Some(width) = width {
                field = field.width(width);
            }
            match field.show(ui) {
                Some(widgets::FormulaEdit::Value(v)) => {
                    if formula.is_some() {
                        out.formulas.push(FormulaSet {
                            id: q.id.clone(),
                            bind: bind.clone(),
                            formula: None,
                        });
                    }
                    out.events.push(PanelEvent::Number {
                        id: q.id.clone(),
                        value: clamp(v),
                    });
                }
                Some(widgets::FormulaEdit::Formula(text)) => {
                    let now = document.evaluate_formula(&text, Some(want));
                    out.formulas.push(FormulaSet {
                        id: q.id.clone(),
                        bind: bind.clone(),
                        formula: Some(text),
                    });
                    if let Ok(v) = now {
                        out.events.push(PanelEvent::Number {
                            id: q.id.clone(),
                            value: v.value,
                        });
                    }
                }
                None => {}
            }
            return;
        }
        if q.bare {
            let mut v = q.value;
            let mut drag = egui::DragValue::new(&mut v).speed(q.speed.unwrap_or(0.1));
            if let Some(unit) = &q.unit {
                drag = drag.suffix(unit);
            }
            if q.min.is_some() || q.max.is_some() {
                drag = drag.range(q.min.unwrap_or(f64::MIN)..=q.max.unwrap_or(f64::MAX));
            }
            if q.decimals == 0 {
                drag = drag.fixed_decimals(0);
            }
            let response = ui.add(drag);
            widgets::hold_dragged(ui, &response, &mut v, &stops);
            if response.changed() {
                out.events.push(PanelEvent::Number {
                    id: q.id.clone(),
                    value: clamp(v),
                });
            }
            return;
        }
        let mut v = q.value;
        let mut field = widgets::QtyField::new(&mut v)
            .unit(unit)
            .decimals(q.decimals)
            .speed(q.speed.unwrap_or(0.1))
            .drag_range(stops.clone())
            .error(q.error.as_deref())
            .right_aligned(out.card);
        if q.min.is_some() || q.max.is_some() {
            field = field.range(q.min.unwrap_or(f64::MIN)..=q.max.unwrap_or(f64::MAX));
        }
        if let Some(width) = width {
            field = field.width(width);
        }
        if field.show(ui) {
            out.events.push(PanelEvent::Number {
                id: q.id.clone(),
                value: clamp(v),
            });
        }
    });
}

pub(super) fn expr_dim(dim: Dim) -> crate::expr::Dim {
    match dim {
        Dim::Length => crate::expr::Dim::LENGTH,
        Dim::Angle => crate::expr::Dim::ANGLE,
        Dim::Number => crate::expr::Dim::NUMBER,
    }
}
