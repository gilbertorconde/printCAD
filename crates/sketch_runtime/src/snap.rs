//! Geometric snap projections and ranking, independent of document storage.

use sketch_geometry::arc::{closest_on_arc, closest_on_circle};
use sketch_geometry::curves::{
    closest_on_ellipse_arc, closest_on_hyperbola_arc, closest_on_parabola_arc,
};
use sketch_geometry::primitives::{project_onto_line, project_onto_segment};
use sketch_geometry::types::{Angle, HyperbolaBranch, HyperbolaParam, ParabolaParam};
use sketch_geometry::world::{ScreenPx, WorldLength, WorldPerPx, WorldPoint, WorldVec};

use crate::pick::{PointRole, SnapKind, SourceRef};

/// Only the geometric role is needed to project an addressed candidate.
pub trait SnapSource: Copy + Ord {
    fn point_role(self) -> PointRole;
}

impl SnapSource for SourceRef {
    fn point_role(self) -> PointRole {
        self.role
    }
}

pub const SNAP_TOLERANCE_PX: ScreenPx = ScreenPx::new(15.0);

pub fn snap_tolerance(view_scale: WorldPerPx) -> WorldLength {
    view_scale * SNAP_TOLERANCE_PX
}

#[derive(Clone, Debug)]
pub enum SnapGeom {
    Point(WorldPoint),
    Segment(WorldPoint, WorldPoint),
    /// Infinite line through two points.
    Line(WorldPoint, WorldPoint),
    Circle {
        center: WorldPoint,
        radius: WorldLength,
    },
    Arc {
        center: WorldPoint,
        radius: WorldLength,
        start_angle: Angle,
        sweep_angle: Angle,
    },
    /// Ellipse arc: center, focus1, semi_minor, start/end angles in ellipse parameter space.
    ArcOfEllipse {
        center: WorldPoint,
        focus1: WorldPoint,
        semi_minor: WorldLength,
        t_start: Angle,
        t_end: Angle,
        clockwise: bool,
    },
    /// Hyperbola arc: center, focus1, semi_minor, start/end params, branch.
    ArcOfHyperbola {
        center: WorldPoint,
        focus1: WorldPoint,
        semi_minor: WorldLength,
        t_start: HyperbolaParam,
        t_end: HyperbolaParam,
        /// Branch identity, which a scalar parameter alone cannot distinguish.
        branch: HyperbolaBranch,
    },
    /// Parabola arc: vertex, focus, start/end u params.
    ArcOfParabola {
        vertex: WorldPoint,
        focus: WorldPoint,
        u_start: ParabolaParam,
        u_end: ParabolaParam,
    },
    /// The exact visible spline window used for rendering.
    BSpline(sketch_geometry::bspline_window::SplineWindow),
    /// Perpendicular foot inside another segment, which is the source.
    /// Built by [`crate::direction::direction_snaps`].
    PerpendicularFoot(WorldPoint),
    /// Tangency from the link start onto the source circle/arc.
    /// Built by [`crate::direction::direction_snaps`].
    TangentPoint(WorldPoint),
    /// Line through the link start parallel to another segment; dir is unit length.
    ParallelThrough {
        origin: WorldPoint,
        dir: WorldVec,
    },
    /// Line through the link start perpendicular to another segment;
    /// dir is unit length and already rotated by 90°.
    SquareThrough {
        origin: WorldPoint,
        dir: WorldVec,
    },
    /// Extension of a→b by half its length at each end, keeping a local target
    /// rather than attracting the cursor across the whole sketch.
    Extension {
        a: WorldPoint,
        b: WorldPoint,
    },
    /// An arc's circle past the arc's ends: the arc helper. Over the arc
    /// itself it gives the nearer end, leaving the arc to its curve target.
    ArcExtension {
        center: WorldPoint,
        radius: WorldLength,
        start_angle: Angle,
        sweep_angle: Angle,
    },
}

#[derive(Clone, Debug)]
pub struct SnapObj<R = SourceRef> {
    pub geom: SnapGeom,
    pub source: R,
}

