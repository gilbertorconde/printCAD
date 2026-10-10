//! Any sketch curve as a point moving with one parameter: lines, circles
//! and their arcs, ellipses and their arcs, parabolas, hyperbolas and
//! splines alike. Where two of them cross, and where along one a point
//! sits, for the editing tools that cut and grow curves. Worked in double
//! precision.

use glam::DVec2;

use crate::conic::Shape;
use crate::sketch::{GeometryElement, Sketch};
use sketch_solver::spline::Basis;

/// What a curve runs along, whatever part of it the element keeps.
#[derive(Debug, Clone)]
enum Carrier {
    Line {
        a: DVec2,
        b: DVec2,
    },
    Circle {
        c: DVec2,
        r: f64,
    },
    Ellipse {
        c: DVec2,
        major: DVec2,
        minor: DVec2,
    },
    Conic(Shape),
    Spline {
        basis: Basis,
        control: Vec<[f64; 2]>,
    },
}

/// A curve element over its parameter.
#[derive(Debug, Clone)]
pub struct Curve {
    carrier: Carrier,
    /// The parameters the element covers, `span.0 < span.1`.
    pub span: (f64, f64),
    /// A closed carrier repeats every `period` (a circle's and ellipse's
    /// turn, a closed spline's domain); zero for an open one.
    pub period: f64,
    /// The element is the whole closed carrier: a circle, a whole ellipse,
    /// a closed spline.
    pub closed: bool,
}

fn dv(p: crate::sketch::Vec2D) -> DVec2 {
    DVec2::new(f64::from(p.x), f64::from(p.y))
}

impl Curve {
    /// The straight segment from `a` to `b`.
    pub fn segment(a: DVec2, b: DVec2) -> Self {
        Self {
            carrier: Carrier::Line { a, b },
            span: (0.0, 1.0),
            period: 0.0,
            closed: false,
        }
    }

    /// The curve of `geom`; `None` for a point or a degenerate curve.
    pub fn of(sketch: &Sketch, geom: &GeometryElement) -> Option<Self> {
        let pos = |id| sketch.point_position(id).map(dv);
        let tau = std::f64::consts::TAU;
        Some(match geom {
            GeometryElement::Point(_) => return None,
            GeometryElement::Line(l) => Self {
                carrier: Carrier::Line {
                    a: pos(l.start)?,
                    b: pos(l.end)?,
                },
                span: (0.0, 1.0),
                period: 0.0,
                closed: false,
            },
            GeometryElement::Circle(c) => Self {
                carrier: Carrier::Circle {
                    c: pos(c.center)?,
                    r: f64::from(c.radius),
                },
                span: (0.0, tau),
                period: tau,
                closed: true,
            },
            GeometryElement::Arc(a) => {
                let c = pos(a.center)?;
                let (s, e) = (pos(a.start)? - c, pos(a.end)? - c);
                let start = s.y.atan2(s.x);
                let mut end = e.y.atan2(e.x);
                while end <= start + 1e-12 {
                    end += tau;
                }
                Self {
                    carrier: Carrier::Circle { c, r: s.length() },
                    span: (start, end),
                    period: tau,
                    closed: false,
                }
            }
            GeometryElement::Ellipse(e) => {
                let major = dv(e.major);
                let (t0, t1) = e.param_span(sketch)?;
                Self {
                    carrier: Carrier::Ellipse {
                        c: pos(e.center)?,
                        major,
                        minor: major.perp() * f64::from(e.ratio),
                    },
                    span: (f64::from(t0), f64::from(t1)),
                    period: tau,
                    closed: e.arc.is_none(),
                }
            }
            GeometryElement::Conic(k) => {
                let (shape, t0, t1) = k.params(sketch)?;
                Self {
                    carrier: Carrier::Conic(shape),
                    span: (t0.min(t1), t0.max(t1)),
                    period: 0.0,
                    closed: false,
                }
            }
            GeometryElement::BSpline(b) => {
                let basis = crate::spline::basis_of(b)?;
                let control = b
                    .control_points
                    .iter()
                    .map(|id| pos(*id).map(|p| [p.x, p.y]))
                    .collect::<Option<Vec<_>>>()?;
                let span = basis.domain();
                Self {
                    period: if b.periodic { span.1 - span.0 } else { 0.0 },
                    carrier: Carrier::Spline { basis, control },
                    span,
                    closed: b.periodic,
                }
            }
        })
    }

