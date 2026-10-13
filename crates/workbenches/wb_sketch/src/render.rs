//! Sketch geometry as renderable meshes: thin quads, or a line list the
//! renderer draws at a fixed pixel width.

use crate::boundary::narrow3;
use crate::sketch::{GeometryElement, Sketch, SketchPlane, Vec2D};
use kernel_api::TriMesh;

/// The sketch as world-space polylines: one per line, sampled curve, or
/// point cross. Shared by the quad mesh and the line list.
pub fn sketch_polylines(sketch: &Sketch, plane: &SketchPlane) -> Vec<Vec<[f64; 3]>> {
    let x_axis = glam::DVec3::from_array(plane.x_axis);
    let y_axis = glam::DVec3::from_array(plane.y_axis);
    let origin = glam::DVec3::from_array(plane.origin);
    let to_world =
        |pos: Vec2D| -> [f64; 3] { (origin + x_axis * pos.x + y_axis * pos.y).to_array() };
    let point = |id| {
        sketch.get_geometry(id).and_then(|g| match g {
            GeometryElement::Point(p) => Some(p.position),
            _ => None,
        })
    };
    let mut out: Vec<Vec<[f64; 3]>> = Vec::new();
    // Projected geometry draws as drawn geometry does: the solid it came
    // from may have changed since, or be built from it.
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
                                    (i as f64 / segments as f64) * 2.0 * std::f64::consts::PI;
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
                                let angle = start_angle + (i as f64 / segments as f64) * sweep;
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
            GeometryElement::Conic(conic) => {
                if let Some(pts) = conic.points(sketch, 48) {
                    out.push(pts.iter().map(|p| to_world(*p)).collect());
                }
            }
            GeometryElement::BSpline(spline) => {
                if let Some(pts) = spline.points(sketch, 64) {
                    out.push(pts.iter().map(|p| to_world(*p)).collect());
                }
            }
        }
    }
    out
}

/// The sketch as a line list: no triangles, every segment an edge pair, so
/// the renderer draws it at a constant pixel width whatever the zoom.
pub fn sketch_to_lines(sketch: &Sketch, plane: &SketchPlane) -> TriMesh {
    let mut positions = Vec::new();
    let mut edges = Vec::new();
    let normal = narrow3(glam::DVec3::from_array(plane.normal).normalize().to_array());
    for polyline in sketch_polylines(sketch, plane) {
        let base = positions.len() as u32;
        positions.extend(polyline.iter().map(|p| narrow3(*p)));
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
        ..TriMesh::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{ExternalReference, ExternalSource, Line, Point};

    /// A closed sketch draws its projected geometry as it draws what was
    /// drawn in it, whether it guides or counts.
    #[test]
    fn projected_geometry_draws_when_the_sketch_is_closed() {
        let mut sketch = Sketch::new("s");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, 0.0))));
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let drawn = sketch_polylines(&sketch, &SketchPlane::default()).len();
        sketch.external.insert(
            line,
            ExternalSource::of_reference(ExternalReference::Datum {
                datum: uuid::Uuid::new_v4(),
            }),
        );
        assert_eq!(
            sketch_polylines(&sketch, &SketchPlane::default()).len(),
            drawn
        );
        let lines = sketch_to_lines(&sketch, &SketchPlane::default());
        assert!(!lines.edges.is_empty());
    }
}
