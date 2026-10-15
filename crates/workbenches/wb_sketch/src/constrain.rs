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
    /// The arcs among `circles`.
    pub arcs: Vec<Uuid>,
    pub ellipses: Vec<Uuid>,
    /// Splines, parabolas and hyperbolas.
    pub others: Vec<Uuid>,
    /// Everything selected in the order it was picked; where that is not
    /// known, in sketch order.
    pub picked: Vec<Uuid>,
    /// For each selected point that ends a line or an arc: that curve, and
    /// whether it is a line.
    pub curve_ends: Vec<(Uuid, Uuid, bool)>,
}

impl SelectionShape {
    /// The selection as it was picked, `order` first (ids it does not name
    /// follow in sketch order).
    pub fn picked_in(sketch: &Sketch, selected: &HashSet<Uuid>, order: &[Uuid]) -> Self {
        let mut shape = Self::of(sketch, selected);
        let mut picked: Vec<Uuid> = order
            .iter()
            .copied()
            .filter(|id| shape.all.contains(id))
            .collect();
        for id in &shape.all {
            if !picked.contains(id) {
                picked.push(*id);
            }
        }
        shape.picked = picked;
        shape
    }

    /// The selected points in the order they were picked.
    fn points_picked(&self) -> Vec<Uuid> {
        self.picked
            .iter()
            .copied()
            .filter(|id| self.points.contains(id))
            .collect()
    }

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
                GeometryElement::Circle(_) => shape.circles.push(id),
                GeometryElement::Arc(_) => {
                    shape.circles.push(id);
                    shape.arcs.push(id);
                }
                GeometryElement::Ellipse(_) => shape.ellipses.push(id),
                GeometryElement::BSpline(_) | GeometryElement::Conic(_) => shape.others.push(id),
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
        shape.picked = shape.all.clone();
        for point in shape.points.clone() {
            let owner = sketch.geometry.iter().find_map(|g| match g {
                GeometryElement::Line(l) if l.start == point || l.end == point => {
                    Some((l.id, true))
                }
                GeometryElement::Arc(a) if a.start == point || a.end == point => {
                    Some((a.id, false))
                }
                _ => None,
            });
            if let Some((curve, line)) = owner {
                shape.curve_ends.push((point, curve, line));
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
pub const TOOLS: &[&str] = &[
    "coincident",
    "point_on_object",
    "midpoint",
    "vertical",
    "horizontal",
    "horizontal_vertical",
    "parallel",
    "perpendicular",
    "tangent",
    "equal",
    "symmetric",
    "block",
    "group",
    "lock",
    "distance_x",
    "distance_y",
    "distance",
    "radius",
    "diameter",
    "angle",
    "angle_x",
    "angle_y",
    "arc_length",
    "arc_angle",
    "angle_three_points",
    "ellipse_minor",
    "gap",
    "radius_diameter",
    "angle_at_point",
    "refraction",
];

/// The two ends of a line, an arc or an open spline.
fn open_curve_ends(sketch: &Sketch, curve: Uuid) -> Option<[Uuid; 2]> {
    match sketch.get_geometry(curve)? {
        GeometryElement::Line(l) => Some([l.start, l.end]),
        GeometryElement::Arc(a) => Some([a.start, a.end]),
        GeometryElement::BSpline(b) if !b.periodic => {
            Some([*b.control_points.first()?, *b.control_points.last()?])
        }
        _ => None,
    }
}

fn axis_distance(horizontal: bool, a: Uuid, b: Option<Uuid>, value: f64) -> ConstraintKind {
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
    // The first element and each other one after it: one constraint
    // chained across a selection of many.
    let chain =
        |ids: &[Uuid], make: &dyn Fn(Uuid, Uuid) -> ConstraintKind| -> Vec<ConstraintKind> {
            ids[1..].iter().map(|other| make(ids[0], *other)).collect()
        };
    let kinds = match tool {
        "coincident" if p.len() >= 2 && shape.only(p.len(), 0, 0, 0) => {
            chain(p, &|a, b| ConstraintKind::Coincident {
                point1: a,
                point2: b,
            })
        }
        // A point and a curve: coincident with the curve is the point on it.
        "coincident"
            if shape.only(1, 1, 0, 0) || shape.only(1, 0, 1, 0) || shape.only(1, 0, 0, 1) =>
        {
            return kinds_for("point_on_object", shape, sketch);
        }
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
        "point_on_object" if p.len() == 1 && shape.others.len() == 1 && shape.total() == 2 => {
            vec![ConstraintKind::PointOnCurve {
                point: p[0],
                curve: shape.others[0],
            }]
        }
        // Two curves at least one of which is an ellipse, a conic or a
        // spline: the general tangency and squareness.
        "tangent" | "perpendicular"
            if p.is_empty()
                && shape.total() == 2
                && !(tool == "tangent" && e.is_empty() && shape.others.is_empty())
                && !(tool == "perpendicular" && l.len() == 2) =>
        {
            let (curve1, curve2) = (shape.all[0], shape.all[1]);
            vec![if tool == "tangent" {
                ConstraintKind::TangentCurves { curve1, curve2 }
            } else {
                ConstraintKind::PerpendicularCurves { curve1, curve2 }
            }]
        }
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
        "horizontal" if shape.only(2, 0, 0, 0) => vec![ConstraintKind::HorizontalPoints {
            point1: p[0],
            point2: p[1],
        }],
        "vertical" if shape.only(2, 0, 0, 0) => vec![ConstraintKind::VerticalPoints {
            point1: p[0],
            point2: p[1],
        }],
        // Whichever of the two each line, or the two points, is nearer.
        "horizontal_vertical" if !l.is_empty() && shape.only(0, l.len(), 0, 0) => l
            .iter()
            .map(|line| {
                if nearer_horizontal(sketch, *line) {
                    ConstraintKind::Horizontal { element: *line }
                } else {
                    ConstraintKind::Vertical { element: *line }
                }
            })
            .collect(),
        "horizontal_vertical" if shape.only(2, 0, 0, 0) => {
            let level = match (sketch.point_position(p[0]), sketch.point_position(p[1])) {
                (Some(a), Some(b)) => (b.x - a.x).abs() >= (b.y - a.y).abs(),
                _ => true,
            };
            vec![if level {
                ConstraintKind::HorizontalPoints {
                    point1: p[0],
                    point2: p[1],
                }
            } else {
                ConstraintKind::VerticalPoints {
                    point1: p[0],
                    point2: p[1],
                }
            }]
        }
        "parallel" if l.len() >= 2 && shape.only(0, l.len(), 0, 0) => {
            chain(l, &|a, b| ConstraintKind::Parallel { line1: a, line2: b })
        }
        "perpendicular" if shape.only(0, 2, 0, 0) => vec![ConstraintKind::Perpendicular {
            line1: l[0],
            line2: l[1],
        }],
        // A point and the two curves that meet there: joined smoothly at it.
        "tangent"
            if p.len() == 1 && l.len() + c.len() == 2 && shape.only(1, l.len(), c.len(), 0) =>
        {
            let point = p[0];
            let curves: Vec<(Uuid, bool)> = shape
                .all
                .iter()
                .copied()
                .filter(|id| *id != point)
                .map(|id| (id, l.contains(&id)))
                .collect();
            let mut kinds: Vec<ConstraintKind> = curves
                .iter()
                .filter(|(curve, _)| !held_on(sketch, point, *curve))
                .map(|(curve, _)| on_curve(shape, point, *curve))
                .collect();
            kinds.push(tangency(curves[0], curves[1]));
            kinds
        }
        // An end of each of two curves: the ends joined, the curves smooth
        // across the join.
        "tangent" if shape.only(2, 0, 0, 0) && shape.curve_ends.len() == 2 => {
            let [(p1, c1, line1), (p2, c2, line2)] = [shape.curve_ends[0], shape.curve_ends[1]];
            if c1 == c2 {
                return None;
            }
            vec![
                ConstraintKind::Coincident {
                    point1: p1,
                    point2: p2,
                },
                tangency((c1, line1), (c2, line2)),
            ]
        }
        "tangent" if shape.only(0, 0, 2, 0) => vec![ConstraintKind::Tangent {
            line_or_circle1: c[0],
            item2: c[1],
        }],
        "tangent" if shape.only(0, 1, 1, 0) => vec![ConstraintKind::Tangent {
            line_or_circle1: l[0],
            item2: c[0],
        }],
        "equal" if l.len() >= 2 && shape.only(0, l.len(), 0, 0) => chain(l, &|a, b| {
            ConstraintKind::EqualLength { line1: a, line2: b }
        }),
        "equal" if e.len() >= 2 && shape.only(0, 0, 0, e.len()) => {
            chain(e, &|a, b| ConstraintKind::EqualEllipse {
                ellipse1: a,
                ellipse2: b,
            })
        }
        "equal" if c.len() >= 2 && shape.only(0, 0, c.len(), 0) => {
            chain(c, &|a, b| ConstraintKind::EqualRadius {
                circle1: a,
                circle2: b,
            })
        }
        "symmetric" if shape.only(2, 1, 0, 0) => vec![ConstraintKind::Symmetric {
            point1: p[0],
            point2: p[1],
            line: l[0],
        }],
        // A curve and a line, the line picked last: the curve's two ends
        // mirror each other about it.
        "symmetric"
            if p.is_empty()
                && shape.total() == 2
                && shape.picked.last().is_some_and(|id| l.contains(id))
                && [l, &shape.arcs, &shape.others]
                    .iter()
                    .any(|kind| kind.contains(&shape.picked[0])) =>
        {
            let (curve, line) = (shape.picked[0], shape.picked[1]);
            // A closed spline has no ends to mirror.
            open_curve_ends(sketch, curve)
                .map(|[point1, point2]| ConstraintKind::Symmetric {
                    point1,
                    point2,
                    line,
                })
                .into_iter()
                .collect()
        }
        // The centre is the point picked last.
        "symmetric" if shape.only(3, 0, 0, 0) => {
            let p = shape.points_picked();
            vec![ConstraintKind::SymmetricAboutPoint {
                point1: p[0],
                point2: p[1],
                center: p[2],
            }]
        }
        "block" if shape.total() >= 1 => shape
            .all
            .iter()
            .map(|id| ConstraintKind::Block { element: *id })
            .collect(),
        // The selection held as one rigid unit; the origin and the axes
        // take no part in one.
        "group" if shape.total() >= 1 => vec![sketch::group_of(sketch, &shape.all)?],
        // Where the point is, as two distances from the origin that show,
        // edit and take formulas like any dimension.
        "lock" if shape.only(1, 0, 0, 0) => {
            let at = sketch.point_position(p[0]).unwrap_or(Vec2D::new(0.0, 0.0));
            vec![
                ConstraintKind::DistanceX {
                    a: p[0],
                    b: None,
                    value: at.x.abs(),
                },
                ConstraintKind::DistanceY {
                    a: p[0],
                    b: None,
                    value: at.y.abs(),
                },
            ]
        }
        // A line: across it, from its start to its end.
        "distance_x" | "distance_y" if shape.only(0, 1, 0, 0) => {
            let Some(GeometryElement::Line(line)) = sketch.get_geometry(l[0]) else {
                return None;
            };
            let horizontal = tool == "distance_x";
            let kind = axis_distance(horizontal, line.start, Some(line.end), 0.0);
            vec![axis_distance(
                horizontal,
                line.start,
                Some(line.end),
                measured(&kind),
            )]
        }
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
        "angle" | "angle_at_point"
            if p.len() == 1 && l.len() + c.len() == 2 && shape.only(1, l.len(), c.len(), 0) =>
        {
            let point = p[0];
            let curves: Vec<Uuid> = shape
                .all
                .iter()
                .copied()
                .filter(|id| *id != point)
                .collect();
            let mut kinds: Vec<ConstraintKind> = curves
                .iter()
                .filter(|curve| !held_on(sketch, point, **curve))
                .map(|curve| on_curve(shape, point, *curve))
                .collect();
            let kind = ConstraintKind::AngleAtPoint {
                curve1: curves[0],
                curve2: curves[1],
                point,
                angle_rad: 0.0,
            };
            kinds.push(ConstraintKind::AngleAtPoint {
                curve1: curves[0],
                curve2: curves[1],
                point,
                angle_rad: measured(&kind).to_radians(),
            });
            kinds
        }
        "radius" | "ellipse_minor" if shape.only(0, 0, 0, 1) => {
            let major = tool == "radius";
            let probe = ConstraintKind::EllipseRadius {
                ellipse: e[0],
                major,
                radius: 0.0,
            };
            vec![ConstraintKind::EllipseRadius {
                ellipse: e[0],
                major,
                radius: measured(&probe),
            }]
        }
        "arc_length" if shape.others.len() == 1 && shape.total() == 1 => {
            let probe = ConstraintKind::CurveLength {
                curve: shape.others[0],
                length: 0.0,
            };
            vec![ConstraintKind::CurveLength {
                curve: shape.others[0],
                length: measured(&probe),
            }]
        }
        "arc_angle" if !shape.arcs.is_empty() && shape.only(0, 0, shape.arcs.len(), 0) => shape
            .arcs
            .iter()
            .map(|arc| {
                let kind = ConstraintKind::ArcAngle {
                    arc: *arc,
                    angle_rad: 0.0,
                };
                ConstraintKind::ArcAngle {
                    arc: *arc,
                    angle_rad: measured(&kind).to_radians(),
                }
            })
            .collect(),
        // Picked arm, corner, arm; the angle is taken the short way round.
        "angle_three_points" if shape.only(3, 0, 0, 0) => {
            let p = shape.points_picked();
            let (mut point1, vertex, mut point2) = (p[0], p[1], p[2]);
            let probe = ConstraintKind::AngleThreePoints {
                point1,
                vertex,
                point2,
                angle_rad: 0.0,
            };
            let mut degrees = measured(&probe);
            if degrees > 180.0 {
                std::mem::swap(&mut point1, &mut point2);
                degrees = 360.0 - degrees;
            }
            vec![ConstraintKind::AngleThreePoints {
                point1,
                vertex,
                point2,
                angle_rad: degrees.to_radians(),
            }]
        }
        "arc_length" if !shape.arcs.is_empty() && shape.only(0, 0, shape.arcs.len(), 0) => shape
            .arcs
            .iter()
            .map(|arc| {
                let kind = ConstraintKind::ArcLength {
                    arc: *arc,
                    length: 0.0,
                };
                ConstraintKind::ArcLength {
                    arc: *arc,
                    length: measured(&kind),
                }
            })
            .collect(),
        "distance" | "gap"
            if shape.only(0, 2, 0, 0)
                || shape.only(1, 1, 0, 0)
                || shape.only(1, 0, 1, 0)
                || shape.only(0, 1, 1, 0)
                || shape.only(0, 0, 2, 0) =>
        {
            let (item1, item2) = (shape.all[0], shape.all[1]);
            let kind = ConstraintKind::Gap {
                item1,
                item2,
                distance: 0.0,
            };
            vec![ConstraintKind::Gap {
                item1,
                item2,
                distance: measured(&kind),
            }]
        }
        "radius_diameter" if !c.is_empty() && shape.only(0, 0, c.len(), 0) => {
            // The first takes the dimension its kind is given, the rest
            // are made equal to it.
            let first = c[0];
            let dimension = if shape.arcs.contains(&first) {
                let kind = ConstraintKind::Radius {
                    circle: first,
                    radius: 0.0,
                };
                ConstraintKind::Radius {
                    circle: first,
                    radius: measured(&kind),
                }
            } else {
                let kind = ConstraintKind::Diameter {
                    circle: first,
                    diameter: 0.0,
                };
                ConstraintKind::Diameter {
                    circle: first,
                    diameter: measured(&kind),
                }
            };
            std::iter::once(dimension)
                .chain(c[1..].iter().map(|other| ConstraintKind::EqualRadius {
                    circle1: first,
                    circle2: *other,
                }))
                .collect()
        }
        "refraction" if shape.only(1, 2, 1, 0) || shape.only(1, 3, 0, 0) => {
            let point = p[0];
            let (rays, interface) = if c.is_empty() {
                // The rays are the lines that end at the point; the
                // interface is the one it lies on.
                let ending: Vec<Uuid> = l
                    .iter()
                    .copied()
                    .filter(|line| ends_at(sketch, *line, point))
                    .collect();
                match ending.as_slice() {
                    [a, b] => {
                        let rest = l.iter().copied().find(|line| !ending.contains(line));
                        ([*a, *b], rest.unwrap_or(l[2]))
                    }
                    _ => ([l[0], l[1]], l[2]),
                }
            } else {
                ([l[0], l[1]], c[0])
            };
            let mut kinds = Vec::new();
            if !held_on(sketch, point, interface) {
                kinds.push(on_curve(shape, point, interface));
            }
            for ray in rays {
                if !ends_at(sketch, ray, point)
                    && let Some(end) = nearer_end(sketch, ray, point)
                {
                    kinds.push(ConstraintKind::Coincident {
                        point1: end,
                        point2: point,
                    });
                }
            }
            let ratio =
                crate::measure::refraction_ratio(sketch, rays[0], rays[1], interface, point)
                    .filter(|r| r.is_finite() && r.abs() > 1e-6)
                    .unwrap_or(1.0);
            kinds.push(ConstraintKind::Refraction {
                ray1: rays[0],
                ray2: rays[1],
                interface,
                point,
                ratio,
            });
            kinds
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

/// The points that stand for `point`: itself and those a coincidence joins
/// it to.
fn same_points(sketch: &Sketch, point: Uuid) -> Vec<Uuid> {
    let mut out = vec![point];
    for c in &sketch.constraints {
        if let ConstraintKind::Coincident { point1, point2 } = c.kind {
            if point1 == point {
                out.push(point2);
            } else if point2 == point {
                out.push(point1);
            }
        }
    }
    out
}

/// Whether `line` (a line or an arc) has an end at `point`.
fn ends_at(sketch: &Sketch, line: Uuid, point: Uuid) -> bool {
    let ends = match sketch.get_geometry(line) {
        Some(GeometryElement::Line(l)) => [l.start, l.end],
        Some(GeometryElement::Arc(a)) => [a.start, a.end],
        _ => return false,
    };
    same_points(sketch, point).iter().any(|p| ends.contains(p))
}

/// The end of a line nearer `point`.
fn nearer_end(sketch: &Sketch, line: Uuid, point: Uuid) -> Option<Uuid> {
    let Some(GeometryElement::Line(l)) = sketch.get_geometry(line) else {
        return None;
    };
    let at = sketch.point_position(point)?.to_glam();
    let d = |id: Uuid| {
        sketch
            .point_position(id)
            .map_or(f64::INFINITY, |p| (p.to_glam() - at).length())
    };
    Some(if d(l.start) <= d(l.end) {
        l.start
    } else {
        l.end
    })
}

/// Whether `point` already lies on `curve`: at one of its ends, or held
/// there by a point-on constraint.
fn held_on(sketch: &Sketch, point: Uuid, curve: Uuid) -> bool {
    if ends_at(sketch, curve, point) {
        return true;
    }
    let same = same_points(sketch, point);
    sketch.constraints.iter().any(|c| match c.kind {
        ConstraintKind::PointOnLine { point, line } => line == curve && same.contains(&point),
        ConstraintKind::PointOnCircle { point, circle } => circle == curve && same.contains(&point),
        ConstraintKind::PointOnEllipse { point, ellipse } => {
            ellipse == curve && same.contains(&point)
        }
        ConstraintKind::PointOnCurve { point, curve: on } => on == curve && same.contains(&point),
        _ => false,
    })
}

/// Two curves that meet held smooth there: two lines in one line, a line
/// and a circle or two circles tangent (`true` marks a line).
fn tangency((a, a_line): (Uuid, bool), (b, b_line): (Uuid, bool)) -> ConstraintKind {
    match (a_line, b_line) {
        (true, true) => ConstraintKind::Parallel { line1: a, line2: b },
        (false, true) => ConstraintKind::Tangent {
            line_or_circle1: b,
            item2: a,
        },
        _ => ConstraintKind::Tangent {
            line_or_circle1: a,
            item2: b,
        },
    }
}

/// The constraint that keeps `point` on `curve`.
fn on_curve(shape: &SelectionShape, point: Uuid, curve: Uuid) -> ConstraintKind {
    if shape.lines.contains(&curve) {
        ConstraintKind::PointOnLine { point, line: curve }
    } else if shape.ellipses.contains(&curve) {
        ConstraintKind::PointOnEllipse {
            point,
            ellipse: curve,
        }
    } else if shape.others.contains(&curve) {
        ConstraintKind::PointOnCurve { point, curve }
    } else {
        ConstraintKind::PointOnCircle {
            point,
            circle: curve,
        }
    }
}

/// The dimensional tool the Dimension button takes for a selection: an
/// angle for two lines or two curves and the point they meet at, a
/// distance or a gap for two items, a radius or diameter (by kind) for
/// circles and arcs, a length for a line. An arc's length is never
/// guessed: it has its own tool.
pub fn dimension_for(shape: &SelectionShape) -> Option<&'static str> {
    [
        "angle",
        "distance",
        "radius_diameter",
        "distance_x",
        "distance_y",
        // An ellipse: its major radius.
        "radius",
    ]
    .into_iter()
    .find(|tool| fits(tool, shape))
}

/// Whether a line lies nearer level than upright (a line not in the sketch
/// counts as level).
fn nearer_horizontal(sketch: &Sketch, line: Uuid) -> bool {
    match crate::measure::item(sketch, line) {
        Some(crate::measure::Item::Line(s, e)) => (e.x - s.x).abs() >= (e.y - s.y).abs(),
        _ => true,
    }
}

/// [`dimension_for`], reading the sketch: two parallel lines take the
/// distance between them, since the angle they make is none.
pub fn dimension_in(shape: &SelectionShape, sketch: &Sketch) -> Option<&'static str> {
    if shape.only(0, 2, 0, 0) && parallel(sketch, shape.lines[0], shape.lines[1]) {
        return Some("distance");
    }
    dimension_for(shape)
}

/// Whether two lines run the same way, within half a degree.
fn parallel(sketch: &Sketch, a: Uuid, b: Uuid) -> bool {
    use crate::measure::Item;
    match (
        crate::measure::item(sketch, a),
        crate::measure::item(sketch, b),
    ) {
        (Some(Item::Line(s1, e1)), Some(Item::Line(s2, e2))) => {
            let (d1, d2) = ((e1 - s1).normalize_or_zero(), (e2 - s2).normalize_or_zero());
            d1.perp_dot(d2).abs() < 0.5f64.to_radians().sin()
        }
        _ => false,
    }
}

/// Whether `tool` applies to `shape` at all.
pub fn fits(tool: &str, shape: &SelectionShape) -> bool {
    kinds_for(tool, shape, &Sketch::new("")).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Arc, Circle, Line, Point};

    fn sketch_with_line_and_circle() -> (Sketch, Uuid, Uuid, Uuid, Uuid) {
        let mut sketch = Sketch::new("t");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 0.0))));
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let center = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, 5.0))));
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
        (sketch, a, b, line, circle)
    }

    /// A curve and a line picked after it: the curve's ends mirror about
    /// the line. Of two lines, the one picked last is the axis.
    #[test]
    fn symmetric_takes_a_curve_and_its_symmetry_line() {
        let (mut sketch, a, b, line, _) = sketch_with_line_and_circle();
        let c = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, -5.0))));
        let d = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, 9.0))));
        let axis = sketch.add_geometry(GeometryElement::Line(Line::new(c, d)));
        let picked = |order: &[Uuid]| {
            SelectionShape::picked_in(&sketch, &order.iter().copied().collect(), order)
        };
        let kinds = kinds_for("symmetric", &picked(&[line, axis]), &sketch).unwrap();
        assert!(matches!(
            kinds[..],
            [ConstraintKind::Symmetric { point1, point2, line: l }]
                if point1 == a && point2 == b && l == axis
        ));
        assert!(fits("symmetric", &picked(&[line, axis])));
        let flipped = kinds_for("symmetric", &picked(&[axis, line]), &sketch).unwrap();
        assert!(
            matches!(
                flipped[..],
                [ConstraintKind::Symmetric { point1, point2, line: l }]
                    if point1 == c && point2 == d && l == line
            ),
            "the line picked last is the axis"
        );
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

    /// A line, a circle, and an arc whose start the line ends at.
    fn sketch_with_an_arc() -> (Sketch, Uuid, Uuid, Uuid, Uuid) {
        let (mut sketch, _, b, line, circle) = sketch_with_line_and_circle();
        let center = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 5.0))));
        let end = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(15.0, 5.0))));
        let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(center, b, end, 5.0)));
        (sketch, b, line, circle, arc)
    }

    #[test]
    fn an_arc_takes_its_length_and_a_circle_does_not() {
        let (sketch, _, _, circle, arc) = sketch_with_an_arc();
        let shape = SelectionShape::of(&sketch, &HashSet::from([arc]));
        let kinds = kinds_for("arc_length", &shape, &sketch).unwrap();
        let quarter = 5.0 * std::f64::consts::FRAC_PI_2;
        assert!(
            matches!(kinds[0], ConstraintKind::ArcLength { length, .. } if (length - quarter).abs() < 1e-4),
            "{kinds:?}"
        );
        let circle_only = SelectionShape::of(&sketch, &HashSet::from([circle]));
        assert!(!fits("arc_length", &circle_only));
        let both = SelectionShape::of(&sketch, &HashSet::from([circle, arc]));
        assert!(!fits("arc_length", &both));
    }

    #[test]
    fn two_curves_or_a_point_and_a_curve_take_a_gap() {
        let (sketch, b, line, circle, arc) = sketch_with_an_arc();
        for pair in [[line, circle], [circle, arc], [b, circle], [b, line]] {
            let shape = SelectionShape::of(&sketch, &HashSet::from(pair));
            assert!(fits("gap", &shape), "{pair:?}");
            let kinds = kinds_for("distance", &shape, &sketch).unwrap();
            assert!(matches!(kinds[0], ConstraintKind::Gap { .. }), "{kinds:?}");
        }
        let shape = SelectionShape::of(&sketch, &HashSet::from([line, circle]));
        let kinds = kinds_for("gap", &shape, &sketch).unwrap();
        assert!(
            matches!(kinds[0], ConstraintKind::Gap { distance, .. } if (distance - 3.0).abs() < 1e-4),
            "the circle at (5, 5) with radius 2 stands 3 off the line: {kinds:?}"
        );
        let mut crossing = sketch.clone();
        let p = crossing.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 9.0))));
        let other = crossing.add_geometry(GeometryElement::Line(Line::new(b, p)));
        let two_lines = SelectionShape::of(&crossing, &HashSet::from([line, other]));
        assert_eq!(
            dimension_in(&two_lines, &crossing),
            Some("angle"),
            "two crossing lines take an angle from the dimension tool"
        );
    }

    #[test]
    fn a_locked_point_is_two_distances_from_the_origin_and_stays() {
        let mut sketch = Sketch::new("t");
        let p = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(-5.0, -3.0))));
        let kinds = kinds_for(
            "lock",
            &SelectionShape::of(&sketch, &HashSet::from([p])),
            &sketch,
        )
        .unwrap();
        assert!(matches!(
            kinds[..],
            [
                ConstraintKind::DistanceX { value: x, .. },
                ConstraintKind::DistanceY { value: y, .. },
            ] if (x - 5.0).abs() < 1e-6 && (y - 3.0).abs() < 1e-6
        ));
        for kind in kinds {
            sketch.add_constraint(kind);
        }
        assert!(matches!(
            crate::solver::solve(&mut sketch),
            crate::operations::SolveOutcome::Converged { .. }
        ));
        let at = sketch.point_position(p).unwrap();
        assert!(
            (at.x + 5.0).abs() < 1e-4 && (at.y + 3.0).abs() < 1e-4,
            "{at:?}"
        );
    }

    #[test]
    fn two_points_level_or_plumb_and_the_nearer_of_the_two() {
        let mut sketch = Sketch::new("t");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 1.0))));
        let c = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(1.0, 10.0))));
        let pair = |x, y| SelectionShape::of(&sketch, &HashSet::from([x, y]));
        for (tool, other, level) in [
            ("horizontal", b, true),
            ("vertical", c, false),
            ("horizontal_vertical", b, true),
            ("horizontal_vertical", c, false),
        ] {
            let kinds = kinds_for(tool, &pair(a, other), &sketch).unwrap();
            let took = match kinds[..] {
                [ConstraintKind::HorizontalPoints { .. }] => true,
                [ConstraintKind::VerticalPoints { .. }] => false,
                _ => panic!("{tool}: {kinds:?}"),
            };
            assert_eq!(took, level, "{tool}: whether level");
        }
        let slanted = sketch.add_geometry(GeometryElement::Line(Line::new(a, c)));
        assert!(matches!(
            kinds_for(
                "horizontal_vertical",
                &SelectionShape::of(&sketch, &HashSet::from([slanted])),
                &sketch
            )
            .unwrap()[..],
            [ConstraintKind::Vertical { .. }]
        ));
    }

    #[test]
    fn one_constraint_chains_across_many_and_coincident_falls_to_on_object() {
        let mut sketch = Sketch::new("t");
        let mut circles = Vec::new();
        let mut points = Vec::new();
        for i in 0..4 {
            let center = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(
                i as f64 * 10.0,
                0.0,
            ))));
            points.push(center);
            circles.push(
                sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 1.0 + i as f64))),
            );
        }
        let many = |ids: &[Uuid]| SelectionShape::of(&sketch, &ids.iter().copied().collect());
        let equal = kinds_for("equal", &many(&circles), &sketch).unwrap();
        assert_eq!(equal.len(), 3, "each held equal to the first: {equal:?}");
        assert!(equal.iter().all(|k| matches!(
            k,
            ConstraintKind::EqualRadius { circle1, .. } if *circle1 == circles[0]
        )));
        let coincident = kinds_for("coincident", &many(&points), &sketch).unwrap();
        assert_eq!(coincident.len(), 3);
        let on = kinds_for("coincident", &many(&[points[0], circles[1]]), &sketch).unwrap();
        assert!(
            matches!(on[..], [ConstraintKind::PointOnCircle { .. }]),
            "a point and a curve: on it: {on:?}"
        );
    }

    #[test]
    fn radius_or_diameter_goes_by_kind_and_the_rest_are_equal() {
        let (sketch, _, _, circle, arc) = sketch_with_an_arc();
        let circle_only = SelectionShape::of(&sketch, &HashSet::from([circle]));
        assert!(matches!(
            kinds_for("radius_diameter", &circle_only, &sketch).unwrap()[..],
            [ConstraintKind::Diameter { diameter, .. }] if (diameter - 4.0).abs() < 1e-4
        ));
        let arc_only = SelectionShape::of(&sketch, &HashSet::from([arc]));
        assert!(matches!(
            kinds_for("radius_diameter", &arc_only, &sketch).unwrap()[..],
            [ConstraintKind::Radius { radius, .. }] if (radius - 5.0).abs() < 1e-4
        ));
        let both = SelectionShape::of(&sketch, &HashSet::from([circle, arc]));
        let kinds = kinds_for("radius_diameter", &both, &sketch).unwrap();
        assert!(
            matches!(
                kinds[..],
                [
                    ConstraintKind::Diameter { circle: first, .. },
                    ConstraintKind::EqualRadius { circle1, circle2 },
                ] if first == circle && circle1 == circle && circle2 == arc
            ),
            "{kinds:?}"
        );
    }

    #[test]
    fn an_angle_at_a_point_keeps_the_point_on_the_curve_it_is_not_an_end_of() {
        let (sketch, b, line, circle, arc) = sketch_with_an_arc();
        let shape = SelectionShape::of(&sketch, &HashSet::from([b, line, arc]));
        let kinds = kinds_for("angle", &shape, &sketch).unwrap();
        assert!(
            matches!(kinds[..], [ConstraintKind::AngleAtPoint { point, .. }] if point == b),
            "the point ends both curves, so nothing else is needed: {kinds:?}"
        );
        let shape = SelectionShape::of(&sketch, &HashSet::from([b, line, circle]));
        let kinds = kinds_for("angle_at_point", &shape, &sketch).unwrap();
        assert!(
            matches!(
                kinds[..],
                [
                    ConstraintKind::PointOnCircle { point, circle: on },
                    ConstraintKind::AngleAtPoint { .. },
                ] if point == b && on == circle
            ),
            "{kinds:?}"
        );
    }

    #[test]
    fn a_refraction_tells_its_rays_from_the_interface() {
        let mut sketch = Sketch::new("t");
        let point = |sketch: &mut Sketch, x: f64, y: f64| {
            sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
        };
        let l = point(&mut sketch, -10.0, 0.0);
        let r = point(&mut sketch, 10.0, 0.0);
        let source = point(&mut sketch, -4.0, 4.0);
        let hit = point(&mut sketch, 0.0, 0.0);
        let exit = point(&mut sketch, 3.0, -4.0);
        let ray1 = sketch.add_geometry(GeometryElement::Line(Line::new(source, hit)));
        // The interface drawn between the two rays, so sketch order alone
        // would take it for a ray.
        let interface = sketch.add_geometry(GeometryElement::Line(Line::new(l, r)));
        let ray2 = sketch.add_geometry(GeometryElement::Line(Line::new(hit, exit)));
        let shape = SelectionShape::of(&sketch, &HashSet::from([hit, ray1, ray2, interface]));
        let kinds = kinds_for("refraction", &shape, &sketch).unwrap();
        let [
            ConstraintKind::PointOnLine { point: on, line },
            ConstraintKind::Refraction {
                ray1: r1,
                ray2: r2,
                interface: i,
                ratio,
                ..
            },
        ] = kinds[..]
        else {
            panic!("{kinds:?}");
        };
        assert_eq!((on, line), (hit, interface));
        assert_eq!((r1, r2, i), (ray1, ray2, interface));
        let expected = 45f64.to_radians().sin() / (3.0f64 / 5.0);
        assert!((ratio - expected).abs() < 1e-4, "{ratio}");
    }

    /// The dimension tool takes what the selection's shape asks for, and
    /// never guesses an arc's length.
    #[test]
    fn the_dimension_tool_picks_what_the_selection_takes() {
        let (mut sketch, b, line, circle, arc) = sketch_with_an_arc();
        let Some(GeometryElement::Line(l)) = sketch.get_geometry(line) else {
            panic!("a line")
        };
        let a = l.start;
        let c = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 5.0))));
        let other = sketch.add_geometry(GeometryElement::Line(Line::new(a, c)));
        for (what, ids, tool) in [
            ("a circle", vec![circle], Some("radius_diameter")),
            ("an arc", vec![arc], Some("radius_diameter")),
            ("two lines", vec![line, other], Some("angle")),
            (
                "two lines and their corner",
                vec![a, line, other],
                Some("angle"),
            ),
            ("two points", vec![a, b], Some("distance")),
            ("a line and a circle", vec![line, circle], Some("distance")),
            ("a point and a circle", vec![b, circle], Some("distance")),
            ("nothing", vec![], None),
        ] {
            let shape = SelectionShape::of(&sketch, &ids.into_iter().collect());
            assert_eq!(dimension_for(&shape), tool, "{what}");
        }
    }
}
