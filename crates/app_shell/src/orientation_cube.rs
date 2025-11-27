//! Orientation cube widget for showing camera orientation in the 3D viewport.
//!
//! This module provides a visual indicator of the current camera orientation,
//! rendered as a 3D cube with labeled faces (FRONT, REAR, LEFT, RIGHT, TOP, BOTTOM)
//! and colored axis arrows (X=red, Y=green, Z=blue).
//!
//! The cube is interactive: clicking faces snaps to that view, clicking arrows rotates 45°.

use axes::AxisSystem;
use egui::{Color32, Context, Pos2, Response, Sense, Stroke, Ui};
use glam::{Mat3, Quat, Vec3};

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
    let mut clicked_face: Option<CameraSnapView> = None;

    // Chamfer amount - matches FreeCAD's default of 0.12
    let m = 0.12_f32;

    // Following FreeCAD's NaviCube.cpp geometry exactly:
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

    // FreeCAD's addCubeFace for ShapeId::Main creates an 8-vertex polygon:
    // x2 = x * (1 - m*2), y2 = y * (1 - m*2)
    // x4 = x * (1 - m*4), y4 = y * (1 - m*4)
    // Vertices: z - x2 - y4, z - x4 - y2, z + x4 - y2, z + x2 - y4,
    //           z + x2 + y4, z + x4 + y2, z - x4 + y2, z - x2 + y4

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
    // Following FreeCAD: x_dir is along the edge, z_dir is the edge normal direction
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
    // Following FreeCAD exactly: x_dir is passed, z_dir points toward corner
    // y_dir is computed as x_dir.cross(-z_dir)
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

    // FreeCAD coordinate system: Front=-Y, Top=+Z, Right=+X
    // Our coordinate system:      Front=+Z, Top=+Y, Right=+X
    // Mapping: FreeCAD (x,y,z) -> Ours (x, -z, y)
    // So: fc_x = x, fc_y = -z, fc_z = y

    let x = Vec3::X;
    let y = Vec3::Y;
    let z = Vec3::Z;

    // FreeCAD vectors mapped to our system
    let fc_x = x;
    let fc_y = -z;
    let fc_z = y;

    // ===== MAIN FACES (6 octagons) =====
    // These were working before - using our coordinate system directly

    // Top (+Y)
    polygons.push(CubePolygon {
        verts: make_main_face(x, z, y),
        normal: Vec3::Y,
        color: top_color,
        label: Some("TOP"),
        snap_view: Some(CameraSnapView::Top),
    });

    // Bottom (-Y)
    polygons.push(CubePolygon {
        verts: make_main_face(x, -z, -y),
        normal: Vec3::NEG_Y,
        color: bottom_color,
        label: Some("BOTTOM"),
        snap_view: Some(CameraSnapView::Bottom),
    });

    // Front (+Z)
    polygons.push(CubePolygon {
        verts: make_main_face(x, y, z),
        normal: Vec3::Z,
        color: front_color,
        label: Some("FRONT"),
        snap_view: Some(CameraSnapView::Front),
    });

    // Rear (-Z)
    polygons.push(CubePolygon {
        verts: make_main_face(-x, y, -z),
        normal: Vec3::NEG_Z,
        color: rear_color,
        label: Some("REAR"),
        snap_view: Some(CameraSnapView::Rear),
    });

    // Right (+X)
    polygons.push(CubePolygon {
        verts: make_main_face(-z, y, x),
        normal: Vec3::X,
        color: right_color,
        label: Some("RIGHT"),
        snap_view: Some(CameraSnapView::Right),
    });

    // Left (-X)
    polygons.push(CubePolygon {
        verts: make_main_face(z, y, -x),
        normal: Vec3::NEG_X,
        color: left_color,
        label: Some("LEFT"),
        snap_view: Some(CameraSnapView::Left),
    });

    // ===== EDGE BEVELS (12 quads) =====
    // Using FreeCAD's exact calls with coordinate mapping (like corners)

    // FreeCAD: addCubeFace(x, z - y, Edge, FrontTop)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_x, fc_z - fc_y),
        normal: (Vec3::Y + Vec3::Z).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::FrontTop),
    });

    // FreeCAD: addCubeFace(x, -z - y, Edge, FrontBottom)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_x, -fc_z - fc_y),
        normal: (Vec3::NEG_Y + Vec3::Z).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::FrontBottom),
    });

    // FreeCAD: addCubeFace(x, y - z, Edge, RearBottom)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_x, fc_y - fc_z),
        normal: (Vec3::NEG_Y + Vec3::NEG_Z).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::RearBottom),
    });

    // FreeCAD: addCubeFace(x, y + z, Edge, RearTop)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_x, fc_y + fc_z),
        normal: (Vec3::Y + Vec3::NEG_Z).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::RearTop),
    });

    // FreeCAD: addCubeFace(z, x + y, Edge, RearRight)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_z, fc_x + fc_y),
        normal: (Vec3::X + Vec3::NEG_Z).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::RearRight),
    });

    // FreeCAD: addCubeFace(z, x - y, Edge, FrontRight)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_z, fc_x - fc_y),
        normal: (Vec3::X + Vec3::Z).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::FrontRight),
    });

    // FreeCAD: addCubeFace(z, -x - y, Edge, FrontLeft)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_z, -fc_x - fc_y),
        normal: (Vec3::NEG_X + Vec3::Z).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::FrontLeft),
    });

    // FreeCAD: addCubeFace(z, y - x, Edge, RearLeft)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_z, fc_y - fc_x),
        normal: (Vec3::NEG_X + Vec3::NEG_Z).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::RearLeft),
    });

    // FreeCAD: addCubeFace(y, z - x, Edge, TopLeft)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_y, fc_z - fc_x),
        normal: (Vec3::NEG_X + Vec3::Y).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::TopLeft),
    });

    // FreeCAD: addCubeFace(y, x + z, Edge, TopRight)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_y, fc_x + fc_z),
        normal: (Vec3::X + Vec3::Y).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::TopRight),
    });

    // FreeCAD: addCubeFace(y, x - z, Edge, BottomRight)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_y, fc_x - fc_z),
        normal: (Vec3::X + Vec3::NEG_Y).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::BottomRight),
    });

    // FreeCAD: addCubeFace(y, -z - x, Edge, BottomLeft)
    polygons.push(CubePolygon {
        verts: make_edge_face(fc_y, -fc_z - fc_x),
        normal: (Vec3::NEG_X + Vec3::NEG_Y).normalize(),
        color: edge_color,
        label: None,
        snap_view: Some(CameraSnapView::BottomLeft),
    });

    // ===== CORNER BEVELS (8 hexagons) =====
    // FreeCAD's prepare() calls with exact parameters

    // FreeCAD: addCubeFace(-x - y, x - y + z, Corner, FrontTopRight)
    polygons.push(CubePolygon {
        verts: make_corner_face(-fc_x - fc_y, fc_x - fc_y + fc_z),
        normal: (Vec3::X + Vec3::Y + Vec3::Z).normalize(),
        color: corner_color,
        label: None,
        snap_view: Some(CameraSnapView::FrontTopRight),
    });

    // FreeCAD: addCubeFace(-x + y, -x - y + z, Corner, FrontTopLeft)
    polygons.push(CubePolygon {
        verts: make_corner_face(-fc_x + fc_y, -fc_x - fc_y + fc_z),
        normal: (Vec3::NEG_X + Vec3::Y + Vec3::Z).normalize(),
        color: corner_color,
        label: None,
        snap_view: Some(CameraSnapView::FrontTopLeft),
    });

    // FreeCAD: addCubeFace(x + y, x - y - z, Corner, FrontBottomRight)
    polygons.push(CubePolygon {
        verts: make_corner_face(fc_x + fc_y, fc_x - fc_y - fc_z),
        normal: (Vec3::X + Vec3::NEG_Y + Vec3::Z).normalize(),
        color: corner_color,
        label: None,
        snap_view: Some(CameraSnapView::FrontBottomRight),
    });

    // FreeCAD: addCubeFace(x - y, -x - y - z, Corner, FrontBottomLeft)
    polygons.push(CubePolygon {
        verts: make_corner_face(fc_x - fc_y, -fc_x - fc_y - fc_z),
        normal: (Vec3::NEG_X + Vec3::NEG_Y + Vec3::Z).normalize(),
        color: corner_color,
        label: None,
        snap_view: Some(CameraSnapView::FrontBottomLeft),
    });

    // FreeCAD: addCubeFace(x - y, x + y + z, Corner, RearTopRight)
    polygons.push(CubePolygon {
        verts: make_corner_face(fc_x - fc_y, fc_x + fc_y + fc_z),
        normal: (Vec3::X + Vec3::Y + Vec3::NEG_Z).normalize(),
        color: corner_color,
        label: None,
        snap_view: Some(CameraSnapView::RearTopRight),
    });

    // FreeCAD: addCubeFace(x + y, -x + y + z, Corner, RearTopLeft)
    polygons.push(CubePolygon {
        verts: make_corner_face(fc_x + fc_y, -fc_x + fc_y + fc_z),
        normal: (Vec3::NEG_X + Vec3::Y + Vec3::NEG_Z).normalize(),
        color: corner_color,
        label: None,
        snap_view: Some(CameraSnapView::RearTopLeft),
    });

    // FreeCAD: addCubeFace(-x + y, x + y - z, Corner, RearBottomRight)
    polygons.push(CubePolygon {
        verts: make_corner_face(-fc_x + fc_y, fc_x + fc_y - fc_z),
        normal: (Vec3::X + Vec3::NEG_Y + Vec3::NEG_Z).normalize(),
        color: corner_color,
        label: None,
        snap_view: Some(CameraSnapView::RearBottomRight),
    });

    // FreeCAD: addCubeFace(-x - y, -x + y - z, Corner, RearBottomLeft)
    polygons.push(CubePolygon {
        verts: make_corner_face(-fc_x - fc_y, -fc_x + fc_y - fc_z),
        normal: (Vec3::NEG_X + Vec3::NEG_Y + Vec3::NEG_Z).normalize(),
        color: corner_color,
        label: None,
        snap_view: Some(CameraSnapView::RearBottomLeft),
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

        // Draw label on main faces - rotated to follow the face
        if let Some(label) = poly.label {
            if normal.z > 0.3 {
                let face_center_2d = Pos2::new(
                    points.iter().map(|p| p.x).sum::<f32>() / points.len() as f32,
                    points.iter().map(|p| p.y).sum::<f32>() / points.len() as f32,
                );
                let text_color = Color32::from_rgba_unmultiplied(220, 220, 230, 255);

                // Calculate the face's local X axis (text direction) from the polygon vertices
                // For an 8-vertex octagon, vertices 0-3 are along the bottom, 4-7 along the top
                // Use the direction from left to right edge
                if transformed_verts.len() >= 4 {
                    // Get the face's local X direction from projected 2D positions
                    let left_bottom = transformed_verts[0];
                    let right_bottom = transformed_verts[3];
                    let left_2d = project(left_bottom);
                    let right_2d = project(right_bottom);

                    // Direction from left to right in screen space
                    let dir = right_2d - left_2d;
                    // Add PI to flip the text right-side up
                    let angle = dir.y.atan2(dir.x) + std::f32::consts::PI;

                    // Calculate text scale based on face size
                    let face_width = (right_2d - left_2d).length();
                    let font_size = (face_width * 0.22).clamp(6.0, 14.0);

                    // Draw rotated text using galley
                    let galley = painter.layout_no_wrap(
                        label.to_string(),
                        egui::FontId::proportional(font_size),
                        text_color,
                    );

                    // Calculate offset to center the text
                    // The text is drawn from pos, so we need to offset by half width/height
                    // But since the text is rotated, we need to rotate the offset too
                    let text_width = galley.size().x;
                    let text_height = galley.size().y;
                    let cos_a = angle.cos();
                    let sin_a = angle.sin();
                    // Offset in text-local space: (-width/2, -height/2)
                    // Rotate to screen space
                    let offset_x = -text_width / 2.0 * cos_a + text_height / 2.0 * sin_a;
                    let offset_y = -text_width / 2.0 * sin_a - text_height / 2.0 * cos_a;
                    let centered_pos =
                        Pos2::new(face_center_2d.x + offset_x, face_center_2d.y + offset_y);

                    // Create a rotated text shape
                    let text_shape = egui::Shape::Text(egui::epaint::TextShape {
                        pos: centered_pos,
                        galley,
                        underline: egui::Stroke::NONE,
                        fallback_color: text_color,
                        override_text_color: Some(text_color),
                        opacity_factor: 1.0,
                        angle,
                    });
                    painter.add(text_shape);
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

        // Hit testing
        let hit_center = tip - outward * (triangle_size / 2.0);
        let hit_radius = 10.0;
        let is_hovered = hover_pos
            .map(|p| (p - hit_center).length() < hit_radius)
            .unwrap_or(false);

        let is_clicked = click_pos
            .map(|p| (p - hit_center).length() < hit_radius)
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
    // Calculate arc midpoint for hit testing
    let mid_angle = (start_angle + end_angle) / 2.0;
    let arc_mid = Pos2::new(
        center.x + mid_angle.cos() * radius,
        center.y + mid_angle.sin() * radius,
    );

    let hit_radius = 12.0;
    let is_hovered = hover_pos
        .map(|p| (p - arc_mid).length() < hit_radius)
        .unwrap_or(false);

    let is_clicked = click_pos
        .map(|p| (p - arc_mid).length() < hit_radius)
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
