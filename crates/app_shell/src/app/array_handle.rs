//! The spacing handle of a row of copies: while the array task lays out a
//! row, an arrow stands on its first copy along the row, and dragging it
//! changes the spacing, the copies following as it moves. The task keeps
//! the change, so OK makes it one undo step with the rest and Cancel puts
//! it back.

use core_document::{
    ArrayId, Document, ScreenSpaceLabel, ScreenSpaceMark, ScreenSpaceOverlay, SketchPalette,
};
use glam::{Vec2, Vec3};

use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::keyboard::{Key, NamedKey};

use crate::PrintCadApp;
use crate::camera::CameraController;

/// How long the arrow is on screen, physical pixels.
const ARROW_PX: f32 = 56.0;
/// How near its tip a press takes hold of it, physical pixels.
const REACH_PX: f32 = 12.0;
/// The steps a dragged spacing goes in, mm.
const STEP_MM: f32 = 0.1;

/// The handle as it stands in the view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Handle {
    pub array: ArrayId,
    /// The first copy's middle, on screen.
    pub base: Vec2,
    /// The arrow's tip, on screen.
    pub tip: Vec2,
    /// Screen pixels the first copy moves for each millimetre of spacing.
    pub pixels_per_mm: Vec2,
    pub spacing: f32,
}

/// A drag of the handle: where it was taken hold of, and the spacing
/// then.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SpacingDrag {
    pub array: ArrayId,
    pub press: Vec2,
    pub pixels_per_mm: Vec2,
    pub spacing: f32,
}

impl SpacingDrag {
    pub fn of(handle: &Handle, press: Vec2) -> Self {
        Self {
            array: handle.array,
            press,
            pixels_per_mm: handle.pixels_per_mm,
            spacing: handle.spacing,
        }
    }

    /// The spacing with the pointer at `at`: the press's spacing changed
    /// by how far the pointer has gone along the row on screen, in steps.
    pub fn spacing_at(&self, at: Vec2) -> f32 {
        let along = self.pixels_per_mm.length_squared();
        if along < 1e-12 {
            return self.spacing;
        }
        let moved = (at - self.press).dot(self.pixels_per_mm) / along;
        ((self.spacing + moved) / STEP_MM).round() * STEP_MM
    }
}

/// Where the handle of `array`'s row stands: on the middle of its first
/// copy, pointing along `direction`. `None` when the copy or the row is
/// out of view.
pub(crate) fn handle(
    document: &Document,
    camera: &CameraController,
    array: ArrayId,
    direction: [f32; 3],
    spacing: f32,
) -> Option<Handle> {
    let row = document.copy_array(array)?;
    let first = row.instances.first()?;
    let (lo, hi) = document.imported_geometry(first.body)?.bounds_mm?;
    let middle = (Vec3::from_array(lo) + Vec3::from_array(hi)) * 0.5;
    let way = Vec3::from_array(direction).normalize_or_zero();
    let to_screen = |p: Vec3| camera.world_to_viewport(p).map(|(x, y)| Vec2::new(x, y));
    let base = to_screen(middle)?;
    let per_mm = to_screen(middle + way)? - base;
    if per_mm.length() < 1e-4 {
        return None;
    }
    // The first copy stands `index` spacings from the source, so it moves
    // that many millimetres for each one the spacing grows.
    let pixels_per_mm = per_mm * first.index as f32;
    let tip = base + per_mm.normalize() * ARROW_PX;
    Some(Handle {
        array,
        base,
        tip,
        pixels_per_mm,
        spacing,
    })
}

impl Handle {
    /// Whether the pointer at `at` takes hold of it: near its tip, or on
    /// the arrow's shaft.
    pub fn is_under(&self, at: Vec2) -> bool {
        if at.distance(self.tip) <= REACH_PX {
            return true;
        }
        let shaft = self.tip - self.base;
        let t = ((at - self.base).dot(shaft) / shaft.length_squared()).clamp(0.3, 1.0);
        at.distance(self.base + shaft * t) <= REACH_PX * 0.5
    }

