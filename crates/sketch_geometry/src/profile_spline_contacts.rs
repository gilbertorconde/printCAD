//! Profile spline crossings, tangencies, overlaps and self-intersections.
//!
//! Probe both carriers for chord crossings and local distance minima. Refine
//! crossings by Newton steps in both arc lengths and tangencies by golden-section
//! distance search. The common `profile_geometry` filter decides contact by
//! membership on both curves within tolerance, as for analytic pairs.

use glam::DVec2;

use super::profile_curve::ProfileCurve;
use crate::trim_curve::TrimCurve;
use crate::world::{WorldLength, WorldOffset, WorldPoint};

/// Probes per full circular turn.
const ROUND_PROBES: f64 = 64.0;

/// Curve samples with arc length from the start.
fn samples(curve: &ProfileCurve) -> Vec<(WorldOffset, WorldPoint)> {
    match curve {
        ProfileCurve::Spline(spline) => spline.samples(),
        ProfileCurve::Analytic(analytic) => {
            let length = analytic.length().0;
            let count = match analytic {
                TrimCurve::Segment { .. } => 1,
                TrimCurve::Arc { radius, .. } | TrimCurve::Circle { radius, .. } => {
                    (length / radius.0 / std::f64::consts::TAU * ROUND_PROBES)
                        .ceil()
                        .max(4.0) as usize
                }
            };
            (0..=count)
                .map(|i| {
                    let s = WorldOffset::new(length * i as f64 / count as f64);
                    (s, curve.pos_at(s))
                })
                .collect()
        }
    }
}

/// Segment intersection parameters `(u, v) ∈ [0, 1]²`; none for parallel segments.
fn chord_hit(p0: DVec2, p1: DVec2, q0: DVec2, q1: DVec2) -> Option<(f64, f64)> {
    let (r, s) = (p1 - p0, q1 - q0);
    let denom = r.perp_dot(s);
    if denom == 0.0 {
        return None;
    }
    let d = q0 - p0;
    let (u, v) = (d.perp_dot(s) / denom, d.perp_dot(r) / denom);
    ((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)).then_some((u, v))
}

/// Normalize s by open clamping or closed wrapping.
fn on_param(curve: &ProfileCurve, s: f64) -> f64 {
    let length = curve.length().0;
    if curve.closed() {
        s.rem_euclid(length)
    } else {
        s.clamp(0.0, length)
    }
}

/// Newton in arc lengths: `A(sa) = B(sb)` has Jacobian `[Ta | −Tb]` with unit
/// tangents. A singular tangency or nonconvergence returns `None`, leaving
/// the candidate to the tangency search.
fn refine(
    a: &ProfileCurve,
    b: &ProfileCurve,
    mut sa: f64,
    mut sb: f64,
    scale: f64,
) -> Option<(f64, f64, DVec2)> {
    for _ in 0..40 {
        let (pa, pb) = (
            a.pos_at(WorldOffset::new(sa)).0,
            b.pos_at(WorldOffset::new(sb)).0,
        );
        let f = pa - pb;
        if f.length() <= 1e-12 * scale {
            return Some((sa, sb, (pa + pb) * 0.5));
        }
        let ta = a.direction(WorldOffset::new(sa)).0;
        let tb = b.direction(WorldOffset::new(sb)).0;
        let det = -ta.perp_dot(tb);
        if det.abs() < 1e-9 {
            return None;
        }
        // Solve [ta −tb]·[dsa dsb]ᵀ = −f by Cramer's rule.
        let dsa = (-f).perp_dot(-tb) / det;
        let dsb = ta.perp_dot(-f) / det;
        sa = on_param(a, sa + dsa);
        sb = on_param(b, sb + dsb);
    }
    None
}

