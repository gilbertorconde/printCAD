//! The Design task panel as declared widgets, written once for both of
//! its passes: declaring lists the widgets the host draws, applying runs
//! the same editor code with one event the panel reported, and the field
//! the event names takes the new value. An editor reads as it would in an
//! immediate-mode panel (`if form.check(..) { .. }`), so its conditions
//! and what each control changes stay in one place.

use std::collections::HashMap;

use bench_api::panel::hinted;
use bench_api::{
    ButtonStyle, Caption, Card, Check, Entry, Flow, Gap, GapSize, Item, ItemList, NoteKind,
    PanelAction, PanelEvent, PanelSection, Quantity, Select, ToggleButton, Toggles, Tone, Widget,
};

/// Which pass a [`Form`] runs.
enum Pass<'e> {
    /// Collecting the widgets, a list per open container.
    Declare(Vec<Vec<Widget>>),
    /// Handing `event` to the widget it names; `None` once taken.
    Apply(Option<&'e PanelEvent>),
}

/// One pass over a task panel's editors.
pub(crate) struct Form<'e> {
    pass: Pass<'e>,
    /// The ids given so far, keeping two widgets of one name apart.
    names: bench_api::panel::Ids,
    /// Why freezing a borrow failed, by the borrow, kept by the bench
    /// between passes until the next try.
    pub(crate) freeze_errors: HashMap<core_document::FeatureId, String>,
}

impl<'e> Form<'e> {
    /// The pass that lists the widgets.
    pub(crate) fn declare() -> Self {
        Self {
            pass: Pass::Declare(vec![Vec::new()]),
            names: Default::default(),
            freeze_errors: HashMap::new(),
        }
    }

    /// The pass that hands `event` to its widget.
    pub(crate) fn apply(event: &'e PanelEvent) -> Self {
        Self {
            pass: Pass::Apply(Some(event)),
            names: Default::default(),
            freeze_errors: HashMap::new(),
        }
    }

    /// The widgets declared.
    pub(crate) fn finish(self) -> Vec<Widget> {
        match self.pass {
            Pass::Declare(mut lists) => lists.pop().unwrap_or_default(),
            Pass::Apply(_) => Vec::new(),
        }
    }

    /// An id for a widget named `name`, unique in the panel.
    fn id(&mut self, name: &str) -> String {
        self.names.next(name)
    }

    fn push(&mut self, widget: impl Into<Widget>) {
        if let Pass::Declare(lists) = &mut self.pass
            && let Some(list) = lists.last_mut()
        {
            list.push(widget.into());
        }
    }

    /// The event, taken, when it names `id`.
    fn take(&mut self, id: &str) -> Option<&'e PanelEvent> {
        match &mut self.pass {
            Pass::Apply(event) if event.is_some_and(|e| e.id() == id) => event.take(),
            _ => None,
        }
    }

    /// Children declared by `body` into a new list, which `wrap` makes a
    /// widget of; in the apply pass `body` runs alone.
    fn nest<R>(
        &mut self,
        body: impl FnOnce(&mut Self) -> R,
        wrap: impl FnOnce(Vec<Widget>) -> Widget,
    ) -> R {
        if let Pass::Declare(lists) = &mut self.pass {
            lists.push(Vec::new());
        }
        let out = body(self);
        if let Pass::Declare(lists) = &mut self.pass
            && let Some(children) = lists.pop()
        {
            self.push(wrap(children));
        }
        out
    }

    /// A number; the value typed or dragged to.
    pub(crate) fn quantity(&mut self, q: Quantity) -> Option<f64> {
        self.quantity_hinted(q, "")
    }

    /// A [`Self::quantity`] with `hint` on hover.
    pub(crate) fn quantity_hinted(&mut self, mut q: Quantity, hint: &str) -> Option<f64> {
        q.id = self.id(&q.label);
        let id = q.id.clone();
        self.push(hinted(hint, q.into()));
        match self.take(&id) {
            Some(PanelEvent::Number { value, .. }) => Some(*value),
            _ => None,
        }
    }

    /// A choice, named `name` apart from its label; the option picked.
    pub(crate) fn select(&mut self, name: &str, select: Select) -> Option<usize> {
        self.select_hinted(name, select, "")
    }

    /// A [`Self::select`] with `hint` on hover over its row.
    pub(crate) fn select_hinted(
        &mut self,
        name: &str,
        mut select: Select,
        hint: &str,
    ) -> Option<usize> {
        select.id = self.id(name);
        let id = select.id.clone();
        self.push(hinted(hint, select.into()));
        match self.take(&id) {
            Some(PanelEvent::Choice { index, .. }) => Some(*index),
            _ => None,
        }
    }

    /// A check box with `text` beside it and `hint` on hover; whether it
    /// was switched, `on` following.
    pub(crate) fn check(&mut self, on: &mut bool, text: &str, hint: &str) -> bool {
        let id = self.id(text);
        self.push(Check {
            id: id.clone(),
            text: text.into(),
            on: *on,
            hint: hint.into(),
            ..Default::default()
        });
        match self.take(&id) {
            Some(PanelEvent::Toggle { on: now, .. }) => {
                *on = *now;
                true
            }
            _ => false,
        }
    }

    /// A row of icon buttons under `label`, each an icon, its words on
    /// hover and whether it is lit; which was clicked.
    pub(crate) fn toggles(&mut self, label: &str, buttons: &[(&str, &str, bool)]) -> Option<usize> {
        let id = self.id(label);
        let ids: Vec<String> = (0..buttons.len()).map(|n| format!("{id}:{n}")).collect();
        self.push(Toggles {
            id: id.clone(),
            label: label.into(),
            buttons: buttons
                .iter()
                .zip(&ids)
                .map(|((icon, hint, on), id)| ToggleButton {
                    id: id.clone(),
                    icon: icon.to_string(),
                    hint: hint.to_string(),
                    on: *on,
                    disabled: false,
                })
                .collect(),
        });
        ids.iter()
            .position(|id| matches!(self.take(id), Some(PanelEvent::Button { .. })))
    }

    /// A button; whether it was pressed.
    pub(crate) fn button(
        &mut self,
        label: &str,
        style: ButtonStyle,
        enabled: bool,
        hint: &str,
    ) -> bool {
        let id = self.id(label);
        self.push(hinted(
            hint,
            Widget::Button {
                id: id.clone(),
                label: label.into(),
                style,
                enabled,
            },
        ));
        matches!(self.take(&id), Some(PanelEvent::Button { .. }))
    }

    /// A list row with a small cross at its end; whether the cross was
    /// pressed.
    pub(crate) fn removable(&mut self, mut item: Item, hint: &str) -> bool {
        let id = self.id(&format!("{} remove", item.label));
        item.field = Some(Box::new(Widget::Action(PanelAction {
            id: id.clone(),
            icon: Some("close".into()),
            label: hint.into(),
            ..Default::default()
        })));
        self.push(item);
        matches!(self.take(&id), Some(PanelEvent::Button { .. }))
    }

    /// A list of rows named `name`; the event it reported.
    pub(crate) fn items(&mut self, name: &str, mut list: ItemList) -> Option<&'e PanelEvent> {
        list.id = self.id(name);
        let id = list.id.clone();
        self.push(list);
        self.take(&id)
    }

    /// A text field named `name`; the event it reported.
    pub(crate) fn entry(&mut self, name: &str, mut entry: Entry) -> Option<&'e PanelEvent> {
        entry.id = self.id(name);
        let id = entry.id.clone();
        self.push(entry);
        self.take(&id)
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
