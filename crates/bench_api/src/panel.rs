//! Functions building a declared panel's blocks: layout, fields, words
//! and colours, each a block with the common settings filled in and the
//! rest at its default. The SDK re-exports them beside its section and
//! item-list builders; printCAD's own panels use them too.

use crate::*;

/// A state as a toned pill, the longer reason on hover: a widget of its
/// own, or a task header's through [`task_header`].
pub fn status(text: &str, tone: Tone, detail: &str) -> StatusLine {
    StatusLine {
        text: text.into(),
        tone,
        detail: detail.into(),
    }
}

/// The task header's `subtitle` under the title and `status` beside it.
pub fn task_header(subtitle: &str, status: Option<StatusLine>) -> Widget {
    TaskHeader {
        subtitle: subtitle.into(),
        status,
    }
    .into()
}

/// Widgets one under another.
pub fn stack(children: impl IntoIterator<Item = Widget>) -> Widget {
    Stack {
        children: children.into_iter().collect(),
        ..Default::default()
    }
    .into()
}

/// Widgets one under another, close together, as a task's lines of text
/// and fields sit.
pub fn tight_stack(children: impl IntoIterator<Item = Widget>) -> Widget {
    Stack {
        children: children.into_iter().collect(),
        tight: true,
        ..Default::default()
    }
    .into()
}

/// Widgets side by side, wrapping when the panel is narrow.
pub fn flow(children: impl IntoIterator<Item = Widget>) -> Widget {
    Flow {
        children: children.into_iter().collect(),
        ..Default::default()
    }
    .into()
}

/// Widgets side by side after a label in the section's label column, as
/// a field's: a value and the button that sets it.
pub fn labelled_flow(label: &str, children: impl IntoIterator<Item = Widget>) -> Widget {
    Flow {
        children: children.into_iter().collect(),
        label: label.into(),
        ..Default::default()
    }
    .into()
}

/// Widgets side by side on one line.
pub fn flow_no_wrap(children: impl IntoIterator<Item = Widget>) -> Widget {
    Flow {
        children: children.into_iter().collect(),
        no_wrap: true,
        ..Default::default()
    }
    .into()
}

/// `child` taking the rest of a flow's width.
pub fn fill(child: impl Into<Widget>) -> Widget {
    Fill {
        child: Box::new(child.into()),
    }
    .into()
}

/// Fields sharing one label column of their own.
pub fn fields(children: impl IntoIterator<Item = Widget>) -> Widget {
    Fields {
        children: children.into_iter().collect(),
        label_width: None,
    }
    .into()
}

/// Fields whose label column is `width` points wide.
pub fn fields_at(width: f32, children: impl IntoIterator<Item = Widget>) -> Widget {
    Fields {
        children: children.into_iter().collect(),
        label_width: Some(width),
    }
    .into()
}

/// A small heading in small capitals over what follows.
pub fn overline(text: &str) -> Widget {
    Caption {
        text: text.into(),
        overline: true,
        ..Default::default()
    }
    .into()
}

/// Lines along a strip `height` points tall, each `(points, tone)`.
pub fn plot(height: f32, series: impl IntoIterator<Item = (Vec<[f64; 2]>, Tone)>) -> Widget {
    Plot {
        height,
        series: series
            .into_iter()
            .map(|(points, tone)| PlotSeries { points, tone })
            .collect(),
    }
    .into()
}

/// A field with its label beside it at its own width, no column.
pub fn inline(child: impl Into<Widget>) -> Widget {
    Inline {
        child: Box::new(child.into()),
    }
    .into()
}

/// What is picked in the view, as a chip whose cross clears it.
pub fn pick_chip(id: &str, label: &str, value: Option<&str>, armed: bool) -> PickChip {
    PickChip {
        id: id.into(),
        label: label.into(),
        value: value.map(String::from),
        armed,
        placeholder: String::new(),
    }
}

/// A line of text with a placeholder while empty.
pub fn entry(id: &str, label: &str, value: &str, placeholder: &str) -> Widget {
    entry_with(id, label, value, placeholder).into()
}

/// An [`Entry`] to set further: `multiline`, `width`, `mono`, `live`.
pub fn entry_with(id: &str, label: &str, value: &str, placeholder: &str) -> Entry {
    Entry {
        id: id.into(),
        label: label.into(),
        value: value.into(),
        placeholder: placeholder.into(),
        ..Default::default()
    }
}

/// A [`Quantity`]: a number to set further (`dim`, `min`, `max`,
/// `decimals`, `speed`, `width`, `enabled`, `bind`, `stepper`).
pub fn quantity(id: &str, label: &str, value: f64) -> Quantity {
    Quantity {
        id: id.into(),
        label: label.into(),
        value,
        ..Default::default()
    }
}

