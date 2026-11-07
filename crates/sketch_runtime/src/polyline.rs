//! Geometry of a mixed line/circular-arc chain, shared by preview and commit.
use sketch_geometry::world::{WorldPoint, WorldVec};

use crate::geometry::{PickElement, SourceId};
use crate::scene::PickScene;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PolylineMode {
    #[default]
    Line,
    Tangent,
    Perpendicular,
    Reverse,
}

impl PolylineMode {
    pub fn next(self) -> Self {
        match self {
            Self::Line => Self::Tangent,
            Self::Tangent => Self::Perpendicular,
            Self::Perpendicular => Self::Reverse,
            Self::Reverse => Self::Line,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Line => "Line",
            Self::Tangent => "Tangent arc",
            Self::Perpendicular => "Perpendicular arc",
            Self::Reverse => "Reverse arc",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ArcGeometry {
    pub center: WorldPoint,
    pub start: WorldPoint,
    pub end: WorldPoint,
    pub clockwise: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum PolylineCurve {
    Line,
    Arc(ArcGeometry),
}

/// Curve relation at the structural shared vertex. The perpendicular angle
/// uses canonical directions (a circle's CCW tangent), independent of traversal.
#[derive(Debug, Clone, Copy)]
pub enum ArcRelation {
    TangentLine(SourceId),
    TangentArc { previous: SourceId, internal: bool },
    Perpendicular(sketch_geometry::types::Angle),
}

pub fn arc_relation(
    scene: &PickScene,
    previous: SourceId,
    arc: ArcGeometry,
    mode: PolylineMode,
) -> ArcRelation {
    let old = scene
        .element(previous)
        .expect("previous chain link is live");
    if mode == PolylineMode::Perpendicular {
        let u = match *old {
            PickElement::Segment { a, b, .. } => {
                (scene.point_position(b).unwrap() - scene.point_position(a).unwrap()).0
            }
            PickElement::Arc { center, .. } => {
                (arc.start - scene.point_position(center).unwrap()).0.perp()
            }
            _ => panic!("chain link must be a segment or circular arc"),
        };
        let v = (arc.start - arc.center).0.perp();
        return ArcRelation::Perpendicular(sketch_geometry::types::Angle(
            u.perp_dot(v).atan2(u.dot(v)),
        ));
    }
    match *old {
        PickElement::Segment { .. } => ArcRelation::TangentLine(previous),
        PickElement::Arc { center, .. } => ArcRelation::TangentArc {
            previous,
            internal: (arc.start - scene.point_position(center).unwrap())
                .0
                .dot((arc.start - arc.center).0)
                > 0.0,
        },
        _ => panic!("chain link must be a segment or circular arc"),
    }
}

/// Traversal into a live junction, independent of the host's stored orientation.
pub fn heading_into(scene: &PickScene, previous: SourceId, at: SourceId) -> WorldVec {
    let p = scene.point_position(at).expect("chain endpoint is live");
    let d = match *scene
        .element(previous)
        .expect("previous chain link is live")
    {
        PickElement::Segment { a, b, .. } => {
            let other = if b == at {
                a
            } else {
                assert_eq!(a, at);
                b
            };
            (p - scene.point_position(other).unwrap()).0
        }
        PickElement::Arc {
            center,
            start,
            end,
            sweep_angle,
            ..
        } => {
            let sign = if end == at {
                1.0
            } else {
                assert_eq!(start, at);
                -1.0
            };
            (p - scene.point_position(center).unwrap()).0.perp() * sweep_angle.0.signum() * sign
        }
        _ => panic!("a polyline link must be a segment or circular arc"),
    };
    assert!(d.length() > 0.0, "chain link has a direction");
    WorldVec::from_dvec2(d.normalize())
}

/// The center lies on the start normal; equal endpoint radii give
/// signed_radius = |end-start|² / (2 * dot(end-start, normal)).
/// Collinear endpoints have no finite circle and leave the mode unchanged.
pub fn arc_plan(
    start: WorldPoint,
    heading: WorldVec,
    end: WorldPoint,
    mode: PolylineMode,
) -> Result<ArcGeometry, &'static str> {
    let d = (end - start).0;
    let h = match mode {
        PolylineMode::Line => panic!("arc_plan requires an arc mode"),
        PolylineMode::Tangent => heading.0,
        PolylineMode::Reverse => -heading.0,
        PolylineMode::Perpendicular => {
            let n = heading.0.perp();
            if n.dot(d) < 0.0 { -n } else { n }
        }
    };
    let normal = h.perp();
    let across = normal.dot(d);
    if d.length() <= sketch_geometry::world::WorldLength::DEGENERATE.0
        || across.abs() <= 1e-12 * d.length()
    {
        return Err("No finite arc here; move the endpoint or choose Line");
    }
    let signed_radius = d.length_squared() / (2.0 * across);
    Ok(ArcGeometry {
        center: start + WorldVec::from_dvec2(normal * signed_radius),
        start,
        end,
        clockwise: signed_radius < 0.0,
    })
}
