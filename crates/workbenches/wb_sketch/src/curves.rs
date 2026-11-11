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
    DVec2::new(p.x, p.y)
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
        Self::of_with(&|id| sketch.point_position(id), geom)
    }

    /// The curve of `geom` with its points looked up by `point`; `None` for
    /// a point, a degenerate curve or a point `point` does not find.
    pub fn of_with(
        point: &dyn Fn(uuid::Uuid) -> Option<crate::sketch::Vec2D>,
        geom: &GeometryElement,
    ) -> Option<Self> {
        let pos = |id| point(id).map(dv);
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
                    r: c.radius,
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
                let (t0, t1) = e.param_span_with(point)?;
                Self {
                    carrier: Carrier::Ellipse {
                        c: pos(e.center)?,
                        major,
                        minor: major.perp() * e.ratio,
                    },
                    span: (t0, t1),
                    period: tau,
                    closed: e.arc.is_none(),
                }
            }
            GeometryElement::Conic(k) => {
                let (shape, t0, t1) = k.params_with(point)?;
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

    /// A circle the element lies within, as its centre and radius: `None`
    /// for a parabola or a hyperbola, which have no cheap one.
    pub fn bound(&self) -> Option<(DVec2, f64)> {
        match &self.carrier {
            Carrier::Line { a, b } => Some(((*a + *b) * 0.5, (*b - *a).length() * 0.5)),
            Carrier::Circle { c, r } => Some((*c, r.abs())),
            Carrier::Ellipse { c, major, minor } => Some((*c, major.length().max(minor.length()))),
            Carrier::Conic(_) => None,
            // A spline keeps within its control polygon's hull.
            Carrier::Spline { control, .. } => {
                let (lo, hi) = control.iter().fold(
                    (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
                    |(lo, hi), p| (lo.min(DVec2::from(*p)), hi.max(DVec2::from(*p))),
                );
                Some(((lo + hi) * 0.5, (hi - lo).length() * 0.5))
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

    /// Whether the point at `t` lies on the curve over `span`: inside it,
    /// or past one of its ends by no more than [`END_SLACK`].
    pub fn holds(&self, t: f64, span: (f64, f64)) -> bool {
        let slack = 1e-9 * (span.1 - span.0).abs().max(1.0);
        if t >= span.0 - slack && t <= span.1 + slack {
            return true;
        }
        let end = if t < span.0 { span.0 } else { span.1 };
        (self.at(t) - self.at(end)).length() <= END_SLACK
    }

    /// How many straight pieces follow the curve closely over `span`.
    fn pieces(&self, span: (f64, f64)) -> usize {
        // Capped, so a span of numbers out of range samples a bounded count.
        let part = |per_turn: f64, turn: f64| {
            ((span.1 - span.0).abs() / turn * per_turn)
                .ceil()
                .min(MAX_PIECES) as usize
                + 4
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
                let t = sample_at(span, i, n);
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
        // The nearest of the samples, worked out one at a time: this runs
        // for every curve under the cursor on every pointer move.
        let n = self.pieces(span);
        let dist = |t: f64| (self.at(t) - p).length_squared();
        let best = (0..=n)
            .min_by(|i, j| dist(sample_at(span, *i, n)).total_cmp(&dist(sample_at(span, *j, n))))
            .unwrap_or(0);
        let (mut lo, mut hi) = (
            sample_at(span, best.saturating_sub(1), n),
            sample_at(span, (best + 1).min(n), n),
        );
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

/// The most straight pieces a span is sampled in.
const MAX_PIECES: f64 = 65536.0;

/// The parameter of sample `i` of `n` along `span`.
fn sample_at(span: (f64, f64), i: usize, n: usize) -> f64 {
    span.0 + (span.1 - span.0) * i as f64 / n as f64
}

/// How far past a curve's end, in sketch units (mm), a crossing still
/// lies on it: trim, extend, the crossing snap and the regions all count a
/// crossing within this of an end, and the regions join two points this
/// near into one vertex. It is the sketch's own join: projected ends and
/// a drawing's ends join within it too. It stays within the kernel's
/// [`kernel_api::JOIN_MM`], so every outline the sketch closes the kernel
/// builds; it is no wider, so a saved sketch keeps the regions it has.
pub const END_SLACK: f64 = 1e-4;
const _: () = assert!(END_SLACK <= kernel_api::JOIN_MM);

/// Where the segments `p0 → p1` and `q0 → q1` cross, as fractions along
/// each, counting a crossing up to `reach` (sketch units) past either
/// segment's ends. Segments parallel to within rounding cross nowhere.
pub(crate) fn segments_cross(
    p0: DVec2,
    p1: DVec2,
    q0: DVec2,
    q1: DVec2,
    reach: f64,
) -> Option<(f64, f64)> {
    let (d, e) = (p1 - p0, q1 - q0);
    let denom = d.perp_dot(e);
    let scale = (d.length() * e.length()).max(1e-300);
    if denom.abs() < 1e-12 * scale {
        return None;
    }
    let w = q0 - p0;
    let s = w.perp_dot(e) / denom;
    let u = w.perp_dot(d) / denom;
    let slack = |len: f64| (reach / len.max(1e-300)).max(1e-9);
    let (ss, su) = (slack(d.length()), slack(e.length()));
    ((-ss..=1.0 + ss).contains(&s) && (-su..=1.0 + su).contains(&u)).then_some((s, u))
}

/// A curve by its parameter, as the crossing search walks it.
pub(crate) trait Path {
    /// The point at `t`.
    fn at(&self, t: f64) -> DVec2;
    /// The step a derivative is taken over, by central difference.
    fn step(&self) -> f64;
    /// `t` brought back where the curve takes it after a Newton step.
    fn settle(&self, t: f64) -> f64 {
        t
    }
}

impl Path for Curve {
    fn at(&self, t: f64) -> DVec2 {
        Curve::at(self, t)
    }

    fn step(&self) -> f64 {
        1e-7 * (self.span.1 - self.span.0).abs().max(1.0)
    }
}

/// Newton's steps from a sampled crossing of `a` and `b` to the exact
/// one: the parameters on each, when they meet within a millionth of a
/// millimetre.
pub(crate) fn refine(a: &impl Path, b: &impl Path, mut ta: f64, mut tb: f64) -> Option<(f64, f64)> {
    let (ha, hb) = (a.step(), b.step());
    for _ in 0..40 {
        let f = a.at(ta) - b.at(tb);
        if f.length() < 1e-12 {
            break;
        }
        let da = (a.at(ta + ha) - a.at(ta - ha)) / (2.0 * ha);
        let db = -(b.at(tb + hb) - b.at(tb - hb)) / (2.0 * hb);
        let det = da.perp_dot(db);
        if det.abs() <= 1e-12 * (da.length() * db.length()).max(1e-300) {
            break;
        }
        // Solve [da db] · (x, y) = -f.
        let x = (-f).perp_dot(db) / det;
        let y = da.perp_dot(-f) / det;
        ta = a.settle(ta + x);
        tb = b.settle(tb + y);
    }
    ((a.at(ta) - b.at(tb)).length() < 1e-6).then_some((ta, tb))
}

/// Where `a` over `sa` crosses `b` over `sb`: the parameters on each, a
/// crossing up to [`END_SLACK`] past an end of either span counting.
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
            let tol = END_SLACK;
            if alo.x > bhi.x + tol
                || blo.x > ahi.x + tol
                || alo.y > bhi.y + tol
                || blo.y > ahi.y + tol
            {
                continue;
            }
            let Some((s, u)) = segments_cross(wa[0].1, wa[1].1, wb[0].1, wb[1].1, END_SLACK) else {
                continue;
            };
            let ta = wa[0].0 + (wa[1].0 - wa[0].0) * s;
            let tb = wb[0].0 + (wb[1].0 - wb[0].0) * u;
            let Some((ta, tb)) = refine(a, b, ta, tb) else {
                continue;
            };
            if !a.holds(ta, sa) || !b.holds(tb, sb) {
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

/// Where `c` over `span` crosses itself (a spline's loop): both parameters
/// of each crossing, the earlier first. Neighbouring sample pieces meet at
/// their shared sample and are skipped, as are the first and last pieces of
/// a whole turn of a closed curve, which meet through its seam.
pub fn self_crossings(c: &Curve, span: (f64, f64)) -> Vec<(f64, f64)> {
    let ps = c.samples(span);
    let pieces = ps.len() - 1;
    let whole_turn = c.period > 0.0 && (span.1 - span.0) >= c.period * (1.0 - 1e-12);
    let apart = 1e-6 * (span.1 - span.0).abs().max(1e-9);
    let mut out: Vec<(f64, f64)> = Vec::new();
    for i in 0..pieces {
        for j in i + 2..pieces {
            if whole_turn && i == 0 && j == pieces - 1 {
                continue;
            }
            let (wa, wb) = (&ps[i..i + 2], &ps[j..j + 2]);
            let Some((s, u)) = segments_cross(wa[0].1, wa[1].1, wb[0].1, wb[1].1, 0.0) else {
                continue;
            };
            let ta = wa[0].0 + (wa[1].0 - wa[0].0) * s;
            let tb = wb[0].0 + (wb[1].0 - wb[0].0) * u;
            let Some((ta, tb)) = refine(c, c, ta, tb) else {
                continue;
            };
            // Newton may slide both parameters onto one point of the curve:
            // that is no crossing.
            let mut gap = (tb - ta).abs();
            if c.period > 0.0 {
                gap = gap.rem_euclid(c.period);
                gap = gap.min(c.period - gap);
            }
            let inside = |t: f64| t >= span.0 - apart && t <= span.1 + apart;
            if gap <= apart || !inside(ta) || !inside(tb) {
                continue;
            }
            let p = c.at(ta);
            if out.iter().all(|(t, _)| (c.at(*t) - p).length() > 1e-7) {
                out.push((ta.min(tb), ta.max(tb)));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Ellipse, Line, Point, Vec2D};

    fn pt(sketch: &mut Sketch, x: f64, y: f64) -> uuid::Uuid {
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