/// Tangency candidate: the midpoint between a local distance minimum on a
/// in arc-length interval [lo, hi] and the nearest point on b.
fn closest_between(a: &ProfileCurve, b: &ProfileCurve, mut lo: f64, mut hi: f64) -> DVec2 {
    const PHI: f64 = 0.618_033_988_749_895;
    let dist = |s: f64| b.distance(a.pos_at(WorldOffset::new(s))).0;
    for _ in 0..60 {
        let m1 = hi - (hi - lo) * PHI;
        let m2 = lo + (hi - lo) * PHI;
        if dist(m1) <= dist(m2) {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    let p = a.pos_at(WorldOffset::new((lo + hi) * 0.5));
    let q = b.pos_at(b.project(p));
    (p.0 + q.0) * 0.5
}

/// Tangency probes on a: local distance minima to b no farther than the probe step.
fn touch_candidates(
    a: &ProfileCurve,
    sa: &[(WorldOffset, WorldPoint)],
    b: &ProfileCurve,
    out: &mut Vec<DVec2>,
) {
    let d: Vec<f64> = sa.iter().map(|&(_, p)| b.distance(p).0).collect();
    for i in 0..sa.len() {
        let (prev, next) = (i.saturating_sub(1), (i + 1).min(sa.len() - 1));
        let step = sa[next].1.distance(sa[prev].1).0;
        if d[i] <= d[prev] && d[i] <= d[next] && d[i] <= step {
            out.push(closest_between(a, b, sa[prev].0.0, sa[next].0.0));
        }
    }
}

/// Contact candidates for a pair containing at least one spline.
pub(crate) fn crossing_candidates(a: &ProfileCurve, b: &ProfileCurve) -> Vec<DVec2> {
    let (sa, sb) = (samples(a), samples(b));
    let scale = 1.0
        + sa.iter()
            .chain(&sb)
            .map(|(_, p)| p.0.abs().max_element())
            .fold(0.0, f64::max);
    let mut out = Vec::new();
    for pa in sa.windows(2) {
        for pb in sb.windows(2) {
            if let Some((u, v)) = chord_hit(pa[0].1.0, pa[1].1.0, pb[0].1.0, pb[1].1.0) {
                let s_a = pa[0].0.0 + (pa[1].0.0 - pa[0].0.0) * u;
                let s_b = pb[0].0.0 + (pb[1].0.0 - pb[0].0.0) * v;
                out.push(
                    refine(a, b, s_a, s_b, scale)
                        .map_or_else(|| pa[0].1.0.lerp(pa[1].1.0, u), |(_, _, p)| p),
                );
            }
        }
    }
    touch_candidates(a, &sa, b, &mut out);
    touch_candidates(b, &sb, a, &mut out);
    out
}

/// Detect overlap between consecutive contacts in spline travel order by
/// testing the spline segment midpoint on the other curve.
pub(crate) fn overlaps(
    spline: &ProfileCurve,
    other: &ProfileCurve,
    points: &[DVec2],
    eps: WorldLength,
) -> bool {
    let mut along: Vec<f64> = points
        .iter()
        .map(|&p| spline.project(WorldPoint::from_dvec2(p)).0)
        .collect();
    along.sort_by(f64::total_cmp);
    along.windows(2).any(|w| {
        w[1] - w[0] > eps.0
            && other.distance(spline.pos_at(WorldOffset::new((w[0] + w[1]) * 0.5))) <= eps
    })
}

/// Separation of two arc lengths, using the shorter way around a closed curve.
fn apart(curve: &ProfileCurve, a: f64, b: f64) -> f64 {
    let d = (a - b).abs();
    if curve.closed() {
        d.min(curve.length().0 - d)
    } else {
        d
    }
}

/// All passes through p within eps, including both sides of a self-crossing.
/// Return the nearest pass first, then local probe minima refined by golden
/// sections, as for tangencies. A cut must affect every pass.
pub fn passes_through(curve: &ProfileCurve, p: WorldPoint, eps: WorldLength) -> Vec<WorldOffset> {
    let mut out = vec![curve.project(p)];
    let ProfileCurve::Spline(_) = curve else {
        return out;
    };
    let s = samples(curve);
    let d: Vec<f64> = s.iter().map(|&(_, q)| q.distance(p).0).collect();
    for i in 0..s.len() {
        let (prev, next) = (i.saturating_sub(1), (i + 1).min(s.len() - 1));
        let step = s[next].1.distance(s[prev].1).0;
        if !(d[i] <= d[prev] && d[i] <= d[next] && d[i] <= step) {
            continue;
        }
        const PHI: f64 = 0.618_033_988_749_895;
        let dist = |x: f64| curve.pos_at(WorldOffset::new(x)).distance(p).0;
        let (mut lo, mut hi) = (s[prev].0.0, s[next].0.0);
        for _ in 0..80 {
            let m1 = hi - (hi - lo) * PHI;
            let m2 = lo + (hi - lo) * PHI;
            if dist(m1) <= dist(m2) {
                hi = m2;
            } else {
                lo = m1;
            }
        }
        let at = (lo + hi) * 0.5;
        // Distinct passes differ in arc length by more than eps; duplicate
        // probes of one pass converge to the same length.
        if dist(at) <= eps.0 && out.iter().all(|o| apart(curve, o.0, at) > eps.0) {
            out.push(WorldOffset::new(at));
        }
    }
    out
}

/// First spline self-intersection (see [`self_crossings`]).
pub fn self_crossing(curve: &ProfileCurve, eps: WorldLength) -> Option<WorldPoint> {
    self_crossings(curve, eps).into_iter().next()
}

/// Refine nonadjacent chord crossings by Newton in two arc lengths of the
/// same curve. Merge candidates within tolerance from neighboring chord pairs.
pub fn self_crossings(curve: &ProfileCurve, eps: WorldLength) -> Vec<WorldPoint> {
    let mut found: Vec<WorldPoint> = Vec::new();
    let s = samples(curve);
    let scale = 1.0
        + s.iter()
            .map(|(_, p)| p.0.abs().max_element())
            .fold(0.0, f64::max);
    let chords = s.len() - 1;
    for i in 0..chords {
        for j in i + 2..chords {
            // First and last chords of a closed curve are adjacent across the seam.
            if curve.closed() && i == 0 && j == chords - 1 {
                continue;
            }
            let Some((u, v)) = chord_hit(s[i].1.0, s[i + 1].1.0, s[j].1.0, s[j + 1].1.0) else {
                continue;
            };
            let s1 = s[i].0.0 + (s[i + 1].0.0 - s[i].0.0) * u;
            let s2 = s[j].0.0 + (s[j + 1].0.0 - s[j].0.0) * v;
            let Some((r1, r2, at)) = refine(curve, curve, s1, s2, scale) else {
                continue;
            };
            // Coincident ends of an open curve form a contour join, not a crossing.
            let (first, last) = (r1.min(r2), r1.max(r2));
            if !curve.closed() && first <= eps.0 && last >= curve.length().0 - eps.0 {
                continue;
            }
            // Refinement may collapse both lengths to one pass; that is not a crossing.
            let at = WorldPoint::from_dvec2(at);
            if apart(curve, r1, r2) > eps.0 && found.iter().all(|f| f.distance(at) > eps) {
                found.push(at);
            }
        }
    }
    found
}
