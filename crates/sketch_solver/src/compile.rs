use crate::curves::{CurveVars, nearest_param, nearest_params};
use crate::problem::{
    Geometry as GeometryElement, Problem as Sketch, Relation as ConstraintKind, *,
};
use crate::residual::{CurveShape, Offset, ResidualSpec, Tangent, segment_length};
use crate::trace::EquationSource;
use std::collections::{HashMap, HashSet};

pub fn compile(problem: &Problem) -> Result<System, crate::input::InputError> {
    crate::input::validate(problem)?;
    Ok(compile_system(problem, None, false))
}

fn shared_end(sketch: &Sketch, a: CurveReference, b: CurveReference) -> Option<PointReference> {
    let ends = |id| match sketch.get_geometry(id)? {
        GeometryElement::Line(l) => Some([l.start, l.end]),
        GeometryElement::Arc(a) => Some([a.start, a.end]),
        _ => None,
    };
    let a = ends(a)?;
    let b = ends(b)?;
    a.into_iter().find(|p| b.contains(p))
}

/// The constraint system: free variables plus resolved residual specs.
pub struct System {
    pub origins: Vec<EquationSource>,
    pub settings: Settings,
    /// Initial values for every variable, free or pinned.
    pub vars: Vec<f64>,
    /// Indices the solver may move. Reference geometry (the origin and the
    /// axes) sits in `vars` so residuals can address it, but stays out of
    /// here: it holds still and costs no degree of freedom.
    pub free: Vec<usize>,
    /// Point id -> index of its x variable (y is at index + 1).
    pub point_vars: HashMap<PointReference, usize>,
    /// Circle/arc id -> index of its radius variable.
    pub radius_vars: HashMap<CurveReference, usize>,
    /// Ellipse or conic id -> index of its three shape variables (see
    /// `CurveShape`), for the curves whose internal geometry is shown.
    pub shape_vars: HashMap<CurveReference, usize>,
    /// Ellipse foci nothing but their curve holds: left out of the solve
    /// and placed from the solved shape after it.
    pub derived_foci: HashSet<PointReference>,
    /// Resolved residuals (user constraints plus implicit arc consistency).
    pub specs: Vec<ResidualSpec>,
    /// Total residual dimension.
    pub residual_len: usize,
}

impl System {
    pub fn point_variable(&self, point: PointReference) -> Option<VariableIndex> {
        self.point_vars.get(&point).copied().map(VariableIndex)
    }
    pub fn radius_variable(&self, curve: CurveReference) -> Option<VariableIndex> {
        self.radius_vars.get(&curve).copied().map(VariableIndex)
    }
    pub fn shape_variable(&self, curve: CurveReference) -> Option<VariableIndex> {
        self.shape_vars.get(&curve).copied().map(VariableIndex)
    }
    pub fn equation_indices(&self) -> impl Iterator<Item = EquationIndex> {
        (0..self.residual_len).map(EquationIndex)
    }
    pub fn variable_indices(&self) -> impl Iterator<Item = VariableIndex> + '_ {
        self.free.iter().copied().map(VariableIndex)
    }
}

