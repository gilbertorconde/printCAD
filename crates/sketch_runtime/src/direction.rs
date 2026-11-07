//! Segment extensions, perpendicular feet and round-curve tangencies.

use sketch_geometry::arc::angle_on_signed_arc;
use sketch_geometry::types::Angle;
use sketch_geometry::world::{WorldLength, WorldPoint, WorldVec};

use crate::geometry::SourceId;
use crate::pick::{SnapKind, Source};
use crate::snap::{SnapGeom, SnapObj, SnapResult};

/// Carry a nearby direction to the selected curve. The primary source owns
/// segment alignment; the second source owns endpoint membership.
pub(crate) fn onto_curve(
    directions: &[SnapObj],
    objects: &[SnapObj],
    cursor: WorldPoint,
    tolerance: WorldLength,
    selected: SnapResult,
) -> SnapResult {
    if !matches!(selected.kind, SnapKind::OnCurve | SnapKind::OnAxis) {
        return selected;
    }
    let Some(onto) = objects.iter().find(|o| o.source == selected.source) else {
        return selected;
    };
    for direction in directions {
        let (origin, dir, kind) = match direction.geom {
            SnapGeom::Line(a, b) if direction.source.source == Source::Guide => {
                if !matches!(
                    axis_between(a, b),
                    Some(ToolAxis::Horizontal | ToolAxis::Vertical)
                ) {
                    continue;
                }
                (a, (b - a).0.normalize(), SnapKind::OnAxis)
            }
            SnapGeom::ParallelThrough { origin, dir } => (origin, dir.0, SnapKind::Parallel),
            SnapGeom::SquareThrough { origin, dir } => (origin, dir.0, SnapKind::PerpendicularTo),
            _ => continue,
        };
        if direction.source == onto.source || (cursor - origin).0.perp_dot(dir).abs() > tolerance.0
        {
            continue;
        }
        let crossing = |a: WorldPoint, along: glam::DVec2| {
            let denom = dir.perp_dot(along);
            (denom.abs() > 1e-12).then(|| {
                origin + WorldVec::from_dvec2(dir * (a - origin).0.perp_dot(along) / denom)
            })
        };
        let candidates: Vec<WorldPoint> = match onto.geom {
            SnapGeom::Segment(a, b) => crossing(a, (b - a).0)
                .filter(|p| sketch_geometry::arc::dist_to_segment(*p, a, b).0 <= 1e-6)
                .into_iter()
                .collect(),
            SnapGeom::Line(a, b) => crossing(a, (b - a).0).into_iter().collect(),
            SnapGeom::Circle { center, radius } | SnapGeom::Arc { center, radius, .. } => {
                let oc = (origin - center).0;
                let b = oc.dot(dir);
                let disc = b * b - (oc.length_squared() - radius.0 * radius.0);
                if disc < 0.0 {
                    continue;
                }
                [-b - disc.sqrt(), -b + disc.sqrt()]
                    .into_iter()
                    .map(|t| origin + WorldVec::from_dvec2(dir * t))
                    .filter(|p| match onto.geom {
                        SnapGeom::Arc {
                            start_angle,
                            sweep_angle,
                            ..
                        } => angle_on_signed_arc(
                            start_angle,
                            sweep_angle,
                            Angle::from_vec(*p - center),
                        ),
                        _ => true,
                    })
                    .collect()
            }
            _ => continue,
        };
        if let Some(pos) = candidates
            .into_iter()
            .min_by(|a, b| a.distance(cursor).0.total_cmp(&b.distance(cursor).0))
            .filter(|p| p.distance(cursor).0 <= 2.0 * tolerance.0)
        {
            return SnapResult {
                snapped_pos: pos,
                distance: pos.distance(cursor),
                kind,
                priority: selected.priority,
                source: direction.source,
                source2: Some(onto.source),
            };
        }
    }
    selected
}

#[derive(Debug, Clone, Copy)]
pub enum DirectionQuery {
    None,
    Extensions,
    From {
        pos: WorldPoint,
        vertex: Option<SourceId>,
    },
}

/// Extensions have a finite reach of half a segment beyond either endpoint.
pub fn extension_snaps<R: Copy>(objects: &[SnapObj<R>]) -> Vec<SnapObj<R>> {
    objects
        .iter()
        .filter_map(|object| {
            let SnapGeom::Segment(a, b) = object.geom else {
                return None;
            };
            (a.distance(b) > WorldLength::DEGENERATE).then_some(SnapObj {
                geom: SnapGeom::Extension { a, b },
                source: object.source,
            })
        })
        .collect()
}

/// Tool axis selected for the link endpoint.
///
/// This gesture declaration determines which constraint a host creates.
/// A diagonal is explicit even though it adds no constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolAxis {
    Horizontal,
    Vertical,
    /// ±45° axes: geometric placement only, without an angular constraint.
    Diagonal,
}

/// Dimensionless sine tolerance for recognizing a snapped axis alignment.
///
/// Snap has already placed the point on an axis, so allow only coordinate
/// rounding: about M/L · 2.2e-16 for coordinate magnitude M and link length L.
/// The 1e-9 bound covers L = 1e-2 at M = 1e5. `WorldLength::DEGENERATE`
/// separately rejects vectors without a direction.
const AXIS_SIN_EPS: f64 = 1e-9;

