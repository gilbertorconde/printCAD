//! External geometry: a solid's edges projected into the sketch.
//!
//! The kernel projects an edge onto the sketch plane as the exact curve it
//! is there (a line, a circle or arc, an ellipse or arc of one, a point);
//! a curve with no closed form comes as points along it and is kept as a
//! chain of lines. Each element remembers its edge (`ExternalSource`), so
//! the sketch projects it again when it is edited and the solid has
//! changed: in place where the shape is the same kind, keeping the
//! constraints that refer to it, and afresh where it is not.

use std::collections::BTreeMap;
use std::f64::consts::TAU;

use kernel_api::ProjectedEdge;
use uuid::Uuid;

use crate::sketch::{
    Arc, Circle, Ellipse, ExternalSource, GeometryElement, Line, Point, Sketch, Vec2D,
};

fn v(p: [f64; 2]) -> Vec2D {
    Vec2D::new(p[0] as f32, p[1] as f32)
}

/// Whether a range of angles closes the turn.
fn full_turn(range: (f64, f64)) -> bool {
    range.1 - range.0 >= TAU - 1e-9
}

fn circle_point(centre: [f64; 2], radius: f64, t: f64) -> [f64; 2] {
    [centre[0] + radius * t.cos(), centre[1] + radius * t.sin()]
}

fn ellipse_point(centre: [f64; 2], major: [f64; 2], ratio: f64, t: f64) -> [f64; 2] {
    let minor = [-major[1] * ratio, major[0] * ratio];
    [
        centre[0] + major[0] * t.cos() + minor[0] * t.sin(),
        centre[1] + major[1] * t.cos() + minor[1] * t.sin(),
    ]
}

/// Add a projected edge to the sketch as external geometry from `source`.
/// Returns how many elements it became.
pub fn add(sketch: &mut Sketch, projected: &ProjectedEdge, source: ExternalSource) -> usize {
    let point = |sketch: &mut Sketch, p: [f64; 2]| {
        sketch.add_geometry(GeometryElement::Point(Point::new(v(p))))
    };
    let mut made: Vec<Uuid> = Vec::new();
    match projected {
        ProjectedEdge::Point(p) => made.push(point(sketch, *p)),
        ProjectedEdge::Line { start, end } => {
            let (a, b) = (point(sketch, *start), point(sketch, *end));
            made.push(sketch.add_geometry(GeometryElement::Line(Line::new(a, b))));
        }
        ProjectedEdge::Circle {
            centre,
            radius,
            range,
        } => {
            let c = point(sketch, *centre);
            let element = if full_turn(*range) {
                GeometryElement::Circle(Circle::new(c, *radius as f32))
            } else {
                let s = point(sketch, circle_point(*centre, *radius, range.0));
                let e = point(sketch, circle_point(*centre, *radius, range.1));
                GeometryElement::Arc(Arc::new(c, s, e, *radius as f32))
            };
            made.push(sketch.add_geometry(element));
        }
        ProjectedEdge::Ellipse {
            centre,
            major,
            ratio,
            range,
        } => {
            let c = point(sketch, *centre);
            let element = if full_turn(*range) {
                GeometryElement::Ellipse(Ellipse::new(c, v(*major), *ratio as f32))
            } else {
                let s = point(sketch, ellipse_point(*centre, *major, *ratio, range.0));
                let e = point(sketch, ellipse_point(*centre, *major, *ratio, range.1));
                GeometryElement::Ellipse(Ellipse::new_arc(c, v(*major), *ratio as f32, s, e))
            };
            made.push(sketch.add_geometry(element));
        }
        ProjectedEdge::Polyline(points) => {
            let ids: Vec<Uuid> = points.iter().map(|p| point(sketch, *p)).collect();
            for pair in ids.windows(2) {
                made.push(sketch.add_geometry(GeometryElement::Line(Line::new(pair[0], pair[1]))));
            }
        }
    }
    let count = made.len();
    for id in made {
        sketch.external.insert(id, source);
    }
    count
}

/// Move the points of `group` (one edge's elements, in the order they
/// were made) to `projected`, when it is the same kind of shape. Returns
/// whether it was.
fn update(sketch: &mut Sketch, group: &[Uuid], projected: &ProjectedEdge) -> bool {
    let place = |sketch: &mut Sketch, id: Uuid, p: [f64; 2]| {
        if let Some(GeometryElement::Point(point)) = sketch.get_geometry_mut(id) {
            point.position = v(p);
        }
    };
    let elements: Vec<GeometryElement> = group
        .iter()
        .filter_map(|id| sketch.get_geometry(*id).cloned())
        .collect();
    match (projected, elements.as_slice()) {
        (ProjectedEdge::Point(p), [GeometryElement::Point(point)]) => {
            place(sketch, point.id, *p);
        }
        (ProjectedEdge::Line { start, end }, [GeometryElement::Line(line)]) => {
            place(sketch, line.start, *start);
            place(sketch, line.end, *end);
        }
        (
            ProjectedEdge::Circle {
                centre,
                radius,
                range,
            },
            [GeometryElement::Circle(circle)],
        ) if full_turn(*range) => {
            place(sketch, circle.center, *centre);
            if let Some(GeometryElement::Circle(c)) = sketch.get_geometry_mut(circle.id) {
                c.radius = *radius as f32;
            }
        }
        (
            ProjectedEdge::Circle {
                centre,
                radius,
                range,
            },
            [GeometryElement::Arc(arc)],
        ) if !full_turn(*range) => {
            place(sketch, arc.center, *centre);
            place(sketch, arc.start, circle_point(*centre, *radius, range.0));
            place(sketch, arc.end, circle_point(*centre, *radius, range.1));
            if let Some(GeometryElement::Arc(a)) = sketch.get_geometry_mut(arc.id) {
                a.radius = *radius as f32;
            }
        }
        (
            ProjectedEdge::Ellipse {
                centre,
                major,
                ratio,
                range,
            },
            [GeometryElement::Ellipse(ellipse)],
        ) if full_turn(*range) == ellipse.arc.is_none() => {
            place(sketch, ellipse.center, *centre);
            if let Some(ends) = ellipse.arc {
                place(
                    sketch,
                    ends.start,
                    ellipse_point(*centre, *major, *ratio, range.0),
                );
                place(
                    sketch,
                    ends.end,
                    ellipse_point(*centre, *major, *ratio, range.1),
                );
            }
            if let Some(GeometryElement::Ellipse(e)) = sketch.get_geometry_mut(ellipse.id) {
                e.major = v(*major);
                e.ratio = *ratio as f32;
            }
        }
        (ProjectedEdge::Polyline(points), lines)
            if lines.len() + 1 == points.len()
                && lines.iter().all(|l| matches!(l, GeometryElement::Line(_))) =>
        {
            for (line, pair) in lines.iter().zip(points.windows(2)) {
                if let GeometryElement::Line(line) = line {
                    place(sketch, line.start, pair[0]);
                    place(sketch, line.end, pair[1]);
                }
            }
        }
        _ => return false,
    }
    true
}

