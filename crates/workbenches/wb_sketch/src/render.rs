//! Rendering utilities for converting sketch geometry to meshes.

use crate::sketch::{GeometryElement, Sketch, SketchPlane, Vec2D};
use kernel_api::TriMesh;

/// Convert sketch geometry to a renderable mesh.
///
/// This tessellates the sketch geometry (lines, circles, arcs) into triangles
/// for rendering in the 3D viewport.
pub fn sketch_to_mesh(sketch: &Sketch, plane: &SketchPlane) -> TriMesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();

    // Convert 2D sketch coordinates to 3D world coordinates
    let to_world = |pos: Vec2D| -> [f32; 3] {
        let x_axis = glam::Vec3::from_array(plane.x_axis);
        let y_axis = glam::Vec3::from_array(plane.y_axis);
        let origin = glam::Vec3::from_array(plane.origin);

        (origin + x_axis * pos.x + y_axis * pos.y).to_array()
    };

    // Get normal vector for the plane (not used currently, but available for future use)
    let _normal = glam::Vec3::from_array(plane.normal).normalize();

    let mut vertex_offset = 0u32;

    for geom in &sketch.geometry {
        match geom {
            GeometryElement::Point(point) => {
                // Render point as a small cross (4 lines forming an X)
                let world_pos = to_world(point.position);
                let size = 0.05; // Point size in world units

                // Create a small cross
                let offsets = [
                    ([-size, 0.0, 0.0], [size, 0.0, 0.0]), // Horizontal line
                    ([0.0, -size, 0.0], [0.0, size, 0.0]), // Vertical line
                ];

                for (start, end) in offsets {
                    let start_pos = [
                        world_pos[0] + start[0],
                        world_pos[1] + start[1],
                        world_pos[2] + start[2],
                    ];
                    let end_pos = [
                        world_pos[0] + end[0],
                        world_pos[1] + end[1],
                        world_pos[2] + end[2],
                    ];

                    // Create a thin line as a quad (two triangles)
                    add_line_quad(
                        &mut positions,
                        &mut normals,
                        &mut indices,
                        &mut vertex_offset,
                        start_pos,
                        end_pos,
                        0.002,
                    );
                }
            }
            GeometryElement::Line(line) => {
                // Get start and end points
                let start_point = sketch.get_geometry(line.start).and_then(|g| match g {
                    GeometryElement::Point(p) => Some(p.position),
                    _ => None,
                });
                let end_point = sketch.get_geometry(line.end).and_then(|g| match g {
                    GeometryElement::Point(p) => Some(p.position),
                    _ => None,
                });

                if let (Some(start), Some(end)) = (start_point, end_point) {
                    let start_world = to_world(start);
                    let end_world = to_world(end);

                    // Render line as a thin quad (two triangles)
                    add_line_quad(
                        &mut positions,
                        &mut normals,
                        &mut indices,
                        &mut vertex_offset,
                        start_world,
                        end_world,
                        0.002,
                    );
                }
            }
            GeometryElement::Circle(circle) => {
                // Get center point
                let center_point = sketch.get_geometry(circle.center).and_then(|g| match g {
                    GeometryElement::Point(p) => Some(p.position),
                    _ => None,
                });

                if let Some(center) = center_point {
                    // Tessellate circle into line segments
                    let segments = 32; // Number of segments for the circle
                    let mut prev_point = None;

                    for i in 0..=segments {
                        let angle = (i as f32 / segments as f32) * 2.0 * std::f32::consts::PI;
                        let offset =
                            Vec2D::new(circle.radius * angle.cos(), circle.radius * angle.sin());
                        let point_world = to_world(center + offset);

                        if let Some(prev) = prev_point {
                            add_line_quad(
                                &mut positions,
                                &mut normals,
                                &mut indices,
                                &mut vertex_offset,
                                prev,
                                point_world,
                                0.002,
                            );
                        }
                        prev_point = Some(point_world);
                    }
                }
            }
            GeometryElement::Arc(arc) => {
                // Get center, start, and end points
                let center_point = sketch.get_geometry(arc.center).and_then(|g| match g {
                    GeometryElement::Point(p) => Some(p.position),
                    _ => None,
                });
                let start_point = sketch.get_geometry(arc.start).and_then(|g| match g {
                    GeometryElement::Point(p) => Some(p.position),
                    _ => None,
                });
                let end_point = sketch.get_geometry(arc.end).and_then(|g| match g {
                    GeometryElement::Point(p) => Some(p.position),
                    _ => None,
                });

                if let (Some(center), Some(start), Some(end)) =
                    (center_point, start_point, end_point)
                {
                    // Calculate angles
                    let start_vec = start - center;
                    let end_vec = end - center;
                    let start_angle = start_vec.y.atan2(start_vec.x);
                    let mut end_angle = end_vec.y.atan2(end_vec.x);

                    // Ensure we go the shorter way
                    if end_angle < start_angle {
                        end_angle += 2.0 * std::f32::consts::PI;
                    }

                    // Tessellate arc
                    let segments = 16;
                    let mut prev_point = None;

                    for i in 0..=segments {
                        let t = i as f32 / segments as f32;
                        let angle = start_angle + t * (end_angle - start_angle);
                        let offset = Vec2D::new(arc.radius * angle.cos(), arc.radius * angle.sin());
                        let point_world = to_world(center + offset);

                        if let Some(prev) = prev_point {
                            add_line_quad(
                                &mut positions,
                                &mut normals,
                                &mut indices,
                                &mut vertex_offset,
                                prev,
                                point_world,
                                0.002,
                            );
                        }
                        prev_point = Some(point_world);
                    }
                }
            }
        }
    }

    TriMesh {
        positions,
        normals,
        indices,
    }
}

/// Add a line segment as a thin quad (two triangles) to the mesh.
fn add_line_quad(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    indices: &mut Vec<u32>,
    vertex_offset: &mut u32,
    start: [f32; 3],
    end: [f32; 3],
    thickness: f32,
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

    let normal = perp.cross(dir_norm).normalize();

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
