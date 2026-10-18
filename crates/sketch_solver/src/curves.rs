use crate::residual::CurveShape;

/// A curve as the system moves it, to find its point at a parameter: a
/// line, a circle or arc, an ellipse, a hyperbola or parabola, a spline
/// over its control points.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CurveVars {
    Line {
        s: usize,
        e: usize,
    },
    Circle {
        c: usize,
        r: usize,
    },
    Ellipse {
        c: usize,
        shape: CurveShape,
    },
    Hyperbola {
        c: usize,
        shape: CurveShape,
    },
    Parabola {
        c: usize,
        shape: CurveShape,
    },
    Spline {
        basis: crate::spline::Basis,
        control: Vec<usize>,
    },
}

impl CurveVars {
    /// The curve's frame from its shape: the axis direction, its length and
    /// the minor radius.
    pub(crate) fn frame(v: &[f64], shape: CurveShape, minimum_length: f64) -> ([f64; 2], f64, f64) {
        let (x, y, minor) = shape.get(v);
        let a = (x * x + y * y).sqrt().max(minimum_length);
        ([x / a, y / a], a, minor)
    }

    /// Its point at `t`.
    pub fn at(&self, v: &[f64], t: f64) -> [f64; 2] {
        self.at_with_minimum(v, t, 1e-12)
    }

    pub(crate) fn at_with_minimum(&self, v: &[f64], t: f64, minimum_length: f64) -> [f64; 2] {
        let along = |c: usize, u: [f64; 2], x: f64, y: f64| {
            [v[c] + u[0] * x - u[1] * y, v[c + 1] + u[1] * x + u[0] * y]
        };
        match self {
            CurveVars::Line { s, e } => [
                v[*s] + t * (v[*e] - v[*s]),
                v[*s + 1] + t * (v[*e + 1] - v[*s + 1]),
            ],
            CurveVars::Circle { c, r } => [v[*c] + v[*r] * t.cos(), v[*c + 1] + v[*r] * t.sin()],
            CurveVars::Ellipse { c, shape } => {
                let (u, a, b) = Self::frame(v, *shape, minimum_length);
                along(*c, u, a * t.cos(), b * t.sin())
            }
            CurveVars::Hyperbola { c, shape } => {
                let (u, a, b) = Self::frame(v, *shape, minimum_length);
                along(*c, u, a * t.cosh(), b * t.sinh())
            }
            // The vertex at `c`, the focus `f` along the axis: y² = 4fx.
            CurveVars::Parabola { c, shape } => {
                let (u, f, _) = Self::frame(v, *shape, minimum_length);
                along(*c, u, t * t / (4.0 * f), t)
            }
            CurveVars::Spline { basis, control } => {
                let pts: Vec<[f64; 2]> = control.iter().map(|&k| [v[k], v[k + 1]]).collect();
                basis.eval(&pts, t)
            }
        }
    }

    /// Its unit direction at `t`, by a small step either way.
    pub(crate) fn direction(&self, v: &[f64], t: f64, minimum_length: f64) -> [f64; 2] {
        let h = 1e-6;
        let at = |t: f64| self.at_with_minimum(v, t, minimum_length);
        // A spline ends where its knots do: within `h` of an end, the
        // difference is taken on the side the curve runs on.
        let (start, end) = match self {
            CurveVars::Spline { basis, .. } => basis.domain(),
            _ => (f64::NEG_INFINITY, f64::INFINITY),
        };
        let one_sided = |sign: f64| {
            let (p0, p1, p2) = (at(t), at(t + sign * h), at(t + 2.0 * sign * h));
            [
                sign * (4.0 * p1[0] - 3.0 * p0[0] - p2[0]),
                sign * (4.0 * p1[1] - 3.0 * p0[1] - p2[1]),
            ]
        };
        let [dx, dy] = if t + h > end {
            one_sided(-1.0)
        } else if t - h < start {
            one_sided(1.0)
        } else {
            let (a, b) = (at(t - h), at(t + h));
            [b[0] - a[0], b[1] - a[1]]
        };
        let len = (dx * dx + dy * dy).sqrt().max(1e-300);
        [dx / len, dy / len]
    }

    /// The parameter of a point on a parabola or hyperbola, in its frame;
    /// `None` for other curves.
    pub(crate) fn param_of(&self, v: &[f64], p: usize, minimum_length: f64) -> Option<f64> {
        let (c, shape, hyperbola) = match self {
            CurveVars::Hyperbola { c, shape } => (*c, *shape, true),
            CurveVars::Parabola { c, shape } => (*c, *shape, false),
            _ => return None,
        };
        let (u, _, b) = Self::frame(v, shape, minimum_length);
        let across = -(v[p] - v[c]) * u[1] + (v[p + 1] - v[c + 1]) * u[0];
        Some(if hyperbola {
            (across / b.max(minimum_length)).asinh()
        } else {
            across
        })
    }

