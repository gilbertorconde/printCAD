//! Orientation cube widget for showing camera orientation in the 3D viewport.
//!
//! This module provides a visual indicator of the current camera orientation,
//! rendered as a 3D cube with labeled faces (FRONT, REAR, LEFT, RIGHT, TOP, BOTTOM)
//! and colored axis arrows (X=red, Y=green, Z=blue).
//!
//! The cube is interactive: clicking faces snaps to that view, clicking arrows rotates 45°.
//!
//! The RGB axis triad is drawn in a separate floating widget at the **top-right** of the
//! viewport so it does not overlap the cube (which sits at the **bottom-right**).
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
    /// Border color
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
    /// If set, snap camera to look from this direction (normalized vector pointing FROM camera TO target)
    pub snap_to_view: Option<CameraSnapView>,
    /// If set, rotate camera by this amount (in degrees) around the specified axis
    pub rotate_delta: Option<RotateDelta>,
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
/// `w` maps as `rot * (Bᵀ w)` with `rot = camera_display_rotation` and `B` the settings canonical basis —
/// same pipeline as cube face directions, which fixes e.g. +Y “back” vs forward for Z‑up.
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

    // Get the available central rect (the viewport area between panels)
    let available = viewport_rect;
    let margin = 10.0;

    // Top-right of the viewport, under the floating view toolbar.
    let toolbar_clearance = 44.0;
    let pos = Pos2::new(
        available.right() - total_width - margin,
        available.top() + toolbar_clearance + margin,
    );

    // Use Area for floating widget in the viewport
    egui::Area::new(egui::Id::new("orientation_cube"))
        .fixed_pos(pos)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let (response, painter) =
                ui.allocate_painter(egui::Vec2::new(total_width, total_height), Sense::click());

            // Center the circle within the allocated space (offset down to make room for arc arrows)
            let local_center = Pos2::new(
                response.rect.center().x,
                response.rect.min.y + arc_arrow_padding + config.widget_size / 2.0 - y_offset,
            );

            // Draw background circle (unchanged outer widget)
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

            // Draw and handle cube face clicks
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
            }

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
            .order(egui::Order::Foreground)
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
    use super::RotateAxis;

    #[test]
    fn only_roll_leaves_the_view_direction_alone() {
        assert!(RotateAxis::ScreenZ.keeps_view_direction());
        assert!(!RotateAxis::ScreenX.keeps_view_direction());
        assert!(!RotateAxis::ScreenY.keeps_view_direction());
    }
}
