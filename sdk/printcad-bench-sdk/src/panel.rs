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
    pub fn badge(mut self, icon: &str, tone: Tone, hint: &str) -> Self {
        self.0.badges.push(Badge {
            icon: icon.into(),
            tone,
            hint: hint.into(),
        });
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
    pub fn menu(mut self, id: &str, label: &str) -> Self {
        self.0.menu.push(PanelAction {
            id: id.into(),
            icon: None,
            label: label.into(),
        });
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
    Entry {
        id: id.into(),
        label: label.into(),
        value: value.into(),
        placeholder: placeholder.into(),
    }
    .into()
}
