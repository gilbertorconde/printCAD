//! Validation at the public semantic boundary.

use crate::problem::{
    ConstraintId, CurveReference, Geometry, ItemReference, PointReference, Problem, Relation,
};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum InputError {
    DuplicateGeometry(ItemReference),
    DuplicateConstraint(ConstraintId),
    MissingReference(ItemReference),
    InvalidRelation {
        constraint: ConstraintId,
        detail: String,
    },
    NonFiniteGeometry(ItemReference),
    InvalidSpline(ItemReference),
    InvalidSettings(String),
}

impl std::fmt::Display for InputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid solver input: {self:?}")
    }
}
impl std::error::Error for InputError {}

pub fn validate(problem: &Problem) -> Result<(), InputError> {
    let s = problem.settings;
    for (name, value) in [
        ("tolerance", s.tolerance),
        ("lambda_initial", s.lambda_initial),
        ("lambda_min", s.lambda_min),
        ("lambda_max", s.lambda_max),
        ("damping_floor", s.damping_floor),
        ("finite_difference_step", s.finite_difference_step),
        ("minimum_length", s.minimum_length.0),
        ("maximum_step_scale", s.maximum_step_scale),
        ("rank_tolerance", s.rank_tolerance),
    ] {
        if !value.is_finite() || value <= 0.0 {
            return Err(InputError::InvalidSettings(name.to_owned()));
        }
    }
    if s.max_iterations == 0
        || s.max_inner_retries == 0
        || s.lambda_min > s.lambda_initial
        || s.lambda_initial > s.lambda_max
    {
        return Err(InputError::InvalidSettings(
            "iteration or damping bounds".to_owned(),
        ));
    }
    let mut geometry_ids = HashSet::new();
    let mut constraint_ids = HashSet::new();
    for g in &problem.geometry {
        if !geometry_ids.insert(g.id()) {
            return Err(InputError::DuplicateGeometry(g.id()));
        }
        let finite = match g {
            Geometry::Point(p) => p.position.x.is_finite() && p.position.y.is_finite(),
            Geometry::Line(_) => true,
            Geometry::Circle(c) => c.radius.0.is_finite(),
            Geometry::Arc(a) => a.radius.0.is_finite(),
            Geometry::Ellipse(e) => {
                e.major.x.is_finite() && e.major.y.is_finite() && e.minor.0.is_finite()
            }
            Geometry::Conic(c) => {
                c.axis.x.is_finite() && c.axis.y.is_finite() && c.minor.0.is_finite()
            }
            Geometry::BSpline(b) => {
                let count = b.control_points.len();
                let degree = (b.degree as usize).min(count.saturating_sub(1));
                if b.degree == 0
                    || (!b.knots.is_empty()
                        && (b.periodic
                            || b.knots.len() != count + degree + 1
                            || b.knots.windows(2).any(|w| w[0] > w[1])))
                {
                    return Err(InputError::InvalidSpline(g.id()));
                }
                if crate::spline::Basis::new(b.degree, b.control_points.len(), &b.knots, b.periodic)
                    .is_none()
                    || (!b.weights.is_empty()
                        && (b.weights.len() != b.control_points.len()
                            || b.weights.iter().any(|w| !w.is_finite() || *w <= 0.0)))
                {
                    return Err(InputError::InvalidSpline(g.id()));
                }
                b.knots.iter().chain(&b.weights).all(|v| v.is_finite())
            }
        };
        if !finite {
            return Err(InputError::NonFiniteGeometry(g.id()));
        }
    }
    let exists = |id: ItemReference| {
        matches!(
            id,
            ItemReference::Point(PointReference::Origin)
                | ItemReference::Curve(CurveReference::XAxis | CurveReference::YAxis)
        ) || geometry_ids.contains(&id)
    };
    for g in &problem.geometry {
        for point in g.point_references() {
            if !exists(point.into()) {
                return Err(InputError::MissingReference(point.into()));
            }
        }
    }
    for id in problem.external.iter().copied().chain(
        problem
            .held_points
            .iter()
            .chain(&problem.application_held_points)
            .copied()
            .map(Into::into),
    ) {
        if !exists(id) {
            return Err(InputError::MissingReference(id));
        }
    }
    for c in &problem.constraints {
        if !constraint_ids.insert(c.id) {
            return Err(InputError::DuplicateConstraint(c.id));
        }
        validate_constraint(problem, c)?;
    }
    Ok(())
}

