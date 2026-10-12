//! Orientation cube widget for showing camera orientation in the 3D viewport.
//!
//! This module provides a visual indicator of the current camera orientation,
//! rendered as a 3D cube with labeled faces (FRONT, REAR, LEFT, RIGHT, TOP, BOTTOM)
//! and colored axis arrows (X=red, Y=green, Z=blue).
//!
//! The cube is interactive: clicking faces snaps to that view (a double
//! click also frames everything), clicking arrows rotates 45°, a left drag
//! on the disc orbits as the middle button does, and the home glyph at its
//! top-left corner returns to the default corner view, framed.
//!
//! The RGB axis triad is drawn in a separate floating widget at the **bottom-left** of the
//! viewport so it does not overlap the cube (which sits at the **top-right**).
//!
//! `cube` draws the cube and takes its clicks, `arrows` the triad and the
//! turning arrows, `faces` the face textures and colours.

use std::collections::HashMap;

use axes::AxisSystem;
use egui::{
    Color32, ColorImage, Context, Id, Pos2, Response, Sense, Stroke, TextureHandle, TextureOptions,
    Ui,
    epaint::{Mesh as EguiMesh, Vertex as EguiVertex},
};
use glam::{Mat3, Quat, Vec3};
use resvg::render;
use tiny_skia::Pixmap;
use usvg::{Options, fontdb};

mod arrows;
mod cube;
mod faces;

use arrows::*;
use cube::*;
pub use faces::warm_face_textures;
use faces::*;

const FACE_TEMPLATE_SVG: &str = include_str!("face_template.svg");

/// Configuration for the orientation cube appearance
#[derive(Debug, Clone)]
pub struct OrientationCubeConfig {
    /// Total widget size (diameter of background circle)
    pub widget_size: f32,
    /// Scale of the cube within the widget
    pub cube_scale: f32,
    /// Background circle color
    pub background_color: Color32,
    pub border_color: Color32,
    /// Whether to show rotation arrows around the circle
    pub show_rotation_arrows: bool,
    /// Whether to show axis arrows
    pub show_axis_arrows: bool,
    /// The host only accepts rotations that keep the view direction (a
    /// sketch is open): the pitch and yaw arrows draw inert.
    pub planar_only: bool,
}

impl Default for OrientationCubeConfig {
    fn default() -> Self {
        Self {
            widget_size: 150.0,
            cube_scale: 40.0,
            background_color: Color32::from_rgba_unmultiplied(40, 40, 45, 220),
            border_color: Color32::from_gray(80),
            show_rotation_arrows: true,
            show_axis_arrows: true,
            planar_only: false,
        }
    }
}

/// Input data for drawing the orientation cube
pub struct OrientationCubeInput {
    /// Camera orientation as quaternion [x, y, z, w]
    pub camera_orientation: [f32; 4],
    /// Axis configuration used across the viewport (settings preset)
    pub axis_system: AxisSystem,
}

/// Result of orientation cube interaction
#[derive(Debug, Clone, Default)]
pub struct OrientationCubeResult {
    /// If set, snap the camera to this preset view.
    pub snap_to_view: Option<CameraSnapView>,
    /// If set, rotate camera by this amount (in degrees) around the specified axis
    pub rotate_delta: Option<RotateDelta>,
    /// `snap_to_view` came from a double click: frame everything as well.
    pub fit: bool,
    /// The home glyph was clicked: the default corner view, framed.
    pub home: bool,
    /// A left drag on the disc, orbiting the view.
    pub drag: Option<CubeDrag>,
}

/// A left drag on the cube's disc.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CubeDrag {
    /// The pointer stands this far from where it was pressed, in logical
    /// pixels.
    Moved(glam::Vec2),
    /// The button came up.
    Released,
}

/// Side of the home glyph, in logical pixels.
const HOME_SIZE: f32 = 18.0;
/// Inset of the home glyph's centre from the widget's top-left corner.
const HOME_INSET: f32 = 14.0;

/// Where the home glyph sits in the widget `rect`: its top-left corner,
/// clear of the disc and of the roll arrows at its sides.
fn home_rect(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_center_size(
        rect.min + egui::vec2(HOME_INSET, HOME_INSET),
        egui::Vec2::splat(HOME_SIZE),
    )
}

