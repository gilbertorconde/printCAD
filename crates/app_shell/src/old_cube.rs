//! Orientation cube widget for showing camera orientation in the 3D viewport.
//!
//! This module provides a visual indicator of the current camera orientation,
//! rendered as a 3D cube with labeled faces (FRONT, REAR, LEFT, RIGHT, TOP, BOTTOM)
//! and colored axis arrows (X=red, Y=green, Z=blue).
//!
//! The cube is interactive: clicking faces snaps to that view, clicking arrows rotates 45°.

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
            widget_size: 120.0,
            cube_scale: 28.0,
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
    Front,
    Rear,
    Left,
    Right,
    Top,
    Bottom,
}

impl CameraSnapView {
    /// Get the yaw and pitch angles (in degrees) for this view.
    /// Used by the turntable camera system.
    pub fn yaw_pitch(&self) -> (f32, f32) {
        match self {
            // Front: camera at +Z looking toward -Z (yaw=0, pitch=0)
            CameraSnapView::Front => (0.0, 0.0),
            // Rear: camera at -Z looking toward +Z (yaw=180, pitch=0)
            CameraSnapView::Rear => (180.0, 0.0),
            // Right: camera at +X looking toward -X (yaw=90, pitch=0)
            CameraSnapView::Right => (90.0, 0.0),
            // Left: camera at -X looking toward +X (yaw=-90, pitch=0)
            CameraSnapView::Left => (-90.0, 0.0),
            // Top: camera above looking down (yaw=0, pitch=-90)
            CameraSnapView::Top => (0.0, -90.0),
            // Bottom: camera below looking up (yaw=0, pitch=90)
            CameraSnapView::Bottom => (0.0, 90.0),
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

/// Face definition for the cube
struct CubeFace {
    /// Vertex indices (4 corners)
    indices: [usize; 4],
    /// Normal direction in world space
    normal: Vec3,
    /// Base color for this face
    color: Color32,
    /// Label to display on the face
    label: &'static str,
    /// Snap view when clicked
    snap_view: CameraSnapView,
}

/// Draws the orientation cube widget and returns interaction results
pub fn draw(
    ctx: &Context,
    input: &OrientationCubeInput,
    config: &OrientationCubeConfig,
) -> OrientationCubeResult {
    let mut result = OrientationCubeResult::default();

    // Extra space at the top for arc arrows
    let arc_arrow_padding = 30.0;
    let total_height = config.widget_size + arc_arrow_padding;
    let total_width = config.widget_size + arc_arrow_padding;

    let y_offset: f32 = 5.5;

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

            // Draw background circle
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
            let q = Quat::from_array(input.camera_orientation).inverse();
            let rot = Mat3::from_quat(q);

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
                draw_axis_arrows(&painter, local_center, &rot);
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

/// Draws the 3D cube with labeled faces and handles clicks
fn draw_cube_interactive(
    ui: &Ui,
    painter: &egui::Painter,
    center: Pos2,
    cube_scale: f32,
    rot: &Mat3,
    response: &Response,
) -> Option<CameraSnapView> {
    let mut clicked_face: Option<CameraSnapView> = None;

    // Define cube vertices (unit cube centered at origin)
    let vertices: [Vec3; 8] = [
        Vec3::new(-1.0, -1.0, -1.0), // 0: back-bottom-left
        Vec3::new(1.0, -1.0, -1.0),  // 1: back-bottom-right
        Vec3::new(1.0, 1.0, -1.0),   // 2: back-top-right
        Vec3::new(-1.0, 1.0, -1.0),  // 3: back-top-left
        Vec3::new(-1.0, -1.0, 1.0),  // 4: front-bottom-left
        Vec3::new(1.0, -1.0, 1.0),   // 5: front-bottom-right
        Vec3::new(1.0, 1.0, 1.0),    // 6: front-top-right
        Vec3::new(-1.0, 1.0, 1.0),   // 7: front-top-left
    ];

    // Transform vertices
    let transformed: Vec<Vec3> = vertices.iter().map(|v| *rot * *v).collect();

    // Project to 2D (simple orthographic)
    let project =
        |v: Vec3| -> Pos2 { Pos2::new(center.x - v.x * cube_scale, center.y - v.y * cube_scale) };

    // Define faces
    let faces = [
        CubeFace {
            indices: [4, 5, 6, 7],
            normal: Vec3::Z,
            color: Color32::from_rgb(100, 130, 170),
            label: "FRONT",
            snap_view: CameraSnapView::Front,
        },
        CubeFace {
            indices: [1, 0, 3, 2],
            normal: Vec3::NEG_Z,
            color: Color32::from_rgb(100, 130, 170),
            label: "REAR",
            snap_view: CameraSnapView::Rear,
        },
        CubeFace {
            indices: [5, 1, 2, 6],
            normal: Vec3::X,
            color: Color32::from_rgb(170, 100, 100),
            label: "RIGHT",
            snap_view: CameraSnapView::Right,
        },
        CubeFace {
            indices: [0, 4, 7, 3],
            normal: Vec3::NEG_X,
            color: Color32::from_rgb(100, 170, 100),
            label: "LEFT",
            snap_view: CameraSnapView::Left,
        },
        CubeFace {
            indices: [7, 6, 2, 3],
            normal: Vec3::Y,
            color: Color32::from_rgb(150, 150, 170),
            label: "TOP",
            snap_view: CameraSnapView::Top,
        },
        CubeFace {
            indices: [0, 1, 5, 4],
            normal: Vec3::NEG_Y,
            color: Color32::from_rgb(120, 120, 140),
            label: "BOTTOM",
            snap_view: CameraSnapView::Bottom,
        },
    ];

    // Calculate face depths and sort back-to-front
    let mut face_data: Vec<_> = faces
        .iter()
        .map(|face| {
            let rotated_normal = *rot * face.normal;
            let face_center: Vec3 =
                face.indices.iter().map(|&i| transformed[i]).sum::<Vec3>() / 4.0;
            (face, rotated_normal, face_center.z)
        })
        .collect();
    face_data.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap());

    // Check for click position
    let click_pos = if response.clicked() {
        response.interact_pointer_pos()
    } else {
        None
    };

    // Track which face is hovered/clicked (front-most)
    let mut hovered_face: Option<&CubeFace> = None;

    // Draw faces (back to front) and check for hover/click
    for (face, normal, _depth) in &face_data {
        // Only draw faces that are visible (facing camera)
        if normal.z <= 0.1 {
            continue;
        }

        let points: Vec<Pos2> = face
            .indices
            .iter()
            .map(|&i| project(transformed[i]))
            .collect();

        // Check if mouse is over this face
        let is_hovered = if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            point_in_polygon(pos, &points)
        } else {
            false
        };

        // Check if clicked on this face
        if let Some(pos) = click_pos {
            if point_in_polygon(pos, &points) {
                clicked_face = Some(face.snap_view);
            }
        }

        if is_hovered {
            hovered_face = Some(face);
        }

        // Shade based on normal direction, brighten if hovered
        let base_brightness = (normal.z * 0.4 + 0.6).clamp(0.4, 1.0);
        let brightness = if is_hovered {
            (base_brightness + 0.2).min(1.0)
        } else {
            base_brightness
        };

        let shaded_color = Color32::from_rgb(
            (face.color.r() as f32 * brightness) as u8,
            (face.color.g() as f32 * brightness) as u8,
            (face.color.b() as f32 * brightness) as u8,
        );

        // Draw filled face
        let stroke_color = if is_hovered {
            Color32::from_gray(150)
        } else {
            Color32::from_gray(60)
        };
        painter.add(egui::Shape::convex_polygon(
            points.clone(),
            shaded_color,
            Stroke::new(1.0, stroke_color),
        ));

        // Draw label on face
        let face_center_2d = Pos2::new(
            points.iter().map(|p| p.x).sum::<f32>() / 4.0,
            points.iter().map(|p| p.y).sum::<f32>() / 4.0,
        );

        // Only show label if face is clearly visible
        if normal.z > 0.3 {
            let text_alpha = ((normal.z - 0.3) * 2.0).clamp(0.0, 1.0);
            let text_color =
                Color32::from_rgba_unmultiplied(220, 220, 230, (text_alpha * 255.0) as u8);
            painter.text(
                face_center_2d,
                egui::Align2::CENTER_CENTER,
                face.label,
                egui::FontId::proportional(10.0),
                text_color,
            );
        }
    }

    // Show tooltip for hovered face
    if let Some(face) = hovered_face {
        response.clone().on_hover_ui_at_pointer(|ui| {
            ui.label(format!("Click to view {}", face.label));
        });
    }

    clicked_face
}

/// Draws the colored axis arrows (X=red, Y=green, Z=blue)
fn draw_axis_arrows(painter: &egui::Painter, center: Pos2, rot: &Mat3) {
    let axis_origin = Pos2::new(center.x - 35.0, center.y + 35.0);
    let axis_len = 18.0;

    let axes = [
        (Vec3::X, Color32::from_rgb(220, 80, 80), "X"),  // Red
        (Vec3::Y, Color32::from_rgb(80, 200, 80), "Y"),  // Green
        (Vec3::Z, Color32::from_rgb(80, 120, 220), "Z"), // Blue
    ];

    // Sort axes by depth
    let mut axis_data: Vec<_> = axes
        .iter()
        .map(|(dir, color, label)| {
            let rotated = *rot * *dir;
            (rotated, *color, *label)
        })
        .collect();
    axis_data.sort_by(|a, b| a.0.z.partial_cmp(&b.0.z).unwrap());

    for (rotated, color, label) in &axis_data {
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
                label,
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
    let arc_radius = widget_size / 2.0 + 8.0; // Slightly outside the circle
    let arc_y_offset = -widget_size / 2.0 - 2.0 - y_offset; // Above the top of the circle
    let arc_center = Pos2::new(center.x, center.y + arc_y_offset + arc_radius);

    // Left-pointing arc arrow (rotate scene counter-clockwise = yaw+)
    // User clicks left arc = wants scene to rotate left = yaw+
    draw_arc_arrow(
        ui,
        painter,
        arc_center,
        arc_radius,
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
    let stroke_width = if is_hovered { 2.5 } else { 1.5 };

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
    let arrow_angle = if arrow_at_start {
        start_angle
    } else {
        end_angle
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
        arrow_tip - tangent * 5.0 + normal * 3.0,
        arrow_tip - tangent * 5.0 - normal * 3.0,
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
