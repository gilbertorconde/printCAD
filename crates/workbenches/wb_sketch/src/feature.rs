//! Sketch feature implementation for the document feature tree.

use core_document::{DocumentResult, FeatureError, FeatureId, WorkbenchFeature, WorkbenchId};
use serde::{Deserialize, Serialize};

use crate::boundary::widen3;
use crate::sketch::{Sketch, SketchPlane};

/// A sketch feature that can be stored in the document's feature tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SketchFeature {
    pub sketch: Sketch,
    /// The plane the sketch lies on, in its body's own frame.
    pub plane: SketchPlane,
    /// The datum the sketch was drawn on, which its plane follows: moved,
    /// turned or flipped, the datum takes the sketch with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub support: Option<DatumSupport>,
    /// The face of the body's solid the sketch was placed on, which its
    /// plane follows: the face moved or turned by a change before it in
    /// the history, the sketch moves with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<FaceSupport>,
    /// Attached as a datum is, by a mode on what was picked (three points,
    /// square to an edge, tangent to a face, a circle's centre, ...): the
    /// sketch's plane is the attachment's, and follows what it stands on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attached: Option<AttachedSupport>,
    /// The numbers the sketch's curves are made from, for a generated
    /// sketch (a gear, a sprocket, a shaft); its curves follow them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<crate::generator::Generator>,
    /// While the sketch is edited, everything on the viewer's side of its
    /// plane is cut away.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub section_view: bool,
    /// The section view was switched off for this sketch, which keeps it
    /// off when editing would cut the view by itself.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub section_off: bool,
}

/// A sketch attached by a mode, as a datum plane would be.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AttachedSupport {
    pub attachment: core_document::DatumAttachment,
    #[serde(default)]
    pub offset: core_document::AttachmentOffset,
}

impl AttachedSupport {
    /// The datum plane the attachment would make.
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

    /// The sketch's plane where the attachment puts it.
    pub fn plane(&self) -> SketchPlane {
        let frame = self.datum().frame();
        SketchPlane {
            origin: widen3(frame.origin),
            normal: widen3(frame.normal),
            x_axis: widen3(frame.x_axis),
            y_axis: widen3(frame.y_axis()),
        }
    }

    /// The attachment with what the solid answered its probes taken in.
    pub fn answered(&self, answers: &[Result<kernel_api::ProbeAnswer, String>]) -> Self {
        let mut datum = self.datum();
        datum.take_answers(answers);
        Self {
            attachment: datum.attachment,
            offset: datum.offset,
        }
    }
}

/// A sketch's place on a datum: a datum plane, or one of a coordinate
/// system's three planes, pushed `offset` along its normal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatumSupport {
    pub datum: FeatureId,
    /// A coordinate system's plane: `XY`, `XZ` or `YZ`. A datum plane has
    /// only its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plane: Option<String>,
    /// Millimetres along the plane's normal.
    #[serde(default)]
    pub offset: f64,
    /// Millimetres across the plane, along its x and y.
    #[serde(default, skip_serializing_if = "is_zero2")]
    pub shift: [f64; 2],
    /// Degrees the sketch is turned about the plane's normal.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub turn: f64,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

fn is_zero2(v: &[f64; 2]) -> bool {
    *v == [0.0, 0.0]
}

/// A sketch's place on a face of its body's solid: the face as it was
/// picked (a point on it, its outward normal there and its name, in the
/// body's frame) and the plane the sketch had on it then.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FaceSupport {
    pub point: [f64; 3],
    pub normal: [f64; 3],
    #[serde(default)]
    pub name: kernel_api::TopoName,
    /// The sketch's plane when it was placed.
    pub placed: SketchPlane,
    /// The borrow that lends the face, and which of its faces it is: the
    /// face is on another body's solid, and found where the borrow finds
    /// it rather than on the sketch's own body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lent_by: Option<LentFace>,
}

/// A face a borrow lends: the borrow, and which of its faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LentFace {
    pub borrow: FeatureId,
    pub index: usize,
}