// ---------------------------------------------------------------------------
// Result
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SnapResult<R = SourceRef> {
    pub snapped_pos: WorldPoint,
    pub distance: WorldLength,
    pub kind: SnapKind,
    pub priority: u32,
    pub source: R,
    /// The other carrier of an intersection, or the landing curve of a
    /// directional snap. Direction and landing keep their distinct roles.
    pub source2: Option<R>,
}

impl<R: Copy> SnapResult<R> {
    /// An intersection is excluded when either of its carriers is excluded.
    pub fn is_excluded(&self, excluded: impl Fn(R) -> bool) -> bool {
        excluded(self.source) || self.source2.is_some_and(excluded)
    }

    /// Neither curve has a first/second role at an intersection. Canonicalize
    /// by stable source address before comparing otherwise identical candidates.
    pub fn canonicalize_sources<K: Ord>(&mut self, key: impl Fn(R) -> K) {
        if let Some(other) = self.source2
            && self.kind == SnapKind::Intersection
            && key(other) < key(self.source)
        {
            self.source2 = Some(self.source);
            self.source = other;
        }
    }
}

/// Query already resolved geometric carriers without a document. Exact rational
/// contacts can be appended to the candidate list before the same comparison.
pub fn snap_at<R: SnapSource>(
    cursor: WorldPoint,
    objects: &[SnapObj<R>],
    tolerance: WorldLength,
) -> Option<SnapResult<R>> {
    let mut candidates = candidates_in_tolerance(cursor, objects, tolerance);
    candidates.extend(crate::intersection::collect_intersection_snaps(
        objects, cursor, tolerance,
    ));
    for candidate in &mut candidates {
        candidate.canonicalize_sources(|source| source);
    }
    candidates.into_iter().min_by(snap_cmp)
}

struct SnapProjection {
    pos: WorldPoint,
    kind: SnapKind,
    priority: u32,
}

fn snap_projection<R: SnapSource>(cursor: WorldPoint, obj: &SnapObj<R>) -> SnapProjection {
    match &obj.geom {
        SnapGeom::Point(p) => {
            let kind = match obj.source.point_role() {
                PointRole::Start | PointRole::End => SnapKind::Endpoint,
                PointRole::Mid => SnapKind::Midpoint,
                PointRole::Center => SnapKind::Center,
                PointRole::Quadrant(_) => SnapKind::Quadrant,
                PointRole::Edge => SnapKind::Endpoint,
            };
            SnapProjection {
                pos: *p,
                kind,
                // Existing vertices precede computed locations even when the latter
                // are closer: accepting a vertex preserves its model identity.
                priority: match kind {
                    SnapKind::Midpoint | SnapKind::Quadrant => 3,
                    _ => 1,
                },
            }
        }
        SnapGeom::Segment(a, b) => SnapProjection {
            pos: project_onto_segment(cursor, *a, *b),
            kind: SnapKind::OnCurve,
            priority: 10,
        },
        SnapGeom::Line(a, b) => SnapProjection {
            pos: project_onto_line(cursor, *a, *b),
            kind: SnapKind::OnAxis,
            priority: 20,
        },
        SnapGeom::Circle { center, radius } => SnapProjection {
            pos: closest_on_circle(cursor, *center, *radius),
            kind: SnapKind::OnCurve,
            priority: 10,
        },
        SnapGeom::Arc {
            center,
            radius,
            start_angle,
            sweep_angle,
        } => SnapProjection {
            pos: closest_on_arc(cursor, *center, *radius, *start_angle, *sweep_angle),
            kind: SnapKind::OnCurve,
            priority: 10,
        },
        SnapGeom::ArcOfEllipse {
            center,
            focus1,
            semi_minor,
            t_start,
            t_end,
            clockwise,
        } => SnapProjection {
            pos: closest_on_ellipse_arc(
                cursor,
                *center,
                *focus1,
                *semi_minor,
                *t_start,
                *t_end,
                *clockwise,
            ),
            kind: SnapKind::OnCurve,
            priority: 10,
        },
        SnapGeom::ArcOfHyperbola {
            center,
            focus1,
            semi_minor,
            t_start,
            t_end,
            branch,
        } => SnapProjection {
            pos: closest_on_hyperbola_arc(
                cursor,
                *center,
                *focus1,
                *semi_minor,
                *t_start,
                *t_end,
                *branch,
            ),
            kind: SnapKind::OnCurve,
            priority: 10,
        },
        SnapGeom::ArcOfParabola {
            vertex,
            focus,
            u_start,
            u_end,
        } => SnapProjection {
            pos: closest_on_parabola_arc(cursor, *vertex, *focus, *u_start, *u_end),
            kind: SnapKind::OnCurve,
            priority: 10,
        },
        SnapGeom::BSpline(window) => SnapProjection {
            pos: window.point(window.nearest(cursor).0),
            kind: SnapKind::OnCurve,
            priority: 10,
        },
        // Feet and tangencies precede ordinary curve targets (10), but follow
        // existing points and intersections (1, 2).
        SnapGeom::PerpendicularFoot(p) => SnapProjection {
            pos: *p,
            kind: SnapKind::Perpendicular,
            priority: 5,
        },
        SnapGeom::TangentPoint(p) => SnapProjection {
            pos: *p,
            kind: SnapKind::Tangent,
            priority: 5,
        },
        // Other segments' directions follow tool axes (20), so an H/V tie
        // creates an axis relation rather than an arbitrary neighbor relation.
        SnapGeom::ParallelThrough { origin, dir } => SnapProjection {
            pos: project_onto_line(cursor, *origin, *origin + *dir),
            kind: SnapKind::Parallel,
            priority: 25,
        },
        SnapGeom::SquareThrough { origin, dir } => SnapProjection {
            pos: project_onto_line(cursor, *origin, *origin + *dir),
            kind: SnapKind::PerpendicularTo,
            priority: 25,
        },
        SnapGeom::Extension { a, b } => SnapProjection {
            pos: extension_foot(cursor, *a, *b),
            kind: SnapKind::OnExtension,
            priority: 30,
        },
        SnapGeom::ArcExtension {
            center,
            radius,
            start_angle,
            sweep_angle,
        } => SnapProjection {
            pos: arc_extension_foot(cursor, *center, *radius, *start_angle, *sweep_angle),
            kind: SnapKind::OnExtension,
            priority: 30,
        },
    }
}

