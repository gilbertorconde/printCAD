//! The declared panels a printCAD workbench and the app exchange: the
//! [`Widget`]s a panel or a card over the view is made of, the
//! [`PanelEvent`]s the host sends back when the user changes one, and the
//! [`FloatingCard`] that holds a quick edit beside what it edits.
//!
//! Packages reach these through `printcad-bench-api`, which re-exports
//! every item here at its own root; the document crate takes them from
//! here, beneath the package contract. The contract's version rules
//! (`printcad-bench-api` › Versions) hold for these types too.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dim {
    #[default]
    Length,
    Angle,
    Number,
}

/// A card over the view, next to what it edits: a quick edit of a value
/// or two (a dimension, a corner's size). Anything with more to it is a
/// task in the right panel. The host draws `widgets` as it draws a task
/// panel's, with OK and Cancel under them; Enter is OK and Escape Cancel.
/// OK and Cancel reach the bench as [`PanelEvent::CardClose`]. Edits wait
/// in the bench until OK: the host makes what OK writes one undo step,
/// and takes back anything written while the card was open on Cancel.
/// A click in the view elsewhere is OK, while OK is allowed.
///
/// With a `hint` the card is a readout instead: the values a tool is
/// drawing with, typed through the bench's own keys. The host draws the
/// hint in place of OK and Cancel, leaves the keys and the pointer with
/// the bench, holds no undo step and sends no `CardClose`. One card shows
/// at a time.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FloatingCard {
    /// Names the card: one with a new id opens afresh, the field named by
    /// `focus` taking the keys again.
    pub id: String,
    pub title: String,
    pub icon: Option<String>,
    /// The point it is beside, in world space.
    pub at: Option<[f32; 3]>,
    /// The point it is beside in viewport pixels, when it has no `at`;
    /// left out of both, the view's middle.
    pub screen: Option<[f32; 2]>,
    pub widgets: Vec<Widget>,
    /// The id of the text or number field that has the keys as the card
    /// opens, its text selected so the first key typed replaces it.
    pub focus: Option<String>,
    /// OK's words; `OK` when empty.
    pub ok: String,
    /// Why OK is held back: written under the widgets, OK disabled and
    /// Enter and a click elsewhere doing nothing.
    pub refusal: Option<String>,
    /// The line under the widgets of a readout card, in place of OK and
    /// Cancel: what the keys do (`Tab next · Enter places it`).
    pub hint: Option<String>,
    /// OK alone, no Cancel: Escape is OK too, and only a refused card is
    /// dropped. For what a step already made leaves to set.
    pub ok_only: bool,
}

// ------------------------------------------------------------------ panel

