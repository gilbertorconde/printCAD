//! 2D sketch constraint solver.
//!
//! Solves the sketch's constraints by adjusting point positions and
//! circle/arc radii using Gauss-Newton iteration with Levenberg-Marquardt
//! damping. The Jacobian is computed numerically (central differences) and
//! the normal equations are solved with a small dense Gaussian elimination —
//! no external solver dependencies.

use std::collections::HashMap;

use uuid::Uuid;

use crate::sketch::{AxisDirection, ConstraintKind, GeometryElement, Sketch, Vec2D};

/// Maximum number of outer (Jacobian) iterations.
const MAX_ITERATIONS: usize = 100;
/// Maximum damping retries per outer iteration before declaring a stall.
const MAX_INNER_RETRIES: usize = 25;
/// Initial Levenberg-Marquardt damping factor.
const LAMBDA_INIT: f64 = 1e-3;
const LAMBDA_MIN: f64 = 1e-12;
const LAMBDA_MAX: f64 = 1e12;
/// Floor for the diagonal damping term so a zero JᵀJ diagonal still damps.
const DAMPING_FLOOR: f64 = 1e-12;
/// Relative convergence tolerance. Convergence is declared when the residual
/// inf-norm drops below `CONVERGENCE_TOL * max(1, |x|_inf)`: residuals carry
/// length units, so scaling the tolerance by the model's coordinate magnitude
/// makes sketches drawn in millimetres and in metres behave the same, while
/// the `max(1, ..)` floor keeps tiny sketches from demanding sub-f64 accuracy.
const CONVERGENCE_TOL: f64 = 1e-9;
/// Relative step for central-difference Jacobian columns.
const FD_EPS: f64 = 1e-6;
/// Minimum length used when normalizing directions, to avoid division by
/// zero on degenerate (zero-length) lines.
const MIN_LEN: f64 = 1e-12;
/// Step cap (times the variable scale) to avoid explosions on ill-conditioned
/// iterations far from the solution.
const MAX_STEP_SCALE: f64 = 100.0;
/// Relative tolerance for the numerical rank of the Jacobian.
const RANK_TOL: f64 = 1e-8;

/// Result of a constraint solve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SolveOutcome {
    /// All residuals below tolerance.
    Converged { iterations: usize },
    /// Iteration limit hit with residual norm still above tolerance.
    NotConverged { residual: f64 },
    /// No constraints (or none referencing existing geometry).
    NothingToSolve,
}

/// Solve the sketch's constraints by adjusting point positions and
/// circle/arc radii in place. On NotConverged the best-effort geometry is
/// still written back. Also updates `sketch.is_fully_constrained`.
pub fn solve(sketch: &mut Sketch) -> SolveOutcome {
    let sys = build_system(sketch);
    if sys.specs.is_empty() || sys.vars.is_empty() {
        sketch.is_fully_constrained = false;
        return SolveOutcome::NothingToSolve;
    }

    let mut x = sys.vars.clone();
    let mut r = eval_residuals(&sys, &x);
    let mut cost = sq_norm(&r);
    let mut lambda = LAMBDA_INIT;
    let mut iterations = 0;
    let mut converged = inf_norm(&r) < CONVERGENCE_TOL * var_scale(&x);

    while !converged && iterations < MAX_ITERATIONS {
        iterations += 1;

        let jac = jacobian(&sys, &x);
        let (jtj, jtr) = normal_equations(&jac, &r);

        let mut improved = false;
        for _ in 0..MAX_INNER_RETRIES {
            // Damped normal equations: (JᵀJ + λ·diag(JᵀJ)) dx = -Jᵀr.
            let mut a = jtj.clone();
            for (i, row) in a.iter_mut().enumerate() {
                row[i] += lambda * jtj[i][i].max(DAMPING_FLOOR);
            }
            let rhs: Vec<f64> = jtr.iter().map(|v| -v).collect();
            let mut step = match solve_linear(a, rhs) {
                Some(s) => s,
                None => {
                    lambda = (lambda * 10.0).min(LAMBDA_MAX);
                    continue;
                }
            };
            cap_step(&mut step, var_scale(&x));

            let trial: Vec<f64> = x.iter().zip(&step).map(|(xi, di)| xi + di).collect();
            let trial_r = eval_residuals(&sys, &trial);
            let trial_cost = sq_norm(&trial_r);
            if trial_cost.is_finite() && trial_cost < cost {
                x = trial;
                r = trial_r;
                cost = trial_cost;
                lambda = (lambda / 10.0).max(LAMBDA_MIN);
                improved = true;
                break;
            }
            lambda = (lambda * 10.0).min(LAMBDA_MAX);
        }

        if inf_norm(&r) < CONVERGENCE_TOL * var_scale(&x) {
            converged = true;
            break;
        }
        if !improved {
            // Damping saturated without any cost reduction: the problem is
            // contradictory or we are at a (possibly non-zero) local minimum.
            break;
        }
    }

    write_back(sketch, &sys, &x);

    let outcome = if converged {
        SolveOutcome::Converged { iterations }
    } else {
        SolveOutcome::NotConverged {
            residual: inf_norm(&r),
        }
    };
    sketch.is_fully_constrained =
        matches!(outcome, SolveOutcome::Converged { .. }) && dof_estimate(sketch) == 0;
    outcome
}

/// Rough remaining-degrees-of-freedom estimate: free variables minus the
/// rank of the constraint Jacobian at the current configuration.
pub fn dof_estimate(sketch: &Sketch) -> i32 {
    let sys = build_system(sketch);
    let n = sys.vars.len() as i32;
    if sys.specs.is_empty() || sys.vars.is_empty() {
        return n;
    }
    let jac = jacobian(&sys, &sys.vars);
    n - jacobian_rank(jac) as i32
}

/// Above this many solver-relevant constraints `diagnose` skips the
/// per-constraint analysis (it is O(n · solve)) and only reports the DoF.
const MAX_DIAGNOSED_CONSTRAINTS: usize = 60;

/// Constraint-level health report for the panel.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Diagnosis {
    /// Remaining degrees of freedom (`dof_estimate`).
    pub dof: i32,
    /// Constraints whose removal does not change the Jacobian rank — they
    /// add nothing the others don't already enforce.
    pub redundant: Vec<Uuid>,
    /// Constraints whose individual removal lets the solve converge.
    pub conflicting: Vec<Uuid>,
    /// False when the sketch had too many constraints to analyze.
    pub analyzed: bool,
}

/// Diagnose the sketch's constraint system. Solves a clone first; on failure
/// each constraint is removed in turn and the solve retried (removal fixing
/// convergence marks it conflicting). After success, a constraint whose
/// exclusion leaves the Jacobian rank unchanged is redundant.
pub fn diagnose(sketch: &Sketch) -> Diagnosis {
    let dof = dof_estimate(sketch);
    let ids: Vec<Uuid> = sketch
        .constraints
        .iter()
        .filter(|c| c.is_solved())
        .map(|c| c.id)
        .collect();
    if ids.len() > MAX_DIAGNOSED_CONSTRAINTS {
        return Diagnosis {
            dof,
            analyzed: false,
            ..Diagnosis::default()
        };
    }

    let mut probe = sketch.clone();
    let outcome = solve(&mut probe);
    let mut diagnosis = Diagnosis {
        dof,
        analyzed: true,
        ..Diagnosis::default()
    };

    match outcome {
        SolveOutcome::NotConverged { .. } => {
            for &id in &ids {
                let mut without = sketch.clone();
                without.constraints.retain(|c| c.id != id);
                if matches!(
                    solve(&mut without),
                    SolveOutcome::Converged { .. } | SolveOutcome::NothingToSolve
                ) {
                    diagnosis.conflicting.push(id);
                }
            }
        }
        SolveOutcome::Converged { .. } | SolveOutcome::NothingToSolve => {
            // Redundancy is rank-based at the solved configuration: equal
            // rank without a constraint means its rows were dependent.
            let full = build_system(&probe);
            if !full.specs.is_empty() && !full.vars.is_empty() {
                let full_rank = jacobian_rank(jacobian(&full, &full.vars));
                for &id in &ids {
                    let sys = build_system_excluding(&probe, Some(id));
                    if sys.residual_len == full.residual_len {
                        continue; // constraint resolved to no rows anyway
                    }
                    let rank = jacobian_rank(jacobian(&sys, &sys.vars));
                    if rank == full_rank {
                        diagnosis.redundant.push(id);
                    }
                }
            }
        }
    }
    diagnosis
}

