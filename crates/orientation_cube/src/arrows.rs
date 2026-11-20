//! The arrows round the cube: the axis triad, and the curved and straight
//! arrows that turn the view by a step.

use super::*;

/// Draws the colored axis arrows for **world** X, Y, and Z (red / green / blue).
/// `rot` is [`world_axes_widget_rotation`]: settings axis and camera quaternion, the cube's own widget basis.
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

/// Draws the step arrows around the cube's circle and returns the turn a
/// click on one asks for. With `planar_only`, the ones that would tilt the
/// view off its plane draw inert.
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

    // Triangle arrows pointing outward at 0° (right), 90° (bottom), 180°
    // (left) and 270° (top), turning about the camera's own axes: ScreenY
    // about its up axis, ScreenX about its right axis.
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

        let outward = egui::Vec2::new(angle.cos(), angle.sin());
        let perp = egui::Vec2::new(-angle.sin(), angle.cos());

        let base_center = tip - outward * triangle_size;
        let p1 = base_center + perp * (triangle_base / 2.0);
        let p2 = base_center - perp * (triangle_base / 2.0);

        let triangle_pts = vec![tip, p1, p2];

        // Hit test against the triangle itself, not its bounding box.
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

    // Arc arrows just outside the circle, one at its left and one at its
    // right (angles near π and 0), roll the view about its direction.
    let arc_width = 10.0;
    let arc_radius = widget_size / 2.0 + arc_width + 4.0; // Slightly outside the circle
    let arc_y_offset = -widget_size / 2.0 - 2.0 - y_offset; // Above the top of the circle
    let arc_center = Pos2::new(center.x, center.y + arc_y_offset + arc_radius);

    draw_arc_arrow(
        painter,
        ArcArrow {
            center: arc_center,
            radius: arc_radius,
            width: arc_width,
            // The left side, its head pointing counter-clockwise.
            start_angle: std::f32::consts::PI - 0.3,
            end_angle: std::f32::consts::PI + 0.3,
            arrow_at_start: true,
            axis: RotateAxis::ScreenZ,
            degrees: -45.0,
        },
        arrow_color,
        hover_color,
        &click_pos,
        &hover_pos,
        &mut result,
    );

    draw_arc_arrow(
        painter,
        ArcArrow {
            center: arc_center,
            radius: arc_radius,
            width: arc_width,
            // The right side, its head pointing clockwise.
            start_angle: -0.3,
            end_angle: 0.3,
            arrow_at_start: false,
            axis: RotateAxis::ScreenZ,
            degrees: 45.0,
        },
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

/// Whether `p` lands on the arc arrow: its band or its head.
pub(super) fn hit_test_arc_arrow(p: Pos2, arrow: &ArcArrow) -> bool {
    let v = p - arrow.center;
    let r = v.length();
    if r == 0.0 {
        return false;
    }
    let theta = v.y.atan2(v.x);
    let half_w = arrow.width * 0.5 + 1.5;
    let in_radius = (r >= arrow.radius - half_w) && (r <= arrow.radius + half_w);
    if in_radius && angle_in_range(theta, arrow.start_angle, arrow.end_angle) {
        return true;
    }
    point_in_polygon(p, &arrow_head(arrow, arrow.width))
}

/// The head of an arc arrow drawn `width` wide: a triangle past the arc's
/// start or end, pointing along it.
fn arrow_head(arrow: &ArcArrow, width: f32) -> [Pos2; 3] {
    let delta_angle = width / arrow.radius;
    let angle = if arrow.arrow_at_start {
        arrow.start_angle - delta_angle
    } else {
        arrow.end_angle + delta_angle
    };
    let tip = Pos2::new(
        arrow.center.x + angle.cos() * arrow.radius,
        arrow.center.y + angle.sin() * arrow.radius,
    );
    // Along the arc, the way the head points, and across it.
    let along = if arrow.arrow_at_start { -1.0 } else { 1.0 };
    let tangent = egui::Vec2::new(-angle.sin(), angle.cos()) * along;
    let normal = egui::Vec2::new(angle.cos(), angle.sin());
    [
        tip,
        tip - tangent * (width + 2.5) + normal * (width + 0.5),
        tip - tangent * (width + 2.5) - normal * (width + 0.5),
    ]
}

/// Draws an arc arrow and sets `result` to its turn when it is clicked.
/// An arc arrow's place and what it does: the arc about `center` from
/// `start_angle` to `end_angle`, its head at the start or the end, turning
/// the view `degrees` about `axis` when clicked.
#[derive(Clone, Copy)]
pub(super) struct ArcArrow {
    pub center: Pos2,
    pub radius: f32,
    pub width: f32,
    pub start_angle: f32,
    pub end_angle: f32,
    pub arrow_at_start: bool,
    pub axis: RotateAxis,
    pub degrees: f32,
}

pub(super) fn draw_arc_arrow(
    painter: &egui::Painter,
    arrow: ArcArrow,
    base_color: Color32,
    hover_color: Color32,
    click_pos: &Option<Pos2>,
    hover_pos: &Option<Pos2>,
    result: &mut Option<RotateDelta>,
) {
    let ArcArrow {
        center,
        radius,
        width,
        start_angle,
        end_angle,
        arrow_at_start: _,
        axis,
        degrees,
    } = arrow;
    let is_hovered = hover_pos.is_some_and(|p| hit_test_arc_arrow(p, &arrow));
    let is_clicked = click_pos.is_some_and(|p| hit_test_arc_arrow(p, &arrow));

    if is_clicked {
        *result = Some(RotateDelta { degrees, axis });
    }

    let color = if is_hovered { hover_color } else { base_color };
    let stroke_width = if is_hovered { width + 1.0 } else { width };

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

    let arrow_pts = arrow_head(&arrow, stroke_width).to_vec();
    painter.add(egui::Shape::convex_polygon(arrow_pts, color, Stroke::NONE));
}
