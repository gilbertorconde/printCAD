//! Editing what a spline is made of: other curves made into splines, the
//! degree raised or lowered, knots inserted or their multiplicity changed,
//! a control point's weight. Worked on weighted control points, so a
//! rational spline stays exact: raising the degree or inserting a knot
//! keeps the curve as it is; lowering either fits the nearest curve the
//! fewer degrees of freedom make.

use std::collections::HashSet;

use glam::DVec2;
use uuid::Uuid;

use crate::sketch::{BSpline, GeometryElement, Point, Sketch, Vec2D};
use crate::spline::{MAX_DEGREE, basis_of};
use crate::tools::ToolEffect;
use sketch_solver::spline::{Basis, insert_knot};

/// A spline in the making: degree, clamped knots, and control points with
/// their weights; `start` and `end` are the sketch points the first and
/// last keep, where the curve it is made from has them.
struct Made {
    degree: u32,
    knots: Vec<f64>,
    control: Vec<DVec2>,
    weights: Vec<f64>,
    start: Option<Uuid>,
    end: Option<Uuid>,
}

fn dv(p: Vec2D) -> DVec2 {
    DVec2::new(f64::from(p.x), f64::from(p.y))
}

/// An arc of the ellipse `c + u cos θ + v sin θ` (a circle when `u` and `v`
/// are square and alike) from `from` sweeping `sweep`, as rational
/// quadratic pieces of at most a quarter turn: exactly the curve.
fn conic_arc(c: DVec2, u: DVec2, v: DVec2, from: f64, sweep: f64) -> Made {
    let pieces = ((sweep.abs() / std::f64::consts::FRAC_PI_2).ceil() as usize).max(1);
    let step = sweep / pieces as f64;
    let at = |t: f64| c + u * t.cos() + v * t.sin();
    let w = (step / 2.0).cos();
    let mut control = vec![at(from)];
    let mut weights = vec![1.0];
    let mut knots = vec![0.0; 3];
    for i in 0..pieces {
        let (a, b) = (from + step * i as f64, from + step * (i + 1) as f64);
        let m = (a + b) / 2.0;
        control.push(c + (u * m.cos() + v * m.sin()) / w);
        weights.push(w);
        control.push(at(b));
        weights.push(1.0);
        if i + 1 < pieces {
            let k = (i + 1) as f64 / pieces as f64;
            knots.extend([k, k]);
        }
    }
    knots.extend([1.0; 3]);
    Made {
        degree: 2,
        knots,
        control,
        weights,
        start: None,
        end: None,
    }
}

/// `geom` as a spline, or `None` for one that already is or a point.
fn made_of(sketch: &Sketch, geom: &GeometryElement) -> Option<Made> {
    let pos = |id: Uuid| sketch.point_position(id).map(dv);
    Some(match geom {
        GeometryElement::Line(l) => {
            let (a, b) = (pos(l.start)?, pos(l.end)?);
            Made {
                degree: 3,
                knots: Vec::new(),
                control: (0..4).map(|i| a + (b - a) * (i as f64 / 3.0)).collect(),
                weights: Vec::new(),
                start: Some(l.start),
                end: Some(l.end),
            }
        }
        GeometryElement::Arc(a) => {
            let c = pos(a.center)?;
            let (s, e) = (pos(a.start)? - c, pos(a.end)? - c);
            let from = s.y.atan2(s.x);
            let mut to = e.y.atan2(e.x);
            while to <= from {
                to += std::f64::consts::TAU;
            }
            let r = s.length();
            Made {
                start: Some(a.start),
                end: Some(a.end),
                ..conic_arc(c, DVec2::X * r, DVec2::Y * r, from, to - from)
            }
        }
        GeometryElement::Circle(k) => {
            let r = f64::from(k.radius);
            conic_arc(
                pos(k.center)?,
                DVec2::X * r,
                DVec2::Y * r,
                0.0,
                std::f64::consts::TAU,
            )
        }
        GeometryElement::Ellipse(e) => {
            let u = dv(e.major);
            let v = u.perp() * f64::from(e.ratio);
            let (t0, t1) = e.param_span(sketch)?;
            let (t0, t1) = (f64::from(t0), f64::from(t1));
            Made {
                start: e.arc.map(|a| a.start),
                end: e.arc.map(|a| a.end),
                ..conic_arc(pos(e.center)?, u, v, t0, t1 - t0)
            }
        }
        GeometryElement::Conic(k) => {
            let (shape, t0, t1) = k.params(sketch)?;
            let (points, w) = shape.quadratic(t0, t1);
            Made {
                degree: 2,
                knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                control: points.iter().map(|p| DVec2::new(p[0], p[1])).collect(),
                weights: vec![1.0, w, 1.0],
                start: Some(k.start),
                end: Some(k.end),
            }
        }
        GeometryElement::BSpline(_) | GeometryElement::Point(_) => return None,
    })
}