/// Nearest point of the arc's circle outside the arc's sweep. Over the arc
/// itself, and at the centre, return the arc's nearer end, as
/// [`extension_foot`] does over a segment's interior.
fn arc_extension_foot(
    cursor: WorldPoint,
    center: WorldPoint,
    radius: WorldLength,
    start_angle: Angle,
    sweep_angle: Angle,
) -> WorldPoint {
    if cursor != center
        && !sketch_geometry::arc::angle_on_signed_arc(
            start_angle,
            sweep_angle,
            Angle::from_vec(cursor - center),
        )
    {
        return closest_on_circle(cursor, center, radius);
    }
    let at = |a: Angle| {
        let (sin, cos) = a.radians().sin_cos();
        WorldPoint::new(center.0.x + cos * radius.0, center.0.y + sin * radius.0)
    };
    let (start, end) = (at(start_angle), at(start_angle + sweep_angle));
    if cursor.distance(start) <= cursor.distance(end) {
        start
    } else {
        end
    }
}

/// Nearest extension point with line parameter in `[-1/2, 0] ∪ [1, 3/2]`.
/// Over the segment interior return its nearer end; the ordinary curve target
/// handles cursors beyond endpoint tolerance there.
fn extension_foot(cursor: WorldPoint, a: WorldPoint, b: WorldPoint) -> WorldPoint {
    let ab = (b - a).0;
    let t = (cursor - a).0.dot(ab) / ab.length_squared();
    let t = if t < 0.5 {
        t.clamp(-0.5, 0.0)
    } else {
        t.clamp(1.0, 1.5)
    };
    WorldPoint::from_dvec2(a.0 + ab * t)
}

/// Squared world distance, an intermediate comparison against squared tolerance.
fn snap_distance2(a: WorldPoint, b: WorldPoint) -> f64 {
    a.0.distance_squared(b.0)
}

fn snap_candidate<R: SnapSource>(cursor: WorldPoint, obj: &SnapObj<R>) -> (SnapResult<R>, f64) {
    let SnapProjection {
        pos,
        kind,
        priority,
    } = snap_projection(cursor, obj);
    let distance2 = snap_distance2(cursor, pos);
    (
        SnapResult {
            snapped_pos: pos,
            distance: WorldLength::new(distance2.sqrt()),
            kind,
            priority,
            source: obj.source,
            source2: None,
        },
        distance2,
    )
}

