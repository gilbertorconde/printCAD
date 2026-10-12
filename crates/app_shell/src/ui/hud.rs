//! Widgets drawn over the viewport from a workbench's [`ViewportHud`]: the
//! tool hint, a state badge, the legend, mono readouts and the on-view
//! parameter card beside the cursor.

use core_document::ViewportHud;
use egui::{Align2, Area, Context, Order, RichText, Vec2};
use ui_kit::tokens::*;
use ui_kit::widgets::{Card, key_chip, mono_label};
use ui_kit::{mono, sans, sans_medium};

use super::overlays::rgb;

const MARGIN: f32 = 12.0;

/// The layer of what floats on the view (its toolbar, cards and cube):
/// above the panels, below dialogs, menus and their pop-ups, which share
/// egui's foreground layer, so a dialog is never drawn under the view's
/// furniture whichever of them was last brought forward.
pub(crate) const VIEW_LAYER: Order = Order::Middle;

fn corner(viewport: egui::Rect, id: &str, align: Align2, offset: Vec2) -> Area {
    let anchor = match align {
        Align2::LEFT_TOP => viewport.left_top() + offset,
        Align2::RIGHT_TOP => viewport.right_top() + Vec2::new(-offset.x, offset.y),
        Align2::LEFT_BOTTOM => viewport.left_bottom() + Vec2::new(offset.x, -offset.y),
        _ => viewport.right_bottom() - offset,
    };
    Area::new(egui::Id::new(id))
        .order(VIEW_LAYER)
        .pivot(align)
        .fixed_pos(anchor)
        .interactable(false)
}

/// Where a top-corner card of the id `id` stands: in the corner, or, when
/// it would reach the view toolbar across the top, just below the toolbar.
/// Both sizes are the ones drawn last frame.
fn top_offset(ctx: &Context, viewport: egui::Rect, id: &str, left: bool) -> Vec2 {
    let rect_of = |id: &str| ctx.memory(|m| m.area_rect(egui::Id::new(id)));
    let (Some(bar), Some(card)) = (rect_of("view_toolbar"), rect_of(id)) else {
        return Vec2::new(MARGIN, MARGIN);
    };
    let clear = if left {
        viewport.left() + MARGIN + card.width() + MARGIN <= bar.left()
    } else {
        viewport.right() - MARGIN - card.width() - MARGIN >= bar.right()
    };
    if clear {
        Vec2::new(MARGIN, MARGIN)
    } else {
        Vec2::new(MARGIN, bar.bottom() - viewport.top() + SPACE_2)
    }
}

