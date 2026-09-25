//! A 2D drawing brought into a sketch: every polyline as lines drawn end
//! to end, visible curves as geometry, hidden ones as construction.
//!
//! Ends that meet share one point, within [`MERGE_MM`], across polylines as
//! much as within one: that shared topology is what makes a drawn outline
//! a closed profile.

use std::collections::HashMap;

use kernel_api::Drawing2d;
use uuid::Uuid;

use crate::sketch::{GeometryElement, Line, Point, Sketch, Vec2D};

/// Ends closer than this, in millimetres, are one point.
pub(crate) const MERGE_MM: f64 = 1e-4;

/// What a drawing added to a sketch.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Added {
    pub lines: usize,
    pub construction: usize,
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

/// Add `drawing`'s polylines to `sketch`, scaled by `scale` (millimetres
/// per drawing unit). Segments too short to have two ends are left out.
pub(crate) fn add_drawing(sketch: &mut Sketch, drawing: &Drawing2d, scale: f64) -> Added {
    let mut points = Points::default();
    let mut added = Added::default();
    for (curves, construction) in [(&drawing.visible, false), (&drawing.hidden, true)] {
        for curve in curves {
            let ids: Vec<Uuid> = curve
                .iter()
                .map(|p| points.at(sketch, [p[0] * scale, p[1] * scale]))
                .collect();
            for pair in ids.windows(2) {
                if pair[0] == pair[1] {
                    continue;
                }
                let line = sketch.add_geometry(GeometryElement::Line(Line::new(pair[0], pair[1])));
                if construction {
                    sketch.set_construction(line, true);
                    added.construction += 1;
                } else {
                    added.lines += 1;
                }
            }
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile;

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
        let drawing = Drawing2d {
            visible: vec![square()],
            hidden: vec![vec![[20.0, 0.0], [30.0, 5.0]]],
        };
        let added = add_drawing(&mut sketch, &drawing, 1.0);
        assert_eq!(
            added,
            Added {
                lines: 4,
                construction: 1
            }
        );
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
        let drawing = Drawing2d {
            visible: vec![
                vec![[0.0, 0.0], [10.0, 0.0]],
                vec![[10.0, 0.0], [10.0, 10.0]],
                vec![[10.0, 10.0], [0.0, 10.0]],
                vec![[0.0, 10.0], [0.0, MERGE_MM / 2.0]],
            ],
            hidden: Vec::new(),
        };
        add_drawing(&mut sketch, &drawing, 1.0);
        assert_eq!(points_of(&sketch), 4);
        assert_eq!(profile::extract_wires(&sketch).unwrap().len(), 1);
    }

    #[test]
    fn the_scale_turns_drawing_units_into_millimetres() {
        let mut sketch = Sketch::new("s");
        let drawing = Drawing2d {
            visible: vec![vec![[1.0, 2.0], [3.0, 2.0], [3.0, 2.0]]],
            hidden: Vec::new(),
        };
        let added = add_drawing(&mut sketch, &drawing, 25.4);
        assert_eq!(added.lines, 1, "a repeated point draws no line");
        let GeometryElement::Line(line) = sketch.geometry.last().unwrap() else {
            panic!("a line");
        };
        let end = sketch.point_position(line.end).unwrap();
        assert!((end.x - 76.2).abs() < 1e-4 && (end.y - 50.8).abs() < 1e-4);
    }
}