fn new_point(sketch: &mut Sketch, p: DVec2) -> Uuid {
    sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(
        p.x as f32, p.y as f32,
    ))))
}

/// Every line, arc, circle, ellipse, arc of an ellipse, parabola and
/// hyperbola of `items` as a spline that is exactly it, keeping its end
/// points, so what met it meets the spline. A closed curve's spline starts
/// and ends on one point.
pub fn to_bspline(sketch: &mut Sketch, items: &HashSet<Uuid>) -> ToolEffect {
    let targets: Vec<GeometryElement> = sketch
        .geometry
        .iter()
        .filter(|g| items.contains(&g.id()) && !sketch.is_external(g.id()))
        .cloned()
        .collect();
    let mut made = 0;
    for geom in targets {
        let Some(m) = made_of(sketch, &geom) else {
            continue;
        };
        let last = m.control.len() - 1;
        let start = m.start.unwrap_or_else(|| new_point(sketch, m.control[0]));
        let end = match m.end {
            Some(end) => end,
            // A closed curve's spline comes back to where it began.
            None if (m.control[last] - m.control[0]).length() < 1e-9 => start,
            None => new_point(sketch, m.control[last]),
        };
        let mut ids = vec![start];
        for p in &m.control[1..last] {
            ids.push(new_point(sketch, *p));
        }
        ids.push(end);
        let construction = sketch.is_construction(geom.id());
        let spline = sketch.add_geometry(GeometryElement::BSpline(BSpline {
            degree: m.degree,
            knots: m.knots,
            weights: m.weights,
            ..BSpline::new(ids, false)
        }));
        sketch.set_construction(spline, construction);
        sketch.remove_geometry_cascade(&[geom.id()]);
        made += 1;
    }
    if made == 0 {
        return ToolEffect::log("Select lines, arcs, circles, ellipses or conics to make splines");
    }
    ToolEffect::changed(format!("Made {made} curve(s) into splines"))
}

/// A spline's make-up in double precision: its basis and its control
/// points lifted by their weights.
struct Lifted {
    degree: usize,
    knots: Vec<f64>,
    control: Vec<[f64; 3]>,
    rational: bool,
}

/// `spline` as an open clamped spline over weighted control points. A
/// closed one is laid open where its parameter starts, a curve that runs
/// from a point back to it, so it can take knots of its own.
fn lifted(sketch: &Sketch, spline: &BSpline) -> Option<Lifted> {
    let basis = basis_of(spline)?;
    let control = spline.control_positions(sketch)?;
    let rational = !basis.weights().is_empty();
    if basis.is_periodic() {
        let (d0, d1) = basis.domain();
        let piece = basis.piece(&control, d0, d1)?;
        let weights = piece.weights.clone();
        return Some(Lifted {
            degree: basis.degree(),
            knots: piece.knots,
            control: piece
                .control
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let w = weights.get(i).copied().unwrap_or(1.0);
                    [c[0] * w, c[1] * w, w]
                })
                .collect(),
            rational,
        });
    }
    let w = |i: usize| basis.weights().get(i).copied().unwrap_or(1.0);
    Some(Lifted {
        degree: basis.degree(),
        knots: basis.knots().to_vec(),
        control: control
            .iter()
            .enumerate()
            .map(|(i, c)| [c[0] * w(i), c[1] * w(i), w(i)])
            .collect(),
        rational,
    })
}

impl Lifted {
    /// The weighted point at `t`.
    fn at(&self, t: f64) -> [f64; 3] {
        let basis = Basis::new(self.degree as u32, self.control.len(), &self.knots, false)
            .expect("a lifted spline's basis");
        basis.row(t).into_iter().fold([0.0; 3], |acc, (i, n)| {
            let q = self.control[i];
            [acc[0] + n * q[0], acc[1] + n * q[1], acc[2] + n * q[2]]
        })
    }