/// One element of a declared panel. The host draws it with the app's
/// widgets and sends back a [`PanelEvent`] when the user changes it.
///
/// A package builds these, so a variant keeps its fields for as long as
/// the contract's minor version holds: a new option comes as a new
/// variant, or as a new struct that derives `Default`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Widget {
    Heading {
        text: String,
    },
    Text {
        text: String,
        #[serde(default)]
        mono: bool,
    },
    Note {
        #[serde(default)]
        kind: NoteKind,
        #[serde(default)]
        title: Option<String>,
        text: String,
    },
    /// A number. Bound to a parameter of a feature (`bind`), it takes
    /// formulas: the host keeps the formula, and the bench is told the
    /// value.
    Number {
        id: String,
        label: String,
        value: f64,
        #[serde(default)]
        dim: Dim,
        #[serde(default)]
        bind: Option<Bind>,
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
        #[serde(default = "two")]
        decimals: usize,
        #[serde(default)]
        error: Option<String>,
    },
    Choice {
        id: String,
        label: String,
        options: Vec<String>,
        selected: usize,
    },
    Toggle {
        id: String,
        label: String,
        on: bool,
    },
    TextField {
        id: String,
        label: String,
        value: String,
    },
    Button {
        id: String,
        label: String,
        #[serde(default)]
        style: ButtonStyle,
        #[serde(default = "yes")]
        enabled: bool,
    },
    /// A row asking for something to be clicked in the viewport. A click
    /// on it arms it (`PanelEvent::Pick`); the bench then reads the picks
    /// from the events that follow.
    Pick {
        id: String,
        label: String,
        /// What has been picked, in words.
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        armed: bool,
    },
    /// Selectable rows.
    List {
        id: String,
        items: Vec<ListItem>,
        #[serde(default)]
        selected: Option<usize>,
    },
    /// Columns of text, one row selectable. A column marked in
    /// `editable` takes typing in its cells (`PanelEvent::Cell`): a tool
    /// table, a list of settings by row.
    Table {
        id: String,
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
        #[serde(default)]
        selected: Option<usize>,
        /// One per column; a column left out is read-only.
        #[serde(default)]
        editable: Vec<bool>,
    },
    Group {
        title: String,
        #[serde(default = "yes")]
        open: bool,
        children: Vec<Widget>,
    },
    Progress {
        label: String,
        /// `None` while the amount is unknown.
        #[serde(default)]
        fraction: Option<f32>,
        /// A job of the bench's: the host shows how far it is.
        #[serde(default)]
        job: Option<u64>,
    },
    Separator,
    /// A drawing: a schematic of what the panel edits, with its measures
    /// marked beside the parts they size. It is drawn in a space of its
    /// own, `[0, width] × [0, height]` with y up, which the host fits to
    /// the panel's width; strokes and text keep their pixel size. A press
    /// on it, and every move of a drag begun on it, comes back as
    /// `PanelEvent::DiagramPress` in those coordinates: a timeline seeks
    /// where it is pressed.
    Diagram {
        id: String,
        width: f32,
        height: f32,
        #[serde(default)]
        shapes: Vec<DiagramShape>,
        #[serde(default)]
        dimensions: Vec<Dimension>,
        #[serde(default)]
        callouts: Vec<Callout>,
    },
    /// A task panel's title with an icon, on a raised band.
    Header {
        icon: String,
        title: String,
    },
    /// A label and a value beside it, read only.
    Value {
        label: String,
        value: String,
        #[serde(default)]
        mono: bool,
    },
    /// Widgets side by side, going on to the next line when the panel is
    /// narrow. A field with an empty label takes no label column here.
    Row {
        children: Vec<Widget>,
    },
    /// A widget with words shown while the pointer rests on it.
    Hinted {
        hint: String,
        widget: Box<Widget>,
    },
    /// A widget with words and a figure under them (a [`Widget::Diagram`])
    /// shown while the pointer rests on it.
    HintedFigure {
        hint: String,
        figure: Box<Widget>,
        widget: Box<Widget>,
    },
    /// A number dragged along a range, for a spread or a place in time
    /// rather than a measure; it comes back as `PanelEvent::Number`.
    Slider {
        id: String,
        label: String,
        value: f64,
        min: f64,
        max: f64,
        /// The values it stops at, every `step` from `min`.
        #[serde(default)]
        step: Option<f64>,
        #[serde(default = "two")]
        decimals: usize,
        /// The value written beside it.
        #[serde(default = "yes")]
        show_value: bool,
    },
    /// Rows of cells under column titles, some cells editable; it scrolls
    /// sideways when wider than the panel.
    Sheet {
        id: String,
        columns: Vec<String>,
        rows: Vec<Vec<Cell>>,
    },
    /// A folding section with a count and a summary while closed.
    Section(PanelSection),
    /// One item: an icon, a name, a value at the end, tags and badges.
    Item(Item),
    /// Items, selectable, narrowed by a [`Widget::Filter`].
    Items(ItemList),
    /// A filter field for item lists, with a kind selector.
    Filter(FilterField),
    /// A state in a few words, as a toned pill.
    Status(StatusLine),
    /// Widgets one under another.
    Stack(Stack),
    /// Widgets side by side.
    Flow(Flow),
    /// A widget taking the rest of a [`Widget::Flow`]'s width.
    Fill(Fill),
    /// Fields sharing one label column of their own.
    Fields(Fields),
    /// A field with its label beside it at its own width, no column.
    Inline(Inline),
    /// What is picked in the view, as a chip that clears.
    PickChip(PickChip),
    /// A line of text the user types, with a placeholder while empty.
    Entry(Entry),
    /// What the task panel's header shows under and beside the title.
    TaskHeader(TaskHeader),
    /// A number field with its drag speed, width and state, and as a
    /// stepper.
    Quantity(Quantity),
    /// A labelled check box.
    Check(Check),
    /// A choice of a few, with its width and state.
    Select(Select),
    /// Fields in a two-column grid, labels as wide as the longest.
    Grid(FieldGrid),
    /// A note the user can click.
    Alert(Alert),
    /// A raised card holding widgets, under an icon and a title.
    Card(Card),
    /// A line of words in a size and a tone.
    Caption(Caption),
    /// Room between widgets.
    Gap(Gap),
    /// Lines plotted along a strip.
    Plot(Plot),
    /// Colours to click, the chosen ones ringed.
    Swatches(Swatches),
    /// A colour, edited in the system's picker.
    Color(ColorField),
    /// A number dragged along a range, written with a suffix of its own.
    Range(Range),
    /// A frameless button as wide as the panel: one of a list of places.
    RowButton(RowButton),
    /// Words that light up while chosen, as one of a few side by side.
    Selectable(Selectable),
    /// Pictures to choose among, each over its name.
    Gallery(Gallery),
    /// A titled, bordered group of rows, as the Preferences pages draw
    /// theirs.
    Settings(SettingsGroup),
    /// Widgets in rows of a few columns, each column as wide as its
    /// widest: a table of readings.
    Cells(Cells),
    /// A small icon button on its own, its words on hover.
    Action(PanelAction),
    /// A row of icon buttons under one label, each lit while on: an
    /// editor's alignment buttons.
    Toggles(Toggles),
}

/// The widgets a widget holds, in the order they are drawn: one match for
/// [`Widget::children`] and [`Widget::children_mut`].
macro_rules! widget_children {
    ($widget:expr, $deref:ident, $iter:ident $(, $mut:ident)?) => {{
        let mut out = Vec::new();
        match $widget {
            Widget::Group { children, .. }
            | Widget::Row { children }
            | Widget::Section(PanelSection { children, .. })
            | Widget::Stack(Stack { children, .. })
            | Widget::Flow(Flow { children, .. })
            | Widget::Fields(Fields { children, .. })
            | Widget::Grid(FieldGrid { children, .. })
            | Widget::Card(Card { children, .. })
            | Widget::Cells(Cells { children, .. })
            | Widget::Settings(SettingsGroup { children, .. }) => out.extend(children),
            Widget::Hinted { widget, .. } => out.push(&$($mut)? **widget),
            Widget::HintedFigure { figure, widget, .. } => {
                out.push(&$($mut)? **figure);
                out.push(&$($mut)? **widget);
            }
            Widget::Fill(Fill { child }) | Widget::Inline(Inline { child }) => {
                out.push(&$($mut)? **child)
            }
            Widget::Item(item) => out.extend(item.field.$deref()),
            Widget::Items(list) => {
                for item in list.items.$iter() {
                    out.extend(item.field.$deref());
                }
            }
            Widget::Heading { .. }
            | Widget::Text { .. }
            | Widget::Note { .. }
            | Widget::Number { .. }
            | Widget::Choice { .. }
            | Widget::Toggle { .. }
            | Widget::TextField { .. }
            | Widget::Button { .. }
            | Widget::Pick { .. }
            | Widget::List { .. }
            | Widget::Table { .. }
            | Widget::Progress { .. }
            | Widget::Separator
            | Widget::Diagram { .. }
            | Widget::Header { .. }
            | Widget::Value { .. }
            | Widget::Slider { .. }
            | Widget::Sheet { .. }
            | Widget::Filter(_)
            | Widget::Status(_)
            | Widget::PickChip(_)
            | Widget::Entry(_)
            | Widget::TaskHeader(_)
            | Widget::Quantity(_)
            | Widget::Check(_)
            | Widget::Select(_)
            | Widget::Alert(_)
            | Widget::Caption(_)
            | Widget::Gap(_)
            | Widget::Plot(_)
            | Widget::Swatches(_)
            | Widget::Color(_)
            | Widget::Range(_)
            | Widget::RowButton(_)
            | Widget::Selectable(_)
            | Widget::Gallery(_)
            | Widget::Action(_)
            | Widget::Toggles(_) => {}
        }
        out
    }};
}