/// One resolved constraint residual, expressed in variable indices.
enum ResidualSpec {
    /// p - pos (2 residuals).
    FixedPoint { p: usize, x: f64, y: f64 },
    /// p1 - p2 (2 residuals).
    Coincident { p1: usize, p2: usize },
    /// y_end - y_start.
    Horizontal { s: usize, e: usize },
    /// x_end - x_start.
    Vertical { s: usize, e: usize },
    /// |end - start| - length.
    Length { s: usize, e: usize, len: f64 },
    /// |p1 - p2| - distance.
    Distance { p1: usize, p2: usize, d: f64 },
    /// r - radius.
    Radius { r: usize, radius: f64 },
    /// 2r - diameter.
    Diameter { r: usize, diameter: f64 },
    /// |coord(b) - coord(a)| - value, on one axis. `a`/`b` index the exact
    /// variable (x or y already applied); `b = None` measures from origin.
    CoordDistance {
        a: usize,
        b: Option<usize>,
        value: f64,
    },
    /// r1 - r2.
    EqualRadius { r1: usize, r2: usize },
    /// |line1| - |line2|.
    EqualLength {
        s1: usize,
        e1: usize,
        s2: usize,
        e2: usize,
    },
    /// cross(d1_hat, d2_hat).
    Parallel {
        s1: usize,
        e1: usize,
        s2: usize,
        e2: usize,
    },
    /// dot(d1_hat, d2_hat).
    Perpendicular {
        s1: usize,
        e1: usize,
        s2: usize,
        e2: usize,
    },
    /// cross(p - a, b - a) / |b - a| (perpendicular distance).
    PointOnLine { p: usize, s: usize, e: usize },
    /// |p - center| - r.
    PointOnCircle { p: usize, c: usize, r: usize },
    /// wrap(atan2(cross(d1, d2), dot(d1, d2)) - angle).
    Angle {
        s1: usize,
        e1: usize,
        s2: usize,
        e2: usize,
        angle: f64,
    },
    /// wrap(atan2(dy, dx) - target): line direction against a fixed target
    /// angle (axis constraints; the axis offset is folded into `target`).
    AngleToTarget { s: usize, e: usize, target: f64 },
    /// Approximate point-on-ellipse: the normalized ellipse equation scaled
    /// by the minor radius, ≈ signed distance near the boundary. The
    /// ellipse's shape (major vector, ratio) is fixed; only the point and
    /// center move.
    PointOnEllipse {
        p: usize,
        c: usize,
        major_x: f64,
        major_y: f64,
        ratio: f64,
    },
    /// Implicit arc consistency: |endpoint - center| - r.
    ArcEndpoint { p: usize, c: usize, r: usize },
    /// |perpendicular distance(center, infinite line)| - r.
    TangentLineCircle {
        s: usize,
        e: usize,
        c: usize,
        r: usize,
    },
    /// |c1 - c2| - (r1 + r2) (external) or |c1 - c2| - |r1 - r2| (internal).
    /// The branch is picked once per solve, from the configuration at solve
    /// start (see `build_system`), never per iteration.
    TangentCircles {
        c1: usize,
        r1: usize,
        c2: usize,
        r2: usize,
        internal: bool,
    },
    /// p1/p2 mirror-symmetric about a line: midpoint on the line (cross
    /// residual) and the p1→p2 direction perpendicular to it (dot residual).
    Symmetric {
        p1: usize,
        p2: usize,
        s: usize,
        e: usize,
    },
    /// p - (s + e)/2 (2 residuals).
    Midpoint { p: usize, s: usize, e: usize },
}

impl ResidualSpec {
    fn dim(&self) -> usize {
        match self {
            ResidualSpec::FixedPoint { .. }
            | ResidualSpec::Coincident { .. }
            | ResidualSpec::Symmetric { .. }
            | ResidualSpec::Midpoint { .. } => 2,
            _ => 1,
        }
    }

    fn eval(&self, v: &[f64], out: &mut Vec<f64>) {
        match *self {
            ResidualSpec::FixedPoint { p, x, y } => {
                out.push(v[p] - x);
                out.push(v[p + 1] - y);
            }
            ResidualSpec::Coincident { p1, p2 } => {
                out.push(v[p1] - v[p2]);
                out.push(v[p1 + 1] - v[p2 + 1]);
            }
            ResidualSpec::Horizontal { s, e } => out.push(v[e + 1] - v[s + 1]),
            ResidualSpec::Vertical { s, e } => out.push(v[e] - v[s]),
            ResidualSpec::Length { s, e, len } => {
                out.push(segment_length(v, s, e) - len);
            }
            ResidualSpec::Distance { p1, p2, d } => {
                out.push(segment_length(v, p1, p2) - d);
            }
            ResidualSpec::Radius { r, radius } => out.push(v[r] - radius),
            ResidualSpec::Diameter { r, diameter } => out.push(2.0 * v[r] - diameter),
            ResidualSpec::CoordDistance { a, b, value } => {
                let d = match b {
                    Some(b) => v[b] - v[a],
                    None => v[a],
                };
                out.push(d.abs() - value);
            }
            ResidualSpec::EqualRadius { r1, r2 } => out.push(v[r1] - v[r2]),
            ResidualSpec::EqualLength { s1, e1, s2, e2 } => {
                out.push(segment_length(v, s1, e1) - segment_length(v, s2, e2));
            }
            ResidualSpec::Parallel { s1, e1, s2, e2 } => {
                let d1 = unit_direction(v, s1, e1);
                let d2 = unit_direction(v, s2, e2);
                out.push(d1.0 * d2.1 - d1.1 * d2.0);
            }
            ResidualSpec::Perpendicular { s1, e1, s2, e2 } => {
                let d1 = unit_direction(v, s1, e1);
                let d2 = unit_direction(v, s2, e2);
                out.push(d1.0 * d2.0 + d1.1 * d2.1);
            }
            ResidualSpec::PointOnLine { p, s, e } => {
                let dx = v[e] - v[s];
                let dy = v[e + 1] - v[s + 1];
                let px = v[p] - v[s];
                let py = v[p + 1] - v[s + 1];
                let len = (dx * dx + dy * dy).sqrt().max(MIN_LEN);
                out.push((px * dy - py * dx) / len);
            }
            ResidualSpec::PointOnCircle { p, c, r } | ResidualSpec::ArcEndpoint { p, c, r } => {
                out.push(segment_length(v, c, p) - v[r]);
            }
            ResidualSpec::Angle {
                s1,
                e1,
                s2,
                e2,
                angle,
            } => {
                let d1 = (v[e1] - v[s1], v[e1 + 1] - v[s1 + 1]);
                let d2 = (v[e2] - v[s2], v[e2 + 1] - v[s2 + 1]);
                let cross = d1.0 * d2.1 - d1.1 * d2.0;
                let dot = d1.0 * d2.0 + d1.1 * d2.1;
                out.push(wrap_angle(cross.atan2(dot) - angle));
            }
            ResidualSpec::AngleToTarget { s, e, target } => {
                let dy = v[e + 1] - v[s + 1];
                let dx = v[e] - v[s];
                out.push(wrap_angle(dy.atan2(dx) - target));
            }
            ResidualSpec::PointOnEllipse {
                p,
                c,
                major_x,
                major_y,
                ratio,
            } => {
                let a = (major_x * major_x + major_y * major_y).sqrt().max(MIN_LEN);
                let b = (a * ratio).max(MIN_LEN);
                // Rotate the point into the ellipse frame.
                let (cos_t, sin_t) = (major_x / a, major_y / a);
                let dx = v[p] - v[c];
                let dy = v[p + 1] - v[c + 1];
                let u = dx * cos_t + dy * sin_t;
                let w = -dx * sin_t + dy * cos_t;
                let q = ((u / a).powi(2) + (w / b).powi(2)).sqrt();
                out.push((q - 1.0) * b);
            }
            ResidualSpec::TangentLineCircle { s, e, c, r } => {
                out.push(point_line_distance(v, c, s, e).abs() - v[r]);
            }
            ResidualSpec::TangentCircles {
                c1,
                r1,
                c2,
                r2,
                internal,
            } => {
                let target = if internal {
                    (v[r1] - v[r2]).abs()
                } else {
                    v[r1] + v[r2]
                };
                out.push(segment_length(v, c1, c2) - target);
            }
            ResidualSpec::Symmetric { p1, p2, s, e } => {
                // (1) The p1p2 midpoint lies on the line (signed
                // perpendicular distance, like PointOnLine).
                let (mx, my) = ((v[p1] + v[p2]) * 0.5, (v[p1 + 1] + v[p2 + 1]) * 0.5);
                let dx = v[e] - v[s];
                let dy = v[e + 1] - v[s + 1];
                let len = (dx * dx + dy * dy).sqrt().max(MIN_LEN);
                out.push(((mx - v[s]) * dy - (my - v[s + 1]) * dx) / len);
                // (2) p1 → p2 perpendicular to the line direction.
                let (ux, uy) = (dx / len, dy / len);
                out.push((v[p1] - v[p2]) * ux + (v[p1 + 1] - v[p2 + 1]) * uy);
            }
            ResidualSpec::Midpoint { p, s, e } => {
                out.push(v[p] - (v[s] + v[e]) * 0.5);
                out.push(v[p + 1] - (v[s + 1] + v[e + 1]) * 0.5);
            }
        }
    }
}

