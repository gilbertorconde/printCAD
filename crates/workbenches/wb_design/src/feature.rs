//! Design feature payloads stored in the document feature tree.

use core_document::{
    BodyId, DocumentResult, FeatureError, FeatureId, WorkbenchFeature, WorkbenchId,
};
use serde::{Deserialize, Serialize};

use crate::hole_tables::{ThreadSize, ThreadStandard};

/// What a revolution or helix spins about. Every axis ends up in the
/// sketch plane: the sketch's own axes and lines lie there already, and a
/// picked edge or a datum line is taken as its shadow on the plane.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum RevolveAxis {
    /// The sketch's vertical (y) axis through the origin (the default).
    #[default]
    SketchY,
    /// The sketch's horizontal (x) axis through the origin.
    SketchX,
    /// An arbitrary in-plane axis (point + direction in sketch coordinates).
    Custom { origin: [f32; 2], dir: [f32; 2] },
    /// A straight edge of the solid, picked in the viewport, in the body's
    /// own frame.
    Edge(EdgePick),
    /// A datum line of the body.
    Datum(FeatureId),
    /// A line of the sketch itself, by its element id: a construction line
    /// drawn for the purpose, or an edge of the profile.
    SketchLine(uuid::Uuid),
    /// A straight edge another body lends this one.
    Borrowed(BorrowedRef),
    /// One of the body's own axes, which must lie in the sketch plane.
    Base(BaseAxis),
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
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum ExtrudeDirection {
    /// Along the sketch's normal (or the profile face's).
    #[default]
    Normal,
    /// Along a vector, in the body's own frame.
    Custom([f32; 3]),
    /// Along a straight edge of the solid, picked in the viewport.
    Edge(EdgePick),
    /// Along a straight edge another body lends this one.
    Borrowed(BorrowedRef),
    /// Along a datum line, or square to a datum plane.
    Datum(FeatureId),
    /// Along a line of a sketch, by its element id.
    SketchLine {
        sketch: FeatureId,
        element: uuid::Uuid,
    },
    /// Along one of the body's own axes.
    Axis(BaseAxis),
}

/// One of a body's own axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

    /// The feature it runs along, when it runs along another feature.
    pub fn reference(&self) -> Option<FeatureId> {
        match self {
            ExtrudeDirection::Datum(datum) => Some(*datum),
            ExtrudeDirection::SketchLine { sketch, .. } => Some(*sketch),
            _ => None,
        }
    }
}

/// What gives a draft's pull direction: a straight edge of the solid, or a
/// datum line.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PullRef {
    Edge(EdgePick),
    Datum(FeatureId),
}

/// A plane an extrusion stops on: one of the body's own planes, or a datum
/// plane (for a coordinate system, the plane of it named).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PlaneTarget {
    Base(core_document::BasePlane),
    Datum {
        datum: FeatureId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plane: Option<core_document::BasePlane>,
    },
}

/// A face picked in the viewport, found again by its name or else by its
/// point and normal.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FacePick {
    pub point: [f32; 3],
    pub normal: [f32; 3],
    /// The face's name when it was picked, which a rebuild finds it by
    /// (`kernel_api::naming`); zero when the pick had none, and the point
    /// finds it.
    #[serde(
        default,
        skip_serializing_if = "is_unnamed",
        deserialize_with = "kernel_api::naming::name_from_number_or_text"
    )]
    pub name: kernel_api::TopoName,
}

fn is_unnamed(name: &kernel_api::TopoName) -> bool {
    *name == 0
}

fn are_unnamed(names: &[kernel_api::TopoName; 2]) -> bool {
    names.iter().all(|n| *n == 0)
}

impl FacePick {
    /// The face `face` picked: where, which way it faced, and its name.
    pub fn of(face: impl std::borrow::Borrow<core_document::FaceRef>) -> Self {
        let face = face.borrow();
        Self {
            point: face.point,
            normal: face.normal,
            name: face.name,
        }
    }

    /// Whether `other` names the same face: the same name, or (without
    /// one) the same point.
    pub fn same_place(&self, other: &FacePick) -> bool {
        if self.name != 0 || other.name != 0 {
            return self.name == other.name;
        }
        self.point
            .iter()
            .zip(other.point)
            .all(|(a, b)| (a - b).abs() < 1e-4)
    }
}

/// Where a pad/pocket stops along the sweep direction.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
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

    /// The datum it stops on, when it stops on one.
    pub fn datum(&self) -> Option<FeatureId> {
        match self {
            ExtrudeMode::UpToPlane(PlaneTarget::Datum { datum, .. }) => Some(*datum),
            _ => None,
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

/// One section of a loft: a feature (a sketch's profile; a datum point or a
/// sketch holding a single point, where the loft closes to it) or a flat
/// face of the solid, picked. A file listing sections by id reads each as
/// a feature.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LoftSection {
    Feature(FeatureId),
    Face(FacePick),
}

impl LoftSection {
    pub fn feature(&self) -> Option<FeatureId> {
        match self {
            LoftSection::Feature(id) => Some(*id),
            LoftSection::Face(_) => None,
        }
    }
}

/// A feature placed by an attachment, as a datum is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Attached {
    pub attachment: core_document::DatumAttachment,
    #[serde(default)]
    pub offset: core_document::AttachmentOffset,
}

impl Attached {
    /// The datum plane the attachment makes.
    pub fn datum(&self) -> core_document::DatumFeature {
        core_document::DatumFeature {
            shape: core_document::DatumShape::Plane { size: 1.0 },
            attachment: self.attachment,
            offset: self.offset,
        }
    }

    /// What the attachment asks of the body's solid to follow it.
    pub fn probes(&self) -> Vec<kernel_api::ShapeProbe> {
        self.datum().probes()
    }

    /// Where the attachment puts the feature: its frame as a placement.
    pub fn placement(&self) -> kernel_api::Placement {
        let frame = self.datum().frame();
        kernel_api::Placement {
            origin: frame.origin.map(f64::from),
            x_axis: frame.x_axis.map(f64::from),
            z_axis: frame.normal.map(f64::from),
        }
    }

    /// The attachment as `datum` has it after following what it stands
    /// on.
    pub fn from_datum(datum: &core_document::DatumFeature) -> Self {
        Self {
            attachment: datum.attachment,
            offset: datum.offset,
        }
    }
}

/// A pad's or pocket's less used settings.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
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
    /// along the way it runs.
    pub start_offset: f32,
    /// The second side's taper, when it differs from the first's.
    pub taper2_deg: Option<f32>,
    /// With a slanted direction, the length is measured along the
    /// profile's normal rather than along the slant.
    pub along_normal: bool,
}

impl ExtrudeExtras {
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }
}

/// How a helix's extent is specified; the missing quantity is derived.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum HelixMode {
    #[default]
    PitchHeight,
    PitchTurns,
    HeightTurns,
    /// Height, turns and a growth per turn in place of the cone angle; a
    /// height of 0 is a flat spiral.
    HeightTurnsGrowth,
}

impl HelixMode {
    pub const ALL: [HelixMode; 4] = [
        HelixMode::PitchHeight,
        HelixMode::PitchTurns,
        HelixMode::HeightTurns,
        HelixMode::HeightTurnsGrowth,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            HelixMode::PitchHeight => "Pitch + height",
            HelixMode::PitchTurns => "Pitch + turns",
            HelixMode::HeightTurns => "Height + turns",
            HelixMode::HeightTurnsGrowth => "Height + turns + growth",
        }
    }
}

/// How a pipe's section turns as it runs down its path.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum PipeOrientation {
    /// The section neither twists nor kinks where the path bends.
    #[default]
    Standard,
    /// The section turns with the path's own curvature frame.
    Frenet,
    /// One direction of the section keeps pointing at a second path sketch
    /// running beside the first.
    Auxiliary { path: FeatureId },
    /// The section keeps a fixed binormal, a direction in the body's frame.
    Binormal { x: f32, y: f32, z: f32 },
    /// The section keeps its orientation in space, as it is drawn.
    Fixed,
}

impl PipeOrientation {
    pub fn label(&self) -> &'static str {
        match self {
            PipeOrientation::Standard => "Standard",
            PipeOrientation::Frenet => "Frenet",
            PipeOrientation::Auxiliary { .. } => "Auxiliary path",
            PipeOrientation::Binormal { .. } => "Binormal",
            PipeOrientation::Fixed => "Fixed",
        }
    }

    /// The second path sketch this orientation follows, if any.
    pub fn path(&self) -> Option<FeatureId> {
        match self {
            PipeOrientation::Auxiliary { path } => Some(*path),
            _ => None,
        }
    }
}

/// Reads a [`PipeOrientation`] or the `frenet` flag it stands in for.
fn deserialize_pipe_orientation<'de, D>(deserializer: D) -> Result<PipeOrientation, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stored {
        Flag(bool),
        Orientation(PipeOrientation),
    }
    Ok(match Stored::deserialize(deserializer)? {
        Stored::Flag(true) => PipeOrientation::Frenet,
        Stored::Flag(false) => PipeOrientation::Standard,
        Stored::Orientation(orientation) => orientation,
    })
}

/// How a pipe's section turns a sharp corner of its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PipeCorner {
    /// Carried into the corner and sheared onto the plane bisecting it.
    #[default]
    Transformed,
    /// Each leg runs on straight past the corner, the outside left square.
    Right,
    /// Turned about the corner, the outside rounded.
    Round,
}

impl PipeCorner {
    pub const ALL: [PipeCorner; 3] = [
        PipeCorner::Transformed,
        PipeCorner::Right,
        PipeCorner::Round,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            PipeCorner::Transformed => "Transformed",
            PipeCorner::Right => "Right corner",
            PipeCorner::Round => "Round corner",
        }
    }
}

/// Chamfer sizing style.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum ChamferMode {
    #[default]
    EqualDistance,
    TwoDistances,
    DistanceAngle,
}

impl ChamferMode {
    pub const ALL: [ChamferMode; 3] = [
        ChamferMode::EqualDistance,
        ChamferMode::TwoDistances,
        ChamferMode::DistanceAngle,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ChamferMode::EqualDistance => "Equal distance",
            ChamferMode::TwoDistances => "Two distances",
            ChamferMode::DistanceAngle => "Distance + angle",
        }
    }
}

