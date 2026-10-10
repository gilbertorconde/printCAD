//! Ordered semantic input. Coordinates and lengths share the caller's
//! length unit; directions and angles are expressed in radians.

macro_rules! id {
    ($name:ident, u128) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        pub struct $name(#[cfg_attr(feature = "serde", serde(with = "id_wire"))] pub u128);
    };
    ($name:ident, $inner:ty) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        pub struct $name(pub $inner);
    };
}

id!(PointId, u128);
id!(CurveId, u128);
id!(ConstraintId, u128);
id!(VariableIndex, usize);
id!(EquationIndex, usize);
id!(ControlEdgeIndex, u32);

#[cfg(feature = "serde")]
mod id_wire {
    use serde::{Deserializer, Serializer};
    pub fn serialize<S: Serializer>(value: &u128, serializer: S) -> Result<S::Ok, S::Error> {
        if let Ok(value) = u64::try_from(*value) {
            serializer.serialize_u64(value)
        } else {
            serializer.serialize_str(&value.to_string())
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u128, D::Error> {
        struct Identity;
        impl serde::de::Visitor<'_> for Identity {
            type Value = u128;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an unsigned integer or a decimal u128 identity string")
            }
            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<u128, E> {
                Ok(u128::from(value))
            }
            fn visit_u128<E: serde::de::Error>(self, value: u128) -> Result<u128, E> {
                Ok(value)
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<u128, E> {
                value.parse().map_err(E::custom)
            }
        }
        deserializer.deserialize_any(Identity)
    }
}

macro_rules! quantity {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        pub struct $name(pub f64);
        impl From<$name> for f64 {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

quantity!(Length);
quantity!(Radians);
quantity!(Ratio);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PointReference {
    Point(PointId),
    Origin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CurveReference {
    Curve(CurveId),
    XAxis,
    YAxis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ItemReference {
    Point(PointReference),
    Curve(CurveReference),
}

impl From<PointReference> for ItemReference {
    fn from(value: PointReference) -> Self {
        Self::Point(value)
    }
}
impl From<CurveReference> for ItemReference {
    fn from(value: CurveReference) -> Self {
        Self::Curve(value)
    }
}
impl From<PointId> for PointReference {
    fn from(value: PointId) -> Self {
        Self::Point(value)
    }
}
impl From<CurveId> for CurveReference {
    fn from(value: CurveId) -> Self {
        Self::Curve(value)
    }
}
impl From<PointId> for ItemReference {
    fn from(value: PointId) -> Self {
        Self::Point(value.into())
    }
}
impl From<CurveId> for ItemReference {
    fn from(value: CurveId) -> Self {
        Self::Curve(value.into())
    }
}

/// A vector in the caller's length unit.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Vector {
    pub x: f64,
    pub y: f64,
}

/// A dimensionless unit direction, or zero for a degenerate direction.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Direction {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Point {
    pub id: PointId,
    pub position: Vector,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Line {
    pub id: CurveId,
    pub start: PointReference,
    pub end: PointReference,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Circle {
    pub id: CurveId,
    pub center: PointReference,
    pub radius: Length,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Arc {
    pub id: CurveId,
    pub center: PointReference,
    pub start: PointReference,
    pub end: PointReference,
    pub radius: Length,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ArcEnds {
    pub start: PointReference,
    pub end: PointReference,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Ellipse {
    pub id: CurveId,
    pub center: PointReference,
    pub major: Vector,
    /// Supplied independently to preserve the caller's storage rounding.
    pub minor: Length,
    pub arc: Option<ArcEnds>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ConicKind {
    Hyperbola,
    Parabola,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Conic {
    pub id: CurveId,
    pub kind: ConicKind,
    pub center: PointReference,
    pub axis: Vector,
    pub minor: Length,
    pub start: PointReference,
    pub end: PointReference,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BSpline {
    pub id: CurveId,
    pub degree: u32,
    pub control_points: Vec<PointReference>,
    pub knots: Vec<f64>,
    pub periodic: bool,
    pub weights: Vec<f64>,
    /// Fit points participate in topology; refitting remains a caller action.
    pub fit_points: Vec<PointReference>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Geometry {
    Point(Point),
    Line(Line),
    Circle(Circle),
    Arc(Arc),
    Ellipse(Ellipse),
    Conic(Conic),
    BSpline(BSpline),
}

impl Geometry {
    pub fn id(&self) -> ItemReference {
        match self {
            Self::Point(p) => p.id.into(),
            Self::Line(l) => l.id.into(),
            Self::Circle(c) => c.id.into(),
            Self::Arc(a) => a.id.into(),
            Self::Ellipse(e) => e.id.into(),
            Self::Conic(c) => c.id.into(),
            Self::BSpline(b) => b.id.into(),
        }
    }

    pub fn point_references(&self) -> Vec<PointReference> {
        match self {
            Self::Point(_) => Vec::new(),
            Self::Line(l) => vec![l.start, l.end],
            Self::Circle(c) => vec![c.center],
            Self::Arc(a) => vec![a.center, a.start, a.end],
            Self::Ellipse(e) => {
                let mut points = vec![e.center];
                if let Some(a) = e.arc {
                    points.extend([a.start, a.end]);
                }
                points
            }
            Self::Conic(c) => vec![c.center, c.start, c.end],
            Self::BSpline(b) => b
                .control_points
                .iter()
                .chain(&b.fit_points)
                .copied()
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AxisDirection {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum InternalRole {
    MajorAxis,
    MinorAxis,
    Focus1,
    Focus2,
    ControlEdge(ControlEdgeIndex),
}

/// Relations use distinct point and curve references. General items occur
/// only where the relation's domain permits either kind of geometry.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Relation {
    FixedPoint {
        point: PointReference,
        position: Vector,
    },
    Coincident {
        point1: PointReference,
        point2: PointReference,
    },
    Parallel {
        line1: CurveReference,
        line2: CurveReference,
    },
    Perpendicular {
        line1: CurveReference,
        line2: CurveReference,
    },
    EqualLength {
        line1: CurveReference,
        line2: CurveReference,
    },
    Length {
        line: CurveReference,
        length: Length,
    },
    EqualRadius {
        circle1: CurveReference,
        circle2: CurveReference,
    },
    Radius {
        circle: CurveReference,
        radius: Length,
    },
    Diameter {
        circle: CurveReference,
        diameter: Length,
    },
    PointOnLine {
        point: PointReference,
        line: CurveReference,
    },
    PointOnCircle {
        point: PointReference,
        circle: CurveReference,
    },
    PointOnEllipse {
        point: PointReference,
        ellipse: CurveReference,
    },
    Horizontal {
        element: CurveReference,
    },
    Vertical {
        element: CurveReference,
    },
    HorizontalPoints {
        point1: PointReference,
        point2: PointReference,
    },
    VerticalPoints {
        point1: PointReference,
        point2: PointReference,
    },
    Block {
        element: ItemReference,
    },
    Distance {
        point1: PointReference,
        point2: PointReference,
        distance: Length,
    },
    /// Positive values are absolute; negative values select a signed branch.
    DistanceX {
        a: PointReference,
        b: Option<PointReference>,
        value: Length,
    },
    DistanceY {
        a: PointReference,
        b: Option<PointReference>,
        value: Length,
    },
    Angle {
        line1: CurveReference,
        line2: CurveReference,
        angle_rad: Radians,
    },
    AngleToAxis {
        line: CurveReference,
        axis: AxisDirection,
        angle_rad: Radians,
    },
    Tangent {
        line_or_circle1: CurveReference,
        item2: CurveReference,
    },
    Symmetric {
        point1: PointReference,
        point2: PointReference,
        line: CurveReference,
    },
    SymmetricAboutPoint {
        point1: PointReference,
        point2: PointReference,
        center: PointReference,
    },
    Midpoint {
        point: PointReference,
        line: CurveReference,
    },
    ArcLength {
        arc: CurveReference,
        length: Length,
    },
    Gap {
        item1: ItemReference,
        item2: ItemReference,
        distance: Length,
    },
    AngleAtPoint {
        curve1: CurveReference,
        curve2: CurveReference,
        point: PointReference,
        angle_rad: Radians,
    },
    EllipseRadius {
        ellipse: CurveReference,
        major: bool,
        radius: Length,
    },
    CurveLength {
        curve: CurveReference,
        length: Length,
    },
    PointOnCurve {
        point: PointReference,
        curve: CurveReference,
    },
    TangentCurves {
        curve1: CurveReference,
        curve2: CurveReference,
    },
    PerpendicularCurves {
        curve1: CurveReference,
        curve2: CurveReference,
    },
    EqualEllipse {
        ellipse1: CurveReference,
        ellipse2: CurveReference,
    },
    Offset {
        pairs: Vec<[CurveReference; 2]>,
        distance: Length,
    },
    Pitch {
        points: Vec<PointReference>,
        columns: u32,
        distance: Length,
        across: bool,
        direction: Option<Direction>,
    },
    PolarPitch {
        center: PointReference,
        points: Vec<PointReference>,
        angle_rad: Radians,
    },
    ArcAngle {
        arc: CurveReference,
        angle_rad: Radians,
    },
    AngleThreePoints {
        point1: PointReference,
        vertex: PointReference,
        point2: PointReference,
        angle_rad: Radians,
    },
    Refraction {
        ray1: CurveReference,
        ray2: CurveReference,
        interface: CurveReference,
        point: PointReference,
        ratio: Ratio,
    },
    InternalAlignment {
        element: ItemReference,
        curve: CurveReference,
        role: InternalRole,
    },
}

impl Relation {
    pub fn references(&self) -> Vec<ItemReference> {
        match self {
            Self::FixedPoint { point, .. } => vec![(*point).into()],
            Self::Coincident { point1, point2 }
            | Self::HorizontalPoints { point1, point2 }
            | Self::VerticalPoints { point1, point2 }
            | Self::Distance { point1, point2, .. } => vec![(*point1).into(), (*point2).into()],
            Self::Parallel { line1, line2 }
            | Self::Perpendicular { line1, line2 }
            | Self::EqualLength { line1, line2 }
            | Self::Angle { line1, line2, .. } => vec![(*line1).into(), (*line2).into()],
            Self::EqualRadius { circle1, circle2 } => vec![(*circle1).into(), (*circle2).into()],
            Self::EqualEllipse { ellipse1, ellipse2 } => {
                vec![(*ellipse1).into(), (*ellipse2).into()]
            }
            Self::Length { line, .. } | Self::AngleToAxis { line, .. } => vec![(*line).into()],
            Self::Radius { circle, .. } | Self::Diameter { circle, .. } => vec![(*circle).into()],
            Self::EllipseRadius { ellipse, .. } => vec![(*ellipse).into()],
            Self::CurveLength { curve, .. } => vec![(*curve).into()],
            Self::ArcLength { arc, .. } | Self::ArcAngle { arc, .. } => vec![(*arc).into()],
            Self::PointOnLine { point, line } | Self::Midpoint { point, line } => {
                vec![(*point).into(), (*line).into()]
            }
            Self::PointOnCircle { point, circle } => vec![(*point).into(), (*circle).into()],
            Self::PointOnEllipse { point, ellipse } => vec![(*point).into(), (*ellipse).into()],
            Self::PointOnCurve { point, curve } => vec![(*point).into(), (*curve).into()],
            Self::Horizontal { element } | Self::Vertical { element } => vec![(*element).into()],
            Self::Block { element } => vec![*element],
            Self::DistanceX { a, b, .. } | Self::DistanceY { a, b, .. } => std::iter::once(*a)
                .chain(b.iter().copied())
                .map(Into::into)
                .collect(),
            Self::Tangent {
                line_or_circle1,
                item2,
            } => vec![(*line_or_circle1).into(), (*item2).into()],
            Self::Symmetric {
                point1,
                point2,
                line,
            } => vec![(*point1).into(), (*point2).into(), (*line).into()],
            Self::SymmetricAboutPoint {
                point1,
                point2,
                center,
            } => vec![(*point1).into(), (*point2).into(), (*center).into()],
            Self::Gap { item1, item2, .. } => vec![*item1, *item2],
            Self::AngleAtPoint {
                curve1,
                curve2,
                point,
                ..
            } => vec![(*curve1).into(), (*curve2).into(), (*point).into()],
            Self::TangentCurves { curve1, curve2 }
            | Self::PerpendicularCurves { curve1, curve2 } => {
                vec![(*curve1).into(), (*curve2).into()]
            }
            Self::Offset { pairs, .. } => pairs.iter().flatten().copied().map(Into::into).collect(),
            Self::Pitch { points, .. } => points.iter().copied().map(Into::into).collect(),
            Self::PolarPitch { center, points, .. } => std::iter::once(*center)
                .chain(points.iter().copied())
                .map(Into::into)
                .collect(),
            Self::AngleThreePoints {
                point1,
                vertex,
                point2,
                ..
            } => vec![(*point1).into(), (*vertex).into(), (*point2).into()],
            Self::Refraction {
                ray1,
                ray2,
                interface,
                point,
                ..
            } => vec![
                (*ray1).into(),
                (*ray2).into(),
                (*interface).into(),
                (*point).into(),
            ],
            Self::InternalAlignment { element, curve, .. } => vec![*element, (*curve).into()],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Constraint {
    pub id: ConstraintId,
    pub kind: Relation,
}

/// Numerical settings are data, independent of process environment.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Settings {
    pub max_iterations: u32,
    pub tolerance: f64,
    pub max_inner_retries: usize,
    pub lambda_initial: f64,
    pub lambda_min: f64,
    pub lambda_max: f64,
    pub damping_floor: f64,
    pub finite_difference_step: f64,
    pub minimum_length: Length,
    pub maximum_step_scale: f64,
    pub rank_tolerance: f64,
    pub diagnosis_limit: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_iterations: 100,
            tolerance: 1e-9,
            max_inner_retries: 25,
            lambda_initial: 1e-3,
            lambda_min: 1e-12,
            lambda_max: 1e12,
            damping_floor: 1e-12,
            finite_difference_step: 1e-6,
            minimum_length: Length(1e-12),
            maximum_step_scale: 100.0,
            rank_tolerance: 1e-8,
            diagnosis_limit: 60,
        }
    }
}

/// Geometry order, constraint order and ID order are observable in
/// compilation. Callers retain their own geometry and apply answers.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Problem {
    pub geometry: Vec<Geometry>,
    pub constraints: Vec<Constraint>,
    pub external: Vec<ItemReference>,
    pub held_points: Vec<PointReference>,
    pub application_held_points: Vec<PointReference>,
    pub settings: Settings,
}

impl Problem {
    pub fn point_position(&self, id: PointReference) -> Option<Vector> {
        if id == PointReference::Origin {
            return Some(Vector { x: 0.0, y: 0.0 });
        }
        match self.get_geometry(id)? {
            Geometry::Point(p) => Some(p.position),
            _ => None,
        }
    }

    pub fn get_geometry(&self, id: impl Into<ItemReference>) -> Option<&Geometry> {
        let id = id.into();
        self.geometry.iter().find(|g| g.id() == id)
    }
}
