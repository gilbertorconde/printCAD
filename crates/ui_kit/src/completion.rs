//! Completion of names while a formula is typed.
//!
//! [`completing_text_edit`] draws a single-line text edit and, while it
//! has the keyboard and the word at the cursor starts a name, a dropdown
//! under it of the five candidates most like that word, each with what it
//! is (a variable's value). Up and Down move through them; Enter or Tab
//! puts the highlighted one in place of the word; a click puts in the one
//! clicked; Escape puts the dropdown away until the text changes.
//!
//! [`Completion`] is the same over any edit and any rule for what a word
//! is: the chat's `@` mentions complete through it in a multiline box.

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
    /// The candidate put in this frame, by key or click.
    pub taken: Option<Candidate>,
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

/// The candidates most like `word`, nearest first, at most `most`; with
/// no word, the first `most` as they come.
pub fn nearest(word: &str, candidates: &[Candidate], most: usize) -> Vec<Candidate> {
    if word.is_empty() {
        return candidates.iter().take(most).cloned().collect();
    }
    let mut ranked: Vec<(usize, &Candidate)> = candidates
        .iter()
        .filter_map(|c| Some((distance(word, &c.text)?, c)))
        .collect();
    ranked.sort_by(|(a, x), (b, y)| {
        a.cmp(b)
            .then(x.text.len().cmp(&y.text.len()))
            .then(x.text.cmp(&y.text))
    });
    ranked
        .into_iter()
        .take(most)
        .map(|(_, c)| c.clone())
        .collect()
}

/// The names most like `word` that would change it, nearest first, at
/// most [`SHOWN`]: a name typed out in full has nothing left to complete.
pub fn best_matches(word: &str, candidates: &[Candidate]) -> Vec<Candidate> {
    let bare = |t: &str| t.to_lowercase().replace('`', "");
    let word_bare = bare(word);
    let open: Vec<Candidate> = candidates
        .iter()
        .filter(|c| bare(&c.text) != word_bare)
        .cloned()
        .collect();
    nearest(word, &open, SHOWN)
}

/// The byte offset of char `index` in `text`.
fn byte_of(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(i, _)| i)
}

/// The cursor of the edit with id `id`, as a byte offset of `text`, from
/// what egui kept of it: the text's end when it kept none.
pub fn cursor_of(ui: &Ui, id: Id, text: &str) -> usize {
    egui::text_edit::TextEditState::load(ui.ctx(), id)
        .and_then(|s| s.cursor.char_range())
        .map_or(text.len(), |r| byte_of(text, r.primary.index.0))
}

/// The byte range of a word to complete and the candidates shown for it.
pub type Found = (std::ops::Range<usize>, Vec<Candidate>);

/// Where the word to complete is and what may take its place: given the
/// text and the cursor's byte offset, the byte range a candidate replaces
/// and the candidates shown, nearest first; `None` (or none shown) while
/// there is nothing to complete. `after` follows a candidate put in.
/// The dropdown opens under the edit, or over it with `above` (an edit at
/// the window's bottom), on the other side where that side has no room.
pub struct Completer<'a> {
    pub find: &'a dyn Fn(&str, usize) -> Option<Found>,
    pub after: &'a str,
    pub above: bool,
}

/// A completion in progress over one text edit: [`Completion::begin`]
/// before the edit is added, which takes the dropdown's keys,
/// [`Completion::finish`] after it, which draws the dropdown.
pub struct Completion<'a> {
    id: Id,
    state: State,
    completer: Completer<'a>,
    /// The dropdown shows and has the keys this frame: Enter and Tab,
    /// the arrows and Escape are its, not the edit's.
    pub open: bool,
    taken: Option<Candidate>,
}

/// Put `candidate` (and `after`) in place of `range`, the cursor after it.
fn put(
    ui: &Ui,
    id: Id,
    text: &mut String,
    range: std::ops::Range<usize>,
    candidate: &str,
    after: &str,
) {
    let inserted = format!("{candidate}{after}");
    text.replace_range(range.clone(), &inserted);
    place_cursor(ui, id, text, range.start + inserted.len());
}

/// Put the cursor of the edit with id `id` at byte `at` of `text`.
pub fn place_cursor(ui: &Ui, id: Id, text: &str, at: usize) {
    let index = text[..at.min(text.len())].chars().count();
    let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(index),
        )));
    state.store(ui.ctx(), id);
}