/// Whether a press at `origin` takes hold of the view: anywhere on the
/// disc of `radius` about `center`.
fn grabs_view(origin: Pos2, center: Pos2, radius: f32) -> bool {
    origin.distance(center) <= radius
}

/// Predefined camera snap views
#[derive(Debug, Clone, Copy)]
pub enum CameraSnapView {
    // Main faces
    Front,
    Rear,
    Left,
    Right,
    Top,
    Bottom,
    // Edges (12 total) - 45° rotations
    FrontTop,
    FrontBottom,
    FrontLeft,
    FrontRight,
    RearTop,
    RearBottom,
    RearLeft,
    RearRight,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    // Corners (8 total) - 45° in two directions
    FrontTopLeft,
    FrontTopRight,
    FrontBottomLeft,
    FrontBottomRight,
    RearTopLeft,
    RearTopRight,
    RearBottomLeft,
    RearBottomRight,
}

impl CameraSnapView {
    /// Get the yaw and pitch angles (in degrees) for this view.
    /// Used by the turntable camera system.
    pub fn yaw_pitch(&self) -> (f32, f32) {
        match self {
            // Main faces
            CameraSnapView::Front => (0.0, 0.0),
            CameraSnapView::Rear => (180.0, 0.0),
            CameraSnapView::Right => (90.0, 0.0),
            CameraSnapView::Left => (-90.0, 0.0),
            CameraSnapView::Top => (0.0, -90.0),
            CameraSnapView::Bottom => (0.0, 90.0),
            // Front edges (yaw=0, pitch varies)
            CameraSnapView::FrontTop => (0.0, -45.0),
            CameraSnapView::FrontBottom => (0.0, 45.0),
            CameraSnapView::FrontLeft => (-45.0, 0.0),
            CameraSnapView::FrontRight => (45.0, 0.0),
            // Rear edges (yaw=180, pitch varies)
            CameraSnapView::RearTop => (180.0, -45.0),
            CameraSnapView::RearBottom => (180.0, 45.0),
            CameraSnapView::RearLeft => (-135.0, 0.0),
            CameraSnapView::RearRight => (135.0, 0.0),
            // Top/Bottom side edges
            CameraSnapView::TopLeft => (-90.0, -45.0),
            CameraSnapView::TopRight => (90.0, -45.0),
            CameraSnapView::BottomLeft => (-90.0, 45.0),
            CameraSnapView::BottomRight => (90.0, 45.0),
            // Front corners (yaw ±45, pitch ±45)
            CameraSnapView::FrontTopLeft => (-45.0, -45.0),
            CameraSnapView::FrontTopRight => (45.0, -45.0),
            CameraSnapView::FrontBottomLeft => (-45.0, 45.0),
            CameraSnapView::FrontBottomRight => (45.0, 45.0),
            // Rear corners (yaw ±135, pitch ±45)
            CameraSnapView::RearTopLeft => (-135.0, -45.0),
            CameraSnapView::RearTopRight => (135.0, -45.0),
            CameraSnapView::RearBottomLeft => (-135.0, 45.0),
            CameraSnapView::RearBottomRight => (135.0, 45.0),
        }
    }

    /// Get the camera orientation quaternion for this view.
    /// Built using the turntable camera's yaw-then-pitch convention.
    pub fn orientation(&self) -> Quat {
        let (yaw_deg, pitch_deg) = self.yaw_pitch();
        let yaw_rad = yaw_deg.to_radians();
        let pitch_rad = pitch_deg.to_radians();
        let yaw_q = Quat::from_rotation_y(yaw_rad);
        let pitch_q = Quat::from_rotation_x(pitch_rad);
        (yaw_q * pitch_q).normalize()
    }
}

/// Rotation delta for arrow clicks
#[derive(Debug, Clone, Copy)]
pub struct RotateDelta {
    /// Rotation in degrees
    pub degrees: f32,
    /// Axis to rotate around (in screen space: X=right, Y=up)
    pub axis: RotateAxis,
}

#[derive(Debug, Clone, Copy)]
pub enum RotateAxis {
    ScreenX, // Horizontal axis (pitch)
    ScreenY, // Vertical axis (yaw)
    ScreenZ, // Z axis (roll)
}

impl RotateAxis {
    /// Whether the rotation leaves the camera looking along the same
    /// direction. Only roll does: it spins the view about the axis it
    /// looks down, which while editing is the sketch plane's normal.
    pub fn keeps_view_direction(self) -> bool {
        matches!(self, RotateAxis::ScreenZ)
    }
}