impl Widget {
    /// The widgets this one holds, in the order they are drawn: a
    /// container's children, a hinted widget (and its figure), an item's
    /// field. Empty for a widget that holds none.
    pub fn children(&self) -> Vec<&Widget> {
        widget_children!(self, as_deref, iter)
    }

    /// [`Widget::children`], to change in place.
    pub fn children_mut(&mut self) -> Vec<&mut Widget> {
        widget_children!(self, as_deref_mut, iter_mut, mut)
    }

    /// This widget and every widget it holds, at any depth, each before
    /// its children.
    pub fn walk<'a>(&'a self, visit: &mut impl FnMut(&'a Widget)) {
        visit(self);
        for child in self.children() {
            child.walk(visit);
        }
    }

    /// [`Widget::walk`], to change in place.
    pub fn walk_mut(&mut self, visit: &mut impl FnMut(&mut Widget)) {
        visit(self);
        for child in self.children_mut() {
            child.walk_mut(visit);
        }
    }

    /// The id its events carry, for a widget that has one.
    pub fn id(&self) -> Option<&str> {
        let id = match self {
            Widget::Number { id, .. }
            | Widget::Choice { id, .. }
            | Widget::Toggle { id, .. }
            | Widget::TextField { id, .. }
            | Widget::Button { id, .. }
            | Widget::Pick { id, .. }
            | Widget::List { id, .. }
            | Widget::Table { id, .. }
            | Widget::Diagram { id, .. }
            | Widget::Slider { id, .. }
            | Widget::Sheet { id, .. }
            | Widget::Section(PanelSection { id, .. })
            | Widget::Item(Item { id, .. })
            | Widget::Items(ItemList { id, .. })
            | Widget::Filter(FilterField { id, .. })
            | Widget::PickChip(PickChip { id, .. })
            | Widget::Entry(Entry { id, .. })
            | Widget::Quantity(Quantity { id, .. })
            | Widget::Check(Check { id, .. })
            | Widget::Select(Select { id, .. })
            | Widget::Swatches(Swatches { id, .. })
            | Widget::Color(ColorField { id, .. })
            | Widget::Range(Range { id, .. })
            | Widget::RowButton(RowButton { id, .. })
            | Widget::Selectable(Selectable { id, .. })
            | Widget::Gallery(Gallery { id, .. })
            | Widget::Action(PanelAction { id, .. })
            | Widget::Toggles(Toggles { id, .. }) => id,
            Widget::Alert(Alert { id, .. }) => id.as_ref()?,
            _ => return None,
        };
        (!id.is_empty()).then_some(id.as_str())
    }

    /// The label a field writes in its label column, for a field that has
    /// one: a check box's or a button's own words are not one.
    pub fn field_label(&self) -> Option<&str> {
        let label = match self {
            Widget::Number { label, .. }
            | Widget::Choice { label, .. }
            | Widget::TextField { label, .. }
            | Widget::Pick { label, .. }
            | Widget::Value { label, .. }
            | Widget::Slider { label, .. }
            | Widget::PickChip(PickChip { label, .. })
            | Widget::Entry(Entry { label, .. })
            | Widget::Quantity(Quantity { label, .. })
            | Widget::Check(Check { label, .. })
            | Widget::Select(Select { label, .. })
            | Widget::Range(Range { label, .. })
            | Widget::Color(ColorField { label, .. })
            | Widget::Toggles(Toggles { label, .. })
            | Widget::Flow(Flow { label, .. }) => label,
            _ => return None,
        };
        (!label.is_empty()).then_some(label.as_str())
    }
}

/// `widgets` keeping only those `keep` holds to, at any depth: a widget
/// left out goes with what it holds. A widget held alone (a hint's, a
/// fill's) stays whatever `keep` says of it.
pub fn retain_widgets(widgets: &mut Vec<Widget>, keep: &mut impl FnMut(&Widget) -> bool) {
    widgets.retain(|w| keep(w));
    for widget in widgets {
        retain_within(widget, keep);
    }
}

fn retain_within(widget: &mut Widget, keep: &mut impl FnMut(&Widget) -> bool) {
    match widget {
        Widget::Group { children, .. }
        | Widget::Row { children }
        | Widget::Section(PanelSection { children, .. })
        | Widget::Stack(Stack { children, .. })
        | Widget::Flow(Flow { children, .. })
        | Widget::Fields(Fields { children, .. })
        | Widget::Grid(FieldGrid { children, .. })
        | Widget::Card(Card { children, .. })
        | Widget::Cells(Cells { children, .. })
        | Widget::Settings(SettingsGroup { children, .. }) => retain_widgets(children, keep),
        other => {
            for child in other.children_mut() {
                retain_within(child, keep);
            }
        }
    }
}

/// Every widget of `widgets`, at any depth, each before its children.
pub fn walk_widgets(widgets: &[Widget]) -> Vec<&Widget> {
    let mut all = Vec::new();
    for widget in widgets {
        widget.walk(&mut |w| all.push(w));
    }
    all
}

// ----------------------------------------------------------- panel blocks
//
// The blocks say what a part of a panel is; the host draws each with the
// controls its own panels use. They came in contract 0.1.4: a package
// using them names `min_contract = "0.1.4"` in its manifest.

