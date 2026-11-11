//! The active bench's floating card: its declared widgets in a card over
//! the view beside the point it gives, drawn as a task panel's are, with
//! OK and Cancel at the bottom. Enter is OK and Escape Cancel while the
//! card has the keys; both close through `UiCommand::CloseCard`, which the
//! host answers with the bench's `CardClose` and the undo step.

use egui::{Align2, Area, Context, Key, Rect, Vec2};
use ui_kit::tokens::*;
use ui_kit::widgets::{Card, small_primary_button, small_secondary_button, text};

use super::host_ctx::{HostCtxParams, PanelWriteback, flush_ctx_logs, panel_ctx};
use crate::ui::ActiveWorkbench;

/// How far the card keeps from the point it is beside, logical pixels.
const GAP: f32 = 16.0;

/// About how wide a card is, to choose the side with room for it.
const WIDTH: f32 = 280.0;

/// The card's widest, logical pixels.
const MAX_WIDTH: f32 = 320.0;

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
}

/// Draw the active bench's card in `viewport` (logical), if it has one.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    ctx: &Context,
    viewport: Rect,
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
    // Below and to the right of the point, or on the side with room.
    let left = at.x + GAP + WIDTH > viewport.right();
    let above = at.y > viewport.center().y;
    let pivot = match (left, above) {
        (false, false) => Align2::LEFT_TOP,
        (true, false) => Align2::RIGHT_TOP,
        (false, true) => Align2::LEFT_BOTTOM,
        (true, true) => Align2::RIGHT_BOTTOM,
    };
    let offset = Vec2::new(
        if left { -GAP } else { GAP },
        if above { -GAP } else { GAP },
    );
    let fresh = state.drawn.as_deref() != Some(card.id.as_str());
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
    let area = Area::new(egui::Id::new("floating_card"))
        .order(super::VIEW_LAYER)
        .pivot(pivot)
        .fixed_pos(at + offset)
        .constrain_to(viewport.shrink(SPACE_2))
        .show(ctx, |ui| {
            Card::floating()
                .border(BORDER_STRONG)
                .padding(SPACE_3)
                .show(ui, |ui| {
                    ui.set_max_width(MAX_WIDTH);
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
                        ui.label(text(why, FONT_XS, DANGER));
                    }
                    ui.add_space(SPACE_1);
                    ui.horizontal(|ui| {
                        ok = ui
                            .add_enabled_ui(!refused, |ui| small_primary_button(ui, ok_label))
                            .inner
                            .clicked();
                        cancel = small_secondary_button(ui, "Cancel").clicked();
                    });
                });
        });
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