/// Which edges a dress-up applies to.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum EdgeSel {
    /// Every edge of the solid (immune to topology churn).
    #[default]
    All,
    /// The edges bordering the picked faces.
    Faces(Vec<FacePick>),
    /// Edges picked one by one in the viewport.
    Edges(Vec<EdgePick>),
}

/// A picked edge, by a point on it and its direction there, re-resolved
/// against the current solid each rebuild: a rebuilt solid numbers its
/// edges afresh, but the edge nearest the point is the one picked.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EdgePick {
    pub point: [f32; 3],
    pub direction: [f32; 3],
    /// The names of the two faces the edge ran between when it was picked,
    /// which a rebuild finds it by; zeros when the pick had none.
    #[serde(
        default,
        skip_serializing_if = "are_unnamed",
        deserialize_with = "kernel_api::naming::names_from_numbers_or_text"
    )]
    pub faces: [kernel_api::TopoName; 2],
}

impl EdgePick {
    /// The edge `edge` picked: where, which way it ran, and its faces'
    /// names.
    pub fn of(edge: impl std::borrow::Borrow<core_document::EdgeRef>) -> Self {
        let edge = edge.borrow();
        Self {
            point: edge.point,
            direction: edge.direction,
            faces: edge.faces,
        }
    }
}

/// A mirror/pattern reference plane.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum MirrorPlane {
    #[default]
    XY,
    XZ,
    YZ,
    Face(FacePick),
    /// A datum plane, or one of a coordinate system's planes.
    Datum {
        datum: FeatureId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plane: Option<core_document::BasePlane>,
    },
    /// A sketch's plane, or the plane through one of its axes square to
    /// it.
    Sketch {
        sketch: FeatureId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        axis: Option<SketchAxis>,
    },
}

impl MirrorPlane {
    pub const BASE: [MirrorPlane; 3] = [MirrorPlane::XY, MirrorPlane::XZ, MirrorPlane::YZ];

    pub fn label(&self) -> &'static str {
        match self {
            MirrorPlane::XY => "XY plane",
            MirrorPlane::XZ => "XZ plane",
            MirrorPlane::YZ => "YZ plane",
            MirrorPlane::Face(_) => "Picked face",
            MirrorPlane::Datum { .. } => "Datum plane",
            MirrorPlane::Sketch { axis: None, .. } => "Sketch plane",
            MirrorPlane::Sketch { axis: Some(_), .. } => "Sketch axis",
        }
    }

    /// The feature it is taken from, when it is another feature.
    pub fn reference(&self) -> Option<FeatureId> {
        match self {
            MirrorPlane::Datum { datum, .. } => Some(*datum),
            MirrorPlane::Sketch { sketch, .. } => Some(*sketch),
            _ => None,
        }
    }

    /// The plane as (point, normal), in the body's own frame; a datum's or
    /// a sketch's, which the build works out, as the XY plane.
    pub fn plane(&self) -> ([f64; 3], [f64; 3]) {
        match self {
            MirrorPlane::Datum { .. } | MirrorPlane::Sketch { .. } => ([0.0; 3], [0.0, 0.0, 1.0]),
            MirrorPlane::XY => ([0.0; 3], [0.0, 0.0, 1.0]),
            MirrorPlane::XZ => ([0.0; 3], [0.0, 1.0, 0.0]),
            MirrorPlane::YZ => ([0.0; 3], [1.0, 0.0, 0.0]),
            MirrorPlane::Face(pick) => (
                [
                    pick.point[0] as f64,
                    pick.point[1] as f64,
                    pick.point[2] as f64,
                ],
                [
                    pick.normal[0] as f64,
                    pick.normal[1] as f64,
                    pick.normal[2] as f64,
                ],
            ),
        }
    }
}

/// Which of a sketch's own axes a pattern runs along or turns about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SketchAxis {
    /// The sketch's horizontal (x) axis through its origin.
    #[default]
    Horizontal,
    /// The sketch's vertical (y) axis through its origin.
    Vertical,
    /// The sketch's normal through its origin.
    Normal,
}

impl SketchAxis {
    pub const ALL: [SketchAxis; 3] = [
        SketchAxis::Horizontal,
        SketchAxis::Vertical,
        SketchAxis::Normal,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            SketchAxis::Horizontal => "H axis",
            SketchAxis::Vertical => "V axis",
            SketchAxis::Normal => "normal",
        }
    }
}

/// A pattern direction/axis, in the body's own frame.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum PatternAxis {
    X,
    #[default]
    Y,
    Z,
    Custom {
        origin: [f32; 3],
        dir: [f32; 3],
    },
    /// A straight edge of the solid picked in the viewport, or the axis of
    /// a circular one: its centre and its normal.
    Edge(EdgePick),
    /// A datum line of the body.
    Datum(FeatureId),
    /// One of a sketch's own axes, where the sketch sits.
    Sketch {
        sketch: FeatureId,
        axis: SketchAxis,
    },
}

impl PatternAxis {
    pub const BASE: [PatternAxis; 3] = [PatternAxis::X, PatternAxis::Y, PatternAxis::Z];

    pub fn label(&self) -> &'static str {
        match self {
            PatternAxis::X => "X axis",
            PatternAxis::Y => "Y axis",
            PatternAxis::Z => "Z axis",
            PatternAxis::Custom { .. } => "Custom axis",
            PatternAxis::Edge(_) => "Picked edge",
            PatternAxis::Datum(_) => "Datum line",
            PatternAxis::Sketch { .. } => "Sketch axis",
        }
    }

    /// The feature the axis follows, when it is a datum's or a sketch's.
    pub fn reference(&self) -> Option<FeatureId> {
        match self {
            PatternAxis::Datum(id) | PatternAxis::Sketch { sketch: id, .. } => Some(*id),
            _ => None,
        }
    }
}

/// One step of a multi-transform (each applies to every result of the
/// previous step).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TransformStep {
    Linear {
        axis: PatternAxis,
        length: f32,
        occurrences: u32,
    },
    Polar {
        axis: PatternAxis,
        angle_deg: f32,
        occurrences: u32,
    },
    Mirror {
        plane: MirrorPlane,
    },
    Scale {
        factor: f32,
        center: [f32; 3],
        occurrences: u32,
    },
    /// Rings about `axis`: ring `i` moved `i × spacing` along `outward`,
    /// then spread evenly round the axis, `counts[i]` occurrences in it,
    /// the original first in the first ring.
    Circular {
        axis: PatternAxis,
        outward: PatternAxis,
        spacing: f32,
        counts: Vec<u32>,
    },
    /// An occurrence at every point of `sketch` after its first, moved by
    /// how far that point is from the first, where the original stands.
    Points {
        sketch: FeatureId,
    },
}

/// A hexagonal pocket for a captive nut, at one end of a hole: sized
/// from the nut of the hole's metric thread, or from an across-flats of
/// its own.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NutTrap {
    /// The nut standard the size comes from.
    pub standard: crate::hole_tables::NutStandard,
    /// Across the pocket's flats, mm, in place of the nut's.
    pub across_flats: Option<f32>,
    /// Added to the nut's across-flats and to its thickness, mm, so the
    /// nut drops in.
    pub clearance: f32,
    /// How deep the pocket is, mm; the nut's thickness and the clearance
    /// when not given.
    pub depth: Option<f32>,
    /// Which end of the hole it sits at.
    pub side: NutSide,
    /// The hexagon turned about the hole's axis, degrees; at 0 a corner
    /// points along the sketch's X.
    pub turn_deg: f32,
}

impl Default for NutTrap {
    fn default() -> Self {
        Self {
            standard: Default::default(),
            across_flats: None,
            clearance: 0.3,
            depth: None,
            side: NutSide::Top,
            turn_deg: 0.0,
        }
    }
}

/// Which end of a hole a nut trap sits at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NutSide {
    /// At its mouth, on the sketch's plane.
    #[default]
    Top,
    /// At its bottom, the pocket's floor where the hole's depth ends: a
    /// nut set in while the part prints.
    Bottom,
}

/// A hole's nut trap as a script may give it: whole, `true` for the
/// usual one, or `false` for none.
pub(crate) fn nut_trap_or_flag<'de, D>(deserializer: D) -> Result<Option<NutTrap>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Given {
        Flag(bool),
        Trap(NutTrap),
    }
    Ok(match Option::<Given>::deserialize(deserializer)? {
        None | Some(Given::Flag(false)) => None,
        Some(Given::Flag(true)) => Some(NutTrap::default()),
        Some(Given::Trap(trap)) => Some(trap),
    })
}

/// What a hole cuts around its mouth, beyond the drill.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum HoleCut {
    #[default]
    None,
    Counterbore {
        diameter: f32,
        depth: f32,
    },
    Countersink {
        diameter: f32,
        angle_deg: f32,
    },
    /// A shallow counterbore: a flat seat faced into a rough or curved
    /// surface.
    Spotface {
        diameter: f32,
        depth: f32,
    },
    /// A wider bore ending in a cone that narrows to the hole, as a
    /// stepped drill leaves it: `angle_deg` is the cone's included angle.
    Counterdrill {
        diameter: f32,
        depth: f32,
        angle_deg: f32,
    },
    /// The seat of a standard screw's head, sized from the hole's metric
    /// thread size.
    Seat {
        seat: crate::hole_tables::ScrewSeat,
    },
}

impl HoleCut {
    pub fn label(&self) -> &'static str {
        match self {
            HoleCut::None => "None",
            HoleCut::Counterbore { .. } => "Counterbore",
            HoleCut::Countersink { .. } => "Countersink",
            HoleCut::Spotface { .. } => "Spotface",
            HoleCut::Counterdrill { .. } => "Counterdrill",
            HoleCut::Seat { seat } => seat.label(),
        }
    }
}

/// How long a hole's modeled thread runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThreadLength {
    /// As deep as the thread depth given.
    #[default]
    Given,
    /// The whole depth of the hole.
    HoleDepth,
    /// The hole's depth less the tap's run-out, three pitches.
    RunOut,
}

