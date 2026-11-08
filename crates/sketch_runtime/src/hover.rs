//! Hover ranking: model points, origin, curves, then implicit axes.

use sketch_geometry::arc::{dist_to_arc, dist_to_circle, dist_to_segment};
use sketch_geometry::bspline_window::SplineWindow;
use sketch_geometry::curves::{
    dist_to_arc_of_ellipse, dist_to_arc_of_hyperbola, dist_to_arc_of_parabola,
};
use sketch_geometry::types::Angle;
use sketch_geometry::world::{ScreenPx, WorldLength, WorldPerPx, WorldPoint};

use crate::pick::Source;

/// Hover uses stored endpoints at span boundaries. They can differ from the
/// trigonometric evaluation by rounding while still naming one shared vertex.
#[derive(Debug, Clone)]
pub enum HoverGeom {
    Point(WorldPoint),
    Segment(WorldPoint, WorldPoint),
    Circle {
        center: WorldPoint,
        radius: WorldLength,
    },
    Arc {
        center: WorldPoint,
        start: WorldPoint,
        end: WorldPoint,
        start_angle: Angle,
        sweep_angle: Angle,
    },
    EllipseArc {
        center: WorldPoint,
        focus1: WorldPoint,
        start: WorldPoint,
        end: WorldPoint,
        clockwise: bool,
    },
    HyperbolaArc {
        center: WorldPoint,
        focus1: WorldPoint,
        start: WorldPoint,
        end: WorldPoint,
    },
    ParabolaArc {
        vertex: WorldPoint,
        focus: WorldPoint,
        start: WorldPoint,
        end: WorldPoint,
    },
    BSpline(SplineWindow),
    Origin,
    HAxis,
    VAxis,
}

#[derive(Debug, Clone)]
pub struct HoverObj<R = Source> {
    pub geo: R,
    pub geom: HoverGeom,
}

pub fn hover_candidates_in_tolerance<R: Copy>(
    objects: &[HoverObj<R>],
    cursor: WorldPoint,
    tolerance: WorldLength,
) -> Vec<HoverCandidate<R>> {
    objects
        .iter()
        .filter_map(|object| {
            let (distance, rank) = match &object.geom {
                HoverGeom::Point(pos) => (cursor.distance(*pos), RANK_POINT),
                HoverGeom::Segment(a, b) => (dist_to_segment(cursor, *a, *b), RANK_CURVE),
                HoverGeom::Circle { center, radius } => {
                    (dist_to_circle(cursor, *center, *radius), RANK_CURVE)
                }
                HoverGeom::Arc {
                    center,
                    start,
                    end,
                    start_angle,
                    sweep_angle,
                } => (
                    dist_to_arc(cursor, *center, *start, *end, *start_angle, *sweep_angle),
                    RANK_CURVE,
                ),
                HoverGeom::EllipseArc {
                    center,
                    focus1,
                    start,
                    end,
                    clockwise,
                } => (
                    dist_to_arc_of_ellipse(cursor, *center, *focus1, *start, *end, *clockwise),
                    RANK_CURVE,
                ),
                HoverGeom::HyperbolaArc {
                    center,
                    focus1,
                    start,
                    end,
                } => (
                    dist_to_arc_of_hyperbola(cursor, *center, *focus1, *start, *end),
                    RANK_CURVE,
                ),
                HoverGeom::ParabolaArc {
                    vertex,
                    focus,
                    start,
                    end,
                } => (
                    dist_to_arc_of_parabola(cursor, *vertex, *focus, *start, *end),
                    RANK_CURVE,
                ),
                HoverGeom::BSpline(window) => (window.nearest(cursor).1, RANK_CURVE),
                HoverGeom::Origin => (WorldLength::new(cursor.0.length()), RANK_ORIGIN),
                HoverGeom::HAxis => (WorldLength::new(cursor.0.y.abs()), RANK_AXIS),
                HoverGeom::VAxis => (WorldLength::new(cursor.0.x.abs()), RANK_AXIS),
            };
            (distance < tolerance).then_some(HoverCandidate {
                geo: object.geo,
                distance,
                rank,
            })
        })
        .collect()
}

pub fn rank_hover<R: Copy + Ord>(
    objects: &[HoverObj<R>],
    cursor: WorldPoint,
    tolerance: WorldLength,
) -> Vec<HoverCandidate<R>> {
    let mut candidates = hover_candidates_in_tolerance(objects, cursor, tolerance);
    candidates.sort_by(hover_cmp);
    candidates
}

pub fn hover_at<R: Copy + Ord>(
    objects: &[HoverObj<R>],
    cursor: WorldPoint,
    tolerance: WorldLength,
) -> Option<R> {
    hover_candidates_in_tolerance(objects, cursor, tolerance)
        .into_iter()
        .min_by(hover_cmp)
        .map(|c| c.geo)
}

pub const HOVER_TOLERANCE_PX: ScreenPx = ScreenPx::new(14.0);
pub fn hover_tolerance(view_scale: WorldPerPx) -> WorldLength {
    view_scale * HOVER_TOLERANCE_PX
}

/// Hover candidate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoverCandidate<R = Source> {
    pub geo: R,
    pub distance: WorldLength,
    /// Coarse priority: points precede curves.
    pub rank: u32,
}

/// A model point precedes a curve even when it lies exactly on the curve;
/// otherwise the curve's zero distance would make the point unreachable.
pub const RANK_POINT: u32 = 0;
/// The origin precedes curves through it, but a model point within tolerance
/// precedes the origin even when farther from the cursor.
pub const RANK_ORIGIN: u32 = 1;
/// Curve priority for segments, circles, arcs and conics.
pub const RANK_CURVE: u32 = 2;
/// Infinite sketch axes H (Y = 0) and V (X = 0) follow model points and curves,
/// keeping geometry on an axis reachable.
///
/// The host queries axes in a plane-normal view and draws them beyond the
/// viewport, so no separate visible-axis extent is needed here.
pub const RANK_AXIS: u32 = 3;

/// Shared hover ordering, analogous to `snap::snap_cmp`.
///
/// Order by priority, distance, then source address to avoid iteration-order
/// ties. Hover targets an element rather than a position, so unlike snap there
/// is no coordinate tie-break.
pub fn hover_cmp<R: Copy + Ord>(
    a: &HoverCandidate<R>,
    b: &HoverCandidate<R>,
) -> std::cmp::Ordering {
    hover_cmp_by_key(a, b, |geo| geo)
}

/// Storage-independent identity changes only the final tie-break, not hover policy.
pub fn hover_cmp_by_key<R: Copy, K: Ord>(
    a: &HoverCandidate<R>,
    b: &HoverCandidate<R>,
    key: impl Fn(R) -> K,
) -> std::cmp::Ordering {
    a.rank
        .cmp(&b.rank)
        .then_with(|| {
            a.distance.partial_cmp(&b.distance).unwrap_or_else(|| {
                panic!(
                    "hover_cmp: distances cannot be compared ({} versus {}); NaN in hover input",
                    a.distance.0, b.distance.0
                )
            })
        })
        .then_with(|| key(a.geo).cmp(&key(b.geo)))
}