/// Tool axis through origin and pos, inverse of [`snap_axes`].
///
/// Use the same `snap::AXIS_DIRECTIONS` as placement so classification and
/// geometric alignment agree at boundaries.
///
/// The tool names the supplied axes in order: H, V, then two diagonals.
pub fn axis_between(origin: WorldPoint, pos: WorldPoint) -> Option<ToolAxis> {
    const NAMES: [ToolAxis; 4] = [
        ToolAxis::Horizontal,
        ToolAxis::Vertical,
        ToolAxis::Diagonal,
        ToolAxis::Diagonal,
    ];

    let delta = (pos - origin).0;
    if delta.length() < WorldLength::DEGENERATE.0 {
        return None;
    }
    let dir = delta.normalize();
    AXIS_DIRECTIONS
        .iter()
        .position(|axis_dir| dir.perp_dot(axis_dir.0).abs() < AXIS_SIN_EPS)
        .map(|idx| NAMES[idx])
}

/// Combine gesture axes with addressed direction candidates. A diagonal which
/// duplicates a bindable direction must not hide that direction's constraint.
pub fn line_snaps<R: Copy>(
    origin: WorldPoint,
    guide: R,
    directions: Vec<SnapObj<R>>,
) -> Vec<SnapObj<R>> {
    let mut objects = snap_axes(origin, guide);
    objects.retain(|axis| {
        let SnapGeom::Line(a, b) = axis.geom else {
            unreachable!("gesture axes are lines")
        };
        let axis_dir = (b - a).0.normalize();
        !matches!(axis_between(a, b), Some(ToolAxis::Diagonal)) || !directions.iter().any(|object| {
            matches!(object.geom, SnapGeom::ParallelThrough { dir, .. } | SnapGeom::SquareThrough { dir, .. }
                if dir.0.perp_dot(axis_dir).abs() < AXIS_SIN_EPS)
        })
    });
    objects.extend(directions);
    objects
}

/// `shares_vertex` reports topology only. Geometric incidence is checked here,
/// together with all direction and tangency policies.
pub fn direction_snaps<R: Copy>(
    objects: &[SnapObj<R>],
    from: WorldPoint,
    shares_vertex: impl Fn(R) -> bool,
) -> Vec<SnapObj<R>> {
    let through_from = |source, a: WorldPoint, b: WorldPoint| {
        shares_vertex(source)
            || a.distance(from) <= WorldLength::DEGENERATE
            || b.distance(from) <= WorldLength::DEGENERATE
    };
    let mut out: Vec<_> = extension_snaps(objects)
        .into_iter()
        .filter(|object| {
            let SnapGeom::Extension { a, b } = object.geom else {
                unreachable!()
            };
            !through_from(object.source, a, b)
        })
        .collect();
    for object in objects {
        let SnapGeom::Segment(a, b) = object.geom else {
            continue;
        };
        let ab = (b - a).0;
        if ab.length() <= WorldLength::DEGENERATE.0 {
            continue;
        }
        let source = object.source;
        let t = (from - a).0.dot(ab) / ab.length_squared();
        let foot = WorldPoint::from_dvec2(a.0 + ab * t);
        if (0.0..=1.0).contains(&t) && foot.distance(from) > WorldLength::DEGENERATE {
            out.push(SnapObj {
                geom: SnapGeom::PerpendicularFoot(foot),
                source,
            });
        }
        let dir = WorldVec::from_dvec2(ab.normalize());
        if !through_from(source, a, b) {
            out.push(SnapObj {
                geom: SnapGeom::ParallelThrough { origin: from, dir },
                source,
            });
        }
        out.push(SnapObj {
            geom: SnapGeom::SquareThrough {
                origin: from,
                dir: WorldVec::from_dvec2(ab.normalize().perp()),
            },
            source,
        });
    }
    for object in objects {
        let (center, radius, span) = match object.geom {
            SnapGeom::Circle { center, radius } => (center, radius, None),
            SnapGeom::Arc {
                center,
                radius,
                start_angle,
                sweep_angle,
            } => (center, radius, Some((start_angle, sweep_angle))),
            _ => continue,
        };
        let away = from - center;
        let d = away.0.length();
        // A point on or inside the carrier has no tangent segment to it.
        if d <= radius.0 + WorldLength::DEGENERATE.0 {
            continue;
        }
        let spread = (radius.0 / d).acos();
        let base = Angle::from_vec(away).0;
        for side in [-1.0, 1.0] {
            let angle = Angle(base + side * spread);
            if span.is_some_and(|(start, sweep)| !angle_on_signed_arc(start, sweep, angle)) {
                continue;
            }
            out.push(SnapObj {
                geom: SnapGeom::TangentPoint(
                    center + WorldVec::new(angle.0.cos() * radius.0, angle.0.sin() * radius.0),
                ),
                source: object.source,
            });
        }
    }
    out
}

/// H/V and both diagonals, in the order used by drawing tools to name axes.
pub const AXIS_DIRECTIONS: [WorldVec; 4] = [
    WorldVec::new(1.0, 0.0),
    WorldVec::new(0.0, 1.0),
    WorldVec::new(
        std::f64::consts::FRAC_1_SQRT_2,
        std::f64::consts::FRAC_1_SQRT_2,
    ),
    WorldVec::new(
        std::f64::consts::FRAC_1_SQRT_2,
        -std::f64::consts::FRAC_1_SQRT_2,
    ),
];

pub fn snap_axes<R: Copy>(origin: WorldPoint, source: R) -> Vec<SnapObj<R>> {
    AXIS_DIRECTIONS
        .iter()
        .map(|dir| SnapObj {
            geom: SnapGeom::Line(origin, origin + *dir * 99999.0),
            source,
        })
        .collect()
}
