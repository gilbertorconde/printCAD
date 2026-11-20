//! Tabs: one tab with its close and dirty marks, dragging tabs into a
//! new order, and the button that adds one.

use super::*;

/// One tab of a strip: the open documents, a panel's pages, the chats.
/// The active tab is filled and underlined in the accent; a closable tab
/// shows its close while hovered or active, and a middle click closes it
/// too.
pub struct Tab<'a> {
    label: &'a str,
    active: bool,
    /// Fixed width; `None` fits the label.
    width: Option<f32>,
    height: f32,
    /// A dot for unsaved edits.
    dirty: bool,
    /// Closable, with this hover text on its close.
    close: Option<&'a str>,
    /// An icon before the label, in its tint: something to know about
    /// what the tab holds.
    mark: Option<(&'a str, Color32)>,
    /// Picked up and moved along its strip by a drag ([`TabReorder`]).
    draggable: bool,
}

/// What a click on a tab asked for.
pub struct TabResponse {
    /// The tab itself, for hover text.
    pub response: Response,
    /// Clicked while not active.
    pub selected: bool,
    /// Its close was clicked, or it was middle-clicked.
    pub closed: bool,
}

const TAB_CLOSE: f32 = 16.0;

const TAB_PAD: f32 = 10.0;

const TAB_MARK: f32 = 12.0;

impl<'a> Tab<'a> {
    pub fn new(label: &'a str, active: bool) -> Self {
        Self {
            label,
            active,
            width: None,
            height: TAB_BAR - 4.0,
            dirty: false,
            close: None,
            mark: None,
            draggable: false,
        }
    }

    /// A drag picks the tab up to move it along its strip; a
    /// [`TabReorder`] over the strip's tabs says where it lands.
    pub fn draggable(mut self) -> Self {
        self.draggable = true;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    pub fn dirty(mut self, dirty: bool) -> Self {
        self.dirty = dirty;
        self
    }

    /// Draw icon `mark.0` in tint `mark.1` before the label; `None`, none.
    pub fn marked(mut self, mark: Option<(&'a str, Color32)>) -> Self {
        self.mark = mark;
        self
    }

    pub fn closable(mut self, hint: &'a str) -> Self {
        self.close = Some(hint);
        self
    }

    pub fn show(self, ui: &mut Ui) -> TabResponse {
        let font = if self.active {
            crate::theme::sans_medium(FONT_SM)
        } else {
            sans(FONT_SM)
        };
        let text_color = if self.active { TEXT1 } else { TEXT2 };
        // Room right of the label: the close, and the dot before it.
        let trailing =
            self.close.map_or(0.0, |_| TAB_CLOSE + 4.0) + if self.dirty { 12.0 } else { 0.0 };
        let leading = if self.mark.is_some() {
            TAB_MARK + 4.0
        } else {
            0.0
        };
        let width = self.width.unwrap_or_else(|| {
            let text =
                ui.painter()
                    .layout_no_wrap(self.label.to_string(), font.clone(), text_color);
            text.size().x + 2.0 * TAB_PAD + leading + trailing
        });
        let sense = if self.draggable {
            Sense::click_and_drag()
        } else {
            Sense::click()
        };
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, self.height), sense);
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::Button, ui.is_enabled(), self.active, self.label)
        });
        let hovered = response.hovered();
        let painter = ui.painter();
        if self.active {
            painter.rect_filled(rect, RADIUS_SM, BG0);
            painter.hline(
                rect.x_range(),
                rect.bottom() - 1.0,
                Stroke::new(2.0, ACCENT),
            );
        } else if hovered {
            painter.rect_filled(rect, RADIUS_SM, BG2);
        }

        let close_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 6.0 - TAB_CLOSE / 2.0, rect.center().y),
            Vec2::splat(TAB_CLOSE),
        );
        let mut text_right = if self.close.is_some() {
            close_rect.left() - 4.0
        } else {
            rect.right() - TAB_PAD
        };
        if self.dirty {
            painter.circle_filled(egui::pos2(text_right - 4.0, rect.center().y), 3.0, WARNING);
            text_right -= 12.0;
        }
        let text_left = rect.left() + TAB_PAD + leading;
        if let Some((name, tint)) = self.mark {
            let mark = egui::Rect::from_center_size(
                egui::pos2(rect.left() + TAB_PAD + TAB_MARK / 2.0, rect.center().y),
                Vec2::splat(TAB_MARK),
            );
            if let Some(texture) = crate::icon::texture(ui.ctx(), name) {
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                painter.image(texture.id(), mark, uv, tint);
            }
        }
        let max_width = (text_right - text_left).max(0.0);
        // One line, cut short with an ellipsis where the tab is too narrow.
        let mut job =
            egui::text::LayoutJob::simple_singleline(self.label.to_string(), font, text_color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(max_width);
        let galley = painter.layout_job(job);
        let clip = egui::Rect::from_min_max(rect.min, egui::pos2(text_right, rect.max.y));
        painter.with_clip_rect(clip).galley(
            egui::pos2(text_left, rect.center().y - galley.size().y / 2.0),
            galley,
            text_color,
        );

        let mut closed = false;
        if let Some(hint) = self.close {
            let close = ui.interact(close_rect, response.id.with("close"), Sense::click());
            close.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), hint));
            if hovered || self.active || close.hovered() {
                if close.hovered() {
                    ui.painter().rect_filled(close_rect, RADIUS_SM, BG2);
                }
                ui.painter().text(
                    close_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "×",
                    sans(FONT_MD),
                    if close.hovered() { TEXT1 } else { TEXT3 },
                );
            }
            closed = close.on_hover_text(hint).clicked() || response.middle_clicked();
        }
        TabResponse {
            selected: !closed && response.clicked() && !self.active,
            closed,
            response,
        }
    }
}

