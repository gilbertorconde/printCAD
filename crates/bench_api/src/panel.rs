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

/// A card over the view beside world point `at`, holding `widgets`,
/// field `focus` taking the keys as it opens (`Frame::card`).
pub fn floating_card(
    id: &str,
    title: &str,
    at: [f32; 3],
    focus: Option<&str>,
    widgets: impl IntoIterator<Item = Widget>,
) -> FloatingCard {
    FloatingCard {
        id: id.into(),
        title: title.into(),
        at: Some(at),
        focus: focus.map(String::from),
        widgets: widgets.into_iter().collect(),
        ..Default::default()
    }
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

/// One walk over a declared panel, written once for both of its ends:
/// without an event it lists the widgets; with one, the binder whose
/// widget the event names applies it to the value it binds. Each binder
/// names its widget through [`Ids`], so a walk that declares and a walk
/// that applies give a widget the same id, and the code that draws a
/// field is the code that says what a change to it does.
///
/// A binder returns its widget unplaced, showing the value after the event,
/// for the caller to put in a row, a flow or a hint ([`Form::put`] puts
/// it next in the list). Containers nest through [`Form::nest`].
///
/// ```ignore
/// let mut form = Form::new(event);
/// let size = form.number("Size", &mut part.size, quantity("", "Size", 0.0));
/// form.put(size);
/// let flip = form.check("Flip", &mut part.flip, check("", "", "Flip", false));
/// form.put(flip);
/// let widgets = form.finish();
/// ```
#[derive(Debug)]
pub struct Form<'e> {
    /// The widgets declared, a list per open container.
    lists: Vec<Vec<Widget>>,
    /// The event to apply; `None` once a widget took it.
    event: Option<&'e PanelEvent>,
    changed: bool,
    ids: Ids,
}

/// A widget a [`Form`] binder declared, showing the value as it is after
/// the event, and what the event did to it.
#[derive(Debug, Clone, PartialEq)]
pub struct Bound<W, R = bool> {
    pub widget: W,
    /// Whether the event set the value (or, for buttons, which was
    /// pressed).
    pub set: R,
}

impl<'e> Form<'e> {
    /// A walk applying `event`, or only declaring without one.
    pub fn new(event: Option<&'e PanelEvent>) -> Self {
        Self {
            lists: vec![Vec::new()],
            event,
            changed: false,
            ids: Ids::default(),
        }
    }

    /// The walk that lists the widgets.
    pub fn declare() -> Self {
        Self::new(None)
    }

    /// The walk that hands `event` to its widget.
    pub fn apply(event: &'e PanelEvent) -> Self {
        Self::new(Some(event))
    }

    /// The id for the next widget named `name`, unique in the panel.
    pub fn id(&mut self, name: &str) -> String {
        self.ids.next(name)
    }

    /// `widget` next in the open container.
    pub fn push(&mut self, widget: impl Into<Widget>) {
        if let Some(list) = self.lists.last_mut() {
            list.push(widget.into());
        }
    }

    /// A bound widget next in the open container; whether the event set
    /// it.
    pub fn put<W: Into<Widget>>(&mut self, bound: Bound<W>) -> bool {
        self.push(bound.widget);
        bound.set
    }

