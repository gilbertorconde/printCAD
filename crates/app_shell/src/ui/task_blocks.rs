//! The words, fields and buttons the application's own tasks are made
//! of, as declared panel blocks (`bench_api::Widget`), and the one call
//! that draws a task's blocks: the same drawing a bench's or a package's
//! panel gets.

use bench_api::{
    ButtonStyle, Caption, Dim, FieldGrid, Flow, Gap, GapSize, PanelEvent, Quantity, Select, Stack,
    Tone, Widget,
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
    Caption {
        text: text.into(),
        tone,
        ..Default::default()
    }
    .into()
}

/// A part of a task under its heading: room above, the overline, a
/// little room under it.
pub fn heading(text: &str) -> [Widget; 3] {
    [
        gap(GapSize::Section),
        Caption {
            text: text.into(),
            overline: true,
            ..Default::default()
        }
        .into(),
        gap(GapSize::Small),
    ]
}

pub fn gap(size: GapSize) -> Widget {
    Gap { size }.into()
}

pub fn button(id: &str, text: &str, style: ButtonStyle, enabled: bool) -> Widget {
    Widget::Button {
        id: id.into(),
        label: text.into(),
        style,
        enabled,
    }
}

/// A secondary button, enabled.
pub fn secondary(id: &str, text: &str) -> Widget {
    button(id, text, ButtonStyle::Secondary, true)
}

/// `widget` with words on hover.
pub fn hinted(hint: &str, widget: Widget) -> Widget {
    Widget::Hinted {
        hint: hint.into(),
        widget: Box::new(widget),
    }
}

/// Widgets side by side on one line.
pub fn row(children: impl IntoIterator<Item = Widget>) -> Widget {
    Flow {
        children: children.into_iter().collect(),
        no_wrap: true,
        ..Default::default()
    }
    .into()
}

/// Widgets side by side, going on to the next line when the panel is
/// narrow.
pub fn wrapping(children: impl IntoIterator<Item = Widget>) -> Widget {
    Flow {
        children: children.into_iter().collect(),
        ..Default::default()
    }
    .into()
}

/// A two-column grid of fields; `apart` sets the controls a little
/// further from the labels.
pub fn grid(id: &str, apart: bool, children: impl IntoIterator<Item = Widget>) -> Widget {
    FieldGrid {
        id: id.into(),
        children: children.into_iter().collect(),
        apart,
        ..Default::default()
    }
    .into()
}

/// A number field: `dim` names its unit, `min` and `max` its range.
pub fn quantity(id: &str, label: &str, value: f32, dim: Dim) -> Quantity {
    Quantity {
        id: id.into(),
        label: label.into(),
        value: f64::from(value),
        dim,
        ..Default::default()
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
    Select {
        id: id.into(),
        label: label.into(),
        options: options.into_iter().map(Into::into).collect(),
        selected,
        width: Some(width),
        ..Default::default()
    }
    .into()
}

/// The index after `prefix` in an id like `drop:3`.
pub fn indexed(id: &str, prefix: &str) -> Option<usize> {
    id.strip_prefix(prefix)?.parse().ok()
}

/// A colour as the swatches draw it: each part clamped and cut to a byte.
pub fn srgb(c: [f32; 3]) -> [u8; 3] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8)
}