/// Shared candidate ordering for every snap query path.
///
/// Ordinary objects, intersections and merged queries use this same policy.
///
/// Compare in order:
/// 1. Priority: vertex/center/origin 1, intersection 2, midpoint/quadrant 3,
///    foot/tangency 5, curve 10, axis 20.
/// 2. Distance, nearest first.
/// 3. Kind, snapped coordinate, then source address as tie-breaks.
///
/// Tie-breaks keep equal-priority, equal-distance candidates independent of
/// collection order and insertion/deletion history.
///
/// Coordinates precede addresses so visibly identical scenes built in different
/// orders select the same position. Addresses distinguish coincident targets last.
///
/// Equal candidates are indistinguishable by every observable property.
///
/// Distances must be finite. NaN diagnoses invalid cursor/geometry input
/// rather than being assigned an arbitrary ordering.
pub fn snap_cmp<R: SnapSource>(a: &SnapResult<R>, b: &SnapResult<R>) -> std::cmp::Ordering {
    snap_cmp_by_key(a, b, |source| source)
}

/// The same ranking with a host's stable source address as the final tie-break.
/// Geometry storage keys may change across a fresh import; priorities do not.
pub fn snap_cmp_by_key<R: SnapSource, K: Ord>(
    a: &SnapResult<R>,
    b: &SnapResult<R>,
    key: impl Fn(R) -> K,
) -> std::cmp::Ordering {
    a.priority
        .cmp(&b.priority)
        .then_with(|| {
            a.distance.partial_cmp(&b.distance).unwrap_or_else(|| {
                panic!(
                    "snap_cmp: distances cannot be compared ({} versus {}); NaN in snap input",
                    a.distance.0, b.distance.0
                )
            })
        })
        .then_with(|| a.kind.cmp(&b.kind))
        .then_with(|| a.snapped_pos.0.x.total_cmp(&b.snapped_pos.0.x))
        .then_with(|| a.snapped_pos.0.y.total_cmp(&b.snapped_pos.0.y))
        .then_with(|| key(a.source).cmp(&key(b.source)))
        .then_with(|| a.source2.map(&key).cmp(&b.source2.map(&key)))
}

/// All candidates within tolerance, sorted best first by [`snap_cmp`].
///
/// Exposing the full list allows checking monotonicity, small-movement stability
/// and ties beyond the winning value.
pub fn rank_snaps<R: SnapSource>(
    cursor: WorldPoint,
    objects: &[SnapObj<R>],
    tolerance: WorldLength,
) -> Vec<SnapResult<R>> {
    let mut ranked = candidates_in_tolerance(cursor, objects, tolerance);
    ranked.sort_by(snap_cmp);
    ranked
}

/// Candidates within tolerance in collection order.
///
/// Collection order is not preference. Apply [`snap_cmp`] before presenting a
/// winner; keeping collection separate also permits independent ordering tests.
pub fn candidates_in_tolerance<R: SnapSource>(
    cursor: WorldPoint,
    objects: &[SnapObj<R>],
    tolerance: WorldLength,
) -> Vec<SnapResult<R>> {
    // WorldPoint::new guarantees a finite cursor position.
    let tolerance2 = tolerance.0 * tolerance.0;
    objects
        .iter()
        .map(|obj| snap_candidate(cursor, obj))
        .filter(|(_, distance2)| *distance2 < tolerance2)
        .map(|(result, _)| result)
        .collect()
}

/// Best ordinary snap object.
///
/// Use the same comparator as the sorted list without sorting or allocating
/// on this per-frame path.
pub fn find_snap<R: SnapSource>(
    cursor: WorldPoint,
    objects: &[SnapObj<R>],
    tolerance: WorldLength,
) -> Option<SnapResult<R>> {
    // WorldPoint::new guarantees a finite cursor position.
    let tolerance2 = tolerance.0 * tolerance.0;
    objects
        .iter()
        .map(|obj| snap_candidate(cursor, obj))
        .filter(|(_, distance2)| *distance2 < tolerance2)
        .map(|(result, _)| result)
        .min_by(snap_cmp)
}
