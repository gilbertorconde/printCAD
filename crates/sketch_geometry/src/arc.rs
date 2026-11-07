use serde::{Deserialize, Serialize};

use crate::types::Angle;
use crate::world::{WorldLength, WorldPoint};

/// Check if angle `p` lies on the arc sweep from `s` to `e` (CCW convention: s → e CCW).
/// Used for standard circular arcs where direction is encoded in start_angle/end_angle ordering.
pub fn angle_on_arc(start: Angle, end: Angle, p: Angle) -> bool {
    start.ccw_contains(end, p)
}

/// Check if angle `p` lies on the arc sweep from `s` to `e` with explicit clockwise flag.
/// Used by conics (ellipse) which carry a clockwise bool.
/// Equal endpoint directions denote a full turn, as in `ellipse_arc_sweep`.
pub fn angle_on_arc_cw(s: Angle, e: Angle, clockwise: bool, p: Angle) -> bool {
    if clockwise {
        e.ccw_sweep_to(s) >= p.ccw_offset(s)
    } else {
        s.ccw_contains(e, p)
    }
}

/// Membership in `start + signed sweep`: zero denotes one point, while
/// |sweep| >= τ covers the circle. The sweep retains its sign and turn count.
pub fn angle_on_signed_arc(start: Angle, sweep: Angle, p: Angle) -> bool {
    assert!(sweep.0.is_finite(), "arc sweep must be finite: {sweep:?}");
    let offset = if sweep.0 < 0.0 {
        p.ccw_offset(start).0
    } else {
        start.ccw_offset(p).0
    };
    offset <= sweep.0.abs().min(std::f64::consts::TAU)
}

/// Largest chord angle with sagitta at most `error` on a circle of `radius`:
/// `s = r·(1 − cos(θ/2))` gives `θ = 4·asin(√(s / 2r))`.
///
/// Cap at a quarter turn so a full circle has at least four chords.
/// Take square roots before division: `2r` can overflow and `s/r` can
/// underflow even when the nonzero angular step is representable.
/// Zero error gives a zero step; the caller must reject an excessive chord count.
pub fn chord_step_for_sagitta(radius: WorldLength, error: WorldLength) -> Angle {
    assert!(
        radius.0.is_finite() && radius.0 > 0.0 && error.0.is_finite() && error.0 >= 0.0,
        "chord_step_for_sagitta: finite positive radius and nonnegative error required: {radius:?}, {error:?}"
    );
    if error >= radius {
        return Angle(std::f64::consts::FRAC_PI_2);
    }
    let sine = error.0.sqrt() / radius.0.sqrt() * std::f64::consts::FRAC_1_SQRT_2;
    Angle((4.0 * sine.asin()).min(std::f64::consts::FRAC_PI_2))
}

/// Distance from a point to a line segment.
pub fn dist_to_segment(p: WorldPoint, a: WorldPoint, b: WorldPoint) -> WorldLength {
    let ab = b.0 - a.0;
    let len_sq = ab.length_squared();
    // Only a segment below the squared geometric degeneracy threshold is a point.
    if len_sq < WorldLength::DEGENERATE.0 * WorldLength::DEGENERATE.0 {
        return p.distance(a);
    }
    let t = ((p.0 - a.0).dot(ab) / len_sq).clamp(0.0, 1.0);
    WorldLength::new(p.0.distance(a.0 + ab * t))
}

/// Distance from a point to a full circle edge.
pub fn dist_to_circle(p: WorldPoint, center: WorldPoint, radius: WorldLength) -> WorldLength {
    WorldLength::new((p.0.distance(center.0) - radius.0).abs())
}

pub fn closest_on_circle(p: WorldPoint, center: WorldPoint, radius: WorldLength) -> WorldPoint {
    assert!(
        radius.0.is_finite() && radius.0 >= 0.0,
        "circle radius must be finite and nonnegative: {radius:?}"
    );
    let delta = p - center;
    // hypot keeps a representable direction when squaring components would
    // overflow/underflow. At the center all directions are equally near: use +X.
    let length = WorldLength::new(delta.0.x.hypot(delta.0.y));
    let dir = if length == WorldLength::ZERO {
        glam::DVec2::X
    } else {
        delta.0 / length.0
    };
    WorldPoint::from_dvec2(center.0 + dir * radius.0)
}