/// Signed perpendicular distance from point `p` to the infinite line
/// through `s` → `e`.
fn point_line_distance(v: &[f64], p: usize, s: usize, e: usize) -> f64 {
    let dx = v[e] - v[s];
    let dy = v[e + 1] - v[s + 1];
    let len = (dx * dx + dy * dy).sqrt().max(MIN_LEN);
    ((v[p] - v[s]) * dy - (v[p + 1] - v[s + 1]) * dx) / len
}

fn segment_length(v: &[f64], a: usize, b: usize) -> f64 {
    let dx = v[b] - v[a];
    let dy = v[b + 1] - v[a + 1];
    (dx * dx + dy * dy).sqrt()
}

fn unit_direction(v: &[f64], s: usize, e: usize) -> (f64, f64) {
    let dx = v[e] - v[s];
    let dy = v[e + 1] - v[s + 1];
    let len = (dx * dx + dy * dy).sqrt().max(MIN_LEN);
    (dx / len, dy / len)
}

fn wrap_angle(a: f64) -> f64 {
    use std::f64::consts::{PI, TAU};
    let mut a = a % TAU;
    if a > PI {
        a -= TAU;
    } else if a < -PI {
        a += TAU;
    }
    a
}

/// The constraint system: free variables plus resolved residual specs.
struct System {
    /// Initial values for all free variables.
    vars: Vec<f64>,
    /// Point id -> index of its x variable (y is at index + 1).
    point_vars: HashMap<Uuid, usize>,
    /// Circle/arc id -> index of its radius variable.
    radius_vars: HashMap<Uuid, usize>,
    /// Resolved residuals (user constraints plus implicit arc consistency).
    specs: Vec<ResidualSpec>,
    /// Total residual dimension.
    residual_len: usize,
}

/// Resolve the sketch into variables and residual specs. Constraints that
/// reference missing geometry ids (or geometry of the wrong kind) are
/// silently skipped, so malformed input never panics. Inactive constraints
/// and reference (non-driving) dimensions contribute nothing.
fn build_system(sketch: &Sketch) -> System {
    build_system_excluding(sketch, None)
}

