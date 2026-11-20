//! Quantity fields: a number with its unit, dragged, stepped by the
//! wheel and typed, and the limits a drag stops at.

use super::*;

/// A number a [`QtyField`] edits: single precision for the host's own
/// values, double for a model's.
pub trait QtyValue {
    fn get(&self) -> f64;
    fn set(&mut self, v: f64);
}

impl QtyValue for f32 {
    fn get(&self) -> f64 {
        f64::from(*self)
    }
    fn set(&mut self, v: f64) {
        *self = v as f32;
    }
}

impl QtyValue for f64 {
    fn get(&self) -> f64 {
        *self
    }
    fn set(&mut self, v: f64) {
        *self = v;
    }
}

/// A numeric field: mono value with a unit suffix, dragging and typing both
/// work. `dim` renders it as not applicable; `error` paints it red with an
/// inline message.
pub struct QtyField<'a> {
    value: &'a mut dyn QtyValue,
    unit: &'a str,
    speed: f64,
    range: std::ops::RangeInclusive<f64>,
    drag_range: std::ops::RangeInclusive<f64>,
    decimals: usize,
    dim: bool,
    error: Option<&'a str>,
    width: f32,
    right: bool,
}

impl<'a> QtyField<'a> {
    pub fn new(value: &'a mut dyn QtyValue) -> Self {
        Self {
            value,
            unit: "",
            speed: 0.1,
            range: f64::NEG_INFINITY..=f64::INFINITY,
            drag_range: f64::NEG_INFINITY..=f64::INFINITY,
            decimals: 2,
            dim: false,
            error: None,
            width: 120.0,
            right: false,
        }
    }

    /// A length, above zero.
    pub fn mm(value: &'a mut dyn QtyValue) -> Self {
        Self::new(value).unit("mm").range(0.001..=1.0e6)
    }

    /// A position or a shift, mm: either way along its axis.
    pub fn offset(value: &'a mut dyn QtyValue) -> Self {
        Self::new(value).unit("mm").range(-1.0e6..=1.0e6)
    }

    pub fn degrees(value: &'a mut dyn QtyValue) -> Self {
        Self::new(value).unit("°").speed(0.5)
    }

    pub fn unit(mut self, unit: &'a str) -> Self {
        self.unit = unit;
        self
    }

    pub fn speed(mut self, speed: f64) -> Self {
        self.speed = speed;
        self
    }

    pub fn range(mut self, range: std::ops::RangeInclusive<f64>) -> Self {
        self.range = range;
        self
    }

    /// What a drag or the wheel may set, inside the range: they stop at
    /// its ends, where a typed value may go past (and be refused by
    /// whoever reads it).
    pub fn drag_range(mut self, range: std::ops::RangeInclusive<f64>) -> Self {
        self.drag_range = range;
        self
    }

    pub fn decimals(mut self, decimals: usize) -> Self {
        self.decimals = decimals;
        self
    }

    pub fn dim(mut self, dim: bool) -> Self {
        self.dim = dim;
        self
    }

