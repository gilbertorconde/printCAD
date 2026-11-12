//! The Design task panel as declared widgets, written once for both of
//! its passes through `bench_api::panel::Form`: declaring lists the
//! widgets the host draws, applying runs the same editor code with one
//! event the panel reported, and the field the event names takes the new
//! value. An editor reads as it would in an immediate-mode panel
//! (`if form.check(..) { .. }`), so its conditions and what each control
//! changes stay in one place. This wrapper keeps Design's own look: a
//! hint on hover, sections, flows and cards.

use std::collections::HashMap;

use bench_api::panel::{self, hinted};
use bench_api::{
    ButtonStyle, Caption, Card, Check, Entry, Flow, Gap, GapSize, Item, ItemList, NoteKind,
    PanelAction, PanelEvent, PanelSection, Quantity, Select, ToggleButton, Tone, Widget,
};

/// One pass over a task panel's editors.
pub(crate) struct Form<'e> {
    form: panel::Form<'e>,
    /// Why freezing a borrow failed, by the borrow, kept by the bench
    /// between passes until the next try.
    pub(crate) freeze_errors: HashMap<core_document::FeatureId, String>,
}

impl<'e> Form<'e> {
    /// The pass that lists the widgets.
    pub(crate) fn declare() -> Self {
        Self {
            form: panel::Form::declare(),
            freeze_errors: HashMap::new(),
        }
    }

    /// The pass that hands `event` to its widget.
    pub(crate) fn apply(event: &'e PanelEvent) -> Self {
        Self {
            form: panel::Form::apply(event),
            freeze_errors: HashMap::new(),
        }
    }

    /// The widgets declared.
    pub(crate) fn finish(self) -> Vec<Widget> {
        self.form.finish()
    }

    fn push(&mut self, widget: impl Into<Widget>) {
        self.form.push(widget);
    }

    /// Children declared by `body` into a new list, which `wrap` makes a
    /// widget of.
    fn nest<R>(
        &mut self,
        body: impl FnOnce(&mut Self) -> R,
        wrap: impl FnOnce(Vec<Widget>) -> Widget,
    ) -> R {
        self.form.open();
        let out = body(self);
        let children = self.form.close();
        self.push(wrap(children));
        out
    }

    /// A number; the value typed or dragged to.
    pub(crate) fn quantity(&mut self, q: Quantity) -> Option<f64> {
        self.quantity_hinted(q, "")
    }

    /// A [`Self::quantity`] with `hint` on hover.
    pub(crate) fn quantity_hinted(&mut self, q: Quantity, hint: &str) -> Option<f64> {
        let mut value = q.value;
        let name = q.label.clone();
        let bound = self.form.number(&name, &mut value, q);
        self.push(hinted(hint, bound.widget.into()));
        bound.set.then_some(value)
    }

    /// A choice, named `name` apart from its label; the option picked.
    pub(crate) fn select(&mut self, name: &str, select: Select) -> Option<usize> {
        self.select_hinted(name, select, "")
    }

    /// A [`Self::select`] with `hint` on hover over its row.
    pub(crate) fn select_hinted(
        &mut self,
        name: &str,
        select: Select,
        hint: &str,
    ) -> Option<usize> {
        let mut index = select.selected;
        let bound = self.form.index(name, &mut index, select);
        self.push(hinted(hint, bound.widget.into()));
        bound.set.then_some(index)
    }

    /// A check box with `text` beside it and `hint` on hover; whether it
    /// was switched, `on` following.
    pub(crate) fn check(&mut self, on: &mut bool, text: &str, hint: &str) -> bool {
        let shape = Check {
            text: text.into(),
            hint: hint.into(),
            ..Default::default()
        };
        let bound = self.form.check(text, on, shape);
        self.form.put(bound)
    }

