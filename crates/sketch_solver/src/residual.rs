use crate::curves::CurveVars;

/// One resolved constraint residual, expressed in variable indices.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ResidualSpec {
    /// An ellipse's semi-major or semi-minor radius, its shape at `k`,
    /// less `radius`.
    EllipseRadius { k: usize, major: bool, radius: f64 },
    /// The length along a curve less `length`: a spline's whole domain, or
    /// a conic's arc between the ends `ends` holds.
    CurveLength {
        curve: CurveVars,
        ends: Option<(usize, usize)>,
        length: f64,
    },
    /// p - curve(t) (2 residuals), `t` a variable of its own.
    PointOnCurve {
        p: usize,
        curve: CurveVars,
        t: usize,
    },
    /// Where two curves meet, a(ta) - b(tb) (2 residuals), and their
    /// directions there: the sine between them for tangent, the cosine for
    /// square.
    CurvesMeet {
        a: CurveVars,
        ta: usize,
        b: CurveVars,
        tb: usize,
        square: bool,
    },
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
    /// |coord(b) - coord(a)| - value on one axis, or the signed
    /// difference less a negative value. `a`/`b` index the exact variable
    /// (x or y already applied); `b = None` measures from origin.
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
    /// by the minor radius, ≈ signed distance near the boundary. The point
    /// and center move, and the shape too when it is in the system.
    PointOnEllipse {
        p: usize,
        c: usize,
        shape: CurveShape,
    },
    /// Implicit arc consistency: |endpoint - center| - r.
    ArcEndpoint { p: usize, c: usize, r: usize },
    /// A line and an arc joined smoothly at their shared end `p`: the
    /// radius there square to the line, dot(unit(p - c), unit(e - s)).
    TangentAtEnd {
        p: usize,
        c: usize,
        s: usize,
        e: usize,
    },
    /// Two arcs joined smoothly at their shared end `p`: both centres on
    /// one line through it, cross(unit(p - c1), unit(p - c2)).
    TangentArcsAtEnd { p: usize, c1: usize, c2: usize },
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
    /// r · sweep - length, the sweep counter-clockwise from the start
    /// point's angle about the center to the end point's.
    ArcLength {
        c: usize,
        s: usize,
        e: usize,
        r: usize,
        len: f64,
    },
    /// The major radii equal, then the minor ones: `a` and `b` index each
    /// ellipse's shape (major x, major y, minor radius).
    EqualEllipse { a: usize, b: usize },
    /// sweep - angle, the sweep counter-clockwise from the start point's
    /// angle about the center to the end point's.
    ArcAngle {
        c: usize,
        s: usize,
        e: usize,
        angle: f64,
    },
    /// wrap(angle at v from the arm to a to the arm to b - angle).
    AngleThreePoints {
        a: usize,
        v: usize,
        b: usize,
        angle: f64,
    },
    /// |perpendicular distance(p, infinite line)| - d.
    GapPointLine {
        p: usize,
        s: usize,
        e: usize,
        d: f64,
    },
    /// |p - c| - r - d, or r - |p - c| - d for a point inside the circle
    /// (the side is picked once per solve, like tangency's branch).
    GapPointCircle {
        p: usize,
        c: usize,
        r: usize,
        inside: bool,
        d: f64,
    },
    /// |perpendicular distance(midpoint of s2-e2, line s1-e1)| - d: two
    /// parallel lines apart.
    GapLines {
        s1: usize,
        e1: usize,
        s2: usize,
        e2: usize,
        d: f64,
    },
    /// A step of a set length and way: `a1 - a0` equals `(dx, dy)`.
    Step {
        a0: usize,
        a1: usize,
        dx: f64,
        dy: f64,
    },
    /// Two steps the same: `b1 - b0` equals `a1 - a0`, in x and in y.
    SameStep {
        a0: usize,
        a1: usize,
        b0: usize,
        b1: usize,
    },
    /// |perpendicular distance(center, infinite line)| - r - d.
    GapLineCircle {
        s: usize,
        e: usize,
        c: usize,
        r: usize,
        d: f64,
    },
    /// |c1 - c2| - (r1 + r2) - d apart, or |r1 - r2| - |c1 - c2| - d for
    /// one circle inside the other (picked once per solve).
    GapCircles {
        c1: usize,
        r1: usize,
        c2: usize,
        r2: usize,
        nested: bool,
        d: f64,
    },
    /// wrap(angle from curve 1's tangent at p to curve 2's - angle).
    AngleAtPoint {
        t1: Tangent,
        t2: Tangent,
        p: usize,
        angle: f64,
    },
    /// sin in - ratio · sin out: each ray's unit direction (toward p for
    /// the first, away from it for the second) along the interface's unit
    /// tangent at p. `near` is the end of a ray at p, picked once per solve.
    Refraction {
        near1: usize,
        far1: usize,
        near2: usize,
        far2: usize,
        interface: Tangent,
        p: usize,
        ratio: f64,
    },
    /// A parabola's or hyperbola's end point on its curve: the curve's
    /// equation over its gradient, ≈ signed distance. The point and the
    /// curve's centre move, and the shape too when it is in the system.
    OnConic {
        p: usize,
        c: usize,
        shape: CurveShape,
        hyperbola: bool,
    },
    /// p - (c + the offset `at` names in the curve's frame) (2 residuals):
    /// a piece of internal geometry where its curve puts it.
    Internal {
        p: usize,
        c: usize,
        shape: CurveShape,
        at: Offset,
    },
    /// An ellipse's focus `p`, its offset from the centre `z = along + i
    /// across` in the curve's frame: `z² = a² − b²` (2 residuals,
    /// `along · across` and `along² − across² − (a² − b²)`, over `2a`).
    /// It has no singular point where the radii meet: the focus slides
    /// into the centre and out across as the minor radius grows past the
    /// major, which side the solve keeps from where the point stands.
    Focus {
        p: usize,
        c: usize,
        shape: CurveShape,
    },
}