/// Compile an ordered problem, optionally excluding a diagnosis probe's
/// constraint and including otherwise unconstrained arc endpoints for rank.
#[doc(hidden)]
pub fn compile_system(sketch: &Sketch, exclude: Option<ConstraintId>, arcs_always: bool) -> System {
    let mut vars = Vec::new();
    let mut point_vars = HashMap::new();
    let mut radius_vars = HashMap::new();
    // The origin and a tip along each axis, pinned where they belong. They
    // only join the system when a constraint points at them, and they bring
    // one residual per variable, so they cost no degree of freedom.
    let references_used = sketch.constraints.iter().any(|c| {
        c.kind.references().iter().any(|id| {
            matches!(
                id,
                ItemReference::Point(PointReference::Origin)
                    | ItemReference::Curve(CurveReference::XAxis | CurveReference::YAxis)
            )
        })
    });
    let mut pinned: Vec<usize> = Vec::new();
    let mut reference_var = |vars: &mut Vec<f64>, x: f64, y: f64| {
        let at = vars.len();
        vars.push(x);
        vars.push(y);
        pinned.push(at);
        pinned.push(at + 1);
        at
    };
    let (origin_var, x_tip_var, y_tip_var) = if references_used {
        (
            Some(reference_var(&mut vars, 0.0, 0.0)),
            Some(reference_var(&mut vars, 1.0, 0.0)),
            Some(reference_var(&mut vars, 0.0, 1.0)),
        )
    } else {
        (None, None, None)
    };
    // An ellipse or conic showing internal geometry has its shape in the
    // system, so the geometry can size and turn it.
    let shaped: HashSet<CurveReference> = sketch
        .constraints
        .iter()
        .filter(|c| exclude != Some(c.id))
        .flat_map(|c| match c.kind {
            ConstraintKind::InternalAlignment { curve, .. } => vec![curve],
            // Equal ellipses size each other, so their shapes are free.
            ConstraintKind::EqualEllipse { ellipse1, ellipse2 } => vec![ellipse1, ellipse2],
            ConstraintKind::EllipseRadius { ellipse, .. } => vec![ellipse],
            _ => Vec::new(),
        })
        .collect();
    let mut shape_vars = HashMap::new();
    for element in &sketch.geometry {
        match element {
            GeometryElement::Ellipse(e) if shaped.contains(&e.id.into()) => {
                shape_vars.insert(e.id.into(), vars.len());
                vars.push(e.major.x);
                vars.push(e.major.y);
                vars.push(f64::from(e.minor));
            }
            GeometryElement::Conic(c) if shaped.contains(&c.id.into()) => {
                shape_vars.insert(c.id.into(), vars.len());
                vars.push(c.axis.x);
                vars.push(c.axis.y);
                vars.push(f64::from(c.minor));
                // A parabola has no minor axis: its slot holds still.
                if c.kind == ConicKind::Parabola {
                    pinned.push(vars.len() - 1);
                }
            }
            _ => {}
        }
    }
    for element in &sketch.geometry {
        match element {
            GeometryElement::Point(p) => {
                point_vars.insert(p.id.into(), vars.len());
                vars.push(p.position.x);
                vars.push(p.position.y);
            }
            GeometryElement::Circle(c) => {
                radius_vars.insert(c.id.into(), vars.len());
                vars.push(f64::from(c.radius));
            }
            GeometryElement::Arc(a) => {
                radius_vars.insert(a.id.into(), vars.len());
                vars.push(f64::from(a.radius));
            }
            // Lines and splines move with their points; an ellipse's or
            // conic's shape joins above when something sizes it.
            GeometryElement::Line(_)
            | GeometryElement::Ellipse(_)
            | GeometryElement::BSpline(_)
            | GeometryElement::Conic(_) => {}
        }
    }
    // External geometry is where the solid's edge put it: its points and
    // radii are held still, as the origin is.
    for id in &sketch.external {
        match id {
            ItemReference::Point(id) => {
                if let Some(&v) = point_vars.get(id) {
                    pinned.extend([v, v + 1]);
                }
            }
            ItemReference::Curve(id) => {
                if let Some(&r) = radius_vars.get(id) {
                    pinned.push(r);
                }
                if let Some(&k) = shape_vars.get(id) {
                    pinned.extend([k, k + 1, k + 2]);
                }
            }
        }
    }
    for id in &sketch.held_points {
        if let Some(&v) = point_vars.get(id) {
            pinned.extend([v, v + 1]);
        }
    }
    // A spline drawn through points has its control points worked out from
    // them after the solve: the solver leaves them be.
    // Fit-spline controls and text outlines are worked out by the caller
    // after the solve; the compiler receives their held IDs explicitly.
    for id in &sketch.application_held_points {
        if let Some(&v) = point_vars.get(id) {
            pinned.extend([v, v + 1]);
        }
    }
    // A focus only its ellipse holds follows the shape after the solve,
    // so moving it costs the solve nothing: a radius dragged past the
    // other trades the axes as it does with the foci hidden.
    let derived_foci = derived_foci(sketch, exclude, &sketch.held_points);
    for id in &derived_foci {
        if let Some(&v) = point_vars.get(id) {
            pinned.extend([v, v + 1]);
        }
    }

    let point_var = |id: PointReference| {
        if id == PointReference::Origin {
            return origin_var;
        }
        point_vars.get(&id).copied()
    };
    // Line -> (start x-var, end x-var), only when both endpoints are points.
    // The axes run from the origin through their tip.
    let line_vars = |id: CurveReference| match id {
        CurveReference::XAxis => Some((origin_var?, x_tip_var?)),
        CurveReference::YAxis => Some((origin_var?, y_tip_var?)),
        _ => match sketch.get_geometry(id) {
            Some(GeometryElement::Line(l)) => Some((point_var(l.start)?, point_var(l.end)?)),
            _ => None,
        },
    };
    // Circle or arc -> (center x-var, radius var).
    let circle_vars = |id: CurveReference| match sketch.get_geometry(id) {
        Some(GeometryElement::Circle(c)) => {
            Some((point_var(c.center)?, radius_vars.get(&id).copied()?))
        }
        Some(GeometryElement::Arc(a)) => {
            Some((point_var(a.center)?, radius_vars.get(&id).copied()?))
        }
        _ => None,
    };
    let radius_var = |id: CurveReference| circle_vars(id).map(|(_, r)| r);
    // Any item a gap, angle or refraction reaches: a point, a line or a
    // circle (an arc counting as its circle).
    let item_vars = |id: ItemReference| match id {
        ItemReference::Point(id) => point_var(id).map(ItemVars::Point),
        ItemReference::Curve(id) => {
            if let Some((s, e)) = line_vars(id) {
                Some(ItemVars::Line(s, e))
            } else {
                circle_vars(id).map(|(c, r)| ItemVars::Circle(c, r))
            }
        }
    };
    let tangent_of = |id: CurveReference| match item_vars(id.into())? {
        ItemVars::Line(s, e) => Some(Tangent::Line { s, e }),
        ItemVars::Circle(c, _) => Some(Tangent::Circle { c }),
        ItemVars::Point(_) => None,
    };
    // A ray's (near, far) ends: the one nearer `p` at solve start first.
    let ray_ends = |id: CurveReference, p: usize| {
        let (s, e) = line_vars(id)?;
        Some(
            if segment_length(&vars, s, p) <= segment_length(&vars, e, p) {
                (s, e)
            } else {
                (e, s)
            },
        )
    };

    // Any curve, to be read at a parameter.
    let curve_of = |id: CurveReference| -> Option<CurveVars> {
        if let Some((s, e)) = line_vars(id) {
            return Some(CurveVars::Line { s, e });
        }
        if let Some((c, r)) = circle_vars(id) {
            return Some(CurveVars::Circle { c, r });
        }
        match sketch.get_geometry(id)? {
            GeometryElement::Ellipse(e) => Some(CurveVars::Ellipse {
                c: point_var(e.center)?,
                shape: curve_shape(sketch, &shape_vars, e.id.into()),
            }),
            GeometryElement::Conic(k) => {
                let c = point_var(k.center)?;
                let shape = curve_shape(sketch, &shape_vars, k.id.into());
                Some(match k.kind {
                    ConicKind::Hyperbola => CurveVars::Hyperbola { c, shape },
                    ConicKind::Parabola => CurveVars::Parabola { c, shape },
                })
            }
            GeometryElement::BSpline(b) => Some(CurveVars::Spline {
                basis: crate::spline::Basis::new(
                    b.degree,
                    b.control_points.len(),
                    &b.knots,
                    b.periodic,
                )?
                .with_weights(&b.weights),
                control: b
                    .control_points
                    .iter()
                    .map(|p| point_var(*p))
                    .collect::<Option<Vec<_>>>()?,
            }),
            _ => None,
        }
    };
    // The parameters a constraint on a curve adds, past the geometry's
    // variables: each where the curve is nearest now.
    let base = vars.len();
    let mut aux: Vec<f64> = Vec::new();

    let mut specs = Vec::new();
    let mut origins = Vec::new();
    for constraint in &sketch.constraints {
        if exclude == Some(constraint.id) {
            continue;
        }
        let start = specs.len();
        match constraint.kind {
            ConstraintKind::EllipseRadius {
                ellipse,
                major,
                radius,
            } => {
                if let Some(&k) = shape_vars.get(&ellipse) {
                    specs.push(ResidualSpec::EllipseRadius {
                        k,
                        major,
                        radius: f64::from(radius),
                    });
                }
            }
            ConstraintKind::CurveLength { curve: id, length } => {
                if let Some(curve) = curve_of(id) {
                    let ends = match sketch.get_geometry(id) {
                        Some(GeometryElement::Conic(k)) => point_var(k.start).zip(point_var(k.end)),
                        _ => None,
                    };
                    specs.push(ResidualSpec::CurveLength {
                        curve,
                        ends,
                        length: f64::from(length),
                    });
                }
            }
            ConstraintKind::PointOnCurve { point, curve } => {
                if let (Some(p), Some(curve)) = (point_var(point), curve_of(curve)) {
                    let t0 = nearest_param(
                        &curve,
                        &vars,
                        [vars[p], vars[p + 1]],
                        sketch.settings.minimum_length.0,
                    );
                    let t = base + aux.len();
                    aux.push(t0);
                    specs.push(ResidualSpec::PointOnCurve { p, curve, t });
                }
            }
            ConstraintKind::TangentCurves { curve1, curve2 }
            | ConstraintKind::PerpendicularCurves { curve1, curve2 } => {
                if let (Some(a), Some(b)) = (curve_of(curve1), curve_of(curve2)) {
                    let (t1, t2) = nearest_params(&a, &b, &vars, sketch.settings.minimum_length.0);
                    let ta = base + aux.len();
                    aux.extend([t1, t2]);
                    specs.push(ResidualSpec::CurvesMeet {
                        a,
                        ta,
                        b,
                        tb: ta + 1,
                        square: matches!(
                            constraint.kind,
                            ConstraintKind::PerpendicularCurves { .. }
                        ),
                    });
                }
            }
            ConstraintKind::FixedPoint { point, position } => {
                if let Some(p) = point_var(point) {
                    specs.push(ResidualSpec::FixedPoint {
                        p,
                        x: position.x,
                        y: position.y,
                    });
                }
            }
            ConstraintKind::Coincident { point1, point2 } => {
                if let (Some(p1), Some(p2)) = (point_var(point1), point_var(point2)) {
                    specs.push(ResidualSpec::Coincident { p1, p2 });
                }
            }
            ConstraintKind::Parallel { line1, line2 } => {
                if let (Some((s1, e1)), Some((s2, e2))) = (line_vars(line1), line_vars(line2)) {
                    specs.push(ResidualSpec::Parallel { s1, e1, s2, e2 });
                }
            }
            ConstraintKind::Perpendicular { line1, line2 } => {
                if let (Some((s1, e1)), Some((s2, e2))) = (line_vars(line1), line_vars(line2)) {
                    specs.push(ResidualSpec::Perpendicular { s1, e1, s2, e2 });
                }
            }
            ConstraintKind::EqualLength { line1, line2 } => {
                if let (Some((s1, e1)), Some((s2, e2))) = (line_vars(line1), line_vars(line2)) {
                    specs.push(ResidualSpec::EqualLength { s1, e1, s2, e2 });
                }
            }
            ConstraintKind::Length { line, length } => {
                if let Some((s, e)) = line_vars(line) {
                    specs.push(ResidualSpec::Length {
                        s,
                        e,
                        len: f64::from(length),
                    });
                }
            }
            ConstraintKind::EqualRadius { circle1, circle2 } => {
                if let (Some(r1), Some(r2)) = (radius_var(circle1), radius_var(circle2)) {
                    specs.push(ResidualSpec::EqualRadius { r1, r2 });
                }
            }
            ConstraintKind::Radius { circle, radius } => {
                if let Some(r) = radius_var(circle) {
                    specs.push(ResidualSpec::Radius {
                        r,
                        radius: f64::from(radius),
                    });
                }
            }
            ConstraintKind::Diameter { circle, diameter } => {
                if let Some(r) = radius_var(circle) {
                    specs.push(ResidualSpec::Diameter {
                        r,
                        diameter: f64::from(diameter),
                    });
                }
            }
            ConstraintKind::PointOnLine { point, line } => {
                if let (Some(p), Some((s, e))) = (point_var(point), line_vars(line)) {
                    specs.push(ResidualSpec::PointOnLine { p, s, e });
                }
            }
            ConstraintKind::PointOnCircle { point, circle } => {
                if let (Some(p), Some((c, r))) = (point_var(point), circle_vars(circle)) {
                    specs.push(ResidualSpec::PointOnCircle { p, c, r });
                }
            }
            ConstraintKind::Horizontal { element } => {
                if let Some((s, e)) = line_vars(element) {
                    specs.push(ResidualSpec::Horizontal { s, e });
                }
            }
            ConstraintKind::Vertical { element } => {
                if let Some((s, e)) = line_vars(element) {
                    specs.push(ResidualSpec::Vertical { s, e });
                }
            }
            // Two points level or plumb: the line between them is.
            ConstraintKind::HorizontalPoints { point1, point2 } => {
                if let (Some(s), Some(e)) = (point_var(point1), point_var(point2)) {
                    specs.push(ResidualSpec::Horizontal { s, e });
                }
            }
            ConstraintKind::VerticalPoints { point1, point2 } => {
                if let (Some(s), Some(e)) = (point_var(point1), point_var(point2)) {
                    specs.push(ResidualSpec::Vertical { s, e });
                }
            }
            ConstraintKind::Block { element } => {
                if let Some(geom) = sketch.get_geometry(element) {
                    // Fix every referenced point (and the element itself,
                    // for standalone points) at its current position, plus
                    // the radius when the element has one.
                    let mut ids = geom.point_references();
                    if let GeometryElement::Point(p) = geom {
                        ids.push(p.id.into());
                    }
                    for pid in ids {
                        if let (Some(p), Some(pos)) = (point_var(pid), sketch.point_position(pid)) {
                            specs.push(ResidualSpec::FixedPoint {
                                p,
                                x: pos.x,
                                y: pos.y,
                            });
                        }
                    }
                    if let ItemReference::Curve(curve) = element
                        && let Some(&r) = radius_vars.get(&curve)
                    {
                        specs.push(ResidualSpec::Radius { r, radius: vars[r] });
                    }
                }
            }
            ConstraintKind::Distance {
                point1,
                point2,
                distance,
            } => {
                if let (Some(p1), Some(p2)) = (point_var(point1), point_var(point2)) {
                    specs.push(ResidualSpec::Distance {
                        p1,
                        p2,
                        d: f64::from(distance),
                    });
                }
            }
            ConstraintKind::DistanceX { a, b, value } => {
                if let (Some(pa), Some(pb)) = (point_var(a), resolve_opt(b, &point_var)) {
                    specs.push(ResidualSpec::CoordDistance {
                        a: pa,
                        b: pb,
                        value: f64::from(value),
                    });
                }
            }
            ConstraintKind::DistanceY { a, b, value } => {
                if let (Some(pa), Some(pb)) = (point_var(a), resolve_opt(b, &point_var)) {
                    specs.push(ResidualSpec::CoordDistance {
                        a: pa + 1,
                        b: pb.map(|i| i + 1),
                        value: f64::from(value),
                    });
                }
            }
            ConstraintKind::Angle {
                line1,
                line2,
                angle_rad,
            } => {
                if let (Some((s1, e1)), Some((s2, e2))) = (line_vars(line1), line_vars(line2)) {
                    specs.push(ResidualSpec::Angle {
                        s1,
                        e1,
                        s2,
                        e2,
                        angle: f64::from(angle_rad),
                    });
                }
            }
            ConstraintKind::AngleToAxis {
                line,
                axis,
                angle_rad,
            } => {
                if let Some((s, e)) = line_vars(line) {
                    let base = match axis {
                        AxisDirection::Horizontal => 0.0,
                        AxisDirection::Vertical => std::f64::consts::FRAC_PI_2,
                    };
                    specs.push(ResidualSpec::AngleToTarget {
                        s,
                        e,
                        target: base + f64::from(angle_rad),
                    });
                }
            }
            ConstraintKind::PointOnEllipse { point, ellipse } => {
                if let (Some(p), Some(GeometryElement::Ellipse(el))) =
                    (point_var(point), sketch.get_geometry(ellipse))
                    && let Some(c) = point_var(el.center)
                {
                    specs.push(ResidualSpec::PointOnEllipse {
                        p,
                        c,
                        shape: curve_shape(sketch, &shape_vars, ellipse),
                    });
                }
            }
            // Resolved together below, once each point is known.
            ConstraintKind::InternalAlignment { .. } => {}
            ConstraintKind::Tangent {
                line_or_circle1,
                item2,
            } => {
                // Joined end to end, the tangency is at the shared end:
                // written there it stays independent of the ends lying on
                // both curves, where the distance form would not be.
                let at_end = shared_end(sketch, line_or_circle1, item2);
                let resolved = match (
                    line_vars(line_or_circle1),
                    circle_vars(line_or_circle1),
                    line_vars(item2),
                    circle_vars(item2),
                ) {
                    (Some((s, e)), _, _, Some((c, _))) | (_, Some((c, _)), Some((s, e)), _)
                        if at_end.and_then(&point_var).is_some() =>
                    {
                        let p = at_end.and_then(&point_var).expect("checked");
                        Some(ResidualSpec::TangentAtEnd { p, c, s, e })
                    }
                    (_, Some((c1, _)), _, Some((c2, _)))
                        if at_end.and_then(&point_var).is_some() =>
                    {
                        let p = at_end.and_then(&point_var).expect("checked");
                        Some(ResidualSpec::TangentArcsAtEnd { p, c1, c2 })
                    }
                    // Line ↔ circle/arc, in either selection order.
                    (Some((s, e)), _, _, Some((c, r))) | (_, Some((c, r)), Some((s, e)), _) => {
                        Some(ResidualSpec::TangentLineCircle { s, e, c, r })
                    }
                    // Circle/arc ↔ circle/arc: choose the external or
                    // internal branch ONCE, from the configuration at solve
                    // start (build_system runs once per solve call).
                    (_, Some((c1, r1)), _, Some((c2, r2))) => {
                        let dist = segment_length(&vars, c1, c2);
                        let external_err = (dist - (vars[r1] + vars[r2])).abs();
                        let internal_err = (dist - (vars[r1] - vars[r2]).abs()).abs();
                        Some(ResidualSpec::TangentCircles {
                            c1,
                            r1,
                            c2,
                            r2,
                            internal: internal_err < external_err,
                        })
                    }
                    _ => None,
                };
                if let Some(spec) = resolved {
                    specs.push(spec);
                }
            }
            ConstraintKind::Symmetric {
                point1,
                point2,
                line,
            } => {
                if let (Some(p1), Some(p2), Some((s, e))) =
                    (point_var(point1), point_var(point2), line_vars(line))
                {
                    specs.push(ResidualSpec::Symmetric { p1, p2, s, e });
                }
            }
            ConstraintKind::SymmetricAboutPoint {
                point1,
                point2,
                center,
            } => {
                // center = (p1 + p2) / 2: exactly the Midpoint residual.
                if let (Some(s), Some(e), Some(p)) =
                    (point_var(point1), point_var(point2), point_var(center))
                {
                    specs.push(ResidualSpec::Midpoint { p, s, e });
                }
            }
            ConstraintKind::Midpoint { point, line } => {
                if let (Some(p), Some((s, e))) = (point_var(point), line_vars(line)) {
                    specs.push(ResidualSpec::Midpoint { p, s, e });
                }
            }
            ConstraintKind::ArcLength { arc, length } => {
                if let Some(GeometryElement::Arc(a)) = sketch.get_geometry(arc)
                    && let (Some(c), Some(s), Some(e), Some(r)) = (
                        point_var(a.center),
                        point_var(a.start),
                        point_var(a.end),
                        radius_vars.get(&arc).copied(),
                    )
                {
                    specs.push(ResidualSpec::ArcLength {
                        c,
                        s,
                        e,
                        r,
                        len: f64::from(length),
                    });
                }
            }
            ConstraintKind::EqualEllipse { ellipse1, ellipse2 } => {
                if let (Some(&a), Some(&b)) = (shape_vars.get(&ellipse1), shape_vars.get(&ellipse2))
                {
                    specs.push(ResidualSpec::EqualEllipse { a, b });
                }
            }
            ConstraintKind::ArcAngle { arc, angle_rad } => {
                if let Some(GeometryElement::Arc(a)) = sketch.get_geometry(arc)
                    && let (Some(c), Some(s), Some(e)) =
                        (point_var(a.center), point_var(a.start), point_var(a.end))
                {
                    specs.push(ResidualSpec::ArcAngle {
                        c,
                        s,
                        e,
                        angle: f64::from(angle_rad),
                    });
                }
            }
            ConstraintKind::AngleThreePoints {
                point1,
                vertex,
                point2,
                angle_rad,
            } => {
                if let (Some(a), Some(v), Some(b)) =
                    (point_var(point1), point_var(vertex), point_var(point2))
                {
                    specs.push(ResidualSpec::AngleThreePoints {
                        a,
                        v,
                        b,
                        angle: f64::from(angle_rad),
                    });
                }
            }
            ConstraintKind::Gap {
                item1,
                item2,
                distance,
            } => {
                let d = f64::from(distance);
                let spec = match (item_vars(item1), item_vars(item2)) {
                    (Some(ItemVars::Point(p)), Some(ItemVars::Line(s, e)))
                    | (Some(ItemVars::Line(s, e)), Some(ItemVars::Point(p))) => {
                        Some(ResidualSpec::GapPointLine { p, s, e, d })
                    }
                    (Some(ItemVars::Point(p)), Some(ItemVars::Circle(c, r)))
                    | (Some(ItemVars::Circle(c, r)), Some(ItemVars::Point(p))) => {
                        Some(ResidualSpec::GapPointCircle {
                            p,
                            c,
                            r,
                            inside: crate::contact::point_inside(
                                segment_length(&vars, c, p),
                                vars[r],
                            ),
                            d,
                        })
                    }
                    (Some(ItemVars::Line(s1, e1)), Some(ItemVars::Line(s2, e2))) => {
                        Some(ResidualSpec::GapLines { s1, e1, s2, e2, d })
                    }
                    (Some(ItemVars::Line(s, e)), Some(ItemVars::Circle(c, r)))
                    | (Some(ItemVars::Circle(c, r)), Some(ItemVars::Line(s, e))) => {
                        Some(ResidualSpec::GapLineCircle { s, e, c, r, d })
                    }
                    (Some(ItemVars::Circle(c1, r1)), Some(ItemVars::Circle(c2, r2))) => {
                        Some(ResidualSpec::GapCircles {
                            c1,
                            r1,
                            c2,
                            r2,
                            nested: crate::contact::circles_nest(
                                segment_length(&vars, c1, c2),
                                vars[r1],
                                vars[r2],
                            ),
                            d,
                        })
                    }
                    _ => None,
                };
                specs.extend(spec);
            }
            ConstraintKind::Pitch {
                ref points,
                columns,
                distance,
                across,
                direction,
            } => {
                let Some(p) = points
                    .iter()
                    .map(|id| point_var(*id))
                    .collect::<Option<Vec<usize>>>()
                else {
                    continue;
                };
                let cols = (columns as usize).max(1);
                // The pairs that step: along each row, or down the first
                // column; the first pair sets the step the rest repeat.
                let steps: Vec<(usize, usize)> = if across {
                    (cols..p.len())
                        .step_by(cols)
                        .map(|i| (p[i - cols], p[i]))
                        .collect()
                } else {
                    (0..p.len())
                        .filter(|i| i % cols != 0)
                        .map(|i| (p[i - 1], p[i]))
                        .collect()
                };
                if let Some(&(a0, a1)) = steps.first() {
                    let d = f64::from(distance);
                    specs.push(match direction {
                        Some(way) => ResidualSpec::Step {
                            a0,
                            a1,
                            dx: way.x * d,
                            dy: way.y * d,
                        },
                        None => ResidualSpec::Distance { p1: a0, p2: a1, d },
                    });
                    for &(b0, b1) in &steps[1..] {
                        specs.push(ResidualSpec::SameStep { a0, a1, b0, b1 });
                    }
                }
            }
            ConstraintKind::PolarPitch {
                center,
                ref points,
                angle_rad,
            } => {
                let (Some(c), Some(p)) = (
                    point_var(center),
                    points
                        .iter()
                        .map(|id| point_var(*id))
                        .collect::<Option<Vec<usize>>>(),
                ) else {
                    continue;
                };
                for w in p.windows(2) {
                    specs.push(ResidualSpec::AngleThreePoints {
                        a: w[0],
                        v: c,
                        b: w[1],
                        angle: f64::from(angle_rad),
                    });
                }
                for &q in p.iter().skip(1) {
                    specs.push(ResidualSpec::EqualLength {
                        s1: c,
                        e1: p[0],
                        s2: c,
                        e2: q,
                    });
                }
            }
            ConstraintKind::Offset {
                ref pairs,
                distance,
            } => {
                let d = f64::from(distance);
                for &[a, b] in pairs {
                    match (item_vars(a.into()), item_vars(b.into())) {
                        (Some(ItemVars::Line(s1, e1)), Some(ItemVars::Line(s2, e2))) => {
                            specs.push(ResidualSpec::Parallel { s1, e1, s2, e2 });
                            specs.push(ResidualSpec::GapLines { s1, e1, s2, e2, d });
                        }
                        (Some(ItemVars::Circle(c1, r1)), Some(ItemVars::Circle(c2, r2))) => {
                            specs.push(ResidualSpec::GapCircles {
                                c1,
                                r1,
                                c2,
                                r2,
                                nested: true,
                                d,
                            });
                        }
                        _ => {}
                    }
                }
            }
            ConstraintKind::AngleAtPoint {
                curve1,
                curve2,
                point,
                angle_rad,
            } => {
                if let (Some(t1), Some(t2), Some(p)) =
                    (tangent_of(curve1), tangent_of(curve2), point_var(point))
                {
                    specs.push(ResidualSpec::AngleAtPoint {
                        t1,
                        t2,
                        p,
                        angle: f64::from(angle_rad),
                    });
                }
            }
            ConstraintKind::Refraction {
                ray1,
                ray2,
                interface,
                point,
                ratio,
            } => {
                if let Some(p) = point_var(point)
                    && let (Some((near1, far1)), Some((near2, far2)), Some(interface)) =
                        (ray_ends(ray1, p), ray_ends(ray2, p), tangent_of(interface))
                {
                    specs.push(ResidualSpec::Refraction {
                        near1,
                        far1,
                        near2,
                        far2,
                        interface,
                        p,
                        ratio: f64::from(ratio),
                    });
                }
            }
        }
        for spec in &specs[start..] {
            origins.extend(std::iter::repeat_n(
                EquationSource::Constraint(constraint.id),
                spec.dim(),
            ));
        }
    }

    // Implicit arc-consistency residuals: whenever the sketch has at least one
    // solvable user constraint, EVERY arc contributes |start - center| - r and
    // |end - center| - r. They are added for all arcs (not only arcs directly
    // referenced by a constraint) because the solver can move an arc's shared
    // points through constraints that never mention the arc itself; without
    // these residuals such an arc would silently become geometrically invalid.
    // When there are no user constraints the solver reports NothingToSolve and
    // never runs, so gating on "at least one constraint exists" costs nothing.
    if arcs_always || !specs.is_empty() {
        for element in &sketch.geometry {
            if let GeometryElement::Arc(arc) = element
                && let (Some(c), Some(s), Some(e), Some(r)) = (
                    point_var(arc.center),
                    point_var(arc.start),
                    point_var(arc.end),
                    radius_vars.get(&arc.id.into()).copied(),
                )
            {
                specs.push(ResidualSpec::ArcEndpoint { p: s, c, r });
                specs.push(ResidualSpec::ArcEndpoint { p: e, c, r });
                origins.push(EquationSource::ArcEndpoint {
                    curve: arc.id.into(),
                    point: arc.start,
                });
                origins.push(EquationSource::ArcEndpoint {
                    curve: arc.id.into(),
                    point: arc.end,
                });
            }
        }
    }

    vars.extend(aux);
    let implicit = conic_end_residuals(sketch, &point_var, &shape_vars)
        .into_iter()
        .chain(internal_residuals(
            sketch,
            exclude,
            &point_var,
            &shape_vars,
            &derived_foci,
        ));
    for (spec, origin) in implicit {
        origins.extend(std::iter::repeat_n(origin, spec.dim()));
        specs.push(spec);
    }

    let residual_len = specs.iter().map(ResidualSpec::dim).sum();
    let free: Vec<usize> = (0..vars.len()).filter(|i| !pinned.contains(i)).collect();
    System {
        origins,
        settings: sketch.settings,
        vars,
        free,
        point_vars,
        radius_vars,
        shape_vars,
        derived_foci,
        specs,
        residual_len,
    }
}

