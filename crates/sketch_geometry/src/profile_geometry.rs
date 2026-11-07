//! Analytic profile-boundary operations. Local-coordinate area and winding
//! avoid cancellation of large areas when the sketch is translated.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use glam::DVec2;

use super::profile_curve::ProfileCurve;
use super::profile_spline_contacts::{crossing_candidates, overlaps};
use crate::arc::{ArcGeom, arc_sweeps_overlap, chord_step_for_sagitta};
use crate::bspline::SplineParam;
use crate::bspline_window::SplineWindow;
use crate::intersections::{
    circle_circle_intersections, seg_circle_intersections, seg_seg_intersection,
};
use crate::primitives::segments_overlap_with_tolerance;
use crate::roundoff::sqrt_of_proven_nonnegative;
use crate::trim_curve::TrimCurve;
use crate::types::Angle;
use crate::world::{WorldArea, WorldLength, WorldPoint};

pub enum PairContact {
    Overlap,
    Points(Vec<WorldPoint>),
}

#[derive(Clone, Copy)]
struct CircularPiece {
    center: WorldPoint,
    radius: WorldLength,
    geom: ArcGeom,
}

fn circular(curve: &TrimCurve) -> Option<CircularPiece> {
    match *curve {
        TrimCurve::Segment { .. } => None,
        TrimCurve::Arc {
            center,
            radius,
            geom,
        } => Some(CircularPiece {
            center,
            radius,
            geom,
        }),
        TrimCurve::Circle { center, radius } => Some(CircularPiece {
            center,
            radius,
            geom: ArcGeom::new(Angle(0.0), Angle(TAU)),
        }),
    }
}

/// World-space chord approximation, shared with derived polygon operations.
/// Full circles use a multiple of four samples so their bbox and axial symmetry
/// do not depend on where the tessellation happens to stop.
pub fn polyline(
    curve: &ProfileCurve,
    reversed: bool,
    error: WorldLength,
) -> Result<Vec<DVec2>, String> {
    let piece = Piece::from_curve(curve, reversed);
    let count = match piece {
        Piece::Line { .. } => 1.0,
        Piece::Spline { spline, reversed } => return spline.polyline(reversed, error),
        Piece::Arc(CircularPiece { radius, geom, .. }) => {
            let sweep = geom.sweep_angle.radians();
            let step = chord_step_for_sagitta(radius, error).radians();
            let count = (sweep.abs() / step).ceil();
            if sweep.abs() == TAU {
                (count / 4.0).ceil() * 4.0
            } else {
                // A curved side and its closing chord can bound material even
                // when the entire bulge fits within the chord error. Keep an
                // interior sample so such a loop does not become a two-point
                // ring before polygon triangulation or pattern clipping.
                count.max(2.0)
            }
        }
    };
    if count > 100_000.0 {
        return Err(
            "Curve tolerance requires more than 100000 segments; increase tolerance".into(),
        );
    }
    let count = count as usize;
    Ok((0..=count)
        .map(|i| piece.at(i as f64 / count as f64))
        .collect())
}

/// `ends` are authoritative model endpoints. The host keeps arc ends on their
/// circle and derives angles from those points. Use stored vertices for shared
/// contacts rather than introducing rounding by reconstructing them trigonometrically.
fn on_curve(
    curve: &ProfileCurve,
    ends: Option<[WorldPoint; 2]>,
    at: DVec2,
    eps: WorldLength,
) -> bool {
    let near_end = ends.is_some_and(|ends| ends.into_iter().any(|end| at.distance(end.0) <= eps.0));
    let curve = match curve {
        ProfileCurve::Analytic(curve) => curve,
        ProfileCurve::Spline(_) => {
            return near_end || curve.distance(WorldPoint::from_dvec2(at)) <= eps;
        }
    };
    let eps = eps.0;
    match *curve {
        TrimCurve::Segment { a, b } => {
            let (a, b) = (a.0, b.0);
            let t = ((at - a).dot(b - a) / a.distance_squared(b)).clamp(0.0, 1.0);
            at.distance(a.lerp(b, t)) <= eps
        }
        TrimCurve::Circle { center, radius } => (at.distance(center.0) - radius.0).abs() <= eps,
        TrimCurve::Arc {
            center,
            radius,
            geom,
        } => {
            let delta = at - center.0;
            let piece = Piece::from_analytic(*curve, false);
            ((delta.length() - radius.0).abs() <= eps
                && geom.contains_angle(Angle(delta.to_angle())))
                || at.distance(piece.at(0.0)) <= eps
                || at.distance(piece.at(1.0)) <= eps
                || near_end
        }
    }
}

