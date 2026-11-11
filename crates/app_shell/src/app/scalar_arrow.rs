//! Screen-space arrows and press-relative scalar dragging for viewport handles.

use core_document::{ScreenSpaceLabel, ScreenSpaceMark, ScreenSpaceOverlay};
use glam::Vec2;

/// Arrow length in physical pixels.
pub(super) const ARROW_PX: f32 = 56.0;
const REACH_PX: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScalarDrag {
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

/// A handle the pointer holds: what it drags (and what a cancel puts back
/// beside the value) and the drag from the press.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Held<K> {
    pub of: K,
    pub drag: ScalarDrag,
}

/// What reaches a handle's gesture.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum HandleEvent<K> {
    /// A press, on a handle (`Some`, taking hold of it) or elsewhere.
    Press(Option<Held<K>>),
    /// The pointer moved, to `Some` point in the view.
    Move(Option<Vec2>),
    Release,
    /// Escape, focus loss or a tab switch: the value goes back to the
    /// press's and the handle is let go.
    Cancel,
    /// What the handle drags is not offered any more (its task closed):
    /// the handle is let go and nothing is set, the task having settled
    /// the value.
    Gone,
}

/// What a handle event did: whether it took the event, and the value to
/// set on what the handle drags.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct HandleStep<K> {
    pub taken: bool,
    pub set: Option<(K, f32)>,
}

/// The one gesture every scalar handle follows: a press takes hold, moves
/// drag the value from the press's, the release keeps it, a cancel puts
/// the press's value back. Nothing stays held after a release or cancel.
pub(super) fn handle_event<K: Clone>(
    held: &mut Option<Held<K>>,
    event: HandleEvent<K>,
) -> HandleStep<K> {
    let step = |taken, set| HandleStep { taken, set };
    match event {
        HandleEvent::Press(Some(hold)) => {
            *held = Some(hold);
            step(true, None)
        }
        HandleEvent::Press(None) => step(false, None),
        HandleEvent::Move(at) => match held {
            Some(hold) => step(true, at.map(|at| (hold.of.clone(), hold.drag.value_at(at)))),
            None => step(false, None),
        },
        HandleEvent::Release | HandleEvent::Gone => step(held.take().is_some(), None),
        HandleEvent::Cancel => match held.take() {
            Some(hold) => step(true, Some((hold.of, hold.drag.value))),
            None => step(false, None),
        },
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

    #[test]
    fn a_held_handle_drags_keeps_on_release_and_restores_on_cancel() {
        use HandleEvent::*;
        let hold = Held {
            of: 7,
            drag: ScalarDrag {
                press: Vec2::new(100.0, 100.0),
                pixels_per_unit: Vec2::new(4.0, 0.0),
                value: 20.0,
                step: 0.1,
            },
        };
        let to = |x: f32| Move(Some(Vec2::new(x, 100.0)));
        let set = |v: f32| Some((7, v));
        // Each run: the events, then what each did (taken, value set).
        type Did = (bool, Option<(i32, f32)>);
        let runs: [(&[HandleEvent<i32>], &[Did]); 5] = [
            // Off the handle nothing is taken.
            (
                &[Press(None), to(140.0), Release, Cancel],
                &[(false, None); 4],
            ),
            // Press, move, release: the last value stays, later moves pass.
            (
                &[Press(Some(hold)), to(140.0), Move(None), Release, to(180.0)],
                &[
                    (true, None),
                    (true, set(30.0)),
                    (true, None),
                    (true, None),
                    (false, None),
                ],
            ),
            // Escape or focus loss mid-drag puts the press's value back.
            (
                &[Press(Some(hold)), to(140.0), Cancel, to(180.0), Release],
                &[
                    (true, None),
                    (true, set(30.0)),
                    (true, set(20.0)),
                    (false, None),
                    (false, None),
                ],
            ),
            // The task closing mid-drag lets go, setting nothing: later
            // moves, a release or a cancel pass.
            (
                &[
                    Press(Some(hold)),
                    to(140.0),
                    Gone,
                    to(180.0),
                    Release,
                    Cancel,
                ],
                &[
                    (true, None),
                    (true, set(30.0)),
                    (true, None),
                    (false, None),
                    (false, None),
                    (false, None),
                ],
            ),
            // A cancel once let go changes nothing.
            (
                &[Press(Some(hold)), Release, Cancel],
                &[(true, None), (true, None), (false, None)],
            ),
        ];
        for (events, expected) in runs {
            let mut held = None;
            let got: Vec<_> = events
                .iter()
                .map(|e| handle_event(&mut held, e.clone()))
                .map(|s| (s.taken, s.set))
                .collect();
            assert_eq!(got, expected, "{events:?}");
            assert!(held.is_none());
        }
    }
}
