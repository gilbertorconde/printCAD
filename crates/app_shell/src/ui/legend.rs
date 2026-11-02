//! The active scalar map's scale and measured distribution.

use core_document::ScalarMap;
use egui::{Align2, Context, Rect, RichText, Sense, Vec2, pos2};
use ui_kit::tokens::*;
use ui_kit::widgets::{Card, mono_label};
use ui_kit::{mono, sans_medium};

pub(crate) fn draw(ctx: &Context, viewport: Rect, map: &ScalarMap, lift: f32) {
    egui::Area::new(egui::Id::new("scalar_legend"))
        .order(super::VIEW_LAYER)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(viewport.left_bottom() + Vec2::new(SPACE_3, -SPACE_3 - lift))
        .interactable(false)
        .show(ctx, |ui| {
            ui.set_max_width((viewport.width() - SPACE_5).max(SPACE_6));
            Card::floating().padding(SPACE_2).show(ui, |ui| {
                ui.label(
                    RichText::new(&map.legend.title)
                        .font(sans_medium(FONT_SM))
                        .color(TEXT1),
                );
                scale(ui, map);
                for (name, value) in &map.legend.stats {
                    mono_label(ui, format!("{name}: {value}"), FONT_XS, TEXT2);
                }
            });
        });
}

fn scale(ui: &mut egui::Ui, map: &ScalarMap) {
    let height = SPACE_6 * 5.0;
    let histogram_width = SPACE_6 * 2.0;
    let labels: Vec<_> = [map.range[0], 0.0, map.range[1]]
        .into_iter()
        .filter(|v| *v >= map.range[0] && *v <= map.range[1])
        .map(|v| {
            (
                v,
                ui.painter().layout_no_wrap(
                    format!("{v:.3} {}", map.legend.unit),
                    mono(FONT_XS),
                    TEXT2,
                ),
            )
        })
        .collect();
    let label_width = labels
        .iter()
        .map(|(_, text)| text.size().x)
        .fold(0.0, f32::max);
    let (area, _) = ui.allocate_exact_size(
        Vec2::new(
            SPACE_4 + SPACE_2 * 2.0 + histogram_width + label_width,
            height + FONT_SM,
        ),
        Sense::hover(),
    );
    let bar = Rect::from_min_size(
        area.min + Vec2::new(0.0, FONT_SM * 0.5),
        Vec2::new(SPACE_4, height),
    );
    let painter = ui.painter();
    let y = |t: f32| bar.bottom() - t * height;
    for i in 0..256 {
        let lo = i as f32 / 256.0;
        let hi = (i + 1) as f32 / 256.0;
        let mut t = (lo + hi) * 0.5;
        if map.bands > 0 {
            let bands = map.bands as f32;
            t = ((t * bands).floor() + 0.5) / bands;
        }
        let at = t.clamp(0.0, 1.0) * map.ramp.len().saturating_sub(1) as f32;
        let low = at.floor() as usize;
        let a = map.ramp.get(low).copied().unwrap_or(map.none);
        let b = map.ramp.get(low + 1).copied().unwrap_or(a);
        let color: [f32; 3] = std::array::from_fn(|c| a[c] + (b[c] - a[c]) * (at - low as f32));
        painter.rect_filled(
            Rect::from_min_max(pos2(bar.left(), y(hi)), pos2(bar.right(), y(lo))),
            0.0,
            egui::Color32::from(egui::Rgba::from_rgb(color[0], color[1], color[2])),
        );
    }
    let histogram_x = bar.right() + SPACE_2;
    let maximum = map
        .legend
        .histogram
        .iter()
        .copied()
        .max()
        .unwrap_or(0)
        .max(1) as f32;
    let bins = map.legend.histogram.len() as f32;
    for (i, count) in map.legend.histogram.iter().enumerate() {
        let rect = Rect::from_min_max(
            pos2(histogram_x, y((i + 1) as f32 / bins)),
            pos2(
                histogram_x + histogram_width * *count as f32 / maximum,
                y(i as f32 / bins),
            ),
        );
        painter.rect_filled(rect, 0.0, TEXT3);
    }
    let fraction = |v: f32| (v - map.range[0]) / (map.range[1] - map.range[0]);
    if let Some((low, high)) = map.legend.tolerance {
        for v in [low, high] {
            if v >= map.range[0] && v <= map.range[1] {
                painter.line_segment(
                    [
                        pos2(bar.left(), y(fraction(v))),
                        pos2(histogram_x + histogram_width, y(fraction(v))),
                    ],
                    egui::Stroke::new(1.0, ACCENT),
                );
            }
        }
    }
    for (v, text) in labels {
        let at = y(fraction(v));
        painter.line_segment(
            [pos2(bar.right(), at), pos2(histogram_x, at)],
            egui::Stroke::new(1.0, TEXT2),
        );
        painter.galley(
            pos2(
                histogram_x + histogram_width + SPACE_2,
                at - text.size().y * 0.5,
            ),
            text,
            TEXT2,
        );
    }
}
