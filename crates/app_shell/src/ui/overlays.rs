//! Painters for the viewport annotations workbenches emit: constant-width
//! lines, point markers and icon glyphs, text labels, and the orbit pivot.
//! Coordinates arrive in physical pixels relative to the viewport origin.

use egui::{Color32, Context};

pub fn rgb(color: [f32; 3], alpha: f32) -> Color32 {
    Color32::from_rgb(
        (color[0] * 255.0) as u8,
        (color[1] * 255.0) as u8,
        (color[2] * 255.0) as u8,
    )
    .gamma_multiply(alpha)
}

fn viewport_painter(ctx: &Context, viewport_rect: egui::Rect, id: &'static str) -> egui::Painter {
    // Background order draws beneath UI panels and on top of the 3D scene,
    // which is composited separately; clip to the viewport area.
    let layer_id = egui::LayerId::new(egui::Order::Background, egui::Id::new(id));
    ctx.layer_painter(layer_id).with_clip_rect(viewport_rect)
}

/// Draw constant-thickness lines in the viewport area.
pub fn draw_screen_space_overlays(
    ctx: &Context,
    viewport_rect: egui::Rect,
    overlays: &[core_document::ScreenSpaceOverlay],
) {
    if overlays.is_empty() {
        return;
    }
    let ppp = ctx.pixels_per_point();
    let painter = viewport_painter(ctx, viewport_rect, "screen_space_overlays");
    for overlay in overlays {
        let start = egui::pos2(
            viewport_rect.min.x + overlay.start[0] / ppp,
            viewport_rect.min.y + overlay.start[1] / ppp,
        );
        let end = egui::pos2(
            viewport_rect.min.x + overlay.end[0] / ppp,
            viewport_rect.min.y + overlay.end[1] / ppp,
        );
        let stroke = egui::Stroke::new(overlay.thickness / ppp, rgb(overlay.color, overlay.alpha));
        match overlay.dash {
            Some((dash, gap)) => {
                painter.add(egui::Shape::dashed_line(
                    &[start, end],
                    stroke,
                    dash / ppp,
                    gap / ppp,
                ));
            }
            None => {
                painter.line_segment([start, end], stroke);
            }
        }
    }
}

/// Draw point markers and icon glyphs, above the lines and beneath the
/// labels.
pub fn draw_screen_space_marks(
    ctx: &Context,
    viewport_rect: egui::Rect,
    marks: &[core_document::ScreenSpaceMark],
) {
    if marks.is_empty() {
        return;
    }
    let ppp = ctx.pixels_per_point();
    let painter = viewport_painter(ctx, viewport_rect, "screen_space_marks");
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    for mark in marks {
        let pos = egui::pos2(
            viewport_rect.min.x + mark.pos[0] / ppp,
            viewport_rect.min.y + mark.pos[1] / ppp,
        );
        let color = rgb(mark.color, mark.alpha);
        match mark.kind {
            core_document::MarkKind::Dot { radius } => {
                painter.circle_filled(pos, radius / ppp, color);
            }
            core_document::MarkKind::Crosshair { size } => {
                let h = size / ppp / 2.0;
                let stroke = egui::Stroke::new(1.0, color);
                painter.line_segment(
                    [egui::pos2(pos.x - h, pos.y), egui::pos2(pos.x + h, pos.y)],
                    stroke,
                );
                painter.line_segment(
                    [egui::pos2(pos.x, pos.y - h), egui::pos2(pos.x, pos.y + h)],
                    stroke,
                );
            }
            core_document::MarkKind::Icon { name, size } => {
                if let Some(tex) = ui_kit::icon::texture(ctx, name) {
                    let rect = egui::Rect::from_center_size(pos, egui::Vec2::splat(size / ppp));
                    painter.image(tex.id(), rect, uv, color);
                }
            }
        }
    }
}

/// Draw text labels: dimension values, constraint suffixes, readouts.
pub fn draw_screen_space_labels(
    ctx: &Context,
    viewport_rect: egui::Rect,
    labels: &[core_document::ScreenSpaceLabel],
) {
    if labels.is_empty() {
        return;
    }
    let ppp = ctx.pixels_per_point();
    let painter = viewport_painter(ctx, viewport_rect, "screen_space_labels");
    for label in labels {
        let pos = egui::pos2(
            viewport_rect.min.x + label.pos[0] / ppp,
            viewport_rect.min.y + label.pos[1] / ppp,
        );
        let color = rgb(label.color, 1.0);
        let font = if label.mono {
            ui_kit::mono(label.size / ppp)
        } else {
            ui_kit::sans(label.size / ppp)
        };
        let galley = painter.layout_no_wrap(label.text.clone(), font, color);
        let rect = egui::Rect::from_center_size(pos, galley.size());
        if label.background {
            painter.rect_filled(
                rect.expand2(egui::Vec2::from(ui_kit::tokens::PILL_PAD) / ppp),
                ui_kit::tokens::RADIUS_SM,
                ui_kit::tokens::BG0,
            );
        }
        painter.galley(rect.min, galley, color);
    }
}

/// The orbit pivot marker: a small ringed dot where the next orbit turns.
pub fn draw_pivot_indicator(ctx: &Context, x: f32, y: f32) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("pivot_indicator"),
    ));
    let ppp = ctx.pixels_per_point();
    let pos = egui::pos2(x / ppp, y / ppp);
    let accent = ui_kit::tokens::ACCENT;
    painter.circle(
        pos,
        8.0,
        ui_kit::tokens::ACCENT_DIM,
        egui::Stroke::new(1.5, accent),
    );
    let cross = 4.0;
    let stroke = egui::Stroke::new(1.5, ui_kit::tokens::TEXT1);
    painter.line_segment(
        [
            egui::pos2(pos.x - cross, pos.y),
            egui::pos2(pos.x + cross, pos.y),
        ],
        stroke,
    );
    painter.line_segment(
        [
            egui::pos2(pos.x, pos.y - cross),
            egui::pos2(pos.x, pos.y + cross),
        ],
        stroke,
    );
}