pub fn contacts(
    a: &ProfileCurve,
    a_ends: Option<[WorldPoint; 2]>,
    b: &ProfileCurve,
    b_ends: Option<[WorldPoint; 2]>,
    eps: WorldLength,
    minimum_segment_length: WorldLength,
) -> PairContact {
    let bounds = |curve: &ProfileCurve, ends: Option<[WorldPoint; 2]>| {
        // Positive rational weights keep the entire spline, including a
        // trimmed window, in the control polygon's convex hull. Analytic
        // arcs need only their ends and the cardinal extrema on their sweep.
        // Include stored endpoints too: they can differ by the solver tolerance.
        let points = match curve {
            ProfileCurve::Spline(s) => s.curve().control_polygon(),
            ProfileCurve::Analytic(TrimCurve::Segment { a, b }) => vec![*a, *b],
            ProfileCurve::Analytic(TrimCurve::Circle { center, radius }) => {
                vec![
                    WorldPoint(center.0 - DVec2::splat(radius.0)),
                    WorldPoint(center.0 + DVec2::splat(radius.0)),
                ]
            }
            ProfileCurve::Analytic(TrimCurve::Arc {
                center,
                radius,
                geom,
            }) => {
                let mut points = vec![
                    WorldPoint(center.0 + DVec2::from_angle(geom.start_angle.0) * radius.0),
                    WorldPoint(center.0 + DVec2::from_angle(geom.end_angle().0) * radius.0),
                ];
                for angle in [0.0, FRAC_PI_2, PI, PI + FRAC_PI_2] {
                    if geom.contains_angle(Angle(angle)) {
                        points.push(WorldPoint(center.0 + DVec2::from_angle(angle) * radius.0));
                    }
                }
                points
            }
        };
        let mut min = DVec2::splat(f64::INFINITY);
        let mut max = DVec2::splat(f64::NEG_INFINITY);
        for p in points.into_iter().chain(ends.into_iter().flatten()) {
            min = min.min(p.0);
            max = max.max(p.0);
        }
        (min, max)
    };
    let (amin, amax) = bounds(a, a_ends);
    let (bmin, bmax) = bounds(b, b_ends);
    // A midpoint candidate may be within eps of both curves while they are
    // 2*eps apart. Keep that same acceptance envelope in the broad phase.
    let reach = DVec2::splat(2.0 * eps.0);
    if (amax + reach).cmplt(bmin).any() || (bmax + reach).cmplt(amin).any() {
        return PairContact::Points(Vec::new());
    }
    let mut points = match (a, b) {
        (ProfileCurve::Analytic(a), ProfileCurve::Analytic(b)) => {
            match analytic_candidates(a, b, eps, minimum_segment_length) {
                Some(points) => points,
                None => return PairContact::Overlap,
            }
        }
        _ => crossing_candidates(a, b),
    };
    // Detect endpoint and T contacts within solve precision, using stored endpoints.
    for (curve, ends) in [(a, a_ends), (b, b_ends)] {
        if !curve.closed() {
            let ends = ends.expect("open profile edge must store endpoints");
            points.extend(ends.map(|end| end.0));
        }
    }
    points.retain(|&p| on_curve(a, a_ends, p, eps) && on_curve(b, b_ends, p, eps));
    let mut unique: Vec<DVec2> = Vec::new();
    for point in points {
        if !unique.iter().any(|p| p.distance(point) <= eps.0) {
            unique.push(point);
        }
    }
    let spline_pair = match (a, b) {
        (ProfileCurve::Spline(_), _) => Some((a, b)),
        (_, ProfileCurve::Spline(_)) => Some((b, a)),
        _ => None,
    };
    if let Some((spline, other)) = spline_pair
        && overlaps(spline, other, &unique, eps)
    {
        return PairContact::Overlap;
    }
    PairContact::Points(unique.into_iter().map(WorldPoint::from_dvec2).collect())
}

