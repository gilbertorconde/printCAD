//! The sketch's regions: the bounded faces of the planar arrangement its
//! profile curves make once each is split where it crosses, touches or
//! branches off another (or itself).
//!
//! Curves that cross without sharing a point, three or more curves meeting
//! at one point and spurs all leave regions behind: the arrangement finds
//! every bounded face of the graph the split pieces make, pruning the
//! pieces that end loose. Each region carries a name made from the sketch
//! elements bounding it and the side of each it lies on, so it keeps its
//! name while dimensions change. A profile is then the outline of chosen
//! regions: neighbours sharing a piece merge into one face, the pieces
//! between them left out.

use std::collections::HashMap;

use kernel_api::{ProfileSegment, ProfileWire, TopoName};

/// How near two points are one vertex, in sketch units (mm): curves meet
/// where the sketch's points put them (a document saved in single
/// precision a few millionths of a millimetre apart), and projected edges
/// within a solid's tolerance.
const JOIN: f64 = 1e-4;

/// A closed region of the sketch's curves.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    /// Its name, from the elements around it and the side of each it lies
    /// on: the same while the sketch's dimensions change.
    pub name: String,
    /// Its area, holes taken out, in mm².
    pub area: f64,
    /// A point inside it, out of its holes.
    pub inside: [f64; 2],
    /// Its outer boundary, then each hole, as polygons on the sketch plane
    /// (first point not repeated).
    pub outline: Vec<Vec<[f64; 2]>>,
    /// Whether a profile with no region chosen takes it: regions nested an
    /// even number of times deep, as loops inside loops alternate between
    /// material and hole.
    pub default: bool,
}

/// One curve of the arrangement, by its own parameter.
#[derive(Debug, Clone)]
struct Curve {
    carrier: Carrier,
    /// The segment it was drawn as, kept for a piece that is all of it.
    segment: ProfileSegment,
    name: TopoName,
    lo: f64,
    hi: f64,
    closed: bool,
}

#[derive(Debug, Clone)]
enum Carrier {
    Line {
        a: [f64; 2],
        b: [f64; 2],
    },
    /// A circle or an arc of one: the angle `start + sweep * t`.
    Round {
        c: [f64; 2],
        r: f64,
        start: f64,
        sweep: f64,
    },
    /// An ellipse or an arc of one: the ellipse parameter `t0 + (t1 - t0) * t`.
    Oval {
        c: [f64; 2],
        major: [f64; 2],
        ratio: f64,
        t0: f64,
        t1: f64,
    },
    Spline(Spline),
}

/// A B-spline of any degree over its knots, rational through homogeneous
/// control points. A periodic one is held unclamped over its control
/// polygon wrapped twice, so a piece running over its seam is one span of
/// parameter.
#[derive(Debug, Clone)]
struct Spline {
    degree: usize,
    knots: Vec<f64>,
    /// `(w·x, w·y, w)` per control point.
    points: Vec<[f64; 3]>,
    rational: bool,
}

impl Spline {
    fn span(&self, u: f64) -> usize {
        let n = self.points.len();
        let p = self.degree;
        let mut k = p;
        while k + 1 < n && self.knots[k + 1] <= u {
            k += 1;
        }
        k
    }

    fn eval(&self, u: f64) -> [f64; 2] {
        let p = self.degree;
        let k = self.span(u);
        let mut d: Vec<[f64; 3]> = (0..=p).map(|j| self.points[j + k - p]).collect();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = j + k - p;
                let denom = self.knots[i + p - r + 1] - self.knots[i];
                let a = if denom.abs() < 1e-300 {
                    0.0
                } else {
                    (u - self.knots[i]) / denom
                };
                let before = d[j - 1];
                d[j] = [0, 1, 2].map(|c| (1.0 - a) * before[c] + a * d[j][c]);
            }
        }
        let w = d[p][2];
        [d[p][0] / w, d[p][1] / w]
    }

    fn multiplicity(&self, u: f64) -> usize {
        self.knots.iter().filter(|&&k| k == u).count()
    }

    /// Insert knot `u` once, the curve unchanged.
    fn insert(&mut self, u: f64) {
        let p = self.degree;
        let n = self.points.len();
        let mut k = 0;
        while k + 1 < self.knots.len() && self.knots[k + 1] <= u {
            k += 1;
        }
        let k = k.clamp(p, n);
        let s = self.multiplicity(u);
        let mut points = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let q = if i + p <= k {
                self.points[i]
            } else if i + s <= k {
                let a = (u - self.knots[i]) / (self.knots[i + p] - self.knots[i]);
                let (before, here) = (self.points[i - 1], self.points[i]);
                [0, 1, 2].map(|c| (1.0 - a) * before[c] + a * here[c])
            } else {
                self.points[i - 1]
            };
            points.push(q);
        }
        self.points = points;
        self.knots.insert(k + 1, u);
    }

    /// The part of the curve over `[u0, u1]`, clamped at both ends.
    fn part(&self, u0: f64, u1: f64) -> Spline {
        let p = self.degree;
        let mut s = self.clone();
        while s.multiplicity(u0) < p {
            s.insert(u0);
        }
        while s.multiplicity(u1) < p {
            s.insert(u1);
        }
        let first = |u: f64| s.knots.iter().position(|&k| k == u).unwrap_or(0);
        let (s0, s1) = (first(u0), first(u1));
        let m0 = s.multiplicity(u0);
        let i0 = s0 + m0 - 1 - p;
        let points = s.points[i0..s1].to_vec();
        let mut knots = vec![u0; p + 1];
        knots.extend_from_slice(&s.knots[s0 + m0..s1]);
        knots.extend(std::iter::repeat_n(u1, p + 1));
        Spline {
            degree: p,
            knots,
            points,
            rational: self.rational,
        }
    }
}

/// The uniform knots `0, 1, 2, …` of an unclamped spline over `wrapped`
/// control points, the polygon of a periodic one wrapped twice with
/// `degree` more, so its domain `[degree, n + degree]` runs on for a
/// second turn.
fn periodic_spline(control: &[[f64; 2]], degree: usize) -> Spline {
    let n = control.len();
    let mut points: Vec<[f64; 3]> = Vec::with_capacity(2 * n + degree);
    for i in 0..2 * n + degree {
        let [x, y] = control[i % n];
        points.push([x, y, 1.0]);
    }
    let knots = (0..points.len() + degree + 1).map(|i| i as f64).collect();
    Spline {
        degree,
        knots,
        points,
        rational: false,
    }
}

fn clamped_uniform(degree: usize, n: usize) -> Vec<f64> {
    let interior = n - degree - 1;
    let mut knots = vec![0.0; degree + 1];
    for i in 1..=interior {
        knots.push(i as f64 / (interior + 1) as f64);
    }
    knots.extend(std::iter::repeat_n(1.0, degree + 1));
    knots
}