impl FaceSupport {
    /// The sketch placed with `placed` on `face`.
    pub fn on(face: &core_document::FaceRef, placed: SketchPlane) -> Self {
        Self {
            point: widen3(face.point),
            normal: widen3(face.normal),
            name: face.name,
            placed,
            lent_by: None,
        }
    }

    /// The support a sketch placed with `placed` on `face` gets, by where
    /// the face comes from: none on a face no rebuild finds again.
    pub fn from_origin(
        face: &core_document::FaceRef,
        origin: core_document::FaceOrigin,
        placed: SketchPlane,
    ) -> Option<Self> {
        match origin {
            core_document::FaceOrigin::Elsewhere => None,
            core_document::FaceOrigin::OwnSolid => Some(Self::on(face, placed)),
            core_document::FaceOrigin::Lent { borrow, index } => Some(Self {
                lent_by: Some(LentFace { borrow, index }),
                ..Self::on(face, placed)
            }),
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

    /// The sketch's plane on the face as the solid now has it (`point` on
    /// it, `normal` out of it): turned as the face turned, and carried
    /// along the face's normal onto it, never sliding across it.
    pub fn plane_at(&self, point: [f64; 3], normal: [f64; 3]) -> SketchPlane {
        use glam::DVec3;
        let n0 = DVec3::from_array(self.normal).normalize_or_zero();
        let n1 = DVec3::from_array(normal).normalize_or_zero();
        let p0 = DVec3::from_array(self.point);
        let p1 = DVec3::from_array(point);
        let turn = glam::DQuat::from_rotation_arc(n0, n1);
        let turned = |v: [f64; 3]| (turn * DVec3::from_array(v)).to_array();
        let origin = p0 + turn * (DVec3::from_array(self.placed.origin) - p0);
        let origin = origin + n1 * (p1 - origin).dot(n1);
        SketchPlane {
            origin: origin.to_array(),
            normal: turned(self.placed.normal),
            x_axis: turned(self.placed.x_axis),
            y_axis: turned(self.placed.y_axis),
        }
    }
}

impl DatumSupport {
    /// The plane this support puts a sketch on, from the datum's `data`;
    /// `None` when it is not a datum plane or coordinate system.
    pub fn plane_from(&self, data: &serde_json::Value) -> Option<SketchPlane> {
        use core_document::{DatumFeature, DatumShape};
        let datum = DatumFeature::from_json(data).ok()?;
        let frame = match datum.shape {
            DatumShape::Plane { .. } => datum.frame(),
            DatumShape::CoordinateSystem { .. } => {
                let which = self.plane.as_deref().unwrap_or("XY");
                datum
                    .frame()
                    .planes()
                    .into_iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(which))?
                    .1
            }
            _ => return None,
        };
        let mut plane = SketchPlane {
            origin: widen3(frame.origin),
            normal: widen3(frame.normal),
            x_axis: widen3(frame.x_axis),
            y_axis: widen3(frame.y_axis()),
        };
        for (o, n) in plane.origin.iter_mut().zip(plane.normal) {
            *o += n * self.offset;
        }
        Some(self.moved_within(plane))
    }

    /// `plane` shifted across itself and turned about its normal as the
    /// support says.
    pub fn moved_within(&self, mut plane: SketchPlane) -> SketchPlane {
        use glam::DVec3;
        let (x, y) = (
            DVec3::from_array(plane.x_axis),
            DVec3::from_array(plane.y_axis),
        );
        let origin = DVec3::from_array(plane.origin) + x * self.shift[0] + y * self.shift[1];
        let (s, c) = self.turn.to_radians().sin_cos();
        plane.origin = origin.to_array();
        plane.x_axis = (x * c + y * s).to_array();
        plane.y_axis = (y * c - x * s).to_array();
        plane
    }
}

impl SketchFeature {
    /// What the sketch asks of its own body's solid, `body`, where it
    /// stands in the body's history: each edge projected from that solid
    /// and each section of one of its faces, onto the sketch's plane. The
    /// build answers them, so the projections follow the solid as it was
    /// before the features the sketch builds, never chasing their change.
    pub fn own_projections(&self, body: core_document::BodyId) -> Vec<kernel_api::ProjectionProbe> {
        crate::external::groups(&self.sketch)
            .into_iter()
            .filter(|(source, _)| source.reference.is_none() && source.body == body.0)
            .map(|(source, _)| projection_probe(&self.plane, &source))
            .collect()
    }