impl ThreadLength {
    pub const ALL: [ThreadLength; 3] = [
        ThreadLength::Given,
        ThreadLength::HoleDepth,
        ThreadLength::RunOut,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ThreadLength::Given => "Depth given",
            ThreadLength::HoleDepth => "Whole hole",
            ThreadLength::RunOut => "Less the run-out",
        }
    }
}

/// ISO 273 metric clearance style for a threaded hole size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HoleFit {
    Close,
    #[default]
    Normal,
    Loose,
}

impl HoleFit {
    pub const ALL: [HoleFit; 3] = [HoleFit::Close, HoleFit::Normal, HoleFit::Loose];

    pub fn label(&self) -> &'static str {
        match self {
            HoleFit::Close => "Close",
            HoleFit::Normal => "Normal",
            HoleFit::Loose => "Loose",
        }
    }
}

/// The thread a standards-driven hole is sized from.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThreadSpec {
    pub standard: ThreadStandard,
    /// The size's designation in its standard: "M6", "1/4-20", "1/2".
    pub size: String,
    /// The class of fit ("6H", "2B"); empty for the standard's default.
    #[serde(default)]
    pub class: String,
    /// A left-hand thread: a modeled one turns the other way.
    #[serde(default)]
    pub left_handed: bool,
}

/// The ISO metric coarse sizes in the order a hole's `metric_index`
/// numbers them.
const INDEXED_METRIC_SIZES: [&str; 10] = [
    "M2", "M2.5", "M3", "M4", "M5", "M6", "M8", "M10", "M12", "M16",
];

/// A thread read as a document or a script gives it: whole, {standard,
/// size, class, left_handed}; as a designation alone, "M6" or "1/4-20",
/// found in the standards that have it; or as a `metric_index`, the place
/// of an ISO metric coarse size in [`INDEXED_METRIC_SIZES`].
impl<'de> Deserialize<'de> for ThreadSpec {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let value = serde_json::Value::deserialize(d)?;
        let shapes = "a thread is a size such as \"M6\" or \"1/4-20\", or {standard, size, \
                      class, left_handed} with standard one of IsoMetricCoarse, IsoMetricFine, \
                      Unc, Unf, Unef, Bsw, Bsf, BspParallel, BspTaper, Npt";
        match &value {
            serde_json::Value::Number(n) => {
                let index = n.as_u64().ok_or_else(|| D::Error::custom(shapes))? as usize;
                Ok(ThreadSpec::new(
                    ThreadStandard::IsoMetricCoarse,
                    &INDEXED_METRIC_SIZES
                        .get(index)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("#{index}")),
                ))
            }
            serde_json::Value::String(size) => ThreadStandard::ALL
                .into_iter()
                .find(|standard| standard.sizes().iter().any(|s| s.name == size))
                .map(|standard| ThreadSpec::new(standard, size))
                .ok_or_else(|| {
                    D::Error::custom(format!("no standard has a thread {size:?}: {shapes}"))
                }),
            serde_json::Value::Object(map) => {
                let standard: ThreadStandard = match map.get("standard") {
                    Some(s) => serde_json::from_value(s.clone())
                        .map_err(|_| D::Error::custom(format!("unknown standard {s}: {shapes}")))?,
                    None => ThreadStandard::default(),
                };
                let size = map
                    .get("size")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| D::Error::custom(format!("the thread has no size: {shapes}")))?;
                Ok(ThreadSpec {
                    standard,
                    size: size.to_string(),
                    class: map
                        .get("class")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    left_handed: map
                        .get("left_handed")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                })
            }
            _ => Err(D::Error::custom(shapes)),
        }
    }
}

impl ThreadSpec {
    pub fn new(standard: ThreadStandard, size: &str) -> Self {
        Self {
            standard,
            size: size.to_string(),
            class: String::new(),
            left_handed: false,
        }
    }

    /// The class the thread is cut to: its own, or its standard's default.
    pub fn class(&self) -> &str {
        if self.class.is_empty() {
            self.standard.default_class()
        } else {
            &self.class
        }
    }

    /// The thread's size, or why the standard has none by its name or
    /// class.
    pub fn resolve(&self) -> Result<ThreadSize, String> {
        let size = self.standard.size(&self.size).ok_or_else(|| {
            format!(
                "there is no {} size \"{}\"",
                self.standard.label(),
                self.size
            )
        })?;
        let classes = self.standard.classes();
        if !self.class.is_empty() && !classes.contains(&self.class.as_str()) {
            return Err(if classes.is_empty() {
                format!("{} threads have no class to choose", self.standard.label())
            } else {
                format!(
                    "{} has no class {}; it has {}",
                    self.standard.label(),
                    self.class,
                    classes.join(", ")
                )
            });
        }
        Ok(size)
    }

    /// How far the class moves the thread's diameters out, mm.
    pub fn allowance(&self, size: &ThreadSize) -> f64 {
        self.standard.class_allowance(self.class(), size.pitch)
    }

    /// The designation as a drawing gives it: "M6-6H", "1/4-20 UNC-2B",
    /// "G 1/2", "M8x1-6G LH".
    pub fn designation(&self) -> String {
        use ThreadStandard as S;
        let mut text = match self.standard {
            S::IsoMetricCoarse | S::IsoMetricFine => self.size.clone(),
            S::Unc | S::Unf | S::Unef | S::Bsw | S::Bsf => {
                format!("{} {}", self.size, self.standard.label())
            }
            S::BspParallel => format!("G {}", self.size),
            S::BspTaper => format!("Rc {}", self.size),
            S::Npt => format!("{} NPT", self.size),
        };
        let class = self.class();
        if !class.is_empty() {
            text.push('-');
            text.push_str(class);
        }
        if self.left_handed {
            text.push_str(" LH");
        }
        text
    }
}

/// The bottom of a blind hole.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum DrillPoint {
    /// Square to the axis, as an end mill leaves it.
    #[default]
    Flat,
    /// A cone of the drill's included point angle (118° and 135° are the
    /// usual ones; 118° when none is given).
    Angled {
        #[serde(default = "drill_point_angle")]
        angle_deg: f32,
    },
}

fn z_axis() -> [f32; 3] {
    [0.0, 0.0, 1.0]
}

fn drill_point_angle() -> f32 {
    118.0
}

/// What a body borrows from another: one of its sketches, or faces and
/// edges of its solid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BorrowSource {
    /// A sketch of another body: its profile, where that body sits.
    Sketch(FeatureId),
    /// Faces and edges of another body's solid, each where it was picked,
    /// in that body's own frame, and re-found on the solid as it is built.
    Solid {
        body: BodyId,
        #[serde(default)]
        faces: Vec<FacePick>,
        #[serde(default)]
        edges: Vec<EdgePick>,
    },
}

/// Borrowed geometry as it was when it was frozen, in the borrowing body's
/// frame: what the body keeps once it stops following the source.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FrozenBorrow {
    /// The sketch, with its plane in the borrowing body's frame, as a
    /// sketch feature's data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sketch: Option<serde_json::Value>,
    #[serde(default)]
    pub faces: Vec<FrozenFace>,
    #[serde(default)]
    pub edges: Vec<FrozenEdge>,
}

/// A borrowed face as it was frozen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrozenFace {
    /// Where it was picked, in the borrowing body's frame.
    pub pick: FacePick,
    /// The face alone, as a native-format snapshot in its own body's frame.
    pub shape: String,
    /// What moves `shape` into the borrowing body's frame, a rigid
    /// row-major 4×4 matrix.
    pub transform: [[f64; 4]; 4],
    /// Its surface, in the borrowing body's frame, when the mesh knew it.
    #[serde(default)]
    pub surface: Option<kernel_api::FaceSurface>,
    /// Its outline, pairs of points in the borrowing body's frame.
    #[serde(default)]
    pub outline: Vec<[f32; 3]>,
}

/// A borrowed edge as it was frozen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrozenEdge {
    /// A point of it and its direction there, in the borrowing body's
    /// frame.
    pub pick: EdgePick,
    /// Its outline, pairs of points in the borrowing body's frame.
    #[serde(default)]
    pub outline: Vec<[f32; 3]>,
}

/// One face or edge of a borrow: the borrow feature and its place in the
/// borrow's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BorrowedRef {
    pub borrow: FeatureId,
    pub index: usize,
}