/// Analytic contact candidates; `None` denotes overlap.
fn analytic_candidates(
    a: &TrimCurve,
    b: &TrimCurve,
    eps: WorldLength,
    minimum_segment_length: WorldLength,
) -> Option<Vec<DVec2>> {
    Some(match (*a, *b) {
        (TrimCurve::Segment { a: a0, b: a1 }, TrimCurve::Segment { a: b0, b: b1 }) => {
            if segments_overlap_with_tolerance(a0, a1, b0, b1, eps, eps, minimum_segment_length) {
                return None;
            }
            seg_seg_intersection(a0, a1, b0, b1)
                .into_iter()
                .map(|p| p.0)
                .collect::<Vec<_>>()
        }
        (TrimCurve::Segment { a, b }, other) | (other, TrimCurve::Segment { a, b }) => {
            let circle = circular(&other).unwrap();
            let mut points: Vec<DVec2> =
                seg_circle_intersections(a, b, circle.center, circle.radius)
                    .into_iter()
                    .map(|p| p.0)
                    .collect();
            let (a, b, center) = (a.0, b.0, circle.center.0);
            // Near-tangency within tolerance is a contact even if the quadratic
            // discriminant rounds slightly below zero.
            let t = ((center - a).dot(b - a) / a.distance_squared(b)).clamp(0.0, 1.0);
            if points.is_empty() {
                points.push(a.lerp(b, t));
            }
            points
        }
        _ => {
            let a = circular(a).unwrap();
            let b = circular(b).unwrap();
            if a.center.distance(b.center) <= eps && a.radius.abs_diff(b.radius) <= eps {
                if arc_sweeps_overlap(a.geom, b.geom, Angle(eps.0 / a.radius.max(b.radius).0)) {
                    return None;
                }
                Vec::new()
            } else {
                let mut points: Vec<DVec2> =
                    circle_circle_intersections(a.center, a.radius, b.center, b.radius)
                        .into_iter()
                        .map(|p| p.0)
                        .collect();
                let (ca, cb, ra, rb) = (a.center.0, b.center.0, a.radius.0, b.radius.0);
                if points.is_empty() && ca != cb {
                    let axis = (cb - ca).normalize();
                    points.extend([
                        ca + axis * ra,
                        ca - axis * ra,
                        cb + axis * rb,
                        cb - axis * rb,
                    ]);
                }
                points
            }
        }
    })
}

#[derive(Clone, Copy)]
enum Piece<'a> {
    Line {
        a: WorldPoint,
        b: WorldPoint,
    },
    Arc(CircularPiece),
    Spline {
        spline: &'a SplineWindow,
        reversed: bool,
    },
}

impl<'a> Piece<'a> {
    fn from_curve(curve: &'a ProfileCurve, reversed: bool) -> Self {
        match curve {
            ProfileCurve::Analytic(curve) => Self::from_analytic(*curve, reversed),
            ProfileCurve::Spline(spline) => Self::Spline { spline, reversed },
        }
    }

    fn from_analytic(curve: TrimCurve, reversed: bool) -> Self {
        if let TrimCurve::Segment { a, b } = curve {
            let (a, b) = if reversed { (b, a) } else { (a, b) };
            return Self::Line { a, b };
        }
        let mut circle = circular(&curve).unwrap();
        circle.geom.start_angle = circle.geom.start_angle.normalized();
        if reversed {
            circle.geom = ArcGeom::new(circle.geom.end_angle(), -circle.geom.sweep_angle);
        }
        Self::Arc(circle)
    }

    fn at(self, t: f64) -> DVec2 {
        match self {
            Self::Line { a, b } => a.0.lerp(b.0, t),
            Self::Arc(CircularPiece {
                center,
                radius,
                geom,
            }) => {
                center.0
                    + DVec2::from_angle(geom.start_angle.radians() + geom.sweep_angle.radians() * t)
                        * radius.0
            }
            // Window fraction in edge travel order; only piece ends are needed.
            Self::Spline { spline, reversed } => {
                let [t0, t1] = spline.window();
                let u = if reversed { 1.0 - t } else { t };
                spline.point(SplineParam(t0.0 + (t1.0 - t0.0) * u)).0
            }
        }
    }

