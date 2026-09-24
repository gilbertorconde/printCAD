//! Completion of names while a formula is typed.
//!
//! [`completing_text_edit`] draws a single-line text edit and, while it
//! has the keyboard and the word at the cursor starts a name, a dropdown
//! under it of the five candidates most like that word, each with what it
//! is (a variable's value). Up and Down move through them; Enter or Tab
//! puts the highlighted one in place of the word; a click puts in the one
//! clicked; Escape puts the dropdown away until the text changes.

use egui::{Id, Response, RichText, Sense, TextEdit, Ui, Vec2};

use crate::tokens::*;
use crate::{mono, sans};

/// A name that can be put in, and what it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// As a formula writes it: `Printer.nozzle`, `` `Pad 2`.length ``.
    pub text: String,
    /// Shown beside it: its value, `0.4 mm`.
    pub detail: String,
}

/// How many the dropdown shows.
pub const SHOWN: usize = 5;

/// What the text edit did.
pub struct Completed {
    pub response: Response,
    /// A candidate was clicked this frame: the edit lost the keyboard to
    /// the click and gets it back next frame, so a caller that keeps the
    /// text when the edit loses the keyboard skips this frame.
    pub picked: bool,
}

#[derive(Debug, Clone, Default)]
struct State {
    /// The highlighted one, of those shown.
    highlight: usize,
    /// The word the highlight is for: a new word starts at the top.
    word: String,
    /// The text Escape put the dropdown away for.
    dismissed: Option<String>,
}

/// Whether `c` can be part of a name being typed.
fn in_name(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '`')
}

/// The word being typed at byte `cursor` of `text`: its byte range, if it
/// can start a name (not empty, not a number, not the unit after a
/// number: `2 in`, `3mm`).
pub fn word_at(text: &str, cursor: usize) -> Option<std::ops::Range<usize>> {
    let cursor = cursor.min(text.len());
    let start = text[..cursor]
        .char_indices()
        .rev()
        .take_while(|(_, c)| in_name(*c))
        .last()
        .map_or(cursor, |(i, _)| i);
    let end = cursor
        + text[cursor..]
            .char_indices()
            .take_while(|(_, c)| in_name(*c))
            .last()
            .map_or(0, |(i, c)| i + c.len_utf8());
    let first = text[start..].chars().next()?;
    let after_number = text[..start]
        .trim_end()
        .chars()
        .next_back()
        .is_some_and(|c| c.is_ascii_digit() || c == '.');
    (start < cursor && !first.is_ascii_digit() && first != '.' && !after_number)
        .then_some(start..end)
}

/// How far `candidate` is from what `word` means, lower nearer; `None`
/// when it is nothing like it.
fn distance(word: &str, candidate: &str) -> Option<usize> {
    let word = word.to_lowercase().replace('`', "");
    let full = candidate.to_lowercase().replace('`', "");
    let property = full.rsplit('.').next().unwrap_or(&full);
    if full == word {
        return None;
    }
    if full.starts_with(&word) {
        return Some(0);
    }
    if property.starts_with(&word) {
        return Some(1);
    }
    if let Some(at) = full.find(&word) {
        return Some(2 + at);
    }
    // The word's letters in order, gaps counted.
    let mut gaps = 0;
    let mut rest = full.chars();
    for w in word.chars() {
        let mut skipped = 0;
        loop {
            match rest.next() {
                Some(c) if c == w => break,
                Some(_) => skipped += 1,
                None => return None,
            }
        }
        gaps += skipped;
    }
    Some(100 + gaps)
}

/// The candidates most like `word`, nearest first, at most [`SHOWN`].
pub fn best_matches<'a>(word: &str, candidates: &'a [Candidate]) -> Vec<&'a Candidate> {
    let mut ranked: Vec<(usize, &Candidate)> = candidates
        .iter()
        .filter_map(|c| Some((distance(word, &c.text)?, c)))
        .collect();
    ranked.sort_by(|(a, x), (b, y)| {
        a.cmp(b)
            .then(x.text.len().cmp(&y.text.len()))
            .then(x.text.cmp(&y.text))
    });
    ranked.into_iter().take(SHOWN).map(|(_, c)| c).collect()
}

/// The byte offset of char `index` in `text`.
fn byte_of(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(i, _)| i)
}

/// The edit's cursor, as a byte offset, from what egui kept of it.
fn cursor_of(ui: &Ui, id: Id, text: &str) -> usize {
    egui::text_edit::TextEditState::load(ui.ctx(), id)
        .and_then(|s| s.cursor.char_range())
        .map_or(text.len(), |r| byte_of(text, r.primary.index.0))
}

/// Put `candidate` in place of the word at `range` and the cursor after it.
fn put(ui: &Ui, id: Id, text: &mut String, range: std::ops::Range<usize>, candidate: &str) {
    text.replace_range(range.clone(), candidate);
    let after = text[..range.start].chars().count() + candidate.chars().count();
    let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(after),
        )));
    state.store(ui.ctx(), id);
}