    /// The point at `t`.
    pub fn at(&self, t: f64) -> DVec2 {
        match &self.carrier {
            Carrier::Line { a, b } => *a + (*b - *a) * t,
            Carrier::Circle { c, r } => *c + DVec2::new(t.cos(), t.sin()) * *r,
            Carrier::Ellipse { c, major, minor } => *c + *major * t.cos() + *minor * t.sin(),
            Carrier::Conic(shape) => shape.point(t).into(),
            Carrier::Spline { basis, control } => {
                let (t0, _) = self.span;
                let t = if self.period > 0.0 {
                    t0 + (t - t0).rem_euclid(self.period)
                } else {
                    t
                };
                basis.eval(control, t).into()
            }
        }
    }

    /// Whether the carrier goes on past the element's ends: a line, an arc
    /// of a circle or ellipse, a parabola or hyperbola. A spline stops at
    /// its ends.
    pub fn extends(&self) -> bool {
        !matches!(self.carrier, Carrier::Spline { .. })
    }

    /// The parameters the carrier covers for looking past the element's
    /// ends: a whole turn of a closed one from the element's start, a long
    /// way on each side of a line's or conic's part, and an open spline's
    /// own span, since it stops at its ends.
    pub fn reach(&self) -> (f64, f64) {
        if self.period > 0.0 {
            return (self.span.0, self.span.0 + self.period);
        }
        match self.carrier {
            Carrier::Spline { .. } => self.span,
            Carrier::Line { .. } => (-1e3, 1e3),
            _ => {
                let len = (self.span.1 - self.span.0).max(1.0);
                (self.span.0 - 4.0 * len, self.span.1 + 4.0 * len)
            }
        }
    }

    /// `t` brought into `[from, from + period)` on a closed curve.
    pub fn wrap(&self, t: f64, from: f64) -> f64 {
        if self.period > 0.0 {
            from + (t - from).rem_euclid(self.period)
        } else {
            t
        }
    }

    /// How many straight pieces follow the curve closely over `span`.
    fn pieces(&self, span: (f64, f64)) -> usize {
        let part = |per_turn: f64, turn: f64| {
            ((span.1 - span.0).abs() / turn * per_turn).ceil() as usize + 4
        };
        match &self.carrier {
            Carrier::Line { .. } => 1,
            Carrier::Circle { .. } | Carrier::Ellipse { .. } => part(128.0, std::f64::consts::TAU),
            Carrier::Conic(_) => 128,
            Carrier::Spline { control, .. } => {
                let domain = self.span.1 - self.span.0;
                part(24.0 * control.len() as f64, domain.max(1e-9))
            }
        }
    }

    /// Points along `span`, with their parameters.
    fn samples(&self, span: (f64, f64)) -> Vec<(f64, DVec2)> {
        let n = self.pieces(span);
        (0..=n)
            .map(|i| {
                let t = span.0 + (span.1 - span.0) * i as f64 / n as f64;
                (t, self.at(t))
            })
            .collect()
    }

    /// The parameter within `span` of the point of the curve nearest `p`.
    pub fn nearest(&self, p: DVec2, span: (f64, f64)) -> f64 {
        if let Carrier::Line { a, b } = self.carrier {
            let d = b - a;
            let t = (p - a).dot(d) / d.length_squared().max(1e-300);
            return t.clamp(span.0, span.1);
        }
        let samples = self.samples(span);
        let n = samples.len() - 1;
        let best = (0..=n)
            .min_by(|i, j| {
                (samples[*i].1 - p)
                    .length_squared()
                    .total_cmp(&(samples[*j].1 - p).length_squared())
            })
            .unwrap_or(0);
        let (mut lo, mut hi) = (
            samples[best.saturating_sub(1)].0,
            samples[(best + 1).min(n)].0,
        );
        let dist = |t: f64| (self.at(t) - p).length_squared();
        for _ in 0..60 {
            let (m1, m2) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
            if dist(m1) < dist(m2) {
                hi = m2;
            } else {
                lo = m1;
            }
        }
        (lo + hi) * 0.5
    }
}

/// Where the segments `p0 → p1` and `q0 → q1` cross, as fractions along
/// each.
fn segments_cross(p0: DVec2, p1: DVec2, q0: DVec2, q1: DVec2) -> Option<(f64, f64)> {
    let (d, e) = (p1 - p0, q1 - q0);
    let denom = d.perp_dot(e);
    if denom.abs() < 1e-300 {
        return None;
    }
    let w = q0 - p0;
    let s = w.perp_dot(e) / denom;
    let u = w.perp_dot(d) / denom;
    let slack = 1e-9;
    ((-slack..=1.0 + slack).contains(&s) && (-slack..=1.0 + slack).contains(&u)).then_some((s, u))
}

