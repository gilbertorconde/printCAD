//! Orientation cube widget for showing camera orientation in the 3D viewport.
//!
//! This module provides a visual indicator of the current camera orientation,
//! rendered as a 3D cube with labeled faces (FRONT, REAR, LEFT, RIGHT, TOP, BOTTOM)
//! and colored axis arrows (X=red, Y=green, Z=blue).
//!
//! The cube is interactive: clicking faces snaps to that view, clicking arrows rotates 45°.

use std::collections::HashMap;

use axes::AxisSystem;
use egui::{
    epaint::{Mesh as EguiMesh, Vertex as EguiVertex},
    Color32, ColorImage, Context, Id, Pos2, Response, Sense, Stroke, TextureHandle, TextureOptions,
    Ui,
};
use glam::{Mat3, Quat, Vec3};
use resvg::render;
use tiny_skia::Pixmap;
use usvg::{fontdb, Options};

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
        }
    }
}

/// Input data for drawing the orientation cube
pub struct OrientationCubeInput {
    /// Camera orientation as quaternion [x, y, z, w]
    pub camera_orientation: [f32; 4],
    /// Axis configuration used across the viewport
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

/// Draws the orientation cube widget and returns interaction results
pub fn draw(
    ctx: &Context,
    input: &OrientationCubeInput,
    config: &OrientationCubeConfig,
) -> OrientationCubeResult {
    let mut result = OrientationCubeResult::default();

    // Extra space at the top for arc arrows
    let arc_arrow_padding = 50.0;
    let total_height = config.widget_size + arc_arrow_padding;
    let total_width = config.widget_size + arc_arrow_padding;

    let y_offset: f32 = 10.0;

    // Get the available central rect (the viewport area between panels)
    let available = ctx.available_rect();
    let margin = 10.0;

    // Position in bottom-right of the available viewport area
    let pos = Pos2::new(
        available.right() - total_width - margin,
        available.bottom() - total_height - margin,
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
                Stroke::new(2.0, config.border_color),
            );

            // Get camera orientation quaternion. We invert so cube shows world axes
            // relative to the camera, then flip X/Z to match the camera's screen axes
            // (so that X/Z rotations appear with the expected handedness).
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

            // Draw and handle cube face clicks
            if let Some(snap) = draw_cube_interactive(
                ui,
                &painter,
                local_center,
                config.cube_scale,
                &rot,
                &response,
            ) {
                result.snap_to_view = Some(snap);
            }

            if config.show_axis_arrows {
                draw_axis_arrows(&painter, local_center, &rot, input.axis_system);
            }

            if config.show_rotation_arrows {
                if let Some(delta) = draw_rotation_arrows_interactive(
                    ui,
                    &painter,
                    local_center,
                    config.widget_size,
                    &response,
                    y_offset,
                ) {
                    result.rotate_delta = Some(delta);
                }
            }
        });

    result
}

/// A polygon to draw on the cube (main face, edge bevel, or corner bevel)
struct CubePolygon {
    /// 3D vertices (will be transformed and projected)
    verts: Vec<Vec3>,
    /// Normal direction in world space
    normal: Vec3,
    /// Base color
    color: Color32,
    /// Optional label (only for main faces)
    label: Option<&'static str>,
    /// Optional snap view (only for main faces)
    snap_view: Option<CameraSnapView>,
    /// Optional per-vertex UVs for textured faces
    uvs: Option<Vec<[f32; 2]>>,
}