/// A solid-modeling feature in a body's linear history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DesignFeature {
    /// Start the body from its base solid: the shape an import, a mesh
    /// conversion or a repair made of it, kept by the document.
    Base {},
    /// Start the body from a copy of another body's solid, as that body
    /// is built now.
    Clone { source: BodyId },
    /// Extrude the sketch profile (or a flat face of the solid), adding
    /// material.
    Pad {
        /// The sketch whose profile it extrudes; `None` when the profile is
        /// `profile_face`.
        #[serde(default)]
        sketch: Option<FeatureId>,
        length: f32,
        /// Extrude along -normal instead of +normal.
        reversed: bool,
        /// Extrude half the length to each side of the sketch plane.
        #[serde(default)]
        symmetric: bool,
        #[serde(default)]
        mode: ExtrudeMode,
        #[serde(default)]
        length2: f32,
        #[serde(default)]
        taper_deg: f32,
        #[serde(default)]
        up_to_face: Option<FacePick>,
        #[serde(default)]
        up_to_offset: f32,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        boolean_tolerance: Option<f32>,
        /// A flat face of the solid extruded in place of a sketch: its
        /// boundaries are the profile and its outward normal the sketch's.
        #[serde(default)]
        profile_face: Option<FacePick>,
        /// A flat face another body lends this one, extruded in place of a
        /// sketch as `profile_face` is.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile_borrowed: Option<BorrowedRef>,
        #[serde(default)]
        direction: ExtrudeDirection,
        /// The faces an up-to-shape extrusion stops on.
        #[serde(default)]
        up_to_shape: Vec<FacePick>,
        /// How the second side ends, when the pad runs both ways.
        #[serde(default)]
        mode2: Option<ExtrudeMode>,
        #[serde(default)]
        up_to_face2: Option<FacePick>,
        #[serde(default)]
        up_to_offset2: f32,
        #[serde(default)]
        up_to_shape2: Vec<FacePick>,
        /// The sketch's regions it takes, by name (`wb_sketch::profile::regions`);
        /// empty takes what the sketch's profile is.
        #[serde(default)]
        regions: Vec<String>,
        #[serde(default)]
        extras: ExtrudeExtras,
    },
    /// Extrude the sketch profile (or a flat face of the solid) and
    /// subtract it (cuts against the sketch normal by default: a face
    /// sketch's normal points out of the material).
    Pocket {
        /// A cutting feature keeps only what it shares with the body
        /// instead of cutting it away.
        #[serde(default)]
        keep_inside: bool,
        /// The sketch whose profile it cuts; `None` when the profile is
        /// `profile_face`.
        #[serde(default)]
        sketch: Option<FeatureId>,
        depth: f32,
        reversed: bool,
        /// Cut half the depth to each side of the sketch plane.
        #[serde(default)]
        symmetric: bool,
        /// The same setting as `mode` ThroughAll, kept in step with it by
        /// commands and the panel; a file with only this set cuts through
        /// all.
        #[serde(default)]
        through_all: bool,
        #[serde(default)]
        mode: ExtrudeMode,
        #[serde(default)]
        depth2: f32,
        #[serde(default)]
        taper_deg: f32,
        #[serde(default)]
        up_to_face: Option<FacePick>,
        #[serde(default)]
        up_to_offset: f32,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        boolean_tolerance: Option<f32>,
        /// A flat face of the solid cut in place of a sketch's profile.
        #[serde(default)]
        profile_face: Option<FacePick>,
        /// A flat face another body lends this one, cut in place of a
        /// sketch's profile.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile_borrowed: Option<BorrowedRef>,
        /// A direction set here is the way the cut runs; along the normal
        /// it runs against it.
        #[serde(default)]
        direction: ExtrudeDirection,
        #[serde(default)]
        up_to_shape: Vec<FacePick>,
        #[serde(default)]
        mode2: Option<ExtrudeMode>,
        #[serde(default)]
        up_to_face2: Option<FacePick>,
        #[serde(default)]
        up_to_offset2: f32,
        #[serde(default)]
        up_to_shape2: Vec<FacePick>,
        /// The sketch's regions it takes, by name (`wb_sketch::profile::regions`);
        /// empty takes what the sketch's profile is.
        #[serde(default)]
        regions: Vec<String>,
        #[serde(default)]
        extras: ExtrudeExtras,
    },
    /// Revolve the sketch profile about an in-plane axis, adding material.
    Revolution {
        sketch: FeatureId,
        angle_deg: f32,
        #[serde(default)]
        axis: RevolveAxis,
        /// An axis that does not run along the sketch plane is taken as its
        /// shadow on it, rather than refused.
        #[serde(default)]
        project_axis: bool,
        #[serde(default)]
        reversed: bool,
        #[serde(default)]
        midplane: bool,
        #[serde(default)]
        second_angle_deg: Option<f32>,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        boolean_tolerance: Option<f32>,
        #[serde(default)]
        mode: RevolveMode,
        #[serde(default)]
        up_to_face: Option<FacePick>,
        /// The sketch's regions it takes, by name (`wb_sketch::profile::regions`);
        /// empty takes what the sketch's profile is.
        #[serde(default)]
        regions: Vec<String>,
    },
    /// Revolve the sketch profile and subtract it.
    Groove {
        /// A cutting feature keeps only what it shares with the body
        /// instead of cutting it away.
        #[serde(default)]
        keep_inside: bool,
        sketch: FeatureId,
        angle_deg: f32,
        #[serde(default)]
        axis: RevolveAxis,
        /// An axis that does not run along the sketch plane is taken as its
        /// shadow on it, rather than refused.
        #[serde(default)]
        project_axis: bool,
        #[serde(default)]
        reversed: bool,
        #[serde(default)]
        midplane: bool,
        #[serde(default)]
        second_angle_deg: Option<f32>,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        boolean_tolerance: Option<f32>,
        #[serde(default)]
        mode: RevolveMode,
        #[serde(default)]
        up_to_face: Option<FacePick>,
        /// The sketch's regions it takes, by name (`wb_sketch::profile::regions`);
        /// empty takes what the sketch's profile is.
        #[serde(default)]
        regions: Vec<String>,
    },
    /// Skin through two or more sections: sketches, datum points (or
    /// sketches of a single point) at either end, and flat faces of the
    /// solid.
    Loft {
        /// A cutting feature keeps only what it shares with the body
        /// instead of cutting it away.
        #[serde(default)]
        keep_inside: bool,
        sections: Vec<LoftSection>,
        ruled: bool,
        closed: bool,
        subtractive: bool,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        boolean_tolerance: Option<f32>,
    },
    /// Sweep a profile sketch along a spine sketch's path.
    Pipe {
        /// A cutting feature keeps only what it shares with the body
        /// instead of cutting it away.
        #[serde(default)]
        keep_inside: bool,
        profile: FeatureId,
        spine: FeatureId,
        /// How the section turns down the path; files that hold only the
        /// `frenet` flag read it as the frame it named.
        #[serde(
            default,
            alias = "frenet",
            deserialize_with = "deserialize_pipe_orientation"
        )]
        orientation: PipeOrientation,
        /// How the section turns the path's sharp corners.
        #[serde(default)]
        corner: PipeCorner,
        /// Further section sketches the pipe passes through down its path,
        /// in order, after the profile; the last may be a datum point or a
        /// sketch of a single point, where the pipe closes to it.
        #[serde(default)]
        sections: Vec<FeatureId>,
        subtractive: bool,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        boolean_tolerance: Option<f32>,
        /// A flat face of the solid as the profile, in place of `profile`.
        #[serde(default)]
        profile_face: Option<FacePick>,
        /// Edges of the solid, picked, as the path in place of `spine`.
        #[serde(default)]
        path_edges: Vec<EdgePick>,
        /// Edges another body lends, as the path in place of `spine`.
        #[serde(default)]
        path_borrowed: Vec<BorrowedRef>,
    },
    /// Sweep the sketch profile along a helix about an in-plane axis.
    Helix {
        sketch: FeatureId,
        axis: RevolveAxis,
        /// An axis that does not run along the sketch plane is taken as its
        /// shadow on it, rather than refused.
        #[serde(default)]
        project_axis: bool,
        mode: HelixMode,
        pitch: f32,
        height: f32,
        turns: f32,
        left_handed: bool,
        cone_angle_deg: f32,
        reversed: bool,
        subtractive: bool,
        /// How far every point moves away from the axis per turn, in
        /// [`HelixMode::HeightTurnsGrowth`].
        #[serde(default)]
        growth: f32,
        /// A subtractive helix keeps what it shares with the body instead
        /// of cutting it away.
        #[serde(default)]
        keep_inside: bool,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        boolean_tolerance: Option<f32>,
    },
    /// Parametric primitive fused into (or cut from) the body.
    Primitive {
        /// A cutting feature keeps only what it shares with the body
        /// instead of cutting it away.
        #[serde(default)]
        keep_inside: bool,
        kind: kernel_api::PrimitiveKind,
        placement: kernel_api::Placement,
        subtractive: bool,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        boolean_tolerance: Option<f32>,
        /// Placed as a datum is, by a mode on what was picked, in place of
        /// `placement`, and following what it stands on.
        #[serde(default)]
        attached: Option<Box<Attached>>,
    },
    /// Standards-aware drilled cuts at every circle center of a sketch.
    Hole {
        sketch: FeatureId,
        diameter: f32,
        depth: f32,
        through_all: bool,
        #[serde(default)]
        cut: HoleCut,
        /// The thread the hole is sized from when it is standards-driven;
        /// the diameter then derives from the thread and the fit.
        #[serde(default, alias = "metric_index")]
        thread: Option<ThreadSpec>,
        #[serde(default)]
        threaded: bool,
        /// A threaded hole's thread cut into its wall, not only its tap
        /// drill: for printing threads rather than tapping them.
        #[serde(default)]
        modeled_thread: bool,
        /// How far down a modeled thread runs, mm.
        #[serde(default)]
        thread_depth: f32,
        /// A threaded hole drilled to its tap size whose thread is shown,
        /// not cut: a stripe down the bore and its designation in the
        /// tree, never in what is exported.
        #[serde(default)]
        cosmetic_thread: bool,
        #[serde(default)]
        fit: HoleFit,
        /// A clearance diameter of one's own, in place of the fit's, mm.
        #[serde(default)]
        clearance: Option<f32>,
        /// How long a modeled thread runs.
        #[serde(default)]
        thread_length: ThreadLength,
        /// The bottom of a blind hole.
        #[serde(default)]
        drill_point: DrillPoint,
        /// The drill point lies within the depth rather than below it.
        #[serde(default)]
        point_in_depth: bool,
        /// The wall leans in toward the bottom by this angle, degrees;
        /// a tapered thread standard's taper stands in for it.
        #[serde(default)]
        taper_deg: f32,
        /// A hexagonal pocket for a captive nut at one end.
        #[serde(default, deserialize_with = "nut_trap_or_flag")]
        nut_trap: Option<NutTrap>,
        #[serde(default)]
        reversed: bool,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        boolean_tolerance: Option<f32>,
    },
    Fillet {
        radius: f32,
        #[serde(default)]
        edges: EdgeSel,
        /// Every edge meeting a selected one tangentially is taken too, on
        /// along the chain.
        #[serde(default)]
        follow_tangent: bool,
    },
    Chamfer {
        size: f32,
        #[serde(default)]
        mode: ChamferMode,
        #[serde(default)]
        size2: f32,
        #[serde(default)]
        angle_deg: f32,
        #[serde(default)]
        flip: bool,
        #[serde(default)]
        edges: EdgeSel,
        /// Every edge meeting a selected one tangentially is taken too, on
        /// along the chain.
        #[serde(default)]
        follow_tangent: bool,
    },
    Draft {
        angle_deg: f32,
        neutral: FacePick,
        faces: Vec<FacePick>,
        #[serde(default)]
        reversed: bool,
        /// One of the body's planes or a datum plane as the neutral plane,
        /// in place of `neutral`.
        #[serde(default)]
        neutral_plane: Option<PlaneTarget>,
        /// The way the draft pulls, in place of the neutral plane's normal.
        #[serde(default)]
        pull: Option<PullRef>,
    },
    /// Delete faces of the solid and close the openings from their
    /// neighbours: a hole, a boss or a round taken away.
    DeleteFaces { faces: Vec<FacePick> },
    /// Push or pull faces of the solid along their outward normals, the
    /// faces around them following: a wall thicker, a bore wider.
    OffsetFaces {
        faces: Vec<FacePick>,
        /// mm along the outward normals; negative moves them in.
        distance: f32,
    },
    /// Move faces of the solid, the faces around them following: shifted
    /// by `translation`, and turned `angle_deg` about the axis through
    /// `axis_point` along `axis_dir`.
    MoveFaces {
        faces: Vec<FacePick>,
        translation: [f32; 3],
        #[serde(default)]
        angle_deg: f32,
        #[serde(default)]
        axis_point: [f32; 3],
        #[serde(default = "z_axis")]
        axis_dir: [f32; 3],
    },
    Thickness {
        value: f32,
        faces: Vec<FacePick>,
        #[serde(default = "default_true")]
        inward: bool,
        /// How the walls meet where the solid's faces meet.
        #[serde(default)]
        join: kernel_api::ThicknessJoin,
        /// Walls on both sides of the faces, the thickness each way.
        #[serde(default)]
        both_sides: bool,
    },
    Mirrored {
        originals: Vec<FeatureId>,
        plane: MirrorPlane,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        boolean_tolerance: Option<f32>,
        /// Occurrences left out, by index: 0 is the original, then each
        /// copy in turn.
        #[serde(default)]
        skipped: Vec<u32>,
    },
    LinearPattern {
        originals: Vec<FeatureId>,
        axis: PatternAxis,
        length: f32,
        occurrences: u32,
        /// `length` is the spacing between occurrences instead of the total.
        #[serde(default)]
        spacing_mode: bool,
        #[serde(default)]
        reversed: bool,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        boolean_tolerance: Option<f32>,
        /// Uneven spacing: the gap from each occurrence to the next, first
        /// to last. A gap past the list's end is the even spacing `length`
        /// gives; an empty list spaces every occurrence evenly.
        #[serde(default)]
        spacings: Vec<f32>,
        /// Occurrences left out, by index: 0 is the original, then each
        /// copy in turn.
        #[serde(default)]
        skipped: Vec<u32>,
    },
    PolarPattern {
        originals: Vec<FeatureId>,
        axis: PatternAxis,
        angle_deg: f32,
        occurrences: u32,
        #[serde(default)]
        reversed: bool,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        boolean_tolerance: Option<f32>,
        /// `angle_deg` is the angle between occurrences instead of the
        /// overall one.
        #[serde(default)]
        step_mode: bool,
        /// Uneven steps: the angle from each occurrence to the next, first
        /// to last, degrees. A step past the list's end is the even one
        /// `angle_deg` gives; an empty list turns every occurrence evenly.
        #[serde(default)]
        angles: Vec<f32>,
        /// Occurrences left out, by index: 0 is the original, then each
        /// copy in turn.
        #[serde(default)]
        skipped: Vec<u32>,
    },
    MultiTransform {
        originals: Vec<FeatureId>,
        steps: Vec<TransformStep>,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        boolean_tolerance: Option<f32>,
        /// Occurrences left out, by index: 0 is the original, then each
        /// copy in turn.
        #[serde(default)]
        skipped: Vec<u32>,
    },
    /// Boolean against another body's built solid.
    BodyBoolean {
        tool_body: BodyId,
        kind: kernel_api::BoolKind,
        /// Further tool bodies, each taken the same way after the first.
        #[serde(default)]
        more_tools: Vec<BodyId>,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        boolean_tolerance: Option<f32>,
    },
    /// Geometry of another body lent to this one, where the two bodies
    /// sit: a sketch whose profile features here take, faces they stop on
    /// or sketch on, edges they turn about or run along. It builds nothing
    /// itself.
    Borrow {
        source: BorrowSource,
        /// The geometry as it was frozen; `None` follows the source.
        #[serde(default)]
        frozen: Option<FrozenBorrow>,
        #[serde(default)]
        options: BorrowOptions,
    },
}