    /// What draws it: the arrow, a knob at its tip and the spacing beside
    /// it, brighter while it is held or under the pointer.
    pub fn drawn(
        &self,
        hot: bool,
        unit: core_document::Unit,
    ) -> (
        Vec<ScreenSpaceOverlay>,
        Vec<ScreenSpaceMark>,
        ScreenSpaceLabel,
    ) {
        let palette = SketchPalette::default();
        let color = if hot {
            palette.preselect
        } else {
            palette.preview
        };
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
        let text = core_document::format_length_mm(self.spacing, unit, 2);
        let at = self.tip + way * 14.0 + side * 10.0;
        let label = ScreenSpaceLabel::new(at.to_array(), text, color, 12.0).pill();
        (lines, marks, label)
    }
}

impl PrintCadApp {
    /// The handle of the row the open array task lays out, where it
    /// stands now.
    pub(crate) fn array_handle(&self) -> Option<Handle> {
        let (array, direction, spacing) = self.gfx.as_ref()?.ui_layer.array_row()?;
        handle(
            &self.session.document,
            &self.session.camera,
            array,
            direction,
            spacing,
        )
    }

    /// Whether the handle is held, or under the pointer.
    pub(crate) fn array_handle_hot(&self, handle: &Handle) -> bool {
        self.array_drag.is_some()
            || self
                .cursor_in_viewport
                .is_some_and(|(x, y)| handle.is_under(Vec2::new(x, y)))
    }

    /// A press on the handle takes hold of it, moves drag it, the release
    /// lets go and Escape puts the spacing back; whether it took the event.
    pub(crate) fn array_handle_input(&mut self, event: &WindowEvent) -> bool {
        let cursor = self.cursor_in_viewport.map(|(x, y)| Vec2::new(x, y));
        match event {
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state: ElementState::Pressed,
                ..
            } => {
                let Some(at) = cursor else {
                    return false;
                };
                let Some(handle) = self.array_handle().filter(|h| h.is_under(at)) else {
                    return false;
                };
                self.array_drag = Some(SpacingDrag::of(&handle, at));
                self.redraw_needed = true;
                true
            }
            WindowEvent::CursorMoved { .. } => {
                let Some(drag) = self.array_drag else {
                    return false;
                };
                if let Some(at) = cursor {
                    self.set_array_spacing(drag.array, drag.spacing_at(at));
                }
                true
            }
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state: ElementState::Released,
                ..
            } => {
                let held = self.array_drag.take().is_some();
                self.redraw_needed |= held;
                held
            }
            WindowEvent::KeyboardInput { event: key, .. }
                if key.state == ElementState::Pressed
                    && key.logical_key == Key::Named(NamedKey::Escape) =>
            {
                let Some(drag) = self.array_drag.take() else {
                    return false;
                };
                self.set_array_spacing(drag.array, drag.spacing);
                true
            }
            _ => false,
        }
    }

    fn set_array_spacing(&mut self, array: ArrayId, spacing: f32) {
        let Some(gfx) = self.gfx.as_mut() else {
            return;
        };
        if gfx.ui_layer.array_row().map(|(id, ..)| id) == Some(array) {
            gfx.ui_layer
                .drag_array_spacing(&mut self.session.document, spacing);
            self.redraw_needed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_along_the_row_changes_the_spacing_in_steps_and_across_it_does_not() {
        let drag = SpacingDrag {
            array: ArrayId::new(),
            press: Vec2::new(100.0, 100.0),
            pixels_per_mm: Vec2::new(4.0, 0.0),
            spacing: 20.0,
        };
        assert_eq!(drag.spacing_at(Vec2::new(140.0, 100.0)), 30.0);
        assert_eq!(drag.spacing_at(Vec2::new(100.0, 180.0)), 20.0);
        let fine = drag.spacing_at(Vec2::new(101.3, 100.0));
        assert!((fine - 20.3).abs() < 1e-4, "{fine}");
    }

    #[test]
    fn the_handle_is_taken_at_its_tip_or_along_its_shaft_not_at_its_foot() {
        let handle = Handle {
            array: ArrayId::new(),
            base: Vec2::ZERO,
            tip: Vec2::new(56.0, 0.0),
            pixels_per_mm: Vec2::new(2.0, 0.0),
            spacing: 10.0,
        };
        assert!(handle.is_under(Vec2::new(60.0, 4.0)));
        assert!(handle.is_under(Vec2::new(30.0, 2.0)));
        assert!(
            !handle.is_under(Vec2::new(2.0, 0.0)),
            "the copy itself stays pickable"
        );
        assert!(!handle.is_under(Vec2::new(30.0, 20.0)));
    }
}
