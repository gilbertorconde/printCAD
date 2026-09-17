//! Constraint tools: which constraint a toolbar button creates for the
//! current selection. The buttons are Action tools enabled by the shape of
//! the selection; dimensional kinds are created at the measured value so
//! the geometry never jumps.

use std::collections::HashSet;

use uuid::Uuid;

use crate::sketch::{
    self, AxisDirection, ConstraintKind, GeometryElement, ORIGIN_ID, Reference, Sketch, Vec2D,
    X_AXIS_ID, Y_AXIS_ID,
};

/// The selection sorted by element kind, in sketch order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectionShape {
    pub all: Vec<Uuid>,
    pub points: Vec<Uuid>,
    pub lines: Vec<Uuid>,
    /// Circles and arcs.
    pub circles: Vec<Uuid>,
    pub ellipses: Vec<Uuid>,
}

impl SelectionShape {
    pub fn of(sketch: &Sketch, selected: &HashSet<Uuid>) -> Self {
        let mut shape = Self::default();
        for g in &sketch.geometry {
            let id = g.id();
            if !selected.contains(&id) {
                continue;
            }
            shape.all.push(id);
            match g {
                GeometryElement::Point(_) => shape.points.push(id),
                GeometryElement::Line(_) => shape.lines.push(id),
                GeometryElement::Circle(_) | GeometryElement::Arc(_) => shape.circles.push(id),
                GeometryElement::Ellipse(_) => shape.ellipses.push(id),
                GeometryElement::BSpline(_) => {}
            }
        }
        // The origin and the axes hold no entry in `geometry`, but they take
        // constraints like the geometry that does. They come last and in a
        // fixed order, so a selection reads the same every time.
        for id in [ORIGIN_ID, X_AXIS_ID, Y_AXIS_ID] {
            if !selected.contains(&id) {
                continue;
            }
            shape.all.push(id);
            match Reference::of(id) {
                Some(reference) if reference.is_point() => shape.points.push(id),
                _ => shape.lines.push(id),
            }
        }
        shape
    }

    fn total(&self) -> usize {
        self.all.len()
    }

    fn only(&self, points: usize, lines: usize, circles: usize, ellipses: usize) -> bool {
        self.points.len() == points
            && self.lines.len() == lines
            && self.circles.len() == circles
            && self.ellipses.len() == ellipses
            && self.total() == points + lines + circles + ellipses
    }
}

/// The constraint tool ids, without the `sketch.constrain.` prefix.
#[cfg(test)]
pub const TOOLS: &[&str] = &[
    "coincident",
    "point_on_object",
    "midpoint",
    "vertical",
    "horizontal",
    "parallel",
    "perpendicular",
    "tangent",
    "equal",
    "symmetric",
    "block",
    "lock",
    "distance_x",
    "distance_y",
    "distance",
    "radius",
    "diameter",
    "angle",
    "angle_x",
    "angle_y",
];

fn axis_distance(horizontal: bool, a: Uuid, b: Option<Uuid>, value: f32) -> ConstraintKind {
    if horizontal {
        ConstraintKind::DistanceX { a, b, value }
    } else {
        ConstraintKind::DistanceY { a, b, value }
    }
}

