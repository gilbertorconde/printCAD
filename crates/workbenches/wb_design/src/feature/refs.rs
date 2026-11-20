//! References a feature keeps to the solid and to datums: faces and edges
//! picked by where they lie, the planes a feature pulls up to, and a
//! placement attached to a datum.

use super::*;

/// What gives a draft's pull direction: a straight edge of the solid, or a
/// datum line.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Walk)]
pub enum PullRef {
    Edge(EdgePick),
    Datum(FeatureId),
}

/// A plane an extrusion stops on: one of the body's own planes, or a datum
/// plane (for a coordinate system, the plane of it named).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Walk)]
pub enum PlaneTarget {
    Base(#[walk(setting)] core_document::BasePlane),
    Datum {
        datum: FeatureId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[walk(setting)]
        plane: Option<core_document::BasePlane>,
    },
}

/// A face picked in the viewport, found again by its name or else by its
/// point and normal.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Walk)]
#[walk(setting)]
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

    /// What a build asks of the solid to find the face again.
    pub fn probe(&self) -> kernel_api::ShapeProbe {
        kernel_api::ShapeProbe::Face {
            point: self.point.map(f64::from),
            normal: self.normal.map(f64::from),
            name: self.name,
        }
    }

    /// The pick moved to where a build found its face: `false` when the
    /// answer is not a face's, or the face is not flat and `curved` does
    /// not allow it.
    pub fn take_answer(
        &mut self,
        answer: &Result<kernel_api::ProbeAnswer, String>,
        curved: bool,
    ) -> bool {
        match answer {
            Ok(kernel_api::ProbeAnswer::Face {
                point,
                normal,
                surface,
            }) if curved || matches!(surface, kernel_api::FaceSurface::Plane { .. }) => {
                self.point = point.map(|c| c as f32);
                self.normal = normal.map(|c| c as f32);
                true
            }
            _ => false,
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

/// A feature placed by an attachment, as a datum is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Walk)]
pub struct Attached {
    pub attachment: core_document::DatumAttachment,
    #[serde(default)]
    #[walk(setting)]
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

/// Which edges a dress-up applies to.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Walk)]
#[walk(setting)]
pub enum EdgeSel {
    /// Every edge of the solid (immune to topology churn).
    #[default]
    All,
    /// The edges bordering the picked faces.
    Faces(Vec<FacePick>),
    /// Edges picked one by one in the viewport.
    Edges(Vec<EdgePick>),
    /// No edge: the round between two picked faces, which need share none.
    /// A fillet only.
    BetweenFaces(Vec<FacePick>),
}

/// A picked edge, by a point on it and its direction there, re-resolved
/// against the current solid each rebuild: a rebuilt solid numbers its
/// edges afresh, but the edge nearest the point is the one picked.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Walk)]
#[walk(setting)]
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
