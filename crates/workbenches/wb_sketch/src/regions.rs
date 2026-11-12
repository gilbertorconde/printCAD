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

use glam::DVec2;
use kernel_api::{ProfileSegment, ProfileWire, ThreePointArc, TopoName};
use sketch_solver::spline::{self, Basis};

use crate::curves::{Path, refine, segments_cross};

/// How near two points are one vertex, in sketch units (mm): curves meet
/// where the sketch's points put them (a document saved in single
/// precision a few millionths of a millimetre apart), and projected edges
/// within a solid's tolerance. An end this near another curve lies on it,
/// as trim and extend count a crossing this far past an end.
const JOIN: f64 = crate::curves::END_SLACK;

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

/// The region of `regions` that holds `p` (on the sketch plane), out of
/// its holes.
pub fn region_at(regions: &[Region], p: [f64; 2]) -> Option<&Region> {
    regions.iter().find(|r| {
        let polygons: Vec<&[[f64; 2]]> = r.outline.iter().map(Vec::as_slice).collect();
        kernel_api::polygon::contains(&polygons, p)
    })
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
    /// A circle: the angle `start + sweep * t`.
    Round {
        c: [f64; 2],
        r: f64,
        start: f64,
        sweep: f64,
    },
    /// An arc of a circle, through its three points.
    Arc(ThreePointArc),
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

/// A B-spline over its `sketch_solver` basis, rational when weighted. A
/// periodic one's parameter runs on for a second turn, so a piece running
/// over its seam is one span of parameter.
#[derive(Debug, Clone)]
struct Spline {
    basis: Basis,
    control: Vec<[f64; 2]>,
    /// The segment carried weights, which its parts keep even when every
    /// one is one.
    rational: bool,
}

impl Spline {
    fn open(degree: usize, control: &[[f64; 2]], knots: &[f64], weights: &[f64]) -> Option<Spline> {
        Some(Spline {
            basis: Basis::new(degree as u32, control.len(), knots, false)?.with_weights(weights),
            control: control.to_vec(),
            rational: !weights.is_empty(),
        })
    }

    fn periodic(degree: usize, control: &[[f64; 2]]) -> Option<Spline> {
        Some(Spline {
            basis: Basis::new(degree as u32, control.len(), &[], true)?,
            control: control.to_vec(),
            rational: false,
        })
    }

    fn eval(&self, u: f64) -> [f64; 2] {
        let (d0, d1) = self.basis.domain();
        let u = if self.basis.is_periodic() {
            d0 + (u - d0).rem_euclid(d1 - d0)
        } else {
            u
        };
        self.basis.eval(&self.control, u)
    }

    /// How many knot spans meet `[t0, t1]`; a periodic spline's uniform
    /// knots run on for a second turn.
    fn spans_over(&self, t0: f64, t1: f64) -> usize {
        if self.basis.is_periodic() {
            let ends = 2 * (self.control.len() + self.basis.degree());
            (0..ends)
                .filter(|&k| (k + 1) as f64 > t0 && (k as f64) < t1)
                .count()
        } else {
            self.basis
                .knots()
                .windows(2)
                .filter(|w| w[1] > w[0] && w[1] > t0 && w[0] < t1)
                .count()
        }
    }

    /// The part of the curve over `[u0, u1]` as an open spline, clamped
    /// at both ends, its knots over `[u0, u1]`.
    fn part(&self, u0: f64, u1: f64) -> Option<spline::Piece> {
        let piece = self.basis.piece(&self.control, u0, u1)?;
        let p = self.basis.degree();
        let count = piece.knots.len();
        let knots = piece
            .knots
            .iter()
            .enumerate()
            .map(|(i, k)| {
                if i <= p {
                    u0
                } else if i + p + 1 >= count {
                    u1
                } else {
                    u0 + (u1 - u0) * k
                }
            })
            .collect();
        let weights = match (self.rational, piece.weights.is_empty()) {
            (false, _) => Vec::new(),
            (true, true) => vec![1.0; piece.control.len()],
            (true, false) => piece.weights,
        };
        Some(spline::Piece {
            control: piece.control,
            knots,
            weights,
        })
    }
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
            ProfileSegment::Arc { start, mid, end } => curve(
                Carrier::Arc(ThreePointArc::through(*start, *mid, *end)?),
                0.0,
                1.0,
                false,
            ),
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
                    let spline = Spline::periodic(degree, control_points)?;
                    curve(
                        Carrier::Spline(spline),
                        degree as f64,
                        (n + degree) as f64,
                        true,
                    )
                } else {
                    let spline = Spline::open(degree, control_points, &[], &[])?;
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
                    let spline = Spline::periodic(degree, control_points)?;
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
                let spline = Spline::open(degree, control_points, knots, weights)?;
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
            Carrier::Arc(arc) => arc.at(t),
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
            Carrier::Arc(arc) => {
                let turn = (arc.sweep * (t1 - t0)).abs() / std::f64::consts::TAU;
                ((96.0 * turn).ceil() as usize).max(4)
            }
            Carrier::Oval { t0: a, t1: b, .. } => {
                let turn = ((b - a) * (t1 - t0)).abs() / std::f64::consts::TAU;
                ((96.0 * turn).ceil() as usize).max(4)
            }
            Carrier::Spline(s) => (s.spans_over(t0, t1) * 12).clamp(12, 600),
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

impl Path for Curve {
    fn at(&self, t: f64) -> DVec2 {
        Curve::at(self, t).into()
    }

    fn step(&self) -> f64 {
        1e-7 * self.period().max(1e-9)
    }

    /// An open curve is held to its ends: an end near another curve is
    /// found by projecting it.
    fn settle(&self, t: f64) -> f64 {
        self.wrap(t)
    }
}

/// Where on curve `c` point `p` lies, when it lies on it.
fn project(c: &Curve, samples: &[(f64, [f64; 2])], p: [f64; 2]) -> Option<f64> {
    // The nearest sample segment, then Newton's method on the distance.
    let mut best = (f64::INFINITY, c.lo);
    for w in samples.windows(2) {
        let ((ta, a), (tb, b)) = (w[0], w[1]);
        let f = kernel_api::nearest::segment_param(p, a, b);
        let q = [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
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
                        let Some((u, v)) = segments_cross(
                            si[a].1.into(),
                            si[a + 1].1.into(),
                            sj[b].1.into(),
                            sj[b + 1].1.into(),
                            0.0,
                        ) else {
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
                        let Some((t, s)) = refine(&curves[i], &curves[j], t, s) else {
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
            let area = kernel_api::polygon::signed_area(&polygon);
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
                if kernel_api::polygon::contains(&[&cycle.polygon], probe) {
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
            Carrier::Round { .. } | Carrier::Arc(_) => ProfileSegment::Arc {
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
                // A piece's parameters lie inside the spline's domain, a
                // closed one's within a turn of its start, where a part
                // always exists.
                let Some(spline::Piece {
                    control: mut control_points,
                    knots,
                    weights,
                }) = spline.part(t0, t1)
                else {
                    return ProfileSegment::Line { start, end };
                };
                if let Some(first) = control_points.first_mut() {
                    *first = start;
                }
                if let Some(last) = control_points.last_mut() {
                    *last = end;
                }
                ProfileSegment::Nurbs {
                    degree: spline.basis.degree() as u32,
                    knots,
                    control_points,
                    weights,
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

    /// A nearly flat arc over a box, at chords from a micron to a metre
    /// (anything smaller is inside the join distance) and flatness on both
    /// sides of the radius limit: the region closes exactly when the shared
    /// three-point rule makes an arc, which is when the kernel builds the
    /// profile (`kernel_ogeom`'s
    /// `the_kernel_builds_a_nearly_flat_arc_when_the_shared_rule_does`),
    /// with the box's area and the arc's sliver.
    #[test]
    fn a_nearly_flat_arc_closes_a_region_when_it_is_an_arc() {
        for chord in [1e-3, 1.0, 10.0, 1000.0] {
            for sine in [2e-13, 2e-12, 1e-11, 1e-9, 1e-7, 1e-6, 1e-4, 1e-2] {
                let (start, mid, end) =
                    ([0.0, 0.0], [chord / 2.0, sine * chord / 2.0], [chord, 0.0]);
                let depth = -chord / 2.0;
                let arranged = Arranged::of(&[
                    (ProfileSegment::Arc { start, mid, end }, 1),
                    line(end, [chord, depth], 2),
                    line([chord, depth], [0.0, depth], 3),
                    line([0.0, depth], start, 4),
                ]);
                let arc = ThreePointArc::through(start, mid, end).is_some();
                let found = areas(&arranged);
                assert_eq!(found.len(), usize::from(arc), "chord {chord}, sine {sine}");
                if arc {
                    // The box, and no more than the sliver the arc bulges
                    // over it.
                    let (box_area, sliver) = (chord * chord / 2.0, chord * mid[1]);
                    let slack = 1e-6 * box_area;
                    assert!(
                        (box_area - slack..=box_area + sliver + slack).contains(&found[0]),
                        "chord {chord}, sine {sine}: {}",
                        found[0]
                    );
                }
            }
        }
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

    /// A spline's points, and the parts a profile takes of it, are
    /// `sketch_solver`'s curve: open, rational and closed, a closed one's
    /// part running over its seam.
    #[test]
    fn splines_are_sketch_solvers() {
        let control = vec![[0.0, 0.0], [1.0, 3.0], [3.0, -1.0], [5.0, 2.0], [6.0, 0.0]];
        let knots = [0.0, 0.0, 0.0, 0.0, 0.4, 1.0, 1.0, 1.0, 1.0];
        let weights = [1.0, 2.0, 0.5, 1.5, 1.0];
        let cases = [
            (
                ProfileSegment::BSpline {
                    control_points: control.clone(),
                    periodic: false,
                },
                Basis::new(3, 5, &[], false).unwrap(),
                (0.2, 0.7),
            ),
            (
                ProfileSegment::Nurbs {
                    degree: 3,
                    knots: knots.to_vec(),
                    control_points: control.clone(),
                    weights: weights.to_vec(),
                    periodic: false,
                },
                Basis::new(3, 5, &knots, false)
                    .unwrap()
                    .with_weights(&weights),
                (0.1, 0.9),
            ),
            (
                ProfileSegment::BSpline {
                    control_points: control.clone(),
                    periodic: true,
                },
                Basis::new(3, 5, &[], true).unwrap(),
                (6.5, 9.5),
            ),
        ];
        for (segment, basis, (u0, u1)) in cases {
            let curve = Curve::of(&segment, 1).unwrap();
            let Carrier::Spline(spline) = &curve.carrier else {
                panic!("a spline: {segment:?}");
            };
            let (d0, d1) = basis.domain();
            for i in 0..=20 {
                let u = d0 + (d1 - d0) * f64::from(i) / 20.0;
                let want = basis.eval(&control, u);
                assert!(dist(curve.at(u), want) < 1e-12, "{segment:?} at {u}");
                if curve.closed {
                    let again = curve.at(u + curve.period());
                    assert!(dist(again, want) < 1e-12, "a second turn at {u}");
                }
            }
            let piece = spline.part(u0, u1).unwrap();
            let points = piece.control;
            let part = Basis::new(3, points.len(), &piece.knots, false)
                .unwrap()
                .with_weights(&piece.weights);
            assert_eq!(part.domain(), (u0, u1));
            for i in 0..=20 {
                let u = u0 + (u1 - u0) * f64::from(i) / 20.0;
                let (a, b) = (curve.at(u), part.eval(&points, u));
                assert!(
                    dist(a, b) < 1e-12,
                    "{segment:?} part at {u}: {a:?} vs {b:?}"
                );
            }
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
