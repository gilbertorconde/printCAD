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

fn corner(viewport: egui::Rect, id: &str, align: Align2, offset: Vec2) -> Area {
    let anchor = match align {
        Align2::LEFT_TOP => viewport.left_top() + offset,
        Align2::RIGHT_TOP => viewport.right_top() + Vec2::new(-offset.x, offset.y),
        Align2::LEFT_BOTTOM => viewport.left_bottom() + Vec2::new(offset.x, -offset.y),
        _ => viewport.right_bottom() - offset,
    };
    Area::new(egui::Id::new(id))
        .order(Order::Foreground)
        .pivot(align)
        .fixed_pos(anchor)
        .interactable(false)
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
            Vec2::new(MARGIN, MARGIN),
        )
        .show(ctx, |ui| {
            Card::floating().padding(6.0).show(ui, |ui| {
                ui.horizontal(|ui| {
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
            Vec2::new(MARGIN, MARGIN),
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
            .order(Order::Foreground)
            .fixed_pos(pos)
            .interactable(false)
            .show(ctx, |ui| {
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

/// Warnings and errors from the last few seconds, as cards under the view
/// toolbar, so a refused action is seen without reading the log.
pub fn draw_toasts(ctx: &Context, viewport: egui::Rect) {
    const SHOW_FOR_SECS: u64 = 6;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
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
        .order(Order::Foreground)
        .fixed_pos(egui::pos2(viewport.center().x, viewport.top() + 52.0))
        .pivot(Align2::CENTER_TOP)
        .interactable(false)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = SPACE_1;
            for entry in recent {
                let (color, icon) = match entry.level {
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
