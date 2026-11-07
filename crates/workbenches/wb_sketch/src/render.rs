//! Sketch geometry as renderable meshes: thin quads, or a line list the
//! renderer draws at a fixed pixel width.

use crate::boundary::narrow3;
use crate::sketch::{GeometryElement, Sketch, SketchPlane, Vec2D};
use kernel_api::TriMesh;

/// The sketch as world-space polylines: one per line, sampled curve, or
/// point cross. Shared by the quad mesh and the line list.
pub fn sketch_polylines(sketch: &Sketch, plane: &SketchPlane) -> Vec<Vec<[f64; 3]>> {
    // Construction is left out, and the points only it holds: what the
    // closed sketch shows is what it makes.
    let curves = || (sketch.geometry.iter()).filter(|g| !matches!(g, GeometryElement::Point(_)));
    let shown: std::collections::HashSet<uuid::Uuid> = curves()
        .filter(|g| !sketch.is_construction(g.id()))
        .flat_map(Sketch::curve_point_ids)
        .collect();
    let guides_only: std::collections::HashSet<uuid::Uuid> = curves()
        .filter(|g| sketch.is_construction(g.id()))
        .flat_map(Sketch::curve_point_ids)
        .filter(|id| !shown.contains(id))
        .collect();
    // Projected geometry draws as drawn geometry does: the solid it came
    // from may have changed since, or be built from it.
    sketch
        .geometry
        .iter()
        .filter(|g| !sketch.is_construction(g.id()) && !guides_only.contains(&g.id()))
        .flat_map(|g| element_polylines(sketch, plane, g))
        .collect()
}

/// The lines curve `id` of the sketch draws, as a line list in the
/// sketch's frame. `None` when it is no curve of the sketch.
pub fn curve_lines(sketch: &Sketch, plane: &SketchPlane, id: uuid::Uuid) -> Option<TriMesh> {
    let geom = sketch
        .get_geometry(id)
        .filter(|g| !matches!(g, GeometryElement::Point(_)))?;
    Some(line_list(element_polylines(sketch, plane, geom), plane))
}

/// The world-space polylines one element draws.
fn element_polylines(
    sketch: &Sketch,
    plane: &SketchPlane,
    geom: &GeometryElement,
) -> Vec<Vec<[f64; 3]>> {
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
                            let angle = (i as f64 / segments as f64) * 2.0 * std::f64::consts::PI;
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
                let (start_angle, sweep) =
                    crate::snap::arc_angles((start - center).to_glam(), (end - center).to_glam());
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
    out
}

/// The sketch as a line list: no triangles, every segment an edge pair, so
/// the renderer draws it at a constant pixel width whatever the zoom.
pub fn sketch_to_lines(sketch: &Sketch, plane: &SketchPlane) -> TriMesh {
    line_list(sketch_polylines(sketch, plane), plane)
}

/// Polylines as a line list facing the plane's normal.
fn line_list(polylines: Vec<Vec<[f64; 3]>>, plane: &SketchPlane) -> TriMesh {
    let mut positions = Vec::new();
    let mut edges = Vec::new();
    let normal = narrow3(glam::DVec3::from_array(plane.normal).normalize().to_array());
    for polyline in polylines {
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

    /// A closed sketch leaves out its construction and the points only
    /// construction holds.
    #[test]
    fn construction_is_left_out_of_the_closed_sketch() {
        let mut sketch = Sketch::new("s");
        let point = |sketch: &mut Sketch, x| {
            sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 0.0))))
        };
        let (a, b, c) = (
            point(&mut sketch, 0.0),
            point(&mut sketch, 5.0),
            point(&mut sketch, 9.0),
        );
        sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let guide = sketch.add_geometry(GeometryElement::Line(Line::new(b, c)));
        let all = sketch_polylines(&sketch, &SketchPlane::default()).len();
        sketch.set_construction(guide, true);
        // The guide and its far end's two-stroke cross go; `b` stays.
        assert_eq!(
            sketch_polylines(&sketch, &SketchPlane::default()).len(),
            all - 3
        );
    }
}
