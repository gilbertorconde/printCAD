//! A 2D drawing brought into a sketch: lines, arcs, circles and ellipses
//! as the sketch's own, a spline as lines through points on it, visible
//! curves as geometry and hidden ones as construction.
//!
//! Ends that meet share one point, within [`MERGE_MM`], across curves as
//! much as within one: that shared topology is what makes a drawn outline
//! a closed profile.

use std::collections::HashMap;

use kernel_api::{Drawing2d, DrawingShape};
use uuid::Uuid;

use crate::sketch::{
    Arc, Circle, ConstraintKind, Ellipse, GeometryElement, Line, Point, Sketch, Vec2D,
};

/// Ends closer than this, in millimetres, are one point.
pub(crate) const MERGE_MM: f64 = 1e-4;

/// What a drawing added to a sketch.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Added {
    pub lines: usize,
    pub arcs: usize,
    pub circles: usize,
    pub ellipses: usize,
    /// Splines brought in as lines through points on them.
    pub splines: usize,
    /// Of all those, the ones made construction.
    pub construction: usize,
}

impl Added {
    /// Every sketch curve made.
    pub(crate) fn curves(&self) -> usize {
        self.lines + self.arcs + self.circles + self.ellipses
    }
}

/// A point made, and where, exactly.
type Placed = (Uuid, [f64; 2]);

/// Points of a sketch found by where they are, on a grid of cells
/// [`MERGE_MM`] wide.
#[derive(Default)]
struct Points {
    cells: HashMap<(i64, i64), Vec<Placed>>,
}

impl Points {
    fn cell(p: [f64; 2]) -> (i64, i64) {
        (
            (p[0] / MERGE_MM).floor() as i64,
            (p[1] / MERGE_MM).floor() as i64,
        )
    }

    /// The point at `p`: one already made within reach, else a new one.
    fn at(&mut self, sketch: &mut Sketch, p: [f64; 2]) -> Uuid {
        let (cx, cy) = Self::cell(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(found) = self.cells.get(&(cx + dx, cy + dy)).and_then(|points| {
                    points
                        .iter()
                        .find(|(_, q)| (q[0] - p[0]).hypot(q[1] - p[1]) <= MERGE_MM)
                }) {
                    return found.0;
                }
            }
        }
        let id = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(
            p[0] as f32,
            p[1] as f32,
        ))));
        self.cells.entry((cx, cy)).or_default().push((id, p));
        id
    }
}

/// Add `drawing`'s curves to `sketch`, scaled by `scale` (millimetres per
/// drawing unit). Segments too short to have two ends are left out.
pub(crate) fn add_drawing(sketch: &mut Sketch, drawing: &Drawing2d, scale: f64) -> Added {
    let mut points = Points::default();
    let mut added = Added::default();
    for curve in &drawing.curves {
        let at = |p: [f64; 2]| [p[0] * scale, p[1] * scale];
        let mut made: Vec<Uuid> = Vec::new();
        match &curve.shape {
            DrawingShape::Polyline {
                points: vertices,
                bulges,
                closed,
            } => {
                let count = vertices.len();
                let segments = if *closed {
                    count
                } else {
                    count.saturating_sub(1)
                };
                for i in 0..segments {
                    let (a, b) = (at(vertices[i]), at(vertices[(i + 1) % count]));
                    let bulge = bulges.get(i).copied().unwrap_or(0.0);
                    if let Some(id) = segment(sketch, &mut points, a, b, bulge, &mut added) {
                        made.push(id);
                    }
                }
            }
            DrawingShape::Spline {
                points: along,
                closed,
            } => {
                let mut ids: Vec<Uuid> = along.iter().map(|p| points.at(sketch, at(*p))).collect();
                if *closed && ids.len() > 2 {
                    ids.push(ids[0]);
                }
                for pair in ids.windows(2) {
                    if pair[0] != pair[1] {
                        made.push(
                            sketch.add_geometry(GeometryElement::Line(Line::new(pair[0], pair[1]))),
                        );
                        added.lines += 1;
                    }
                }
                added.splines += 1;
            }
            DrawingShape::Arc {
                centre,
                radius,
                start_angle,
                end_angle,
            } => {
                let c = at(*centre);
                let r = radius * scale;
                let on = |t: f64| [c[0] + r * t.cos(), c[1] + r * t.sin()];
                let (start, end) = (
                    points.at(sketch, on(*start_angle)),
                    points.at(sketch, on(*end_angle)),
                );
                if start != end {
                    let centre = points.at(sketch, c);
                    made.push(sketch.add_geometry(GeometryElement::Arc(Arc::new(
                        centre, start, end, r as f32,
                    ))));
                    added.arcs += 1;
                }
            }
            DrawingShape::Circle { centre, radius } => {
                let centre = points.at(sketch, at(*centre));
                made.push(sketch.add_geometry(GeometryElement::Circle(Circle::new(
                    centre,
                    (radius * scale) as f32,
                ))));
                added.circles += 1;
            }
            DrawingShape::Ellipse {
                centre,
                major,
                ratio,
                start_param,
                end_param,
            } => {
                let c = at(*centre);
                let m = [major[0] * scale, major[1] * scale];
                let major_v = Vec2D::new(m[0] as f32, m[1] as f32);
                let centre_id = points.at(sketch, c);
                let whole = (end_param - start_param).abs() >= std::f64::consts::TAU - 1e-9;
                if whole {
                    made.push(sketch.add_geometry(GeometryElement::Ellipse(Ellipse::new(
                        centre_id,
                        major_v,
                        *ratio as f32,
                    ))));
                } else {
                    // `centre + major cos t + ratio perp(major) sin t`.
                    let on = |t: f64| {
                        let (sin, cos) = t.sin_cos();
                        [
                            c[0] + m[0] * cos - ratio * m[1] * sin,
                            c[1] + m[1] * cos + ratio * m[0] * sin,
                        ]
                    };
                    let start = points.at(sketch, on(*start_param));
                    let end = points.at(sketch, on(*end_param));
                    let ellipse = sketch.add_geometry(GeometryElement::Ellipse(Ellipse::new_arc(
                        centre_id,
                        major_v,
                        *ratio as f32,
                        start,
                        end,
                    )));
                    for point in [start, end] {
                        sketch.add_constraint(ConstraintKind::PointOnEllipse { point, ellipse });
                    }
                    made.push(ellipse);
                }
                added.ellipses += 1;
            }
        }
        if curve.hidden {
            for id in &made {
                sketch.set_construction(*id, true);
            }
            added.construction += made.len();
        }
    }
    added
}

