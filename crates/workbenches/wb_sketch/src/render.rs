//! Sketch geometry as renderable meshes: thin quads, or a line list the
//! renderer draws at a fixed pixel width.

use crate::sketch::{GeometryElement, Sketch, SketchPlane, Vec2D};
use kernel_api::TriMesh;

/// Convert sketch geometry to a renderable mesh.
///
/// This tessellates the sketch geometry (lines, circles, arcs) into triangles
/// for rendering in the 3D viewport.
/// The sketch as world-space polylines: one per line, sampled curve, or
/// point cross. Shared by the quad mesh and the line list.
pub fn sketch_polylines(sketch: &Sketch, plane: &SketchPlane) -> Vec<Vec<[f32; 3]>> {
    let x_axis = glam::Vec3::from_array(plane.x_axis);
    let y_axis = glam::Vec3::from_array(plane.y_axis);
    let origin = glam::Vec3::from_array(plane.origin);
    let to_world =
        |pos: Vec2D| -> [f32; 3] { (origin + x_axis * pos.x + y_axis * pos.y).to_array() };
    let point = |id| {
        sketch.get_geometry(id).and_then(|g| match g {
            GeometryElement::Point(p) => Some(p.position),
            _ => None,
        })
    };
    let mut out: Vec<Vec<[f32; 3]>> = Vec::new();
    for geom in &sketch.geometry {
        match geom {
            GeometryElement::Point(p) => {
                // A small cross in the plane.
                let size = 0.05;
                let c = p.position;
                out.push(vec![
                    to_world(Vec2D::new(c.x - size, c.y)),
                    to_world(Vec2D::new(c.x + size, c.y)),
                ]);
                out.push(vec![
                    to_world(Vec2D::new(c.x, c.y - size)),
                    to_world(Vec2D::new(c.x, c.y + size)),
                ]);
            }
            GeometryElement::Line(line) => {
                if let (Some(start), Some(end)) = (point(line.start), point(line.end)) {
                    out.push(vec![to_world(start), to_world(end)]);
                }
            }
            GeometryElement::Circle(circle) => {
                if let Some(center) = point(circle.center) {
                    let segments = 32;
                    out.push(
                        (0..=segments)
                            .map(|i| {
                                let angle =
                                    (i as f32 / segments as f32) * 2.0 * std::f32::consts::PI;
                                let offset = Vec2D::new(
                                    circle.radius * angle.cos(),
                                    circle.radius * angle.sin(),
                                );
                                to_world(center + offset)
                            })
                            .collect(),
                    );
                }
            }
            GeometryElement::Arc(arc) => {
                if let (Some(center), Some(start), Some(end)) =
                    (point(arc.center), point(arc.start), point(arc.end))
                {
                    // CCW sweep, matching every other consumer of arcs
                    // (overlay, profile extraction, hit-testing).
                    let (start_angle, sweep) = crate::snap::arc_angles(
                        (start - center).to_glam(),
                        (end - center).to_glam(),
                    );
                    let segments = 16;
                    out.push(
                        (0..=segments)
                            .map(|i| {
                                let angle = start_angle + (i as f32 / segments as f32) * sweep;
                                let offset =
                                    Vec2D::new(arc.radius * angle.cos(), arc.radius * angle.sin());
                                to_world(center + offset)
                            })
                            .collect(),
                    );
                }
            }
            GeometryElement::Ellipse(ellipse) => {
                if let Some(pts) = ellipse.points(sketch, 48) {
                    out.push(pts.iter().map(|p| to_world(*p)).collect());
                }
            }
            GeometryElement::BSpline(spline) => {
                let ctrl: Option<Vec<Vec2D>> = spline
                    .control_points
                    .iter()
                    .map(|id| sketch.point_position(*id))
                    .collect();
                if let Some(ctrl) = ctrl {
                    let pts = crate::geom2d::bspline_points(&ctrl, spline.periodic, 64);
                    out.push(pts.iter().map(|p| to_world(*p)).collect());
                }
            }
        }
    }
    out
}