    pub fn error(mut self, error: Option<&'a str>) -> Self {
        self.error = error;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// The value against the box's right edge, as a compact card's
    /// numbers stand.
    pub fn right_aligned(mut self, right: bool) -> Self {
        self.right = right;
        self
    }

    /// Draws the field; returns whether the value changed.
    pub fn show(self, ui: &mut Ui) -> bool {
        self.show_settling(ui).changed
    }

    /// Draws the field and says whether the value changed and whether an
    /// edit settled this frame: a drag let go, or typing finished. A caller
    /// that applies every change live saves on the settle.
    pub fn show_settling(self, ui: &mut Ui) -> QtyEdit {
        let border = if self.error.is_some() { DANGER } else { BORDER };
        let text = if self.dim {
            TEXT3
        } else if self.error.is_some() {
            DANGER
        } else {
            TEXT1
        };
        let mut changed = false;
        let mut settled = false;
        let mut hover: Option<Response> = None;
        Frame::new()
            .fill(BG2)
            .stroke(Stroke::new(1.0, border))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(Margin::symmetric(SPACE_2 as i8, 0))
            .show(ui, |ui| {
                ui.set_min_size(Vec2::new(self.width, INPUT - 2.0));
                ui.set_max_width(self.width);
                // An input's height, whatever the row it stands in offers:
                // the centred row inside would grow to fill a grid cell
                // as tall as a neighbour's last frame.
                ui.set_max_height(INPUT - 2.0);
                // Left-aligned, so the value sits at the start of the box it
                // is stretched across; right-aligned when asked.
                let row = if self.right {
                    Layout::right_to_left(egui::Align::Center)
                } else {
                    Layout::left_to_right(egui::Align::Center).with_main_align(egui::Align::Min)
                };
                ui.with_layout(row, |ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_1;
                    let focus = ui.visuals_mut();
                    focus.widgets.inactive.bg_fill = Color32::TRANSPARENT;
                    focus.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                    focus.widgets.inactive.bg_stroke = Stroke::NONE;
                    focus.widgets.hovered.bg_fill = Color32::TRANSPARENT;
                    focus.widgets.hovered.weak_bg_fill = Color32::TRANSPARENT;
                    focus.widgets.hovered.bg_stroke = Stroke::NONE;
                    focus.widgets.active.bg_stroke = Stroke::NONE;
                    focus.widgets.active.bg_fill = Color32::TRANSPARENT;
                    focus.override_text_color = Some(text);
                    ui.style_mut().override_font_id = Some(mono(FONT_SM));
                    let suffix = if self.unit.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", self.unit)
                    };
                    if self.error.is_none() {
                        // The whole box takes the click that starts typing,
                        // not only the digits drawn in it.
                        ui.spacing_mut().interact_size.x = ui.available_width();
                    }
                    let mut v = self.value.get();
                    let resp = ui.add_enabled(
                        !self.dim,
                        // The range bounds what is dragged or typed; a value
                        // outside it is shown as it is, never changed by
                        // being shown.
                        egui::DragValue::new(&mut v)
                            .speed(self.speed)
                            .range(self.range.clone())
                            .clamp_existing_to_range(false)
                            .fixed_decimals(self.decimals)
                            .suffix(suffix),
                    );
                    let stops = intersect(&self.range, &self.drag_range);
                    hold_dragged(ui, &resp, &mut v, &stops);
                    let wheeled = (!self.dim)
                        .then(|| wheel_step(ui, &resp, v, self.decimals, &stops))
                        .flatten();
                    if let Some(w) = wheeled {
                        v = w;
                    }
                    if resp.changed() || wheeled.is_some() {
                        self.value.set(v);
                        changed = true;
                    }
                    settled = resp.drag_stopped() || resp.lost_focus() || wheeled.is_some();
                    hover = Some(resp);
                    if let Some(err) = self.error {
                        ui.label(RichText::new(err).font(sans(FONT_XS)).color(DANGER));
                    }
                });
            });
        QtyEdit {
            changed,
            settled,
            response: hover,
        }
    }
}

/// A number field's box holding text it does not edit, drawn as
/// [`QtyField`] is: a value typed through other keys, `unit` after it,
/// against the box's right edge. `ringed` draws it in the accent, as the
/// field those keys type into.
pub fn qty_readout(ui: &mut Ui, text: &str, unit: &str, ringed: bool, width: f32) -> Response {
    let border = if ringed { ACCENT } else { BORDER };
    let text = if unit.is_empty() {
        text.to_string()
    } else {
        format!("{text} {unit}")
    };
    Frame::new()
        .fill(BG2)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(SPACE_2 as i8, 0))
        .show(ui, |ui| {
            ui.set_min_size(Vec2::new(width, INPUT - 2.0));
            ui.set_max_width(width);
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(RichText::new(text).font(mono(FONT_SM)).color(TEXT1))
                        .truncate(),
                );
            });
        })
        .response
}