fn circumcircle(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<([f64; 2], f64)> {
    let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
    if d.abs() < 1e-14 {
        return None;
    }
    let (a2, b2, c2) = (
        a[0] * a[0] + a[1] * a[1],
        b[0] * b[0] + b[1] * b[1],
        c[0] * c[0] + c[1] * c[1],
    );
    let center = [
        (a2 * (b[1] - c[1]) + b2 * (c[1] - a[1]) + c2 * (a[1] - b[1])) / d,
        (a2 * (c[0] - b[0]) + b2 * (a[0] - c[0]) + c2 * (b[0] - a[0])) / d,
    ];
    Some((center, dist(center, a)))
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

impl Curve {
    fn of(segment: &ProfileSegment, name: TopoName) -> Option<Curve> {
        use std::f64::consts::TAU;
        let curve = |carrier, lo, hi, closed| {
            Some(Curve {
                carrier,
                segment: segment.clone(),
                name,
                lo,
                hi,
                closed,
            })
        };
        match segment {
            ProfileSegment::Line { start, end } => {
                curve(Carrier::Line { a: *start, b: *end }, 0.0, 1.0, false)
            }
            ProfileSegment::Arc { start, mid, end } => {
                let (c, r) = circumcircle(*start, *mid, *end)?;
                let angle = |p: [f64; 2]| (p[1] - c[1]).atan2(p[0] - c[0]);
                let (a0, am, a1) = (angle(*start), angle(*mid), angle(*end));
                let ccw = (a1 - a0).rem_euclid(TAU);
                let sweep = if (am - a0).rem_euclid(TAU) <= ccw {
                    ccw
                } else {
                    ccw - TAU
                };
                curve(
                    Carrier::Round {
                        c,
                        r,
                        start: a0,
                        sweep,
                    },
                    0.0,
                    1.0,
                    false,
                )
            }
            ProfileSegment::Circle { center, radius } => curve(
                Carrier::Round {
                    c: *center,
                    r: *radius,
                    start: 0.0,
                    sweep: TAU,
                },
                0.0,
                1.0,
                true,
            ),
            ProfileSegment::Ellipse {
                center,
                major,
                ratio,
            } => curve(
                Carrier::Oval {
                    c: *center,
                    major: *major,
                    ratio: *ratio,
                    t0: 0.0,
                    t1: TAU,
                },
                0.0,
                1.0,
                true,
            ),
            ProfileSegment::EllipseArc {
                center,
                major,
                ratio,
                start_param,
                end_param,
            } => {
                let mut t1 = *end_param;
                while t1 <= *start_param {
                    t1 += TAU;
                }
                curve(
                    Carrier::Oval {
                        c: *center,
                        major: *major,
                        ratio: *ratio,
                        t0: *start_param,
                        t1,
                    },
                    0.0,
                    1.0,
                    false,
                )
            }
            ProfileSegment::BSpline {
                control_points,
                periodic,
            } => {
                let n = control_points.len();
                if n < 2 || (*periodic && n < 3) {
                    return None;
                }
                let degree = 3usize.min(n - 1).max(1);
                if *periodic {
                    let spline = periodic_spline(control_points, degree);
                    curve(
                        Carrier::Spline(spline),
                        degree as f64,
                        (n + degree) as f64,
                        true,
                    )
                } else {
                    let spline = Spline {
                        degree,
                        knots: clamped_uniform(degree, n),
                        points: control_points.iter().map(|p| [p[0], p[1], 1.0]).collect(),
                        rational: false,
                    };
                    curve(Carrier::Spline(spline), 0.0, 1.0, false)
                }
            }
            ProfileSegment::Nurbs {
                degree,
                knots,
                control_points,
                weights,
                periodic,
            } => {
                let n = control_points.len();
                let degree = *degree as usize;
                if degree == 0 || n <= degree {
                    return None;
                }
                if *periodic {
                    let spline = periodic_spline(control_points, degree);
                    return curve(
                        Carrier::Spline(spline),
                        degree as f64,
                        (n + degree) as f64,
                        true,
                    );
                }
                if knots.len() != n + degree + 1 || (!weights.is_empty() && weights.len() != n) {
                    return None;
                }
                let points = control_points
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let w = weights.get(i).copied().unwrap_or(1.0);
                        [p[0] * w, p[1] * w, w]
                    })
                    .collect();
                let spline = Spline {
                    degree,
                    knots: knots.clone(),
                    points,
                    rational: !weights.is_empty(),
                };
                let (lo, hi) = (knots[degree], knots[n]);
                curve(Carrier::Spline(spline), lo, hi, false)
            }
        }
    }

    /// Whether `t0` and `t1` are one place along the curve: the curve
    /// between them goes nowhere. A curve crossing itself passes one point
    /// twice, at two places.
    fn same_place(&self, t0: f64, t1: f64) -> bool {
        let (a, b) = (self.at(t0), self.at(t1));
        dist(a, b) < JOIN && dist(self.at((t0 + t1) * 0.5), a) < JOIN
    }

    fn period(&self) -> f64 {
        self.hi - self.lo
    }

    /// The parameter taken back into the curve's first turn, for a closed
    /// one.
    fn wrap(&self, t: f64) -> f64 {
        if self.closed {
            self.lo + (t - self.lo).rem_euclid(self.period())
        } else {
            t.clamp(self.lo, self.hi)
        }
    }

    fn at(&self, t: f64) -> [f64; 2] {
        match &self.carrier {
            Carrier::Line { a, b } => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
            Carrier::Round { c, r, start, sweep } => {
                let angle = start + sweep * t;
                [c[0] + r * angle.cos(), c[1] + r * angle.sin()]
            }
            Carrier::Oval {
                c,
                major,
                ratio,
                t0,
                t1,
            } => {
                let u = t0 + (t1 - t0) * t;
                let minor = [-major[1] * ratio, major[0] * ratio];
                let (sin, cos) = u.sin_cos();
                [
                    c[0] + major[0] * cos + minor[0] * sin,
                    c[1] + major[1] * cos + minor[1] * sin,
                ]
            }
            Carrier::Spline(s) => s.eval(t),
        }
    }

    /// The derivative, by central difference.
    fn tangent(&self, t: f64) -> [f64; 2] {
        let h = 1e-7 * self.period().max(1e-9);
        let (a, b) = (self.at(t - h), self.at(t + h));
        [(b[0] - a[0]) / (2.0 * h), (b[1] - a[1]) / (2.0 * h)]
    }

    /// Samples over `[t0, t1]`, both ends included.
    fn samples(&self, t0: f64, t1: f64) -> Vec<[f64; 2]> {
        let count = match &self.carrier {
            Carrier::Line { .. } => 1,
            Carrier::Round { sweep, .. } => {
                let turn = (sweep * (t1 - t0)).abs() / std::f64::consts::TAU;
                ((96.0 * turn).ceil() as usize).max(4)
            }
            Carrier::Oval { t0: a, t1: b, .. } => {
                let turn = ((b - a) * (t1 - t0)).abs() / std::f64::consts::TAU;
                ((96.0 * turn).ceil() as usize).max(4)
            }
            Carrier::Spline(s) => {
                let spans = s
                    .knots
                    .windows(2)
                    .filter(|w| w[1] > w[0] && w[1] > t0 && w[0] < t1)
                    .count();
                (spans * 12).clamp(12, 600)
            }
        };
        (0..=count)
            .map(|i| self.at(t0 + (t1 - t0) * i as f64 / count as f64))
            .collect()
    }
}

/// A part of one curve between two vertices.
#[derive(Debug, Clone, Copy)]
struct Piece {
    curve: usize,
    t0: f64,
    t1: f64,
    v0: usize,
    v1: usize,
}