/// How a borrow lends what it borrows.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BorrowOptions {
    /// Moved and turned from where the bodies put it: along and about the
    /// borrowing body's own axes (the turn about z, then the tilt about x
    /// and y).
    pub offset: core_document::AttachmentOffset,
    /// Borrowed edges that close into a loop in one plane lend the face
    /// they bound, as a profile features take.
    pub fill: bool,
    /// All of the other body's solid, drawn as reference: its edges lent,
    /// its faces to stop on.
    pub whole: bool,
}

impl BorrowOptions {
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }

    /// What the offset does to the borrowed geometry, in the borrowing
    /// body's frame.
    pub fn placement(&self) -> core_document::BodyPlacement {
        let o = &self.offset;
        let turn = glam::Quat::from_rotation_z(o.rotation_deg.to_radians())
            * glam::Quat::from_rotation_x(o.tilt[0].to_radians())
            * glam::Quat::from_rotation_y(o.tilt[1].to_radians())
            * if o.flip {
                glam::Quat::from_rotation_x(std::f32::consts::PI)
            } else {
                glam::Quat::IDENTITY
            };
        core_document::BodyPlacement::new(turn, glam::Vec3::from_array(o.translation))
    }
}

fn default_true() -> bool {
    true
}

impl DesignFeature {
    /// The sketch this feature consumes, when it is sketch-based.
    pub fn sketch(&self) -> Option<FeatureId> {
        match self {
            DesignFeature::Pad { sketch, .. } | DesignFeature::Pocket { sketch, .. } => *sketch,
            DesignFeature::Revolution { sketch, .. }
            | DesignFeature::Groove { sketch, .. }
            | DesignFeature::Helix { sketch, .. }
            | DesignFeature::Hole { sketch, .. } => Some(*sketch),
            DesignFeature::Pipe { profile, .. } => Some(*profile),
            _ => None,
        }
    }

    /// The sketch regions a pad, pocket, revolution or groove takes by
    /// name; empty for every other feature, and for one taking its whole
    /// profile.
    pub fn regions(&self) -> &[String] {
        match self {
            DesignFeature::Pad { regions, .. }
            | DesignFeature::Pocket { regions, .. }
            | DesignFeature::Revolution { regions, .. }
            | DesignFeature::Groove { regions, .. } => regions,
            _ => &[],
        }
    }

    /// Every sketch referenced by this feature.
    pub fn sketches(&self) -> Vec<FeatureId> {
        match self {
            DesignFeature::Loft { sections, .. } => {
                sections.iter().filter_map(LoftSection::feature).collect()
            }
            DesignFeature::Pipe {
                profile,
                spine,
                orientation,
                sections,
                ..
            } => {
                let mut all = vec![*profile, *spine];
                all.extend(orientation.path());
                all.extend(sections.iter().copied());
                all
            }
            _ => self.sketch().into_iter().collect(),
        }
    }

    /// The borrows whose faces or edges this feature takes: a face it
    /// stops on, an edge it turns about or runs along.
    pub fn borrows(&self) -> Vec<FeatureId> {
        let mut borrows = Vec::new();
        match self {
            DesignFeature::Pad {
                mode,
                mode2,
                direction,
                extras,
                ..
            }
            | DesignFeature::Pocket {
                mode,
                mode2,
                direction,
                extras,
                ..
            } => {
                for mode in std::iter::once(mode).chain(mode2) {
                    if let ExtrudeMode::UpToBorrowed(r) = mode {
                        borrows.push(r.borrow);
                    }
                }
                let (first, second) = mode.sides(*mode2);
                if first == ExtrudeMode::UpToShape {
                    borrows.extend(extras.up_to_shape_borrowed.iter().map(|r| r.borrow));
                }
                if second == Some(ExtrudeMode::UpToShape) {
                    borrows.extend(extras.up_to_shape_borrowed2.iter().map(|r| r.borrow));
                }
                if let ExtrudeDirection::Borrowed(r) = direction {
                    borrows.push(r.borrow);
                }
            }
            DesignFeature::Revolution { axis, mode, .. }
            | DesignFeature::Groove { axis, mode, .. } => {
                if let RevolveAxis::Borrowed(r) = axis {
                    borrows.push(r.borrow);
                }
                if let RevolveMode::UpToBorrowed(r) = mode {
                    borrows.push(r.borrow);
                }
            }
            DesignFeature::Helix {
                axis: RevolveAxis::Borrowed(r),
                ..
            } => borrows.push(r.borrow),
            _ => {}
        }
        let mut seen = std::collections::HashSet::new();
        borrows.retain(|b| seen.insert(*b));
        borrows
    }

    /// Earlier features this feature re-applies (patterns/mirror).
    pub fn originals(&self) -> Option<&[FeatureId]> {
        match self {
            DesignFeature::Mirrored { originals, .. }
            | DesignFeature::LinearPattern { originals, .. }
            | DesignFeature::PolarPattern { originals, .. }
            | DesignFeature::MultiTransform { originals, .. } => Some(originals),
            _ => None,
        }
    }

    /// The occurrences a pattern or mirror leaves out (0 the original).
    pub fn skipped(&self) -> Option<&[u32]> {
        match self {
            DesignFeature::Mirrored { skipped, .. }
            | DesignFeature::LinearPattern { skipped, .. }
            | DesignFeature::PolarPattern { skipped, .. }
            | DesignFeature::MultiTransform { skipped, .. } => Some(skipped),
            _ => None,
        }
    }