    /// The knots strictly inside the domain, each once, with how many
    /// times it stands.
    fn interior(&self) -> Vec<(f64, usize)> {
        let (d0, d1) = (self.knots[self.degree], self.knots[self.control.len()]);
        let mut out: Vec<(f64, usize)> = Vec::new();
        for k in self.knots.iter().copied().filter(|k| *k > d0 && *k < d1) {
            match out.last_mut() {
                Some((v, m)) if (*v - k).abs() < 1e-12 => *m += 1,
                _ => out.push((k, 1)),
            }
        }
        out
    }

    /// The same curve's nearest over `degree` and interior knots `inner`
    /// (value and multiplicity), its ends where they were: exact where the
    /// new make-up holds the curve.
    fn refit(&self, degree: usize, inner: &[(f64, usize)]) -> Option<Lifted> {
        let (d0, d1) = (self.knots[self.degree], self.knots[self.control.len()]);
        let mut knots = vec![d0; degree + 1];
        for &(k, m) in inner {
            knots.extend(std::iter::repeat_n(k, m));
        }
        knots.extend(std::iter::repeat_n(d1, degree + 1));
        let count = knots.len() - degree - 1;
        if count < degree + 1 {
            return None;
        }
        let basis = Basis::new(degree as u32, count, &knots, false)?;
        // Samples dense in every span, of the old knots and the new.
        let mut breaks: Vec<f64> = self.knots.iter().chain(&knots).copied().collect();
        breaks.retain(|k| *k >= d0 && *k <= d1);
        breaks.sort_by(f64::total_cmp);
        breaks.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
        let per = 4 * (degree.max(self.degree) + 1);
        let mut params = Vec::new();
        for w in breaks.windows(2) {
            for i in 0..per {
                params.push(w[0] + (w[1] - w[0]) * i as f64 / per as f64);
            }
        }
        params.push(d1);
        let first = self.control[0];
        let last = *self.control.last()?;
        let mut control = vec![first; count];
        control[count - 1] = last;
        if count > 2 {
            let inner_count = count - 2;
            let mut normal = vec![vec![0.0; inner_count]; inner_count];
            let mut rhs = vec![[0.0; 3]; inner_count];
            for t in &params {
                let row = basis.row(*t);
                let mut target = self.at(*t);
                for &(i, n) in &row {
                    let pinned = if i == 0 {
                        Some(first)
                    } else if i == count - 1 {
                        Some(last)
                    } else {
                        None
                    };
                    if let Some(q) = pinned {
                        for d in 0..3 {
                            target[d] -= n * q[d];
                        }
                    }
                }
                for &(i, ni) in row.iter().filter(|(i, _)| *i > 0 && *i < count - 1) {
                    for d in 0..3 {
                        rhs[i - 1][d] += ni * target[d];
                    }
                    for &(j, nj) in row.iter().filter(|(j, _)| *j > 0 && *j < count - 1) {
                        normal[i - 1][j - 1] += ni * nj;
                    }
                }
            }
            let solved = solve3(normal, rhs)?;
            control[1..count - 1].copy_from_slice(&solved);
        }
        Some(Lifted {
            degree,
            knots,
            control,
            rational: self.rational,
        })
    }
}

/// Solve `a · x = b` for a square `a` and three columns of right-hand side,
/// by Gaussian elimination with partial pivoting.
fn solve3(mut a: Vec<Vec<f64>>, mut b: Vec<[f64; 3]>) -> Option<Vec<[f64; 3]>> {
    let n = a.len();
    for col in 0..n {
        let pivot = (col..n).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-14 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in col + 1..n {
            let f = a[row][col] / a[col][col];
            if f == 0.0 {
                continue;
            }
            let above = a[col].clone();
            for (value, over) in a[row][col..].iter_mut().zip(&above[col..]) {
                *value -= f * over;
            }
            let top = b[col];
            for d in 0..3 {
                b[row][d] -= f * top[d];
            }
        }
    }
    let mut x = vec![[0.0; 3]; n];
    for row in (0..n).rev() {
        let mut acc = b[row];
        for k in row + 1..n {
            for d in 0..3 {
                acc[d] -= a[row][k] * x[k][d];
            }
        }
        for d in 0..3 {
            x[row][d] = acc[d] / a[row][row];
        }
    }
    x.iter()
        .all(|q| q.iter().all(|v| v.is_finite()))
        .then_some(x)
}