    /// A row of icon buttons under `label`, each an icon, its words on
    /// hover and whether it is lit; which was clicked.
    pub(crate) fn toggles(&mut self, label: &str, buttons: &[(&str, &str, bool)]) -> Option<usize> {
        let buttons = buttons
            .iter()
            .map(|(icon, hint, on)| ToggleButton {
                icon: icon.to_string(),
                hint: hint.to_string(),
                on: *on,
                ..Default::default()
            })
            .collect();
        let bound = self.form.toggles(label, label, buttons);
        self.push(bound.widget);
        bound.set
    }

    /// A button; whether it was pressed.
    pub(crate) fn button(
        &mut self,
        label: &str,
        style: ButtonStyle,
        enabled: bool,
        hint: &str,
    ) -> bool {
        let bound = self.form.button(label, label, style, enabled);
        self.push(hinted(hint, bound.widget));
        bound.set
    }

    /// A list row with a small cross at its end; whether the cross was
    /// pressed.
    pub(crate) fn removable(&mut self, mut item: Item, hint: &str) -> bool {
        let cross = PanelAction {
            icon: Some("close".into()),
            label: hint.into(),
            ..Default::default()
        };
        let bound = self.form.action(&format!("{} remove", item.label), cross);
        item.field = Some(Box::new(Widget::Action(bound.widget)));
        self.push(item);
        bound.set
    }

    /// A list of rows named `name`; the event it reported.
    pub(crate) fn items(&mut self, name: &str, mut list: ItemList) -> Option<&'e PanelEvent> {
        list.id = self.form.id(name);
        let id = list.id.clone();
        self.push(list);
        self.form.take(&id)
    }

    /// A text field named `name`; the event it reported.
    pub(crate) fn entry(&mut self, name: &str, mut entry: Entry) -> Option<&'e PanelEvent> {
        entry.id = self.form.id(name);
        let id = entry.id.clone();
        self.push(entry);
        self.form.take(&id)
    }
    /// Words in a tone, `small` or the panel's size, `mono` or not.
    pub(crate) fn caption(&mut self, text: &str, tone: Tone, small: bool, mono: bool) {
        self.push(Caption {
            text: text.into(),
            tone,
            small,
            mono,
            ..Default::default()
        });
    }

    /// A value read: small mono words in the primary text colour.
    pub(crate) fn reading(&mut self, text: &str) {
        self.push(Caption {
            text: text.into(),
            small: true,
            mono: true,
            primary: true,
            ..Default::default()
        });
    }

    /// A note card.
    pub(crate) fn note(&mut self, kind: NoteKind, title: Option<&str>, text: &str) {
        self.push(Widget::Note {
            kind,
            title: title.map(String::from),
            text: text.into(),
        });
    }

    pub(crate) fn gap(&mut self, size: GapSize) {
        self.push(Gap { size });
    }

    /// What `body` declares side by side after `label` in the label
    /// column, going on to the next line when narrow if `wrap`.
    pub(crate) fn flow<R>(
        &mut self,
        label: &str,
        wrap: bool,
        body: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let label = label.to_string();
        self.nest(body, |children| {
            Flow {
                children,
                no_wrap: !wrap,
                label,
            }
            .into()
        })
    }

    /// A fold titled `title`, its `summary` while closed, open at first
    /// when `open`.
    pub(crate) fn section<R>(
        &mut self,
        title: &str,
        summary: &str,
        open: bool,
        body: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let (title, summary) = (title.to_string(), summary.to_string());
        self.nest(body, |children| {
            PanelSection {
                id: title.clone(),
                title,
                summary: Some(summary),
                open,
                children,
                ..Default::default()
            }
            .into()
        })
    }

    /// A raised card `padding` points in from its edge.
    pub(crate) fn card<R>(&mut self, padding: f32, body: impl FnOnce(&mut Self) -> R) -> R {
        self.nest(body, |children| {
            Card {
                children,
                padding: Some(padding),
                ..Default::default()
            }
            .into()
        })
    }
}