/// Validate references and the numerical domain of one relation against the
/// supplied geometry. Geometry and settings must be validated separately.
pub fn validate_constraint(
    problem: &Problem,
    constraint: &crate::problem::Constraint,
) -> Result<(), InputError> {
    for id in constraint.kind.references() {
        if !matches!(
            id,
            ItemReference::Point(PointReference::Origin)
                | ItemReference::Curve(CurveReference::XAxis | CurveReference::YAxis)
        ) && problem.get_geometry(id).is_none()
        {
            return Err(InputError::MissingReference(id));
        }
    }
    validate_relation(problem, constraint.id, &constraint.kind)
}

fn validate_relation(
    problem: &Problem,
    id: ConstraintId,
    relation: &Relation,
) -> Result<(), InputError> {
    let line = |id| {
        matches!(id, CurveReference::XAxis | CurveReference::YAxis)
            || matches!(problem.get_geometry(id), Some(Geometry::Line(_)))
    };
    let circle = |id| {
        matches!(
            problem.get_geometry(id),
            Some(Geometry::Circle(_) | Geometry::Arc(_))
        )
    };
    let ellipse = |id| matches!(problem.get_geometry(id), Some(Geometry::Ellipse(_)));
    let curve = |id| {
        matches!(id, CurveReference::XAxis | CurveReference::YAxis)
            || matches!(
                problem.get_geometry(id),
                Some(
                    Geometry::Line(_)
                        | Geometry::Circle(_)
                        | Geometry::Arc(_)
                        | Geometry::Ellipse(_)
                        | Geometry::Conic(_)
                        | Geometry::BSpline(_)
                )
            )
    };
    let tangent = |id| line(id) || circle(id);
    let arc = |id| matches!(problem.get_geometry(id), Some(Geometry::Arc(_)));
    let finite = |v: f64| v.is_finite();
    let valid = match relation {
        Relation::FixedPoint { position, .. } => finite(position.x) && finite(position.y),
        Relation::Coincident { .. }
        | Relation::HorizontalPoints { .. }
        | Relation::VerticalPoints { .. }
        | Relation::SymmetricAboutPoint { .. }
        | Relation::Block { .. } => true,
        Relation::Parallel { line1, line2 }
        | Relation::Perpendicular { line1, line2 }
        | Relation::EqualLength { line1, line2 } => line(*line1) && line(*line2),
        Relation::Length { line: l, length } => line(*l) && finite(length.0),
        Relation::EqualRadius { circle1, circle2 } => circle(*circle1) && circle(*circle2),
        Relation::Radius { circle: c, radius } => circle(*c) && finite(radius.0),
        Relation::Diameter {
            circle: c,
            diameter,
        } => circle(*c) && finite(diameter.0),
        Relation::PointOnLine { line: l, .. }
        | Relation::Midpoint { line: l, .. }
        | Relation::Symmetric { line: l, .. } => line(*l),
        Relation::PointOnCircle { circle: c, .. } => circle(*c),
        Relation::PointOnEllipse { ellipse: e, .. } => ellipse(*e),
        Relation::Horizontal { element } | Relation::Vertical { element } => line(*element),
        Relation::Distance { distance, .. } => finite(distance.0),
        Relation::Gap {
            item1,
            item2,
            distance,
        } => {
            let supports = match (*item1, *item2) {
                (ItemReference::Point(_), ItemReference::Curve(c))
                | (ItemReference::Curve(c), ItemReference::Point(_)) => line(c) || circle(c),
                (ItemReference::Curve(a), ItemReference::Curve(b)) => tangent(a) && tangent(b),
                (ItemReference::Point(_), ItemReference::Point(_)) => false,
            };
            supports && finite(distance.0)
        }
        Relation::DistanceX { value, .. } | Relation::DistanceY { value, .. } => finite(value.0),
        Relation::Angle {
            line1,
            line2,
            angle_rad,
        } => line(*line1) && line(*line2) && finite(angle_rad.0),
        Relation::AngleToAxis {
            line: l, angle_rad, ..
        } => line(*l) && finite(angle_rad.0),
        Relation::Tangent {
            line_or_circle1,
            item2,
        } => {
            tangent(*line_or_circle1)
                && tangent(*item2)
                && (circle(*line_or_circle1) || circle(*item2))
        }
        Relation::ArcLength { arc: a, length } => arc(*a) && finite(length.0),
        Relation::AngleAtPoint {
            curve1,
            curve2,
            angle_rad,
            ..
        } => tangent(*curve1) && tangent(*curve2) && finite(angle_rad.0),
        Relation::EllipseRadius {
            ellipse: e, radius, ..
        } => ellipse(*e) && finite(radius.0),
        Relation::CurveLength { curve: c, length } => curve(*c) && finite(length.0),
        Relation::PointOnCurve { curve: c, .. } => curve(*c),
        Relation::TangentCurves { curve1, curve2 }
        | Relation::PerpendicularCurves { curve1, curve2 } => curve(*curve1) && curve(*curve2),
        Relation::EqualEllipse { ellipse1, ellipse2 } => ellipse(*ellipse1) && ellipse(*ellipse2),
        Relation::Offset { pairs, distance } => {
            finite(distance.0)
                && pairs
                    .iter()
                    .all(|[a, b]| (line(*a) && line(*b)) || (circle(*a) && circle(*b)))
        }
        Relation::Pitch {
            direction,
            distance,
            columns,
            ..
        } => {
            *columns > 0
                && finite(distance.0)
                && direction.is_none_or(|d| {
                    finite(d.x)
                        && finite(d.y)
                        && (d.x.hypot(d.y) == 0.0 || (d.x.hypot(d.y) - 1.0).abs() < 1e-6)
                })
        }
        Relation::PolarPitch { angle_rad, .. } | Relation::AngleThreePoints { angle_rad, .. } => {
            finite(angle_rad.0)
        }
        Relation::ArcAngle { arc: a, angle_rad } => {
            arc(*a)
                && finite(angle_rad.0)
                && angle_rad.0 > 0.0
                && angle_rad.0 <= std::f64::consts::TAU
        }
        Relation::Refraction {
            ray1,
            ray2,
            interface,
            ratio,
            ..
        } => line(*ray1) && line(*ray2) && tangent(*interface) && finite(ratio.0),
        Relation::InternalAlignment {
            element,
            curve: c,
            role,
        } => {
            use crate::problem::InternalRole;
            match role {
                InternalRole::Focus1 | InternalRole::Focus2 => {
                    matches!(element, ItemReference::Point(_))
                        && matches!(
                            problem.get_geometry(*c),
                            Some(Geometry::Ellipse(_) | Geometry::Conic(_))
                        )
                }
                InternalRole::MajorAxis | InternalRole::MinorAxis => {
                    matches!(element,ItemReference::Curve(e) if line(*e))
                        && matches!(
                            problem.get_geometry(*c),
                            Some(Geometry::Ellipse(_) | Geometry::Conic(_))
                        )
                }
                InternalRole::ControlEdge(index) => {
                    matches!(element,ItemReference::Curve(e) if line(*e))
                        && matches!(problem.get_geometry(*c),Some(Geometry::BSpline(b)) if (index.0 as usize) < b.control_points.len().saturating_sub(usize::from(!b.periodic)))
                }
            }
        }
    };
    if valid {
        Ok(())
    } else {
        Err(InputError::InvalidRelation {
            constraint: id,
            detail: "wrong geometry kind or numerical domain".to_owned(),
        })
    }
}
