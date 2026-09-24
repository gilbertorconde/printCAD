//! The chamfered cube itself: its faces, edge and corner bevels, drawn
//! turned with the view, each face or bevel a click that snaps to it.

use super::*;

/// A polygon to draw on the cube (main face, edge bevel, or corner bevel)
pub(super) struct CubePolygon {
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
pub(super) fn draw_cube_interactive(
    ui: &Ui,
    painter: &egui::Painter,
    center: Pos2,
    cube_scale: f32,
    rot: &Mat3,
    axes: &AxisSystem,
    response: &Response,
) -> Option<CameraSnapView> {
    let ctx = ui.ctx();
    let mut clicked_face: Option<CameraSnapView> = None;

    let m = 0.12_f32;

    // - Main faces are 8-sided polygons (octagons with cut corners)
    // - Edge bevels are quads
    // - Corner bevels are hexagons

    let mut polygons: Vec<CubePolygon> = Vec::new();

    // A face wears the colour of the world axis it faces, as the triad in
    // the corner draws that axis, so the two read as one thing. Faces are
    // built canonical — Y up, Z toward the viewer — and the preset says which
    // world axis each of those is, so opposite faces share a colour by
    // construction.
    let front_color = face_color(axes, Vec3::Z);
    let rear_color = face_color(axes, Vec3::NEG_Z);
    let right_color = face_color(axes, Vec3::X);
    let left_color = face_color(axes, Vec3::NEG_X);
    let top_color = face_color(axes, Vec3::Y);
    let bottom_color = face_color(axes, Vec3::NEG_Y);
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
        uvs: Some(face_uvs(&verts_top, x, z, false, true)),
        verts: verts_top,
        normal: Vec3::Y,
        color: top_color,
        label: Some("TOP"),
        snap_view: Some(CameraSnapView::Top),
    });

    // Bottom (-Y)
    let verts_bottom = make_main_face(x, -z, -y);
    polygons.push(CubePolygon {
        uvs: Some(face_uvs(&verts_bottom, x, -z, false, true)),
        verts: verts_bottom,
        normal: Vec3::NEG_Y,
        color: bottom_color,
        label: Some("BOTTOM"),
        snap_view: Some(CameraSnapView::Bottom),
    });

    // Front (+Z)
    let verts_front = make_main_face(x, y, z);
    polygons.push(CubePolygon {
        uvs: Some(face_uvs(&verts_front, x, y, false, false)),
        verts: verts_front,
        normal: Vec3::Z,
        color: front_color,
        label: Some("FRONT"),
        snap_view: Some(CameraSnapView::Front),
    });

    // Rear (-Z)
    let verts_rear = make_main_face(-x, y, -z);
    polygons.push(CubePolygon {
        uvs: Some(face_uvs(&verts_rear, -x, y, false, false)),
        verts: verts_rear,
        normal: Vec3::NEG_Z,
        color: rear_color,
        label: Some("REAR"),
        snap_view: Some(CameraSnapView::Rear),
    });

    // Right (+X)
    let verts_right = make_main_face(-z, y, x);
    polygons.push(CubePolygon {
        uvs: Some(face_uvs(&verts_right, -z, y, false, false)),
        verts: verts_right,
        normal: Vec3::X,
        color: right_color,
        label: Some("RIGHT"),
        snap_view: Some(CameraSnapView::Right),
    });

    // Left (-X)
    let verts_left = make_main_face(z, y, -x);
    polygons.push(CubePolygon {
        uvs: Some(face_uvs(&verts_left, z, y, false, false)),
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

    // Orthographic projection from cube-local space to widget pixels:
    //   * +X (cube right) → +screen X (right of widget)
    //   * +Y (cube up)    → -screen Y (top of widget, since egui Y points down)
    // The camera bakes the Vulkan Y-flip into its projection, so the main
    // viewport is not mirrored and the cube must use a normal +X mapping.
    // Negating X here would make it appear L/R flipped and rotate in the
    // opposite direction from the part when the camera orbits.
    let project =
        |v: Vec3| -> Pos2 { Pos2::new(center.x + v.x * cube_scale, center.y - v.y * cube_scale) };

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
        if let Some(pos) = click_pos
            && point_in_polygon(pos, &points)
            && let Some(snap) = poly.snap_view
        {
            clicked_face = Some(snap);
        }

        if is_hovered && let Some(label) = poly.label {
            hovered_label = Some(label);
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
            Stroke::new(0.5_f32, stroke_color),
        ));

        if let (Some(label), Some(uvs)) = (poly.label, &poly.uvs)
            && normal.z > 0.3
            && points.len() >= 3
        {
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

    // Show tooltip for hovered face
    if let Some(label) = hovered_label {
        response.clone().on_hover_ui_at_pointer(|ui| {
            ui.label(format!("Click to view {}", label));
        });
    }

    clicked_face
}

pub(super) fn face_uvs(
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