/// Moving tabs along a strip by dragging: each [`Tab::draggable`] tab's
/// response goes in as it is drawn, and [`TabReorder::finish`] marks
/// where a held tab would land and, on release, says where it went.
#[derive(Default)]
pub struct TabReorder {
    rects: Vec<egui::Rect>,
    held: Option<usize>,
    dropped: bool,
}

impl TabReorder {
    pub fn push(&mut self, tab: &TabResponse) {
        let response = &tab.response;
        if response.dragged() || response.drag_stopped() {
            self.held = Some(self.rects.len());
            self.dropped = response.drag_stopped();
        }
        self.rects.push(response.rect);
    }

    /// While a tab is held, a line where it would go; on its release, the
    /// tab's index and the index it takes in the reordered strip, when it
    /// moved.
    pub fn finish(self, ui: &Ui) -> Option<(usize, usize)> {
        let from = self.held?;
        let pointer = ui.ctx().pointer_latest_pos()?;
        let gap = insertion_at(&self.rects, pointer);
        if self.dropped {
            let to = landing(from, gap);
            return (to != from).then_some((from, to));
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if landing(from, gap) != from {
            let x = match self.rects.get(gap) {
                Some(rect) => rect.left() - 1.0,
                None => self.rects.last().map_or(0.0, |r| r.right() + 1.0),
            };
            let row = self.rects.get(gap).or(self.rects.last()).copied();
            if let Some(row) = row {
                ui.painter()
                    .vline(x, row.y_range(), Stroke::new(2.0, ACCENT));
            }
        }
        None
    }
}

/// The gap a pointer is over, `0..=rects.len()`: before the tab whose
/// left half it is on, after the one whose right half, the rows of a
/// wrapped strip told apart by height.
fn insertion_at(rects: &[egui::Rect], pointer: egui::Pos2) -> usize {
    let nearest = rects.iter().enumerate().min_by(|(_, a), (_, b)| {
        let d = |r: &egui::Rect| {
            let dy = (pointer.y - r.center().y).abs() * 4.0;
            let dx = (pointer.x - r.x_range().clamp(pointer.x)).abs();
            dx + dy
        };
        d(a).total_cmp(&d(b))
    });
    match nearest {
        Some((i, rect)) if pointer.x < rect.center().x => i,
        Some((i, _)) => i + 1,
        None => 0,
    }
}

/// Where the tab at `from` ends up when put in gap `gap`.
fn landing(from: usize, gap: usize) -> usize {
    if gap > from { gap - 1 } else { gap }
}

/// The "+" at the end of a tab strip.
pub fn tab_plus(ui: &mut Ui, height: f32) -> Response {
    let (rect, plus) = ui.allocate_exact_size(Vec2::new(26.0, height), Sense::click());
    plus.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), "+"));
    if plus.hovered() {
        ui.painter().rect_filled(rect, RADIUS_SM, BG2);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "+",
        crate::theme::sans_medium(FONT_MD),
        if plus.hovered() { TEXT1 } else { TEXT2 },
    );
    plus
}

#[cfg(test)]
mod tab_reorder_tests {
    use super::{insertion_at, landing};
    use egui::{Rect, pos2, vec2};

    #[test]
    fn a_dropped_tab_lands_in_the_gap_under_the_pointer_on_its_own_row() {
        // Two rows of three 100-wide tabs, as a wrapped strip lays them.
        let rects: Vec<Rect> = (0..6)
            .map(|i| {
                Rect::from_min_size(
                    pos2((i % 3) as f32 * 100.0, (i / 3) as f32 * 30.0),
                    vec2(98.0, 28.0),
                )
            })
            .collect();
        let gap = |x: f32, y: f32| insertion_at(&rects, pos2(x, y));
        assert_eq!(gap(10.0, 14.0), 0, "left half of the first");
        assert_eq!(gap(160.0, 14.0), 2, "right half of the second");
        assert_eq!(gap(500.0, 14.0), 3, "past the end of the first row");
        assert_eq!(gap(10.0, 44.0), 3, "start of the second row");
        assert_eq!(gap(500.0, 44.0), 6, "past the last");
        // The first tab dropped after the third takes the third's place;
        // the last dropped before the first takes the first's.
        assert_eq!(landing(0, 3), 2);
        assert_eq!(landing(5, 0), 0);
        assert_eq!(landing(2, 2), 2, "its own left gap leaves it where it is");
        assert_eq!(landing(2, 3), 2, "and so does its right");
    }
}