/// Put `made` back as spline `id`: its first and last control points keep
/// their points (one point for a closed spline laid open), `keep` says
/// which new control point is which old one (by index) where the edit left
/// it in place, and old control points no curve uses any more go.
fn store(sketch: &mut Sketch, id: Uuid, made: Lifted, keep: &dyn Fn(usize) -> Option<usize>) {
    let Some(GeometryElement::BSpline(old)) = sketch.get_geometry(id).cloned() else {
        return;
    };
    let n = made.control.len();
    let first = old.control_points[0];
    let last = if old.periodic {
        first
    } else {
        *old.control_points
            .last()
            .expect("a spline has control points")
    };
    let ids: Vec<Uuid> = made
        .control
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let at = DVec2::new(q[0] / q[2], q[1] / q[2]);
            let kept = match i {
                0 => Some(first),
                i if i == n - 1 => Some(last),
                _ if old.periodic => None,
                _ => keep(i).and_then(|j| old.control_points.get(j).copied()),
            };
            match kept {
                Some(pid) => {
                    if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(pid) {
                        p.position = Vec2D::new(at.x as f32, at.y as f32);
                    }
                    pid
                }
                None => new_point(sketch, at),
            }
        })
        .collect();
    let weights = if made.rational {
        made.control.iter().map(|q| q[2]).collect()
    } else {
        Vec::new()
    };
    let (d0, d1) = (made.knots[made.degree], made.knots[n]);
    let spline = BSpline {
        id,
        control_points: ids,
        periodic: false,
        degree: made.degree as u32,
        knots: made.knots.iter().map(|k| (k - d0) / (d1 - d0)).collect(),
        fit_points: Vec::new(),
        fit_params: Vec::new(),
        weights,
    };
    if let Some(slot) = sketch.geometry.iter_mut().find(|g| g.id() == id) {
        *slot = GeometryElement::BSpline(spline);
    }
    for pid in old.point_ids() {
        let used = sketch
            .geometry
            .iter()
            .any(|g| Sketch::curve_point_ids(g).contains(&pid));
        if !used {
            sketch.remove_geometry_cascade(&[pid]);
        }
    }
}

fn selected_splines(sketch: &Sketch, items: &HashSet<Uuid>) -> Vec<Uuid> {
    sketch
        .geometry
        .iter()
        .filter(|g| matches!(g, GeometryElement::BSpline(_)) && items.contains(&g.id()))
        .map(|g| g.id())
        .collect()
}

/// Raise (`by` > 0) or lower the degree of each selected spline. Raising
/// keeps the curve, each knot standing once more so it stays as smooth;
/// lowering fits the nearest curve of the lower degree.
pub fn change_degree(sketch: &mut Sketch, items: &HashSet<Uuid>, by: i32) -> ToolEffect {
    let mut changed = 0;
    for id in selected_splines(sketch, items) {
        let Some(GeometryElement::BSpline(spline)) = sketch.get_geometry(id).cloned() else {
            continue;
        };
        let Some(lift) = lifted(sketch, &spline) else {
            continue;
        };
        let degree = (lift.degree as i32 + by).clamp(1, MAX_DEGREE as i32) as usize;
        if degree == lift.degree {
            continue;
        }
        let inner: Vec<(f64, usize)> = lift
            .interior()
            .into_iter()
            .map(|(k, m)| {
                let m = if by > 0 {
                    m + 1
                } else {
                    m.saturating_sub(1).max(1)
                };
                (k, m.min(degree))
            })
            .collect();
        let Some(made) = lift.refit(degree, &inner) else {
            continue;
        };
        store(sketch, id, made, &|_| None);
        changed += 1;
    }
    if changed == 0 {
        return ToolEffect::log("Select a spline whose degree can change (1 to 5)");
    }
    ToolEffect::changed(format!(
        "{} the degree of {changed} spline(s)",
        if by > 0 { "Raised" } else { "Lowered" }
    ))
}

/// The knot values inside a spline's domain and how many times each
/// stands, over its parameter as stored.
pub fn knots_of(sketch: &Sketch, spline: Uuid) -> Vec<(f64, usize)> {
    let Some(GeometryElement::BSpline(b)) = sketch.get_geometry(spline) else {
        return Vec::new();
    };
    lifted(sketch, b).map(|l| l.interior()).unwrap_or_default()
}