/// Draw every part of `hud` that is present. `footer_extra` are readouts
/// the shell contributes to the bottom-right line (projection, draw style).
pub fn draw_viewport_hud(
    ctx: &Context,
    viewport: egui::Rect,
    hud: Option<&ViewportHud>,
    footer_extra: &[String],
) {
    let empty = ViewportHud::default();
    let hud = hud.unwrap_or(&empty);

    if let Some(tool) = &hud.tool {
        corner(
            viewport,
            "hud_tool_hint",
            Align2::LEFT_TOP,
            top_offset(ctx, viewport, "hud_tool_hint", true),
        )
        .show(ctx, |ui| {
            // A narrow viewport wraps the keys onto a second line rather
            // than running the card past its edge.
            ui.set_max_width(viewport.width() - 2.0 * MARGIN);
            Card::floating().padding(6.0).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_2;
                    ui_kit::icon::draw(ui, tool.icon, 16.0, ACCENT);
                    ui.label(
                        RichText::new(&tool.name)
                            .font(sans_medium(FONT_SM))
                            .color(TEXT1),
                    );
                    if !tool.prompt.is_empty() {
                        ui.label(RichText::new(&tool.prompt).font(sans(FONT_SM)).color(TEXT3));
                    }
                    for (key, meaning) in &tool.keys {
                        key_chip(ui, key);
                        ui.label(RichText::new(*meaning).font(sans(FONT_XS)).color(TEXT3));
                    }
                });
            });
        });
    }

    if let Some((color, text)) = &hud.badge {
        let color = rgb(*color, 1.0);
        corner(
            viewport,
            "hud_badge",
            Align2::RIGHT_TOP,
            top_offset(ctx, viewport, "hud_badge", false),
        )
        .show(ctx, |ui| {
            Card::floating()
                .border(with_alpha(color, 0.5))
                .padding(6.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_2;
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.0, color);
                        mono_label(ui, text, FONT_SM, TEXT1);
                    });
                });
        });
    }

    if !hud.legend.is_empty() {
        corner(
            viewport,
            "hud_legend",
            Align2::LEFT_BOTTOM,
            Vec2::new(MARGIN + 4.0, MARGIN),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_3;
                for (color, label) in &hud.legend {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(10.0, 2.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 0.0, rgb(*color, 1.0));
                        ui.label(RichText::new(*label).font(sans(FONT_XS)).color(TEXT3));
                    });
                }
            });
        });
    }

    let footer: Vec<&str> = hud
        .footer
        .iter()
        .chain(footer_extra.iter())
        .map(String::as_str)
        .collect();
    if !footer.is_empty() {
        corner(
            viewport,
            "hud_footer",
            Align2::RIGHT_BOTTOM,
            Vec2::new(MARGIN + 4.0, MARGIN),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                for item in footer {
                    mono_label(ui, item, FONT_XS, TEXT3);
                }
            });
        });
    }

    if let Some(ovp) = &hud.ovp {
        let ppp = ctx.pixels_per_point();
        let pos = egui::pos2(
            viewport.min.x + ovp.anchor[0] / ppp,
            viewport.min.y + ovp.anchor[1] / ppp,
        );
        Area::new(egui::Id::new("hud_ovp"))
            .order(VIEW_LAYER)
            .fixed_pos(pos)
            .interactable(false)
            .show(ctx, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                Card::floating()
                    .border(BORDER_STRONG)
                    .padding(6.0)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = SPACE_1;
                        for row in &ovp.rows {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                ui.add_sized(
                                    Vec2::new(38.0, INPUT - 2.0),
                                    egui::Label::new(
                                        RichText::new(row.label).font(mono(FONT_XS)).color(TEXT2),
                                    ),
                                );
                                let border = if row.focused { ACCENT } else { BORDER };
                                // A fixed footprint: a box that sized itself to
                                // the card would feed the card's width back
                                // into itself every frame.
                                let (rect, _) = ui.allocate_exact_size(
                                    Vec2::new(96.0, INPUT - 4.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect(
                                    rect,
                                    4.0,
                                    BG2,
                                    egui::Stroke::new(1.0, border),
                                    egui::StrokeKind::Inside,
                                );
                                let text = if row.unit.is_empty() {
                                    row.value.clone()
                                } else {
                                    format!("{} {}", row.value, row.unit)
                                };
                                ui.painter().text(
                                    egui::pos2(rect.right() - 8.0, rect.center().y),
                                    Align2::RIGHT_CENTER,
                                    text,
                                    mono(FONT_SM),
                                    TEXT1,
                                );
                                let (rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(14.0), egui::Sense::hover());
                                let fill = if row.locked { ACCENT } else { BG4 };
                                ui.painter().rect_filled(rect, RADIUS_SM, fill);
                            });
                        }
                        if !ovp.hint.is_empty() {
                            ui.label(RichText::new(ovp.hint).font(sans(10.0)).color(TEXT3));
                        }
                    });
            });
    }
}

/// The card that names what sits under the cursor, beside the pointer.
pub fn draw_hover_card(
    ctx: &Context,
    viewport: egui::Rect,
    card: &super::HoverCard,
    unit: core_document::Unit,
) {
    let Some(pointer) = ctx.pointer_hover_pos() else {
        return;
    };
    if !viewport.contains(pointer) {
        return;
    }
    let pos =
        (pointer + Vec2::new(14.0, 14.0)).min(viewport.right_bottom() - Vec2::new(200.0, 48.0));
    let [x, y, z] = card.point_mm;
    let coords = format!(
        "({}, {}, {})",
        core_document::format_length_mm(x, unit, 2),
        core_document::format_length_mm(y, unit, 2),
        core_document::format_length_mm(z, unit, 2)
    );
    Area::new(egui::Id::new("hud_hover_card"))
        .order(Order::Tooltip)
        .fixed_pos(pos)
        .interactable(false)
        .show(ctx, |ui| {
            // Each line on one line: an area near the view's edge would
            // otherwise wrap the card to a sliver.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            Card::floating().padding(6.0).radius(5.0).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(
                    RichText::new(&card.title)
                        .font(sans_medium(FONT_XS))
                        .color(TEXT1),
                );
                mono_label(ui, coords, FONT_XS, TEXT3);
            });
        });
}