/// Nearest point on a canonical signed arc. At the center the nearest point is
/// not unique; choose the start, which belongs even to a zero-sweep arc.
pub fn closest_on_arc(
    p: WorldPoint,
    center: WorldPoint,
    radius: WorldLength,
    start_angle: Angle,
    sweep_angle: Angle,
) -> WorldPoint {
    assert!(
        radius.0.is_finite() && radius.0 >= 0.0,
        "arc radius must be finite and nonnegative: {radius:?}"
    );
    // The circle's arbitrary +X tie-break need not belong to this arc. Its start
    // always does, including a zero sweep. Validate angles before either return.
    let angle = if p == center {
        start_angle
    } else {
        Angle::from_vec(p - center)
    };
    let on_arc = angle_on_signed_arc(start_angle, sweep_angle, angle);
    if p != center && on_arc {
        closest_on_circle(p, center, radius)
    } else {
        let at = |a: Angle| {
            let (sin, cos) = a.radians().sin_cos();
            WorldPoint::new(center.0.x + cos * radius.0, center.0.y + sin * radius.0)
        };
        let start_pt = at(start_angle);
        if p == center {
            return start_pt;
        }
        let end_pt = at(start_angle + sweep_angle);
        if p.distance(start_pt) < p.distance(end_pt) {
            start_pt
        } else {
            end_pt
        }
    }
}

/// Distance from a point to a circular arc.
/// Uses the canonical signed sweep, including zero and multiple full turns.
pub fn dist_to_arc(
    p: WorldPoint,
    center: WorldPoint,
    start: WorldPoint,
    end: WorldPoint,
    start_angle: Angle,
    sweep_angle: Angle,
) -> WorldLength {
    let radius = center.distance(start);
    let angle_p = Angle::from_vec(p - center);
    let on_arc = angle_on_signed_arc(start_angle, sweep_angle, angle_p);
    if radius.0 < 1e-8 {
        return p.distance(center);
    }
    if on_arc {
        WorldLength::new((p.distance(center).0 - radius.0).abs())
    } else {
        let to_start = p.distance(start);
        let to_end = p.distance(end);
        if to_start < to_end { to_start } else { to_end }
    }
}

#[cfg(test)]
#[path = "arc_projection_tests.rs"]
mod projection_tests;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ArcGeom {
    pub start_angle: Angle,
    pub sweep_angle: Angle,
}

impl ArcGeom {
    pub fn new(start_angle: Angle, sweep_angle: Angle) -> Self {
        Self {
            start_angle,
            sweep_angle,
        }
    }

    pub fn from_start_end(start_angle: Angle, end_angle: Angle) -> Self {
        Self {
            start_angle,
            sweep_angle: end_angle - start_angle,
        }
    }

    pub fn end_angle(self) -> Angle {
        self.start_angle + self.sweep_angle
    }

    pub fn clockwise(self) -> bool {
        self.sweep_angle.radians() < 0.0
    }

    /// Preserve the signed sweep: zero is a point; one or more full turns
    /// cover the circle, including the arc produced by splitting a circle.
    pub fn contains_angle(self, angle: Angle) -> bool {
        angle_on_signed_arc(self.start_angle, self.sweep_angle, angle)
    }

    pub fn strictly_contains_angle(self, angle: Angle, endpoint_tol: Angle) -> bool {
        assert!(
            endpoint_tol.0.is_finite() && endpoint_tol.0 >= 0.0,
            "ArcGeom: endpoint tolerance must be finite and nonnegative"
        );
        if !self.contains_angle(angle) {
            return false;
        }
        let endpoint_tol = endpoint_tol.radians();
        if self.clockwise() {
            let offset = angle.ccw_offset(self.start_angle).0;
            let tail = self.end_angle().ccw_offset(angle).0;
            offset > endpoint_tol && tail > endpoint_tol
        } else {
            let offset = self.start_angle.ccw_offset(angle).0;
            let tail = angle.ccw_offset(self.end_angle()).0;
            offset > endpoint_tol && tail > endpoint_tol
        }
    }

    pub fn split_at(self, angle: Angle) -> Option<(ArcGeom, ArcGeom)> {
        if !self.contains_angle(angle) {
            return None;
        }
        let left = ArcGeom::from_start_end(self.start_angle, angle);
        let right = ArcGeom::from_start_end(angle, self.end_angle());
        Some((left, right))
    }
}

/// Positive overlap of sweeps on the same circle. The caller checks center
/// and radius; a shared endpoint alone is not an overlap.
pub fn arc_sweeps_overlap(a: ArcGeom, b: ArcGeom, angle_tolerance: Angle) -> bool {
    let tolerance = angle_tolerance.radians();
    let mut boundaries = vec![
        Angle::normalized(a.start_angle),
        Angle::normalized(a.end_angle()),
        Angle::normalized(b.start_angle),
        Angle::normalized(b.end_angle()),
    ];
    boundaries.sort_by(|a, b| a.radians().total_cmp(&b.radians()));
    boundaries.dedup_by(|a, b| (a.radians() - b.radians()).abs() < tolerance);
    (0..boundaries.len()).any(|i| {
        let start = boundaries[i].radians();
        let mut end = boundaries[(i + 1) % boundaries.len()].radians();
        if i + 1 == boundaries.len() {
            end += std::f64::consts::TAU;
        }
        end - start > tolerance
            && a.contains_angle(Angle((start + end) * 0.5).normalized())
            && b.contains_angle(Angle((start + end) * 0.5).normalized())
    })
}