    fn area(self, origin: DVec2) -> WorldArea {
        let chord = (self.at(0.0) - origin).perp_dot(self.at(1.0) - origin) * 0.5;
        WorldArea::new(match self {
            Self::Line { .. } => chord,
            Self::Arc(CircularPiece { radius, geom, .. }) => {
                let sweep = geom.sweep_angle.radians();
                chord + radius.0 * radius.0 * (sweep - sweep.sin()) * 0.5
            }
            Self::Spline { spline, reversed } => spline.area(origin, reversed),
        })
    }

    /// Rightward horizontal ray. Split arcs at y extrema for monotone pieces;
    /// half-open intervals count a shared vertex once and a tangency zero times.
    fn crossings(self, point: DVec2) -> usize {
        match self {
            Self::Line { a, b } => {
                let (a, b) = (a.0, b.0);
                if (a.y > point.y) == (b.y > point.y) {
                    return 0;
                }
                usize::from(a.x + (point.y - a.y) * (b.x - a.x) / (b.y - a.y) > point.x)
            }
            Self::Arc(mut circle) => {
                circle.geom.start_angle = circle.geom.start_angle.normalized();
                let arc = Self::Arc(circle);
                let CircularPiece {
                    center,
                    radius,
                    geom,
                } = circle;
                let (center, radius) = (center.0, radius.0);
                let (start, sweep) = (geom.start_angle.radians(), geom.sweep_angle.radians());
                let mut cuts = vec![0.0, 1.0];
                for k in -3..=5 {
                    let t = (FRAC_PI_2 + f64::from(k) * PI - start) / sweep;
                    if t > 0.0 && t < 1.0 {
                        cuts.push(t);
                    }
                }
                cuts.sort_by(f64::total_cmp);
                cuts.windows(2)
                    .filter(|ts| {
                        let a = arc.at(ts[0]);
                        let b = arc.at(ts[1]);
                        if (a.y > point.y) == (b.y > point.y) {
                            return false;
                        }
                        let dy = point.y - center.y;
                        let sign = (start + sweep * (ts[0] + ts[1]) * 0.5).cos().signum();
                        let dx_squared = radius * radius - dy * dy;
                        // The y interval proves a root exists, so `r² − dy² ≥ 0` up to
                        // roundoff at an extremum: `arc.at` and `dy` carry ε·(|center| +
                        // |point| + r), and `dy²` doubles it by `|dy| ≤ r`.
                        let magnitude =
                            radius + center.abs().max_element() + point.abs().max_element();
                        let x = center.x
                            + sign
                                * sqrt_of_proven_nonnegative(
                                    dx_squared,
                                    magnitude * magnitude,
                                    "profile winding: ray × arc crossing",
                                );
                        x > point.x
                    })
                    .count()
            }
            Self::Spline { spline, .. } => spline.crossings(point),
        }
    }
}

/// Short connectors close only joins already accepted within tolerance.
/// They are neither persisted in a sketch nor turned into independent edges.
fn pieces(curves: &[(ProfileCurve, bool)]) -> Vec<Piece<'_>> {
    let mut result = Vec::with_capacity(curves.len() * 2);
    for (i, (curve, reversed)) in curves.iter().enumerate() {
        let piece = Piece::from_curve(curve, *reversed);
        let (next, reverse_next) = &curves[(i + 1) % curves.len()];
        result.push(piece);
        result.push(Piece::Line {
            a: WorldPoint::from_dvec2(piece.at(1.0)),
            b: WorldPoint::from_dvec2(Piece::from_curve(next, *reverse_next).at(0.0)),
        });
    }
    result
}

pub fn signed_area(curves: &[(ProfileCurve, bool)]) -> WorldArea {
    let pieces = pieces(curves);
    let origin = pieces[0].at(0.0);
    WorldArea::new(pieces.iter().map(|p| p.area(origin).0).sum())
}

pub fn contains(curves: &[(ProfileCurve, bool)], point: WorldPoint) -> bool {
    pieces(curves)
        .iter()
        .map(|p| p.crossings(point.0))
        .sum::<usize>()
        % 2
        == 1
}