/// How a mark, a tag or a status reads: what it means, not its colour.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    #[default]
    Neutral,
    Good,
    Warning,
    Bad,
    Info,
    /// The accent: what is in hand.
    Accent,
    /// Dim: what is out of play or says little.
    Muted,
}

/// A small icon button: in a section's title, on its own
/// ([`Widget::Action`]: a cross at a row's end that takes it out), or an
/// entry of an item's menu (`label` then names it). A click comes back as
/// `PanelEvent::Button` (a section's or one on its own) or
/// `PanelEvent::ContextMenu` (a menu's) with `id`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PanelAction {
    pub id: String,
    #[serde(default)]
    pub icon: Option<String>,
    /// The words on hover, or the menu entry.
    #[serde(default)]
    pub label: String,
    /// A menu entry's words on hover.
    #[serde(default)]
    pub hint: String,
    /// A menu entry with a line above it, setting it apart.
    #[serde(default)]
    pub separated: bool,
}

/// A [`Widget::Toggles`]: icon buttons in a row under one label, each lit
/// while `on` and its words shown on hover, the way an editor shows its
/// alignment and wrapping. A click comes back as `PanelEvent::Button` with
/// the button's `id`; whether lighting one puts the others out is the
/// bench's to decide.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Toggles {
    pub id: String,
    pub label: String,
    pub buttons: Vec<ToggleButton>,
}

/// One button of [`Toggles`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToggleButton {
    pub id: String,
    /// An icon of the package (`icons/<name>.svg`) or of the design set.
    pub icon: String,
    /// The words on hover: what the button does.
    pub hint: String,
    pub on: bool,
    /// Shown but not to be clicked.
    pub disabled: bool,
}

/// A [`Widget::Section`]: a fold the user opens and closes, its title in
/// small capitals, the count of what it holds, a one-line summary while
/// closed, icon actions at its right end, and its children, whose fields
/// share one label column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelSection {
    /// Keeps the fold's state; the title when empty.
    pub id: String,
    pub title: String,
    pub count: Option<usize>,
    pub summary: Option<String>,
    /// Open the first time it is shown.
    pub open: bool,
    pub actions: Vec<PanelAction>,
    pub children: Vec<Widget>,
}

impl Default for PanelSection {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            count: None,
            summary: None,
            open: true,
            actions: Vec::new(),
            children: Vec::new(),
        }
    }
}

/// A small rounded name in an item's row: an element a constraint ties.
/// A click comes back as `PanelEvent::Tag`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tag {
    pub text: String,
    /// `Neutral` draws it plain.
    pub tone: Tone,
    /// What it names is selected.
    pub selected: bool,
    /// What it names is in hand or tied to it: outlined.
    pub lit: bool,
}

/// A glyph badge at an item's end: a constraint on an element. A click
/// comes back as `PanelEvent::Badge`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Badge {
    pub icon: String,
    pub tone: Tone,
    /// Words on hover.
    pub hint: String,
    /// What it stands for is in hand: drawn on a lit ground.
    pub lit: bool,
    /// Its colour as sRGB, in place of the tone's.
    pub color: Option<[u8; 3]>,
}

/// One row of a list: what an [`ItemList`] holds, or alone as a
/// [`Widget::Item`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    /// Names the row in events when it stands alone; in a list the row's
    /// place names it.
    pub id: String,
    pub icon: Option<String>,
    /// The icon's tone; `Neutral` is the row's plain one.
    pub tone: Tone,
    /// Out of play: its name dim.
    pub muted: bool,
    /// A problem: its name in the danger colour.
    pub problem: bool,
    pub label: String,
    /// Dim words after the name.
    pub secondary: Option<String>,
    /// At the row's end.
    pub value: Option<String>,
    /// The value in the plain face rather than the mono one.
    pub value_plain: bool,
    pub tags: Vec<Tag>,
    pub badges: Vec<Badge>,
    pub selected: bool,
    /// What a [`FilterField`]'s kind selector matches.
    pub kind: Option<String>,
    /// Words on hover.
    pub hint: Option<String>,
    /// The entries of its right-click menu.
    pub menu: Vec<PanelAction>,
    /// Tied to what is in hand (hovered or selected elsewhere): a faint
    /// tint and a thin bar at its left edge.
    pub related: bool,
    /// What is in hand itself, hovered through something else (a tag,
    /// the view): lit as under the pointer.
    pub lit: bool,
    /// The icon's colour as sRGB, in place of the tone's: the colour the
    /// view draws it in.
    pub color: Option<[u8; 3]>,
    /// The value's tone; `Neutral` is the row's plain one.
    pub value_tone: Tone,
    /// Words on hover over the value.
    pub value_hint: Option<String>,
    /// A word after the value in its tone, its `detail` on hover.
    pub flag: Option<StatusLine>,
    /// Scrolled into view.
    pub reveal: bool,
    /// Its name as a text field, focused: told as `PanelEvent::ItemText`
    /// at each change and once more as it is left.
    pub editing: bool,
    /// A control at the row's end in place of the value (a
    /// [`Widget::Quantity`] the row's value is typed into); its events are
    /// its own.
    pub field: Option<Box<Widget>>,
}

/// A [`Widget::Items`]: rows a click selects (`PanelEvent::Select`), or
/// with `multi` a click with Ctrl adds to (`PanelEvent::Selection`), a
/// double click opens (`PanelEvent::Activate`); the pointer over one is
/// told as `PanelEvent::Hover`, so the bench lights it in the view.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemList {
    pub id: String,
    pub items: Vec<Item>,
    pub multi: bool,
    /// The [`FilterField`] that narrows it, by its id.
    pub filter: Option<String>,
    /// Shown when nothing is listed, or nothing passes the filter.
    pub empty: Option<String>,
}