/// A single-line text edit with id `id` over `text` that completes names
/// from `candidates` (asked for only while there is a word to complete).
/// `style` sets the edit up: its font, width, hint.
pub fn completing_text_edit(
    ui: &mut Ui,
    id: Id,
    text: &mut String,
    candidates: &dyn Fn() -> Vec<Candidate>,
    style: impl FnOnce(TextEdit<'_>) -> TextEdit<'_>,
) -> Completed {
    let state_id = id.with("completion");
    let mut state: State = ui.data(|d| d.get_temp(state_id)).unwrap_or_default();
    let focused = ui.memory(|m| m.has_focus(id));

    // The keys, before the edit sees them: they belong to the dropdown
    // while it shows.
    if focused {
        let cursor = cursor_of(ui, id, text);
        if let Some(range) = word_at(text, cursor)
            && state.dismissed.as_deref() != Some(text.as_str())
        {
            let all = candidates();
            let shown = best_matches(&text[range.start..cursor], &all);
            if !shown.is_empty() {
                let word = text[range.start..cursor].to_string();
                if word != state.word {
                    state.word = word;
                    state.highlight = 0;
                }
                state.highlight = state.highlight.min(shown.len() - 1);
                let (down, up, take, escape) = ui.input_mut(|i| {
                    let none = egui::Modifiers::NONE;
                    (
                        i.consume_key(none, egui::Key::ArrowDown),
                        i.consume_key(none, egui::Key::ArrowUp),
                        i.consume_key(none, egui::Key::Enter)
                            || i.consume_key(none, egui::Key::Tab),
                        i.consume_key(none, egui::Key::Escape),
                    )
                });
                if down {
                    state.highlight = (state.highlight + 1) % shown.len();
                }
                if up {
                    state.highlight = (state.highlight + shown.len() - 1) % shown.len();
                }
                if take {
                    let chosen = shown[state.highlight].text.clone();
                    put(ui, id, text, range, &chosen);
                }
                if escape {
                    state.dismissed = Some(text.clone());
                }
            }
        }
    }

    let response = ui.add(style(TextEdit::singleline(text).id(id)));

    // The dropdown, under the edit, over everything else.
    let mut picked = false;
    let still = response.has_focus() || response.lost_focus();
    if still && state.dismissed.as_deref() != Some(text.as_str()) {
        let cursor = cursor_of(ui, id, text);
        if let Some(range) = word_at(text, cursor) {
            let all = candidates();
            let shown = best_matches(&text[range.start..cursor], &all);
            if !shown.is_empty() {
                let highlight = state.highlight.min(shown.len() - 1);
                let width = response.rect.width().max(240.0);
                let area = egui::Area::new(id.with("completion_popup"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(response.rect.left_bottom() + Vec2::new(0.0, 2.0))
                    .show(ui.ctx(), |ui| {
                        egui::Frame::new()
                            .fill(BG2)
                            .stroke(egui::Stroke::new(1.0, BORDER_STRONG))
                            .corner_radius(RADIUS_MD as u8)
                            .inner_margin(egui::Margin::same(3))
                            .show(ui, |ui| {
                                ui.set_width(width);
                                let mut clicked = None;
                                for (i, c) in shown.iter().enumerate() {
                                    let (rect, row) = ui.allocate_exact_size(
                                        Vec2::new(width, 22.0),
                                        Sense::click(),
                                    );
                                    if i == highlight {
                                        ui.painter().rect_filled(rect, RADIUS_SM, ACCENT_DIM);
                                    } else if row.hovered() {
                                        ui.painter().rect_filled(rect, RADIUS_SM, BG3);
                                    }
                                    let mut line = ui.new_child(
                                        egui::UiBuilder::new()
                                            .max_rect(rect.shrink2(Vec2::new(6.0, 0.0)))
                                            .layout(egui::Layout::left_to_right(
                                                egui::Align::Center,
                                            )),
                                    );
                                    line.add(
                                        egui::Label::new(
                                            RichText::new(&c.text).font(mono(FONT_SM)).color(TEXT1),
                                        )
                                        .truncate()
                                        .selectable(false),
                                    );
                                    line.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new(&c.detail)
                                                    .font(sans(FONT_XS))
                                                    .color(TEXT3),
                                            );
                                        },
                                    );
                                    if row.clicked() {
                                        clicked = Some(c.text.clone());
                                    }
                                }
                                clicked
                            })
                            .inner
                    });
                if let Some(chosen) = area.inner {
                    put(ui, id, text, range, &chosen);
                    ui.memory_mut(|m| m.request_focus(id));
                    picked = true;
                }
            }
        }
    }
    ui.data_mut(|d| d.insert_temp(state_id, state));
    Completed { response, picked }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates(names: &[&str]) -> Vec<Candidate> {
        names
            .iter()
            .map(|n| Candidate {
                text: n.to_string(),
                detail: String::new(),
            })
            .collect()
    }

    fn best(word: &str, names: &[&str]) -> Vec<String> {
        let all = candidates(names);
        best_matches(word, &all)
            .into_iter()
            .map(|c| c.text.clone())
            .collect()
    }

    #[test]
    fn the_word_at_the_cursor_is_what_is_completed() {
        let text = "2 * Printer.noz + 1";
        assert_eq!(word_at(text, 15), Some(4..15));
        // Mid-word: the whole word is replaced.
        assert_eq!(word_at(text, 8), Some(4..15));
        assert_eq!(word_at("12.5", 4), None, "a number is not a name");
        assert_eq!(word_at("2 * ", 4), None);
        assert_eq!(word_at("1 in", 4), None, "a unit is not a name");
        assert_eq!(word_at("3mm", 3), None);
        assert_eq!(
            word_at("2 * wall", 8),
            Some(4..8),
            "after an operator it is"
        );
        assert_eq!(
            word_at("x + `Pad", 8),
            Some(4..8),
            "the start of a backtick name"
        );
    }

    #[test]
    fn prefixes_come_first_then_the_property_then_the_rest() {
        let names = [
            "Printer.nozzle",
            "Printer.layer",
            "Bracket.nozzle_gap",
            "Pad.length",
            "Printer.wall",
            "Plate.width",
            "Sketch.span",
        ];
        assert_eq!(best("Printer.n", &names), ["Printer.nozzle"]);
        assert_eq!(
            best("noz", &names),
            ["Printer.nozzle", "Bracket.nozzle_gap"],
            "the property's start, the shorter first"
        );
        assert_eq!(
            best("pr", &names)[0],
            "Printer.wall",
            "shortest of the prefixes"
        );
        assert_eq!(best("P", &names).len(), SHOWN, "no more than five");
        assert_eq!(best("pln", &names), ["Pad.length"], "its letters in order");
        assert!(
            best("Printer.nozzle", &names).is_empty(),
            "nothing left to complete"
        );
        assert!(best("zzz", &names).is_empty());
    }

    /// Focus an edit holding `text`, press `keys` a frame each: the text.
    fn type_keys(text: &str, keys: &[egui::Key]) -> String {
        let ctx = egui::Context::default();
        let id = Id::new("formula");
        let mut text = text.to_string();
        let all = candidates(&["Printer.nozzle", "Bracket.nozzle_gap", "Pad.length"]);
        ctx.memory_mut(|m| m.request_focus(id));
        let mut frames: Vec<Vec<egui::Event>> = vec![Vec::new()];
        frames.extend(keys.iter().map(|key| {
            vec![egui::Event::Key {
                key: *key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]
        }));
        for events in frames {
            let raw = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                completing_text_edit(ui, id, &mut text, &|| all.clone(), |e| e);
            });
            output.textures_delta.clear();
        }
        text
    }

    #[test]
    fn arrows_move_through_the_suggestions_and_enter_takes_one() {
        use egui::Key::{ArrowDown, ArrowUp, Enter, Escape, Tab};
        assert_eq!(type_keys("2 * noz", &[Enter]), "2 * Printer.nozzle");
        assert_eq!(
            type_keys("2 * noz", &[ArrowDown, Enter]),
            "2 * Bracket.nozzle_gap"
        );
        assert_eq!(
            type_keys("2 * noz", &[ArrowDown, ArrowDown, Tab]),
            "2 * Printer.nozzle",
            "round again"
        );
        assert_eq!(
            type_keys("2 * noz", &[ArrowUp, Enter]),
            "2 * Bracket.nozzle_gap"
        );
        assert_eq!(
            type_keys("2 * noz", &[Escape, Enter]),
            "2 * noz",
            "put away: Enter is the edit's again"
        );
        assert_eq!(type_keys("2 in", &[Enter]), "2 in", "a unit is left alone");
    }

    #[test]
    fn a_click_takes_a_suggestion_and_the_edit_keeps_the_keyboard() {
        let ctx = egui::Context::default();
        let id = Id::new("formula");
        let mut text = "2 * noz".to_string();
        let all = candidates(&["Printer.nozzle", "Bracket.nozzle_gap"]);
        ctx.memory_mut(|m| m.request_focus(id));
        let mut picked = false;
        let frame = |ctx: &egui::Context, events: Vec<egui::Event>, text: &mut String| {
            let raw = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut out = false;
            let mut output = ctx.run_ui(raw, |ui| {
                out = completing_text_edit(ui, id, text, &|| all.clone(), |e| e).picked;
            });
            output.textures_delta.clear();
            out
        };
        frame(&ctx, Vec::new(), &mut text);
        let popup = ctx
            .memory(|m| m.area_rect(id.with("completion_popup")))
            .expect("the dropdown shows");
        // The second row.
        let at = egui::pos2(popup.center().x, popup.top() + 3.0 + 22.0 + 11.0);
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, vec![egui::Event::PointerMoved(at)], &mut text);
        frame(&ctx, vec![button(true)], &mut text);
        picked |= frame(&ctx, vec![button(false)], &mut text);
        frame(&ctx, Vec::new(), &mut text);
        assert_eq!(text, "2 * Bracket.nozzle_gap");
        assert!(picked, "the edit says a suggestion was picked");
        assert!(ctx.memory(|m| m.has_focus(id)), "and has the keyboard back");
    }
}
