//! The active bench's floating card: its declared widgets in a card over
//! the view beside the point it gives, drawn as a task panel's are, with
//! OK and Cancel at the bottom. It is opaque and above everything a bench
//! draws on the view, and stands on the side of its point where the
//! least of that drawing lies, so it hides little of what it edits. Enter is OK and Escape Cancel while the
//! card has the keys; both close through `UiCommand::CloseCard`, which the
//! host answers with the bench's `CardClose` and the undo step.

use egui::{Area, Context, Key, Pos2, Rect, Vec2};
use ui_kit::tokens::*;
use ui_kit::widgets::{Card, Note, note_card, primary_button, secondary_button};

use super::host_ctx::{HostCtxParams, PanelWriteback, flush_ctx_logs, panel_ctx};
use crate::ui::ActiveWorkbench;

/// How far the card keeps from the point it is beside, logical pixels.
const GAP: f32 = 16.0;

/// About how large a card is before it has been drawn once.
const SIZE_GUESS: Vec2 = Vec2::new(240.0, 140.0);

/// The card's widest, logical pixels.
const MAX_WIDTH: f32 = 260.0;

/// How many frames a new card may still change side: its first one is
/// placed by [`SIZE_GUESS`], the next by the size it was drawn at.
const SETTLE_FRAMES: u8 = 3;

/// What the bench draws on the view this frame, in physical pixels from
/// the viewport's corner: what the card keeps clear of.
#[derive(Clone, Copy)]
pub struct Drawn<'a> {
    pub lines: &'a [core_document::ScreenSpaceOverlay],
    pub marks: &'a [core_document::ScreenSpaceMark],
    pub labels: &'a [core_document::ScreenSpaceLabel],
}

/// A side of the point the card can stand on, in the order tried when
/// the drawing gives no reason to prefer one: x and y are -1, 0 or 1.
const SIDES: [(i8, i8); 8] = [
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
];

/// A card on screen this frame: its id, whether its OK is held back, and
/// what its widgets left for the host.
pub struct Shown {
    pub id: String,
    pub refused: bool,
    pub writeback: PanelWriteback,
    /// OK (true) or Cancel this frame, for after the writeback's requests.
    pub close: Option<bool>,
}

/// What the card keeps between frames: UI-local.
#[derive(Default)]
pub struct CardState {
    /// The card drawn last frame: a new id gives its field the keys.
    drawn: Option<String>,
    /// The widget inside the card that had the keys as last frame ended.
    focus: Option<egui::Id>,
    /// The side of its point the card stands on, and the frames it has
    /// been drawn: it keeps its side once settled, so a preview that
    /// moves the drawing does not move it.
    side: Option<(i8, i8)>,
    frames: u8,
}

/// The card's rect of `size` on `side` of `at`, inside `room`.
fn placed(at: Pos2, size: Vec2, side: (i8, i8), room: Rect) -> Rect {
    let along = |s: i8, at: f32, len: f32| match s {
        1 => at + GAP,
        -1 => at - GAP - len,
        _ => at - len / 2.0,
    };
    let min = Pos2::new(along(side.0, at.x, size.x), along(side.1, at.y, size.y));
    let max = (room.max - size).max(room.min);
    Rect::from_min_size(min.clamp(room.min, max), size)
}