/// An item's variables: a point's x, a line's two ends, a circle's center
/// and radius.
#[derive(Debug, Clone, Copy)]
enum ItemVars {
    Point(usize),
    Line(usize, usize),
    Circle(usize, usize),
}

/// A parabola's or hyperbola's arc ends on its curve whatever else holds:
/// both ends of every one join every solve, so dragging an end slides it
/// along the curve and dragging the centre takes the arc along.
fn conic_end_residuals(
    sketch: &Sketch,
    point_var: &impl Fn(PointReference) -> Option<usize>,
    shape_vars: &HashMap<CurveReference, usize>,
) -> Vec<(ResidualSpec, EquationSource)> {
    let mut specs = Vec::new();
    for element in &sketch.geometry {
        let GeometryElement::Conic(conic) = element else {
            continue;
        };
        let Some(c) = point_var(conic.center) else {
            continue;
        };
        for end in [conic.start, conic.end] {
            if let Some(p) = point_var(end) {
                specs.push((
                    ResidualSpec::OnConic {
                        p,
                        c,
                        shape: curve_shape(sketch, shape_vars, conic.id.into()),
                        hyperbola: conic.kind == ConicKind::Hyperbola,
                    },
                    EquationSource::ConicEndpoint {
                        curve: conic.id.into(),
                        point: end,
                    },
                ));
            }
        }
    }
    specs
}