/// Convert sketch geometry to a renderable mesh of thin quads.
pub fn sketch_to_mesh(sketch: &Sketch, plane: &SketchPlane) -> TriMesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    let plane_normal = glam::Vec3::from_array(plane.normal).normalize();
    let mut vertex_offset = 0u32;
    for polyline in sketch_polylines(sketch, plane) {
        for pair in polyline.windows(2) {
            add_line_quad(
                &mut positions,
                &mut normals,
                &mut indices,
                &mut vertex_offset,
                pair[0],
                pair[1],
                0.1,
                plane_normal,
            );
        }
    }
    TriMesh {
        positions,
        normals,
        indices,
        edges: Vec::new(),
        colors: Vec::new(),
        faces: Vec::new(),
        edge_ids: Vec::new(),
    }
}

/// The sketch as a line list: no triangles, every segment an edge pair, so
/// the renderer draws it at a constant pixel width whatever the zoom.
pub fn sketch_to_lines(sketch: &Sketch, plane: &SketchPlane) -> TriMesh {
    let mut positions = Vec::new();
    let mut edges = Vec::new();
    let normal = glam::Vec3::from_array(plane.normal).normalize().to_array();
    for polyline in sketch_polylines(sketch, plane) {
        let base = positions.len() as u32;
        positions.extend_from_slice(&polyline);
        for i in 1..polyline.len() as u32 {
            edges.push(base + i - 1);
            edges.push(base + i);
        }
    }
    let normals = vec![normal; positions.len()];
    TriMesh {
        positions,
        normals,
        indices: Vec::new(),
        edges,
        colors: Vec::new(),
        faces: Vec::new(),
        edge_ids: Vec::new(),
    }
}

/// Add a line segment as a thin quad (two triangles) to the mesh.
#[allow(clippy::too_many_arguments)]
fn add_line_quad(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    indices: &mut Vec<u32>,
    vertex_offset: &mut u32,
    start: [f32; 3],
    end: [f32; 3],
    thickness: f32,
    plane_normal: glam::Vec3,
) {
    let dir = glam::Vec3::from_array([end[0] - start[0], end[1] - start[1], end[2] - start[2]]);
    let length = dir.length();
    if length < 1e-6 {
        return; // Degenerate line
    }

    let dir_norm = dir / length;

    // Find a perpendicular vector for the quad width
    // Use a simple approach: cross with a standard vector
    let up = glam::Vec3::new(0.0, 0.0, 1.0);
    let perp = if (dir_norm.dot(up)).abs() > 0.9 {
        // If line is nearly vertical, use a different vector
        glam::Vec3::new(1.0, 0.0, 0.0).cross(dir_norm)
    } else {
        up.cross(dir_norm)
    }
    .normalize()
        * thickness;

    // Use the plane normal instead of calculating from the line direction
    // This ensures consistent lighting for all geometry on the same plane
    let normal = plane_normal;

    // Create quad vertices
    let v0 = glam::Vec3::from_array(start) - perp;
    let v1 = glam::Vec3::from_array(start) + perp;
    let v2 = glam::Vec3::from_array(end) + perp;
    let v3 = glam::Vec3::from_array(end) - perp;

    let base = *vertex_offset;
    positions.push(v0.to_array());
    positions.push(v1.to_array());
    positions.push(v2.to_array());
    positions.push(v3.to_array());

    normals.push(normal.to_array());
    normals.push(normal.to_array());
    normals.push(normal.to_array());
    normals.push(normal.to_array());

    // Two triangles: (0, 1, 2) and (0, 2, 3)
    indices.push(base);
    indices.push(base + 1);
    indices.push(base + 2);
    indices.push(base);
    indices.push(base + 2);
    indices.push(base + 3);

    *vertex_offset += 4;
}
