//! The settings of extruded and revolved features: their axes,
//! directions, modes and less used extras.

use super::*;

/// What a revolution or helix spins about. Every axis ends up in the
/// sketch plane: the sketch's own axes and lines lie there already, and a
/// picked edge or a datum line is taken as its shadow on the plane.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Walk)]
pub enum RevolveAxis {
    /// The sketch's vertical (y) axis through the origin (the default).
    #[default]
    SketchY,
    /// The sketch's horizontal (x) axis through the origin.
    SketchX,
    /// An arbitrary in-plane axis (point + direction in sketch coordinates).
    Custom {
        #[walk(setting)]
        origin: [f32; 2],
        #[walk(setting)]
        dir: [f32; 2],
    },
    /// A straight edge of the solid, picked in the viewport, in the body's
    /// own frame.
    Edge(EdgePick),
    /// A datum line of the body.
    Datum(FeatureId),
    /// A line of the sketch itself, by its element id: a construction line
    /// drawn for the purpose, or an edge of the profile.
    SketchLine(#[walk(setting)] uuid::Uuid),
    /// A straight edge another body lends this one.
    Borrowed(BorrowedRef),
    /// One of the body's own axes, which must lie in the sketch plane.
    Base(#[walk(setting)] BaseAxis),
    /// Square to the sketch through its origin: a helix's alone, which
    /// climbs about it with the profile level.
    SketchNormal,
}

impl RevolveAxis {
    pub fn label(&self) -> &'static str {
        match self {
            RevolveAxis::SketchY => "Sketch Y axis",
            RevolveAxis::SketchX => "Sketch X axis",
            RevolveAxis::Custom { .. } => "Custom axis",
            RevolveAxis::Edge(_) => "Picked edge",
            RevolveAxis::Datum(_) => "Datum line",
            RevolveAxis::SketchLine(_) => "Sketch line",
            RevolveAxis::Borrowed(_) => "Borrowed edge",
            RevolveAxis::Base(axis) => axis.label(),
            RevolveAxis::SketchNormal => "Sketch normal",
        }
    }
}

/// Where a revolution or groove stops turning.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Walk)]
pub enum RevolveMode {
    /// Through its angle (or two, or centred on the sketch).
    #[default]
    Angle,
    /// On the first face of the existing material it meets.
    ToFirst,
    /// On the last face of the existing material it meets.
    ToLast,
    /// On a picked flat face whose plane holds the axis.
    UpToFace,
    /// On a face another body lends.
    UpToBorrowed(BorrowedRef),
}

impl RevolveMode {
    pub const ALL: [RevolveMode; 4] = [
        RevolveMode::Angle,
        RevolveMode::ToFirst,
        RevolveMode::ToLast,
        RevolveMode::UpToFace,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            RevolveMode::Angle => "Angle",
            RevolveMode::ToFirst => "To first",
            RevolveMode::ToLast => "To last",
            RevolveMode::UpToFace => "Up to face",
            RevolveMode::UpToBorrowed(_) => "Up to borrowed face",
        }
    }
}

/// Which way a pad or pocket runs.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Walk)]
pub enum ExtrudeDirection {
    /// Along the sketch's normal (or the profile face's).
    #[default]
    Normal,
    /// Along a vector, in the body's own frame.
    Custom(#[walk(axes("direction_", "Direction ", number))] [f32; 3]),
    /// Along a straight edge of the solid, picked in the viewport.
    Edge(EdgePick),
    /// Along a straight edge another body lends this one.
    Borrowed(BorrowedRef),
    /// Along a datum line, or square to a datum plane.
    Datum(FeatureId),
    /// Along a line of a sketch, by its element id.
    SketchLine {
        sketch: FeatureId,
        #[walk(setting)]
        element: uuid::Uuid,
    },
    /// Along one of the body's own axes.
    Axis(#[walk(setting)] BaseAxis),
}

/// One of a body's own axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Describe)]
pub enum BaseAxis {
    X,
    Y,
    Z,
}

impl BaseAxis {
    pub const ALL: [BaseAxis; 3] = [BaseAxis::X, BaseAxis::Y, BaseAxis::Z];

    pub fn vector(&self) -> [f64; 3] {
        match self {
            BaseAxis::X => [1.0, 0.0, 0.0],
            BaseAxis::Y => [0.0, 1.0, 0.0],
            BaseAxis::Z => [0.0, 0.0, 1.0],
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            BaseAxis::X => "X axis",
            BaseAxis::Y => "Y axis",
            BaseAxis::Z => "Z axis",
        }
    }
}

impl ExtrudeDirection {
    pub fn label(&self) -> &'static str {
        match self {
            ExtrudeDirection::Normal => "Sketch normal",
            ExtrudeDirection::Custom(_) => "Custom vector",
            ExtrudeDirection::Edge(_) => "Picked edge",
            ExtrudeDirection::Borrowed(_) => "Borrowed edge",
            ExtrudeDirection::Datum(_) => "Datum",
            ExtrudeDirection::SketchLine { .. } => "Sketch line",
            ExtrudeDirection::Axis(axis) => axis.label(),
        }
    }

    /// The direction set, in the body's own frame; `None` along the normal.
    /// A borrowed edge's, a datum's and a sketch line's are where they
    /// stand, which the build works out.
    pub fn vector(&self) -> Option<[f64; 3]> {
        match self {
            ExtrudeDirection::Normal
            | ExtrudeDirection::Borrowed(_)
            | ExtrudeDirection::Datum(_)
            | ExtrudeDirection::SketchLine { .. } => None,
            ExtrudeDirection::Custom(v) => Some(v.map(f64::from)),
            ExtrudeDirection::Edge(edge) => Some(edge.direction.map(f64::from)),
            ExtrudeDirection::Axis(axis) => Some(axis.vector()),
        }
    }
}

/// Where a pad/pocket stops along the sweep direction.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Walk)]
pub enum ExtrudeMode {
    /// Fixed length/depth.
    #[default]
    Dimension,
    /// Independent lengths to each side of the sketch plane.
    TwoLengths,
    /// Through every face of the existing material.
    ThroughAll,
    /// Stop at the first face hit along the direction.
    ToFirst,
    /// Stop at the last face hit along the direction.
    ToLast,
    /// Stop on a picked face (plus offset).
    UpToFace,
    /// Stop on a set of picked faces, each line of the sweep at the first
    /// of them it meets (plus offset along the sweep).
    UpToShape,
    /// Stop on a face another body lends (plus offset).
    UpToBorrowed(BorrowedRef),
    /// Stop on one of the body's planes or a datum plane (plus offset).
    UpToPlane(PlaneTarget),
}

impl ExtrudeMode {
    pub const ALL: [ExtrudeMode; 7] = [
        ExtrudeMode::Dimension,
        ExtrudeMode::TwoLengths,
        ExtrudeMode::ThroughAll,
        ExtrudeMode::ToFirst,
        ExtrudeMode::ToLast,
        ExtrudeMode::UpToFace,
        ExtrudeMode::UpToShape,
    ];