/// An ellipse's or conic's shape as the system has it: its variables when
/// it has them, else where it is.
fn curve_shape(
    sketch: &Sketch,
    shape_vars: &HashMap<CurveReference, usize>,
    id: CurveReference,
) -> CurveShape {
    if let Some(&k) = shape_vars.get(&id) {
        return CurveShape::Var(k);
    }
    match sketch.get_geometry(id) {
        Some(GeometryElement::Ellipse(e)) => CurveShape::Fixed {
            x: e.major.x,
            y: e.major.y,
            minor: f64::from(e.minor),
        },
        Some(GeometryElement::Conic(c)) => CurveShape::Fixed {
            x: c.axis.x,
            y: c.axis.y,
            minor: f64::from(c.minor),
        },
        _ => CurveShape::Fixed {
            x: 1.0,
            y: 0.0,
            minor: 0.0,
        },
    }
}

/// Each piece of internal geometry held where its curve puts it: every
/// point of it that is not the curve's own, once, however many pieces
/// share it (a parabola's axis ends at its focus). A control polygon is
/// drawn through the spline's own points and needs none.
fn internal_residuals(
    sketch: &Sketch,
    exclude: Option<ConstraintId>,
    point_var: &impl Fn(PointReference) -> Option<usize>,
    shape_vars: &HashMap<CurveReference, usize>,
    derived_foci: &HashSet<PointReference>,
) -> Vec<(ResidualSpec, EquationSource)> {
    let mut specs = Vec::new();
    let mut placed: HashSet<PointReference> = HashSet::new();
    type Focus = (PointReference, ConstraintId);
    type Foci = (usize, CurveShape, Option<Focus>, Option<Focus>);
    let mut foci: HashMap<CurveReference, Foci> = HashMap::new();
    for constraint in &sketch.constraints {
        let ConstraintKind::InternalAlignment {
            element,
            curve,
            role,
        } = constraint.kind
        else {
            continue;
        };
        if exclude == Some(constraint.id) {
            continue;
        }
        let Some(geometry) = sketch.get_geometry(curve) else {
            continue;
        };
        let (center, ellipse, hyperbola) = match geometry {
            GeometryElement::Ellipse(e) => (e.center, true, false),
            GeometryElement::Conic(c) => (c.center, false, c.kind == ConicKind::Hyperbola),
            _ => continue,
        };
        let Some(c) = point_var(center) else {
            continue;
        };
        let own = geometry.point_references();
        let shape = curve_shape(sketch, shape_vars, curve);
        // The points of the element and where each goes. An ellipse's
        // foci are placed after the loop, the second from the first.
        let targets: Vec<(PointReference, Offset)> = match (role, sketch.get_geometry(element)) {
            (InternalRole::Focus1 | InternalRole::Focus2, Some(GeometryElement::Point(p)))
                if ellipse && derived_foci.contains(&p.id.into()) =>
            {
                Vec::new()
            }
            (InternalRole::Focus1 | InternalRole::Focus2, Some(GeometryElement::Point(p)))
                if ellipse =>
            {
                let slot = foci.entry(curve).or_insert((c, shape, None, None));
                if role == InternalRole::Focus1 {
                    slot.2 = slot.2.or(Some((p.id.into(), constraint.id)));
                } else {
                    slot.3 = slot.3.or(Some((p.id.into(), constraint.id)));
                }
                Vec::new()
            }
            (InternalRole::Focus1 | InternalRole::Focus2, Some(GeometryElement::Point(p))) => {
                let at = if hyperbola {
                    Offset::HyperbolaFocus
                } else {
                    Offset::Major(1.0)
                };
                vec![(p.id.into(), at)]
            }
            (InternalRole::MajorAxis, Some(GeometryElement::Line(l))) => {
                let mut ends = vec![(l.end, Offset::Major(1.0))];
                if ellipse {
                    ends.push((l.start, Offset::Major(-1.0)));
                }
                ends
            }
            (InternalRole::MinorAxis, Some(GeometryElement::Line(l))) => {
                vec![(l.start, Offset::Minor(-1.0)), (l.end, Offset::Minor(1.0))]
            }
            _ => Vec::new(),
        };
        for (point, at) in targets {
            if own.contains(&point) || !placed.insert(point) {
                continue;
            }
            if let Some(p) = point_var(point) {
                specs.push((
                    ResidualSpec::Internal { p, c, shape, at },
                    EquationSource::Constraint(constraint.id),
                ));
            }
        }
    }
    // An ellipse's first focus solves `z² = a² − b²` and the second stands
    // opposite it through the centre, which keeps them on opposite sides
    // as they cross it; a lone focus solves the square itself.
    let mut foci: Vec<_> = foci.into_iter().collect();
    foci.sort_by_key(|(curve, _)| *curve);
    for (_, (c, shape, first, second)) in foci {
        let first = first
            .filter(|(id, _)| placed.insert(*id))
            .and_then(|(id, source)| point_var(id).map(|p| (p, source)));
        let second = second
            .filter(|(id, _)| placed.insert(*id))
            .and_then(|(id, source)| point_var(id).map(|p| (p, source)));
        match (first, second) {
            (Some((p, first)), Some((q, second))) => {
                specs.push((
                    ResidualSpec::Focus { p, c, shape },
                    EquationSource::Constraint(first),
                ));
                specs.push((
                    ResidualSpec::Midpoint { p: c, s: p, e: q },
                    EquationSource::Constraint(second),
                ));
            }
            (Some((p, source)), None) | (None, Some((p, source))) => specs.push((
                ResidualSpec::Focus { p, c, shape },
                EquationSource::Constraint(source),
            )),
            (None, None) => {}
        }
    }
    specs
}

