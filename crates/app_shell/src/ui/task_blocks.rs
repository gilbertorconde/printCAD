//! The words, fields and buttons the application's own tasks are made
//! of, as declared panel blocks (`bench_api::Widget`), and the one call
//! that draws a task's blocks: the same drawing a bench's or a package's
//! panel gets. Each task walks its panel once through a
//! `bench_api::panel::Form`, which both declares the widgets and hands an
//! event to the field it names.

use bench_api::panel;
pub use bench_api::panel::{Bound, Form, gap, hinted, srgb};
use bench_api::{
    ButtonStyle, Check, Dim, GapSize, PanelEvent, Quantity, Select, Stack, Tone, Widget,
};
use core_document::Document;

/// Draw a task's widgets, one under another close together as a task's
/// lines sit; what the user did this frame.
pub fn draw(ui: &mut egui::Ui, document: &Document, widgets: Vec<Widget>) -> Vec<PanelEvent> {
    let stack = Widget::Stack(Stack {
        children: widgets,
        tight: true,
        ..Default::default()
    });
    let out = core_document::panel::show(
        ui,
        egui::Id::new("host_task"),
        std::slice::from_ref(&stack),
        Some(document),
    );
    // The application's tasks bind no number to a feature's parameter:
    // the UI holds the document only to read it.
    debug_assert!(out.formulas.is_empty(), "a host task bound a formula");
    out.events
}

/// Dim words explaining what is around them.
pub fn note(text: impl Into<String>) -> Widget {
    caption(text, Tone::Muted)
}

/// A field's name, beside it.
pub fn label(text: impl Into<String>) -> Widget {
    caption(text, Tone::Neutral)
}

/// Words in `tone`.
pub fn caption(text: impl Into<String>, tone: Tone) -> Widget {
    panel::caption(&text.into(), tone, false)
}

/// A part of a task under its heading: room above, the overline, a
/// little room under it.
pub fn heading(text: &str) -> [Widget; 3] {
    [
        gap(GapSize::Section),
        panel::overline(text),
        gap(GapSize::Small),
    ]
}

/// A heading, as `heading` makes it, next in `form`.
pub fn heading_in(form: &mut Form, text: &str) {
    for widget in heading(text) {
        form.push(widget);
    }
}

/// A secondary button named `name`, enabled.
pub fn secondary(form: &mut Form, name: &str, text: &str) -> Bound<Widget> {
    form.button(name, text, ButtonStyle::Secondary, true)
}

/// A number field named by `shape`'s id, bound to `value`.
pub fn number(form: &mut Form, value: &mut f32, shape: Quantity) -> Bound<Quantity> {
    let mut wide = f64::from(*value);
    let name = shape.id.clone();
    let bound = form.number(&name, &mut wide, shape);
    if bound.set {
        *value = wide as f32;
    }
    bound
}

/// A choice named by `shape`'s id, bound to the option `selected`.
pub fn choice(form: &mut Form, selected: &mut usize, shape: Select) -> Bound<Select> {
    let name = shape.id.clone();
    form.index(&name, selected, shape)
}

/// A check box named `name` with `text` beside it, bound to `on`.
pub fn check(form: &mut Form, name: &str, text: &str, on: &mut bool) -> Bound<Check> {
    let shape = Check {
        text: text.into(),
        ..Default::default()
    };
    form.check(name, on, shape)
}

/// The event, taken, when it names the widget `name` declares; the
/// widget's id.
pub fn raw<'e>(form: &mut Form<'e>, name: &str) -> (String, Option<&'e PanelEvent>) {
    let id = form.id(name);
    let event = form.take(&id);
    (id, event)
}

/// Widgets side by side on one line.
pub fn row(children: impl IntoIterator<Item = Widget>) -> Widget {
    panel::flow_no_wrap(children)
}

/// Widgets side by side, going on to the next line when the panel is
/// narrow.
pub fn wrapping(children: impl IntoIterator<Item = Widget>) -> Widget {
    panel::flow(children)
}

/// A two-column grid of fields; `apart` sets the controls a little
/// further from the labels.
pub fn grid(id: &str, apart: bool, children: impl IntoIterator<Item = Widget>) -> Widget {
    if apart {
        panel::grid_apart(id, children)
    } else {
        panel::grid(id, children)
    }
}

/// A number field: `dim` names its unit, `min` and `max` its range.
pub fn quantity(id: &str, label: &str, value: f32, dim: Dim) -> Quantity {
    Quantity {
        dim,
        ..panel::quantity(id, label, f64::from(value))
    }
}

/// A position or a shift along an axis, mm, either way.
pub fn offset(id: &str, label: &str, value: f32) -> Quantity {
    Quantity {
        min: Some(-1.0e6),
        max: Some(1.0e6),
        ..quantity(id, label, value, Dim::Length)
    }
}

/// A turn, degrees, within `min..=max` when given.
pub fn degrees(id: &str, label: &str, value: f32, range: Option<(f64, f64)>) -> Quantity {
    Quantity {
        speed: Some(0.5),
        min: range.map(|r| r.0),
        max: range.map(|r| r.1),
        ..quantity(id, label, value, Dim::Angle)
    }
}

/// A length above zero, mm, within `min..=max`.
pub fn length(id: &str, label: &str, value: f32, min: f64, max: f64) -> Quantity {
    Quantity {
        min: Some(min),
        max: Some(max),
        ..quantity(id, label, value, Dim::Length)
    }
}

/// One of `options`, `width` points wide.
pub fn select<S: Into<String>>(
    id: &str,
    label: &str,
    options: impl IntoIterator<Item = S>,
    selected: usize,
    width: f32,
) -> Select {
    Select {
        options: options.into_iter().map(Into::into).collect(),
        width: Some(width),
        ..panel::select(id, label, [], selected)
    }
}

/// Run `f` in a frame of a panel with the design's fonts, as the task
/// panel would.
#[cfg(test)]
pub fn in_frame(f: impl FnOnce(&mut egui::Ui)) {
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
