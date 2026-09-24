//! The arrows round the cube: the axis triad, and the curved and straight
//! arrows that turn the view by a step.

use super::*;

/// Draws the colored axis arrows for **world** X, Y, and Z (red / green / blue).
/// `rot` is [`world_axes_widget_rotation`] — settings axis + camera quaternion, same widget basis as the cube.
pub(super) fn draw_axis_arrows(painter: &egui::Painter, axis_origin: Pos2, rot: &Mat3) {
    let axis_len = 18.0;

    let mut axis_data: Vec<_> = [
        (Vec3::X, ui_kit::tokens::AXIS_X, "X"),
        (Vec3::Y, ui_kit::tokens::AXIS_Y, "Y"),
        (Vec3::Z, ui_kit::tokens::AXIS_Z, "Z"),
    ]
    .into_iter()
    .map(|(dir, color, label)| {
        let rotated = *rot * dir;
        (rotated, color, label)
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

        let thickness = if rotated.z > 0.0 { 2.5_f32 } else { 1.5_f32 };
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
                *label,
                egui::FontId::proportional(10.0),
                faded,
            );
        }
    }
}

/// Draws interactive rotation arrows around the circle
pub(super) fn draw_rotation_arrows_interactive(
    ui: &Ui,
    painter: &egui::Painter,
    center: Pos2,
    widget_size: f32,
    response: &Response,
    y_offset: f32,
    planar_only: bool,
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
    //   - Positive = rotate right (view shifts right)
    //   - Negative = rotate left (view shifts left)
    // - ScreenX: rotate around camera's RIGHT axis
    //   - Positive = rotate up (view shifts up)
    //   - Negative = rotate down (view shifts down)
    let triangle_arrows = [
        (0.0_f32, RotateAxis::ScreenY, 45.0), // Right arrow -> rotate right 45°
        (180.0, RotateAxis::ScreenY, -45.0),  // Left arrow -> rotate left 45°
        (90.0, RotateAxis::ScreenX, 45.0),    // Bottom arrow -> rotate down 45°
        (270.0, RotateAxis::ScreenX, -45.0),  // Top arrow -> rotate up 45°
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

        // Pitch and yaw would tilt the view off the sketch plane.
        let inert = planar_only && !axis.keeps_view_direction();
        if is_clicked && !inert {
            result = Some(RotateDelta { degrees, axis });
        }

        let color = if inert {
            Color32::from_gray(58)
        } else if is_hovered {
            hover_color
        } else {
            arrow_color
        };
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

    // Left-pointing arc arrow (yaw around view direction)
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
        -45.0,
        arrow_color,
        hover_color,
        &click_pos,
        &hover_pos,
        &mut result,
    );

    // Right-pointing arc arrow (yaw around view direction)
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
        45.0,
        arrow_color,
        hover_color,
        &click_pos,
        &hover_pos,
        &mut result,
    );

    result
}

pub(super) fn angle_in_range(theta: f32, start: f32, end: f32) -> bool {
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

pub(super) fn hit_test_arc_arrow(
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
pub(super) fn draw_arc_arrow(
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