/// An ellipse's or conic's shape: the vector along its axis (an ellipse's
/// center to major vertex, a conic's axis) and its minor radius (a
/// hyperbola's semi-minor axis; a parabola has none). Held where it is,
/// or three variables of the system from `Var`'s index on, when internal
/// geometry can move it.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CurveShape {
    Fixed { x: f64, y: f64, minor: f64 },
    Var(usize),
}

impl CurveShape {
    /// `(axis x, axis y, minor)`.
    pub(crate) fn get(self, v: &[f64]) -> (f64, f64, f64) {
        match self {
            CurveShape::Fixed { x, y, minor } => (x, y, minor),
            CurveShape::Var(k) => (v[k], v[k + 1], v[k + 2]),
        }
    }
}

/// Where a piece of internal geometry sits from its curve's centre, along
/// the axis `u` and across it `w` (`u` turned a quarter counter-clockwise),
/// with `a` the axis length and `b` the minor radius.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Offset {
    /// `sign · a` along: an ellipse's major vertices, a conic's axis end.
    Major(f64),
    /// `sign · b` across.
    Minor(f64),
    /// `√(a² + b²)` along: a hyperbola's focus.
    HyperbolaFocus,
}

impl Offset {
    fn of(self, (x, y, minor): (f64, f64, f64), minimum_length: f64) -> (f64, f64) {
        let a = x.hypot(y).max(minimum_length);
        let (u, w) = ((x / a, y / a), (-y / a, x / a));
        let (along, across) = match self {
            Offset::Major(sign) => (sign * a, 0.0),
            Offset::Minor(sign) => (0.0, sign * minor),
            Offset::HyperbolaFocus => (a.hypot(minor), 0.0),
        };
        (u.0 * along + w.0 * across, u.1 * along + w.1 * across)
    }
}

/// Where a curve's tangent at a point comes from: a line's two ends, or a
/// circle's center (the tangent is square to the radius through the point,
/// counter-clockwise).
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Tangent {
    Line { s: usize, e: usize },
    Circle { c: usize },
}

impl Tangent {
    /// The (unnormalized) tangent at the point with x-variable `p`.
    fn at(self, v: &[f64], p: usize) -> (f64, f64) {
        match self {
            Tangent::Line { s, e } => (v[e] - v[s], v[e + 1] - v[s + 1]),
            Tangent::Circle { c } => (-(v[p + 1] - v[c + 1]), v[p] - v[c]),
        }
    }
}

fn unit(d: (f64, f64), minimum_length: f64) -> (f64, f64) {
    let len = (d.0 * d.0 + d.1 * d.1).sqrt().max(minimum_length);
    (d.0 / len, d.1 / len)
}

impl ResidualSpec {
    pub fn dim(&self) -> usize {
        match self {
            ResidualSpec::FixedPoint { .. }
            | ResidualSpec::Coincident { .. }
            | ResidualSpec::Symmetric { .. }
            | ResidualSpec::Midpoint { .. }
            | ResidualSpec::EqualEllipse { .. }
            | ResidualSpec::PointOnCurve { .. }
            | ResidualSpec::SameStep { .. }
            | ResidualSpec::Step { .. }
            | ResidualSpec::Internal { .. }
            | ResidualSpec::Focus { .. } => 2,
            ResidualSpec::CurvesMeet { .. } => 3,
            _ => 1,
        }
    }