/// The ellipse foci nothing holds but their curve: no other constraint
/// names them, no curve runs through them, and they are not being dragged.
fn derived_foci(
    sketch: &Sketch,
    exclude: Option<ConstraintId>,
    held: &[PointReference],
) -> HashSet<PointReference> {
    let mut foci = std::collections::HashSet::new();
    for c in &sketch.constraints {
        if let ConstraintKind::InternalAlignment {
            element,
            curve,
            role: InternalRole::Focus1 | InternalRole::Focus2,
        } = c.kind
            && exclude != Some(c.id)
            && matches!(
                sketch.get_geometry(curve),
                Some(GeometryElement::Ellipse(_))
            )
            && matches!(
                sketch.get_geometry(element),
                Some(GeometryElement::Point(_))
            )
            && !held.iter().any(|p| ItemReference::from(*p) == element)
            && let ItemReference::Point(p) = element
        {
            foci.insert(p);
        }
    }
    for c in &sketch.constraints {
        if exclude == Some(c.id) || matches!(c.kind, ConstraintKind::InternalAlignment { .. }) {
            continue;
        }
        for id in c.kind.references() {
            if let ItemReference::Point(p) = id {
                foci.remove(&p);
            }
        }
    }
    for g in &sketch.geometry {
        for id in g.point_references() {
            foci.remove(&id);
        }
    }
    foci
}

/// Resolve an optional point reference: absent is fine (measure from the
/// origin), present-but-unresolvable skips the whole constraint (`None`).
fn resolve_opt(
    id: Option<PointReference>,
    point_var: &impl Fn(PointReference) -> Option<usize>,
) -> Option<Option<usize>> {
    match id {
        None => Some(None),
        Some(id) => point_var(id).map(Some),
    }
}