    /// The event, taken, when it names `id`; a later widget of the same
    /// id does not see it again.
    pub fn take(&mut self, id: &str) -> Option<&'e PanelEvent> {
        if self.event.is_some_and(|e| e.id() == id) {
            self.event.take()
        } else {
            None
        }
    }

    /// Whether the event is a click on the button `id`, taken if it is.
    pub fn pressed(&mut self, id: &str) -> bool {
        matches!(self.take(id), Some(PanelEvent::Button { .. }))
    }

    /// Whether the event changed a value the walk binds.
    pub fn changed(&self) -> bool {
        self.changed
    }

    /// Says the event changed a value, for a change made outside the
    /// binders (a row taken out of a list).
    pub fn mark_changed(&mut self) {
        self.changed = true;
    }

    /// Starts a container: what is declared until [`Form::close`] goes in
    /// it.
    pub fn open(&mut self) {
        self.lists.push(Vec::new());
    }

    /// Ends the container [`Form::open`] started; its children.
    pub fn close(&mut self) -> Vec<Widget> {
        if self.lists.len() > 1 {
            self.lists.pop().unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    /// What `body` declares, made one widget by `wrap`.
    pub fn nest<R>(
        &mut self,
        body: impl FnOnce(&mut Self) -> R,
        wrap: impl FnOnce(Vec<Widget>) -> Widget,
    ) -> R {
        self.open();
        let out = body(self);
        let children = self.close();
        self.push(wrap(children));
        out
    }

    /// The widgets declared at the top.
    pub fn finish(mut self) -> Vec<Widget> {
        self.lists.truncate(1);
        self.lists.pop().unwrap_or_default()
    }

    /// A number named `name`, as `shape` draws it, showing `value`;
    /// a number typed or dragged to sets it.
    pub fn number(&mut self, name: &str, value: &mut f64, mut shape: Quantity) -> Bound<Quantity> {
        shape.id = self.id(name);
        let set = match self.take(&shape.id) {
            Some(PanelEvent::Number { value: now, .. }) => {
                *value = *now;
                self.changed = true;
                true
            }
            _ => false,
        };
        shape.value = *value;
        Bound { widget: shape, set }
    }

    /// A check box named `name`, as `shape` draws it, showing `on`; a
    /// click sets it.
    pub fn check(&mut self, name: &str, on: &mut bool, mut shape: Check) -> Bound<Check> {
        shape.id = self.id(name);
        let set = match self.take(&shape.id) {
            Some(PanelEvent::Toggle { on: now, .. }) => {
                *on = *now;
                self.changed = true;
                true
            }
            _ => false,
        };
        shape.on = *on;
        Bound { widget: shape, set }
    }

    /// A choice named `name` among `options`, each a value and its words,
    /// showing `value`, or `shape.selected` when no option is `value`.
    /// Picking an option sets `value`; `set` says whether it changed.
    pub fn choose<T: PartialEq + Clone, S: AsRef<str>>(
        &mut self,
        name: &str,
        value: &mut T,
        options: &[(T, S)],
        mut shape: Select,
    ) -> Bound<Select> {
        shape.id = self.id(name);
        let chosen = match self.take(&shape.id) {
            Some(PanelEvent::Choice { index, .. }) => options.get(*index),
            _ => None,
        };
        let set = chosen.is_some_and(|(v, _)| v != value);
        if let Some((v, _)) = chosen {
            *value = v.clone();
        }
        self.changed |= set;
        shape.options = options
            .iter()
            .map(|(_, o)| o.as_ref().to_string())
            .collect();
        if let Some(at) = options.iter().position(|(v, _)| v == value) {
            shape.selected = at;
        }
        Bound { widget: shape, set }
    }

    /// A choice named `name` among the options `shape` holds, showing
    /// option `selected`; picking one sets it, the same one again too.
    pub fn index(&mut self, name: &str, selected: &mut usize, mut shape: Select) -> Bound<Select> {
        shape.id = self.id(name);
        let set = match self.take(&shape.id) {
            Some(PanelEvent::Choice { index, .. }) => {
                *selected = *index;
                self.changed = true;
                true
            }
            _ => false,
        };
        shape.selected = *selected;
        Bound { widget: shape, set }
    }

    /// A button named `name`; `set` when it was pressed.
    pub fn button(
        &mut self,
        name: &str,
        label: &str,
        style: ButtonStyle,
        enabled: bool,
    ) -> Bound<Widget> {
        let id = self.id(name);
        let set = self.pressed(&id);
        Bound {
            widget: button(&id, label, style, enabled),
            set,
        }
    }

    /// A small icon button named `name`, as `shape` draws it; `set` when
    /// it was pressed.
    pub fn action(&mut self, name: &str, mut shape: PanelAction) -> Bound<PanelAction> {
        shape.id = self.id(name);
        let set = self.pressed(&shape.id);
        Bound { widget: shape, set }
    }

    /// A row of icon buttons named `name` under `label`, each its own id
    /// (the row's, a colon and its place); which was pressed.
    pub fn toggles(
        &mut self,
        name: &str,
        label: &str,
        mut buttons: Vec<ToggleButton>,
    ) -> Bound<Toggles, Option<usize>> {
        let id = self.id(name);
        for (n, b) in buttons.iter_mut().enumerate() {
            b.id = format!("{id}:{n}");
        }
        let set = (0..buttons.len()).find(|n| self.pressed(&buttons[*n].id));
        Bound {
            widget: Toggles {
                id,
                label: label.into(),
                buttons,
            },
            set,
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

    /// Every binder answers, in the walk that applies, to exactly the id
    /// it declared: an event built from the declared id sets it, and the
    /// same event aimed at the widget of the same name before it does not.
    #[test]
    fn every_binder_answers_to_the_id_it_declares() {
        type Walk = fn(&mut Form) -> bool;
        type Event = fn(String) -> PanelEvent;
        fn ids(widgets: &[Widget]) -> Vec<String> {
            let mut out = Vec::new();
            for w in widgets {
                w.walk(&mut |w| match w {
                    Widget::Toggles(t) => out.extend(t.buttons.iter().map(|b| b.id.clone())),
                    w => out.extend(w.id().map(String::from)),
                });
            }
            out
        }
        let cases: [(&str, Walk, Event); 7] = [
            (
                "number",
                |f| {
                    let b = f.number("Size", &mut 1.0, Quantity::default());
                    f.put(b)
                },
                |id| PanelEvent::Number { id, value: 2.0 },
            ),
            (
                "check",
                |f| {
                    let b = f.check("Flip", &mut false, Check::default());
                    f.put(b)
                },
                |id| PanelEvent::Toggle { id, on: true },
            ),
            (
                "choose",
                |f| {
                    let b = f.choose(
                        "Way",
                        &mut 'a',
                        &[('a', "A"), ('b', "B")],
                        Select::default(),
                    );
                    f.put(b)
                },
                |id| PanelEvent::Choice { id, index: 1 },
            ),
            (
                "index",
                |f| {
                    let b = f.index("Kind", &mut 0, Select::default());
                    f.put(b)
                },
                |id| PanelEvent::Choice { id, index: 1 },
            ),
            (
                "button",
                |f| {
                    let b = f.button("Go", "Go", ButtonStyle::Small, true);
                    f.put(b)
                },
                |id| PanelEvent::Button { id },
            ),
            (
                "action",
                |f| {
                    let b = f.action("Out", PanelAction::default());
                    f.put(b)
                },
                |id| PanelEvent::Button { id },
            ),
            (
                "toggles",
                |f| {
                    let b = f.toggles("Align", "", vec![ToggleButton::default(); 2]);
                    let set = b.set == Some(1);
                    f.push(b.widget);
                    set
                },
                |id| PanelEvent::Button { id },
            ),
        ];
        for (name, walk, event) in cases {
            // Two widgets of one name: the event names the second.
            let mut declare = Form::declare();
            assert!(!walk(&mut declare) && !walk(&mut declare), "{name}");
            let declared = ids(&declare.finish());
            let last = declared.last().cloned().unwrap_or_default();
            assert!(last.contains("#2"), "{name}: {declared:?}");
            let event = event(last);
            let mut apply = Form::apply(&event);
            assert_eq!(
                (walk(&mut apply), walk(&mut apply)),
                (false, true),
                "{name}"
            );
            assert_eq!(ids(&apply.finish()), declared, "{name}");
        }
    }
}
