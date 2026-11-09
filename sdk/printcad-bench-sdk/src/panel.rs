//! Builders for a declared panel's blocks: sections with counts and
//! summaries, item lists with values, tags and badges, a filter, a status
//! pill, and the layout around them. Each says what a part of the panel
//! is; printCAD draws it as its own panels draw theirs.
//!
//! The blocks came in contract 0.1.4: a package using them names
//! `min_contract = "0.1.4"` in its `bench.toml`, so an earlier printCAD
//! refuses it rather than draw part of its panel.
//!
//! ```ignore
//! use printcad_bench_sdk::{api::*, panel};
//!
//! let parts = panel::section("Parts")
//!     .count(2)
//!     .child(panel::filter("find").kinds(["All", "Bolts", "Nuts"]))
//!     .child(
//!         panel::items("parts")
//!             .filter("find")
//!             .item(panel::item("M4 bolt").icon("hole").value("×4").kind("Bolts"))
//!             .item(panel::item("M4 nut").icon("hexagon").value("×4").kind("Nuts")),
//!     );
//! let widgets: Vec<Widget> = vec![parts.into()];
//! ```

use crate::api::*;

/// A folding [`PanelSection`], open at first.
pub fn section(title: &str) -> SectionBuilder {
    SectionBuilder(PanelSection {
        title: title.into(),
        ..Default::default()
    })
}

pub struct SectionBuilder(pub PanelSection);

impl SectionBuilder {
    /// Keeps the fold's state apart from another section of its title.
    pub fn id(mut self, id: &str) -> Self {
        self.0.id = id.into();
        self
    }
    pub fn count(mut self, count: usize) -> Self {
        self.0.count = Some(count);
        self
    }
    /// One line of what it holds, shown while it is closed.
    pub fn summary(mut self, summary: &str) -> Self {
        self.0.summary = Some(summary.into());
        self
    }
    /// Closed the first time it is shown.
    pub fn closed(mut self) -> Self {
        self.0.open = false;
        self
    }
    /// An icon button in its title, told as `PanelEvent::Button { id }`.
    pub fn action(mut self, id: &str, icon: &str, hint: &str) -> Self {
        self.0.actions.push(PanelAction {
            id: id.into(),
            icon: Some(icon.into()),
            label: hint.into(),
            ..Default::default()
        });
        self
    }
    pub fn child(mut self, child: impl Into<Widget>) -> Self {
        self.0.children.push(child.into());
        self
    }
    pub fn children(mut self, children: impl IntoIterator<Item = Widget>) -> Self {
        self.0.children.extend(children);
        self
    }
}

impl From<SectionBuilder> for Widget {
    fn from(b: SectionBuilder) -> Self {
        b.0.into()
    }
}

/// An [`Item`]: a row of a list, or alone with [`ItemBuilder::id`].
pub fn item(label: &str) -> ItemBuilder {
    ItemBuilder(Item {
        label: label.into(),
        ..Default::default()
    })
}

pub struct ItemBuilder(pub Item);

impl ItemBuilder {
    /// Names an item standing alone in its events.
    pub fn id(mut self, id: &str) -> Self {
        self.0.id = id.into();
        self
    }
    pub fn icon(mut self, icon: &str) -> Self {
        self.0.icon = Some(icon.into());
        self
    }
    /// The icon's tone.
    pub fn tone(mut self, tone: Tone) -> Self {
        self.0.tone = tone;
        self
    }
    /// Out of play: its name dim.
    pub fn muted(mut self, muted: bool) -> Self {
        self.0.muted = muted;
        self
    }
    /// A problem: its name in the danger colour.
    pub fn problem(mut self, problem: bool) -> Self {
        self.0.problem = problem;
        self
    }
    /// Dim words after the name.
    pub fn secondary(mut self, text: &str) -> Self {
        self.0.secondary = Some(text.into());
        self
    }
    /// At the row's end, in the mono face.
    pub fn value(mut self, value: &str) -> Self {
        self.0.value = Some(value.into());
        self
    }
    /// At the row's end, in the plain face.
    pub fn plain_value(mut self, value: &str) -> Self {
        self.0.value = Some(value.into());
        self.0.value_plain = true;
        self
    }
    pub fn tag(self, text: &str) -> Self {
        self.tag_with(Tag {
            text: text.into(),
            ..Default::default()
        })
    }
    pub fn tag_with(mut self, tag: Tag) -> Self {
        self.0.tags.push(tag);
        self
    }
    pub fn badge(self, icon: &str, tone: Tone, hint: &str) -> Self {
        self.badge_with(Badge {
            icon: icon.into(),
            tone,
            hint: hint.into(),
            ..Default::default()
        })
    }
    /// A badge lit or in a colour of its own; a click on it is told as
    /// `PanelEvent::Badge`.
    pub fn badge_with(mut self, badge: Badge) -> Self {
        self.0.badges.push(badge);
        self
    }
    pub fn selected(mut self, selected: bool) -> Self {
        self.0.selected = selected;
        self
    }
    /// What a filter's kind selector matches.
    pub fn kind(mut self, kind: &str) -> Self {
        self.0.kind = Some(kind.into());
        self
    }
    pub fn hint(mut self, hint: &str) -> Self {
        self.0.hint = Some(hint.into());
        self
    }
    /// An entry of its right-click menu, told as
    /// `PanelEvent::ContextMenu { action: id, .. }`.
    pub fn menu(self, id: &str, label: &str) -> Self {
        self.menu_with(PanelAction {
            id: id.into(),
            label: label.into(),
            ..Default::default()
        })
    }
    /// A menu entry with words on hover, or set apart by a line.
    pub fn menu_with(mut self, action: PanelAction) -> Self {
        self.0.menu.push(action);
        self
    }
    /// Tied to what is in hand: faintly tinted.
    pub fn related(mut self, related: bool) -> Self {
        self.0.related = related;
        self
    }
    /// What is in hand, hovered through something else: lit.
    pub fn lit(mut self, lit: bool) -> Self {
        self.0.lit = lit;
        self
    }
    /// The icon in this sRGB colour.
    pub fn color(mut self, rgb: [u8; 3]) -> Self {
        self.0.color = Some(rgb);
        self
    }
    /// The value's tone.
    pub fn value_tone(mut self, tone: Tone) -> Self {
        self.0.value_tone = tone;
        self
    }
    /// Words on hover over the value.
    pub fn value_hint(mut self, hint: &str) -> Self {
        self.0.value_hint = Some(hint.into());
        self
    }
    /// A word after the value in its tone, `detail` on hover.
    pub fn flag(mut self, flag: StatusLine) -> Self {
        self.0.flag = Some(flag);
        self
    }
    /// Scrolled into view.
    pub fn reveal(mut self, reveal: bool) -> Self {
        self.0.reveal = reveal;
        self
    }
    /// Its name as a text field, told as `PanelEvent::ItemText`.
    pub fn editing(mut self, editing: bool) -> Self {
        self.0.editing = editing;
        self
    }
    /// A control at the row's end in place of its value.
    pub fn field(mut self, field: impl Into<Widget>) -> Self {
        self.0.field = Some(Box::new(field.into()));
        self
    }
}