/// A [`Widget::Filter`]: a search field narrowing the item lists that
/// name it, by their label, secondary words and tags, and a kind selector
/// when `kinds` names any (the first, "All" say, passes every item; the
/// others pass items of that `kind`). The host filters as the user types;
/// with `by_bench` it leaves the lists whole and tells the bench
/// (`PanelEvent::Filter`) instead.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterField {
    pub id: String,
    /// In the field while it is empty.
    pub hint: String,
    pub kinds: Vec<String>,
    /// The kind chosen when first shown.
    pub kind: usize,
    pub by_bench: bool,
}

/// A [`Widget::Status`], and a task header's state ([`TaskHeader`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusLine {
    pub text: String,
    pub tone: Tone,
    /// The longer reason, on hover.
    pub detail: String,
}

/// A [`Widget::TaskHeader`]: in a task's panel, the header above it
/// shows `subtitle` under the title and `status` as a pill beside it,
/// as the app's own tasks do; anywhere else they draw in the panel.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskHeader {
    /// What the task edits or where it sits.
    pub subtitle: String,
    pub status: Option<StatusLine>,
}

/// A [`Widget::Stack`]: its children the panel's spacing apart, or
/// close together when `tight`, as a task's lines of text and fields
/// sit.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stack {
    pub children: Vec<Widget>,
    pub tight: bool,
    /// The room between its children in points, in place of the panel's.
    pub spacing: Option<f32>,
}

/// A [`Widget::Flow`]: side by side, going on to the next line when the
/// panel is narrow unless `no_wrap`. A [`Widget::Fill`] among them takes
/// what the others leave.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Flow {
    pub children: Vec<Widget>,
    pub no_wrap: bool,
    /// Written in its section's label column before the children, as a
    /// field's label is.
    pub label: String,
}

/// A [`Widget::Fill`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fill {
    pub child: Box<Widget>,
}

impl Default for Fill {
    fn default() -> Self {
        Self {
            child: Box::new(Widget::Separator),
        }
    }
}

/// A [`Widget::Fields`]: its fields' labels in one column as wide as the
/// longest, within the theme's limits, apart from the section's; or
/// `label_width` wide.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fields {
    pub children: Vec<Widget>,
    /// The column's width in points, whatever the labels.
    pub label_width: Option<f32>,
}

/// A [`Widget::Inline`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inline {
    pub child: Box<Widget>,
}

impl Default for Inline {
    fn default() -> Self {
        Self {
            child: Box::new(Widget::Separator),
        }
    }
}

/// A [`Widget::PickChip`]: a click arms it (`PanelEvent::Pick`), its
/// cross clears it (`PanelEvent::Clear`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PickChip {
    pub id: String,
    pub label: String,
    /// What has been picked, in words.
    pub value: Option<String>,
    pub armed: bool,
    /// In the chip while nothing is picked.
    pub placeholder: String,
}

/// A [`Widget::Entry`]: told once typing ends (`PanelEvent::Text`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub id: String,
    pub label: String,
    pub value: String,
    pub placeholder: String,
    /// Lines may break: Enter starts a new one.
    pub multiline: bool,
    /// Its width in points; the panel's field width when left out.
    pub width: Option<f32>,
    /// In the mono face.
    pub mono: bool,
    /// Told at each change rather than once typing ends.
    pub live: bool,
    /// Offers the document's names (variables, features and their
    /// properties) as a formula is typed, as the formula fields do.
    pub formulas: bool,
    /// In the panel's larger text size: the one text a task edits.
    pub large: bool,
}

/// A [`Widget::Quantity`]: a number as a [`Widget::Number`] says it, with
/// how it drags and how wide it is. Bound to a parameter (`bind`), it
/// takes formulas as a bound `Number` does. Changes come back as
/// `PanelEvent::Number`; a stepper's buttons as the value one up or down.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Quantity {
    pub id: String,
    pub label: String,
    pub value: f64,
    pub dim: Dim,
    pub bind: Option<Bind>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub decimals: usize,
    pub error: Option<String>,
    /// How much the value moves per point dragged; left out, as a bound
    /// `Number` drags.
    pub speed: Option<f64>,
    /// Its width in points.
    pub width: Option<f32>,
    pub enabled: bool,
    /// Takes the keyboard focus this frame, as a field just made does.
    pub focus: bool,
    /// Written after the value in place of the `dim`'s unit: `N·m`.
    pub unit: Option<String>,
    /// The value between a down and an up button, in place of a field.
    pub stepper: Option<Stepper>,
    /// The system's bare number drag in place of the panel's field: its
    /// `unit` written after the value as given, its decimals as the value
    /// needs (none when `decimals` is 0).
    pub bare: bool,
    /// The least and largest values a drag (or the wheel, or a stepper's
    /// buttons) sets, inside `min` and `max`: a drag stops there, where a
    /// typed value may go past. For a size that builds only up to a
    /// limit, so that only typing can ask for one that fails.
    pub drag_min: Option<f64>,
    pub drag_max: Option<f64>,
    /// The field's text as the package has it, written in place of the
    /// value and not edited there: a value typed through the bench's own
    /// keys, a formula with what it comes to. `focus` then rings the
    /// field as the one those keys type into.
    pub shown: Option<String>,
    /// A lock after the field, shut while the value is held (typed, it
    /// no longer follows the cursor). A click on it comes back as
    /// `PanelEvent::Toggle` with the field's id.
    pub held: Option<bool>,
}

impl Default for Quantity {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            value: 0.0,
            dim: Dim::default(),
            bind: None,
            min: None,
            max: None,
            decimals: 2,
            error: None,
            speed: None,
            width: None,
            enabled: true,
            focus: false,
            unit: None,
            stepper: None,
            bare: false,
            drag_min: None,
            drag_max: None,
            shown: None,
            held: None,
        }
    }
}