/// The curves split and joined into a planar graph, with its faces.
struct Arrangement {
    curves: Vec<Curve>,
    vertices: Vec<[f64; 2]>,
    pieces: Vec<Piece>,
    /// Per half-edge (`2 * piece` along the curve, `2 * piece + 1`
    /// against it): the next half-edge round the face on its left.
    next: Vec<usize>,
    /// Per half-edge: its cycle.
    cycle_of: Vec<usize>,
    cycles: Vec<Cycle>,
    /// Per cycle: the face it bounds, outer or hole; `None` outside every
    /// face.
    face_of: Vec<Option<usize>>,
    /// The bounded faces: the cycle round each, its holes, its depth.
    faces: Vec<Face>,
    /// Whether any vertex joins more than two pieces: curves crossing,
    /// touching or branching.
    branches: bool,
}

struct Cycle {
    edges: Vec<usize>,
    area: f64,
    polygon: Vec<[f64; 2]>,
}

struct Face {
    outer: usize,
    holes: Vec<usize>,
    depth: usize,
}

/// Where segment `p0→p1` crosses `q0→q1`, as their parameters.
fn segment_hit(p0: [f64; 2], p1: [f64; 2], q0: [f64; 2], q1: [f64; 2]) -> Option<(f64, f64)> {
    let r = [p1[0] - p0[0], p1[1] - p0[1]];
    let s = [q1[0] - q0[0], q1[1] - q0[1]];
    let denom = r[0] * s[1] - r[1] * s[0];
    let scale = (r[0].hypot(r[1]) * s[0].hypot(s[1])).max(1e-300);
    if denom.abs() < 1e-12 * scale {
        return None;
    }
    let qp = [q0[0] - p0[0], q0[1] - p0[1]];
    let t = (qp[0] * s[1] - qp[1] * s[0]) / denom;
    let u = (qp[0] * r[1] - qp[1] * r[0]) / denom;
    let slack = 1e-9;
    ((-slack..=1.0 + slack).contains(&t) && (-slack..=1.0 + slack).contains(&u)).then_some((t, u))
}

/// The parameters where curves `a` and `b` meet, refined by Newton's
/// method from `(t, s)`.
fn refine_hit(a: &Curve, b: &Curve, mut t: f64, mut s: f64) -> Option<(f64, f64)> {
    for _ in 0..40 {
        let (pa, pb) = (a.at(t), b.at(s));
        let f = [pa[0] - pb[0], pa[1] - pb[1]];
        if f[0].hypot(f[1]) < 1e-12 {
            break;
        }
        let (da, db) = (a.tangent(t), b.tangent(s));
        // [da, -db] [dt, ds]ᵀ = -f
        let det = da[0] * -db[1] - (-db[0]) * da[1];
        if det.abs() < 1e-18 {
            return None;
        }
        let dt = (-f[0] * -db[1] - (-db[0]) * -f[1]) / det;
        let ds = (da[0] * -f[1] - (-f[0]) * da[1]) / det;
        t = a.wrap(t + dt);
        s = b.wrap(s + ds);
    }
    (dist(a.at(t), b.at(s)) < JOIN * 0.01).then_some((t, s))
}