/// A [`Quantity`] drawn as `text` between a down and an up button.
pub fn stepper(id: &str, label: &str, value: f64, text: &str, down: bool, up: bool) -> Quantity {
    Quantity {
        stepper: Some(Stepper {
            text: text.into(),
            down,
            up,
            ..Default::default()
        }),
        ..quantity(id, label, value)
    }
}

/// A labelled [`Check`] box, `text` beside it.
pub fn check(id: &str, label: &str, text: &str, on: bool) -> Check {
    Check {
        id: id.into(),
        label: label.into(),
        text: text.into(),
        on,
        ..Default::default()
    }
}

/// A [`Select`] of `options`.
pub fn select<'a>(
    id: &str,
    label: &str,
    options: impl IntoIterator<Item = &'a str>,
    selected: usize,
) -> Select {
    Select {
        id: id.into(),
        label: label.into(),
        options: options.into_iter().map(String::from).collect(),
        selected,
        ..Default::default()
    }
}

/// A [`Select`] of `options` side by side, the chosen one lit.
pub fn segmented<'a>(
    id: &str,
    label: &str,
    options: impl IntoIterator<Item = &'a str>,
    selected: usize,
) -> Select {
    Select {
        segmented: true,
        ..select(id, label, options, selected)
    }
}

/// Fields in a two-column [`FieldGrid`], rows close together.
pub fn grid(id: &str, children: impl IntoIterator<Item = Widget>) -> Widget {
    FieldGrid {
        id: id.into(),
        children: children.into_iter().collect(),
        ..Default::default()
    }
    .into()
}

/// Fields in a two-column [`FieldGrid`], its controls a little further
/// from the labels.
pub fn grid_apart(id: &str, children: impl IntoIterator<Item = Widget>) -> Widget {
    FieldGrid {
        id: id.into(),
        children: children.into_iter().collect(),
        apart: true,
        ..Default::default()
    }
    .into()
}

/// A note; with an `id`, told as `PanelEvent::Button` when clicked.
pub fn alert(id: Option<&str>, kind: NoteKind, title: Option<&str>, text: &str) -> Widget {
    Alert {
        id: id.map(String::from),
        kind,
        title: title.map(String::from),
        text: text.into(),
    }
    .into()
}

/// A raised [`Card`] under an icon and a title.
pub fn card(icon: Option<&str>, title: &str, children: impl IntoIterator<Item = Widget>) -> Widget {
    Card {
        icon: icon.map(String::from),
        title: title.into(),
        children: children.into_iter().collect(),
        ..Default::default()
    }
    .into()
}

/// Words in a tone; `small` in the small text size.
pub fn caption(text: &str, tone: Tone, small: bool) -> Widget {
    Caption {
        text: text.into(),
        tone,
        small,
        ..Default::default()
    }
    .into()
}

/// Words in the medium face and the primary text colour: a title over
/// the rows under it.
pub fn title(text: &str) -> Widget {
    Caption {
        text: text.into(),
        medium: true,
        primary: true,
        ..Default::default()
    }
    .into()
}

/// Room between widgets.
pub fn gap(size: GapSize) -> Widget {
    Gap { size }.into()
}

/// Words in the semibold face and the primary text colour: the name of
/// what the widgets after it edit.
pub fn strong(text: &str) -> Widget {
    Caption {
        text: text.into(),
        strong: true,
        ..Default::default()
    }
    .into()
}

/// Colours to click, as sRGB, `chosen` ringed; told as
/// `PanelEvent::Choice` with the swatch's place.
pub fn swatches(id: &str, colors: impl IntoIterator<Item = ([u8; 3], bool)>) -> Widget {
    Swatches {
        id: id.into(),
        swatches: colors
            .into_iter()
            .map(|(color, chosen)| Swatch { color, chosen })
            .collect(),
        keys: false,
    }
    .into()
}

/// Small squares of colour (sRGB) keying what the view paints; one
/// beside each caption in a flow makes a legend.
pub fn color_keys(colors: impl IntoIterator<Item = [u8; 3]>) -> Widget {
    Swatches {
        swatches: colors
            .into_iter()
            .map(|color| Swatch {
                color,
                chosen: false,
            })
            .collect(),
        keys: true,
        ..Default::default()
    }
    .into()
}

/// A colour in the system's picker, each part from 0 to 1; told as
/// `PanelEvent::Color`.
pub fn color(id: &str, label: &str, color: [f32; 3]) -> Widget {
    ColorField {
        id: id.into(),
        label: label.into(),
        color,
    }
    .into()
}