    pub fn eval(&self, v: &[f64], out: &mut Vec<f64>) {
        self.eval_with_minimum(v, out, 1e-12);
    }

    pub(crate) fn eval_with_minimum(&self, v: &[f64], out: &mut Vec<f64>, minimum_length: f64) {
        match self {
            ResidualSpec::CurveLength {
                curve,
                ends,
                length,
            } => {
                let (t0, t1) = match ends {
                    Some((s, e)) => (
                        curve.param_of(v, *s, minimum_length).unwrap_or(0.0),
                        curve.param_of(v, *e, minimum_length).unwrap_or(0.0),
                    ),
                    None => curve.range(v, minimum_length),
                };
                let n = 128;
                let mut total = 0.0;
                let mut last = curve.at_with_minimum(v, t0, minimum_length);
                for i in 1..=n {
                    let q = curve.at_with_minimum(
                        v,
                        t0 + (t1 - t0) * i as f64 / n as f64,
                        minimum_length,
                    );
                    total += ((q[0] - last[0]).powi(2) + (q[1] - last[1]).powi(2)).sqrt();
                    last = q;
                }
                out.push(total - length);
                return;
            }
            ResidualSpec::PointOnCurve { p, curve, t } => {
                let q = curve.at_with_minimum(v, v[*t], minimum_length);
                out.push(v[*p] - q[0]);
                out.push(v[*p + 1] - q[1]);
                return;
            }
            ResidualSpec::CurvesMeet {
                a,
                ta,
                b,
                tb,
                square,
            } => {
                let (pa, pb) = (
                    a.at_with_minimum(v, v[*ta], minimum_length),
                    b.at_with_minimum(v, v[*tb], minimum_length),
                );
                out.push(pa[0] - pb[0]);
                out.push(pa[1] - pb[1]);
                let (da, db) = (
                    a.direction(v, v[*ta], minimum_length),
                    b.direction(v, v[*tb], minimum_length),
                );
                out.push(if *square {
                    da[0] * db[0] + da[1] * db[1]
                } else {
                    da[0] * db[1] - da[1] * db[0]
                });
                return;
            }
            _ => {}
        }
        match *self {
            ResidualSpec::PointOnCurve { .. }
            | ResidualSpec::CurvesMeet { .. }
            | ResidualSpec::CurveLength { .. } => {}
            ResidualSpec::EllipseRadius { k, major, radius } => {
                out.push(if major {
                    (v[k] * v[k] + v[k + 1] * v[k + 1]).sqrt() - radius
                } else {
                    v[k + 2] - radius
                });
            }
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
                // A positive value is how far apart, either way round; a
                // negative one is the way too: `b` that far on the
                // negative side of `a`.
                out.push(if value < 0.0 {
                    d - value
                } else {
                    d.abs() - value
                });
            }
            ResidualSpec::EqualRadius { r1, r2 } => out.push(v[r1] - v[r2]),
            ResidualSpec::EqualLength { s1, e1, s2, e2 } => {
                out.push(segment_length(v, s1, e1) - segment_length(v, s2, e2));
            }
            ResidualSpec::Parallel { s1, e1, s2, e2 } => {
                let d1 = unit_direction(v, s1, e1, minimum_length);
                let d2 = unit_direction(v, s2, e2, minimum_length);
                out.push(d1.0 * d2.1 - d1.1 * d2.0);
            }
            ResidualSpec::Perpendicular { s1, e1, s2, e2 } => {
                let d1 = unit_direction(v, s1, e1, minimum_length);
                let d2 = unit_direction(v, s2, e2, minimum_length);
                out.push(d1.0 * d2.0 + d1.1 * d2.1);
            }
            ResidualSpec::PointOnLine { p, s, e } => {
                let dx = v[e] - v[s];
                let dy = v[e + 1] - v[s + 1];
                let px = v[p] - v[s];
                let py = v[p + 1] - v[s + 1];
                let len = (dx * dx + dy * dy).sqrt().max(minimum_length);
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
            ResidualSpec::PointOnEllipse { p, c, shape } => {
                let (major_x, major_y, minor) = shape.get(v);
                let a = (major_x * major_x + major_y * major_y)
                    .sqrt()
                    .max(minimum_length);
                // Either radius may be the longer part way through a solve.
                let b = minor.abs().max(minimum_length);
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
                out.push(point_line_distance(v, c, s, e, minimum_length).abs() - v[r]);
            }
            ResidualSpec::TangentAtEnd { p, c, s, e } => {
                let radial = unit_direction(v, c, p, minimum_length);
                let along = unit_direction(v, s, e, minimum_length);
                out.push(radial.0 * along.0 + radial.1 * along.1);
            }
            ResidualSpec::TangentArcsAtEnd { p, c1, c2 } => {
                let a = unit_direction(v, c1, p, minimum_length);
                let b = unit_direction(v, c2, p, minimum_length);
                out.push(a.0 * b.1 - a.1 * b.0);
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
                let len = (dx * dx + dy * dy).sqrt().max(minimum_length);
                out.push(((mx - v[s]) * dy - (my - v[s + 1]) * dx) / len);
                // (2) p1 → p2 perpendicular to the line direction.
                let (ux, uy) = (dx / len, dy / len);
                out.push((v[p1] - v[p2]) * ux + (v[p1 + 1] - v[p2 + 1]) * uy);
            }
            ResidualSpec::Midpoint { p, s, e } => {
                out.push(v[p] - (v[s] + v[e]) * 0.5);
                out.push(v[p + 1] - (v[s + 1] + v[e + 1]) * 0.5);
            }
            ResidualSpec::ArcLength { c, s, e, r, len } => {
                let a0 = (v[s + 1] - v[c + 1]).atan2(v[s] - v[c]);
                let a1 = (v[e + 1] - v[c + 1]).atan2(v[e] - v[c]);
                let mut sweep = (a1 - a0) % std::f64::consts::TAU;
                if sweep <= 0.0 {
                    sweep += std::f64::consts::TAU;
                }
                out.push(v[r] * sweep - len);
            }
            ResidualSpec::GapPointLine { p, s, e, d } => {
                out.push(point_line_distance(v, p, s, e, minimum_length).abs() - d);
            }
            ResidualSpec::EqualEllipse { a, b } => {
                let major = |k: usize| (v[k] * v[k] + v[k + 1] * v[k + 1]).sqrt();
                out.push(major(a) - major(b));
                out.push(v[a + 2] - v[b + 2]);
            }
            ResidualSpec::ArcAngle { c, s, e, angle } => {
                let a0 = (v[s + 1] - v[c + 1]).atan2(v[s] - v[c]);
                let a1 = (v[e + 1] - v[c + 1]).atan2(v[e] - v[c]);
                let mut sweep = (a1 - a0) % std::f64::consts::TAU;
                if sweep <= 0.0 {
                    sweep += std::f64::consts::TAU;
                }
                out.push(sweep - angle);
            }
            ResidualSpec::AngleThreePoints { a, v: at, b, angle } => {
                let (x1, y1) = (v[a] - v[at], v[a + 1] - v[at + 1]);
                let (x2, y2) = (v[b] - v[at], v[b + 1] - v[at + 1]);
                let now = (x1 * y2 - y1 * x2).atan2(x1 * x2 + y1 * y2);
                let pi = std::f64::consts::PI;
                out.push((now - angle + pi).rem_euclid(2.0 * pi) - pi);
            }
            ResidualSpec::GapPointCircle { p, c, r, inside, d } => {
                let from_center = segment_length(v, c, p);
                out.push(if inside {
                    v[r] - from_center - d
                } else {
                    from_center - v[r] - d
                });
            }
            ResidualSpec::GapLineCircle { s, e, c, r, d } => {
                out.push(point_line_distance(v, c, s, e, minimum_length).abs() - v[r] - d);
            }
            ResidualSpec::GapLines { s1, e1, s2, e2, d } => {
                let (mx, my) = ((v[s2] + v[e2]) * 0.5, (v[s2 + 1] + v[e2 + 1]) * 0.5);
                let (dx, dy) = (v[e1] - v[s1], v[e1 + 1] - v[s1 + 1]);
                let len = (dx * dx + dy * dy).sqrt().max(minimum_length);
                let across = ((mx - v[s1]) * dy - (my - v[s1 + 1]) * dx) / len;
                out.push(across.abs() - d);
            }
            ResidualSpec::Step { a0, a1, dx, dy } => {
                out.push(v[a1] - v[a0] - dx);
                out.push(v[a1 + 1] - v[a0 + 1] - dy);
            }
            ResidualSpec::SameStep { a0, a1, b0, b1 } => {
                out.push((v[b1] - v[b0]) - (v[a1] - v[a0]));
                out.push((v[b1 + 1] - v[b0 + 1]) - (v[a1 + 1] - v[a0 + 1]));
            }
            ResidualSpec::GapCircles {
                c1,
                r1,
                c2,
                r2,
                nested,
                d,
            } => {
                let between = segment_length(v, c1, c2);
                out.push(if nested {
                    (v[r1] - v[r2]).abs() - between - d
                } else {
                    between - v[r1] - v[r2] - d
                });
            }
            ResidualSpec::AngleAtPoint { t1, t2, p, angle } => {
                let d1 = t1.at(v, p);
                let d2 = t2.at(v, p);
                let cross = d1.0 * d2.1 - d1.1 * d2.0;
                let dot = d1.0 * d2.0 + d1.1 * d2.1;
                out.push(wrap_angle(cross.atan2(dot) - angle));
            }
            ResidualSpec::Refraction {
                near1,
                far1,
                near2,
                far2,
                interface,
                p,
                ratio,
            } => {
                let t = unit(interface.at(v, p), minimum_length);
                let d1 = unit(
                    (v[near1] - v[far1], v[near1 + 1] - v[far1 + 1]),
                    minimum_length,
                );
                let d2 = unit(
                    (v[far2] - v[near2], v[far2 + 1] - v[near2 + 1]),
                    minimum_length,
                );
                let sin_in = d1.0 * t.0 + d1.1 * t.1;
                let sin_out = d2.0 * t.0 + d2.1 * t.1;
                out.push(sin_in - ratio * sin_out);
            }
            ResidualSpec::OnConic {
                p,
                c,
                shape,
                hyperbola,
            } => {
                let (axis_x, axis_y, minor) = shape.get(v);
                let a = axis_x.hypot(axis_y).max(minimum_length);
                let (ux, uy) = (axis_x / a, axis_y / a);
                let (dx, dy) = (v[p] - v[c], v[p + 1] - v[c + 1]);
                let x = dx * ux + dy * uy;
                let y = -dx * uy + dy * ux;
                let distance = if hyperbola {
                    let (a2, b2) = (a * a, minor.max(minimum_length).powi(2));
                    let f = x * x / a2 - y * y / b2 - 1.0;
                    let g = ((2.0 * x / a2).powi(2) + (2.0 * y / b2).powi(2)).sqrt();
                    f / g.max(minimum_length)
                } else {
                    (x - y * y / (4.0 * a)) / (1.0 + (y / (2.0 * a)).powi(2)).sqrt()
                };
                out.push(distance);
            }
            ResidualSpec::Internal { p, c, shape, at } => {
                let (dx, dy) = at.of(shape.get(v), minimum_length);
                out.push(v[p] - (v[c] + dx));
                out.push(v[p + 1] - (v[c + 1] + dy));
            }
            ResidualSpec::Focus { p, c, shape } => {
                let (x, y, minor) = shape.get(v);
                let a = x.hypot(y).max(minimum_length);
                let (dx, dy) = (v[p] - v[c], v[p + 1] - v[c + 1]);
                let along = (dx * x + dy * y) / a;
                let across = (dy * x - dx * y) / a;
                let scale = 2.0 * a.max(minor.abs());
                out.push(2.0 * along * across / scale);
                out.push((along * along - across * across - (a * a - minor * minor)) / scale);
            }
        }
    }
}

/// Signed perpendicular distance from point `p` to the infinite line
/// through `s` → `e`.
pub(crate) fn point_line_distance(
    v: &[f64],
    p: usize,
    s: usize,
    e: usize,
    minimum_length: f64,
) -> f64 {
    let dx = v[e] - v[s];
    let dy = v[e + 1] - v[s + 1];
    let len = (dx * dx + dy * dy).sqrt().max(minimum_length);
    ((v[p] - v[s]) * dy - (v[p + 1] - v[s + 1]) * dx) / len
}

pub(crate) fn segment_length(v: &[f64], a: usize, b: usize) -> f64 {
    let dx = v[b] - v[a];
    let dy = v[b + 1] - v[a + 1];
    (dx * dx + dy * dy).sqrt()
}

pub(crate) fn unit_direction(v: &[f64], s: usize, e: usize, minimum_length: f64) -> (f64, f64) {
    let dx = v[e] - v[s];
    let dy = v[e + 1] - v[s + 1];
    let len = (dx * dx + dy * dy).sqrt().max(minimum_length);
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
