//! Pairwise analytic intersections. Both source addresses survive the query.

use sketch_geometry::arc::{angle_on_signed_arc, dist_to_circle, dist_to_segment};
use sketch_geometry::bbox::{Bbox, circle_bbox, segment_bbox};
use sketch_geometry::intersections::{
    circle_circle_intersections, seg_circle_intersections, seg_seg_intersection,
};
use sketch_geometry::profile_curve::ProfileCurve;
use sketch_geometry::profile_geometry::{PairContact, contacts};
use sketch_geometry::types::Angle;
use sketch_geometry::world::{WorldLength, WorldPoint};

use crate::pick::SnapKind;
use crate::snap::{SnapGeom, SnapObj, SnapResult};

/// A resolved carrier and its real boundary positions. Representation poles
/// carry no source address and cannot become point targets.
#[derive(Debug, Clone)]
pub struct ContactCurve<R> {
    pub source: R,
    pub curve: ProfileCurve,
    pub ends: Option<[WorldPoint; 2]>,
}

/// Contacts involving a rational carrier, refined on both exact curves.
/// The host supplies its model tolerances; pointer reach is a separate input.
pub fn rational_intersections_near<R: Copy>(
    curves: &[ContactCurve<R>],
    cursor: WorldPoint,
    tolerance: WorldLength,
    contact_tolerance: WorldLength,
    minimum_segment_length: WorldLength,
) -> Vec<(WorldPoint, R, R)> {
    if !curves
        .iter()
        .any(|c| matches!(c.curve, ProfileCurve::Spline(_)))
    {
        return Vec::new();
    }
    let curves: Vec<_> = curves
        .iter()
        .filter(|c| {
            !matches!(c.curve, ProfileCurve::Analytic(sketch_geometry::trim_curve::TrimCurve::Segment { .. })
                if c.curve.length() <= minimum_segment_length)
                && c.curve.distance(cursor) < tolerance
        })
        .collect();
    let mut out = Vec::new();
    for (i, a) in curves.iter().enumerate() {
        for b in &curves[i + 1..] {
            if !matches!(a.curve, ProfileCurve::Spline(_))
                && !matches!(b.curve, ProfileCurve::Spline(_))
            {
                continue;
            }
            if let PairContact::Points(points) = contacts(
                &a.curve,
                a.ends,
                &b.curve,
                b.ends,
                contact_tolerance,
                minimum_segment_length,
            ) {
                out.extend(
                    points
                        .into_iter()
                        .filter(|p| p.distance(cursor) < tolerance)
                        .map(|p| (p, a.source, b.source)),
                );
            }
        }
    }
    out
}

/// Only finite segments and round carriers participate. Rational contacts use
/// their exact spline carriers and the shared contact refinement separately.
pub fn collect_intersection_snaps<R: Copy>(
    objects: &[SnapObj<R>],
    cursor: WorldPoint,
    tolerance: WorldLength,
) -> Vec<SnapResult<R>> {
    struct SegEntry<R> {
        geoid: R,
        a: WorldPoint,
        b: WorldPoint,
        aabb: Bbox,
    }
    struct CircEntry<R> {
        geoid: R,
        center: WorldPoint,
        radius: WorldLength,
        sweep: Option<(Angle, Angle)>,
        aabb: Bbox,
    }
    impl<R> CircEntry<R> {
        fn contains(&self, point: WorldPoint) -> bool {
            self.sweep.is_none_or(|(start, sweep)| {
                angle_on_signed_arc(start, sweep, Angle::from_vec(point - self.center))
            })
        }
    }
    let mut segs = Vec::new();
    let mut circs = Vec::new();
    // Broad-phase proximity only prunes pairwise intersections. Whole-object
    // projections remain available anywhere along their carriers.
    for object in objects {
        match object.geom {
            SnapGeom::Segment(a, b) if dist_to_segment(cursor, a, b) < tolerance => {
                segs.push(SegEntry {
                    geoid: object.source,
                    a,
                    b,
                    aabb: segment_bbox(a, b),
                });
            }
            SnapGeom::Circle { center, radius } | SnapGeom::Arc { center, radius, .. }
                if dist_to_circle(cursor, center, radius) < tolerance =>
            {
                let sweep = match object.geom {
                    SnapGeom::Arc {
                        start_angle,
                        sweep_angle,
                        ..
                    } => Some((start_angle, sweep_angle)),
                    _ => None,
                };
                circs.push(CircEntry {
                    geoid: object.source,
                    center,
                    radius,
                    sweep,
                    aabb: circle_bbox(center, radius),
                });
            }
            _ => {}
        }
    }
    let mut results = Vec::new();
    let tolerance2 = tolerance.0 * tolerance.0;
    let mut push = |pt: WorldPoint, geo_a: R, geo_b: R| {
        let dist2 = cursor.0.distance_squared(pt.0);
        if dist2 < tolerance2 {
            results.push(SnapResult {
                snapped_pos: pt,
                distance: WorldLength::new(dist2.sqrt()),
                kind: SnapKind::Intersection,
                priority: 2,
                source: geo_a,
                source2: Some(geo_b),
            });
        }
    };
    // Segment × Segment
    for i in 0..segs.len() {
        for j in (i + 1)..segs.len() {
            if !segs[i].aabb.overlaps(segs[j].aabb) {
                continue;
            }
            if let Some(pt) = seg_seg_intersection(segs[i].a, segs[i].b, segs[j].a, segs[j].b) {
                push(pt, segs[i].geoid, segs[j].geoid);
            }
        }
    }

    // Segment × Arc/Circle
    for seg in &segs {
        for circ in &circs {
            if !seg.aabb.overlaps(circ.aabb) {
                continue;
            }
            for pt in seg_circle_intersections(seg.a, seg.b, circ.center, circ.radius) {
                if circ.contains(pt) {
                    push(pt, seg.geoid, circ.geoid);
                }
            }
        }
    }

    // Arc/Circle × Arc/Circle
    for i in 0..circs.len() {
        for j in (i + 1)..circs.len() {
            let a = &circs[i];
            let b = &circs[j];
            if !a.aabb.overlaps(b.aabb) {
                continue;
            }
            // Both carriers check their own travel: circles have no angular
            // limit, while arcs follow their signed sweep.
            for pt in circle_circle_intersections(a.center, a.radius, b.center, b.radius) {
                if a.contains(pt) && b.contains(pt) {
                    push(pt, a.geoid, b.geoid);
                }
            }
        }
    }

    results
}
