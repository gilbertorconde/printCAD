//! The radial menu over the view (`app/radial.rs` opens it and starts
//! what is let go on): the slices round the press, clockwise from the
//! top, the one pointed at lit and named in the middle.

use egui::{Align2, Pos2, Rect, Stroke, Vec2, pos2, vec2};
use ui_kit::sans_medium;
use ui_kit::tokens::*;

/// The open menu, as the UI draws it.
#[derive(Debug, Clone)]
pub struct RadialView {
    /// Its middle, in the view's physical pixels.
    pub centre: [f32; 2],
    /// Each slice's icon and name.
    pub slices: Vec<(&'static str, String)>,
    pub pointed: Option<usize>,
}

/// From the middle to the slices' icons, and their chips' radius, in
/// points.
const REACH: f32 = 64.0;
const CHIP: f32 = 18.0;

pub fn draw(ctx: &egui::Context, viewport: Rect, view: &RadialView) {
    let scale = ctx.pixels_per_point();
    let centre = viewport.min + vec2(view.centre[0], view.centre[1]) / scale;
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("radial_menu"),
    ));
    let outer = REACH + CHIP + 8.0;
    painter.circle(
        centre,
        outer,
        BG1.gamma_multiply(0.92),
        Stroke::new(1.0, BORDER_STRONG),
    );
    painter.circle_stroke(centre, 24.0, Stroke::new(1.0, BORDER));
    let share = std::f32::consts::TAU / view.slices.len().max(1) as f32;
    for (i, (icon, _)) in view.slices.iter().enumerate() {
        let turn = i as f32 * share;
        let at: Pos2 = centre + Vec2::new(turn.sin(), -turn.cos()) * REACH;
        let lit = view.pointed == Some(i);
        painter.circle(
            at,
            CHIP,
            if lit { ACCENT_DIM } else { BG2 },
            Stroke::new(1.0, if lit { ACCENT } else { BORDER }),
        );
        if let Some(texture) = ui_kit::icon::texture(ctx, icon) {
            let rect = Rect::from_center_size(at, Vec2::splat(18.0));
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(texture.id(), rect, uv, if lit { TEXT1 } else { TEXT2 });
        }
    }
    let named = view.pointed.and_then(|i| view.slices.get(i));
    if let Some((_, label)) = named {
        painter.text(
            centre + vec2(0.0, outer + 14.0),
            Align2::CENTER_CENTER,
            label,
            sans_medium(FONT_SM),
            TEXT1,
        );
    }
    painter.circle_filled(centre, 3.0, TEXT3);
}