/// `build_system` minus the constraint with id `exclude` (diagnostics probe
/// the system with individual constraints removed).
fn build_system_excluding(sketch: &Sketch, exclude: Option<Uuid>) -> System {
    let mut vars = Vec::new();
    let mut point_vars = HashMap::new();
    let mut radius_vars = HashMap::new();
    for element in &sketch.geometry {
        match element {
            GeometryElement::Point(p) => {
                point_vars.insert(p.id, vars.len());
                vars.push(f64::from(p.position.x));
                vars.push(f64::from(p.position.y));
            }
            GeometryElement::Circle(c) => {
                radius_vars.insert(c.id, vars.len());
                vars.push(f64::from(c.radius));
            }
            GeometryElement::Arc(a) => {
                radius_vars.insert(a.id, vars.len());
                vars.push(f64::from(a.radius));
            }
            // Lines are fully defined by their endpoint variables; ellipse
            // shape and spline control points carry no residuals (yet).
            GeometryElement::Line(_)
            | GeometryElement::Ellipse(_)
            | GeometryElement::BSpline(_) => {}
        }
    }

    let point_var = |id: Uuid| point_vars.get(&id).copied();
    // Line -> (start x-var, end x-var), only when both endpoints are points.
    let line_vars = |id: Uuid| match sketch.get_geometry(id) {
        Some(GeometryElement::Line(l)) => Some((point_var(l.start)?, point_var(l.end)?)),
        _ => None,
    };
    // Circle or arc -> (center x-var, radius var).
    let circle_vars = |id: Uuid| match sketch.get_geometry(id) {
        Some(GeometryElement::Circle(c)) => {
            Some((point_var(c.center)?, radius_vars.get(&id).copied()?))
        }
        Some(GeometryElement::Arc(a)) => {
            Some((point_var(a.center)?, radius_vars.get(&id).copied()?))
        }
        _ => None,
    };
    let radius_var = |id: Uuid| circle_vars(id).map(|(_, r)| r);

    let mut specs = Vec::new();
    for constraint in &sketch.constraints {
        if !constraint.is_solved() || exclude == Some(constraint.id) {
            continue;
        }
        match constraint.kind {
            ConstraintKind::FixedPoint { point, position } => {
                if let Some(p) = point_var(point) {
                    specs.push(ResidualSpec::FixedPoint {
                        p,
                        x: f64::from(position.x),
                        y: f64::from(position.y),
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
            ConstraintKind::Block { element } => {
                if let Some(geom) = sketch.get_geometry(element) {
                    // Fix every referenced point (and the element itself,
                    // for standalone points) at its current position, plus
                    // the radius when the element has one.
                    let mut ids = Sketch::curve_point_ids(geom);
                    if let GeometryElement::Point(p) = geom {
                        ids.push(p.id);
                    }
                    for pid in ids {
                        if let (Some(p), Some(pos)) = (point_var(pid), sketch.point_position(pid)) {
                            specs.push(ResidualSpec::FixedPoint {
                                p,
                                x: f64::from(pos.x),
                                y: f64::from(pos.y),
                            });
                        }
                    }
                    if let Some(&r) = radius_vars.get(&element) {
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
                {
                    if let Some(c) = point_var(el.center) {
                        specs.push(ResidualSpec::PointOnEllipse {
                            p,
                            c,
                            major_x: f64::from(el.major.x),
                            major_y: f64::from(el.major.y),
                            ratio: f64::from(el.ratio),
                        });
                    }
                }
            }
            ConstraintKind::Tangent {
                line_or_circle1,
                item2,
            } => {
                let resolved = match (
                    line_vars(line_or_circle1),
                    circle_vars(line_or_circle1),
                    line_vars(item2),
                    circle_vars(item2),
                ) {
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
    if !specs.is_empty() {
        for element in &sketch.geometry {
            if let GeometryElement::Arc(arc) = element {
                if let (Some(c), Some(s), Some(e), Some(r)) = (
                    point_var(arc.center),
                    point_var(arc.start),
                    point_var(arc.end),
                    radius_vars.get(&arc.id).copied(),
                ) {
                    specs.push(ResidualSpec::ArcEndpoint { p: s, c, r });
                    specs.push(ResidualSpec::ArcEndpoint { p: e, c, r });
                }
            }
        }
    }

    let residual_len = specs.iter().map(ResidualSpec::dim).sum();
    System {
        vars,
        point_vars,
        radius_vars,
        specs,
        residual_len,
    }
}

/// Resolve an optional point reference: absent is fine (measure from the
/// origin), present-but-unresolvable skips the whole constraint (`None`).
fn resolve_opt(
    id: Option<Uuid>,
    point_var: &impl Fn(Uuid) -> Option<usize>,
) -> Option<Option<usize>> {
    match id {
        None => Some(None),
        Some(id) => point_var(id).map(Some),
    }
}

fn eval_residuals(sys: &System, x: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(sys.residual_len);
    for spec in &sys.specs {
        spec.eval(x, &mut out);
    }
    out
}

/// Central-difference Jacobian (m residuals x n variables).
fn jacobian(sys: &System, x: &[f64]) -> Vec<Vec<f64>> {
    let n = x.len();
    let mut jac = vec![vec![0.0; n]; sys.residual_len];
    let mut probe = x.to_vec();
    for j in 0..n {
        let eps = FD_EPS * x[j].abs().max(1.0);
        let original = probe[j];
        probe[j] = original + eps;
        let r_plus = eval_residuals(sys, &probe);
        probe[j] = original - eps;
        let r_minus = eval_residuals(sys, &probe);
        probe[j] = original;
        for (row, (rp, rm)) in jac.iter_mut().zip(r_plus.iter().zip(&r_minus)) {
            row[j] = (rp - rm) / (2.0 * eps);
        }
    }
    jac
}

/// Build JᵀJ and Jᵀr for the normal equations.
fn normal_equations(jac: &[Vec<f64>], r: &[f64]) -> (Vec<Vec<f64>>, Vec<f64>) {
    let n = jac.first().map_or(0, Vec::len);
    let mut jtj = vec![vec![0.0; n]; n];
    let mut jtr = vec![0.0; n];
    for (row, ri) in jac.iter().zip(r) {
        for (a, ra) in row.iter().enumerate() {
            jtr[a] += ra * ri;
            for (acc, rb) in jtj[a].iter_mut().zip(row) {
                *acc += ra * rb;
            }
        }
    }
    (jtj, jtr)
}

/// Solve `a * x = b` with Gaussian elimination and partial pivoting.
/// Returns `None` when the matrix is numerically singular.
fn solve_linear(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let mut pivot_row = col;
        let mut pivot_val = a[col][col].abs();
        for (row, row_vals) in a.iter().enumerate().skip(col + 1) {
            let v = row_vals[col].abs();
            if v > pivot_val {
                pivot_val = v;
                pivot_row = row;
            }
        }
        if !pivot_val.is_finite() || pivot_val < 1e-300 {
            return None;
        }
        a.swap(col, pivot_row);
        b.swap(col, pivot_row);
        let (upper, lower) = a.split_at_mut(col + 1);
        let pivot_vals = &upper[col];
        let pivot = pivot_vals[col];
        let b_pivot = b[col];
        for (offset, row_vals) in lower.iter_mut().enumerate() {
            let factor = row_vals[col] / pivot;
            if factor == 0.0 {
                continue;
            }
            for (dst, src) in row_vals[col..].iter_mut().zip(&pivot_vals[col..]) {
                *dst -= factor * src;
            }
            b[col + 1 + offset] -= factor * b_pivot;
        }
    }
    let mut x = vec![0.0; n];
    for col in (0..n).rev() {
        let mut sum = b[col];
        for k in (col + 1)..n {
            sum -= a[col][k] * x[k];
        }
        x[col] = sum / a[col][col];
    }
    if x.iter().all(|v| v.is_finite()) {
        Some(x)
    } else {
        None
    }
}

/// Numerical rank via row echelon form with partial pivoting. Pivots below
/// `RANK_TOL` times the largest Jacobian entry are treated as zero, which is
/// loose enough to flag redundant (dependent) constraint rows near a solution
/// while keeping genuinely independent rows.
fn jacobian_rank(mut m: Vec<Vec<f64>>) -> usize {
    let rows = m.len();
    let cols = m.first().map_or(0, Vec::len);
    if rows == 0 || cols == 0 {
        return 0;
    }
    let max_abs = m.iter().flatten().fold(0.0_f64, |acc, v| acc.max(v.abs()));
    if max_abs == 0.0 || !max_abs.is_finite() {
        return 0;
    }
    let tol = max_abs * RANK_TOL;

    let mut rank = 0;
    let mut row = 0;
    for col in 0..cols {
        if row >= rows {
            break;
        }
        let mut pivot_row = row;
        let mut pivot_val = m[row][col].abs();
        for (r, row_vals) in m.iter().enumerate().skip(row + 1) {
            let v = row_vals[col].abs();
            if v > pivot_val {
                pivot_val = v;
                pivot_row = r;
            }
        }
        if pivot_val <= tol {
            continue;
        }
        m.swap(row, pivot_row);
        let (upper, lower) = m.split_at_mut(row + 1);
        let pivot_vals = &upper[row];
        let pivot = pivot_vals[col];
        for row_vals in lower.iter_mut() {
            let factor = row_vals[col] / pivot;
            if factor == 0.0 {
                continue;
            }
            for (dst, src) in row_vals[col..].iter_mut().zip(&pivot_vals[col..]) {
                *dst -= factor * src;
            }
        }
        row += 1;
        rank += 1;
    }
    rank
}

fn sq_norm(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum()
}

fn inf_norm(v: &[f64]) -> f64 {
    v.iter().fold(0.0_f64, |acc, x| acc.max(x.abs()))
}

/// Characteristic magnitude of the variable vector, floored at 1.
fn var_scale(x: &[f64]) -> f64 {
    inf_norm(x).max(1.0)
}

/// Cap the step inf-norm relative to the variable scale so a single
/// ill-conditioned iteration cannot fling the geometry to infinity.
fn cap_step(step: &mut [f64], scale: f64) {
    let max_step = MAX_STEP_SCALE * scale;
    let norm = inf_norm(step);
    if norm > max_step {
        let factor = max_step / norm;
        for v in step.iter_mut() {
            *v *= factor;
        }
    }
}

/// Write the solved variables back into the sketch geometry.
fn write_back(sketch: &mut Sketch, sys: &System, x: &[f64]) {
    for element in &mut sketch.geometry {
        match element {
            GeometryElement::Point(p) => {
                if let Some(&i) = sys.point_vars.get(&p.id) {
                    p.position = Vec2D::new(x[i] as f32, x[i + 1] as f32);
                }
            }
            GeometryElement::Circle(c) => {
                if let Some(&i) = sys.radius_vars.get(&c.id) {
                    c.radius = x[i] as f32;
                }
            }
            GeometryElement::Arc(a) => {
                if let Some(&i) = sys.radius_vars.get(&a.id) {
                    a.radius = x[i] as f32;
                }
            }
            GeometryElement::Line(_)
            | GeometryElement::Ellipse(_)
            | GeometryElement::BSpline(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Arc, Circle, Constraint, Ellipse, Line, Point};
    use std::f32::consts::PI;

    fn add_point(sketch: &mut Sketch, x: f32, y: f32) -> Uuid {
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
    }

    fn add_line(sketch: &mut Sketch, start: Uuid, end: Uuid) -> Uuid {
        sketch.add_geometry(GeometryElement::Line(Line::new(start, end)))
    }

    fn fix(sketch: &mut Sketch, point: Uuid, x: f32, y: f32) {
        sketch.add_constraint(ConstraintKind::FixedPoint {
            point,
            position: Vec2D::new(x, y),
        });
    }

    fn pos(sketch: &Sketch, id: Uuid) -> glam::Vec2 {
        match sketch.get_geometry(id) {
            Some(GeometryElement::Point(p)) => p.position.to_glam(),
            _ => panic!("expected point"),
        }
    }

    fn circle_radius(sketch: &Sketch, id: Uuid) -> f32 {
        match sketch.get_geometry(id) {
            Some(GeometryElement::Circle(c)) => c.radius,
            Some(GeometryElement::Arc(a)) => a.radius,
            _ => panic!("expected circle or arc"),
        }
    }

    fn assert_converged(outcome: SolveOutcome) {
        assert!(
            matches!(outcome, SolveOutcome::Converged { .. }),
            "expected convergence, got {outcome:?}"
        );
    }

    fn assert_near(actual: f32, expected: f32, tol: f32) {
        assert!(
            (actual - expected).abs() <= tol,
            "expected {expected}, got {actual} (tol {tol})"
        );
    }

    #[test]
    fn empty_sketch_returns_nothing_to_solve() {
        let mut sketch = Sketch::new("empty");
        assert_eq!(solve(&mut sketch), SolveOutcome::NothingToSolve);
        assert!(!sketch.is_fully_constrained);
    }

    #[test]
    fn constraints_on_missing_geometry_are_skipped() {
        let mut sketch = Sketch::new("missing");
        add_point(&mut sketch, 1.0, 2.0);
        sketch.add_constraint(ConstraintKind::Length {
            line: Uuid::new_v4(),
            length: 5.0,
        });
        sketch.add_constraint(ConstraintKind::Coincident {
            point1: Uuid::new_v4(),
            point2: Uuid::new_v4(),
        });
        // Horizontal on a non-line element is also skipped.
        let p = add_point(&mut sketch, 3.0, 4.0);
        sketch.add_constraint(ConstraintKind::Horizontal { element: p });
        assert_eq!(solve(&mut sketch), SolveOutcome::NothingToSolve);
    }

    #[test]
    fn fixed_point_moves_point() {
        let mut sketch = Sketch::new("fixed");
        let p = add_point(&mut sketch, 3.0, 4.0);
        fix(&mut sketch, p, 1.0, 2.0);
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, p).x, 1.0, 1e-4);
        assert_near(pos(&sketch, p).y, 2.0, 1e-4);
        assert!(sketch.is_fully_constrained);
    }

    #[test]
    fn horizontal_levels_sloped_line() {
        let mut sketch = Sketch::new("horizontal");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 2.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Horizontal { element: line });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, b).y, pos(&sketch, a).y, 1e-4);
        // The end's x stays free, so the sketch is not fully constrained.
        assert!(!sketch.is_fully_constrained);
    }

    #[test]
    fn vertical_aligns_sloped_line() {
        let mut sketch = Sketch::new("vertical");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 2.0, 10.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Vertical { element: line });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, b).x, pos(&sketch, a).x, 1e-4);
    }

    #[test]
    fn length_sets_line_length_keeping_direction() {
        let mut sketch = Sketch::new("length");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 3.0, 4.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Length { line, length: 10.0 });
        assert_converged(solve(&mut sketch));
        let dir = (pos(&sketch, b) - pos(&sketch, a)).normalize();
        assert_near((pos(&sketch, b) - pos(&sketch, a)).length(), 10.0, 1e-3);
        // Direction should be roughly preserved (initially (0.6, 0.8)).
        assert!(dir.dot(glam::Vec2::new(0.6, 0.8)) > 0.9);
    }

    #[test]
    fn coincident_merges_two_points() {
        let mut sketch = Sketch::new("coincident");
        let p1 = add_point(&mut sketch, 0.0, 0.0);
        let p2 = add_point(&mut sketch, 2.0, 2.0);
        sketch.add_constraint(ConstraintKind::Coincident {
            point1: p1,
            point2: p2,
        });
        assert_converged(solve(&mut sketch));
        assert_near((pos(&sketch, p1) - pos(&sketch, p2)).length(), 0.0, 1e-4);
    }

    #[test]
    fn distance_constraint_separates_points() {
        let mut sketch = Sketch::new("distance");
        let p1 = add_point(&mut sketch, 0.0, 0.0);
        let p2 = add_point(&mut sketch, 1.0, 0.0);
        fix(&mut sketch, p1, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Distance {
            point1: p1,
            point2: p2,
            distance: 5.0,
        });
        assert_converged(solve(&mut sketch));
        assert_near((pos(&sketch, p2) - pos(&sketch, p1)).length(), 5.0, 1e-3);
    }

    #[test]
    fn radius_constraint_resizes_circle() {
        let mut sketch = Sketch::new("radius");
        let center = add_point(&mut sketch, 1.0, 1.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
        sketch.add_constraint(ConstraintKind::Radius {
            circle,
            radius: 5.0,
        });
        assert_converged(solve(&mut sketch));
        assert_near(circle_radius(&sketch, circle), 5.0, 1e-4);
    }

    #[test]
    fn equal_radius_matches_two_circles() {
        let mut sketch = Sketch::new("equal_radius");
        let c1 = add_point(&mut sketch, 0.0, 0.0);
        let c2 = add_point(&mut sketch, 10.0, 0.0);
        let circle1 = sketch.add_geometry(GeometryElement::Circle(Circle::new(c1, 2.0)));
        let circle2 = sketch.add_geometry(GeometryElement::Circle(Circle::new(c2, 6.0)));
        sketch.add_constraint(ConstraintKind::EqualRadius { circle1, circle2 });
        assert_converged(solve(&mut sketch));
        assert_near(
            circle_radius(&sketch, circle1),
            circle_radius(&sketch, circle2),
            1e-4,
        );
    }

    #[test]
    fn equal_length_matches_two_lines() {
        let mut sketch = Sketch::new("equal_length");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let c = add_point(&mut sketch, 0.0, 5.0);
        let d = add_point(&mut sketch, 4.0, 5.0);
        let line1 = add_line(&mut sketch, a, b);
        let line2 = add_line(&mut sketch, c, d);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        fix(&mut sketch, c, 0.0, 5.0);
        sketch.add_constraint(ConstraintKind::EqualLength { line1, line2 });
        assert_converged(solve(&mut sketch));
        assert_near((pos(&sketch, d) - pos(&sketch, c)).length(), 10.0, 1e-3);
    }

    #[test]
    fn parallel_and_perpendicular_pair() {
        let mut sketch = Sketch::new("parallel_perpendicular");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let base = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);

        let c = add_point(&mut sketch, 0.0, 2.0);
        let d = add_point(&mut sketch, 8.0, 4.0);
        let para = add_line(&mut sketch, c, d);
        fix(&mut sketch, c, 0.0, 2.0);
        sketch.add_constraint(ConstraintKind::Parallel {
            line1: base,
            line2: para,
        });

        let e = add_point(&mut sketch, 5.0, 1.0);
        let f = add_point(&mut sketch, 6.0, 9.0);
        let perp = add_line(&mut sketch, e, f);
        fix(&mut sketch, e, 5.0, 1.0);
        sketch.add_constraint(ConstraintKind::Perpendicular {
            line1: base,
            line2: perp,
        });

        assert_converged(solve(&mut sketch));
        // Parallel to the x-axis base: equal y at both ends.
        assert_near(pos(&sketch, d).y, pos(&sketch, c).y, 1e-3);
        // Perpendicular to the x-axis base: equal x at both ends.
        assert_near(pos(&sketch, f).x, pos(&sketch, e).x, 1e-3);
    }

    #[test]
    fn point_on_line_projects_point() {
        let mut sketch = Sketch::new("point_on_line");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        let p = add_point(&mut sketch, 4.0, 3.0);
        sketch.add_constraint(ConstraintKind::PointOnLine { point: p, line });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, p).y, 0.0, 1e-3);
    }

    #[test]
    fn point_on_circle_snaps_to_radius() {
        let mut sketch = Sketch::new("point_on_circle");
        let center = add_point(&mut sketch, 0.0, 0.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 5.0)));
        fix(&mut sketch, center, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Radius {
            circle,
            radius: 5.0,
        });
        let p = add_point(&mut sketch, 8.0, 0.0);
        sketch.add_constraint(ConstraintKind::PointOnCircle { point: p, circle });
        assert_converged(solve(&mut sketch));
        assert_near((pos(&sketch, p) - pos(&sketch, center)).length(), 5.0, 1e-3);
    }

    #[test]
    fn angle_constraint_rotates_line() {
        let mut sketch = Sketch::new("angle");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let base = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        let c = add_point(&mut sketch, 0.0, 0.0);
        let d = add_point(&mut sketch, 10.0, 1.0);
        let rotated = add_line(&mut sketch, c, d);
        fix(&mut sketch, c, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Angle {
            line1: base,
            line2: rotated,
            angle_rad: PI / 4.0,
        });
        assert_converged(solve(&mut sketch));
        let dir = pos(&sketch, d) - pos(&sketch, c);
        assert_near(dir.y.atan2(dir.x), PI / 4.0, 1e-3);
    }

    #[test]
    fn rectangle_solves_and_reports_dimensions() {
        let mut sketch = Sketch::new("rectangle");
        // Roughly a 4 x 3 rectangle, perturbed.
        let a = add_point(&mut sketch, 0.1, -0.1);
        let b = add_point(&mut sketch, 3.8, 0.2);
        let c = add_point(&mut sketch, 4.1, 3.2);
        let d = add_point(&mut sketch, -0.2, 2.9);
        let bottom = add_line(&mut sketch, a, b);
        let right = add_line(&mut sketch, b, c);
        let top = add_line(&mut sketch, c, d);
        let left = add_line(&mut sketch, d, a);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Horizontal { element: bottom });
        sketch.add_constraint(ConstraintKind::Horizontal { element: top });
        sketch.add_constraint(ConstraintKind::Vertical { element: right });
        sketch.add_constraint(ConstraintKind::Vertical { element: left });
        sketch.add_constraint(ConstraintKind::Length {
            line: bottom,
            length: 4.0,
        });
        sketch.add_constraint(ConstraintKind::Length {
            line: right,
            length: 3.0,
        });

        assert_converged(solve(&mut sketch));
        let (pa, pb, pc, pd) = (
            pos(&sketch, a),
            pos(&sketch, b),
            pos(&sketch, c),
            pos(&sketch, d),
        );
        assert_near(pa.x, 0.0, 1e-3);
        assert_near(pa.y, 0.0, 1e-3);
        assert_near((pb - pa).length(), 4.0, 1e-3);
        assert_near((pc - pb).length(), 3.0, 1e-3);
        assert_near(pb.y, pa.y, 1e-3);
        assert_near(pc.x, pb.x, 1e-3);
        assert_near(pd.y, pc.y, 1e-3);
        assert_near(pd.x, pa.x, 1e-3);
        // 8 variables, 8 independent equations: fully constrained.
        assert_eq!(dof_estimate(&sketch), 0);
        assert!(sketch.is_fully_constrained);
    }

    #[test]
    fn overconstrained_but_consistent_converges() {
        let mut sketch = Sketch::new("overconstrained");
        let a = add_point(&mut sketch, 0.5, 0.2);
        let b = add_point(&mut sketch, 9.7, 0.1);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        // Redundant but consistent with the fixed endpoints.
        sketch.add_constraint(ConstraintKind::Length { line, length: 10.0 });
        sketch.add_constraint(ConstraintKind::Horizontal { element: line });
        assert_converged(solve(&mut sketch));
        assert_near((pos(&sketch, b) - pos(&sketch, a)).length(), 10.0, 1e-3);
        assert!(sketch.is_fully_constrained);
    }

    #[test]
    fn contradictory_lengths_return_not_converged() {
        let mut sketch = Sketch::new("contradictory");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 6.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Length { line, length: 5.0 });
        sketch.add_constraint(ConstraintKind::Length { line, length: 8.0 });
        match solve(&mut sketch) {
            SolveOutcome::NotConverged { residual } => assert!(residual > 1e-3),
            other => panic!("expected NotConverged, got {other:?}"),
        }
        assert!(!sketch.is_fully_constrained);
    }

    #[test]
    fn arc_endpoints_follow_radius_constraint() {
        let mut sketch = Sketch::new("arc");
        let center = add_point(&mut sketch, 0.0, 0.0);
        let start = add_point(&mut sketch, 5.0, 0.0);
        let end = add_point(&mut sketch, 0.0, 5.0);
        let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(center, start, end, 5.0)));
        fix(&mut sketch, center, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Radius {
            circle: arc,
            radius: 3.0,
        });
        assert_converged(solve(&mut sketch));
        assert_near(circle_radius(&sketch, arc), 3.0, 1e-3);
        // Implicit arc-consistency residuals pull the endpoints onto the
        // new radius.
        assert_near(
            (pos(&sketch, start) - pos(&sketch, center)).length(),
            3.0,
            1e-3,
        );
        assert_near(
            (pos(&sketch, end) - pos(&sketch, center)).length(),
            3.0,
            1e-3,
        );
    }

    #[test]
    fn tangent_line_circle_moves_center_onto_offset() {
        let mut sketch = Sketch::new("tangent_line_circle");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        let center = add_point(&mut sketch, 5.0, 3.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
        sketch.add_constraint(ConstraintKind::Radius {
            circle,
            radius: 2.0,
        });
        sketch.add_constraint(ConstraintKind::Tangent {
            line_or_circle1: line,
            item2: circle,
        });
        assert_converged(solve(&mut sketch));
        // Perpendicular distance from the center to the x-axis line must
        // equal the radius (center approached from y=3, so it lands at +2).
        assert_near(pos(&sketch, center).y, 2.0, 1e-3);
        assert_near(circle_radius(&sketch, circle), 2.0, 1e-4);
    }

    #[test]
    fn tangent_line_circle_works_with_swapped_operands() {
        let mut sketch = Sketch::new("tangent_swapped");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        let center = add_point(&mut sketch, 5.0, 3.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
        fix(&mut sketch, center, 5.0, 3.0);
        // Circle first, line second: the radius adapts instead.
        sketch.add_constraint(ConstraintKind::Tangent {
            line_or_circle1: circle,
            item2: line,
        });
        assert_converged(solve(&mut sketch));
        assert_near(circle_radius(&sketch, circle), 3.0, 1e-3);
    }

    #[test]
    fn tangent_circles_external_branch_stays_external() {
        let mut sketch = Sketch::new("tangent_external");
        let c1 = add_point(&mut sketch, 0.0, 0.0);
        let c2 = add_point(&mut sketch, 10.0, 0.0);
        let circle1 = sketch.add_geometry(GeometryElement::Circle(Circle::new(c1, 3.0)));
        let circle2 = sketch.add_geometry(GeometryElement::Circle(Circle::new(c2, 4.0)));
        fix(&mut sketch, c1, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Radius {
            circle: circle1,
            radius: 3.0,
        });
        sketch.add_constraint(ConstraintKind::Radius {
            circle: circle2,
            radius: 4.0,
        });
        sketch.add_constraint(ConstraintKind::Tangent {
            line_or_circle1: circle1,
            item2: circle2,
        });
        assert_converged(solve(&mut sketch));
        // Externally-tangent circles STAY external: center distance is
        // r1 + r2 = 7, not |r1 - r2| = 1.
        assert_near((pos(&sketch, c2) - pos(&sketch, c1)).length(), 7.0, 1e-3);
    }

    #[test]
    fn tangent_circles_internal_branch_when_overlapping() {
        let mut sketch = Sketch::new("tangent_internal");
        let c1 = add_point(&mut sketch, 0.0, 0.0);
        let c2 = add_point(&mut sketch, 1.5, 0.0);
        let circle1 = sketch.add_geometry(GeometryElement::Circle(Circle::new(c1, 3.0)));
        let circle2 = sketch.add_geometry(GeometryElement::Circle(Circle::new(c2, 4.0)));
        fix(&mut sketch, c1, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Radius {
            circle: circle1,
            radius: 3.0,
        });
        sketch.add_constraint(ConstraintKind::Radius {
            circle: circle2,
            radius: 4.0,
        });
        sketch.add_constraint(ConstraintKind::Tangent {
            line_or_circle1: circle1,
            item2: circle2,
        });
        assert_converged(solve(&mut sketch));
        // One circle inside the other: the internal branch |r1 - r2| = 1 is
        // closer than the external 7, so the solve keeps them nested.
        assert_near((pos(&sketch, c2) - pos(&sketch, c1)).length(), 1.0, 1e-3);
    }

    #[test]
    fn symmetric_mirrors_point_about_line() {
        let mut sketch = Sketch::new("symmetric");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        let p1 = add_point(&mut sketch, 3.0, 4.0);
        let p2 = add_point(&mut sketch, 5.0, -2.0);
        fix(&mut sketch, p1, 3.0, 4.0);
        sketch.add_constraint(ConstraintKind::Symmetric {
            point1: p1,
            point2: p2,
            line,
        });
        assert_converged(solve(&mut sketch));
        // Mirror of (3, 4) about the x-axis is (3, -4).
        assert_near(pos(&sketch, p2).x, 3.0, 1e-3);
        assert_near(pos(&sketch, p2).y, -4.0, 1e-3);
    }

    #[test]
    fn symmetric_about_diagonal_line() {
        let mut sketch = Sketch::new("symmetric_diag");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 10.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 10.0);
        let p1 = add_point(&mut sketch, 6.0, 2.0);
        let p2 = add_point(&mut sketch, 1.0, 5.0);
        fix(&mut sketch, p1, 6.0, 2.0);
        sketch.add_constraint(ConstraintKind::Symmetric {
            point1: p1,
            point2: p2,
            line,
        });
        assert_converged(solve(&mut sketch));
        // Mirror of (6, 2) about y = x is (2, 6).
        assert_near(pos(&sketch, p2).x, 2.0, 1e-3);
        assert_near(pos(&sketch, p2).y, 6.0, 1e-3);
    }

    #[test]
    fn midpoint_pins_point_to_line_center() {
        let mut sketch = Sketch::new("midpoint");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 4.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 4.0);
        let p = add_point(&mut sketch, 7.0, 7.0);
        sketch.add_constraint(ConstraintKind::Midpoint { point: p, line });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, p).x, 5.0, 1e-3);
        assert_near(pos(&sketch, p).y, 2.0, 1e-3);
    }

    #[test]
    fn dof_of_unconstrained_line_is_four() {
        let mut sketch = Sketch::new("dof_line");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        add_line(&mut sketch, a, b);
        assert_eq!(dof_estimate(&sketch), 4);
    }

    #[test]
    fn dof_of_fully_fixed_line_is_zero() {
        let mut sketch = Sketch::new("dof_fixed");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        assert_eq!(dof_estimate(&sketch), 0);
    }

    #[test]
    fn already_satisfied_converges_immediately() {
        let mut sketch = Sketch::new("satisfied");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        sketch.add_constraint(ConstraintKind::Horizontal { element: line });
        assert_eq!(
            solve(&mut sketch),
            SolveOutcome::Converged { iterations: 0 }
        );
    }

    #[test]
    fn diameter_constraint_resizes_circle() {
        let mut sketch = Sketch::new("diameter");
        let center = add_point(&mut sketch, 1.0, 1.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
        sketch.add_constraint(ConstraintKind::Diameter {
            circle,
            diameter: 9.0,
        });
        assert_converged(solve(&mut sketch));
        assert_near(circle_radius(&sketch, circle), 4.5, 1e-4);
    }

    #[test]
    fn distance_x_and_y_between_points() {
        let mut sketch = Sketch::new("dxdy");
        let p1 = add_point(&mut sketch, 0.0, 0.0);
        let p2 = add_point(&mut sketch, 1.0, 1.0);
        fix(&mut sketch, p1, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::DistanceX {
            a: p1,
            b: Some(p2),
            value: 7.0,
        });
        sketch.add_constraint(ConstraintKind::DistanceY {
            a: p1,
            b: Some(p2),
            value: 3.0,
        });
        assert_converged(solve(&mut sketch));
        assert_near((pos(&sketch, p2).x - pos(&sketch, p1).x).abs(), 7.0, 1e-3);
        assert_near((pos(&sketch, p2).y - pos(&sketch, p1).y).abs(), 3.0, 1e-3);
        assert!(sketch.is_fully_constrained);
    }

    #[test]
    fn distance_x_from_origin() {
        let mut sketch = Sketch::new("dx_origin");
        let p = add_point(&mut sketch, 2.0, 5.0);
        sketch.add_constraint(ConstraintKind::DistanceX {
            a: p,
            b: None,
            value: 6.0,
        });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, p).x.abs(), 6.0, 1e-3);
        assert_near(pos(&sketch, p).y, 5.0, 1e-4);
    }

    #[test]
    fn block_freezes_line_while_other_geometry_moves() {
        let mut sketch = Sketch::new("block");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        add_line(&mut sketch, a, b);
        let blocked_line = sketch
            .geometry
            .iter()
            .find_map(|g| match g {
                GeometryElement::Line(l) => Some(l.id),
                _ => None,
            })
            .unwrap();
        sketch.add_constraint(ConstraintKind::Block {
            element: blocked_line,
        });
        // A free point pulled onto the blocked endpoint: only the point moves.
        let p = add_point(&mut sketch, 3.0, 4.0);
        sketch.add_constraint(ConstraintKind::Coincident {
            point1: p,
            point2: b,
        });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, a).x, 0.0, 1e-4);
        assert_near(pos(&sketch, b).x, 10.0, 1e-4);
        assert_near(pos(&sketch, b).y, 0.0, 1e-4);
        assert_near(pos(&sketch, p).x, 10.0, 1e-3);
        assert_near(pos(&sketch, p).y, 0.0, 1e-3);
    }

    #[test]
    fn length_on_blocked_line_conflicts() {
        let mut sketch = Sketch::new("block_conflict");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        sketch.add_constraint(ConstraintKind::Block { element: line });
        sketch.add_constraint(ConstraintKind::Length { line, length: 5.0 });
        assert!(matches!(
            solve(&mut sketch),
            SolveOutcome::NotConverged { .. }
        ));
    }

    #[test]
    fn symmetric_about_point_centers_pair() {
        let mut sketch = Sketch::new("sym_point");
        let p1 = add_point(&mut sketch, 2.0, 3.0);
        let center = add_point(&mut sketch, 5.0, 5.0);
        let p2 = add_point(&mut sketch, 9.0, 9.0);
        fix(&mut sketch, p1, 2.0, 3.0);
        fix(&mut sketch, center, 5.0, 5.0);
        sketch.add_constraint(ConstraintKind::SymmetricAboutPoint {
            point1: p1,
            point2: p2,
            center,
        });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, p2).x, 8.0, 1e-3);
        assert_near(pos(&sketch, p2).y, 7.0, 1e-3);
    }

    #[test]
    fn angle_to_axis_rotates_line() {
        let mut sketch = Sketch::new("angle_axis");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 1.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::AngleToAxis {
            line,
            axis: AxisDirection::Horizontal,
            angle_rad: PI / 4.0,
        });
        assert_converged(solve(&mut sketch));
        let dir = pos(&sketch, b) - pos(&sketch, a);
        assert_near(dir.y.atan2(dir.x), PI / 4.0, 1e-3);

        // Vertical axis, zero angle: the line becomes vertical.
        let c = add_point(&mut sketch, 20.0, 0.0);
        let d = add_point(&mut sketch, 21.0, 10.0);
        let line2 = add_line(&mut sketch, c, d);
        fix(&mut sketch, c, 20.0, 0.0);
        sketch.add_constraint(ConstraintKind::AngleToAxis {
            line: line2,
            axis: AxisDirection::Vertical,
            angle_rad: 0.0,
        });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, d).x, pos(&sketch, c).x, 1e-3);
    }

    #[test]
    fn tangent_line_arc_moves_arc_center() {
        let mut sketch = Sketch::new("tangent_arc");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        let center = add_point(&mut sketch, 5.0, 3.0);
        let start = add_point(&mut sketch, 7.0, 3.0);
        let end = add_point(&mut sketch, 5.0, 5.0);
        let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(center, start, end, 2.0)));
        sketch.add_constraint(ConstraintKind::Radius {
            circle: arc,
            radius: 2.0,
        });
        sketch.add_constraint(ConstraintKind::Tangent {
            line_or_circle1: line,
            item2: arc,
        });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, center).y, 2.0, 1e-3);
        // Implicit arc consistency: endpoints follow the radius.
        assert_near(
            (pos(&sketch, start) - pos(&sketch, center)).length(),
            2.0,
            1e-3,
        );
    }

    #[test]
    fn tangent_arc_arc_separates_centers() {
        let mut sketch = Sketch::new("tangent_arcs");
        let make_arc = |sketch: &mut Sketch, cx: f32, r: f32| {
            let c = add_point(sketch, cx, 0.0);
            let s = add_point(sketch, cx + r, 0.0);
            let e = add_point(sketch, cx, r);
            let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, r)));
            (c, arc)
        };
        let (c1, arc1) = make_arc(&mut sketch, 0.0, 3.0);
        let (c2, arc2) = make_arc(&mut sketch, 10.0, 4.0);
        fix(&mut sketch, c1, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Radius {
            circle: arc1,
            radius: 3.0,
        });
        sketch.add_constraint(ConstraintKind::Radius {
            circle: arc2,
            radius: 4.0,
        });
        sketch.add_constraint(ConstraintKind::Tangent {
            line_or_circle1: arc1,
            item2: arc2,
        });
        assert_converged(solve(&mut sketch));
        assert_near((pos(&sketch, c2) - pos(&sketch, c1)).length(), 7.0, 1e-3);
    }

    #[test]
    fn point_on_ellipse_pulls_point_to_boundary() {
        let mut sketch = Sketch::new("point_on_ellipse");
        let center = add_point(&mut sketch, 0.0, 0.0);
        fix(&mut sketch, center, 0.0, 0.0);
        let ellipse = sketch.add_geometry(GeometryElement::Ellipse(Ellipse::new(
            center,
            Vec2D::new(4.0, 0.0),
            0.5,
        )));
        // Off the minor vertex: pulled to (0, 2).
        let p = add_point(&mut sketch, 0.0, 3.0);
        sketch.add_constraint(ConstraintKind::PointOnEllipse { point: p, ellipse });
        assert_converged(solve(&mut sketch));
        assert_near(pos(&sketch, p).x, 0.0, 1e-3);
        assert_near(pos(&sketch, p).y, 2.0, 1e-3);
    }

    #[test]
    fn reference_dimension_does_not_constrain() {
        let mut sketch = Sketch::new("reference");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 6.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        let mut reference = Constraint::new(ConstraintKind::Length { line, length: 10.0 });
        reference.driving = false;
        sketch.constraints.push(reference);
        assert_converged(solve(&mut sketch));
        // The line keeps its 6-long geometry; the reference only measures.
        assert_near((pos(&sketch, b) - pos(&sketch, a)).length(), 6.0, 1e-4);
        assert_near(
            crate::sketch::measured_value(&sketch, &sketch.constraints[1].kind).unwrap(),
            6.0,
            1e-4,
        );
    }

    #[test]
    fn inactive_constraint_is_skipped() {
        let mut sketch = Sketch::new("inactive");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 6.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Length { line, length: 5.0 });
        let mut disabled = Constraint::new(ConstraintKind::Length { line, length: 8.0 });
        disabled.active = false;
        sketch.constraints.push(disabled);
        // With the contradictory 8-length disabled, the solve converges.
        assert_converged(solve(&mut sketch));
        assert_near((pos(&sketch, b) - pos(&sketch, a)).length(), 5.0, 1e-3);
    }

    #[test]
    fn diagnose_flags_conflicting_lengths() {
        let mut sketch = Sketch::new("diag_conflict");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 6.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        let len5 = sketch.add_constraint(ConstraintKind::Length { line, length: 5.0 });
        let len8 = sketch.add_constraint(ConstraintKind::Length { line, length: 8.0 });
        let diag = diagnose(&sketch);
        assert!(diag.analyzed);
        assert!(diag.conflicting.contains(&len5));
        assert!(diag.conflicting.contains(&len8));
        // The anchor is not part of the conflict: removing it doesn't help.
        assert_eq!(diag.conflicting.len(), 2);
    }

    #[test]
    fn diagnose_flags_redundant_duplicate_parallel() {
        let mut sketch = Sketch::new("diag_redundant");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let base = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        fix(&mut sketch, b, 10.0, 0.0);
        let c = add_point(&mut sketch, 0.0, 2.0);
        let d = add_point(&mut sketch, 8.0, 2.0);
        let other = add_line(&mut sketch, c, d);
        fix(&mut sketch, c, 0.0, 2.0);
        let par1 = sketch.add_constraint(ConstraintKind::Parallel {
            line1: base,
            line2: other,
        });
        let par2 = sketch.add_constraint(ConstraintKind::Parallel {
            line1: base,
            line2: other,
        });
        let diag = diagnose(&sketch);
        assert!(diag.analyzed);
        assert!(diag.conflicting.is_empty());
        // Either copy can go without losing rank: both are flagged.
        assert!(diag.redundant.contains(&par1));
        assert!(diag.redundant.contains(&par2));
        // The fixes are all independent: none flagged.
        assert_eq!(diag.redundant.len(), 2);
    }

    #[test]
    fn diagnose_healthy_sketch_reports_nothing() {
        let mut sketch = Sketch::new("diag_ok");
        let a = add_point(&mut sketch, 0.0, 0.0);
        let b = add_point(&mut sketch, 10.0, 0.0);
        let line = add_line(&mut sketch, a, b);
        fix(&mut sketch, a, 0.0, 0.0);
        sketch.add_constraint(ConstraintKind::Horizontal { element: line });
        let diag = diagnose(&sketch);
        assert!(diag.analyzed);
        assert!(diag.conflicting.is_empty() && diag.redundant.is_empty());
        assert_eq!(diag.dof, 1, "the free endpoint's x remains");
    }

    #[test]
    fn diagnose_early_outs_above_constraint_limit() {
        let mut sketch = Sketch::new("diag_limit");
        let center = add_point(&mut sketch, 0.0, 0.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 5.0)));
        for _ in 0..61 {
            sketch.add_constraint(ConstraintKind::Radius {
                circle,
                radius: 5.0,
            });
        }
        let diag = diagnose(&sketch);
        assert!(!diag.analyzed, "too many constraints to analyze");
        assert!(diag.conflicting.is_empty() && diag.redundant.is_empty());
    }
}