/// 3×3 rotation for the orientation cube (horizontal / vertical / depth basis).
fn camera_display_rotation(input: &OrientationCubeInput) -> Mat3 {
    let q_world = Quat::from_array(input.camera_orientation).inverse();
    let world_rot = Mat3::from_quat(q_world);
    let basis = input.axis_system.canonical_basis();
    let mut rot = basis.transpose() * world_rot * basis;

    let right = input.axis_system.horizontal().vector();
    let up = input.axis_system.vertical().vector();
    let depth = input.axis_system.depth().vector();
    let parity = right.cross(up).dot(depth);
    if parity < 0.0 {
        let adjust = Mat3::from_diagonal(Vec3::new(-1.0, 1.0, 1.0));
        rot = adjust * rot * adjust;
    }
    rot
}

/// Where **world** +X / +Y / +Z land in the axis widget (same `(Δx, −Δy)` as the cube).
/// `w` maps as `rot * (Bᵀ w)` with `rot = camera_display_rotation` and `B` the settings canonical basis:
/// the same pipeline as the cube's face directions, so the triad and the cube agree on which way
/// +Y points (back, not forward, for Z-up).
fn world_axes_widget_rotation(input: &OrientationCubeInput) -> Mat3 {
    let basis = input.axis_system.canonical_basis();
    camera_display_rotation(input) * basis.transpose()
}

/// Draws the orientation cube widget and returns interaction results
pub fn draw(
    ctx: &Context,
    viewport_rect: egui::Rect,
    input: &OrientationCubeInput,
    config: &OrientationCubeConfig,
) -> OrientationCubeResult {
    let mut result = OrientationCubeResult::default();

    let rot = camera_display_rotation(input);

    // Extra space at the top for arc arrows
    let arc_arrow_padding = 50.0;
    let total_height = config.widget_size + arc_arrow_padding;
    let total_width = config.widget_size + arc_arrow_padding;

    let y_offset: f32 = 10.0;

    let available = viewport_rect;
    let margin = 10.0;

    // Top-right of the viewport, under the floating view toolbar.
    let toolbar_clearance = 44.0;
    let pos = Pos2::new(
        available.right() - total_width - margin,
        available.top() + toolbar_clearance + margin,
    );

    egui::Area::new(egui::Id::new("orientation_cube"))
        .fixed_pos(pos)
        .order(crate::ui::VIEW_LAYER)
        .show(ctx, |ui| {
            let (response, painter) = ui.allocate_painter(
                egui::Vec2::new(total_width, total_height),
                Sense::click_and_drag(),
            );

            // Center the circle within the allocated space (offset down to make room for arc arrows)
            let local_center = Pos2::new(
                response.rect.center().x,
                response.rect.min.y + arc_arrow_padding + config.widget_size / 2.0 - y_offset,
            );

            painter.circle_filled(
                local_center,
                config.widget_size / 2.0,
                config.background_color,
            );
            painter.circle_stroke(
                local_center,
                config.widget_size / 2.0,
                Stroke::new(2.0_f32, config.border_color),
            );

            if let Some(snap) = draw_cube_interactive(
                ui,
                &painter,
                local_center,
                config.cube_scale,
                &rot,
                &input.axis_system,
                &response,
            ) {
                result.snap_to_view = Some(snap);
                result.fit = response.double_clicked();
            }

            result.drag = cube_drag(
                ui,
                &response,
                local_center,
                config.widget_size / 2.0,
                config.planar_only,
            );
            result.home = draw_home_glyph(ui, &response, config.planar_only);

            if config.show_rotation_arrows
                && let Some(delta) = draw_rotation_arrows_interactive(
                    ui,
                    &painter,
                    local_center,
                    config.widget_size,
                    &response,
                    y_offset,
                    config.planar_only,
                )
            {
                result.rotate_delta = Some(delta);
            }
        });

    // Axis triad alone: bottom-left, away from the top-right cube and
    // above the legend line a workbench may draw there.
    if config.show_axis_arrows {
        let axis_margin = margin;
        let axis_widget = 80.0_f32;
        let axis_pos = Pos2::new(
            available.left() + axis_margin,
            available.bottom() - axis_widget - axis_margin - 24.0,
        );
        egui::Area::new(egui::Id::new("orientation_axes"))
            .fixed_pos(axis_pos)
            .order(crate::ui::VIEW_LAYER)
            .show(ctx, |ui| {
                let (response, painter) =
                    ui.allocate_painter(egui::Vec2::splat(axis_widget), Sense::hover());
                let axis_center = response.rect.center();
                let axis_rot = world_axes_widget_rotation(input);
                draw_axis_arrows(&painter, axis_center, &axis_rot);
            });
    }

    result
}