/// Where on curve `c` point `p` lies, when it lies on it.
fn project(c: &Curve, samples: &[(f64, [f64; 2])], p: [f64; 2]) -> Option<f64> {
    // The nearest sample segment, then Newton's method on the distance.
    let mut best = (f64::INFINITY, c.lo);
    for w in samples.windows(2) {
        let ((ta, a), (tb, b)) = (w[0], w[1]);
        let ab = [b[0] - a[0], b[1] - a[1]];
        let len2 = ab[0] * ab[0] + ab[1] * ab[1];
        let f = if len2 > 0.0 {
            (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let q = [a[0] + ab[0] * f, a[1] + ab[1] * f];
        let d = dist(p, q);
        if d < best.0 {
            best = (d, ta + (tb - ta) * f);
        }
    }
    if best.0 > JOIN * 100.0 + 1e-3 {
        return None;
    }
    let mut t = best.1;
    for _ in 0..30 {
        let q = c.at(t);
        let d = c.tangent(t);
        let len2 = d[0] * d[0] + d[1] * d[1];
        if len2 < 1e-24 {
            break;
        }
        let step = ((p[0] - q[0]) * d[0] + (p[1] - q[1]) * d[1]) / len2;
        t = c.wrap(t + step);
        if step.abs() < 1e-14 * c.period().max(1.0) {
            break;
        }
    }
    (dist(c.at(t), p) < JOIN).then_some(t)
}

fn shoelace(polygon: &[[f64; 2]]) -> f64 {
    let n = polygon.len();
    (0..n)
        .map(|i| {
            let (a, b) = (polygon[i], polygon[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        * 0.5
}

/// Even-odd: whether `p` lies inside the polygons taken together.
fn inside(polygons: &[&[[f64; 2]]], p: [f64; 2]) -> bool {
    let mut odd = false;
    for polygon in polygons {
        let n = polygon.len();
        for i in 0..n {
            let (a, b) = (polygon[i], polygon[(i + 1) % n]);
            if (a[1] > p[1]) != (b[1] > p[1]) {
                let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if x > p[0] {
                    odd = !odd;
                }
            }
        }
    }
    odd
}

/// A point well inside the polygons taken together: the middle of the
/// widest run a few horizontal lines across them find inside.
fn point_inside(polygons: &[&[[f64; 2]]]) -> [f64; 2] {
    let (mut y_min, mut y_max) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in polygons.iter().flat_map(|p| p.iter()) {
        y_min = y_min.min(p[1]);
        y_max = y_max.max(p[1]);
    }
    let mut best = (f64::NEG_INFINITY, polygons[0][0]);
    for fraction in [0.5, 0.37, 0.63, 0.25, 0.75, 0.13, 0.87, 0.05, 0.95] {
        let y = y_min + (y_max - y_min) * fraction;
        let mut xs: Vec<f64> = Vec::new();
        for polygon in polygons {
            let n = polygon.len();
            for i in 0..n {
                let (a, b) = (polygon[i], polygon[(i + 1) % n]);
                if (a[1] > y) != (b[1] > y) {
                    xs.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                }
            }
        }
        xs.sort_by(f64::total_cmp);
        for pair in xs.as_chunks::<2>().0 {
            let width = pair[1] - pair[0];
            if width > best.0 {
                best = (width, [(pair[0] + pair[1]) * 0.5, y]);
            }
        }
    }
    best.1
}

impl Arrangement {
    fn build(curves: Vec<Curve>) -> Arrangement {
        let samples: Vec<Vec<(f64, [f64; 2])>> = curves
            .iter()
            .map(|c| {
                let points = c.samples(c.lo, c.hi);
                let n = points.len() - 1;
                points
                    .into_iter()
                    .enumerate()
                    .map(|(i, p)| (c.lo + (c.hi - c.lo) * i as f64 / n as f64, p))
                    .collect()
            })
            .collect();
        let bounds: Vec<[f64; 4]> = samples
            .iter()
            .map(|s| {
                s.iter().fold(
                    [
                        f64::INFINITY,
                        f64::INFINITY,
                        f64::NEG_INFINITY,
                        f64::NEG_INFINITY,
                    ],
                    |b, (_, p)| {
                        [
                            b[0].min(p[0]),
                            b[1].min(p[1]),
                            b[2].max(p[0]),
                            b[3].max(p[1]),
                        ]
                    },
                )
            })
            .collect();
        let overlap = |a: &[f64; 4], b: &[f64; 4]| {
            a[0] <= b[2] + JOIN && b[0] <= a[2] + JOIN && a[1] <= b[3] + JOIN && b[1] <= a[3] + JOIN
        };

        // Where each curve is split: crossings, its own included, and the
        // ends of other curves lying on it.
        let mut splits: Vec<Vec<f64>> = vec![Vec::new(); curves.len()];
        // Pairs swept along x: a curve meets only those starting before it
        // ends.
        let mut order: Vec<usize> = (0..curves.len()).collect();
        order.sort_by(|&a, &b| bounds[a][0].total_cmp(&bounds[b][0]));
        let mut pairs: Vec<(usize, usize)> = Vec::new();
        for (k, &i) in order.iter().enumerate() {
            // Only a spline crosses itself.
            if matches!(curves[i].carrier, Carrier::Spline(_)) {
                pairs.push((i, i));
            }
            for &j in &order[k + 1..] {
                if bounds[j][0] > bounds[i][2] + JOIN {
                    break;
                }
                if overlap(&bounds[i], &bounds[j]) {
                    pairs.push((i.min(j), i.max(j)));
                }
            }
        }
        for (i, j) in pairs {
            {
                let (si, sj) = (&samples[i], &samples[j]);
                for a in 0..si.len() - 1 {
                    let ba = [
                        si[a].1[0].min(si[a + 1].1[0]),
                        si[a].1[1].min(si[a + 1].1[1]),
                        si[a].1[0].max(si[a + 1].1[0]),
                        si[a].1[1].max(si[a + 1].1[1]),
                    ];
                    let from = if i == j { a + 2 } else { 0 };
                    for b in from..sj.len() - 1 {
                        if i == j && curves[i].closed && a == 0 && b == sj.len() - 2 {
                            continue;
                        }
                        let bb = [
                            sj[b].1[0].min(sj[b + 1].1[0]),
                            sj[b].1[1].min(sj[b + 1].1[1]),
                            sj[b].1[0].max(sj[b + 1].1[0]),
                            sj[b].1[1].max(sj[b + 1].1[1]),
                        ];
                        if !overlap(&ba, &bb) {
                            continue;
                        }
                        let Some((u, v)) = segment_hit(si[a].1, si[a + 1].1, sj[b].1, sj[b + 1].1)
                        else {
                            continue;
                        };
                        let t = si[a].0 + (si[a + 1].0 - si[a].0) * u;
                        let s = sj[b].0 + (sj[b + 1].0 - sj[b].0) * v;
                        // Curves joined end to end meet there and split
                        // nothing.
                        let at = [
                            si[a].1[0] + (si[a + 1].1[0] - si[a].1[0]) * u,
                            si[a].1[1] + (si[a + 1].1[1] - si[a].1[1]) * u,
                        ];
                        let end_of = |samples: &[(f64, [f64; 2])], closed: bool| {
                            !closed
                                && (dist(at, samples[0].1) < JOIN
                                    || dist(at, samples[samples.len() - 1].1) < JOIN)
                        };
                        if end_of(si, curves[i].closed) && end_of(sj, curves[j].closed) {
                            continue;
                        }
                        let Some((t, s)) = refine_hit(&curves[i], &curves[j], t, s) else {
                            continue;
                        };
                        if i == j && dist(curves[i].at(t), curves[i].at(s)) < JOIN {
                            // A self-crossing needs two places on the curve.
                            if (t - s).abs() < 1e-6 * curves[i].period() {
                                continue;
                            }
                        }
                        splits[i].push(t);
                        splits[j].push(s);
                    }
                }
            }
        }
        for i in 0..curves.len() {
            if curves[i].closed {
                continue;
            }
            for end in [curves[i].at(curves[i].lo), curves[i].at(curves[i].hi)] {
                for (j, curve) in curves.iter().enumerate() {
                    let b = &bounds[j];
                    if end[0] < b[0] - JOIN
                        || end[0] > b[2] + JOIN
                        || end[1] < b[1] - JOIN
                        || end[1] > b[3] + JOIN
                    {
                        continue;
                    }
                    // An end meeting another's end joins it as a vertex,
                    // splitting nothing.
                    let at_its_end = !curve.closed
                        && (dist(end, samples[j][0].1) < JOIN
                            || dist(end, samples[j][samples[j].len() - 1].1) < JOIN);
                    if at_its_end {
                        continue;
                    }
                    if let Some(t) = project(curve, &samples[j], end) {
                        splits[j].push(t);
                    }
                }
            }
        }

        // Vertices: the curves' own ends first, so ends sharing a point
        // land exactly where it is.
        let mut vertices: Vec<[f64; 2]> = Vec::new();
        let vertex_of = |p: [f64; 2], vertices: &mut Vec<[f64; 2]>| -> usize {
            if let Some(i) = vertices.iter().position(|v| dist(*v, p) < JOIN) {
                return i;
            }
            vertices.push(p);
            vertices.len() - 1
        };
        for c in curves.iter().filter(|c| !c.closed) {
            vertex_of(c.at(c.lo), &mut vertices);
            vertex_of(c.at(c.hi), &mut vertices);
        }

        let mut pieces: Vec<Piece> = Vec::new();
        for (index, c) in curves.iter().enumerate() {
            let (start, end) = (c.at(c.lo), c.at(c.hi));
            let mut cuts: Vec<f64> = splits[index]
                .iter()
                .map(|&t| c.wrap(t))
                .filter(|&t| {
                    c.closed || (dist(c.at(t), start) >= JOIN && dist(c.at(t), end) >= JOIN)
                })
                .collect();
            cuts.sort_by(f64::total_cmp);
            let mut kept: Vec<f64> = Vec::new();
            for t in cuts {
                if kept.last().is_none_or(|&last| !c.same_place(last, t)) {
                    kept.push(t);
                }
            }
            if c.closed
                && kept.len() > 1
                && c.same_place(*kept.last().expect("two cuts"), kept[0] + c.period())
            {
                kept.pop();
            }
            let mut spans: Vec<(f64, f64)> = Vec::new();
            if c.closed {
                if kept.is_empty() {
                    spans.push((c.lo, c.hi));
                } else {
                    for w in kept.windows(2) {
                        spans.push((w[0], w[1]));
                    }
                    let last = *kept.last().expect("a cut");
                    spans.push((last, kept[0] + c.period()));
                }
            } else {
                let mut at = c.lo;
                for t in kept {
                    spans.push((at, t));
                    at = t;
                }
                spans.push((at, c.hi));
            }
            for (t0, t1) in spans {
                let v0 = vertex_of(c.at(t0), &mut vertices);
                let v1 = vertex_of(c.at(t1), &mut vertices);
                // A piece from a point back to itself counts only when it
                // goes somewhere.
                if v0 == v1 && dist(c.at((t0 + t1) * 0.5), vertices[v0]) < JOIN {
                    continue;
                }
                pieces.push(Piece {
                    curve: index,
                    t0,
                    t1,
                    v0,
                    v1,
                });
            }
        }

        // A piece drawn twice (a line over another) is one piece.
        let mut unique: Vec<Piece> = Vec::new();
        for piece in pieces {
            let mid = curves[piece.curve].at((piece.t0 + piece.t1) * 0.5);
            let twin = unique.iter().any(|other| {
                let same_ends = (other.v0 == piece.v0 && other.v1 == piece.v1)
                    || (other.v0 == piece.v1 && other.v1 == piece.v0);
                same_ends && dist(curves[other.curve].at((other.t0 + other.t1) * 0.5), mid) < JOIN
            });
            if !twin {
                unique.push(piece);
            }
        }
        let mut pieces = unique;

        // Pieces with a loose end go, and whatever that leaves loose.
        loop {
            let mut degree = vec![0usize; vertices.len()];
            for p in &pieces {
                degree[p.v0] += 1;
                degree[p.v1] += 1;
            }
            let before = pieces.len();
            pieces.retain(|p| degree[p.v0] > 1 && degree[p.v1] > 1);
            if pieces.len() == before {
                break;
            }
        }
        let mut degree = vec![0usize; vertices.len()];
        for p in &pieces {
            degree[p.v0] += 1;
            degree[p.v1] += 1;
        }
        let branches = degree.iter().any(|&d| d > 2);

        let mut arrangement = Arrangement {
            curves,
            vertices,
            pieces,
            next: Vec::new(),
            cycle_of: Vec::new(),
            cycles: Vec::new(),
            face_of: Vec::new(),
            faces: Vec::new(),
            branches,
        };
        arrangement.link();
        arrangement.find_faces();
        arrangement
    }

    fn origin(&self, h: usize) -> usize {
        let p = &self.pieces[h / 2];
        if h.is_multiple_of(2) { p.v0 } else { p.v1 }
    }

    /// The half-edge's samples, in the way it runs.
    fn half_samples(&self, h: usize) -> Vec<[f64; 2]> {
        let p = &self.pieces[h / 2];
        let mut points = self.curves[p.curve].samples(p.t0, p.t1);
        if let Some(first) = points.first_mut() {
            *first = self.vertices[p.v0];
        }
        if let Some(last) = points.last_mut() {
            *last = self.vertices[p.v1];
        }
        if h % 2 == 1 {
            points.reverse();
        }
        points
    }

    /// Which way the half-edge leaves its origin: towards a point a little
    /// along it, so curves leaving along one tangent still part.
    fn heading(&self, h: usize) -> f64 {
        let p = &self.pieces[h / 2];
        let c = &self.curves[p.curve];
        let step = (p.t1 - p.t0) * 1e-3;
        let (from, to) = if h.is_multiple_of(2) {
            (self.vertices[p.v0], c.at(p.t0 + step))
        } else {
            (self.vertices[p.v1], c.at(p.t1 - step))
        };
        (to[1] - from[1]).atan2(to[0] - from[0])
    }

    /// Each half-edge's successor round the face on its left: at the
    /// vertex it reaches, the outgoing half-edge just clockwise of the way
    /// back.
    fn link(&mut self) {
        let half = self.pieces.len() * 2;
        let mut around: Vec<Vec<(f64, usize)>> = vec![Vec::new(); self.vertices.len()];
        for h in 0..half {
            around[self.origin(h)].push((self.heading(h), h));
        }
        let mut slot = vec![(0usize, 0usize); half];
        for (v, list) in around.iter_mut().enumerate() {
            list.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (i, (_, h)) in list.iter().enumerate() {
                slot[*h] = (v, i);
            }
        }
        self.next = (0..half)
            .map(|h| {
                let back = h ^ 1;
                let (v, i) = slot[back];
                let list = &around[v];
                list[(i + list.len() - 1) % list.len()].1
            })
            .collect();
    }

    fn find_faces(&mut self) {
        let half = self.pieces.len() * 2;
        self.cycle_of = vec![usize::MAX; half];
        for start in 0..half {
            if self.cycle_of[start] != usize::MAX {
                continue;
            }
            let id = self.cycles.len();
            let mut edges = Vec::new();
            let mut h = start;
            loop {
                self.cycle_of[h] = id;
                edges.push(h);
                h = self.next[h];
                if h == start || edges.len() > half {
                    break;
                }
            }
            let mut polygon = Vec::new();
            for &e in &edges {
                let mut points = self.half_samples(e);
                points.pop();
                polygon.extend(points);
            }
            let area = shoelace(&polygon);
            self.cycles.push(Cycle {
                edges,
                area,
                polygon,
            });
        }

        // The pieces' connected parts: each has one cycle round its
        // outside, the rest bound its faces.
        let mut part: Vec<usize> = (0..self.vertices.len()).collect();
        fn root(part: &mut [usize], mut v: usize) -> usize {
            while part[v] != v {
                part[v] = part[part[v]];
                v = part[v];
            }
            v
        }
        for p in &self.pieces {
            let (a, b) = (root(&mut part, p.v0), root(&mut part, p.v1));
            part[a] = b;
        }
        let mut outside: HashMap<usize, usize> = HashMap::new();
        let mut part_of_cycle = Vec::with_capacity(self.cycles.len());
        for (id, cycle) in self.cycles.iter().enumerate() {
            let group = root(&mut part, self.origin(cycle.edges[0]));
            part_of_cycle.push(group);
            let entry = outside.entry(group).or_insert(id);
            if cycle.area < self.cycles[*entry].area {
                *entry = id;
            }
        }
        let tiny = 1e-9;
        self.face_of = vec![None; self.cycles.len()];
        for (id, cycle) in self.cycles.iter().enumerate() {
            if outside.get(&part_of_cycle[id]) != Some(&id) && cycle.area > tiny {
                self.face_of[id] = Some(self.faces.len());
                self.faces.push(Face {
                    outer: id,
                    holes: Vec::new(),
                    depth: 0,
                });
            }
        }

        // Each part sits in the smallest face of another part around it,
        // or outside them all.
        let mut parent: HashMap<usize, usize> = HashMap::new();
        for (&group, &outer) in &outside {
            let probe = self.cycles[outer].polygon[0];
            let mut best: Option<(f64, usize)> = None;
            for (face_id, face) in self.faces.iter().enumerate() {
                if part_of_cycle[face.outer] == group {
                    continue;
                }
                let cycle = &self.cycles[face.outer];
                if best.is_some_and(|(area, _)| area <= cycle.area) {
                    continue;
                }
                if inside(&[&cycle.polygon], probe) {
                    best = Some((cycle.area, face_id));
                }
            }
            if let Some((_, face_id)) = best {
                parent.insert(group, face_id);
                self.face_of[outer] = Some(face_id);
                self.faces[face_id].holes.push(outer);
            }
        }
        let depth_of_part = |mut group: usize, faces: &[Face]| {
            let mut depth = 0;
            while let Some(&face) = parent.get(&group) {
                depth += 1;
                group = part_of_cycle[faces[face].outer];
                if depth > faces.len() {
                    break;
                }
            }
            depth
        };
        let depths: Vec<usize> = self
            .faces
            .iter()
            .map(|f| depth_of_part(part_of_cycle[f.outer], &self.faces))
            .collect();
        for (face, depth) in self.faces.iter_mut().zip(depths) {
            face.depth = depth;
        }
    }

    /// Each face's name: the elements round it (holes too) and the side of
    /// each it lies on, hashed; faces alike in that are told apart by
    /// where they start along their first element.
    fn names(&self) -> Vec<String> {
        let mut keys: Vec<(Vec<(TopoName, bool)>, f64)> = Vec::new();
        for face in &self.faces {
            let mut entries: Vec<(TopoName, bool)> = Vec::new();
            let mut first: Option<(TopoName, f64)> = None;
            for &cycle in std::iter::once(&face.outer).chain(&face.holes) {
                for &h in &self.cycles[cycle].edges {
                    let piece = &self.pieces[h / 2];
                    let name = self.curves[piece.curve].name;
                    entries.push((name, h.is_multiple_of(2)));
                    let at = piece.t0.min(piece.t1);
                    if first.is_none_or(|(n, t)| (name, at) < (n, t)) {
                        first = Some((name, at));
                    }
                }
            }
            entries.sort_unstable();
            entries.dedup();
            keys.push((entries, first.map_or(0.0, |(_, t)| t)));
        }
        let hash = |entries: &[(TopoName, bool)]| {
            let mut bytes = Vec::with_capacity(entries.len() * 9);
            for (name, left) in entries {
                bytes.extend_from_slice(&name.to_le_bytes());
                bytes.push(u8::from(*left));
            }
            kernel_api::naming::name_of(&bytes)
        };
        let mut names = vec![String::new(); self.faces.len()];
        let mut alike: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, (entries, _)) in keys.iter().enumerate() {
            alike.entry(hash(entries)).or_default().push(i);
        }
        for (hash, mut faces) in alike {
            faces.sort_by(|&a, &b| keys[a].1.total_cmp(&keys[b].1));
            for (k, face) in faces.into_iter().enumerate() {
                names[face] = if k == 0 {
                    format!("{hash:016x}")
                } else {
                    format!("{hash:016x}.{k}")
                };
            }
        }
        names
    }

    fn regions(&self) -> Vec<Region> {
        let names = self.names();
        let mut regions: Vec<Region> = self
            .faces
            .iter()
            .zip(names)
            .map(|(face, name)| {
                let outline: Vec<Vec<[f64; 2]>> = std::iter::once(&face.outer)
                    .chain(&face.holes)
                    .map(|&c| self.cycles[c].polygon.clone())
                    .collect();
                let polygons: Vec<&[[f64; 2]]> = outline.iter().map(Vec::as_slice).collect();
                let area = self.cycles[face.outer].area
                    + face.holes.iter().map(|&c| self.cycles[c].area).sum::<f64>();
                Region {
                    name,
                    area,
                    inside: point_inside(&polygons),
                    default: face.depth.is_multiple_of(2),
                    outline,
                }
            })
            .collect();
        regions.sort_by(|a, b| {
            (a.inside[0], a.inside[1])
                .partial_cmp(&(b.inside[0], b.inside[1]))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        regions
    }

    /// The outline of the chosen faces: faces sharing a piece become one,
    /// and each run of boundary round it a wire.
    fn wires(&self, chosen: &[bool]) -> Vec<ProfileWire> {
        // Chosen faces joined across the pieces between them.
        let mut group: Vec<usize> = (0..self.faces.len()).collect();
        fn root(group: &mut [usize], mut f: usize) -> usize {
            while group[f] != f {
                group[f] = group[group[f]];
                f = group[f];
            }
            f
        }
        let face_of_half = |h: usize| self.face_of[self.cycle_of[h]];
        for piece in 0..self.pieces.len() {
            if let (Some(a), Some(b)) = (face_of_half(2 * piece), face_of_half(2 * piece + 1))
                && chosen[a]
                && chosen[b]
            {
                let (ra, rb) = (root(&mut group, a), root(&mut group, b));
                group[ra] = rb;
            }
        }
        let groups: Vec<usize> = (0..self.faces.len()).map(|f| root(&mut group, f)).collect();
        let group_of_half = |h: usize| face_of_half(h).filter(|&f| chosen[f]).map(|f| groups[f]);
        let boundary =
            |h: usize| group_of_half(h).is_some() && group_of_half(h) != group_of_half(h ^ 1);

        let half = self.pieces.len() * 2;
        let mut used = vec![false; half];
        let mut loops: Vec<Vec<usize>> = Vec::new();
        for start in 0..half {
            if used[start] || !boundary(start) {
                continue;
            }
            let mut run = Vec::new();
            let mut h = start;
            loop {
                used[h] = true;
                run.push(h);
                let mut e = self.next[h];
                let mut turns = 0;
                while !boundary(e) && turns <= half {
                    e = self.next[e ^ 1];
                    turns += 1;
                }
                if e == start || used[e] || turns > half {
                    break;
                }
                h = e;
            }
            loops.push(run);
        }

        // The pieces of a run that follow on along one curve are one
        // segment again.
        let mut runs: Vec<Vec<(usize, f64, f64, bool)>> = loops
            .iter()
            .map(|run| {
                let mut out: Vec<(usize, f64, f64, bool)> = Vec::new();
                for &h in run {
                    let p = &self.pieces[h / 2];
                    let forward = h.is_multiple_of(2);
                    if let Some(last) = out.last_mut()
                        && last.0 == p.curve
                        && last.3 == forward
                    {
                        if forward && (last.2 - p.t0).abs() < 1e-12 {
                            last.2 = p.t1;
                            continue;
                        }
                        if !forward && (last.1 - p.t1).abs() < 1e-12 {
                            last.1 = p.t0;
                            continue;
                        }
                    }
                    out.push((p.curve, p.t0, p.t1, forward));
                }
                // Round the seam of the run, and of a closed curve.
                while out.len() > 1 {
                    let (first, last) = (out[0], out[out.len() - 1]);
                    if first.0 != last.0 || first.3 != last.3 {
                        break;
                    }
                    let c = &self.curves[first.0];
                    let period = if c.closed { c.period() } else { 0.0 };
                    let (a, b) = if first.3 {
                        (last, first)
                    } else {
                        (first, last)
                    };
                    let joins = |end: f64, start: f64| {
                        (end - start).abs() < 1e-12
                            || (c.closed && (end - start - period).abs() < 1e-9 * period.max(1.0))
                    };
                    if !joins(a.2, b.1) {
                        break;
                    }
                    let shift = if (a.2 - b.1).abs() < 1e-12 {
                        0.0
                    } else {
                        period
                    };
                    let merged = (a.0, a.1, b.2 + shift, first.3);
                    out.pop();
                    out[0] = merged;
                }
                out
            })
            .collect();

        // An element giving the profile more than one segment names its
        // first along it after itself and the rest after their place.
        let mut along: HashMap<usize, Vec<(f64, usize, usize)>> = HashMap::new();
        for (r, run) in runs.iter().enumerate() {
            for (s, &(curve, t0, ..)) in run.iter().enumerate() {
                along
                    .entry(curve)
                    .or_default()
                    .push((self.curves[curve].wrap(t0), r, s));
            }
        }
        let mut names: HashMap<(usize, usize), TopoName> = HashMap::new();
        for (curve, mut list) in along {
            list.sort_by(|a, b| a.0.total_cmp(&b.0));
            let base = self.curves[curve].name;
            for (k, (_, r, s)) in list.into_iter().enumerate() {
                let name = if k == 0 {
                    base
                } else {
                    kernel_api::naming::child(base, &(k as u64).to_le_bytes())
                };
                names.insert((r, s), name);
            }
        }

        let mut wires = Vec::new();
        for (r, run) in runs.iter_mut().enumerate() {
            let mut segments = Vec::new();
            let mut wire_names = Vec::new();
            for (s, &(curve, t0, t1, forward)) in run.iter().enumerate() {
                let name = names[&(r, s)];
                for segment in self.segments_of(curve, t0, t1, forward) {
                    segments.push(segment);
                    wire_names.push(name);
                }
            }
            wires.push(ProfileWire {
                segments,
                names: wire_names,
            });
        }
        wires
    }

    /// Curve `curve` over `[t0, t1]` as profile segments run the given
    /// way: the segment it was drawn as when that is all of it, a loop
    /// back to its start in two halves.
    fn segments_of(&self, curve: usize, t0: f64, t1: f64, forward: bool) -> Vec<ProfileSegment> {
        let c = &self.curves[curve];
        let whole = (t0 - c.lo).abs() < 1e-12 && (t1 - c.hi).abs() < 1e-12;
        if whole && (c.closed || forward) {
            return vec![c.segment.clone()];
        }
        if whole {
            return vec![crate::profile::reversed(&c.segment)];
        }
        let parts = if dist(c.at(t0), c.at(t1)) < JOIN {
            let mid = (t0 + t1) * 0.5;
            vec![(t0, mid), (mid, t1)]
        } else {
            vec![(t0, t1)]
        };
        let mut out: Vec<ProfileSegment> = parts
            .into_iter()
            .map(|(a, b)| self.part_of(c, a, b))
            .collect();
        if !forward {
            out.reverse();
            out = out.iter().map(crate::profile::reversed).collect();
        }
        out
    }

    fn snap(&self, p: [f64; 2]) -> [f64; 2] {
        self.vertices
            .iter()
            .copied()
            .find(|v| dist(*v, p) < JOIN)
            .unwrap_or(p)
    }

    fn part_of(&self, c: &Curve, t0: f64, t1: f64) -> ProfileSegment {
        let (start, end) = (self.snap(c.at(t0)), self.snap(c.at(t1)));
        match &c.carrier {
            Carrier::Line { .. } => ProfileSegment::Line { start, end },
            Carrier::Round { .. } => ProfileSegment::Arc {
                start,
                mid: c.at((t0 + t1) * 0.5),
                end,
            },
            Carrier::Oval {
                c: center,
                major,
                ratio,
                t0: a,
                t1: b,
            } => ProfileSegment::EllipseArc {
                center: *center,
                major: *major,
                ratio: *ratio,
                start_param: a + (b - a) * t0,
                end_param: a + (b - a) * t1,
            },
            Carrier::Spline(spline) => {
                let part = spline.part(t0, t1);
                let mut control_points: Vec<[f64; 2]> = part
                    .points
                    .iter()
                    .map(|p| [p[0] / p[2], p[1] / p[2]])
                    .collect();
                if let Some(first) = control_points.first_mut() {
                    *first = start;
                }
                if let Some(last) = control_points.last_mut() {
                    *last = end;
                }
                ProfileSegment::Nurbs {
                    degree: part.degree as u32,
                    knots: part.knots,
                    control_points,
                    weights: if part.rational {
                        part.points.iter().map(|p| p[2]).collect()
                    } else {
                        Vec::new()
                    },
                    periodic: false,
                }
            }
        }
    }
}

/// The arrangement of the segments given with their names.
fn arrange(segments: &[(ProfileSegment, TopoName)]) -> Arrangement {
    let curves = segments
        .iter()
        .filter_map(|(segment, name)| Curve::of(segment, *name))
        .collect();
    Arrangement::build(curves)
}

/// What [`crate::profile`] asks of the arrangement of a sketch's curves.
pub(crate) struct Arranged {
    arrangement: Arrangement,
    regions: std::cell::OnceCell<Vec<Region>>,
}

impl Arranged {
    pub(crate) fn of(segments: &[(ProfileSegment, TopoName)]) -> Arranged {
        Arranged {
            arrangement: arrange(segments),
            regions: std::cell::OnceCell::new(),
        }
    }

    /// Whether a vertex joins more than two pieces once loose ends are
    /// gone: curves crossing, touching or branching.
    pub(crate) fn branches(&self) -> bool {
        self.arrangement.branches
    }

    pub(crate) fn regions(&self) -> &[Region] {
        self.regions.get_or_init(|| self.arrangement.regions())
    }

    /// Whether the curves close any region at all.
    pub(crate) fn closes_any(&self) -> bool {
        !self.arrangement.faces.is_empty()
    }

    /// The outline of the regions named, or of the default ones when
    /// `chosen` is empty. `Err` names a region that is not there.
    pub(crate) fn wires(&self, chosen: &[String]) -> Result<Vec<ProfileWire>, String> {
        let names = self.arrangement.names();
        let picked: Vec<bool> = if chosen.is_empty() {
            self.arrangement
                .faces
                .iter()
                .map(|f| f.depth.is_multiple_of(2))
                .collect()
        } else {
            if let Some(missing) = chosen.iter().find(|c| !names.contains(c)) {
                return Err(missing.clone());
            }
            names.iter().map(|n| chosen.contains(n)).collect()
        };
        Ok(self.arrangement.wires(&picked))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn circle(x: f64, y: f64, r: f64, name: TopoName) -> (ProfileSegment, TopoName) {
        (
            ProfileSegment::Circle {
                center: [x, y],
                radius: r,
            },
            name,
        )
    }

    fn line(a: [f64; 2], b: [f64; 2], name: TopoName) -> (ProfileSegment, TopoName) {
        (ProfileSegment::Line { start: a, end: b }, name)
    }

    fn areas(arranged: &Arranged) -> Vec<f64> {
        let mut a: Vec<f64> = arranged.regions().iter().map(|r| r.area).collect();
        a.sort_by(f64::total_cmp);
        a
    }

    /// The area two circles of radius `r` with centres `d` apart share.
    fn lens(r: f64, d: f64) -> f64 {
        2.0 * r * r * (d / (2.0 * r)).acos() - d / 2.0 * (4.0 * r * r - d * d).sqrt()
    }

    #[test]
    fn spline_parts_follow_the_curve() {
        let control = [[0.0, 0.0], [1.0, 3.0], [3.0, -1.0], [5.0, 2.0], [6.0, 0.0]];
        let spline = Spline {
            degree: 3,
            knots: clamped_uniform(3, 5),
            points: control.iter().map(|p| [p[0], p[1], 1.0]).collect(),
            rational: false,
        };
        let part = spline.part(0.2, 0.7);
        for i in 0..=10 {
            let u = 0.2 + 0.5 * f64::from(i) / 10.0;
            let (a, b) = (spline.eval(u), part.eval(u));
            assert!(dist(a, b) < 1e-9, "at {u}: {a:?} vs {b:?}");
        }
        let periodic = periodic_spline(&control, 3);
        let wrap = periodic.part(6.5, 9.5);
        for i in 0..=10 {
            let u = 6.5 + 3.0 * f64::from(i) / 10.0;
            assert!(dist(periodic.eval(u), wrap.eval(u)) < 1e-9);
        }
    }

    #[test]
    fn two_crossing_circles_make_three_regions() {
        let arranged = Arranged::of(&[circle(0.0, 0.0, 10.0, 1), circle(10.0, 0.0, 10.0, 2)]);
        assert!(arranged.branches());
        let a = areas(&arranged);
        assert_eq!(a.len(), 3, "{a:?}");
        let shared = lens(10.0, 10.0);
        let one = std::f64::consts::PI * 100.0;
        assert!((a[0] - shared).abs() < 1e-2 * shared, "the lens: {a:?}");
        assert!(
            (a[1] - (one - shared)).abs() < 1e-2 * one,
            "a crescent: {a:?}"
        );
        assert!(arranged.regions().iter().all(|r| r.default));
        let names: std::collections::HashSet<&str> =
            arranged.regions().iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names.len(), 3, "names differ");
        // Every region by default: the union, one outline.
        let wires = arranged.wires(&[]).unwrap();
        assert_eq!(wires.len(), 1);
        assert_eq!(wires[0].segments.len(), 2);
        assert!(
            wires[0]
                .segments
                .iter()
                .all(|s| matches!(s, ProfileSegment::Arc { .. }))
        );
    }

    #[test]
    fn the_lens_alone_is_its_two_arcs() {
        let arranged = Arranged::of(&[circle(0.0, 0.0, 10.0, 1), circle(10.0, 0.0, 10.0, 2)]);
        let lens_region = arranged
            .regions()
            .iter()
            .find(|r| (r.inside[0] - 5.0).abs() < 1.0)
            .expect("the lens");
        let wires = arranged
            .wires(std::slice::from_ref(&lens_region.name))
            .unwrap();
        assert_eq!(wires.len(), 1);
        assert_eq!(wires[0].segments.len(), 2);
        assert_eq!(wires[0].names.len(), 2);
        assert_ne!(wires[0].names[0], wires[0].names[1]);
        for s in &wires[0].segments {
            let ProfileSegment::Arc { mid, .. } = s else {
                panic!("arcs: {s:?}")
            };
            assert!(
                (-1e-9..=10.0 + 1e-9).contains(&mid[0]),
                "on the lens: {mid:?}"
            );
        }
        assert!(arranged.wires(&["nowhere".into()]).is_err());
    }

    #[test]
    fn names_hold_while_the_circles_move() {
        let names = |d: f64| {
            let arranged = Arranged::of(&[circle(0.0, 0.0, 10.0, 1), circle(d, 0.0, 10.0, 2)]);
            let mut by_x: Vec<(f64, String)> = arranged
                .regions()
                .iter()
                .map(|r| (r.inside[0], r.name.clone()))
                .collect();
            by_x.sort_by(|a, b| a.0.total_cmp(&b.0));
            by_x.into_iter().map(|(_, n)| n).collect::<Vec<_>>()
        };
        assert_eq!(names(8.0), names(12.0));
    }

    #[test]
    fn three_overlapping_circles_make_seven_regions() {
        let r = 10.0;
        let arranged = Arranged::of(&[
            circle(0.0, 0.0, r, 1),
            circle(10.0, 0.0, r, 2),
            circle(5.0, 8.0, r, 3),
        ]);
        let regions = arranged.regions();
        assert_eq!(regions.len(), 7, "{:?}", areas(&arranged));
        let total: f64 = regions.iter().map(|r| r.area).sum();
        let wires = arranged.wires(&[]).unwrap();
        assert_eq!(wires.len(), 1, "the union is one outline");
        // Every region's own outline closes, and together they tile it.
        for region in regions {
            let w = arranged.wires(std::slice::from_ref(&region.name)).unwrap();
            assert_eq!(w.len(), 1, "{}", region.name);
        }
        assert!(total > std::f64::consts::PI * r * r);
    }

    #[test]
    fn a_figure_eight_spline_makes_two_lobes() {
        // A closed spline crossing itself once in the middle.
        let control = vec![
            [0.0, 0.0],
            [5.0, 5.0],
            [10.0, 0.0],
            [5.0, -5.0],
            [0.0, 0.0],
            [-5.0, 5.0],
            [-10.0, 0.0],
            [-5.0, -5.0],
        ];
        let arranged = Arranged::of(&[(
            ProfileSegment::BSpline {
                control_points: control,
                periodic: true,
            },
            7,
        )]);
        let regions = arranged.regions();
        assert_eq!(regions.len(), 2, "two lobes");
        assert!(regions[0].inside[0] < 0.0 && regions[1].inside[0] > 0.0);
        assert!((regions[0].area - regions[1].area).abs() < 1e-3 * regions[0].area);
        let wires = arranged
            .wires(std::slice::from_ref(&regions[1].name))
            .unwrap();
        assert_eq!(wires.len(), 1);
        assert!(
            wires[0]
                .segments
                .iter()
                .all(|s| matches!(s, ProfileSegment::Nurbs { .. }))
        );
    }

    #[test]
    fn a_t_branch_splits_the_square() {
        let (a, b, c, d) = ([0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]);
        let arranged = Arranged::of(&[
            line(a, b, 1),
            line(b, c, 2),
            line(c, d, 3),
            line(d, a, 4),
            // From the middle of the bottom up to the top.
            line([4.0, 0.0], [4.0, 10.0], 5),
        ]);
        assert!(arranged.branches());
        assert_eq!(areas(&arranged), vec![40.0, 60.0]);
        // Both halves by default: the square again, its bottom whole.
        let wires = arranged.wires(&[]).unwrap();
        assert_eq!(wires.len(), 1);
        assert_eq!(wires[0].segments.len(), 4, "{:?}", wires[0].segments);
        let mut names = wires[0].names.clone();
        names.sort_unstable();
        assert_eq!(names, vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_spur_is_left_out() {
        let (a, b, c, d) = ([0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]);
        let arranged = Arranged::of(&[
            line(a, b, 1),
            line(b, c, 2),
            line(c, d, 3),
            line(d, a, 4),
            // Into the square from its side, ending inside.
            line([0.0, 5.0], [5.0, 5.0], 5),
            // And out of it past a corner.
            line([10.0, 10.0], [15.0, 15.0], 6),
        ]);
        assert!(!arranged.branches());
        assert_eq!(areas(&arranged), vec![100.0]);
        let wires = arranged.wires(&[]).unwrap();
        assert_eq!(wires.len(), 1);
        assert_eq!(wires[0].segments.len(), 4);
    }

    #[test]
    fn a_circle_in_a_square_is_a_hole_unless_chosen() {
        let (a, b, c, d) = ([0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]);
        let arranged = Arranged::of(&[
            line(a, b, 1),
            line(b, c, 2),
            line(c, d, 3),
            line(d, a, 4),
            circle(5.0, 5.0, 2.0, 5),
        ]);
        let regions = arranged.regions();
        assert_eq!(regions.len(), 2);
        let ring = regions.iter().find(|r| r.default).expect("the ring");
        let disc = regions.iter().find(|r| !r.default).expect("the disc");
        assert!((ring.area - (100.0 - std::f64::consts::PI * 4.0)).abs() < 0.1);
        assert!(dist(ring.inside, [5.0, 5.0]) > 2.0, "out of the hole");
        assert_eq!(arranged.wires(&[]).unwrap().len(), 2, "outline and hole");
        let both = arranged
            .wires(&[ring.name.clone(), disc.name.clone()])
            .unwrap();
        assert_eq!(both.len(), 1, "the hole filled");
        let disc_only = arranged.wires(std::slice::from_ref(&disc.name)).unwrap();
        assert!(matches!(
            disc_only[0].segments[..],
            [ProfileSegment::Circle { .. }]
        ));
    }

    #[test]
    fn crossing_lines_close_a_triangle() {
        let arranged = Arranged::of(&[
            line([-2.0, 0.0], [12.0, 0.0], 1),
            line([-1.0, -2.0], [6.0, 12.0], 2),
            line([11.0, -2.0], [4.0, 12.0], 3),
        ]);
        let regions = arranged.regions();
        assert_eq!(regions.len(), 1);
        // The base from x = 0 to 10, the apex where the sides meet.
        assert!(
            (regions[0].area - 0.5 * 10.0 * 10.0).abs() < 1e-6,
            "{}",
            regions[0].area
        );
    }
}
