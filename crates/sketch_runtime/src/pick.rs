use sketch_geometry::world::WorldPoint;

use crate::geometry::SourceId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Geometry(SourceId),
    Axis(Axis),
    Origin,
    /// A gesture's synthetic guide has no document binding.
    Guide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PointRole {
    Edge,
    Start,
    End,
    Mid,
    Center,
    Quadrant(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceRef {
    pub source: Source,
    pub role: PointRole,
}

/// Snap kind.
///
/// `Ord` breaks ties after priority and distance in `snap_cmp`. Declaration
/// order makes comparison total; `SnapResult::priority` defines preference.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum SnapKind {
    Endpoint,
    Midpoint,
    Center,
    Quadrant,
    /// Intersection of two distinct geometry elements.
    Intersection,
    OnCurve,
    OnAxis,
    /// Perpendicular foot from the link start onto a segment.
    Perpendicular,
    /// Circle/arc tangency point from the link start.
    Tangent,
    /// Link parallel to another segment.
    Parallel,
    /// Link perpendicular to another segment, without endpoint membership.
    PerpendicularTo,
    /// Endpoint on a segment's extension.
    OnExtension,
}

impl SnapKind {
    /// A specific point target takes precedence over the grid, unlike a curve,
    /// axis or direction target (`Placement::resolve`).
    pub fn is_point(self) -> bool {
        match self {
            SnapKind::Endpoint
            | SnapKind::Midpoint
            | SnapKind::Center
            | SnapKind::Quadrant
            | SnapKind::Intersection
            | SnapKind::Tangent
            | SnapKind::Perpendicular => true,
            SnapKind::OnCurve
            | SnapKind::OnAxis
            | SnapKind::Parallel
            | SnapKind::PerpendicularTo
            | SnapKind::OnExtension => false,
        }
    }

    /// Marker and status label.
    pub fn label(self) -> &'static str {
        match self {
            SnapKind::Endpoint => "Endpoint",
            SnapKind::Midpoint => "Midpoint",
            SnapKind::Center => "Center",
            SnapKind::Quadrant => "Quadrant",
            SnapKind::Intersection => "Intersection",
            SnapKind::OnCurve => "On curve",
            SnapKind::OnAxis => "On axis",
            SnapKind::Perpendicular => "Perpendicular",
            SnapKind::Tangent => "Tangent",
            SnapKind::Parallel => "Parallel",
            SnapKind::PerpendicularTo => "Perpendicular to",
            SnapKind::OnExtension => "On extension",
        }
    }
}

/// A selected source, including both carriers of an intersection. A direction
/// can carry a second source for the curve its endpoint lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snap {
    pub kind: SnapKind,
    pub source: SourceRef,
    pub source2: Option<SourceRef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub cursor: WorldPoint,
    pub pos: WorldPoint,
    pub snap: Option<Snap>,
}