/// The card saying printCAD `tag` is out, at the view's bottom right, with
/// the release's `page`. Whether Details or the close button was pressed.
pub fn draw_release_notice(
    ctx: &Context,
    viewport: egui::Rect,
    tag: &str,
    page: &str,
) -> (bool, bool) {
    let mut details = false;
    let mut dismiss = false;
    Area::new(egui::Id::new("release_notice"))
        .order(VIEW_LAYER)
        .pivot(Align2::RIGHT_BOTTOM)
        .fixed_pos(viewport.right_bottom() - Vec2::splat(SPACE_4))
        .show(ctx, |ui| {
            Card::floating()
                .border(with_alpha(SUCCESS, 0.6))
                .padding(SPACE_3)
                .show(ui, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_2;
                        ui_kit::icon::draw(ui, "info", 16.0, SUCCESS);
                        ui.label(
                            RichText::new(format!("printCAD {tag} is out"))
                                .font(sans_medium(FONT_SM))
                                .color(TEXT1),
                        );
                        ui.add_space(SPACE_2);
                        ui.hyperlink_to(RichText::new("Get it").font(ui_kit::sans(FONT_SM)), page);
                        details = ui_kit::widgets::small_secondary_button(ui, "Details")
                            .on_hover_text("Preferences › Updates")
                            .clicked();
                        dismiss = ui
                            .add(egui::Button::new(RichText::new("×").color(TEXT3)).frame(false))
                            .on_hover_text("Put the notice away")
                            .clicked();
                    });
                });
        });
    (details, dismiss)
}

/// The card saying a newer printCAD wrote the document's file, at the
/// view's bottom right `lift` above its corner, with `text` and a Save As.
/// Whether Save As or the close button was pressed.
pub fn draw_file_notice(
    ctx: &Context,
    viewport: egui::Rect,
    lift: f32,
    text: &str,
) -> (bool, bool) {
    let mut save_as = false;
    let mut dismiss = false;
    Area::new(egui::Id::new("file_notice"))
        .order(VIEW_LAYER)
        .pivot(Align2::RIGHT_BOTTOM)
        .fixed_pos(viewport.right_bottom() - Vec2::new(SPACE_4, SPACE_4 + lift))
        .show(ctx, |ui| {
            Card::floating()
                .border(with_alpha(WARNING, 0.6))
                .padding(SPACE_3)
                .show(ui, |ui| {
                    ui.set_max_width(420.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_2;
                        ui_kit::icon::draw(ui, "warning", 16.0, WARNING);
                        ui.add(
                            egui::Label::new(
                                RichText::new(text).font(ui_kit::sans(FONT_SM)).color(TEXT1),
                            )
                            .wrap(),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_2;
                        save_as = ui_kit::widgets::small_secondary_button(ui, "Save As…")
                            .on_hover_text("Keep the file as it is and save under another name")
                            .clicked();
                        dismiss = ui
                            .add(
                                egui::Button::new(RichText::new("Put away").color(TEXT3))
                                    .frame(false),
                            )
                            .clicked();
                    });
                });
        });
    (save_as, dismiss)
}

/// Good news, warnings and errors from the last few seconds, as cards under
/// the view toolbar, so they are seen without reading the log. They draw
/// over dialogs too: an install finishing in Preferences shows there.
pub fn draw_toasts(ctx: &Context, viewport: egui::Rect) {
    const SHOW_FOR_SECS: u64 = 6;
    let now = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let recent: Vec<crate::log_panel::LogEntry> = crate::log_panel::entries()
        .into_iter()
        .rev()
        .filter(|e| {
            !matches!(e.level, crate::log_panel::LogLevel::Info)
                && now.saturating_sub(e.timestamp_secs) < SHOW_FOR_SECS
        })
        .take(3)
        .collect();
    if recent.is_empty() {
        return;
    }
    // The card has to go away on its own, without waiting for input.
    ctx.request_repaint_after(std::time::Duration::from_millis(500));
    Area::new(egui::Id::new("hud_toasts"))
        .order(Order::Tooltip)
        .fixed_pos(egui::pos2(viewport.center().x, viewport.top() + 52.0))
        .pivot(Align2::CENTER_TOP)
        .interactable(false)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = SPACE_1;
            for entry in recent {
                let (color, icon) = match entry.level {
                    crate::log_panel::LogLevel::Success => (SUCCESS, "check"),
                    crate::log_panel::LogLevel::Warn => (WARNING, "warning"),
                    _ => (DANGER, "error"),
                };
                Card::floating()
                    .border(with_alpha(color, 0.6))
                    .padding(SPACE_2)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = SPACE_2;
                            ui_kit::icon::draw(ui, icon, 16.0, color);
                            ui.label(
                                RichText::new(&entry.message)
                                    .font(sans_medium(FONT_SM))
                                    .color(TEXT1),
                            );
                        });
                    });
            }
        });
}

#[cfg(test)]
mod toast_tests {
    use super::*;

