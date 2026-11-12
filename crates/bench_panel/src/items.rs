//! Item rows and lists, their filter, and what the pointer does over them.

use super::*;

/// The most badges an item's row shows before "+N".
pub(super) const ITEM_BADGES: usize = 4;

/// What the user did to an item's row.
pub(super) struct ItemDrawn {
    pub(super) response: egui::Response,
    /// The tag and the badge clicked.
    pub(super) tag: Option<usize>,
    pub(super) badge: Option<usize>,
    /// The tag and the badge under the pointer.
    pub(super) over_tag: Option<usize>,
    pub(super) over_badge: Option<usize>,
    /// An `editing` item's name as typed this frame, and whether its
    /// field was left.
    pub(super) text: Option<(String, bool)>,
}

/// One item as a panel list row: icon, name, dim words and tags, then its
/// value (or its field) and badges at the end. An `editing` item is its
/// icon and a text field for its name.
pub(super) fn item_row(
    ui: &mut Ui,
    salt: egui::Id,
    item: &Item,
    field: &mut dyn FnMut(&mut Ui, &Widget),
) -> ItemDrawn {
    let icon_color = match item.color {
        Some(_) => color_or(item.color, item.tone),
        None if item.muted => TEXT3,
        None => tone_color(item.tone),
    };
    let mut drawn = Marks::default();
    if item.editing {
        let mut name = item.label.clone();
        let response = ui
            .horizontal(|ui| {
                ui_kit::icon::draw(
                    ui,
                    item.icon.as_deref().unwrap_or_default(),
                    14.0,
                    icon_color,
                );
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .id(salt.with("name"))
                        .desired_width(ui.available_width())
                        .font(ui_kit::theme::sans(FONT_SM)),
                );
                // Focus once, as editing starts.
                if !edit.has_focus() && !edit.lost_focus() {
                    edit.request_focus();
                }
                if edit.changed() || edit.lost_focus() {
                    drawn.text = Some((name.clone(), edit.lost_focus()));
                }
            })
            .response;
        return drawn.on(response.interact(egui::Sense::click()));
    }
    let look = widgets::RowLook {
        selected: item.selected,
        problem: item.problem,
        dim: item.muted,
        related: item.related,
        lit: item.lit,
    };
    let response = widgets::list_row_with(
        ui,
        salt,
        look,
        (item.icon.as_deref().unwrap_or_default(), icon_color),
        &item.label,
        |ui| {
            widgets::row_detail(ui, item.secondary.as_deref().unwrap_or_default());
            let tags: Vec<(&str, widgets::ChipLook)> = item
                .tags
                .iter()
                .map(|t| {
                    let look = widgets::ChipLook {
                        selected: t.selected,
                        lit: t.lit,
                        tone: (!matches!(t.tone, Tone::Neutral | Tone::Accent | Tone::Muted))
                            .then(|| note_of(t.tone)),
                    };
                    (t.text.as_str(), look)
                })
                .collect();
            for (i, response) in widgets::chips(ui, &tags).into_iter().enumerate() {
                if response.hovered() {
                    drawn.over_tag = Some(i);
                }
                if response.clicked() {
                    drawn.tag = Some(i);
                }
            }
        },
        |ui| {
            if let Some(widget) = &item.field {
                field(ui, widget);
            } else if let Some(value) = &item.value {
                let color = tone_color(item.value_tone);
                let response = if item.value_plain {
                    ui.label(
                        RichText::new(value)
                            .font(ui_kit::theme::sans(FONT_SM))
                            .color(color),
                    )
                } else {
                    widgets::row_value(ui, value, color)
                };
                if let Some(hint) = &item.value_hint {
                    response.on_hover_text(hint);
                }
            }
            if let Some(flag) = &item.flag {
                let response = widgets::mono_label(ui, &flag.text, FONT_XS, tone_color(flag.tone));
                if !flag.detail.is_empty() {
                    response.on_hover_text(&flag.detail);
                }
            }
            let badges: Vec<(&str, egui::Color32, bool)> = item
                .badges
                .iter()
                .map(|b| (b.icon.as_str(), color_or(b.color, b.tone), b.lit))
                .collect();
            for (i, (response, badge)) in widgets::badges(ui, &badges, ITEM_BADGES)
                .into_iter()
                .zip(&item.badges)
                .enumerate()
            {
                let response = if badge.hint.is_empty() {
                    response
                } else {
                    response.on_hover_text(&badge.hint)
                };
                if response.hovered() {
                    drawn.over_badge = Some(i);
                }
                if response.clicked() {
                    drawn.badge = Some(i);
                }
            }
        },
    );
    let response = match &item.hint {
        Some(hint) => response.on_hover_text(hint),
        None => response,
    };
    if item.reveal {
        response.scroll_to_me(Some(egui::Align::Center));
    }
    drawn.on(response)
}