/// A field's lock: a small square beside the field, filled in the
/// accent with the lock shut while its value is held, dim and open while
/// it follows. Answers the click that turns it.
pub fn lock_toggle(ui: &mut Ui, held: bool) -> Response {
    let side = 20.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), held, "Held"));
    if ui.is_rect_visible(rect) {
        let square = rect.shrink(2.0);
        let (fill, name, tint) = if held {
            (ACCENT, "lock", BG0)
        } else {
            (BG4, "unlock", TEXT2)
        };
        let stroke = if response.hovered() {
            Stroke::new(1.0, BORDER_STRONG)
        } else {
            Stroke::NONE
        };
        ui.painter().rect(
            square,
            CornerRadius::same(RADIUS_SM as u8),
            fill,
            stroke,
            egui::StrokeKind::Inside,
        );
        if let Some(image) = icon::image(ui.ctx(), name, 12.0, tint) {
            image.paint_at(
                ui,
                egui::Rect::from_center_size(square.center(), Vec2::splat(12.0)),
            );
        }
    }
    response
}

/// A number field's value stepped by the wheel turned over it (`response`):
/// a whole unit a notch, a tenth with Shift where the field shows decimals,
/// stopping on the ends of `stops` (see [`step_within`]).
/// The field takes the scroll, so the panel under it stays put.
pub fn wheel_step(
    ui: &Ui,
    response: &Response,
    value: f64,
    decimals: usize,
    stops: &std::ops::RangeInclusive<f64>,
) -> Option<f64> {
    if !response.hovered() || response.has_focus() {
        return None;
    }
    let wheel = |e: &egui::Event| match e {
        egui::Event::MouseWheel { delta, .. } => Some(*delta),
        _ => None,
    };
    let (delta, shift) = ui.input(|i| {
        let delta = i
            .events
            .iter()
            .filter_map(wheel)
            .fold(Vec2::ZERO, |a, d| a + d);
        (delta, i.modifiers.shift)
    });
    // Shift turns the wheel sideways.
    let turned = if delta.y != 0.0 { delta.y } else { delta.x };
    if turned == 0.0 {
        return None;
    }
    ui.ctx().input_mut(|i| {
        i.events.retain(|e| wheel(e).is_none());
        i.smooth_scroll_delta = Vec2::ZERO;
    });
    let step = if shift && decimals > 0 { 0.1 } else { 1.0 };
    Some(step_within(
        value,
        turned > 0.0,
        step,
        *stops.start(),
        *stops.end(),
    ))
}

/// The whole `step` after `value`, up or down, within `least..=most`:
/// a limit that is no whole step is a stop of its own, so from 4 with a
/// step of 1 and a most of 5.3 the steps up are 5 and 5.3, and down from
/// 5.3 they are 5 and 4.
pub fn step_within(value: f64, up: bool, step: f64, least: f64, most: f64) -> f64 {
    // A value a rounding error off a step counts as on it.
    let on = value / step;
    let slack = 1e-9 * on.abs().max(1.0);
    let next = if up {
        (on + slack).floor() + 1.0
    } else {
        (on - slack).ceil() - 1.0
    };
    (next * step).clamp(least, most)
}

/// The values both ranges hold.
pub fn intersect(
    a: &std::ops::RangeInclusive<f64>,
    b: &std::ops::RangeInclusive<f64>,
) -> std::ops::RangeInclusive<f64> {
    let lo = a.start().max(*b.start());
    lo..=a.end().min(*b.end()).max(lo)
}