/// The drag a left press on the disc makes, `None` while the view is
/// locked to a plane or the press began off the disc.
fn cube_drag(
    ui: &Ui,
    response: &Response,
    center: Pos2,
    radius: f32,
    planar_only: bool,
) -> Option<CubeDrag> {
    if response.drag_stopped_by(egui::PointerButton::Primary) {
        return Some(CubeDrag::Released);
    }
    if planar_only || !response.dragged_by(egui::PointerButton::Primary) {
        return None;
    }
    let (origin, pointer) = ui.input(|i| (i.pointer.press_origin(), i.pointer.latest_pos()));
    let (origin, pointer) = (origin?, pointer?);
    if !grabs_view(origin, center, radius) {
        return None;
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    let offset = pointer - origin;
    Some(CubeDrag::Moved(glam::Vec2::new(offset.x, offset.y)))
}

/// Draws the home glyph and answers whether it was clicked. While the view
/// is locked to a plane it draws inert, as the arrows that would leave the
/// plane do.
fn draw_home_glyph(ui: &Ui, response: &Response, planar_only: bool) -> bool {
    let rect = home_rect(response.rect);
    let hovered = !planar_only
        && ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|p| rect.contains(p));
    let tint = if planar_only {
        ui_kit::tokens::TEXT3
    } else if hovered {
        ui_kit::tokens::TEXT1
    } else {
        ui_kit::tokens::TEXT2
    };
    if let Some(image) = ui_kit::icon::image(ui.ctx(), "home", HOME_SIZE, tint) {
        image.paint_at(ui, rect);
    }
    if hovered {
        response.clone().on_hover_ui_at_pointer(|ui| {
            ui.label("Home view: the default corner view, fitted");
        });
    }
    !planar_only
        && response.clicked()
        && response
            .interact_pointer_pos()
            .is_some_and(|p| rect.contains(p))
}

/// Point-in-polygon test using ray casting
pub(super) fn point_in_polygon(point: Pos2, polygon: &[Pos2]) -> bool {
    let n = polygon.len();
    if n < 3 {
        return false;
    }

    let mut inside = false;
    let mut j = n - 1;

    for i in 0..n {
        let pi = polygon[i];
        let pj = polygon[j];

        if ((pi.y > point.y) != (pj.y > point.y))
            && (point.x < (pj.x - pi.x) * (point.y - pi.y) / (pj.y - pi.y) + pi.x)
        {
            inside = !inside;
        }
        j = i;
    }

    inside
}

#[cfg(test)]
mod tests {
    use super::{RotateAxis, grabs_view, home_rect};
    use egui::{Pos2, Rect, vec2};

    #[test]
    fn the_home_glyph_sits_in_the_corner_off_the_disc() {
        // The widget as `draw` lays it out: 200 px square, the disc of
        // radius 75 centred 115 px down.
        let widget = Rect::from_min_size(Pos2::new(1000.0, 60.0), vec2(200.0, 200.0));
        let center = Pos2::new(widget.center().x, widget.min.y + 115.0);
        let home = home_rect(widget);
        assert!(widget.contains_rect(home));
        assert!(home.contains(widget.min + vec2(14.0, 14.0)));
        for corner in [
            home.left_top(),
            home.right_top(),
            home.left_bottom(),
            home.right_bottom(),
        ] {
            assert!(!grabs_view(corner, center, 75.0), "{corner:?} on the disc");
        }
        assert!(grabs_view(center, center, 75.0));
        assert!(grabs_view(center + vec2(0.0, 74.0), center, 75.0));
        assert!(!grabs_view(center + vec2(60.0, 60.0), center, 75.0));
    }

    #[test]
    fn only_roll_leaves_the_view_direction_alone() {
        assert!(RotateAxis::ScreenZ.keeps_view_direction());
        assert!(!RotateAxis::ScreenX.keeps_view_direction());
        assert!(!RotateAxis::ScreenY.keeps_view_direction());
    }
}