    /// Whether editing the sketch cuts the view at its plane: its own
    /// choice when it made one, else `on_open`, the Sketcher's setting.
    pub fn shows_section(&self, on_open: bool) -> bool {
        self.section_view || (on_open && !self.section_off)
    }

    /// The sketch's own choice of section view, which holds whatever the
    /// setting says.
    pub fn set_section(&mut self, on: bool) {
        self.section_view = on;
        self.section_off = !on;
    }

    pub fn new(sketch: Sketch, plane: SketchPlane) -> Self {
        Self {
            sketch,
            plane,
            support: None,
            face: None,
            attached: None,
            generator: None,
            section_view: false,
            section_off: false,
        }
    }

    pub fn from_sketch(sketch: Sketch) -> Self {
        Self {
            sketch,
            plane: SketchPlane::default(),
            support: None,
            face: None,
            attached: None,
            generator: None,
            section_view: false,
            section_off: false,
        }
    }
}

impl WorkbenchFeature for SketchFeature {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from("wb.sketch")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("SketchFeature should always serialize")
    }

    fn from_json(value: &serde_json::Value) -> DocumentResult<Self> {
        serde_json::from_value(value.clone()).map_err(|e| {
            core_document::DocumentError::Feature(FeatureError::Deserialization(e.to_string()))
        })
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        let lent = self.face.and_then(|f| f.lent_by).map(|l| l.borrow);
        self.support.iter().map(|s| s.datum).chain(lent).collect()
    }

    fn name(&self) -> &str {
        &self.sketch.name
    }
}

/// `source` projected onto the sketch plane `plane`, both in the frame of
/// the body the sketch and the source share.
pub(crate) fn projection_probe(
    plane: &SketchPlane,
    source: &crate::sketch::ExternalSource,
) -> kernel_api::ProjectionProbe {
    kernel_api::ProjectionProbe {
        point: source.point,
        faces: source.faces,
        section: source.section,
        plane: kernel_api::ProfilePlane {
            origin: plane.origin,
            x_axis: plane.x_axis,
            y_axis: plane.y_axis,
            normal: plane.normal,
        },
    }
}

#[cfg(test)]
mod face_support {
    use super::*;

    #[test]
    fn a_sketch_on_a_face_moves_along_it_and_turns_with_it() {
        let top = core_document::FaceRef {
            point: [3.0, 4.0, 10.0],
            normal: [0.0, 0.0, 1.0],
            surface: None,
            name: 7,
        };
        let placed = SketchPlane::from_face(widen3(top.point), widen3(top.normal));
        let support = FaceSupport::on(&top, placed);
        // The face rises 5 mm, found at another spot of it: the sketch
        // rises with it and does not slide.
        let risen = support.plane_at([8.0, 1.0, 15.0], [0.0, 0.0, 1.0]);
        assert_eq!(risen.origin, [0.0, 0.0, 15.0]);
        assert_eq!(risen.x_axis, placed.x_axis);
        // The face turns to face +X through the same point: so does the
        // sketch, its normal with the face's.
        let turned = support.plane_at([3.0, 4.0, 10.0], [1.0, 0.0, 0.0]);
        assert!((glam::DVec3::from_array(turned.normal) - glam::DVec3::X).length() < 1e-6);
        let origin = glam::DVec3::from_array(turned.origin);
        assert!(
            (origin - glam::DVec3::new(3.0, 4.0, 10.0))
                .dot(glam::DVec3::X)
                .abs()
                < 1e-5
        );
    }
}
