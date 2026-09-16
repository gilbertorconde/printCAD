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
                                egui::Frame::new()
                                    .fill(BG2)
                                    .stroke(egui::Stroke::new(1.0, border))
                                    .corner_radius(4)
                                    .inner_margin(egui::Margin::symmetric(8, 0))
                                    .show(ui, |ui| {
                                        ui.set_min_size(Vec2::new(80.0, INPUT - 4.0));
                                        ui.centered_and_justified(|ui| {
                                            let text = if row.unit.is_empty() {
                                                row.value.clone()
                                            } else {
                                                format!("{} {}", row.value, row.unit)
                                            };
                                            ui.label(
                                                RichText::new(text)
                                                    .font(mono(FONT_SM))
                                                    .color(TEXT1),
                                            );
                                        });
                                    });
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