/// One polyline segment from `a` to `b`: a line, or where `bulge` is not
/// zero the arc it describes (included angle four times its arctangent,
/// counter-clockwise when positive). The element made, if any.
fn segment(
    sketch: &mut Sketch,
    points: &mut Points,
    a: [f64; 2],
    b: [f64; 2],
    bulge: f64,
    added: &mut Added,
) -> Option<Uuid> {
    let (start, end) = (points.at(sketch, a), points.at(sketch, b));
    if start == end {
        return None;
    }
    if bulge.abs() < 1e-12 {
        added.lines += 1;
        return Some(sketch.add_geometry(GeometryElement::Line(Line::new(start, end))));
    }
    let chord = [b[0] - a[0], b[1] - a[1]];
    let length = chord[0].hypot(chord[1]);
    // The centre sits off the chord's middle, to its left for a positive
    // bulge, by a quarter of the chord times (1 - bulge²) / bulge.
    let off = length * (1.0 - bulge * bulge) / (4.0 * bulge);
    let left = [-chord[1] / length, chord[0] / length];
    let centre = [
        (a[0] + b[0]) / 2.0 + left[0] * off,
        (a[1] + b[1]) / 2.0 + left[1] * off,
    ];
    let radius = (a[0] - centre[0]).hypot(a[1] - centre[1]);
    let centre = points.at(sketch, centre);
    // Sketch arcs run counter-clockwise from their start.
    let (from, to) = if bulge > 0.0 {
        (start, end)
    } else {
        (end, start)
    };
    added.arcs += 1;
    Some(sketch.add_geometry(GeometryElement::Arc(Arc::new(
        centre,
        from,
        to,
        radius as f32,
    ))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile;
    use kernel_api::DrawingCurve;

    fn open(points: Vec<[f64; 2]>, hidden: bool) -> DrawingCurve {
        DrawingCurve {
            hidden,
            shape: DrawingShape::Polyline {
                points,
                bulges: Vec::new(),
                closed: false,
            },
        }
    }

    fn drawing(curves: Vec<DrawingCurve>) -> Drawing2d {
        Drawing2d {
            unit_mm: None,
            curves,
        }
    }

    fn square() -> Vec<[f64; 2]> {
        vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
            [0.0, 0.0],
        ]
    }

    fn points_of(sketch: &Sketch) -> usize {
        sketch
            .geometry
            .iter()
            .filter(|g| matches!(g, GeometryElement::Point(_)))
            .count()
    }

    #[test]
    fn a_closed_polyline_is_a_closed_profile_on_shared_points() {
        let mut sketch = Sketch::new("s");
        let drawing = drawing(vec![
            open(square(), false),
            open(vec![[20.0, 0.0], [30.0, 5.0]], true),
        ]);
        let added = add_drawing(&mut sketch, &drawing, 1.0);
        assert_eq!((added.lines, added.construction), (5, 1));
        assert_eq!(
            points_of(&sketch),
            6,
            "four corners and the hidden line's two"
        );
        let wires = profile::extract_wires(&sketch).expect("the square closes");
        assert_eq!(wires.len(), 1);
        assert_eq!(wires[0].segments.len(), 4);
    }

    #[test]
    fn ends_that_meet_across_polylines_are_one_point() {
        let mut sketch = Sketch::new("s");
        // Four separate lines around a square, as a drawing of LINEs has
        // it, one end off by less than the merge distance.
        let drawing = drawing(vec![
            open(vec![[0.0, 0.0], [10.0, 0.0]], false),
            open(vec![[10.0, 0.0], [10.0, 10.0]], false),
            open(vec![[10.0, 10.0], [0.0, 10.0]], false),
            open(vec![[0.0, 10.0], [0.0, MERGE_MM / 2.0]], false),
        ]);
        add_drawing(&mut sketch, &drawing, 1.0);
        assert_eq!(points_of(&sketch), 4);
        assert_eq!(profile::extract_wires(&sketch).unwrap().len(), 1);
    }

    #[test]
    fn the_scale_turns_drawing_units_into_millimetres() {
        let mut sketch = Sketch::new("s");
        let drawing = drawing(vec![open(vec![[1.0, 2.0], [3.0, 2.0], [3.0, 2.0]], false)]);
        let added = add_drawing(&mut sketch, &drawing, 25.4);
        assert_eq!(added.lines, 1, "a repeated point draws no line");
        let GeometryElement::Line(line) = sketch.geometry.last().unwrap() else {
            panic!("a line");
        };
        let end = sketch.point_position(line.end).unwrap();
        assert!((end.x - 76.2).abs() < 1e-4 && (end.y - 50.8).abs() < 1e-4);
    }

    /// A bulged segment is the arc it describes: a closed slot of two
    /// lines and two half circles is one closed profile.
    #[test]
    fn a_bulged_polyline_is_lines_and_arcs() {
        let mut sketch = Sketch::new("s");
        let slot = DrawingCurve {
            hidden: false,
            shape: DrawingShape::Polyline {
                points: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 4.0], [0.0, 4.0]],
                // A half turn is a bulge of tan(180° / 4) = 1.
                bulges: vec![0.0, 1.0, 0.0, 1.0],
                closed: true,
            },
        };
        let added = add_drawing(&mut sketch, &drawing(vec![slot]), 1.0);
        assert_eq!((added.lines, added.arcs), (2, 2));
        let arcs: Vec<&Arc> = sketch
            .geometry
            .iter()
            .filter_map(|g| match g {
                GeometryElement::Arc(a) => Some(a),
                _ => None,
            })
            .collect();
        for arc in &arcs {
            assert!((arc.radius - 2.0).abs() < 1e-5, "{}", arc.radius);
        }
        let right = sketch.point_position(arcs[0].center).unwrap();
        assert!((right.x - 10.0).abs() < 1e-5 && (right.y - 2.0).abs() < 1e-5);
        let wires = profile::extract_wires(&sketch).expect("the slot closes");
        assert_eq!(wires.len(), 1);
    }

    /// Circles, arcs and ellipses come in as the sketch's own; a spline as
    /// the lines through its points.
    #[test]
    fn circles_arcs_ellipses_and_splines_come_in() {
        let mut sketch = Sketch::new("s");
        let curve = |shape| DrawingCurve {
            hidden: false,
            shape,
        };
        let half_pi = std::f64::consts::FRAC_PI_2;
        let added = add_drawing(
            &mut sketch,
            &drawing(vec![
                curve(DrawingShape::Circle {
                    centre: [0.0, 0.0],
                    radius: 3.0,
                }),
                curve(DrawingShape::Arc {
                    centre: [10.0, 0.0],
                    radius: 2.0,
                    start_angle: 0.0,
                    end_angle: half_pi,
                }),
                curve(DrawingShape::Ellipse {
                    centre: [20.0, 0.0],
                    major: [4.0, 0.0],
                    ratio: 0.5,
                    start_param: 0.0,
                    end_param: half_pi,
                }),
                curve(DrawingShape::Spline {
                    points: vec![[30.0, 0.0], [31.0, 1.0], [32.0, 0.0]],
                    closed: false,
                }),
            ]),
            2.0,
        );
        assert_eq!(
            (
                added.circles,
                added.arcs,
                added.ellipses,
                added.splines,
                added.lines
            ),
            (1, 1, 1, 1, 2)
        );
        let circle = sketch.geometry.iter().find_map(|g| match g {
            GeometryElement::Circle(c) => Some(c.radius),
            _ => None,
        });
        assert_eq!(circle, Some(6.0), "scaled");
        let end = sketch
            .geometry
            .iter()
            .find_map(|g| match g {
                GeometryElement::Ellipse(e) => e.arc.as_ref().map(|a| a.end),
                _ => None,
            })
            .and_then(|id| sketch.point_position(id))
            .unwrap();
        assert!(
            (end.x - 40.0).abs() < 1e-4 && (end.y - 4.0).abs() < 1e-4,
            "{end:?}"
        );
    }
}
