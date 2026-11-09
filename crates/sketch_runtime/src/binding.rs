//! Selected geometric relations, translated to constraints by each host.

use crate::geometry::SourceId;
use crate::pick::{Axis, PointRole, Snap, SnapKind, Source, SourceRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    Coincident(SourceRef),
    Midpoint(SourceId),
    Quadrant { circle: SourceId, quadrant: u8 },
    OnCurve(SourceId),
    OnAxis(Axis),
}

fn on_curve(source: SourceRef) -> Option<Binding> {
    match source.source {
        Source::Geometry(id) => Some(Binding::OnCurve(id)),
        Source::Axis(_) | Source::Origin | Source::Guide => None,
    }
}

/// Point membership comes from the selected snap, never a coordinate guess.
/// Segment alignment (parallel, square, tangent) belongs to the segment plan.
pub fn for_snap(snap: &Snap) -> Vec<Binding> {
    if matches!(snap.kind, SnapKind::Parallel | SnapKind::PerpendicularTo)
        || (snap.kind == SnapKind::OnAxis && snap.source.source == Source::Guide)
    {
        return snap
            .source2
            .into_iter()
            .filter_map(|source| match source.source {
                Source::Axis(axis) => Some(Binding::OnAxis(axis)),
                _ => on_curve(source),
            })
            .collect();
    }
    match snap.kind {
        SnapKind::Endpoint if snap.source.source == Source::Origin => {
            vec![
                Binding::OnAxis(Axis::Horizontal),
                Binding::OnAxis(Axis::Vertical),
            ]
        }
        SnapKind::Endpoint | SnapKind::Center => vec![Binding::Coincident(snap.source)],
        SnapKind::Midpoint => {
            let (Source::Geometry(id), PointRole::Mid) = (snap.source.source, snap.source.role)
            else {
                panic!("midpoint snap must name a segment midpoint");
            };
            vec![Binding::Midpoint(id)]
        }
        SnapKind::Quadrant => {
            let (Source::Geometry(circle), PointRole::Quadrant(quadrant)) =
                (snap.source.source, snap.source.role)
            else {
                panic!("quadrant snap must name a circle quadrant");
            };
            assert!(quadrant < 4, "circle quadrant must be in 0..4");
            vec![Binding::Quadrant { circle, quadrant }]
        }
        SnapKind::OnCurve | SnapKind::Perpendicular | SnapKind::Tangent | SnapKind::OnExtension => {
            on_curve(snap.source).into_iter().collect()
        }
        SnapKind::OnAxis => match snap.source.source {
            Source::Axis(axis) => vec![Binding::OnAxis(axis)],
            _ => vec![],
        },
        SnapKind::Parallel | SnapKind::PerpendicularTo => vec![],
        SnapKind::Intersection => std::iter::once(snap.source)
            .chain(snap.source2)
            .filter_map(on_curve)
            .collect(),
    }
}