    #[test]
    fn a_newer_release_draws_as_a_card_that_stays() {
        let ctx = Context::default();
        ui_kit::theme::apply_theme(&ctx);
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        for time in [0.0, 60.0] {
            let input = egui::RawInput {
                time: Some(time),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let (details, dismiss) =
                    draw_release_notice(ui.ctx(), viewport, "v9.0.0", "https://example.org");
                assert!(!details && !dismiss);
            });
            output.textures_delta.clear();
            let card = ctx.memory(|m| m.area_rect(egui::Id::new("release_notice")));
            let card = card.expect("the notice is on screen");
            assert!(viewport.contains_rect(card), "inside the view: {card:?}");
        }
    }

    /// The card for a file from a newer version stands inside the view,
    /// above the release card when both are up.
    #[test]
    fn a_newer_file_draws_as_a_card_above_the_release_card() {
        let ctx = Context::default();
        ui_kit::theme::apply_theme(&ctx);
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let text = "Written by printCAD 9.0.0, newer than this one";
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                draw_release_notice(ui.ctx(), viewport, "v9.0.0", "https://example.org");
                let lift = ui
                    .ctx()
                    .memory(|m| m.area_rect(egui::Id::new("release_notice")))
                    .map_or(0.0, |card| card.height() + SPACE_2);
                let (save_as, dismiss) = draw_file_notice(ui.ctx(), viewport, lift, text);
                assert!(!save_as && !dismiss);
            });
            output.textures_delta.clear();
        }
        let release = ctx.memory(|m| m.area_rect(egui::Id::new("release_notice")));
        let file = ctx.memory(|m| m.area_rect(egui::Id::new("file_notice")));
        let (release, file) = (release.expect("release card"), file.expect("file card"));
        assert!(viewport.contains_rect(file), "inside the view: {file:?}");
        assert!(file.bottom() <= release.top(), "{file:?} above {release:?}");
    }

    #[test]
    fn good_news_draws_as_a_card_over_the_view() {
        crate::log_panel::success("A package is ready");
        let ctx = Context::default();
        ui_kit::theme::apply_theme(&ctx);
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            draw_toasts(ui.ctx(), viewport);
        });
        output.textures_delta.clear();
        let card = ctx.memory(|m| m.area_rect(egui::Id::new("hud_toasts")));
        assert!(card.is_some(), "the notice is on screen");
        let card = card.unwrap();
        assert!(
            card.center().x > 300.0 && card.center().x < 500.0,
            "centred: {card:?}"
        );
        assert!(card.top() < 100.0, "at the top: {card:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::ToolHint;

    /// A pill the view toolbar's size, where it stands, under its id.
    fn stand_in_toolbar(ctx: &Context, viewport: egui::Rect) {
        Area::new(egui::Id::new("view_toolbar"))
            .order(VIEW_LAYER)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(viewport.center_top() + Vec2::new(0.0, 10.0))
            .show(ctx, |ui| {
                ui.allocate_exact_size(Vec2::new(560.0, 36.0), egui::Sense::hover());
            });
    }

    fn hud() -> ViewportHud {
        ViewportHud {
            tool: Some(ToolHint {
                icon: "select",
                name: "Select".into(),
                prompt: "Click to select · drag for a box".into(),
                keys: vec![
                    ("Click".into(), "add to selection"),
                    ("Empty click".into(), "clear"),
                    ("Del".into(), "delete"),
                ],
            }),
            badge: Some(([0.3, 0.8, 0.5], "Fully constrained".into())),
            ..Default::default()
        }
    }

    /// Where the hint and the badge stand once the layout has settled.
    fn settle(width: f32) -> (egui::Rect, egui::Rect, egui::Rect) {
        let ctx = Context::default();
        ui_kit::theme::apply_theme(&ctx);
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 40.0), Vec2::new(width, 700.0));
        let hud = hud();
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width, 800.0),
                )),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                stand_in_toolbar(ui.ctx(), viewport);
                draw_viewport_hud(ui.ctx(), viewport, Some(&hud), &[]);
            });
            out.textures_delta.clear();
        }
        let rect = |id: &str| ctx.memory(|m| m.area_rect(egui::Id::new(id))).unwrap();
        (
            rect("view_toolbar"),
            rect("hud_tool_hint"),
            rect("hud_badge"),
        )
    }

    #[test]
    fn corner_cards_stay_clear_of_the_view_toolbar() {
        // Wide: both in their corners, level with the toolbar.
        let (bar, hint, badge) = settle(2400.0);
        assert!(!hint.intersects(bar) && !badge.intersects(bar));
        assert!(hint.top() < bar.bottom(), "the hint stays in its corner");
        // Narrow: the hint would reach the toolbar, so it stands below it,
        // and neither runs past the viewport.
        let (bar, hint, badge) = settle(1000.0);
        assert!(!hint.intersects(bar), "{hint:?} against {bar:?}");
        assert!(!badge.intersects(bar), "{badge:?} against {bar:?}");
        assert!(hint.top() >= bar.bottom());
        assert!(hint.right() <= 1000.0 && badge.left() >= 0.0);
    }
}
