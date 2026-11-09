//! Screen-space arrows and press-relative scalar dragging for viewport handles.

use core_document::{ScreenSpaceLabel, ScreenSpaceMark, ScreenSpaceOverlay};
use glam::Vec2;

/// Arrow length in physical pixels.
pub(super) const ARROW_PX: f32 = 56.0;
const REACH_PX: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ScalarDrag {
    pub press: Vec2,
    pub pixels_per_unit: Vec2,
    pub value: f32,
    pub step: f32,
}

impl ScalarDrag {
    pub fn value_at(&self, at: Vec2) -> f32 {
        let along = self.pixels_per_unit.length_squared();
        if along < 1e-12 {
            return self.value;
        }
        let moved = (at - self.press).dot(self.pixels_per_unit) / along;
        ((self.value + moved) / self.step).round() * self.step
    }
}

pub(super) struct ScalarArrow {
    pub base: Vec2,
    pub tip: Vec2,
}

impl ScalarArrow {
    pub fn is_under(&self, at: Vec2) -> bool {
        if at.distance(self.tip) <= REACH_PX {
            return true;
        }
        let shaft = self.tip - self.base;
        let t = ((at - self.base).dot(shaft) / shaft.length_squared()).clamp(0.3, 1.0);
        at.distance(self.base + shaft * t) <= REACH_PX * 0.5
    }

    pub fn drawn(
        &self,
        text: String,
        color: [f32; 3],
    ) -> (
        Vec<ScreenSpaceOverlay>,
        Vec<ScreenSpaceMark>,
        ScreenSpaceLabel,
    ) {
        let way = (self.tip - self.base).normalize_or_zero();
        let side = Vec2::new(-way.y, way.x);
        let head = |s: f32| self.tip - way * 10.0 + side * s * 5.0;
        let line =
            |a: Vec2, b: Vec2| ScreenSpaceOverlay::new(a.to_array(), b.to_array(), color, 2.0);
        let lines = vec![
            line(self.base, self.tip),
            line(head(1.0), self.tip),
            line(head(-1.0), self.tip),
        ];
        let marks = vec![ScreenSpaceMark::dot(self.base.to_array(), 3.0, color)];
        let at = self.tip + way * 14.0 + side * 10.0;
        let label = ScreenSpaceLabel::new(at.to_array(), text, color, 12.0).pill();
        (lines, marks, label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_scale_step_and_degenerate_projection() {
        for (pixels, step, expected) in [
            (Vec2::ZERO, 0.1, 1.03),
            (Vec2::new(1e-7, 0.0), 0.1, 1.03),
            (Vec2::new(2.0, 0.0), 0.5, 2.5),
            (Vec2::new(4.0, 0.0), 0.1, 1.8),
            (Vec2::new(-2.0, 0.0), 0.5, -0.5),
        ] {
            let drag = ScalarDrag {
                press: Vec2::new(10.0, 10.0),
                pixels_per_unit: pixels,
                value: 1.03,
                step,
            };
            assert!((drag.value_at(Vec2::new(13.0, 15.0)) - expected).abs() < 1e-6);
        }
    }
}