/// What a row's tags, badges and name field reported, before the row's
/// own response is known.
#[derive(Default)]
pub(super) struct Marks {
    pub(super) tag: Option<usize>,
    pub(super) badge: Option<usize>,
    pub(super) over_tag: Option<usize>,
    pub(super) over_badge: Option<usize>,
    pub(super) text: Option<(String, bool)>,
}

impl Marks {
    fn on(self, response: egui::Response) -> ItemDrawn {
        ItemDrawn {
            response,
            tag: self.tag,
            badge: self.badge,
            over_tag: self.over_tag,
            over_badge: self.over_badge,
            text: self.text,
        }
    }
}

/// An item's double click, tag and badge clicks, name typed and menu, as
/// events of `id`.
pub(super) fn item_events(
    id: &str,
    index: usize,
    item: &Item,
    drawn: &ItemDrawn,
    out: &mut PanelOutput,
) {
    if drawn.response.double_clicked() {
        out.events.push(PanelEvent::Activate {
            id: id.to_string(),
            index,
        });
    }
    if let Some(tag) = drawn.tag {
        out.events.push(PanelEvent::Tag {
            id: id.to_string(),
            index,
            tag,
        });
    }
    if let Some(badge) = drawn.badge {
        out.events.push(PanelEvent::Badge {
            id: id.to_string(),
            index,
            badge,
        });
    }
    if let Some((value, done)) = &drawn.text {
        out.events.push(PanelEvent::ItemText {
            id: id.to_string(),
            index,
            value: value.clone(),
            done: *done,
        });
    }
    if !item.menu.is_empty() {
        drawn.response.context_menu(|ui| {
            for action in &item.menu {
                if action.separated {
                    ui.separator();
                }
                let response = match &action.icon {
                    Some(icon) => {
                        ui.horizontal(|ui| {
                            ui_kit::icon::draw(ui, icon, 14.0, TEXT2);
                            ui.button(&action.label)
                        })
                        .inner
                    }
                    None => ui.button(&action.label),
                };
                let response = if action.hint.is_empty() {
                    response
                } else {
                    response.on_hover_text(&action.hint)
                };
                if response.clicked() {
                    out.events.push(PanelEvent::ContextMenu {
                        id: id.to_string(),
                        index,
                        action: action.id.clone(),
                    });
                    ui.close();
                }
            }
        });
    }
}

/// What the pointer is on in a list: the item, and its tag or badge.
pub(super) type Pointed = (Option<usize>, Option<usize>, Option<usize>);

/// Tell the bench when the tag or badge under the pointer changed, kept
/// under `key`.
pub(super) fn part_changed(ui: &Ui, key: egui::Id, id: &str, now: Pointed, out: &mut PanelOutput) {
    let before: Pointed = ui.data(|d| d.get_temp(key)).unwrap_or_default();
    if before != now {
        ui.data_mut(|d| d.insert_temp(key, now));
        out.events.push(PanelEvent::HoverPart {
            id: id.to_string(),
            index: now.0,
            tag: now.1,
            badge: now.2,
        });
    }
}

/// Tell the bench when the item under the pointer changed since the last
/// frame drawn, kept under `key`.
pub(super) fn hover_changed(
    ui: &Ui,
    key: egui::Id,
    id: &str,
    hovered: Option<usize>,
    out: &mut PanelOutput,
) {
    let before: Option<usize> = ui.data(|d| d.get_temp(key)).flatten();
    if before != hovered {
        ui.data_mut(|d| d.insert_temp(key, hovered));
        out.events.push(PanelEvent::Hover {
            id: id.to_string(),
            index: hovered,
        });
    }
}

/// What a [`Widget::Filter`] holds between frames: the text typed, the
/// kind chosen, and what the lists it narrows need to read them.
#[derive(Debug, Clone, Default)]
pub(super) struct FilterState {
    pub(super) text: String,
    pub(super) kind: usize,
    pub(super) kinds: Vec<String>,
    pub(super) by_bench: bool,
}

pub(super) fn filter_key(panel: egui::Id, filter: &str) -> egui::Id {
    panel.with(("filter", filter))
}