/// A [`Quantity`] drawn as a stepper: `text` (the value when empty)
/// between a down and an up button, each with its state and its words on
/// hover.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stepper {
    pub text: String,
    pub down: bool,
    pub up: bool,
    pub down_hint: String,
    pub up_hint: String,
}

/// A [`Widget::Check`]: `text` beside its box, in its section's label
/// column under `label` when that is not empty. Told as
/// `PanelEvent::Toggle`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Check {
    pub id: String,
    pub label: String,
    pub text: String,
    pub on: bool,
    pub enabled: bool,
    /// Words on hover.
    pub hint: String,
}

impl Default for Check {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            text: String::new(),
            on: false,
            enabled: true,
            hint: String::new(),
        }
    }
}

/// A [`Widget::Select`]: one of `options`, told as `PanelEvent::Choice`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Select {
    pub id: String,
    pub label: String,
    pub options: Vec<String>,
    pub selected: usize,
    /// Its width in points; the panel's field width when left out.
    pub width: Option<f32>,
    pub enabled: bool,
    /// Shown in the closed field in place of the option chosen: a count,
    /// or a prompt for a list that only adds (`selected` past the end
    /// then marks none).
    pub text: Option<String>,
    /// The options offered but not to be chosen now, by their place.
    pub disabled: Vec<usize>,
    /// Words on hover over each option, by its place; empty for none.
    pub hints: Vec<String>,
    /// The options with a line above them, setting them apart.
    pub separated: Vec<usize>,
    /// The options side by side in the row, the chosen one lit, in place
    /// of a dropdown.
    pub segmented: bool,
    /// A search field atop the open dropdown, narrowing a long list to
    /// the options that match what is typed.
    pub search: bool,
}

impl Default for Select {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            options: Vec::new(),
            selected: 0,
            width: None,
            enabled: true,
            text: None,
            disabled: Vec::new(),
            hints: Vec::new(),
            separated: Vec::new(),
            segmented: false,
            search: false,
        }
    }
}

/// A [`Widget::Grid`]: two columns, a field's label in the first and its
/// control in the second, the column as wide as the grid's longest label.
/// A widget with no label takes one cell, so two of them make a row.
/// Rows sit close, or a little apart when `roomy`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FieldGrid {
    /// Keeps it apart from another grid of the panel.
    pub id: String,
    pub children: Vec<Widget>,
    pub roomy: bool,
    /// The control column a little further from the labels.
    pub apart: bool,
}

/// A [`Widget::Alert`]: a note as [`Widget::Note`] draws it; with an
/// `id`, a click on it comes back as `PanelEvent::Button`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Alert {
    pub id: Option<String>,
    pub kind: NoteKind,
    pub title: Option<String>,
    pub text: String,
}

/// A [`Widget::Card`]: a raised card, its icon and title on the first
/// line, then its children.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Card {
    pub icon: Option<String>,
    pub title: String,
    /// The icon's size in points.
    pub icon_size: f32,
    pub children: Vec<Widget>,
    /// The room inside its edge in points, in place of the card's own.
    pub padding: Option<f32>,
}

impl Default for Card {
    fn default() -> Self {
        Self {
            icon: None,
            title: String::new(),
            icon_size: 18.0,
            children: Vec::new(),
            padding: None,
        }
    }
}

/// A [`Widget::Caption`]: words in the panel's text size, or the small
/// one, in a tone (`Neutral` is the panel's secondary text, `Muted`
/// dimmer still), or in the primary text colour.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Caption {
    pub text: String,
    pub tone: Tone,
    pub small: bool,
    pub mono: bool,
    /// A small heading over what follows, in small capitals.
    pub overline: bool,
    /// In the semibold face and the panel's primary text colour, over
    /// the tone: the name of what the widgets after it edit.
    pub strong: bool,
    /// In the medium face: a heading over the rows under it.
    pub medium: bool,
    /// In the primary text colour, in place of the tone's: a value read.
    pub primary: bool,
}

/// A [`Widget::Plot`]: lines along a strip `height` points tall, each
/// series scaled to its own largest value, all on one x range from zero.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Plot {
    pub height: f32,
    pub series: Vec<PlotSeries>,
}

/// A line of a [`Plot`]: its points as `[x, y]`, in its tone.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlotSeries {
    pub points: Vec<[f64; 2]>,
    pub tone: Tone,
}

/// A [`Widget::Cells`]: `children` in rows of `columns` cells, left to
/// right, each column as wide as its widest cell; rows sit close and
/// columns a little apart. Column titles as strong captions, then a
/// caption per reading, make a table that reads as the panel's text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cells {
    /// Keeps it apart from another of the panel.
    pub id: String,
    /// Cells a row; one when left at zero.
    pub columns: usize,
    pub children: Vec<Widget>,
}

/// How much room a [`Widget::Gap`] leaves.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapSize {
    #[default]
    Small,
    Medium,
    Large,
    /// The room before a part of a panel under its own heading, between
    /// `medium` and `large`.
    Section,
}

/// A [`Widget::Gap`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Gap {
    pub size: GapSize,
}

/// A colour of a [`Swatches`] block, as sRGB.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Swatch {
    pub color: [u8; 3],
    /// It is the colour in use: ringed in the accent.
    pub chosen: bool,
}

/// A [`Widget::Swatches`]: small squares of colour, side by side and
/// going on to the next line; a click is `PanelEvent::Choice` with the
/// swatch's place.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Swatches {
    pub id: String,
    pub swatches: Vec<Swatch>,
    /// Keys to the colours the view paints rather than colours to pick:
    /// smaller, unringed, and no click; one beside each caption in a flow
    /// makes a legend.
    pub keys: bool,
}

/// A [`Widget::Color`]: a button showing `color` that opens the system's
/// colour picker; each change is `PanelEvent::Color`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorField {
    pub id: String,
    pub label: String,
    /// Red, green and blue, each from 0 to 1 (sRGB).
    pub color: [f32; 3],
}

