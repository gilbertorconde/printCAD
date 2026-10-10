//! Storage precision, references and application pinning at the solver boundary.

use crate::sketch::{self, ConstraintKind as C, GeometryElement as G, Sketch, Vec2D};
use sketch_solver::problem as core;
use uuid::Uuid;

pub(super) fn point(id: Uuid) -> core::PointReference {
    if id == sketch::ORIGIN_ID {
        core::PointReference::Origin
    } else {
        core::PointId(id.as_u128()).into()
    }
}

pub(super) fn curve(id: Uuid) -> core::CurveReference {
    match id {
        sketch::X_AXIS_ID => core::CurveReference::XAxis,
        sketch::Y_AXIS_ID => core::CurveReference::YAxis,
        _ => core::CurveId(id.as_u128()).into(),
    }
}

fn item(sketch: &Sketch, id: Uuid) -> core::ItemReference {
    if id == sketch::ORIGIN_ID || matches!(sketch.get_geometry(id), Some(G::Point(_))) {
        point(id).into()
    } else {
        curve(id).into()
    }
}

fn vector(v: Vec2D) -> core::Vector {
    core::Vector {
        x: f64::from(v.x),
        y: f64::from(v.y),
    }
}

fn length(v: f32) -> core::Length {
    core::Length(f64::from(v))
}
fn radians(v: f32) -> core::Radians {
    core::Radians(f64::from(v))
}

pub(super) fn settings(settings: sketch::SolverSettings) -> core::Settings {
    core::Settings {
        max_iterations: settings.max_iterations.max(1),
        tolerance: if settings.tolerance > 0.0 {
            settings.tolerance
        } else {
            1e-9
        },
        ..core::Settings::default()
    }
}