/// Where `a` over `sa` crosses `b` over `sb`: the parameters on each.
pub fn crossings(a: &Curve, sa: (f64, f64), b: &Curve, sb: (f64, f64)) -> Vec<(f64, f64)> {
    let pa = a.samples(sa);
    let pb = b.samples(sb);
    let bounds = |w: &[(f64, DVec2)]| {
        w.iter().fold(
            (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
            |(lo, hi), (_, p)| (lo.min(*p), hi.max(*p)),
        )
    };
    let mut out: Vec<(f64, f64)> = Vec::new();
    for wa in pa.windows(2) {
        let (alo, ahi) = bounds(wa);
        for wb in pb.windows(2) {
            let (blo, bhi) = bounds(wb);
            let tol = 1e-9;
            if alo.x > bhi.x + tol
                || blo.x > ahi.x + tol
                || alo.y > bhi.y + tol
                || blo.y > ahi.y + tol
            {
                continue;
            }
            let Some((s, u)) = segments_cross(wa[0].1, wa[1].1, wb[0].1, wb[1].1) else {
                continue;
            };
            let ta = wa[0].0 + (wa[1].0 - wa[0].0) * s;
            let tb = wb[0].0 + (wb[1].0 - wb[0].0) * u;
            let Some((ta, tb)) = refine(a, b, ta, tb) else {
                continue;
            };
            let within = |t: f64, span: (f64, f64)| {
                let slack = 1e-9 * (span.1 - span.0).abs().max(1.0);
                t >= span.0 - slack && t <= span.1 + slack
            };
            if !within(ta, sa) || !within(tb, sb) {
                continue;
            }
            let p = a.at(ta);
            if out.iter().all(|(t, _)| (a.at(*t) - p).length() > 1e-7) {
                out.push((ta, tb));
            }
        }
    }
    out
}

/// Newton's steps from a sampled crossing to the exact one.
fn refine(a: &Curve, b: &Curve, mut ta: f64, mut tb: f64) -> Option<(f64, f64)> {
    let ha = 1e-7 * (a.span.1 - a.span.0).abs().max(1.0);
    let hb = 1e-7 * (b.span.1 - b.span.0).abs().max(1.0);
    for _ in 0..20 {
        let f = a.at(ta) - b.at(tb);
        if f.length() < 1e-12 {
            break;
        }
        let da = (a.at(ta + ha) - a.at(ta - ha)) / (2.0 * ha);
        let db = -(b.at(tb + hb) - b.at(tb - hb)) / (2.0 * hb);
        let det = da.perp_dot(db);
        if det.abs() < 1e-300 {
            break;
        }
        // Solve [da db] · (x, y) = -f.
        let x = (-f).perp_dot(db) / det;
        let y = da.perp_dot(-f) / det;
        ta += x;
        tb += y;
    }
    let miss = (a.at(ta) - b.at(tb)).length();
    (miss < 1e-6).then_some((ta, tb))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Ellipse, Line, Point, Vec2D};

    fn pt(sketch: &mut Sketch, x: f32, y: f32) -> uuid::Uuid {
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
    }

    #[test]
    fn a_line_crosses_an_ellipse_twice() {
        let mut sketch = Sketch::new("t");
        let c = pt(&mut sketch, 0.0, 0.0);
        let e = sketch.add_geometry(GeometryElement::Ellipse(Ellipse::new(
            c,
            Vec2D::new(10.0, 0.0),
            0.5,
        )));
        let a = pt(&mut sketch, -20.0, 0.0);
        let b = pt(&mut sketch, 20.0, 0.0);
        let l = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let ellipse = Curve::of(&sketch, sketch.get_geometry(e).unwrap()).unwrap();
        let line = Curve::of(&sketch, sketch.get_geometry(l).unwrap()).unwrap();
        let hits = crossings(&ellipse, ellipse.span, &line, line.span);
        assert_eq!(hits.len(), 2);
        for (te, tl) in hits {
            let p = ellipse.at(te);
            assert!((p - line.at(tl)).length() < 1e-9);
            assert!((p.x.abs() - 10.0).abs() < 1e-9 && p.y.abs() < 1e-9, "{p:?}");
        }
    }

    #[test]
    fn the_nearest_point_of_an_ellipse() {
        let mut sketch = Sketch::new("t");
        let c = pt(&mut sketch, 0.0, 0.0);
        let e = sketch.add_geometry(GeometryElement::Ellipse(Ellipse::new(
            c,
            Vec2D::new(10.0, 0.0),
            0.5,
        )));
        let ellipse = Curve::of(&sketch, sketch.get_geometry(e).unwrap()).unwrap();
        let t = ellipse.nearest(DVec2::new(0.0, 9.0), ellipse.span);
        assert!((ellipse.at(t) - DVec2::new(0.0, 5.0)).length() < 1e-6);
    }
}