/// A slider from `min` to `max`, its value written with `suffix`; set
/// `decimals`, `step` and `enabled` further.
pub fn range(id: &str, label: &str, value: f64, min: f64, max: f64, suffix: &str) -> Range {
    Range {
        id: id.into(),
        label: label.into(),
        value,
        min,
        max,
        suffix: suffix.into(),
        ..Default::default()
    }
}

/// A frameless button the panel's width; the `current` one is lit and
/// takes no click.
pub fn row_button(id: &str, text: &str, current: bool) -> Widget {
    RowButton {
        id: id.into(),
        text: text.into(),
        current,
    }
    .into()
}

/// Words lit while `selected`, told as `PanelEvent::Button` when clicked.
pub fn selectable(id: &str, text: &str, selected: bool) -> Widget {
    Selectable {
        id: id.into(),
        text: text.into(),
        selected,
    }
    .into()
}

/// Pictures to choose among, each `(label, picture)`; told as
/// `PanelEvent::Choice` with the tile's place.
pub fn gallery(
    id: &str,
    tiles: impl IntoIterator<Item = (String, Picture)>,
    selected: Option<usize>,
) -> Widget {
    Gallery {
        id: id.into(),
        tiles: tiles
            .into_iter()
            .map(|(label, picture)| Tile { label, picture })
            .collect(),
        selected,
    }
    .into()
}

/// A bordered group of rows as the Preferences pages draw them, each
/// child's label at the left and its control at the right.
pub fn settings(title: &str, children: impl IntoIterator<Item = Widget>) -> Widget {
    SettingsGroup {
        title: title.into(),
        children: children.into_iter().collect(),
    }
    .into()
}

/// `children` in rows of `columns` cells, each column as wide as its
/// widest: a table of readings under column titles.
pub fn cells(id: &str, columns: usize, children: impl IntoIterator<Item = Widget>) -> Widget {
    Cells {
        id: id.into(),
        columns,
        children: children.into_iter().collect(),
    }
    .into()
}

/// A small icon button on its own, `hint` on hover; told as
/// `PanelEvent::Button` with `id`.
pub fn action(id: &str, icon: &str, hint: &str) -> Widget {
    PanelAction {
        id: id.into(),
        icon: Some(icon.into()),
        label: hint.into(),
        ..Default::default()
    }
    .into()
}

/// `widget` with `hint` on hover; `widget` alone when the hint is empty.
pub fn hinted(hint: &str, widget: Widget) -> Widget {
    if hint.is_empty() {
        widget
    } else {
        Widget::Hinted {
            hint: hint.into(),
            widget: Box::new(widget),
        }
    }
}

/// A button in `style`; told as `PanelEvent::Button` with `id`.
pub fn button(id: &str, label: &str, style: ButtonStyle, enabled: bool) -> Widget {
    Widget::Button {
        id: id.into(),
        label: label.into(),
        style,
        enabled,
    }
}

/// A note of `kind`, its `title` above the text when given.
pub fn note(kind: NoteKind, title: Option<&str>, text: &str) -> Widget {
    Widget::Note {
        kind,
        title: title.map(String::from),
        text: text.into(),
    }
}

/// A colour as sRGB bytes, each part from 0 to 1 clamped and rounded.
pub fn srgb(color: [f32; 3]) -> [u8; 3] {
    color.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The id of the `index`th of a kind of widget: `prefix` then the index
/// (`indexed_id("drop:", 3)` is `drop:3`), read back by [`indexed`].
pub fn indexed_id(prefix: &str, index: usize) -> String {
    format!("{prefix}{index}")
}

/// The index after `prefix` in an id [`indexed_id`] made.
pub fn indexed(id: &str, prefix: &str) -> Option<usize> {
    id.strip_prefix(prefix)?.parse().ok()
}

/// Ids for a panel's widgets taken from their names, each unique in the
/// panel: a name's first widget has the name, the next the name with "#2"
/// after it, and so on.
/// A panel declared and then walked again for an event gives each widget
/// the same id both times.
#[derive(Debug, Clone, Default)]
pub struct Ids {
    taken: std::collections::HashMap<String, usize>,
}

impl Ids {
    /// The id for the next widget named `name`.
    pub fn next(&mut self, name: &str) -> String {
        let seen = self.taken.entry(name.to_string()).or_default();
        *seen += 1;
        match *seen {
            1 => name.to_string(),
            n => format!("{name}#{n}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_stay_apart_and_indices_read_back() {
        let mut ids = Ids::default();
        assert_eq!(
            [ids.next("Size"), ids.next("Size"), ids.next("Turn")],
            ["Size", "Size#2", "Turn"]
        );
        assert_eq!(indexed(&indexed_id("drop:", 3), "drop:"), Some(3));
        assert_eq!(indexed("drop:x", "drop:"), None);
        assert_eq!(srgb([1.2, 0.5, -1.0]), [255, 128, 0]);
    }
}