/// Draws the 3D chamfered cube with labeled faces and handles clicks
fn draw_cube_interactive(
    ui: &Ui,
    painter: &egui::Painter,
    center: Pos2,
    cube_scale: f32,
    rot: &Mat3,
    response: &Response,
) -> Option<CameraSnapView> {
    let ctx = ui.ctx();
    let mut clicked_face: Option<CameraSnapView> = None;

    let m = 0.12_f32;

    // - Main faces are 8-sided polygons (octagons with cut corners)
    // - Edge bevels are quads
    // - Corner bevels are hexagons

    let mut polygons: Vec<CubePolygon> = Vec::new();

    // Face colors
    let front_color = Color32::from_rgb(100, 130, 170);
    let rear_color = Color32::from_rgb(100, 130, 170);
    let right_color = Color32::from_rgb(170, 100, 100);
    let left_color = Color32::from_rgb(100, 170, 100);
    let top_color = Color32::from_rgb(150, 150, 170);
    let bottom_color = Color32::from_rgb(120, 120, 140);
    let edge_color = Color32::from_rgb(160, 165, 175);
    let corner_color = Color32::from_rgb(145, 150, 160);

    // Helper to create main face vertices (8-sided polygon)
    // x_dir and y_dir are the face's local X and Y axes, z_dir is the normal (pointing outward)
    let make_main_face = |x_dir: Vec3, y_dir: Vec3, z_dir: Vec3| -> Vec<Vec3> {
        let x2 = x_dir * (1.0 - m * 2.0);
        let y2 = y_dir * (1.0 - m * 2.0);
        let x4 = x_dir * (1.0 - m * 4.0);
        let y4 = y_dir * (1.0 - m * 4.0);
        vec![
            z_dir - x2 - y4,
            z_dir - x4 - y2,
            z_dir + x4 - y2,
            z_dir + x2 - y4,
            z_dir + x2 + y4,
            z_dir + x4 + y2,
            z_dir - x4 + y2,
            z_dir - x2 + y4,
        ]
    };

    // Helper to create edge bevel vertices (4-sided polygon)
    // Following x_dir is along the edge, z_dir is the edge normal direction
    // y_dir is computed as x_dir.cross(-z_dir)
    let make_edge_face = |x_dir: Vec3, z_dir: Vec3| -> Vec<Vec3> {
        let y_dir = x_dir.cross(-z_dir);
        let x4 = x_dir * (1.0 - m * 4.0);
        let y_e = y_dir * m;
        let z_e = z_dir * (1.0 - m);
        vec![
            z_e - x4 - y_e,
            z_e + x4 - y_e,
            z_e + x4 + y_e,
            z_e - x4 + y_e,
        ]
    };

    // Helper to create corner bevel vertices (6-sided polygon / hexagon)
    let make_corner_face = |x_dir: Vec3, z_dir: Vec3| -> Vec<Vec3> {
        let y_dir = x_dir.cross(-z_dir);
        let x_c = x_dir * m;
        let y_c = y_dir * m;
        let z_c = z_dir * (1.0 - 2.0 * m);
        vec![
            z_c - x_c * 2.0,
            z_c - x_c - y_c,
            z_c + x_c - y_c,
            z_c + x_c * 2.0,
            z_c + x_c + y_c,
            z_c - x_c + y_c,
        ]
    };

    let x = Vec3::X;
    let y = Vec3::Y;
    let z = Vec3::Z;

    let fc_x = x;
    let fc_y = -z;
    let fc_z = y;

    // ===== MAIN FACES (6 octagons) =====
    // These were working before - using our coordinate system directly

    // Top (+Y)
    let verts_top = make_main_face(x, z, y);
    polygons.push(CubePolygon {
        uvs: Some(face_uvs(&verts_top, x, z, true, true)),
        verts: verts_top,
        normal: Vec3::Y,
        color: top_color,
        label: Some("TOP"),
        snap_view: Some(CameraSnapView::Top),
    });

    // Bottom (-Y)
    let verts_bottom = make_main_face(x, -z, -y);
    polygons.push(CubePolygon {
        uvs: Some(face_uvs(&verts_bottom, x, -z, true, true)),
        verts: verts_bottom,
        normal: Vec3::NEG_Y,
        color: bottom_color,
        label: Some("BOTTOM"),
        snap_view: Some(CameraSnapView::Bottom),
    });

    // Front (+Z)
    let verts_front = make_main_face(x, y, z);
    polygons.push(CubePolygon {
        uvs: Some(face_uvs(&verts_front, x, y, true, false)),
        verts: verts_front,
        normal: Vec3::Z,
        color: front_color,
        label: Some("FRONT"),
        snap_view: Some(CameraSnapView::Front),
    });

    // Rear (-Z)
    let verts_rear = make_main_face(-x, y, -z);
    polygons.push(CubePolygon {
        uvs: Some(face_uvs(&verts_rear, -x, y, true, false)),
        verts: verts_rear,
        normal: Vec3::NEG_Z,
        color: rear_color,
        label: Some("REAR"),
        snap_view: Some(CameraSnapView::Rear),
    });

    // Right (+X)
    let verts_right = make_main_face(-z, y, x);
    polygons.push(CubePolygon {
        uvs: Some(face_uvs(&verts_right, -z, y, true, false)),
        verts: verts_right,
        normal: Vec3::X,
        color: right_color,
        label: Some("RIGHT"),
        snap_view: Some(CameraSnapView::Right),
    });

    // Left (-X)
    let verts_left = make_main_face(z, y, -x);
    polygons.push(CubePolygon {
        uvs: Some(face_uvs(&verts_left, z, y, true, false)),
        verts: verts_left,
        normal: Vec3::NEG_X,
        color: left_color,
        label: Some("LEFT"),
        snap_view: Some(CameraSnapView::Left),
    });

    // ===== EDGE BEVELS (12 quads) =====
    // addCubeFace(x, z - y, Edge, FrontTop)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_x, fc_z - fc_y),
        normal: (Vec3::Y + Vec3::Z).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::FrontTop),
        uvs: None,
    });

    // addCubeFace(x, -z - y, Edge, FrontBottom)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_x, -fc_z - fc_y),
        normal: (Vec3::NEG_Y + Vec3::Z).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::FrontBottom),
        uvs: None,
    });

    // addCubeFace(x, y - z, Edge, RearBottom)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_x, fc_y - fc_z),
        normal: (Vec3::NEG_Y + Vec3::NEG_Z).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::RearBottom),
        uvs: None,
    });

    // addCubeFace(x, y + z, Edge, RearTop)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_x, fc_y + fc_z),
        normal: (Vec3::Y + Vec3::NEG_Z).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::RearTop),
        uvs: None,
    });

    // addCubeFace(z, x + y, Edge, RearRight)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_z, fc_x + fc_y),
        normal: (Vec3::X + Vec3::NEG_Z).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::RearRight),
        uvs: None,
    });

    // addCubeFace(z, x - y, Edge, FrontRight)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_z, fc_x - fc_y),
        normal: (Vec3::X + Vec3::Z).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::FrontRight),
        uvs: None,
    });

    // addCubeFace(z, -x - y, Edge, FrontLeft)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_z, -fc_x - fc_y),
        normal: (Vec3::NEG_X + Vec3::Z).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::FrontLeft),
        uvs: None,
    });

    // addCubeFace(z, y - x, Edge, RearLeft)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_z, fc_y - fc_x),
        normal: (Vec3::NEG_X + Vec3::NEG_Z).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::RearLeft),
        uvs: None,
    });

    // addCubeFace(y, z - x, Edge, TopLeft)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_y, fc_z - fc_x),
        normal: (Vec3::NEG_X + Vec3::Y).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::TopLeft),
        uvs: None,
    });

    // addCubeFace(y, x + z, Edge, TopRight)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_y, fc_x + fc_z),
        normal: (Vec3::X + Vec3::Y).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::TopRight),
        uvs: None,
    });

    // addCubeFace(y, x - z, Edge, BottomRight)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_y, fc_x - fc_z),
        normal: (Vec3::X + Vec3::NEG_Y).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::BottomRight),
        uvs: None,
    });

    // addCubeFace(y, -z - x, Edge, BottomLeft)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_y, -fc_z - fc_x),
        normal: (Vec3::NEG_X + Vec3::NEG_Y).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::BottomLeft),
        uvs: None,
    });

    // ===== CORNER BEVELS (8 hexagons) =====
    // prepare() calls with exact parameters

    // addCubeFace(-x - y, x - y + z, Corner, FrontTopRight)
    polygons.push(CubePolygon {
        verts: make_corner_face(-fc_x - fc_y, fc_x - fc_y + fc_z),
        normal: (Vec3::X + Vec3::Y + Vec3::Z).normalize(),
        color: corner_color,
        label: None,
        snap_view: Some(CameraSnapView::FrontTopRight),
        uvs: None,
    });

    // addCubeFace(-x + y, -x - y + z, Corner, FrontTopLeft)
    polygons.push(CubePolygon {
        verts: make_corner_face(-fc_x + fc_y, -fc_x - fc_y + fc_z),
        normal: (Vec3::NEG_X + Vec3::Y + Vec3::Z).normalize(),
        color: corner_color,
        label: None,
        snap_view: Some(CameraSnapView::FrontTopLeft),
        uvs: None,
    });

    // addCubeFace(x + y, x - y - z, Corner, FrontBottomRight)
    polygons.push(CubePolygon {
        verts: make_corner_face(fc_x + fc_y, fc_x - fc_y - fc_z),
        normal: (Vec3::X + Vec3::NEG_Y + Vec3::Z).normalize(),
        color: corner_color,
        label: None,
        snap_view: Some(CameraSnapView::FrontBottomRight),
        uvs: None,
    });

    // addCubeFace(x - y, -x - y - z, Corner, FrontBottomLeft)
    polygons.push(CubePolygon {
        verts: make_corner_face(fc_x - fc_y, -fc_x - fc_y - fc_z),
        normal: (Vec3::NEG_X + Vec3::NEG_Y + Vec3::Z).normalize(),
        color: corner_color,
        label: None,
        snap_view: Some(CameraSnapView::FrontBottomLeft),
        uvs: None,
    });

    // addCubeFace(x - y, x + y + z, Corner, RearTopRight)
    polygons.push(CubePolygon {
        verts: make_corner_face(fc_x - fc_y, fc_x + fc_y + fc_z),
        normal: (Vec3::X + Vec3::Y + Vec3::NEG_Z).normalize(),
        color: corner_color,
        label: None,
        snap_view: Some(CameraSnapView::RearTopRight),
        uvs: None,
    });

    // addCubeFace(x + y, -x + y + z, Corner, RearTopLeft)
    polygons.push(CubePolygon {
        verts: make_corner_face(fc_x + fc_y, -fc_x + fc_y + fc_z),
        normal: (Vec3::NEG_X + Vec3::Y + Vec3::NEG_Z).normalize(),
        color: corner_color,
        label: None,
        snap_view: Some(CameraSnapView::RearTopLeft),
        uvs: None,
    });

    // addCubeFace(-x + y, x + y - z, Corner, RearBottomRight)
    polygons.push(CubePolygon {
        verts: make_corner_face(-fc_x + fc_y, fc_x + fc_y - fc_z),
        normal: (Vec3::X + Vec3::NEG_Y + Vec3::NEG_Z).normalize(),
        color: corner_color,
        label: None,
        snap_view: Some(CameraSnapView::RearBottomRight),
        uvs: None,
    });

    // addCubeFace(-x - y, -x + y - z, Corner, RearBottomLeft)
    polygons.push(CubePolygon {
        verts: make_corner_face(-fc_x - fc_y, -fc_x + fc_y - fc_z),
        normal: (Vec3::NEG_X + Vec3::NEG_Y + Vec3::NEG_Z).normalize(),
        color: corner_color,
        label: None,
        snap_view: Some(CameraSnapView::RearBottomLeft),
        uvs: None,
    });

    // Project to 2D (simple orthographic)
    let project =
        |v: Vec3| -> Pos2 { Pos2::new(center.x - v.x * cube_scale, center.y - v.y * cube_scale) };

    // Calculate depth and sort back-to-front
    let mut poly_data: Vec<_> = polygons
        .iter()
        .map(|poly| {
            let rotated_normal = *rot * poly.normal;
            let transformed_verts: Vec<Vec3> = poly.verts.iter().map(|v| *rot * *v).collect();
            let center_z =
                transformed_verts.iter().map(|v| v.z).sum::<f32>() / transformed_verts.len() as f32;
            (poly, rotated_normal, transformed_verts, center_z)
        })
        .collect();
    poly_data.sort_by(|a, b| a.3.partial_cmp(&b.3).unwrap());

    // Check for click position
    let click_pos = if response.clicked() {
        response.interact_pointer_pos()
    } else {
        None
    };

    // Track which face is hovered
    let mut hovered_label: Option<&'static str> = None;

    // Draw polygons (back to front)
    for (poly, normal, transformed_verts, _depth) in &poly_data {
        // Only draw faces that are visible (facing camera)
        if normal.z <= 0.05 {
            continue;
        }

        // Project vertices to 2D
        let points: Vec<Pos2> = transformed_verts.iter().map(|v| project(*v)).collect();

        // Check if mouse is over this polygon
        let is_hovered = if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            point_in_polygon(pos, &points)
        } else {
            false
        };

        // Check if clicked on this polygon
        if let Some(pos) = click_pos {
            if point_in_polygon(pos, &points) {
                if let Some(snap) = poly.snap_view {
                    clicked_face = Some(snap);
                }
            }
        }

        if is_hovered {
            if let Some(label) = poly.label {
                hovered_label = Some(label);
            }
        }

        // Shade based on normal direction, brighten if hovered
        let base_brightness = (normal.z * 0.4 + 0.6).clamp(0.4, 1.0);
        let brightness = if is_hovered && poly.snap_view.is_some() {
            (base_brightness + 0.2).min(1.0)
        } else {
            base_brightness
        };

        let shaded_color = Color32::from_rgb(
            (poly.color.r() as f32 * brightness) as u8,
            (poly.color.g() as f32 * brightness) as u8,
            (poly.color.b() as f32 * brightness) as u8,
        );

        // Draw filled polygon
        let stroke_color = if is_hovered && poly.snap_view.is_some() {
            Color32::from_gray(150)
        } else {
            Color32::from_gray(60)
        };
        painter.add(egui::Shape::convex_polygon(
            points.clone(),
            shaded_color,
            Stroke::new(0.5, stroke_color),
        ));

        if let (Some(label), Some(uvs)) = (poly.label, &poly.uvs) {
            if normal.z > 0.3 && points.len() >= 3 {
                let text_color = auto_text_color(poly.color);
                if let Some(texture) = get_face_texture(ctx, label, poly.color, text_color) {
                    let mut mesh = EguiMesh::with_texture(texture.id());
                    for (pos, uv) in points.iter().zip(uvs.iter()) {
                        mesh.vertices.push(EguiVertex {
                            pos: *pos,
                            uv: Pos2::new(uv[0], uv[1]),
                            color: Color32::WHITE,
                        });
                    }
                    for idx in 1..(points.len() - 1) {
                        mesh.indices
                            .extend_from_slice(&[0, idx as u32, (idx as u32 + 1)]);
                    }
                    painter.add(egui::Shape::mesh(mesh));
                }
            }
        }
    }

    // Show tooltip for hovered face
    if let Some(label) = hovered_label {
        response.clone().on_hover_ui_at_pointer(|ui| {
            ui.label(format!("Click to view {}", label));
        });
    }

    clicked_face
}