/// The constraints `tool` creates for `shape`, or `None` when the selection
/// does not fit the tool. Dimensional kinds carry the value measured on
/// `sketch`.
pub fn kinds_for(
    tool: &str,
    shape: &SelectionShape,
    sketch: &Sketch,
) -> Option<Vec<ConstraintKind>> {
    let measured = |kind: &ConstraintKind| sketch::measured_value(sketch, kind).unwrap_or(0.0);
    let (p, l, c, e) = (&shape.points, &shape.lines, &shape.circles, &shape.ellipses);
    let kinds = match tool {
        "coincident" if shape.only(2, 0, 0, 0) => vec![ConstraintKind::Coincident {
            point1: p[0],
            point2: p[1],
        }],
        "point_on_object" if shape.only(1, 1, 0, 0) => vec![ConstraintKind::PointOnLine {
            point: p[0],
            line: l[0],
        }],
        "point_on_object" if shape.only(1, 0, 1, 0) => vec![ConstraintKind::PointOnCircle {
            point: p[0],
            circle: c[0],
        }],
        "point_on_object" if shape.only(1, 0, 0, 1) => vec![ConstraintKind::PointOnEllipse {
            point: p[0],
            ellipse: e[0],
        }],
        "midpoint" if shape.only(1, 1, 0, 0) => vec![ConstraintKind::Midpoint {
            point: p[0],
            line: l[0],
        }],
        "horizontal" if !l.is_empty() && shape.only(0, l.len(), 0, 0) => l
            .iter()
            .map(|line| ConstraintKind::Horizontal { element: *line })
            .collect(),
        "vertical" if !l.is_empty() && shape.only(0, l.len(), 0, 0) => l
            .iter()
            .map(|line| ConstraintKind::Vertical { element: *line })
            .collect(),
        "parallel" if shape.only(0, 2, 0, 0) => vec![ConstraintKind::Parallel {
            line1: l[0],
            line2: l[1],
        }],
        "perpendicular" if shape.only(0, 2, 0, 0) => vec![ConstraintKind::Perpendicular {
            line1: l[0],
            line2: l[1],
        }],
        "tangent" if shape.only(0, 0, 2, 0) => vec![ConstraintKind::Tangent {
            line_or_circle1: c[0],
            item2: c[1],
        }],
        "tangent" if shape.only(0, 1, 1, 0) => vec![ConstraintKind::Tangent {
            line_or_circle1: l[0],
            item2: c[0],
        }],
        "equal" if shape.only(0, 2, 0, 0) => vec![ConstraintKind::EqualLength {
            line1: l[0],
            line2: l[1],
        }],
        "equal" if shape.only(0, 0, 2, 0) => vec![ConstraintKind::EqualRadius {
            circle1: c[0],
            circle2: c[1],
        }],
        "symmetric" if shape.only(2, 1, 0, 0) => vec![ConstraintKind::Symmetric {
            point1: p[0],
            point2: p[1],
            line: l[0],
        }],
        "symmetric" if shape.only(3, 0, 0, 0) => vec![ConstraintKind::SymmetricAboutPoint {
            point1: p[0],
            point2: p[1],
            center: p[2],
        }],
        "block" if shape.total() >= 1 => shape
            .all
            .iter()
            .map(|id| ConstraintKind::Block { element: *id })
            .collect(),
        "lock" if shape.only(1, 0, 0, 0) => vec![ConstraintKind::FixedPoint {
            point: p[0],
            position: sketch.point_position(p[0]).unwrap_or(Vec2D::new(0.0, 0.0)),
        }],
        "distance_x" | "distance_y" if shape.only(1, 0, 0, 0) || shape.only(2, 0, 0, 0) => {
            let horizontal = tool == "distance_x";
            let b = p.get(1).copied();
            let kind = axis_distance(horizontal, p[0], b, 0.0);
            vec![axis_distance(horizontal, p[0], b, measured(&kind))]
        }
        "distance" if shape.only(2, 0, 0, 0) => {
            let kind = ConstraintKind::Distance {
                point1: p[0],
                point2: p[1],
                distance: 0.0,
            };
            vec![ConstraintKind::Distance {
                point1: p[0],
                point2: p[1],
                distance: measured(&kind),
            }]
        }
        "distance" if shape.only(0, 1, 0, 0) => {
            let kind = ConstraintKind::Length {
                line: l[0],
                length: 0.0,
            };
            vec![ConstraintKind::Length {
                line: l[0],
                length: measured(&kind),
            }]
        }
        "radius" if shape.only(0, 0, 1, 0) => {
            let kind = ConstraintKind::Radius {
                circle: c[0],
                radius: 0.0,
            };
            vec![ConstraintKind::Radius {
                circle: c[0],
                radius: measured(&kind),
            }]
        }
        "diameter" if shape.only(0, 0, 1, 0) => {
            let kind = ConstraintKind::Diameter {
                circle: c[0],
                diameter: 0.0,
            };
            vec![ConstraintKind::Diameter {
                circle: c[0],
                diameter: measured(&kind),
            }]
        }
        "angle" if shape.only(0, 2, 0, 0) => {
            let kind = ConstraintKind::Angle {
                line1: l[0],
                line2: l[1],
                angle_rad: 0.0,
            };
            vec![ConstraintKind::Angle {
                line1: l[0],
                line2: l[1],
                angle_rad: measured(&kind).to_radians(),
            }]
        }
        "angle_x" | "angle_y" if shape.only(0, 1, 0, 0) => {
            let axis = if tool == "angle_x" {
                AxisDirection::Horizontal
            } else {
                AxisDirection::Vertical
            };
            let kind = ConstraintKind::AngleToAxis {
                line: l[0],
                axis,
                angle_rad: 0.0,
            };
            vec![ConstraintKind::AngleToAxis {
                line: l[0],
                axis,
                angle_rad: measured(&kind).to_radians(),
            }]
        }
        _ => return None,
    };
    Some(kinds)
}

/// Whether `tool` applies to `shape` at all.
pub fn fits(tool: &str, shape: &SelectionShape) -> bool {
    kinds_for(tool, shape, &Sketch::new("")).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Circle, Line, Point};

    fn sketch_with_line_and_circle() -> (Sketch, Uuid, Uuid, Uuid, Uuid) {
        let mut sketch = Sketch::new("t");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 0.0))));
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let center = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, 5.0))));
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
        (sketch, a, b, line, circle)
    }

    #[test]
    fn a_line_alone_takes_length_and_orientation_tools() {
        let (sketch, _, _, line, _) = sketch_with_line_and_circle();
        let shape = SelectionShape::of(&sketch, &HashSet::from([line]));
        assert!(fits("horizontal", &shape));
        assert!(fits("distance", &shape));
        assert!(fits("angle_x", &shape));
        assert!(!fits("radius", &shape));
        assert!(!fits("coincident", &shape));
        let kinds = kinds_for("distance", &shape, &sketch).unwrap();
        assert!(
            matches!(kinds[0], ConstraintKind::Length { length, .. } if (length - 10.0).abs() < 1e-4)
        );
    }

    #[test]
    fn point_on_object_dispatches_by_the_other_element() {
        let (sketch, a, _, line, circle) = sketch_with_line_and_circle();
        let on_line = SelectionShape::of(&sketch, &HashSet::from([a, line]));
        assert!(matches!(
            kinds_for("point_on_object", &on_line, &sketch).unwrap()[0],
            ConstraintKind::PointOnLine { .. }
        ));
        let on_circle = SelectionShape::of(&sketch, &HashSet::from([a, circle]));
        assert!(matches!(
            kinds_for("point_on_object", &on_circle, &sketch).unwrap()[0],
            ConstraintKind::PointOnCircle { .. }
        ));
    }

    #[test]
    fn block_takes_any_selection_and_nothing_takes_an_empty_one() {
        let (sketch, a, _, line, _) = sketch_with_line_and_circle();
        let shape = SelectionShape::of(&sketch, &HashSet::from([a, line]));
        assert_eq!(kinds_for("block", &shape, &sketch).unwrap().len(), 2);
        let empty = SelectionShape::default();
        assert!(TOOLS.iter().all(|t| !fits(t, &empty)));
    }
}
