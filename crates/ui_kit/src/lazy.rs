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
/// out whatever.
pub fn row<R>(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    force: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> Row<R> {
    let id = ui.id().with(("lazy_row", salt));
    let width = ui.available_width();
    let laid: Option<Laid> = ui.data(|d| d.get_temp(id));
    if let Some(laid) = laid
        && !force
        && !laid.focus
        && (laid.width - width).abs() < 0.5
        && !busy(ui.ctx())
    {
        let top = ui.cursor().top();
        let sight = ui.clip_rect().expand(MARGIN);
        if top > sight.bottom() || top + laid.height < sight.top() {
            let (_, rect) = ui.allocate_space(egui::vec2(width, laid.height));
            return Row { inner: None, rect };
        }
    }
    let shown = ui.scope(add);
    let rect = shown.response.rect;
    let focus = ui.ctx().memory(|m| m.focused()).is_some_and(|focused| {
        ui.ctx()
            .read_response(focused)
            .is_some_and(|r| rect.contains_rect(r.rect))
    });
    ui.data_mut(|d| {
        d.insert_temp(
            id,
            Laid {
                width,
                height: rect.height(),
                focus,
            },
        );
    });
    Row {
        inner: Some(shown.inner),
        rect,
    }
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
    /// tall the list stood.
    fn second_frame(rows: usize, offset: f32) -> (Vec<usize>, f32) {
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
                        for i in 0..rows {
                            row(ui, i, false, |ui| {
                                laid.push(i);
                                ui.allocate_space(egui::vec2(50.0, 20.0));
                            });
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
        let (laid, height) = second_frame(200, 1000.0);
        let (all, full) = second_frame(200, 0.0);
        assert_eq!(height, full, "the list stands as tall either way");
        // What shows, a hundred points of rows 20 tall around 1000 down,
        // and its margin: one short run, nowhere near either end.
        assert!(laid.len() < 20, "{} rows laid out", laid.len());
        assert!(laid.windows(2).all(|w| w[1] == w[0] + 1), "{laid:?}");
        assert!(laid[0] > 30 && laid[laid.len() - 1] < 70, "{laid:?}");
        assert!(all.contains(&0) && !all.contains(&199), "{all:?}");
    }
}