/// A [`Widget::Range`]: a slider over `min` to `max`, its value written
/// beside it with `decimals` and `suffix` (` %`); told as
/// `PanelEvent::Number`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Range {
    pub id: String,
    pub label: String,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    /// The values it stops at, every `step` from `min`.
    pub step: Option<f64>,
    pub decimals: usize,
    pub suffix: String,
    pub enabled: bool,
}

impl Default for Range {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            value: 0.0,
            min: 0.0,
            max: 1.0,
            step: None,
            decimals: 2,
            suffix: String::new(),
            enabled: true,
        }
    }
}

/// A [`Widget::RowButton`]: `text` at the left of a frameless button the
/// panel's width, told as `PanelEvent::Button`. The `current` one (where
/// what the panel moves stands) is in the accent, strong, and takes no
/// click.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RowButton {
    pub id: String,
    pub text: String,
    pub current: bool,
}

/// A [`Widget::Selectable`]: words that light up while `selected`, told
/// as `PanelEvent::Button` when clicked; a few in a [`Widget::Flow`] pick
/// one of them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Selectable {
    pub id: String,
    pub text: String,
    pub selected: bool,
}

/// A picture a panel shows, as rows of sRGB pixels with their alpha.
/// The host keeps it by `key`, so a picture shown again under the same
/// key is not sent to the screen again: a key names one picture.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Picture {
    pub key: String,
    pub width: u32,
    pub height: u32,
    /// `width × height` pixels of four bytes each, red, green, blue and
    /// alpha, row by row from the top.
    pub rgba: Vec<u8>,
}

/// A choice of a [`Gallery`]: its picture over its name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tile {
    pub label: String,
    pub picture: Picture,
}

/// A [`Widget::Gallery`]: small square pictures side by side, going on
/// to the next line, each over its name; the `selected` one ringed in
/// the accent. A click is `PanelEvent::Choice` with the tile's place.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Gallery {
    pub id: String,
    pub tiles: Vec<Tile>,
    pub selected: Option<usize>,
}

/// A [`Widget::Settings`]: under its overline `title` (none when empty),
/// a bordered group with one row per child, the child's label at the
/// left and its control at the right, a line between rows. A
/// [`Widget::Hinted`] child's hint shows under its label; a
/// [`Widget::Check`] with no label is a switch, its `text` the label.
/// The Preferences search narrows the rows by label and hint.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SettingsGroup {
    pub title: String,
    pub children: Vec<Widget>,
}

macro_rules! block_into_widget {
    ($($block:ident => $variant:ident),* $(,)?) => {
        $(impl From<$block> for Widget {
            fn from(block: $block) -> Self {
                Widget::$variant(block)
            }
        })*
    };
}

block_into_widget! {
    PanelSection => Section,
    Item => Item,
    ItemList => Items,
    FilterField => Filter,
    StatusLine => Status,
    Stack => Stack,
    Flow => Flow,
    Fill => Fill,
    Fields => Fields,
    Inline => Inline,
    PickChip => PickChip,
    Entry => Entry,
    TaskHeader => TaskHeader,
    Quantity => Quantity,
    Check => Check,
    Select => Select,
    FieldGrid => Grid,
    Alert => Alert,
    Card => Card,
    Caption => Caption,
    Gap => Gap,
    Plot => Plot,
    Swatches => Swatches,
    ColorField => Color,
    Range => Range,
    Toggles => Toggles,
    RowButton => RowButton,
    Selectable => Selectable,
    Gallery => Gallery,
    SettingsGroup => Settings,
    Cells => Cells,
    PanelAction => Action,
}

/// A cell of a [`Widget::Sheet`].
///
/// A package builds these, so a variant keeps its fields for as long as
/// the contract's minor version holds: a new option comes as a new
/// variant, or as a new struct that derives `Default`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Cell {
    /// Text; `strong` for a row heading the rows under it.
    Text {
        text: String,
        #[serde(default)]
        mono: bool,
        #[serde(default)]
        strong: bool,
    },
    /// Text a click on selects its row (`PanelEvent::Select`).
    Link { text: String },
    /// Text the user edits, told once typing ends (`PanelEvent::CellText`).
    Edit { text: String },
    /// A box ticked or not (`PanelEvent::CellCheck`).
    Check {
        on: bool,
        #[serde(default)]
        hint: Option<String>,
    },
}

/// A line of a [`Widget::Diagram`]: what it stands for, not its colour,
/// so the host draws it in the design's own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagramStroke {
    /// The outline of the part.
    #[default]
    Outline,
    /// An edge behind the surface, dashed.
    Hidden,
    /// An axis or a centre line, long-dashed and faint.
    Axis,
    /// A faint line: a hatch, a reference.
    Thin,
    /// Drawn in the accent colour.
    Accent,
}

/// A shape of a [`Widget::Diagram`], in its coordinates.
///
/// A package builds these, so a variant keeps its fields for as long as
/// the contract's minor version holds: a new option comes as a new
/// variant, or as a new struct that derives `Default`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DiagramShape {
    /// A run of straight lines; `fill` shades it, and reads only a convex
    /// outline right.
    Path {
        points: Vec<[f32; 2]>,
        #[serde(default)]
        closed: bool,
        #[serde(default)]
        stroke: DiagramStroke,
        #[serde(default)]
        fill: bool,
    },
    Circle {
        center: [f32; 2],
        radius: f32,
        #[serde(default)]
        stroke: DiagramStroke,
        #[serde(default)]
        fill: bool,
    },
    /// Text at a point, centred on it.
    Text {
        at: [f32; 2],
        text: String,
        #[serde(default)]
        mono: bool,
    },
}