    /// The parameters worth searching for a first guess.
    pub(crate) fn range(&self, v: &[f64], minimum_length: f64) -> (f64, f64) {
        match self {
            // A line's own length and as much again either side.
            CurveVars::Line { .. } => (-1.0, 2.0),
            CurveVars::Circle { .. } | CurveVars::Ellipse { .. } => (0.0, std::f64::consts::TAU),
            CurveVars::Hyperbola { .. } => (-4.0, 4.0),
            CurveVars::Parabola { shape, .. } => {
                let (_, f, _) = Self::frame(v, *shape, minimum_length);
                (-20.0 * f, 20.0 * f)
            }
            CurveVars::Spline { basis, .. } => basis.domain(),
        }
    }
}

/// The parameter where `curve` comes nearest `target`: sampled, then
/// narrowed.
pub(crate) fn nearest_param(
    curve: &CurveVars,
    v: &[f64],
    target: [f64; 2],
    minimum_length: f64,
) -> f64 {
    let d = |t: f64| {
        let p = curve.at_with_minimum(v, t, minimum_length);
        (p[0] - target[0]).powi(2) + (p[1] - target[1]).powi(2)
    };
    let (start, end) = curve.range(v, minimum_length);
    // A circle's and a line's nearest points are had exactly.
    match *curve {
        CurveVars::Circle { c, .. } => {
            let angle = (target[1] - v[c + 1]).atan2(target[0] - v[c]);
            return angle.rem_euclid(std::f64::consts::TAU);
        }
        CurveVars::Line { s, e } => {
            let (dx, dy) = (v[e] - v[s], v[e + 1] - v[s + 1]);
            let length_sq = dx * dx + dy * dy;
            if length_sq > minimum_length * minimum_length {
                let t = ((target[0] - v[s]) * dx + (target[1] - v[s + 1]) * dy) / length_sq;
                return t.clamp(start, end);
            }
        }
        _ => {}
    }
    let (mut lo, mut hi) = (start, end);
    let mut best = lo;
    for _ in 0..4 {
        let n = 64;
        let step = (hi - lo) / n as f64;
        best = (0..=n)
            .map(|i| lo + step * i as f64)
            .min_by(|a, b| d(*a).total_cmp(&d(*b)))
            .unwrap_or(lo);
        lo = best - step;
        hi = best + step;
    }
    // The narrowed samples miss the ends by a rounding; an end that is
    // the nearest is taken exactly.
    [start, end]
        .into_iter()
        .fold(best, |best, t| if d(t) <= d(best) { t } else { best })
}

/// The parameters where two curves come nearest each other: sampled, then
/// each narrowed against the other, as long as narrowing brings them
/// nearer. Where curves touch tangentially they part only slowly, and
/// narrowing one against the other can wander from where they meet.
pub(crate) fn nearest_params(
    a: &CurveVars,
    b: &CurveVars,
    v: &[f64],
    minimum_length: f64,
) -> (f64, f64) {
    let (a0, a1) = a.range(v, minimum_length);
    let (b0, b1) = b.range(v, minimum_length);
    let n = 48;
    let mut best = (a0, b0, f64::INFINITY);
    for i in 0..=n {
        let ta = a0 + (a1 - a0) * i as f64 / n as f64;
        let pa = a.at_with_minimum(v, ta, minimum_length);
        for j in 0..=n {
            let tb = b0 + (b1 - b0) * j as f64 / n as f64;
            let pb = b.at_with_minimum(v, tb, minimum_length);
            let d = (pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2);
            if d < best.2 {
                best = (ta, tb, d);
            }
        }
    }
    let gap = |ta: f64, tb: f64| {
        let (pa, pb) = (
            a.at_with_minimum(v, ta, minimum_length),
            b.at_with_minimum(v, tb, minimum_length),
        );
        (pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2)
    };
    // Each curve's ends against the other: where curves join end to end
    // or one ends on the other, the meeting is there exactly.
    for ta in [a0, a1] {
        let tb = nearest_param(
            b,
            v,
            a.at_with_minimum(v, ta, minimum_length),
            minimum_length,
        );
        let d = gap(ta, tb);
        if d < best.2 {
            best = (ta, tb, d);
        }
    }
    for tb in [b0, b1] {
        let ta = nearest_param(
            a,
            v,
            b.at_with_minimum(v, tb, minimum_length),
            minimum_length,
        );
        let d = gap(ta, tb);
        if d < best.2 {
            best = (ta, tb, d);
        }
    }
    let (mut ta, mut tb, mut d) = best;
    for _ in 0..4 {
        let nb = nearest_param(
            b,
            v,
            a.at_with_minimum(v, ta, minimum_length),
            minimum_length,
        );
        let na = nearest_param(
            a,
            v,
            b.at_with_minimum(v, nb, minimum_length),
            minimum_length,
        );
        let nd = gap(na, nb);
        if nd >= d {
            break;
        }
        (ta, tb, d) = (na, nb, nd);
    }
    (ta, tb)
}