fn relation(sketch: &Sketch, kind: &C) -> core::Relation {
    use core::Relation as R;
    match *kind {
        C::FixedPoint { point: p, position } => R::FixedPoint {
            point: point(p),
            position: vector(position),
        },
        C::Coincident { point1, point2 } => R::Coincident {
            point1: point(point1),
            point2: point(point2),
        },
        C::Parallel { line1, line2 } => R::Parallel {
            line1: curve(line1),
            line2: curve(line2),
        },
        C::Perpendicular { line1, line2 } => R::Perpendicular {
            line1: curve(line1),
            line2: curve(line2),
        },
        C::EqualLength { line1, line2 } => R::EqualLength {
            line1: curve(line1),
            line2: curve(line2),
        },
        C::Length { line, length: v } => R::Length {
            line: curve(line),
            length: length(v),
        },
        C::EqualRadius { circle1, circle2 } => R::EqualRadius {
            circle1: curve(circle1),
            circle2: curve(circle2),
        },
        C::Radius { circle, radius } => R::Radius {
            circle: curve(circle),
            radius: length(radius),
        },
        C::Diameter { circle, diameter } => R::Diameter {
            circle: curve(circle),
            diameter: length(diameter),
        },
        C::PointOnLine { point: p, line } => R::PointOnLine {
            point: point(p),
            line: curve(line),
        },
        C::PointOnCircle { point: p, circle } => R::PointOnCircle {
            point: point(p),
            circle: curve(circle),
        },
        C::PointOnEllipse { point: p, ellipse } => R::PointOnEllipse {
            point: point(p),
            ellipse: curve(ellipse),
        },
        C::Horizontal { element } => R::Horizontal {
            element: curve(element),
        },
        C::Vertical { element } => R::Vertical {
            element: curve(element),
        },
        C::HorizontalPoints { point1, point2 } => R::HorizontalPoints {
            point1: point(point1),
            point2: point(point2),
        },
        C::VerticalPoints { point1, point2 } => R::VerticalPoints {
            point1: point(point1),
            point2: point(point2),
        },
        C::Block { element } => R::Block {
            element: item(sketch, element),
        },
        C::Distance {
            point1,
            point2,
            distance,
        } => R::Distance {
            point1: point(point1),
            point2: point(point2),
            distance: length(distance),
        },
        C::DistanceX { a, b, value } => R::DistanceX {
            a: point(a),
            b: b.map(point),
            value: length(value),
        },
        C::DistanceY { a, b, value } => R::DistanceY {
            a: point(a),
            b: b.map(point),
            value: length(value),
        },
        C::Angle {
            line1,
            line2,
            angle_rad,
        } => R::Angle {
            line1: curve(line1),
            line2: curve(line2),
            angle_rad: radians(angle_rad),
        },
        C::AngleToAxis {
            line,
            axis,
            angle_rad,
        } => R::AngleToAxis {
            line: curve(line),
            axis: match axis {
                sketch::AxisDirection::Horizontal => core::AxisDirection::Horizontal,
                sketch::AxisDirection::Vertical => core::AxisDirection::Vertical,
            },
            angle_rad: radians(angle_rad),
        },
        C::Tangent {
            line_or_circle1,
            item2,
        } => R::Tangent {
            line_or_circle1: curve(line_or_circle1),
            item2: curve(item2),
        },
        C::Symmetric {
            point1,
            point2,
            line,
        } => R::Symmetric {
            point1: point(point1),
            point2: point(point2),
            line: curve(line),
        },
        C::SymmetricAboutPoint {
            point1,
            point2,
            center,
        } => R::SymmetricAboutPoint {
            point1: point(point1),
            point2: point(point2),
            center: point(center),
        },
        C::Midpoint { point: p, line } => R::Midpoint {
            point: point(p),
            line: curve(line),
        },
        C::ArcLength { arc, length: v } => R::ArcLength {
            arc: curve(arc),
            length: length(v),
        },
        C::Gap {
            item1,
            item2,
            distance,
        } => R::Gap {
            item1: item(sketch, item1),
            item2: item(sketch, item2),
            distance: length(distance),
        },
        C::AngleAtPoint {
            curve1,
            curve2,
            point: p,
            angle_rad,
        } => R::AngleAtPoint {
            curve1: curve(curve1),
            curve2: curve(curve2),
            point: point(p),
            angle_rad: radians(angle_rad),
        },
        C::EllipseRadius {
            ellipse,
            major,
            radius,
        } => R::EllipseRadius {
            ellipse: curve(ellipse),
            major,
            radius: length(radius),
        },
        C::CurveLength {
            curve: c,
            length: v,
        } => R::CurveLength {
            curve: curve(c),
            length: length(v),
        },
        C::PointOnCurve { point: p, curve: c } => R::PointOnCurve {
            point: point(p),
            curve: curve(c),
        },
        C::TangentCurves { curve1, curve2 } => R::TangentCurves {
            curve1: curve(curve1),
            curve2: curve(curve2),
        },
        C::PerpendicularCurves { curve1, curve2 } => R::PerpendicularCurves {
            curve1: curve(curve1),
            curve2: curve(curve2),
        },
        C::EqualEllipse { ellipse1, ellipse2 } => R::EqualEllipse {
            ellipse1: curve(ellipse1),
            ellipse2: curve(ellipse2),
        },
        C::Offset {
            ref pairs,
            distance,
        } => R::Offset {
            pairs: pairs.iter().map(|[a, b]| [curve(*a), curve(*b)]).collect(),
            distance: length(distance),
        },
        C::Pitch {
            ref points,
            columns,
            distance,
            across,
            direction,
        } => R::Pitch {
            points: points.iter().copied().map(point).collect(),
            columns,
            distance: length(distance),
            across,
            direction: direction.map(|v| {
                let v = v.to_glam().normalize_or_zero();
                core::Direction {
                    x: v.x as f64,
                    y: v.y as f64,
                }
            }),
        },
        C::PolarPitch {
            center,
            ref points,
            angle_rad,
        } => R::PolarPitch {
            center: point(center),
            points: points.iter().copied().map(point).collect(),
            angle_rad: radians(angle_rad),
        },
        C::ArcAngle { arc, angle_rad } => R::ArcAngle {
            arc: curve(arc),
            angle_rad: radians(angle_rad),
        },
        C::AngleThreePoints {
            point1,
            vertex,
            point2,
            angle_rad,
        } => R::AngleThreePoints {
            point1: point(point1),
            vertex: point(vertex),
            point2: point(point2),
            angle_rad: radians(angle_rad),
        },
        C::Refraction {
            ray1,
            ray2,
            interface,
            point: p,
            ratio,
        } => R::Refraction {
            ray1: curve(ray1),
            ray2: curve(ray2),
            interface: curve(interface),
            point: point(p),
            ratio: core::Ratio(f64::from(ratio)),
        },
        C::InternalAlignment {
            element,
            curve: c,
            role,
        } => R::InternalAlignment {
            element: item(sketch, element),
            curve: curve(c),
            role: match role {
                sketch::InternalRole::MajorAxis => core::InternalRole::MajorAxis,
                sketch::InternalRole::MinorAxis => core::InternalRole::MinorAxis,
                sketch::InternalRole::Focus1 => core::InternalRole::Focus1,
                sketch::InternalRole::Focus2 => core::InternalRole::Focus2,
                sketch::InternalRole::ControlEdge(n) => {
                    core::InternalRole::ControlEdge(core::ControlEdgeIndex(n))
                }
            },
        },
    }
}