impl<'a> Completion<'a> {
    /// Start completing in the edit with id `id`: while it has the
    /// keyboard and the dropdown shows, its keys move the highlight, put
    /// the highlighted candidate in (Enter, Tab) or put the dropdown away
    /// until the text changes (Escape).
    pub fn begin(ui: &mut Ui, id: Id, text: &mut String, completer: Completer<'a>) -> Self {
        let mut state: State = ui
            .data(|d| d.get_temp(id.with("completion")))
            .unwrap_or_default();
        let mut open = false;
        let mut taken = None;
        if ui.memory(|m| m.has_focus(id)) && state.dismissed.as_deref() != Some(text.as_str()) {
            let cursor = cursor_of(ui, id, text);
            if let Some((range, shown)) = (completer.find)(text, cursor)
                && !shown.is_empty()
            {
                open = true;
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
                    let chosen = shown[state.highlight].clone();
                    put(ui, id, text, range, &chosen.text, completer.after);
                    // Tab would move the keyboard on to the next widget: it stays.
                    ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
                    taken = Some(chosen);
                }
                if escape {
                    state.dismissed = Some(text.clone());
                }
            }
        }
        Self {
            id,
            state,
            completer,
            open,
            taken,
        }
    }

    /// Draw the dropdown for the edit that answered `response`, on the
    /// side its completer asks for while there is room, over everything
    /// else. A click puts a candidate in and gives the edit the keyboard
    /// back.
    pub fn finish(mut self, ui: &mut Ui, response: Response, text: &mut String) -> Completed {
        let id = self.id;
        let mut picked = false;
        let still = response.has_focus() || response.lost_focus();
        if still && self.state.dismissed.as_deref() != Some(text.as_str()) {
            let cursor = cursor_of(ui, id, text);
            if let Some((range, shown)) = (self.completer.find)(text, cursor)
                && !shown.is_empty()
            {
                let highlight = self.state.highlight.min(shown.len() - 1);
                let width = response.rect.width().max(240.0);
                let height = shown.len() as f32 * ROW + 8.0;
                let room = ui.ctx().content_rect();
                let fits_below = response.rect.bottom() + 2.0 + height <= room.bottom();
                let fits_above = response.rect.top() - 2.0 - height >= room.top();
                let below = match self.completer.above {
                    true => !fits_above,
                    false => fits_below,
                };
                let (pivot, at) = match below {
                    true => (
                        egui::Align2::LEFT_TOP,
                        response.rect.left_bottom() + Vec2::new(0.0, 2.0),
                    ),
                    false => (
                        egui::Align2::LEFT_BOTTOM,
                        response.rect.left_top() - Vec2::new(0.0, 2.0),
                    ),
                };
                let area = egui::Area::new(id.with("completion_popup"))
                    .order(egui::Order::Foreground)
                    .pivot(pivot)
                    .fixed_pos(at)
                    .show(ui.ctx(), |ui| dropdown(ui, &shown, highlight, width));
                if let Some(chosen) = area.inner {
                    put(ui, id, text, range, &chosen.text, self.completer.after);
                    ui.memory_mut(|m| m.request_focus(id));
                    picked = true;
                    self.taken = Some(chosen);
                }
            }
        }
        ui.data_mut(|d| d.insert_temp(id.with("completion"), self.state));
        Completed {
            response,
            picked,
            taken: self.taken,
        }
    }
}

/// A dropdown row's height.
const ROW: f32 = 22.0;

/// The candidates, `highlight` lit, each `width` wide: the one clicked.
fn dropdown(ui: &mut Ui, shown: &[Candidate], highlight: usize, width: f32) -> Option<Candidate> {
    egui::Frame::new()
        .fill(BG2)
        .stroke(egui::Stroke::new(1.0, BORDER_STRONG))
        .corner_radius(RADIUS_MD as u8)
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| {
            ui.set_width(width);
            let mut clicked = None;
            for (i, c) in shown.iter().enumerate() {
                let (rect, row) = ui.allocate_exact_size(Vec2::new(width, ROW), Sense::click());
                if i == highlight {
                    ui.painter().rect_filled(rect, RADIUS_SM, ACCENT_DIM);
                } else if row.hovered() {
                    ui.painter().rect_filled(rect, RADIUS_SM, BG3);
                }
                let mut line = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect.shrink2(Vec2::new(6.0, 0.0)))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                line.add(
                    egui::Label::new(RichText::new(&c.text).font(mono(FONT_SM)).color(TEXT1))
                        .truncate()
                        .selectable(false),
                );
                line.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&c.detail).font(sans(FONT_XS)).color(TEXT3))
                            .truncate()
                            .selectable(false),
                    );
                });
                if row.clicked() {
                    clicked = Some(c.clone());
                }
            }
            clicked
        })
        .inner
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
    let find = |text: &str, cursor: usize| {
        let range = word_at(text, cursor)?;
        let all = candidates();
        let shown = best_matches(&text[range.start..cursor], &all);
        Some((range, shown))
    };
    let completion = Completion::begin(
        ui,
        id,
        text,
        Completer {
            find: &find,
            after: "",
            above: false,
        },
    );
    let response = ui.add(style(TextEdit::singleline(text).id(id)));
    completion.finish(ui, response, text)
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
            .map(|c| c.text)
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