    /// Leave occurrence `index` out, or bring it back when it was: whether
    /// the feature is one with occurrences.
    pub fn toggle_skipped(&mut self, index: u32) -> bool {
        let skipped = match self {
            DesignFeature::Mirrored { skipped, .. }
            | DesignFeature::LinearPattern { skipped, .. }
            | DesignFeature::PolarPattern { skipped, .. }
            | DesignFeature::MultiTransform { skipped, .. } => skipped,
            _ => return false,
        };
        match skipped.iter().position(|&i| i == index) {
            Some(at) => {
                skipped.remove(at);
            }
            None => {
                skipped.push(index);
                skipped.sort_unstable();
            }
        }
        true
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            DesignFeature::Pad { .. } => "Pad",
            DesignFeature::Pocket { .. } => "Pocket",
            DesignFeature::Revolution { .. } => "Revolution",
            DesignFeature::Groove { .. } => "Groove",
            DesignFeature::Loft { subtractive, .. } => {
                if *subtractive {
                    "Subtractive Loft"
                } else {
                    "Additive Loft"
                }
            }
            DesignFeature::Pipe { subtractive, .. } => {
                if *subtractive {
                    "Subtractive Pipe"
                } else {
                    "Additive Pipe"
                }
            }
            DesignFeature::Helix { subtractive, .. } => {
                if *subtractive {
                    "Subtractive Helix"
                } else {
                    "Additive Helix"
                }
            }
            DesignFeature::Primitive { subtractive, .. } => {
                if *subtractive {
                    "Subtractive Primitive"
                } else {
                    "Primitive"
                }
            }
            DesignFeature::Hole { .. } => "Hole",
            DesignFeature::Fillet { .. } => "Fillet",
            DesignFeature::Chamfer { .. } => "Chamfer",
            DesignFeature::Draft { .. } => "Draft",
            DesignFeature::Thickness { .. } => "Thickness",
            DesignFeature::DeleteFaces { .. } => "Delete Faces",
            DesignFeature::OffsetFaces { .. } => "Offset Faces",
            DesignFeature::MoveFaces { .. } => "Move Faces",
            DesignFeature::Mirrored { .. } => "Mirrored",
            DesignFeature::LinearPattern { .. } => "Linear Pattern",
            DesignFeature::PolarPattern { .. } => "Polar Pattern",
            DesignFeature::MultiTransform { .. } => "Multi Transform",
            DesignFeature::BodyBoolean { .. } => "Boolean",
            DesignFeature::Borrow { .. } => "Borrowed geometry",
            DesignFeature::Base { .. } => "Base shape",
            DesignFeature::Clone { .. } => "Clone",
        }
    }

    /// The design set's icon for this feature.
    pub fn icon(&self) -> &'static str {
        use kernel_api::PrimitiveKind as P;
        match self {
            DesignFeature::Pad { .. } => "pad",
            DesignFeature::Pocket { .. } => "pocket",
            DesignFeature::Revolution { .. } => "revolution",
            DesignFeature::Groove { .. } => "groove",
            DesignFeature::Loft { subtractive, .. } => {
                if *subtractive {
                    "subtractive-loft"
                } else {
                    "additive-loft"
                }
            }
            DesignFeature::Pipe { subtractive, .. } => {
                if *subtractive {
                    "subtractive-pipe"
                } else {
                    "additive-pipe"
                }
            }
            DesignFeature::Helix { subtractive, .. } => {
                if *subtractive {
                    "subtractive-helix"
                } else {
                    "additive-helix"
                }
            }
            DesignFeature::Primitive {
                kind, subtractive, ..
            } => {
                let shape = match kind {
                    P::Box { .. } => "box",
                    P::Cylinder { .. } => "cylinder",
                    P::Sphere { .. } => "sphere",
                    P::Cone { .. } => "cone",
                    P::Torus { .. } => "torus",
                    P::Ellipsoid { .. } => "ellipsoid",
                    P::Prism { .. } => "prism",
                    P::Wedge { .. } => "wedge",
                };
                primitive_icon(shape, *subtractive)
            }
            DesignFeature::Hole { .. } => "hole",
            DesignFeature::Fillet { .. } => "fillet",
            DesignFeature::Chamfer { .. } => "chamfer",
            DesignFeature::Draft { .. } => "draft",
            DesignFeature::Thickness { .. } => "thickness",
            DesignFeature::DeleteFaces { .. } => "delete",
            DesignFeature::OffsetFaces { .. } => "offset-geometry",
            DesignFeature::MoveFaces { .. } => "move-geometry",
            DesignFeature::Mirrored { .. } => "mirrored",
            DesignFeature::LinearPattern { .. } => "linear-pattern",
            DesignFeature::PolarPattern { .. } => "polar-pattern",
            DesignFeature::MultiTransform { .. } => "multi-transform",
            DesignFeature::BodyBoolean { .. } => "boolean",
            DesignFeature::Borrow { .. } => "clone-geometry",
            DesignFeature::Base { .. } => "file-document",
            DesignFeature::Clone { .. } => "clone",
        }
    }

    /// Whether the feature merges the coplanar faces its fuse or cut
    /// leaves behind.
    pub fn refine(&self) -> bool {
        match self {
            DesignFeature::Pad { refine, .. }
            | DesignFeature::Pocket { refine, .. }
            | DesignFeature::Revolution { refine, .. }
            | DesignFeature::Groove { refine, .. }
            | DesignFeature::Loft { refine, .. }
            | DesignFeature::Pipe { refine, .. }
            | DesignFeature::Helix { refine, .. }
            | DesignFeature::Primitive { refine, .. }
            | DesignFeature::Hole { refine, .. }
            | DesignFeature::Mirrored { refine, .. }
            | DesignFeature::LinearPattern { refine, .. }
            | DesignFeature::PolarPattern { refine, .. }
            | DesignFeature::MultiTransform { refine, .. }
            | DesignFeature::BodyBoolean { refine, .. } => *refine,
            _ => false,
        }
    }

    /// Set whether the feature refines its result. A feature that neither
    /// fuses nor cuts has nothing to refine and is left as it is.
    pub fn set_refine(&mut self, on: bool) {
        match self {
            DesignFeature::Pad { refine, .. }
            | DesignFeature::Pocket { refine, .. }
            | DesignFeature::Revolution { refine, .. }
            | DesignFeature::Groove { refine, .. }
            | DesignFeature::Loft { refine, .. }
            | DesignFeature::Pipe { refine, .. }
            | DesignFeature::Helix { refine, .. }
            | DesignFeature::Primitive { refine, .. }
            | DesignFeature::Hole { refine, .. }
            | DesignFeature::Mirrored { refine, .. }
            | DesignFeature::LinearPattern { refine, .. }
            | DesignFeature::PolarPattern { refine, .. }
            | DesignFeature::MultiTransform { refine, .. }
            | DesignFeature::BodyBoolean { refine, .. } => *refine = on,
            _ => {}
        }
    }

    /// The tolerance the feature's fuse or cut runs at, when the user
    /// stated one: a positive length, zero and `None` meaning none.
    pub fn boolean_tolerance(&self) -> Option<f32> {
        match self {
            DesignFeature::Pad {
                boolean_tolerance, ..
            }
            | DesignFeature::Pocket {
                boolean_tolerance, ..
            }
            | DesignFeature::Revolution {
                boolean_tolerance, ..
            }
            | DesignFeature::Groove {
                boolean_tolerance, ..
            }
            | DesignFeature::Loft {
                boolean_tolerance, ..
            }
            | DesignFeature::Pipe {
                boolean_tolerance, ..
            }
            | DesignFeature::Helix {
                boolean_tolerance, ..
            }
            | DesignFeature::Primitive {
                boolean_tolerance, ..
            }
            | DesignFeature::Hole {
                boolean_tolerance, ..
            }
            | DesignFeature::Mirrored {
                boolean_tolerance, ..
            }
            | DesignFeature::LinearPattern {
                boolean_tolerance, ..
            }
            | DesignFeature::PolarPattern {
                boolean_tolerance, ..
            }
            | DesignFeature::MultiTransform {
                boolean_tolerance, ..
            }
            | DesignFeature::BodyBoolean {
                boolean_tolerance, ..
            } => boolean_tolerance.filter(|t| t.is_finite() && *t > 0.0),
            _ => None,
        }
    }

    /// State the tolerance the feature's fuse or cut runs at, or leave it
    /// to the kernel with `None`. A feature with no fuse or cut is left as
    /// it is.
    pub fn set_boolean_tolerance(&mut self, tolerance: Option<f32>) {
        match self {
            DesignFeature::Pad {
                boolean_tolerance, ..
            }
            | DesignFeature::Pocket {
                boolean_tolerance, ..
            }
            | DesignFeature::Revolution {
                boolean_tolerance, ..
            }
            | DesignFeature::Groove {
                boolean_tolerance, ..
            }
            | DesignFeature::Loft {
                boolean_tolerance, ..
            }
            | DesignFeature::Pipe {
                boolean_tolerance, ..
            }
            | DesignFeature::Helix {
                boolean_tolerance, ..
            }
            | DesignFeature::Primitive {
                boolean_tolerance, ..
            }
            | DesignFeature::Hole {
                boolean_tolerance, ..
            }
            | DesignFeature::Mirrored {
                boolean_tolerance, ..
            }
            | DesignFeature::LinearPattern {
                boolean_tolerance, ..
            }
            | DesignFeature::PolarPattern {
                boolean_tolerance, ..
            }
            | DesignFeature::MultiTransform {
                boolean_tolerance, ..
            }
            | DesignFeature::BodyBoolean {
                boolean_tolerance, ..
            } => *boolean_tolerance = tolerance,
            _ => {}
        }
    }

    /// Whether the feature has a refine switch at all.
    pub fn can_refine(&self) -> bool {
        let mut probe = self.clone();
        probe.set_refine(true);
        probe.refine()
    }

    /// A cutting feature that keeps what it shares with the body (the
    /// common) rather than cutting it away.
    pub fn keeps_inside(&self) -> bool {
        match self {
            DesignFeature::Pocket { keep_inside, .. }
            | DesignFeature::Groove { keep_inside, .. }
            | DesignFeature::Loft { keep_inside, .. }
            | DesignFeature::Pipe { keep_inside, .. }
            | DesignFeature::Helix { keep_inside, .. }
            | DesignFeature::Primitive { keep_inside, .. } => *keep_inside && self.is_subtractive(),
            _ => false,
        }
    }

    /// The keep-inside switch of a cutting feature that offers it in
    /// the task with the rest: Pocket, Groove, and a cutting Loft, Pipe or
    /// Primitive.
    pub fn keep_inside_mut(&mut self) -> Option<&mut bool> {
        let cutting = self.is_subtractive();
        match self {
            DesignFeature::Pocket { keep_inside, .. }
            | DesignFeature::Groove { keep_inside, .. } => Some(keep_inside),
            DesignFeature::Loft { keep_inside, .. }
            | DesignFeature::Pipe { keep_inside, .. }
            | DesignFeature::Primitive { keep_inside, .. }
                if cutting =>
            {
                Some(keep_inside)
            }
            _ => None,
        }
    }

    /// A cosmetic thread's designation ("M6", "1/4-20 2B LH"), for a
    /// threaded hole that shows its thread rather than cutting it.
    pub fn cosmetic_thread(&self) -> Option<String> {
        let DesignFeature::Hole {
            thread: Some(spec),
            threaded: true,
            modeled_thread: false,
            cosmetic_thread: true,
            ..
        } = self
        else {
            return None;
        };
        let mut name = spec.size.clone();
        if !spec.class.is_empty() {
            name = format!("{name} {}", spec.class);
        }
        if spec.left_handed {
            name.push_str(" LH");
        }
        Some(name)
    }

    /// True when this feature removes material (must not be a body's first).
    pub fn is_subtractive(&self) -> bool {
        match self {
            DesignFeature::Pocket { .. }
            | DesignFeature::Groove { .. }
            | DesignFeature::Hole { .. } => true,
            DesignFeature::Loft { subtractive, .. }
            | DesignFeature::Pipe { subtractive, .. }
            | DesignFeature::Helix { subtractive, .. }
            | DesignFeature::Primitive { subtractive, .. } => *subtractive,
            _ => false,
        }
    }

    /// True for features that modify the running solid instead of sweeping a
    /// new tool (dress-ups, patterns, booleans). These need existing material.
    pub fn is_modifier(&self) -> bool {
        matches!(
            self,
            DesignFeature::Fillet { .. }
                | DesignFeature::Chamfer { .. }
                | DesignFeature::Draft { .. }
                | DesignFeature::Thickness { .. }
                | DesignFeature::DeleteFaces { .. }
                | DesignFeature::OffsetFaces { .. }
                | DesignFeature::MoveFaces { .. }
                | DesignFeature::Mirrored { .. }
                | DesignFeature::LinearPattern { .. }
                | DesignFeature::PolarPattern { .. }
                | DesignFeature::MultiTransform { .. }
                | DesignFeature::BodyBoolean { .. }
        )
    }
}

