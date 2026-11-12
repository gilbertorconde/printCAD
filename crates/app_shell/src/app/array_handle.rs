//! The spacing handle of a row of copies: while the array task lays out a
//! row, an arrow stands on its first copy along the row, and dragging it
//! changes the spacing, the copies following as it moves. The task keeps
//! the change, so OK makes it one undo step with the rest and Cancel puts
//! it back.

use core_document::{
    ArrayId, Document, ScreenSpaceLabel, ScreenSpaceMark, ScreenSpaceOverlay, SketchPalette,
};
use glam::{Vec2, Vec3};

use super::gesture::HandleTarget;
use super::scalar_arrow::{ARROW_PX, HandleEvent, Held, ScalarArrow, ScalarDrag};
use crate::PrintCadApp;
use crate::camera::CameraController;

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

impl Handle {
    /// The drag a press at `press` starts.
    pub fn held(&self, press: Vec2) -> Held<HandleTarget> {
        Held {
            of: HandleTarget::Array(self.array),
            drag: ScalarDrag {
                press,
                pixels_per_unit: self.pixels_per_mm,
                value: self.spacing,
                step: STEP_MM,
                stops: super::scalar_arrow::UNBOUNDED,
            },
        }
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
        self.arrow().is_under(at)
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
        self.arrow().drawn(
            core_document::format_length_mm(self.spacing, unit, 2),
            color,
        )
    }

    fn arrow(&self) -> ScalarArrow {
        ScalarArrow {
            base: self.base,
            tip: self.tip,
        }
    }
}

impl PrintCadApp {
    /// The handle of the row the open array task lays out, where it
    /// stands now.
    pub(crate) fn array_handle(&self) -> Option<Handle> {
        let (array, direction, spacing) = self.gfx.as_ref()?.ui_layer.array_row()?;
        handle(
            &self.session.document,
            &self.session.view.camera,
            array,
            direction,
            spacing,
        )
    }

    /// Whether the handle is held, or under the pointer.
    pub(crate) fn array_handle_hot(&self, handle: &Handle) -> bool {
        matches!(
            self.session.gestures.handle_held(),
            Some(HandleTarget::Array(_))
        ) || self
            .input
            .cursor_in_viewport
            .is_some_and(|(x, y)| handle.is_under(Vec2::new(x, y)))
    }

    /// A held handle whose row the array task no longer lays out (the
    /// task closed or moved on while the button was down) is let go, so it
    /// takes no more of the pointer.
    pub(super) fn let_go_of_a_row_gone(&mut self) {
        let row = self
            .gfx
            .as_ref()
            .and_then(|gfx| gfx.ui_layer.array_row())
            .map(|(id, ..)| id);
        if let Some(HandleTarget::Array(held)) = self.session.gestures.handle_held()
            && Some(*held) != row
        {
            self.handle_event(HandleEvent::Gone);
        }
    }

    pub(super) fn set_array_spacing(&mut self, array: ArrayId, spacing: f32) {
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