/// Length of segment `a`-`b` inside `rect`.
fn length_inside(rect: Rect, a: Pos2, b: Pos2) -> f32 {
    let d = b - a;
    let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
    for (p, q) in [
        (-d.x, a.x - rect.min.x),
        (d.x, rect.max.x - a.x),
        (-d.y, a.y - rect.min.y),
        (d.y, rect.max.y - a.y),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return 0.0;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    if t1 > t0 { (t1 - t0) * d.length() } else { 0.0 }
}

/// How much of the bench's drawing `rect` would hide: the length of
/// strong lines under it, and its marks and labels, each label counted
/// as a long line since it is what the card most often edits.
fn hidden(rect: Rect, drawn: Drawn<'_>, origin: Pos2, ppp: f32) -> f32 {
    let at = |p: [f32; 2]| origin + Vec2::new(p[0], p[1]) / ppp;
    let lines: f32 = drawn
        .lines
        .iter()
        .filter(|l| l.alpha >= 0.3)
        .map(|l| length_inside(rect, at(l.start), at(l.end)) * l.alpha)
        .sum();
    let marks = drawn
        .marks
        .iter()
        .filter(|m| rect.contains(at(m.pos)))
        .count() as f32;
    let labels = drawn
        .labels
        .iter()
        .filter(|l| {
            let size = l.size / ppp;
            let half = Vec2::new(0.3 * size * l.text.chars().count() as f32, 0.5 * size);
            rect.intersects(Rect::from_center_size(
                at(l.pos),
                2.0 * half + Vec2::splat(SPACE_2),
            ))
        })
        .count() as f32;
    lines + 20.0 * marks + 120.0 * labels
}

/// The side of `at` where a card of `size` hides the least of `drawn`,
/// never one where it would cover `at` itself.
fn best_side(
    at: Pos2,
    size: Vec2,
    room: Rect,
    drawn: Drawn<'_>,
    origin: Pos2,
    ppp: f32,
) -> (i8, i8) {
    let clear_of_point = |rect: Rect| !rect.expand(GAP / 2.0).contains(at);
    SIDES
        .iter()
        .enumerate()
        .filter_map(|(order, &side)| {
            let rect = placed(at, size, side, room);
            clear_of_point(rect).then(|| {
                // Pushed back inside the view, it strays from its point.
                let ideal = placed(at, size, side, Rect::EVERYTHING);
                let strayed = (rect.center() - ideal.center()).length();
                let cost = hidden(rect, drawn, origin, ppp) + 0.5 * strayed + order as f32;
                (cost, side)
            })
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map_or(SIDES[0], |(_, side)| side)
}

/// Draw the active bench's card in `viewport` (logical), if it has one.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    ctx: &Context,
    viewport: Rect,
    drawn: Drawn<'_>,
    state: &mut CardState,
    active_workbench: &ActiveWorkbench,
    document: &mut core_document::Document,
    registry: &mut core_document::DocumentService,
    host: &HostCtxParams,
    active_document_object: Option<core_document::FeatureId>,
) -> Option<Shown> {
    let wb = registry.workbench_mut(&active_workbench.0).ok()?;
    let mut pctx = panel_ctx(document, host, active_document_object);
    let Some(card) = wb.floating_card(&pctx) else {
        *state = CardState::default();
        return None;
    };
    let ppp = host.pixels_per_point.max(0.1);
    let point = card
        .at
        .and_then(|at| pctx.world_to_viewport(at))
        .map(|(x, y)| [x, y])
        .or(card.screen);
    let at = point.map_or(viewport.center(), |[x, y]| {
        viewport.min + Vec2::new(x, y) / ppp
    });
    let fresh = state.drawn.as_deref() != Some(card.id.as_str());
    if fresh {
        state.side = None;
        state.frames = 0;
    }
    let area_id = egui::Id::new(("floating_card", &card.id));
    let size = ctx
        .memory(|m| m.area_rect(area_id))
        .map_or(SIZE_GUESS, |r| r.size());
    let room = viewport.shrink(SPACE_2);
    let kept = state
        .side
        .filter(|_| state.frames >= SETTLE_FRAMES)
        .filter(|&side| !placed(at, size, side, room).expand(GAP / 2.0).contains(at));
    let side = kept.unwrap_or_else(|| best_side(at, size, room, drawn, viewport.min, ppp));
    state.side = Some(side);
    state.frames = state.frames.saturating_add(1);
    let rect = placed(at, size, side, room);
    let focus_before = ctx.memory(|m| m.focused());
    // The card has the keys when nothing else does, or one of its fields
    // did as the last frame ended.
    let keys = focus_before.is_none() || (!fresh && focus_before == state.focus);
    let panel_id = egui::Id::new(("floating_card", &active_workbench.0, &card.id));
    let refused = card.refusal.is_some();
    let ok_label = if card.ok.is_empty() { "OK" } else { &card.ok };
    let mut out = None;
    let mut ok = false;
    let mut cancel = false;
    let area = Area::new(area_id)
        .order(super::VIEW_LAYER)
        .fixed_pos(rect.min)
        .constrain_to(room)
        .show(ctx, |ui| {
            // Opaque: the drawing it stands over never shows through.
            Card::floating()
                .fill(BG1)
                .border(BORDER_STRONG)
                .show(ui, |ui| {
                    ui.set_max_width(MAX_WIDTH);
                    ui.spacing_mut().item_spacing.y = SPACE_1;
                    ui.horizontal(|ui| {
                        if let Some(icon) = &card.icon {
                            ui_kit::icon::draw(ui, icon, 14.0, TEXT2);
                        }
                        ui.label(
                            egui::RichText::new(&card.title)
                                .font(ui_kit::sans_medium(FONT_SM))
                                .color(TEXT1),
                        );
                    });
                    let shown = core_document::panel::show(
                        ui,
                        panel_id,
                        &card.widgets,
                        Some(&*pctx.document),
                    );
                    if fresh && let Some(field) = &card.focus {
                        core_document::panel::focus_entry(ui, panel_id, &card.widgets, field);
                    }
                    out = Some(shown);
                    if let Some(why) = &card.refusal {
                        note_card(ui, Note::Warning, None, why);
                    }
                    ui.add_space(SPACE_1);
                    ui.horizontal(|ui| {
                        ok = ui
                            .add_enabled_ui(!refused, |ui| primary_button(ui, ok_label))
                            .inner
                            .clicked();
                        cancel = secondary_button(ui, "Cancel").clicked();
                    });
                });
        });
    // Above the view's other furniture on its layer, whichever was last
    // brought forward; the bench's drawing is on the layer below.
    ctx.move_to_top(area.response.layer_id);
    let (enter, escape) = ctx.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
    if let Some(out) = out {
        let by_formula = out.write_formulas(&mut pctx);
        for event in out.events_besides(&by_formula) {
            wb.card_event(&mut pctx, event);
        }
    }
    let close = if cancel || (keys && escape) {
        Some(false)
    } else {
        (!refused && (ok || (keys && enter))).then_some(true)
    };
    state.drawn = Some(card.id.clone());
    // Whether the widget with the keys lies in the card.
    let card_rect = area.response.rect;
    state.focus = ctx.memory(|m| m.focused()).filter(|id| {
        ctx.read_response(*id)
            .is_some_and(|r| card_rect.contains_rect(r.rect))
    });
    let writeback = PanelWriteback::take(&mut pctx, active_document_object);
    flush_ctx_logs(&mut pctx);
    Some(Shown {
        id: card.id,
        refused,
        writeback,
        close,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(start: [f32; 2], end: [f32; 2]) -> core_document::ScreenSpaceOverlay {
        core_document::ScreenSpaceOverlay {
            start,
            end,
            color: [1.0; 3],
            thickness: 1.0,
            alpha: 1.0,
            dash: None,
            dash_offset: 0.0,
        }
    }

    /// The card takes the side of its point where nothing is drawn, stays
    /// in the view, and never covers the point, even in a corner.
    #[test]
    fn a_card_stands_clear_of_the_drawing_and_its_point() {
        let room = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let size = Vec2::new(200.0, 120.0);
        // A box below and to the right of the point, as a dimension's
        // label above the rectangle it measures.
        let lines = [
            line([400.0, 320.0], [700.0, 320.0]),
            line([700.0, 320.0], [700.0, 500.0]),
            line([700.0, 500.0], [400.0, 500.0]),
            line([400.0, 500.0], [400.0, 320.0]),
            line([450.0, 360.0], [650.0, 460.0]),
        ];
        let drawn = Drawn {
            lines: &lines,
            marks: &[],
            labels: &[],
        };
        let at = Pos2::new(550.0, 300.0);
        let side = best_side(at, size, room, drawn, Pos2::ZERO, 1.0);
        let rect = placed(at, size, side, room);
        assert_eq!(hidden(rect, drawn, Pos2::ZERO, 1.0), 0.0, "{side:?}");
        for at in [
            Pos2::new(795.0, 595.0),
            Pos2::new(5.0, 5.0),
            Pos2::new(400.0, 300.0),
        ] {
            let empty = Drawn {
                lines: &[],
                marks: &[],
                labels: &[],
            };
            let rect = placed(
                at,
                size,
                best_side(at, size, room, empty, Pos2::ZERO, 1.0),
                room,
            );
            assert!(room.contains_rect(rect) && !rect.contains(at), "{at:?}");
        }
    }
}
