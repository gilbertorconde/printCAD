//! Rows of a long list drawn only near sight. egui builds every widget
//! every frame, so a list that grows with the document costs its whole
//! length each frame however little of it shows; [`row`] lays a row out
//! only when it is within [`MARGIN`] of the visible part of its `Ui`, and
//! otherwise takes the room it had when last laid out at that width.
//!
//! A row is always laid out while it holds the keyboard focus (egui drops
//! the focus of a widget a frame does not build), while anything is being
//! dragged and while a popup is open (a drag's source and a context
//! menu's row must go on being built), when it has never been laid out at
//! this width, and when the caller says so (a row being scrolled to).

use egui::{Rect, Ui};

/// How far past the visible part rows are still laid out, in points: the
/// margin also keeps the next field a Tab moves to built.
pub const MARGIN: f32 = 60.0;

/// What a row was, the last time it was laid out.
#[derive(Clone, Copy)]
struct Laid {
    width: f32,
    height: f32,
    focus: bool,
}

/// A row: what its drawing returned, `None` when it was passed over, and
/// the room it took either way.
pub struct Row<R> {
    pub inner: Option<R>,
    pub rect: Rect,
}

/// Lays out `add` as one row of a vertical list, known by `salt` within
/// `ui`, or takes its last room when it is out of sight. `force` lays it
/// out whatever. A list of many rows goes through [`rows`] instead,
/// which reads what its rows were once for the whole list.
pub fn row<R>(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    force: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> Row<R> {
    let id = ui.id().with(("lazy_row", salt));
    let laid: Option<Laid> = ui.data(|d| d.get_temp(id));
    let ctx = ui.ctx().clone();
    let (row, now) = lay_out(ui, laid, force, || busy(&ctx), add);
    if let Some(now) = now {
        ui.data_mut(|d| d.insert_temp(id, now));
    }
    row
}

/// The rows of one list, each known by its index in the list, with what
/// each was when last laid out: read once for the list, kept again once
/// it is drawn.
pub struct Rows {
    laid: Vec<Option<Laid>>,
    busy: bool,
}

/// Draws a list known by `salt` within `ui` through `add`, which lays
/// out each of its rows with [`Rows::row`].
pub fn rows<R>(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    add: impl FnOnce(&mut Ui, &mut Rows) -> R,
) -> R {
    let id = ui.id().with(("lazy_rows", salt));
    let laid = ui.data_mut(|d| d.remove_temp(id)).unwrap_or_default();
    let mut rows = Rows {
        laid,
        busy: busy(ui.ctx()),
    };
    let out = add(ui, &mut rows);
    let laid = rows.laid;
    ui.data_mut(|d| d.insert_temp(id, laid));
    out
}

impl Rows {
    /// Lays out `add` as the row at `index`, or takes its last room when
    /// it is out of sight, as [`row`] does.
    pub fn row<R>(
        &mut self,
        ui: &mut Ui,
        index: usize,
        force: bool,
        add: impl FnOnce(&mut Ui) -> R,
    ) -> Row<R> {
        let laid = self.laid.get(index).copied().flatten();
        let (row, now) = lay_out(ui, laid, force, || self.busy, add);
        if let Some(now) = now {
            if self.laid.len() <= index {
                self.laid.resize(index + 1, None);
            }
            self.laid[index] = Some(now);
        }
        row
    }
}

/// A row passed over, taking `laid`'s room, when it was laid out at this
/// width, held no focus and stands out of sight, unless `force` or
/// `busy`; otherwise laid out, with what it is now.
fn lay_out<R>(
    ui: &mut Ui,
    laid: Option<Laid>,
    force: bool,
    busy: impl FnOnce() -> bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> (Row<R>, Option<Laid>) {
    let width = ui.available_width();
    if let Some(laid) = laid
        && !force
        && !laid.focus
        && (laid.width - width).abs() < 0.5
        && !busy()
    {
        let top = ui.cursor().top();
        let sight = ui.clip_rect().expand(MARGIN);
        if top > sight.bottom() || top + laid.height < sight.top() {
            let (_, rect) = ui.allocate_space(egui::vec2(width, laid.height));
            return (Row { inner: None, rect }, None);
        }
    }
    let shown = ui.scope(add);
    let rect = shown.response.rect;
    let focus = ui.ctx().memory(|m| m.focused()).is_some_and(|focused| {
        ui.ctx()
            .read_response(focused)
            .is_some_and(|r| rect.contains_rect(r.rect))
    });
    let now = Laid {
        width,
        height: rect.height(),
        focus,
    };
    let row = Row {
        inner: Some(shown.inner),
        rect,
    };
    (row, Some(now))
}

/// Whether rows must all be built this frame: something is dragged or a
/// popup is open.
fn busy(ctx: &egui::Context) -> bool {
    ctx.dragged_id().is_some() || egui::Popup::is_any_open(ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `rows` rows of 20 points in a 100-point window scrolled to
    /// `offset`, twice (the first frame lays every row out to learn its
    /// height), and returns which rows the second frame laid out and how
    /// tall the list stood; with `listed`, the rows drawn as one list.
    fn second_frame(rows: usize, offset: f32, listed: bool) -> (Vec<usize>, f32) {
        let ctx = egui::Context::default();
        let mut laid = Vec::new();
        let mut height = 0.0;
        for _ in 0..2 {
            laid.clear();
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(200.0, 100.0),
                )),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let out = egui::ScrollArea::vertical()
                    .vertical_scroll_offset(offset)
                    .show(ui, |ui| {
                        let add = |i: usize| {
                            move |ui: &mut Ui| {
                                ui.allocate_space(egui::vec2(50.0, 20.0));
                                i
                            }
                        };
                        if listed {
                            super::rows(ui, "list", |ui, list| {
                                for i in 0..rows {
                                    laid.extend(list.row(ui, i, false, add(i)).inner);
                                }
                            });
                        } else {
                            for i in 0..rows {
                                laid.extend(row(ui, i, false, add(i)).inner);
                            }
                        }
                    });
                height = out.content_size.y;
            });
            out.textures_delta.clear();
        }
        (laid, height)
    }

    #[test]
    fn rows_far_out_of_sight_are_not_laid_out_but_keep_their_room() {
        for listed in [false, true] {
            far_rows_passed_over(listed);
        }
    }

    fn far_rows_passed_over(listed: bool) {
        let (laid, height) = second_frame(200, 1000.0, listed);
        let (all, full) = second_frame(200, 0.0, listed);
        assert_eq!(height, full, "the list stands as tall either way");
        // What shows, a hundred points of rows 20 tall around 1000 down,
        // and its margin: one short run, nowhere near either end.
        assert!(laid.len() < 20, "{} rows laid out", laid.len());
        assert!(laid.windows(2).all(|w| w[1] == w[0] + 1), "{laid:?}");
        assert!(laid[0] > 30 && laid[laid.len() - 1] < 70, "{laid:?}");
        assert!(all.contains(&0) && !all.contains(&199), "{all:?}");
    }
}
