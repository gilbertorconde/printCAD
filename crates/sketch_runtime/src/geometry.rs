use sketch_geometry::types::Angle;
use sketch_geometry::world::{WorldLength, WorldPoint};

/// Stable source address supplied by the host, independent of storage order.
/// It is an opaque projection key, not a persisted document format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(pub u128);

/// A visible, possibly unwrapped interval on a spline's exact carrier.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplineSpan {
    /// Stored start/end roles, independent of the parameter window's order.
    pub endpoints: [SourceId; 2],
    pub parameters: [sketch_geometry::bspline::SplineParam; 2],
}

/// Exact input geometry. Representation poles of rational curves stay private.
#[derive(Debug, Clone, PartialEq)]
pub enum PickElement {
    Point {
        id: SourceId,
        pos: WorldPoint,
    },
    Segment {
        id: SourceId,
        a: SourceId,
        b: SourceId,
    },
    Circle {
        id: SourceId,
        center: SourceId,
        radius: WorldLength,
    },
    Arc {
        id: SourceId,
        center: SourceId,
        start: SourceId,
        end: SourceId,
        start_angle: Angle,
        sweep_angle: Angle,
    },
    Ellipse {
        id: SourceId,
        center: SourceId,
        focus1: WorldPoint,
        start: WorldPoint,
    },
    EllipseArc {
        id: SourceId,
        center: SourceId,
        focus1: WorldPoint,
        start: SourceId,
        end: SourceId,
        clockwise: bool,
    },
    HyperbolaArc {
        id: SourceId,
        center: SourceId,
        focus1: SourceId,
        start: SourceId,
        end: SourceId,
        clockwise: bool,
    },
    ParabolaArc {
        id: SourceId,
        vertex: SourceId,
        focus: SourceId,
        start: SourceId,
        end: SourceId,
        clockwise: bool,
    },
    BSpline {
        id: SourceId,
        poles: Vec<SourceId>,
        shape: sketch_geometry::bspline::SplineShape,
        basis: Option<sketch_geometry::bspline::SplineBasis>,
        span: Option<SplineSpan>,
    },
    RationalCurve {
        id: SourceId,
        poles: Vec<WorldPoint>,
        basis: sketch_geometry::bspline::SplineBasis,
        periodic: bool,
        endpoints: Option<[SourceId; 2]>,
        center: Option<SourceId>,
    },
}

impl PickElement {
    pub fn id(&self) -> SourceId {
        match self {
            Self::Point { id, .. }
            | Self::Segment { id, .. }
            | Self::Circle { id, .. }
            | Self::Arc { id, .. }
            | Self::Ellipse { id, .. }
            | Self::EllipseArc { id, .. }
            | Self::HyperbolaArc { id, .. }
            | Self::ParabolaArc { id, .. }
            | Self::BSpline { id, .. }
            | Self::RationalCurve { id, .. } => *id,
        }
    }
}