    /// The ways the second side of a two-sided extrusion may end.
    pub const SECOND_SIDE: [ExtrudeMode; 6] = [
        ExtrudeMode::Dimension,
        ExtrudeMode::ThroughAll,
        ExtrudeMode::ToFirst,
        ExtrudeMode::ToLast,
        ExtrudeMode::UpToFace,
        ExtrudeMode::UpToShape,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ExtrudeMode::Dimension => "Dimension",
            ExtrudeMode::TwoLengths => "Two lengths",
            ExtrudeMode::ThroughAll => "Through all",
            ExtrudeMode::ToFirst => "To first",
            ExtrudeMode::ToLast => "To last",
            ExtrudeMode::UpToFace => "Up to face",
            ExtrudeMode::UpToShape => "Up to shape",
            ExtrudeMode::UpToBorrowed(_) => "Up to borrowed face",
            ExtrudeMode::UpToPlane(_) => "Up to plane",
        }
    }

    /// Whether the mode ends on existing material, so needs some.
    pub fn needs_material(&self) -> bool {
        matches!(
            self,
            ExtrudeMode::ThroughAll
                | ExtrudeMode::ToFirst
                | ExtrudeMode::ToLast
                | ExtrudeMode::UpToShape
        )
    }

    /// Whether Symmetric acts with this end condition and `mode2`: a
    /// single dimension, centred on the profile, or through all, which
    /// it runs both ways. Everywhere else it is neither shown nor read.
    pub fn takes_symmetric(self, mode2: Option<ExtrudeMode>) -> bool {
        mode2.is_none() && matches!(self, ExtrudeMode::Dimension | ExtrudeMode::ThroughAll)
    }

    /// The end condition of each side: the first, and the second when the
    /// extrusion runs both ways. Two lengths is a dimension each way, the
    /// second side ending as `mode2` says when it is set.
    pub fn sides(self, mode2: Option<ExtrudeMode>) -> (ExtrudeMode, Option<ExtrudeMode>) {
        match self {
            ExtrudeMode::TwoLengths => (
                ExtrudeMode::Dimension,
                Some(mode2.unwrap_or(ExtrudeMode::Dimension)),
            ),
            mode => (mode, mode2),
        }
    }
}

/// A pad's or pocket's less used settings.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Walk)]
#[serde(default)]
pub struct ExtrudeExtras {
    /// Faces other bodies lend that an up-to-shape side stops on, beside
    /// the faces of its own solid it picks.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub up_to_shape_borrowed: Vec<BorrowedRef>,
    /// The same for the second side.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub up_to_shape_borrowed2: Vec<BorrowedRef>,
    /// Millimetres the extrusion starts away from the profile's plane,
    /// along the way it runs; from `start_at`'s plane when that is set.
    #[walk(length("start_offset", "Start offset"))]
    pub start_offset: f32,
    /// A datum plane or a sketch whose plane the extrusion starts on, the
    /// profile carried there along the way it runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_at: Option<FeatureId>,
    /// A flat face of the solid the extrusion starts on, in place of
    /// `start_at`: found again on the solid before the feature, as a
    /// sketch placed on a face is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_face: Option<FacePick>,
    /// The second side's taper, when it differs from the first's.
    #[walk(setting)]
    pub taper2_deg: Option<f32>,
    /// With a slanted direction, the length is measured along the
    /// profile's normal rather than along the slant.
    #[walk(setting)]
    pub along_normal: bool,
}

impl ExtrudeExtras {
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }

    /// The profile does not start where it is drawn.
    pub fn moves_start(&self) -> bool {
        self.start_offset != 0.0 || self.start_at.is_some() || self.start_face.is_some()
    }
}