/// A measure of a [`Widget::Diagram`]: the distance from `from` to `to`,
/// drawn as a dimension line `offset` away from them (to the left of the
/// way from `from` to `to`; negative for the right), its ends led out to
/// the points, with `text` on it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Dimension {
    pub from: [f32; 2],
    pub to: [f32; 2],
    #[serde(default)]
    pub offset: f32,
    pub text: String,
    /// Drawn in the accent colour: the one being edited.
    #[serde(default)]
    pub emphasis: bool,
}

/// A note of a [`Widget::Diagram`] pointing at a feature of it: a dot at
/// `anchor`, a leader to `at`, and `text` there.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Callout {
    pub anchor: [f32; 2],
    pub at: [f32; 2],
    pub text: String,
    #[serde(default)]
    pub emphasis: bool,
}

fn two() -> usize {
    2
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonStyle {
    Primary,
    #[default]
    Secondary,
    Destructive,
    /// A small secondary button, for an action on the row it sits in.
    Small,
    /// The plain button of the system's controls, unframed in style.
    Plain,
    /// A plain button in the small text size.
    PlainSmall,
    /// Outlined in the accent colour: takes what is picked in the view.
    Outline,
}

/// A number field's parameter: feature `feature`'s parameter `key`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bind {
    pub feature: String,
    pub key: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ListItem {
    pub label: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

/// Which declared panel an event came from.
#[cfg_attr(not(feature = "exhaustive"), non_exhaustive)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelSlot {
    Task,
    Settings,
    /// The strip under the view (`Frame::transport`).
    Transport,
    /// The card over the view (`Frame::card`).
    Card,
}

/// What the user did to a panel widget, by the widget's id.
#[cfg_attr(not(feature = "exhaustive"), non_exhaustive)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PanelEvent {
    Number {
        id: String,
        value: f64,
    },
    Choice {
        id: String,
        index: usize,
    },
    Toggle {
        id: String,
        on: bool,
    },
    Text {
        id: String,
        value: String,
    },
    Button {
        id: String,
    },
    Pick {
        id: String,
    },
    Select {
        id: String,
        index: usize,
    },
    /// A cell of a table's editable column, typed in: its text as left.
    Cell {
        id: String,
        row: usize,
        column: usize,
        value: String,
    },
    /// A sheet's editable cell, once typing ended.
    CellText {
        id: String,
        row: usize,
        column: usize,
        value: String,
    },
    /// A sheet's box ticked or cleared.
    CellCheck {
        id: String,
        row: usize,
        column: usize,
        on: bool,
    },
    /// A diagram pressed at `x`, `y` in its own coordinates; `dragging`
    /// for each move of the pointer held down after the press, and
    /// `released` once, where the drag ended.
    DiagramPress {
        id: String,
        x: f32,
        y: f32,
        #[serde(default)]
        dragging: bool,
        #[serde(default)]
        released: bool,
    },
    /// The pointer came onto an item (`index` in its list, 0 for an
    /// item alone), or left the list (`None`).
    Hover {
        id: String,
        index: Option<usize>,
    },
    /// An item double clicked.
    Activate {
        id: String,
        index: usize,
    },
    /// An entry of an item's menu chosen: the entry's id in `action`.
    ContextMenu {
        id: String,
        index: usize,
        action: String,
    },
    /// A tag of an item clicked: `tag` is its place among the item's.
    Tag {
        id: String,
        index: usize,
        tag: usize,
    },
    /// A multiple list's selection, whole, as the user left it.
    Selection {
        id: String,
        indices: Vec<usize>,
    },
    /// A filter the bench applies, typed in or its kind chosen.
    Filter {
        id: String,
        text: String,
        kind: usize,
    },
    /// A picked chip cleared.
    Clear {
        id: String,
    },
    /// A badge of an item clicked: `badge` is its place among the item's.
    Badge {
        id: String,
        index: usize,
        badge: usize,
    },
    /// What the pointer is on in a list changed: the item (`None` off
    /// the list), and the tag or badge of it under the pointer.
    HoverPart {
        id: String,
        index: Option<usize>,
        tag: Option<usize>,
        badge: Option<usize>,
    },
    /// An item's name typed in while it is `editing`: at each change,
    /// and with `done` as the field is left.
    ItemText {
        id: String,
        index: usize,
        value: String,
        done: bool,
    },
    /// A colour field's colour changed: red, green and blue from 0 to 1.
    Color {
        id: String,
        color: [f32; 3],
    },
    /// A `live` [`Entry`]'s field left, its text as it stands: by Escape
    /// when `escaped`, which drops what was typed.
    TextLeft {
        id: String,
        value: String,
        escaped: bool,
    },
    /// The floating card `id` closed: by OK (or Enter, or a click
    /// elsewhere in the view) when `accept`, else by Cancel or Escape.
    CardClose {
        id: String,
        accept: bool,
    },
}

impl PanelEvent {
    pub fn id(&self) -> &str {
        match self {
            PanelEvent::Number { id, .. }
            | PanelEvent::Choice { id, .. }
            | PanelEvent::Toggle { id, .. }
            | PanelEvent::Text { id, .. }
            | PanelEvent::Button { id }
            | PanelEvent::Pick { id }
            | PanelEvent::Select { id, .. }
            | PanelEvent::Cell { id, .. }
            | PanelEvent::CellText { id, .. }
            | PanelEvent::CellCheck { id, .. }
            | PanelEvent::DiagramPress { id, .. }
            | PanelEvent::Hover { id, .. }
            | PanelEvent::Activate { id, .. }
            | PanelEvent::ContextMenu { id, .. }
            | PanelEvent::Tag { id, .. }
            | PanelEvent::Selection { id, .. }
            | PanelEvent::Filter { id, .. }
            | PanelEvent::Clear { id }
            | PanelEvent::Badge { id, .. }
            | PanelEvent::HoverPart { id, .. }
            | PanelEvent::ItemText { id, .. }
            | PanelEvent::Color { id, .. }
            | PanelEvent::TextLeft { id, .. }
            | PanelEvent::CardClose { id, .. } => id,
        }
    }
}