impl WorkbenchFeature for DesignFeature {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from("wb.design")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn from_json(value: &serde_json::Value) -> DocumentResult<Self> {
        serde_json::from_value(value.clone()).map_err(|e| {
            core_document::DocumentError::Feature(FeatureError::Deserialization(e.to_string()))
        })
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        let mut deps = self.sketches();
        if let Some(originals) = self.originals() {
            deps.extend_from_slice(originals);
        }
        if let DesignFeature::Revolution {
            axis: RevolveAxis::Datum(datum),
            ..
        }
        | DesignFeature::Groove {
            axis: RevolveAxis::Datum(datum),
            ..
        }
        | DesignFeature::Helix {
            axis: RevolveAxis::Datum(datum),
            ..
        } = self
        {
            deps.push(*datum);
        }
        let axes: Vec<&PatternAxis> = match self {
            DesignFeature::LinearPattern { axis, .. }
            | DesignFeature::PolarPattern { axis, .. } => {
                vec![axis]
            }
            DesignFeature::MultiTransform { steps, .. } => steps
                .iter()
                .flat_map(|step| match step {
                    TransformStep::Linear { axis, .. } | TransformStep::Polar { axis, .. } => {
                        vec![axis]
                    }
                    TransformStep::Circular { axis, outward, .. } => vec![axis, outward],
                    _ => Vec::new(),
                })
                .collect(),
            _ => Vec::new(),
        };
        if let DesignFeature::MultiTransform { steps, .. } = self {
            for step in steps {
                if let TransformStep::Points { sketch } = step
                    && !deps.contains(sketch)
                {
                    deps.push(*sketch);
                }
            }
        }
        for reference in axes.into_iter().filter_map(PatternAxis::reference) {
            if !deps.contains(&reference) {
                deps.push(reference);
            }
        }
        if let DesignFeature::Pad {
            mode,
            mode2,
            direction,
            ..
        }
        | DesignFeature::Pocket {
            mode,
            mode2,
            direction,
            ..
        } = self
        {
            for reference in [
                mode.datum(),
                mode2.and_then(|m| m.datum()),
                direction.reference(),
            ]
            .into_iter()
            .flatten()
            {
                if !deps.contains(&reference) {
                    deps.push(reference);
                }
            }
        }
        if let DesignFeature::Draft {
            neutral_plane,
            pull,
            ..
        } = self
        {
            let datum = match neutral_plane {
                Some(PlaneTarget::Datum { datum, .. }) => Some(*datum),
                _ => None,
            };
            let line = match pull {
                Some(PullRef::Datum(datum)) => Some(*datum),
                _ => None,
            };
            for reference in [datum, line].into_iter().flatten() {
                if !deps.contains(&reference) {
                    deps.push(reference);
                }
            }
        }
        let mirrors: Vec<&MirrorPlane> = match self {
            DesignFeature::Mirrored { plane, .. } => vec![plane],
            DesignFeature::MultiTransform { steps, .. } => steps
                .iter()
                .filter_map(|step| match step {
                    TransformStep::Mirror { plane } => Some(plane),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        for reference in mirrors.into_iter().filter_map(MirrorPlane::reference) {
            if !deps.contains(&reference) {
                deps.push(reference);
            }
        }
        if let DesignFeature::Primitive {
            attached: Some(attached),
            ..
        } = self
        {
            for reference in attached.attachment.datums() {
                if !deps.contains(&reference) {
                    deps.push(reference);
                }
            }
        }
        for borrow in self.borrows() {
            if !deps.contains(&borrow) {
                deps.push(borrow);
            }
        }
        // A borrowed sketch that follows its source changes with it.
        if let DesignFeature::Borrow {
            source: BorrowSource::Sketch(sketch),
            frozen: None,
            ..
        } = self
        {
            deps.push(*sketch);
        }
        deps
    }

    fn name(&self) -> &str {
        self.kind_label()
    }
}

/// The icon of a primitive shape, additive or subtractive.
pub fn primitive_icon(shape: &str, subtractive: bool) -> &'static str {
    match (shape, subtractive) {
        ("box", false) => "additive-box",
        ("box", true) => "subtractive-box",
        ("cylinder", false) => "additive-cylinder",
        ("cylinder", true) => "subtractive-cylinder",
        ("sphere", false) => "additive-sphere",
        ("sphere", true) => "subtractive-sphere",
        ("cone", false) => "additive-cone",
        ("cone", true) => "subtractive-cone",
        ("torus", false) => "additive-torus",
        ("torus", true) => "subtractive-torus",
        ("ellipsoid", false) => "additive-ellipsoid",
        ("ellipsoid", true) => "subtractive-ellipsoid",
        ("prism", false) => "additive-prism",
        ("prism", true) => "subtractive-prism",
        ("wedge", false) => "additive-wedge",
        ("wedge", true) => "subtractive-wedge",
        (_, false) => "additive-box",
        (_, true) => "subtractive-box",
    }
}

/// A primitive of the named shape with default dimensions.
pub fn primitive_preset(shape: &str) -> Option<kernel_api::PrimitiveKind> {
    use kernel_api::PrimitiveKind as P;
    Some(match shape {
        "box" => P::Box {
            length: 10.0,
            width: 10.0,
            height: 10.0,
        },
        "cylinder" => P::Cylinder {
            radius: 5.0,
            height: 10.0,
            angle_deg: 360.0,
        },
        "sphere" => P::Sphere {
            radius: 5.0,
            angle1_deg: -90.0,
            angle2_deg: 90.0,
            angle3_deg: 360.0,
        },
        "cone" => P::Cone {
            radius1: 5.0,
            radius2: 2.0,
            height: 10.0,
            angle_deg: 360.0,
        },
        "torus" => P::Torus {
            radius1: 10.0,
            radius2: 2.0,
            angle1_deg: -180.0,
            angle2_deg: 180.0,
            angle3_deg: 360.0,
        },
        "ellipsoid" => P::Ellipsoid {
            radius1: 8.0,
            radius2: 5.0,
            radius3: 3.0,
            angle1_deg: -90.0,
            angle2_deg: 90.0,
            angle3_deg: 360.0,
        },
        "prism" => P::Prism {
            sides: 6,
            circumradius: 5.0,
            height: 10.0,
            skew_x_deg: 0.0,
            skew_y_deg: 0.0,
        },
        "wedge" => P::Wedge {
            xmin: 0.0,
            xmax: 10.0,
            ymin: 0.0,
            ymax: 10.0,
            zmin: 0.0,
            zmax: 10.0,
            x2min: 2.0,
            x2max: 8.0,
            z2min: 2.0,
            z2max: 8.0,
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_pad_json_without_new_fields_still_deserializes() {
        // A Pad with none of the termination-mode fields.
        let old = serde_json::json!({
            "Pad": { "sketch": FeatureId::new(), "length": 5.0, "reversed": false }
        });
        let feature = DesignFeature::from_json(&old).unwrap();
        assert!(matches!(
            feature,
            DesignFeature::Pad {
                symmetric: false,
                mode: ExtrudeMode::Dimension,
                taper_deg: t,
                up_to_face: None,
                ..
            } if t == 0.0
        ));
    }

    /// A feature saved before it had a boolean tolerance reads as leaving
    /// it automatic, and a stated one goes through a save and a load.
    #[test]
    fn a_boolean_tolerance_reads_automatic_when_absent_and_keeps_when_stated() {
        let old = serde_json::json!({
            "Pocket": { "sketch": FeatureId::new(), "depth": 5.0, "reversed": false }
        });
        let mut feature = DesignFeature::from_json(&old).unwrap();
        assert_eq!(feature.boolean_tolerance(), None);
        feature.set_boolean_tolerance(Some(0.02));
        let again = DesignFeature::from_json(&feature.to_json()).unwrap();
        assert_eq!(again.boolean_tolerance(), Some(0.02));
        // Zero is automatic too, and a feature with no fuse or cut has none.
        feature.set_boolean_tolerance(Some(0.0));
        assert_eq!(feature.boolean_tolerance(), None);
        let mut fillet = DesignFeature::from_json(&serde_json::json!({
            "Fillet": { "radius": 1.0, "edges": "All" }
        }))
        .unwrap();
        fillet.set_boolean_tolerance(Some(0.02));
        assert_eq!(fillet.boolean_tolerance(), None);
    }

    #[test]
    fn old_pocket_json_still_deserializes() {
        let old = serde_json::json!({
            "Pocket": {
                "sketch": FeatureId::new(),
                "depth": 5.0,
                "reversed": false,
                "through_all": true
            }
        });
        let feature = DesignFeature::from_json(&old).unwrap();
        assert!(matches!(
            feature,
            DesignFeature::Pocket {
                symmetric: false,
                through_all: true,
                mode: ExtrudeMode::Dimension,
                ..
            }
        ));
    }

    #[test]
    fn an_old_pad_reads_as_one_sided_along_the_normal_from_its_sketch() {
        let sketch = FeatureId::new();
        let old = serde_json::json!({
            "Pad": { "sketch": sketch, "length": 5.0, "reversed": false, "mode": "TwoLengths" }
        });
        let feature = DesignFeature::from_json(&old).unwrap();
        assert_eq!(feature.sketch(), Some(sketch));
        let DesignFeature::Pad {
            profile_face,
            direction,
            up_to_shape,
            mode,
            mode2,
            ..
        } = feature
        else {
            panic!("a pad");
        };
        assert_eq!(profile_face, None);
        assert_eq!(direction, ExtrudeDirection::Normal);
        assert!(up_to_shape.is_empty());
        // Two lengths is a dimension each way.
        assert_eq!(
            mode.sides(mode2),
            (ExtrudeMode::Dimension, Some(ExtrudeMode::Dimension))
        );
        assert_eq!(
            ExtrudeMode::UpToFace.sides(Some(ExtrudeMode::ThroughAll)),
            (ExtrudeMode::UpToFace, Some(ExtrudeMode::ThroughAll))
        );
        assert_eq!(
            ExtrudeMode::ToFirst.sides(None),
            (ExtrudeMode::ToFirst, None)
        );
    }

    #[test]
    fn a_datum_axis_is_a_dependency() {
        let (sketch, datum) = (FeatureId::new(), FeatureId::new());
        let old = serde_json::json!({
            "Revolution": { "sketch": sketch, "angle_deg": 180.0, "axis": {"Datum": datum} }
        });
        let feature = DesignFeature::from_json(&old).unwrap();
        assert_eq!(feature.dependencies(), vec![sketch, datum]);
    }

    #[test]
    fn old_revolution_json_still_deserializes() {
        let old = serde_json::json!({
            "Revolution": { "sketch": FeatureId::new(), "angle_deg": 180.0 }
        });
        let feature = DesignFeature::from_json(&old).unwrap();
        assert!(matches!(
            feature,
            DesignFeature::Revolution {
                axis: RevolveAxis::SketchY,
                midplane: false,
                second_angle_deg: None,
                ..
            }
        ));
    }

    #[test]
    fn an_old_pipe_reads_its_frenet_flag_as_its_orientation() {
        let old = |frenet: bool| {
            serde_json::json!({
                "Pipe": {
                    "profile": FeatureId::new(),
                    "spine": FeatureId::new(),
                    "frenet": frenet,
                    "subtractive": false
                }
            })
        };
        for (frenet, orientation) in [
            (true, PipeOrientation::Frenet),
            (false, PipeOrientation::Standard),
        ] {
            let feature = DesignFeature::from_json(&old(frenet)).unwrap();
            let DesignFeature::Pipe {
                orientation: read,
                corner,
                sections,
                ..
            } = &feature
            else {
                panic!("not a pipe: {feature:?}");
            };
            assert_eq!(*read, orientation);
            assert_eq!(*corner, PipeCorner::Transformed);
            assert!(sections.is_empty());
            // Written again, it carries the orientation and reads back.
            let again = DesignFeature::from_json(&feature.to_json()).unwrap();
            assert_eq!(again, feature);
        }
    }

    #[test]
    fn a_pipe_orientation_round_trips() {
        let feature = DesignFeature::Pipe {
            keep_inside: false,
            path_borrowed: Vec::new(),
            path_edges: Vec::new(),
            profile_face: None,
            refine: false,
            boolean_tolerance: None,
            profile: FeatureId::new(),
            spine: FeatureId::new(),
            orientation: PipeOrientation::Binormal {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            corner: PipeCorner::Round,
            sections: vec![FeatureId::new()],
            subtractive: true,
        };
        assert_eq!(
            DesignFeature::from_json(&feature.to_json()).unwrap(),
            feature
        );
    }

    #[test]
    fn an_old_helix_neither_grows_nor_keeps_inside() {
        let old = serde_json::json!({
            "Helix": {
                "sketch": FeatureId::new(), "axis": "SketchY", "mode": "PitchHeight",
                "pitch": 2.0, "height": 10.0, "turns": 5.0, "left_handed": false,
                "cone_angle_deg": 0.0, "reversed": false, "subtractive": true
            }
        });
        let feature = DesignFeature::from_json(&old).unwrap();
        assert!(matches!(
            feature,
            DesignFeature::Helix {
                keep_inside: false,
                growth,
                ..
            } if growth == 0.0
        ));
    }

    #[test]
    fn dependencies_cover_sketches_and_originals() {
        let a = FeatureId::new();
        let b = FeatureId::new();
        let pipe = DesignFeature::Pipe {
            keep_inside: false,
            path_borrowed: Vec::new(),
            path_edges: Vec::new(),
            profile_face: None,
            refine: false,
            boolean_tolerance: None,
            profile: a,
            spine: b,
            orientation: PipeOrientation::Standard,
            corner: PipeCorner::Transformed,
            sections: Vec::new(),
            subtractive: false,
        };
        assert_eq!(pipe.dependencies(), vec![a, b]);

        let (c, d) = (FeatureId::new(), FeatureId::new());
        let guided = DesignFeature::Pipe {
            keep_inside: false,
            path_borrowed: Vec::new(),
            path_edges: Vec::new(),
            profile_face: None,
            refine: false,
            boolean_tolerance: None,
            profile: a,
            spine: b,
            orientation: PipeOrientation::Auxiliary { path: c },
            corner: PipeCorner::Transformed,
            sections: vec![d],
            subtractive: false,
        };
        assert_eq!(guided.dependencies(), vec![a, b, c, d]);

        let pattern = DesignFeature::LinearPattern {
            skipped: Vec::new(),
            refine: false,
            boolean_tolerance: None,
            originals: vec![a],
            axis: PatternAxis::X,
            length: 10.0,
            occurrences: 3,
            spacing_mode: false,
            reversed: false,
            spacings: Vec::new(),
        };
        assert_eq!(pattern.dependencies(), vec![a]);
    }

    #[test]
    fn a_pattern_saved_before_skipping_skips_nothing_and_a_toggle_flips_one() {
        let original = FeatureId::new();
        let saved = serde_json::json!({
            "LinearPattern": {
                "originals": [original], "axis": "X", "length": 10.0, "occurrences": 3
            }
        });
        let mut pattern = DesignFeature::from_json(&saved).unwrap();
        assert_eq!(pattern.skipped(), Some(&[][..]));
        assert!(pattern.toggle_skipped(2));
        assert!(pattern.toggle_skipped(0));
        assert_eq!(pattern.skipped(), Some(&[0, 2][..]));
        assert!(pattern.toggle_skipped(2));
        assert_eq!(pattern.skipped(), Some(&[0][..]));
        let read = DesignFeature::from_json(&pattern.to_json()).unwrap();
        assert_eq!(read.skipped(), Some(&[0][..]));
        let mut fillet =
            DesignFeature::from_json(&serde_json::json!({ "Fillet": { "radius": 1.0 } })).unwrap();
        assert!(!fillet.toggle_skipped(0));
        assert_eq!(fillet.skipped(), None);
    }

    #[test]
    fn an_old_fillet_or_chamfer_takes_its_edges_alone() {
        let fillet = serde_json::json!({ "Fillet": { "radius": 1.0 } });
        assert!(matches!(
            DesignFeature::from_json(&fillet).unwrap(),
            DesignFeature::Fillet {
                follow_tangent: false,
                ..
            }
        ));
        let chamfer = serde_json::json!({ "Chamfer": { "size": 1.0 } });
        assert!(matches!(
            DesignFeature::from_json(&chamfer).unwrap(),
            DesignFeature::Chamfer {
                follow_tangent: false,
                ..
            }
        ));
    }

    #[test]
    fn a_pattern_follows_the_datum_or_sketch_its_axis_comes_from() {
        let (original, datum, sketch) = (FeatureId::new(), FeatureId::new(), FeatureId::new());
        let linear = serde_json::json!({
            "LinearPattern": {
                "originals": [original], "axis": {"Datum": datum},
                "length": 10.0, "occurrences": 3
            }
        });
        let linear = DesignFeature::from_json(&linear).unwrap();
        assert_eq!(linear.dependencies(), vec![original, datum]);
        let polar = serde_json::json!({
            "PolarPattern": {
                "originals": [original],
                "axis": {"Sketch": {"sketch": sketch, "axis": "Normal"}},
                "angle_deg": 90.0, "occurrences": 3
            }
        });
        let polar = DesignFeature::from_json(&polar).unwrap();
        assert_eq!(polar.dependencies(), vec![original, sketch]);
        assert!(matches!(
            polar,
            DesignFeature::PolarPattern { step_mode: false, ref angles, .. } if angles.is_empty()
        ));
    }
}