/// Draws the colored axis arrows (X=red, Y=green, Z=blue)
fn draw_axis_arrows(painter: &egui::Painter, center: Pos2, rot: &Mat3, axis_system: AxisSystem) {
    let axis_origin = Pos2::new(center.x - 35.0, center.y + 35.0);
    let axis_len = 18.0;

    let mut axis_data: Vec<_> = [
        (
            Vec3::X,
            Color32::from_rgb(220, 80, 80),
            axis_system.horizontal(),
        ),
        (
            Vec3::Y,
            Color32::from_rgb(80, 200, 80),
            axis_system.vertical(),
        ),
        (
            Vec3::Z,
            Color32::from_rgb(80, 120, 220),
            axis_system.depth(),
        ),
    ]
    .into_iter()
    .map(|(dir, color, axis)| {
        let rotated = *rot * dir;
        (rotated, color, axis)
    })
    .collect();
    axis_data.sort_by(|a, b| a.0.z.partial_cmp(&b.0.z).unwrap());

    for (rotated, color, axis) in &axis_data {
        let end = Pos2::new(
            axis_origin.x + rotated.x * axis_len,
            axis_origin.y - rotated.y * axis_len,
        );

        let alpha = ((rotated.z + 1.0) * 0.35 + 0.3).clamp(0.3, 1.0);
        let faded =
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (alpha * 255.0) as u8);

        let thickness = if rotated.z > 0.0 { 2.5 } else { 1.5 };
        painter.line_segment([axis_origin, end], Stroke::new(thickness, faded));

        // Arrow head
        if rotated.z > -0.5 {
            let dir_2d = (end - axis_origin).normalized();
            let perp = egui::Vec2::new(-dir_2d.y, dir_2d.x);
            let arrow_size = 4.0;
            let arrow_points = vec![
                end,
                end - dir_2d * arrow_size + perp * arrow_size * 0.5,
                end - dir_2d * arrow_size - perp * arrow_size * 0.5,
            ];
            painter.add(egui::Shape::convex_polygon(
                arrow_points,
                faded,
                Stroke::NONE,
            ));
        }

        // Label
        if rotated.z > 0.0 {
            let label_pos = Pos2::new(
                axis_origin.x + rotated.x * (axis_len + 10.0),
                axis_origin.y - rotated.y * (axis_len + 10.0),
            );
            painter.text(
                label_pos,
                egui::Align2::CENTER_CENTER,
                axis.signed_label(),
                egui::FontId::proportional(10.0),
                faded,
            );
        }
    }
}