pub(super) fn prepare(sketch: &Sketch, held: &[Uuid]) -> core::Problem {
    let geometry = sketch
        .geometry
        .iter()
        .map(|g| match g {
            G::Point(p) => core::Geometry::Point(core::Point {
                id: core::PointId(p.id.as_u128()),
                position: vector(p.position),
            }),
            G::Line(l) => core::Geometry::Line(core::Line {
                id: core::CurveId(l.id.as_u128()),
                start: point(l.start),
                end: point(l.end),
            }),
            G::Circle(c) => core::Geometry::Circle(core::Circle {
                id: core::CurveId(c.id.as_u128()),
                center: point(c.center),
                radius: length(c.radius),
            }),
            G::Arc(a) => core::Geometry::Arc(core::Arc {
                id: core::CurveId(a.id.as_u128()),
                center: point(a.center),
                start: point(a.start),
                end: point(a.end),
                radius: length(a.radius),
            }),
            G::Ellipse(e) => core::Geometry::Ellipse(core::Ellipse {
                id: core::CurveId(e.id.as_u128()),
                center: point(e.center),
                major: vector(e.major),
                minor: length(e.major.to_glam().length() * e.ratio),
                arc: e.arc.map(|a| core::ArcEnds {
                    start: point(a.start),
                    end: point(a.end),
                }),
            }),
            G::Conic(c) => core::Geometry::Conic(core::Conic {
                id: core::CurveId(c.id.as_u128()),
                kind: match c.kind {
                    sketch::ConicKind::Hyperbola => core::ConicKind::Hyperbola,
                    sketch::ConicKind::Parabola => core::ConicKind::Parabola,
                },
                center: point(c.center),
                axis: vector(c.axis),
                minor: length(c.minor),
                start: point(c.start),
                end: point(c.end),
            }),
            G::BSpline(b) => core::Geometry::BSpline(core::BSpline {
                id: core::CurveId(b.id.as_u128()),
                degree: b.degree,
                control_points: b.control_points.iter().copied().map(point).collect(),
                knots: b.knots.clone(),
                periodic: b.periodic,
                weights: b.weights.clone(),
                fit_points: b.fit_points.iter().copied().map(point).collect(),
            }),
        })
        .collect();
    let mut problem = core::Problem {
        geometry,
        constraints: sketch
            .constraints
            .iter()
            .filter(|c| c.is_solved())
            .map(|c| core::Constraint {
                id: core::ConstraintId(c.id.as_u128()),
                kind: relation(sketch, &c.kind),
            })
            .collect(),
        external: sketch
            .external_ids()
            .into_iter()
            .map(|id| item(sketch, id))
            .collect(),
        held_points: held.iter().copied().map(point).collect(),
        application_held_points: crate::spline::derived_points(sketch)
            .into_iter()
            .chain(crate::text::outline_points(sketch))
            .map(point)
            .collect(),
        settings: settings(sketch.solver),
    };
    problem.external.sort();
    problem.application_held_points.sort();
    problem
}