impl From<ItemBuilder> for Item {
    fn from(b: ItemBuilder) -> Self {
        b.0
    }
}

impl From<ItemBuilder> for Widget {
    fn from(b: ItemBuilder) -> Self {
        b.0.into()
    }
}

/// An [`ItemList`].
pub fn items(id: &str) -> ItemsBuilder {
    ItemsBuilder(ItemList {
        id: id.into(),
        ..Default::default()
    })
}

pub struct ItemsBuilder(pub ItemList);

impl ItemsBuilder {
    pub fn item(mut self, item: impl Into<Item>) -> Self {
        self.0.items.push(item.into());
        self
    }
    pub fn items(mut self, items: impl IntoIterator<Item = Item>) -> Self {
        self.0.items.extend(items);
        self
    }
    /// Ctrl adds to the selection, told whole as `PanelEvent::Selection`.
    pub fn multi(mut self) -> Self {
        self.0.multi = true;
        self
    }
    /// Narrowed by the filter of this id.
    pub fn filter(mut self, filter: &str) -> Self {
        self.0.filter = Some(filter.into());
        self
    }
    /// Shown when nothing is listed.
    pub fn empty(mut self, text: &str) -> Self {
        self.0.empty = Some(text.into());
        self
    }
}

impl From<ItemsBuilder> for Widget {
    fn from(b: ItemsBuilder) -> Self {
        b.0.into()
    }
}

/// A [`FilterField`] for the item lists naming `id`.
pub fn filter(id: &str) -> FilterBuilder {
    FilterBuilder(FilterField {
        id: id.into(),
        ..Default::default()
    })
}

pub struct FilterBuilder(pub FilterField);

impl FilterBuilder {
    pub fn hint(mut self, hint: &str) -> Self {
        self.0.hint = hint.into();
        self
    }
    /// The kind selector's choices; the first passes every item.
    pub fn kinds<'a>(mut self, kinds: impl IntoIterator<Item = &'a str>) -> Self {
        self.0.kinds = kinds.into_iter().map(String::from).collect();
        self
    }
    pub fn kind(mut self, kind: usize) -> Self {
        self.0.kind = kind;
        self
    }
    /// The bench filters, told each change as `PanelEvent::Filter`.
    pub fn by_bench(mut self) -> Self {
        self.0.by_bench = true;
        self
    }
}

impl From<FilterBuilder> for Widget {
    fn from(b: FilterBuilder) -> Self {
        b.0.into()
    }
}

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
        tight: false,
    }
    .into()
}

/// Widgets one under another, close together, as a task's lines of text
/// and fields sit.
pub fn tight_stack(children: impl IntoIterator<Item = Widget>) -> Widget {
    Stack {
        children: children.into_iter().collect(),
        tight: true,
    }
    .into()
}

/// Widgets side by side, wrapping when the panel is narrow.
pub fn flow(children: impl IntoIterator<Item = Widget>) -> Widget {
    Flow {
        children: children.into_iter().collect(),
        no_wrap: false,
    }
    .into()
}

/// Widgets side by side on one line.
pub fn flow_no_wrap(children: impl IntoIterator<Item = Widget>) -> Widget {
    Flow {
        children: children.into_iter().collect(),
        no_wrap: true,
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