/// A number field's value held within `stops` while it is dragged
/// (`response`), so the drag stops at the limit rather than carrying the
/// value past it; what is typed is left alone. The drag then moves from
/// the limit, not from where the pointer would have taken it.
pub fn hold_dragged(
    ui: &Ui,
    response: &Response,
    value: &mut f64,
    stops: &std::ops::RangeInclusive<f64>,
) {
    if !response.dragged() {
        return;
    }
    let held = value.clamp(*stops.start(), *stops.end());
    if held != *value {
        *value = held;
        // The drag keeps its unrounded value under the widget's id.
        ui.data_mut(|d| d.insert_temp::<f64>(response.id, held));
    }
}

/// What [`QtyField::show_settling`] saw this frame.
pub struct QtyEdit {
    pub changed: bool,
    /// A drag was let go or typing finished.
    pub settled: bool,
    /// The value's own response: its tooltip, whether it holds the
    /// keyboard.
    pub response: Option<Response>,
}

#[cfg(test)]
mod qty_field_tests {
    use super::QtyField;

    /// Shown, a field leaves a value outside its range as it is and says
    /// nothing changed: only an edit changes a value.
    #[test]
    fn showing_a_field_never_changes_its_value() {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let mut value = 0.0_f32;
        let mut changed = true;
        let mut out = ctx.run_ui(Default::default(), |ui| {
            changed = QtyField::mm(&mut value).show(ui);
        });
        out.textures_delta.clear();
        assert_eq!(value, 0.0, "0 is not raised to the range's 0.001");
        assert!(!changed);
    }

    /// A click anywhere in the box starts typing, not only one on the
    /// digits: a one-digit count leaves most of its box empty.
    #[test]
    fn a_click_at_the_end_of_the_box_types_into_it() {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let mut value = 3.0_f32;
        // One frame; the box the field drew.
        let frame = |time: f64, events: Vec<egui::Event>, value: &mut f32| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut boxed = egui::Rect::NOTHING;
            let mut out = ctx.run_ui(input, |ui| {
                boxed = ui
                    .horizontal(|ui| {
                        QtyField::new(value)
                            .range(1.0..=1000.0)
                            .decimals(0)
                            .show(ui);
                    })
                    .response
                    .rect;
            });
            out.textures_delta.clear();
            boxed
        };
        let boxed = frame(0.0, vec![], &mut value);
        let at = egui::pos2(boxed.right() - 6.0, boxed.center().y);
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(1.0, vec![egui::Event::PointerMoved(at)], &mut value);
        frame(1.1, vec![press(true)], &mut value);
        frame(1.2, vec![press(false)], &mut value);
        frame(1.3, vec![], &mut value);
        frame(1.4, vec![egui::Event::Text("5".into())], &mut value);
        let enter = egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        frame(1.5, vec![enter], &mut value);
        assert_eq!(value, 5.0, "clicked at {at:?} in the box {boxed:?}");
    }

    /// The wheel over a number field steps it a unit a notch, a tenth with
    /// Shift.
    #[test]
    fn the_wheel_over_a_field_steps_it() {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let mut value = 3.0_f32;
        let frame = |time: f64, events: Vec<egui::Event>, value: &mut f32| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut boxed = egui::Rect::NOTHING;
            let mut out = ctx.run_ui(input, |ui| {
                boxed = ui
                    .horizontal(|ui| QtyField::mm(value).show(ui))
                    .response
                    .rect;
            });
            out.textures_delta.clear();
            boxed
        };
        let boxed = frame(0.0, vec![], &mut value);
        let wheel = |shift| egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 1.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers {
                shift,
                ..Default::default()
            },
        };
        frame(
            1.0,
            vec![egui::Event::PointerMoved(boxed.center())],
            &mut value,
        );
        frame(1.1, vec![wheel(false)], &mut value);
        assert_eq!(value, 4.0);
        let shift = egui::Event::ModifiersChanged(egui::Modifiers {
            shift: true,
            ..Default::default()
        });
        frame(1.2, vec![shift, wheel(true)], &mut value);
        assert!((value - 4.1).abs() < 1e-6, "{value}");
    }
}
