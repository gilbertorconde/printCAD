//! The words, fields and buttons the application's own tasks are made
//! of, as declared panel blocks (`bench_api::Widget`), and the one call
//! that draws a task's blocks: the same drawing a bench's or a package's
//! panel gets.

use bench_api::panel;
pub use bench_api::panel::{button, gap, hinted, indexed, srgb};
use bench_api::{ButtonStyle, Dim, GapSize, PanelEvent, Quantity, Stack, Tone, Widget};
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

/// A secondary button, enabled.
pub fn secondary(id: &str, text: &str) -> Widget {
    button(id, text, ButtonStyle::Secondary, true)
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
pub fn offset(id: &str, label: &str, value: f32) -> Widget {
    Quantity {
        min: Some(-1.0e6),
        max: Some(1.0e6),
        ..quantity(id, label, value, Dim::Length)
    }
    .into()
}

/// A turn, degrees, within `min..=max` when given.
pub fn degrees(id: &str, label: &str, value: f32, range: Option<(f64, f64)>) -> Widget {
    Quantity {
        speed: Some(0.5),
        min: range.map(|r| r.0),
        max: range.map(|r| r.1),
        ..quantity(id, label, value, Dim::Angle)
    }
    .into()
}

/// A length above zero, mm, within `min..=max`.
pub fn length(id: &str, label: &str, value: f32, min: f64, max: f64) -> Widget {
    Quantity {
        min: Some(min),
        max: Some(max),
        ..quantity(id, label, value, Dim::Length)
    }
    .into()
}

/// One of `options`, `width` points wide.
pub fn select<S: Into<String>>(
    id: &str,
    label: &str,
    options: impl IntoIterator<Item = S>,
    selected: usize,
    width: f32,
) -> Widget {
    bench_api::Select {
        options: options.into_iter().map(Into::into).collect(),
        width: Some(width),
        ..panel::select(id, label, [], selected)
    }
    .into()
}