/// A [`Widget::Filter`]: the panel's filter field, its kinds chosen at
/// its end.
pub(super) fn filter_widget(
    ui: &mut Ui,
    id: egui::Id,
    filter: &FilterField,
    out: &mut PanelOutput,
) {
    let key = filter_key(id, &filter.id);
    let mut state: FilterState = ui.data(|d| d.get_temp(key)).unwrap_or_else(|| FilterState {
        kind: filter.kind.min(filter.kinds.len().saturating_sub(1)),
        ..Default::default()
    });
    state.kinds.clone_from(&filter.kinds);
    state.by_bench = filter.by_bench;
    let mut kind = state.kind;
    let kinds: Vec<(usize, &str)> = filter
        .kinds
        .iter()
        .enumerate()
        .map(|(i, k)| (i, k.as_str()))
        .collect();
    let mut kind_changed = false;
    let hint = if filter.hint.is_empty() {
        "Filter"
    } else {
        &filter.hint
    };
    let typed = widgets::filter_field(ui, &mut state.text, hint, |ui| {
        if !kinds.is_empty() {
            kind_changed =
                widgets::select_field(ui, key.with("kind"), &mut kind, &kinds, FILTER_KIND_WIDTH);
        }
    });
    state.kind = kind;
    if (typed || kind_changed) && filter.by_bench {
        out.events.push(PanelEvent::Filter {
            id: filter.id.clone(),
            text: state.text.clone(),
            kind,
        });
    }
    ui.data_mut(|d| d.insert_temp(key, state));
}

/// The width of a filter's kind selector.
pub(super) const FILTER_KIND_WIDTH: f32 = 104.0;

/// Whether `item` passes a filter's text (in its label, secondary words
/// or tags, any case) and kind (the first passing every item).
pub(super) fn passes(item: &Item, state: &FilterState) -> bool {
    let wanted = state.text.trim().to_lowercase();
    let text_ok = wanted.is_empty()
        || item.label.to_lowercase().contains(&wanted)
        || item
            .secondary
            .as_ref()
            .is_some_and(|s| s.to_lowercase().contains(&wanted))
        || item
            .tags
            .iter()
            .any(|t| t.text.to_lowercase().contains(&wanted));
    let kind_ok = state.kind == 0
        || state
            .kinds
            .get(state.kind)
            .is_none_or(|kind| item.kind.as_ref() == Some(kind));
    text_ok && kind_ok
}

/// A [`Widget::Items`]: its rows as the filter it names passes them, a
/// click selecting (Ctrl adding, in a multiple list).
pub(super) fn item_list(
    ui: &mut Ui,
    id: egui::Id,
    list: &ItemList,
    document: Option<&Document>,
    out: &mut PanelOutput,
) {
    out.lists.push(list.id.clone());
    let state: Option<FilterState> = list
        .filter
        .as_ref()
        .and_then(|f| ui.data(|d| d.get_temp(filter_key(id, f))));
    let shown: Vec<usize> = (0..list.items.len())
        .filter(|&i| {
            state
                .as_ref()
                .is_none_or(|s| s.by_bench || passes(&list.items[i], s))
        })
        .collect();
    if shown.is_empty()
        && let Some(empty) = &list.empty
    {
        ui.label(
            RichText::new(empty)
                .font(ui_kit::theme::sans(FONT_SM))
                .color(TEXT3),
        );
    }
    let mut pointed: Pointed = (None, None, None);
    let mut clicked = None;
    for &i in &shown {
        let item = &list.items[i];
        let drawn = {
            let mut field = field_drawer(id, document, out);
            item_row(ui, id.with(("items", &list.id, i)), item, &mut field)
        };
        if ui.rect_contains_pointer(drawn.response.rect) {
            pointed.0 = Some(i);
        }
        if drawn.over_tag.is_some() || drawn.over_badge.is_some() {
            pointed = (Some(i), drawn.over_tag, drawn.over_badge);
        }
        if drawn.response.clicked() {
            clicked = Some(i);
        }
        item_events(&list.id, i, item, &drawn, out);
    }
    if let Some(i) = clicked {
        if list.multi {
            let adding = ui.input(|input| input.modifiers.command);
            let mut indices: Vec<usize> = if adding {
                (0..list.items.len())
                    .filter(|&k| list.items[k].selected != (k == i))
                    .collect()
            } else {
                vec![i]
            };
            indices.sort_unstable();
            out.events.push(PanelEvent::Selection {
                id: list.id.clone(),
                indices,
            });
        } else {
            out.events.push(PanelEvent::Select {
                id: list.id.clone(),
                index: i,
            });
        }
    }
    hover_changed(
        ui,
        id.with(("items_hover", &list.id)),
        &list.id,
        pointed.0,
        out,
    );
    part_changed(
        ui,
        id.with(("items_part", &list.id)),
        &list.id,
        pointed,
        out,
    );
}

/// Draws an item's end control: a quantity as its bare field, anything
/// else as a widget of its own.
pub(super) fn field_drawer<'a>(
    id: egui::Id,
    document: Option<&'a Document>,
    out: &'a mut PanelOutput,
) -> impl FnMut(&mut Ui, &Widget) + 'a {
    move |ui, widget| match widget {
        Widget::Quantity(q) => quantity(ui, id, q, false, document, out),
        other => {
            let place = Place {
                column: Column::Labels(LabelColumn::inline()),
                fill: false,
            };
            show_one(ui, id, other, place, document, out);
        }
    }
}