/// The external elements grouped by the edge they came from, each group
/// in the order its elements were made.
pub fn groups(sketch: &Sketch) -> Vec<(ExternalSource, Vec<Uuid>)> {
    let key = |s: &ExternalSource| {
        let mut k = s.body.as_u128().to_le_bytes().to_vec();
        for c in s.point.iter().chain(&s.direction) {
            k.extend(c.to_bits().to_le_bytes());
        }
        k
    };
    let mut by_source: BTreeMap<Vec<u8>, (ExternalSource, Vec<Uuid>)> = BTreeMap::new();
    for element in &sketch.geometry {
        if let Some(source) = sketch.external.get(&element.id()) {
            by_source
                .entry(key(source))
                .or_insert_with(|| (*source, Vec::new()))
                .1
                .push(element.id());
        }
    }
    by_source.into_values().collect()
}

/// Bring one edge's elements up to `projected`: moved in place where the
/// shape is the same kind, else made again (constraints on the old ones go
/// with them).
pub fn refresh_group(
    sketch: &mut Sketch,
    source: ExternalSource,
    group: &[Uuid],
    projected: &ProjectedEdge,
) {
    if !update(sketch, group, projected) {
        sketch.remove_geometry_cascade(group);
        add(sketch, projected, source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> ExternalSource {
        ExternalSource {
            body: Uuid::nil(),
            point: [1.0, 2.0, 3.0],
            direction: [0.0, 0.0, 1.0],
        }
    }

    #[test]
    fn each_projection_becomes_the_sketch_curve_it_is() {
        let mut sketch = Sketch::new("t");
        let full = ProjectedEdge::Circle {
            centre: [1.0, 2.0],
            radius: 3.0,
            range: (0.0, TAU),
        };
        assert_eq!(add(&mut sketch, &full, source()), 1);
        let arc = ProjectedEdge::Circle {
            centre: [0.0, 0.0],
            radius: 2.0,
            range: (0.0, std::f64::consts::FRAC_PI_2),
        };
        add(&mut sketch, &arc, source());
        let chain = ProjectedEdge::Polyline(vec![[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]]);
        assert_eq!(add(&mut sketch, &chain, source()), 2);
        let kinds: Vec<&str> = sketch
            .geometry
            .iter()
            .filter(|g| sketch.external.contains_key(&g.id()))
            .map(|g| match g {
                GeometryElement::Circle(_) => "circle",
                GeometryElement::Arc(_) => "arc",
                GeometryElement::Line(_) => "line",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["circle", "arc", "line", "line"]);
        // The arc's ends sit on the circle where its angles say.
        let arc = sketch
            .geometry
            .iter()
            .find_map(|g| match g {
                GeometryElement::Arc(a) => Some(a.clone()),
                _ => None,
            })
            .unwrap();
        let end = sketch.point_position(arc.end).unwrap();
        assert!(end.x.abs() < 1e-6 && (end.y - 2.0).abs() < 1e-6);
    }

    #[test]
    fn a_refresh_moves_the_same_kind_in_place_and_remakes_another() {
        let mut sketch = Sketch::new("t");
        add(
            &mut sketch,
            &ProjectedEdge::Line {
                start: [0.0, 0.0],
                end: [5.0, 0.0],
            },
            source(),
        );
        let (src, group) = groups(&sketch).remove(0);
        let line_id = group[0];
        refresh_group(
            &mut sketch,
            src,
            &group,
            &ProjectedEdge::Line {
                start: [0.0, 1.0],
                end: [8.0, 1.0],
            },
        );
        assert!(sketch.get_geometry(line_id).is_some(), "moved in place");
        let GeometryElement::Line(line) = sketch.get_geometry(line_id).unwrap() else {
            panic!()
        };
        assert_eq!(sketch.point_position(line.end), Some(Vec2D::new(8.0, 1.0)));
        // The edge is now round: the line gives way to a circle.
        refresh_group(
            &mut sketch,
            src,
            &group,
            &ProjectedEdge::Circle {
                centre: [0.0, 0.0],
                radius: 1.0,
                range: (0.0, TAU),
            },
        );
        assert!(sketch.get_geometry(line_id).is_none());
        assert_eq!(groups(&sketch).len(), 1);
        assert!(matches!(
            sketch.get_geometry(groups(&sketch)[0].1[0]),
            Some(GeometryElement::Circle(_))
        ));
    }
}