/// The parameter of the point of `spline` nearest `at`, over the domain
/// the edits here work in (a closed spline's laid open).
pub fn param_near(sketch: &Sketch, spline: Uuid, at: Vec2D) -> Option<f64> {
    let Some(GeometryElement::BSpline(b)) = sketch.get_geometry(spline) else {
        return None;
    };
    let lift = lifted(sketch, b)?;
    let (d0, d1) = (lift.knots[lift.degree], lift.knots[lift.control.len()]);
    let target = dv(at);
    let point = |t: f64| {
        let q = lift.at(t);
        DVec2::new(q[0] / q[2], q[1] / q[2])
    };
    let steps = 400;
    let best = (0..=steps)
        .map(|i| d0 + (d1 - d0) * i as f64 / steps as f64)
        .min_by(|a, b| {
            (point(*a) - target)
                .length()
                .total_cmp(&(point(*b) - target).length())
        })?;
    let h = (d1 - d0) / steps as f64;
    let (mut lo, mut hi) = ((best - h).max(d0), (best + h).min(d1));
    for _ in 0..60 {
        let (m1, m2) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
        if (point(m1) - target).length() < (point(m2) - target).length() {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    Some((lo + hi) / 2.0)
}

/// One more knot in `spline` at parameter `t`, the curve unchanged.
pub fn add_knot(sketch: &mut Sketch, spline: Uuid, t: f64) -> ToolEffect {
    let Some(GeometryElement::BSpline(b)) = sketch.get_geometry(spline).cloned() else {
        return ToolEffect::log("Click a spline to insert a knot");
    };
    let Some(mut lift) = lifted(sketch, &b) else {
        return ToolEffect::none();
    };
    let p = lift.degree;
    let (d0, d1) = (lift.knots[p], lift.knots[lift.control.len()]);
    let eps = (d1 - d0) * 1e-6;
    if t <= d0 + eps || t >= d1 - eps {
        return ToolEffect::log("A knot goes inside the spline, not at its ends");
    }
    let standing = lift
        .knots
        .iter()
        .filter(|k| (**k - t).abs() < 1e-12)
        .count();
    if standing >= p {
        return ToolEffect::log("That knot already stands as often as the degree allows");
    }
    // The control points before the span insertion changes stay, those
    // after it move up one.
    let k = lift
        .knots
        .iter()
        .rposition(|u| *u <= t)
        .unwrap_or(p)
        .min(lift.control.len() - 1);
    insert_knot(&mut lift.knots, &mut lift.control, p, t);
    let keep = move |i: usize| {
        if i + p <= k {
            Some(i)
        } else if i > k {
            Some(i - 1)
        } else {
            None
        }
    };
    store(sketch, spline, lift, &keep);
    ToolEffect::changed("Inserted a knot")
}

/// Make the knot at `value` stand `multiplicity` times (1 up to the
/// degree): more is exact, fewer fits the nearest smoother curve.
pub fn set_multiplicity(
    sketch: &mut Sketch,
    spline: Uuid,
    value: f64,
    multiplicity: usize,
) -> ToolEffect {
    let Some(GeometryElement::BSpline(b)) = sketch.get_geometry(spline).cloned() else {
        return ToolEffect::none();
    };
    let Some(lift) = lifted(sketch, &b) else {
        return ToolEffect::none();
    };
    let inner = lift.interior();
    let Some(&(k, now)) = inner.iter().find(|(k, _)| (*k - value).abs() < 1e-9) else {
        return ToolEffect::log("The spline has no knot there");
    };
    let multiplicity = multiplicity.clamp(1, lift.degree);
    if multiplicity == now {
        return ToolEffect::none();
    }
    if multiplicity > now {
        let mut effect = ToolEffect::none();
        for _ in now..multiplicity {
            effect = add_knot(sketch, spline, k);
        }
        return effect;
    }
    let inner: Vec<(f64, usize)> = inner
        .into_iter()
        .map(|(v, m)| if v == k { (v, multiplicity) } else { (v, m) })
        .collect();
    match lift.refit(lift.degree, &inner) {
        Some(made) => {
            store(sketch, spline, made, &|_| None);
            ToolEffect::changed(format!("The knot now stands {multiplicity} time(s)"))
        }
        None => ToolEffect::none(),
    }
}

/// Remove the knot at `value` altogether, fitting the nearest curve
/// without it.
pub fn remove_knot(sketch: &mut Sketch, spline: Uuid, value: f64) -> ToolEffect {
    let Some(GeometryElement::BSpline(b)) = sketch.get_geometry(spline).cloned() else {
        return ToolEffect::none();
    };
    let Some(lift) = lifted(sketch, &b) else {
        return ToolEffect::none();
    };
    let inner: Vec<(f64, usize)> = lift
        .interior()
        .into_iter()
        .filter(|(v, _)| (*v - value).abs() >= 1e-9)
        .collect();
    match lift.refit(lift.degree, &inner) {
        Some(made) => {
            store(sketch, spline, made, &|_| None);
            ToolEffect::changed("Removed a knot")
        }
        None => ToolEffect::log("Too few control points would remain"),
    }
}

/// Weigh control point `point` of `spline` by `weight`: more pulls the
/// curve toward it, less lets it go.
pub fn set_weight(sketch: &mut Sketch, spline: Uuid, point: Uuid, weight: f64) -> ToolEffect {
    if !(weight > 0.0 && weight.is_finite()) {
        return ToolEffect::log("A weight is more than zero");
    }
    let Some(GeometryElement::BSpline(b)) = sketch.get_geometry_mut(spline) else {
        return ToolEffect::none();
    };
    let Some(index) = b.control_points.iter().position(|p| *p == point) else {
        return ToolEffect::log("That point is not one of the spline's control points");
    };
    if b.weights.len() != b.control_points.len() {
        b.weights = vec![1.0; b.control_points.len()];
    }
    b.weights[index] = weight;
    // A closed spline that names a point twice weighs it once.
    let named: Vec<usize> = b
        .control_points
        .iter()
        .enumerate()
        .filter(|(_, p)| **p == point)
        .map(|(i, _)| i)
        .collect();
    for i in named {
        b.weights[i] = weight;
    }
    if b.weights.iter().all(|w| *w == 1.0) {
        b.weights.clear();
    }
    ToolEffect::changed(format!("Weight {weight:.3}"))
}

/// The weight of control point `point` of `spline`.
pub fn weight_of(sketch: &Sketch, spline: Uuid, point: Uuid) -> Option<f64> {
    let GeometryElement::BSpline(b) = sketch.get_geometry(spline)? else {
        return None;
    };
    let index = b.control_points.iter().position(|p| *p == point)?;
    Some(b.weights.get(index).copied().unwrap_or(1.0))
}

/// A knot inserted into the spline nearest `cursor`, within `tol`, where
/// it passes nearest it: the knot tool's click.
pub fn knot_at_click(sketch: &mut Sketch, cursor: Vec2D, tol: f32) -> ToolEffect {
    let near = sketch
        .geometry
        .iter()
        .filter(|g| matches!(g, GeometryElement::BSpline(_)))
        .filter_map(|g| crate::snap::distance_to_element(sketch, g, cursor).map(|d| (g.id(), d)))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id);
    let Some(spline) = near else {
        return ToolEffect::log("Click a spline where the knot goes");
    };
    match param_near(sketch, spline, cursor) {
        Some(t) => add_knot(sketch, spline, t),
        None => ToolEffect::none(),
    }
}

/// How far a comb's longest tooth reaches, as a share of the spline's
/// size.
const COMB_REACH: f64 = 0.25;
/// Teeth of a comb along each span between knots.
const COMB_TEETH: usize = 16;

/// A spline's curvature comb: from points along it, teeth square to it as
/// long as it is curved there (on the side it turns away from), and the
/// line through their tips; in sketch units.
pub fn comb(sketch: &Sketch, spline: Uuid) -> Vec<(Vec2D, Vec2D)> {
    let Some(GeometryElement::BSpline(b)) = sketch.get_geometry(spline) else {
        return Vec::new();
    };
    let Some(lift) = lifted(sketch, b) else {
        return Vec::new();
    };
    let point = |t: f64| {
        let q = lift.at(t);
        DVec2::new(q[0] / q[2], q[1] / q[2])
    };
    let (d0, d1) = (lift.knots[lift.degree], lift.knots[lift.control.len()]);
    let spans = lift.interior().len() + 1;
    let n = COMB_TEETH * spans;
    let h = (d1 - d0) * 1e-4;
    let teeth: Vec<(DVec2, DVec2)> = (0..=n)
        .map(|i| {
            let t = (d0 + (d1 - d0) * i as f64 / n as f64).clamp(d0 + h, d1 - h);
            let (a, p, c) = (point(t - h), point(t), point(t + h));
            let d1v = (c - a) / (2.0 * h);
            let d2v = (c - 2.0 * p + a) / (h * h);
            let speed = d1v.length().max(1e-12);
            let curvature = d1v.perp_dot(d2v) / speed.powi(3);
            // Out of the side the curve turns away from.
            (p, -d1v.perp() / speed * curvature)
        })
        .collect();
    let most = teeth.iter().map(|(_, k)| k.length()).fold(0.0, f64::max);
    let (lo, hi) = lift.control.iter().fold(
        (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
        |(lo, hi), q| {
            let p = DVec2::new(q[0] / q[2], q[1] / q[2]);
            (lo.min(p), hi.max(p))
        },
    );
    if most < 1e-12 {
        return Vec::new();
    }
    let scale = (hi - lo).length() * COMB_REACH / most;
    let v = |p: DVec2| Vec2D::new(p.x as f32, p.y as f32);
    let tips: Vec<DVec2> = teeth.iter().map(|(p, k)| *p + *k * scale).collect();
    let mut out: Vec<(Vec2D, Vec2D)> = teeth
        .iter()
        .zip(&tips)
        .map(|((p, _), tip)| (v(*p), v(*tip)))
        .collect();
    out.extend(tips.windows(2).map(|w| (v(w[0]), v(w[1]))));
    out
}

/// Where a spline's knots inside its ends sit on it, with how many times
/// each stands.
pub fn knot_points(sketch: &Sketch, spline: Uuid) -> Vec<(Vec2D, usize)> {
    let Some(GeometryElement::BSpline(b)) = sketch.get_geometry(spline) else {
        return Vec::new();
    };
    let Some(lift) = lifted(sketch, b) else {
        return Vec::new();
    };
    lift.interior()
        .into_iter()
        .map(|(k, m)| {
            let q = lift.at(k);
            (Vec2D::new((q[0] / q[2]) as f32, (q[1] / q[2]) as f32), m)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Arc, Circle, Line};

    fn pt(sketch: &mut Sketch, x: f32, y: f32) -> Uuid {
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
    }

    fn only_spline(sketch: &Sketch) -> BSpline {
        sketch
            .geometry
            .iter()
            .find_map(|g| match g {
                GeometryElement::BSpline(b) => Some(b.clone()),
                _ => None,
            })
            .expect("a spline")
    }

    fn samples(sketch: &Sketch, spline: &BSpline) -> Vec<glam::Vec2> {
        crate::measure::curve_samples(sketch, spline.id).unwrap()
    }

    #[test]
    fn an_arc_becomes_a_spline_on_the_same_circle() {
        let mut sketch = Sketch::new("t");
        let c = pt(&mut sketch, 1.0, 2.0);
        let s = pt(&mut sketch, 6.0, 2.0);
        let e = pt(&mut sketch, 1.0, -3.0);
        // Three quarters of a turn, counter-clockwise.
        let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, 5.0)));
        let effect = to_bspline(&mut sketch, &[arc].into_iter().collect());
        assert!(effect.changed);
        assert!(sketch.get_geometry(arc).is_none());
        let spline = only_spline(&sketch);
        assert_eq!(spline.control_points[0], s);
        assert_eq!(*spline.control_points.last().unwrap(), e);
        for p in samples(&sketch, &spline) {
            let r = (p - glam::Vec2::new(1.0, 2.0)).length();
            assert!((r - 5.0).abs() < 1e-4, "{p:?} at {r}");
        }
    }

    #[test]
    fn a_circle_becomes_a_closed_spline() {
        let mut sketch = Sketch::new("t");
        let c = pt(&mut sketch, 0.0, 0.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(c, 4.0)));
        to_bspline(&mut sketch, &[circle].into_iter().collect());
        let spline = only_spline(&sketch);
        assert_eq!(
            spline.control_points[0],
            *spline.control_points.last().unwrap()
        );
        for p in samples(&sketch, &spline) {
            assert!((p.length() - 4.0).abs() < 1e-4);
        }
        let wires = crate::profile::extract_wires(&sketch).unwrap();
        assert_eq!(wires.len(), 1);
    }

    /// An open cubic over five control points.
    fn wavy(sketch: &mut Sketch) -> Uuid {
        let pts: Vec<Uuid> = [
            (0.0, 0.0),
            (5.0, 8.0),
            (10.0, -4.0),
            (15.0, 6.0),
            (20.0, 0.0),
        ]
        .iter()
        .map(|&(x, y)| pt(sketch, x, y))
        .collect();
        sketch.add_geometry(GeometryElement::BSpline(BSpline::new(pts, false)))
    }

    fn off_by(before: &[glam::Vec2], sketch: &Sketch, spline: &BSpline) -> f32 {
        let after = samples(sketch, spline);
        after
            .iter()
            .zip(before)
            .map(|(a, b)| (*a - *b).length())
            .fold(0.0, f32::max)
    }

    #[test]
    fn raising_the_degree_or_inserting_a_knot_keeps_the_curve() {
        let mut sketch = Sketch::new("t");
        let id = wavy(&mut sketch);
        let before = samples(&sketch, &only_spline(&sketch));
        let items: HashSet<Uuid> = [id].into_iter().collect();
        assert!(change_degree(&mut sketch, &items, 1).changed);
        let raised = only_spline(&sketch);
        assert_eq!(raised.degree, 4);
        assert!(off_by(&before, &sketch, &raised) < 1e-3);
        let t = param_near(&sketch, id, Vec2D::new(7.0, 3.0)).unwrap();
        assert!(add_knot(&mut sketch, id, t).changed);
        let knotted = only_spline(&sketch);
        assert_eq!(
            knotted.control_points.len(),
            raised.control_points.len() + 1
        );
        assert!(off_by(&before, &sketch, &knotted) < 1e-3);
        // Standing as often as the degree, the knot is a corner the curve
        // may turn at, and still the same curve.
        let (value, _) = knots_of(&sketch, id)
            .into_iter()
            .find(|(k, _)| (*k - t).abs() < 1e-6)
            .unwrap();
        assert!(set_multiplicity(&mut sketch, id, value, 4).changed);
        assert_eq!(
            knots_of(&sketch, id)
                .iter()
                .find(|(k, _)| (*k - value).abs() < 1e-6)
                .unwrap()
                .1,
            4
        );
        assert!(off_by(&before, &sketch, &only_spline(&sketch)) < 1e-3);
    }

    #[test]
    fn lowering_the_degree_stays_near_and_keeps_the_ends() {
        let mut sketch = Sketch::new("t");
        let id = wavy(&mut sketch);
        let first = only_spline(&sketch).control_points[0];
        let items: HashSet<Uuid> = [id].into_iter().collect();
        assert!(change_degree(&mut sketch, &items, -1).changed);
        let lowered = only_spline(&sketch);
        assert_eq!(lowered.degree, 2);
        assert_eq!(lowered.control_points[0], first);
        let end = sketch
            .point_position(*lowered.control_points.last().unwrap())
            .unwrap();
        assert!((end.x - 20.0).abs() < 1e-4 && end.y.abs() < 1e-4);
    }

    #[test]
    fn a_weight_pulls_the_curve_toward_its_point() {
        let mut sketch = Sketch::new("t");
        let id = wavy(&mut sketch);
        let spline = only_spline(&sketch);
        let peak = spline.control_points[1];
        let near = |sketch: &Sketch| {
            samples(sketch, &only_spline(sketch))
                .iter()
                .map(|p| (*p - glam::Vec2::new(5.0, 8.0)).length())
                .fold(f32::MAX, f32::min)
        };
        let before = near(&sketch);
        assert!(set_weight(&mut sketch, id, peak, 4.0).changed);
        assert!(near(&sketch) < before * 0.7);
        assert_eq!(weight_of(&sketch, id, peak), Some(4.0));
        assert!(set_weight(&mut sketch, id, peak, 1.0).changed);
        assert!(only_spline(&sketch).weights.is_empty());
    }

    #[test]
    fn a_line_becomes_a_straight_spline_between_its_ends() {
        let mut sketch = Sketch::new("t");
        let a = pt(&mut sketch, 0.0, 0.0);
        let b = pt(&mut sketch, 9.0, 3.0);
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        to_bspline(&mut sketch, &[line].into_iter().collect());
        let spline = only_spline(&sketch);
        assert_eq!((spline.control_points[0], spline.control_points[3]), (a, b));
        for p in samples(&sketch, &spline) {
            assert!((p.y - p.x / 3.0).abs() < 1e-4);
        }
    }

    #[test]
    fn a_comb_stands_out_where_the_curve_bends() {
        let mut sketch = Sketch::new("t");
        let c = pt(&mut sketch, 0.0, 0.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(c, 4.0)));
        to_bspline(&mut sketch, &[circle].into_iter().collect());
        let id = only_spline(&sketch).id;
        let teeth = comb(&sketch, id);
        assert!(!teeth.is_empty());
        // A circle's curvature is the same all round: every tooth as long,
        // pointing out of it.
        let n = COMB_TEETH * (knots_of(&sketch, id).len() + 1) + 1;
        let lengths: Vec<f32> = teeth[..n]
            .iter()
            .map(|(a, b)| (b.to_glam() - a.to_glam()).length())
            .collect();
        let (lo, hi) = lengths
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), l| (lo.min(*l), hi.max(*l)));
        assert!(hi - lo < 1e-2 * hi, "{lo}..{hi}");
        for (base, tip) in &teeth[..n] {
            assert!(tip.to_glam().length() > base.to_glam().length());
        }
        assert_eq!(knot_points(&sketch, id).len(), 3);
    }
}