/// Draws interactive rotation arrows around the circle
fn draw_rotation_arrows_interactive(
    ui: &Ui,
    painter: &egui::Painter,
    center: Pos2,
    widget_size: f32,
    response: &Response,
    y_offset: f32,
) -> Option<RotateDelta> {
    let mut result: Option<RotateDelta> = None;
    let arrow_radius = widget_size / 2.0 - 2.0;
    let arrow_color = Color32::from_gray(100);
    let hover_color = Color32::from_gray(180);

    let click_pos = if response.clicked() {
        response.interact_pointer_pos()
    } else {
        None
    };

    let hover_pos = ui.input(|i| i.pointer.hover_pos());

    // === Triangle arrows pointing outward (right, left, bottom, top) ===
    // Arrow positions: 0=right, 90=bottom, 180=left, 270=top
    //
    // Screen-space rotation (around camera's local axes):
    // - ScreenY: rotate around camera's UP axis
    //   - Positive = rotate left (view shifts left)
    //   - Negative = rotate right (view shifts right)
    // - ScreenX: rotate around camera's RIGHT axis
    //   - Positive = rotate up (view shifts up)
    //   - Negative = rotate down (view shifts down)
    let triangle_arrows = [
        (0.0_f32, RotateAxis::ScreenY, -45.0), // Right arrow -> rotate right 45°
        (180.0, RotateAxis::ScreenY, 45.0),    // Left arrow -> rotate left 45°
        (90.0, RotateAxis::ScreenX, 45.0),     // Bottom arrow -> rotate down 45°
        (270.0, RotateAxis::ScreenX, -45.0),   // Top arrow -> rotate up 45°
    ];

    let triangle_size = 10.0;
    let triangle_base = 16.0;

    for (angle_deg, axis, degrees) in triangle_arrows {
        let angle = angle_deg.to_radians();

        // Triangle tip points outward from center
        let tip = Pos2::new(
            center.x + angle.cos() * arrow_radius,
            center.y + angle.sin() * arrow_radius,
        );

        // Direction pointing outward
        let outward = egui::Vec2::new(angle.cos(), angle.sin());
        let perp = egui::Vec2::new(-angle.sin(), angle.cos());

        // Triangle base is inward from the tip
        let base_center = tip - outward * triangle_size;
        let p1 = base_center + perp * (triangle_base / 2.0);
        let p2 = base_center - perp * (triangle_base / 2.0);

        let triangle_pts = vec![tip, p1, p2];

        // Hit testing: use actual triangle shape
        let is_hovered = hover_pos
            .map(|p| point_in_polygon(p, &triangle_pts))
            .unwrap_or(false);

        let is_clicked = click_pos
            .map(|p| point_in_polygon(p, &triangle_pts))
            .unwrap_or(false);

        if is_clicked {
            result = Some(RotateDelta { degrees, axis });
        }

        let color = if is_hovered { hover_color } else { arrow_color };
        painter.add(egui::Shape::convex_polygon(
            triangle_pts,
            color,
            Stroke::NONE,
        ));
    }

    // === Arc arrows at the top (for horizontal rotation) ===
    let arc_width = 10.0;
    let arc_radius = widget_size / 2.0 + arc_width + 4.0; // Slightly outside the circle
    let arc_y_offset = -widget_size / 2.0 - 2.0 - y_offset; // Above the top of the circle
    let arc_center = Pos2::new(center.x, center.y + arc_y_offset + arc_radius);

    // Left-pointing arc arrow (rotate scene counter-clockwise = yaw+)
    // User clicks left arc = wants scene to rotate left = yaw+
    draw_arc_arrow(
        ui,
        painter,
        arc_center,
        arc_radius,
        arc_width,
        std::f32::consts::PI - 0.3, // End angle
        std::f32::consts::PI + 0.3, // Start angle (left side, going up)
        true,                       // Arrow points left (counter-clockwise)
        RotateAxis::ScreenZ,
        45.0, // Swapped: positive = counter-clockwise
        arrow_color,
        hover_color,
        &click_pos,
        &hover_pos,
        &mut result,
    );

    // Right-pointing arc arrow (rotate scene clockwise = yaw-)
    // User clicks right arc = wants scene to rotate right = yaw-
    draw_arc_arrow(
        ui,
        painter,
        arc_center,
        arc_radius,
        arc_width,
        -0.3,  // Start angle (right side)
        0.3,   // End angle
        false, // Arrow points right (clockwise)
        RotateAxis::ScreenZ,
        -45.0, // Swapped: negative = clockwise
        arrow_color,
        hover_color,
        &click_pos,
        &hover_pos,
        &mut result,
    );

    result
}

fn face_uvs(
    verts: &[Vec3],
    x_axis: Vec3,
    y_axis: Vec3,
    flip_u: bool,
    flip_v: bool,
) -> Vec<[f32; 2]> {
    let x_axis = x_axis.normalize();
    let y_axis = y_axis.normalize();
    verts
        .iter()
        .map(|v| {
            let mut u = 0.5 + v.dot(x_axis) * 0.5;
            let mut v_coord = 0.5 - v.dot(y_axis) * 0.5;
            if flip_u {
                u = 1.0 - u;
            }
            if flip_v {
                v_coord = 1.0 - v_coord;
            }
            [u, v_coord]
        })
        .collect()
}

fn auto_text_color(bg: Color32) -> Color32 {
    let r = bg.r() as f32 / 255.0;
    let g = bg.g() as f32 / 255.0;
    let b = bg.b() as f32 / 255.0;
    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    if luminance > 0.6 {
        Color32::from_rgb(30, 30, 30)
    } else {
        Color32::from_rgb(240, 240, 240)
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct FaceKey {
    label: &'static str,
    background: [u8; 3],
    text: [u8; 3],
}

impl FaceKey {
    fn new(label: &'static str, background: Color32, text: Color32) -> Self {
        Self {
            label,
            background: [background.r(), background.g(), background.b()],
            text: [text.r(), text.g(), text.b()],
        }
    }
}

#[derive(Clone, Default)]
struct FaceTextureCache {
    handles: HashMap<FaceKey, TextureHandle>,
}

fn get_face_texture(
    ctx: &Context,
    label: &'static str,
    background: Color32,
    text: Color32,
) -> Option<TextureHandle> {
    let key = FaceKey::new(label, background, text);
    let cache_id = Id::new("orientation_cube_face_textures");

    if let Some(handle) = ctx.data(|data| {
        data.get_temp::<FaceTextureCache>(cache_id)
            .and_then(|cache| cache.handles.get(&key).cloned())
    }) {
        return Some(handle);
    }

    let texture = create_face_texture(ctx, &key)?;

    ctx.data_mut(|data| {
        let cache = data.get_temp_mut_or_insert_with(cache_id, FaceTextureCache::default);
        cache.handles.insert(key.clone(), texture.clone());
    });

    Some(texture)
}

fn create_face_texture(ctx: &Context, key: &FaceKey) -> Option<TextureHandle> {
    let svg = FACE_TEMPLATE_SVG
        .replace("{{BACKGROUND_COLOR}}", &rgb_to_hex(key.background))
        .replace("{{TEXT_COLOR}}", &rgb_to_hex(key.text))
        .replace("{{LABEL}}", key.label);
    let image = rasterize_svg(&svg)?;
    let name = format!(
        "orientation_cube_face_{}_{}_{}",
        key.label,
        rgb_to_hex(key.background),
        rgb_to_hex(key.text)
    );
    Some(ctx.load_texture(name, image, TextureOptions::LINEAR))
}

fn rgb_to_hex(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

fn rasterize_svg(svg: &str) -> Option<ColorImage> {
    let mut opt = Options::default();
    opt.font_family = "DejaVu Sans".into();
    opt.languages = vec!["en".into()];
    opt.font_size = 44.0;
    let mut fontdb = fontdb::Database::new();
    fontdb.load_system_fonts();
    let tree = usvg::Tree::from_data(svg.as_bytes(), &opt, &fontdb).ok()?;
    let size = tree.size().to_int_size();
    let (width, height) = (size.width(), size.height());
    let mut pixmap = Pixmap::new(width, height)?;
    let mut pixmap_mut = pixmap.as_mut();
    render(&tree, tiny_skia::Transform::identity(), &mut pixmap_mut);
    let data = pixmap.data().to_vec();
    Some(ColorImage::from_rgba_premultiplied(
        [width as usize, height as usize],
        &data,
    ))
}

fn angle_in_range(theta: f32, start: f32, end: f32) -> bool {
    let mut t = theta;
    let mut s = start;
    let mut e = end;
    // Normalize to [0, 2π)
    let two_pi = std::f32::consts::TAU;
    let norm = |a: f32| {
        let mut v = a % two_pi;
        if v < 0.0 {
            v += two_pi;
        }
        v
    };
    t = norm(t);
    s = norm(s);
    e = norm(e);
    if s <= e {
        t >= s && t <= e
    } else {
        // Wrapped around 2π
        t >= s || t <= e
    }
}

fn hit_test_arc_arrow(
    p: Pos2,
    center: Pos2,
    radius: f32,
    width: f32,
    start_angle: f32,
    end_angle: f32,
    arrow_at_start: bool,
) -> bool {
    let v = p - center;
    let r = v.length();
    if r == 0.0 {
        return false;
    }
    let theta = v.y.atan2(v.x);

    // Check hit against the circular band of the arc
    let half_w = width * 0.5 + 1.5;
    let in_radius = (r >= radius - half_w) && (r <= radius + half_w);
    let in_angle = angle_in_range(theta, start_angle, end_angle);
    if in_radius && in_angle {
        return true;
    }

    // Also include the arrow head triangle at the start or end of the arc
    let delta_angle = width / radius;
    let arrow_angle = if arrow_at_start {
        start_angle - delta_angle
    } else {
        end_angle + delta_angle
    };

    let arrow_tip = Pos2::new(
        center.x + arrow_angle.cos() * radius,
        center.y + arrow_angle.sin() * radius,
    );

    // Tangent direction (perpendicular to radius)
    let tangent_dir = if arrow_at_start { -1.0 } else { 1.0 };
    let tangent = egui::Vec2::new(-arrow_angle.sin(), arrow_angle.cos()) * tangent_dir;
    let normal = egui::Vec2::new(arrow_angle.cos(), arrow_angle.sin());

    let arrow_pts = vec![
        arrow_tip,
        arrow_tip - tangent * (width + 2.5) + normal * (width + 0.5),
        arrow_tip - tangent * (width + 2.5) - normal * (width + 0.5),
    ];

    point_in_polygon(p, &arrow_pts)
}

/// Helper to draw an arc arrow with interaction
#[allow(clippy::too_many_arguments)]
fn draw_arc_arrow(
    _ui: &Ui,
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    width: f32,
    start_angle: f32,
    end_angle: f32,
    arrow_at_start: bool, // If true, arrow head at start; if false, at end
    axis: RotateAxis,
    degrees: f32,
    base_color: Color32,
    hover_color: Color32,
    click_pos: &Option<Pos2>,
    hover_pos: &Option<Pos2>,
    result: &mut Option<RotateDelta>,
) {
    // Hit testing: match the visual arc band + arrow head more closely
    let is_hovered = hover_pos
        .map(|p| {
            hit_test_arc_arrow(
                p,
                center,
                radius,
                width,
                start_angle,
                end_angle,
                arrow_at_start,
            )
        })
        .unwrap_or(false);

    let is_clicked = click_pos
        .map(|p| {
            hit_test_arc_arrow(
                p,
                center,
                radius,
                width,
                start_angle,
                end_angle,
                arrow_at_start,
            )
        })
        .unwrap_or(false);

    if is_clicked {
        *result = Some(RotateDelta { degrees, axis });
    }

    let color = if is_hovered { hover_color } else { base_color };
    let stroke_width = if is_hovered { width + 1.0 } else { width };

    // Draw arc
    let segments = 12;
    let mut points = Vec::new();
    for i in 0..=segments {
        let t = start_angle + (end_angle - start_angle) * (i as f32 / segments as f32);
        points.push(Pos2::new(
            center.x + t.cos() * radius,
            center.y + t.sin() * radius,
        ));
    }

    for i in 0..points.len() - 1 {
        painter.line_segment([points[i], points[i + 1]], Stroke::new(stroke_width, color));
    }

    // Arrow head
    let delta_angle = stroke_width / radius;

    let arrow_angle = if arrow_at_start {
        start_angle - delta_angle
    } else {
        end_angle + delta_angle
    };

    let arrow_tip = Pos2::new(
        center.x + arrow_angle.cos() * radius,
        center.y + arrow_angle.sin() * radius,
    );

    // Tangent direction (perpendicular to radius)
    let tangent_dir = if arrow_at_start { -1.0 } else { 1.0 };
    let tangent = egui::Vec2::new(-arrow_angle.sin(), arrow_angle.cos()) * tangent_dir;
    let normal = egui::Vec2::new(arrow_angle.cos(), arrow_angle.sin());

    let arrow_pts = vec![
        arrow_tip,
        arrow_tip - tangent * (stroke_width + 2.5) + normal * (stroke_width + 0.5),
        arrow_tip - tangent * (stroke_width + 2.5) - normal * (stroke_width + 0.5),
    ];
    painter.add(egui::Shape::convex_polygon(arrow_pts, color, Stroke::NONE));
}

/// Point-in-polygon test using ray casting
fn point_in_polygon(point: Pos2, polygon: &[Pos2]) -> bool {
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
