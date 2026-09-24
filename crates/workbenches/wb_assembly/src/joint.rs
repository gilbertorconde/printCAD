//! A joint: how one body sits against another. Each end is an anchor on a
//! face, kept in its own body's frame, so a joint holds however either body
//! is later moved or rebuilt.

use core_document::{
    BodyId, BodyPlacement, DocumentResult, FaceRef, FeatureError, FeatureId, WorkbenchFeature,
    WorkbenchId,
};
use glam::{DQuat, DVec3};
use kernel_api::FaceSurface;
use serde::{Deserialize, Serialize};

/// The feature kind joints are stored as.
pub const JOINT_KIND: &str = "wb.assembly";

/// What a joint holds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum JointKind {
    /// Two flat faces against each other, `offset` millimetres apart. With
    /// `flip` they face the same way instead, as a shelf and the floor
    /// under it do.
    Mate { flip: bool, offset: f32 },
    /// Two round faces on one axis: a pin in a hole, a shaft in a bearing.
    /// The moving body may still turn about the axis and slide along it.
    Align,
    /// Two flat faces at `degrees` between their outward normals: 180
    /// faces them at each other, 90 stands one square to the other. Only
    /// the turn is held; where the faces sit is left free.
    Angle { degrees: f32 },
}

impl JointKind {
    pub fn label(&self) -> &'static str {
        match self {
            JointKind::Mate { .. } => "Mate",
            JointKind::Align => "Align",
            JointKind::Angle { .. } => "Angle",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            JointKind::Mate { .. } => "joint-mate",
            JointKind::Align => "joint-align",
            JointKind::Angle { .. } => "constraint-angle",
        }
    }
}

/// Where a joint takes hold of a body, in that body's own frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Anchor {
    /// A flat face: a point on it and its outward normal.
    Plane { point: [f32; 3], normal: [f32; 3] },
    /// A round face's axis: a point on it and its direction.
    Axis {
        point: [f32; 3],
        direction: [f32; 3],
    },
}

impl Anchor {
    /// The angle between the directions of two anchors where two bodies sit,
    /// in degrees.
    pub fn angle_to(&self, at: &Rigid, other: &Anchor, other_at: &Rigid) -> f32 {
        let (_, a) = self.placed(at);
        let (_, b) = other.placed(other_at);
        a.cross(b).length().atan2(a.dot(b)).to_degrees() as f32
    }

    /// The flat face a pick landed on, when it is flat.
    pub fn plane_of(face: &FaceRef) -> Option<Anchor> {
        match face.surface {
            Some(FaceSurface::Plane { origin, normal }) => Some(Anchor::Plane {
                point: nearest_on_plane(face.point, origin, normal),
                normal,
            }),
            // A mesh body has no recorded surfaces; its picked triangle is
            // flat, and that is what it touches with.
            None => Some(Anchor::Plane {
                point: face.point,
                normal: face.normal,
            }),
            Some(_) => None,
        }
    }

    /// The axis of the round face a pick landed on.
    pub fn axis_of(face: &FaceRef) -> Option<Anchor> {
        let (point, direction) = face.surface?.axis()?;
        Some(Anchor::Axis { point, direction })
    }

    /// The anchor seen from a frame `placement` moves points into.
    pub fn moved(&self, placement: &BodyPlacement) -> Anchor {
        match *self {
            Anchor::Plane { point, normal } => Anchor::Plane {
                point: placement.point(point),
                normal: placement.direction(normal),
            },
            Anchor::Axis { point, direction } => Anchor::Axis {
                point: placement.point(point),
                direction: placement.direction(direction),
            },
        }
    }

    /// Its point and direction where `at` puts them, in double precision.
    pub fn placed(&self, at: &Rigid) -> (DVec3, DVec3) {
        let (p, d) = self.parts();
        (at.rotation * p + at.translation, at.rotation * d)
    }

    /// Its point and direction in double precision.
    pub fn parts(&self) -> (DVec3, DVec3) {
        let (p, d) = match *self {
            Anchor::Plane { point, normal } => (point, normal),
            Anchor::Axis { point, direction } => (point, direction),
        };
        (
            DVec3::from_array(p.map(f64::from)),
            DVec3::from_array(d.map(f64::from)).normalize_or_zero(),
        )
    }
}

/// The point of the plane through `origin` nearest to `p`.
fn nearest_on_plane(p: [f32; 3], origin: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
    let (p, o, n) = (
        glam::Vec3::from_array(p),
        glam::Vec3::from_array(origin),
        glam::Vec3::from_array(normal).normalize_or_zero(),
    );
    (p - n * (p - o).dot(n)).to_array()
}

/// A placement in double precision, for solving: the solver's small steps
/// would be lost in a `BodyPlacement`'s single-precision numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rigid {
    pub rotation: DQuat,
    pub translation: DVec3,
}

impl From<BodyPlacement> for Rigid {
    fn from(p: BodyPlacement) -> Self {
        Self {
            rotation: p.quat().as_dquat(),
            translation: p.offset().as_dvec3(),
        }
    }
}

impl From<Rigid> for BodyPlacement {
    fn from(r: Rigid) -> Self {
        BodyPlacement::new(r.rotation.normalize().as_quat(), r.translation.as_vec3())
    }
}

/// A joint, stored as a feature of the body it moves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JointFeature {
    pub kind: JointKind,
    /// On the body that moves: the body the feature belongs to.
    pub moving: Anchor,
    /// The body it is held against, which it follows.
    pub other_body: BodyId,
    pub fixed: Anchor,
}

/// Scales a direction mismatch against a distance: a turn this many
/// millimetres out at arm's length weighs as much as a millimetre.
const ARM_MM: f64 = 50.0;

impl JointFeature {
    /// How far the joint is from holding with the two bodies placed so: a
    /// list of mismatches, each zero when it holds, in millimetres.
    pub fn residuals(&self, moving: &Rigid, fixed: &Rigid, out: &mut Vec<f64>) {
        let (pm, dm) = self.moving.placed(moving);
        let (pf, df) = self.fixed.placed(fixed);
        match self.kind {
            JointKind::Mate { flip, offset } => {
                // Opposed normals face each other; flipped, they agree.
                let facing = if flip { dm - df } else { dm + df };
                out.extend(facing.to_array().map(|c| c * ARM_MM));
                out.push((pm - pf).dot(df) - f64::from(offset));
            }
            JointKind::Align => {
                out.extend(dm.cross(df).to_array().map(|c| c * ARM_MM));
                out.extend((pm - pf).cross(df).to_array());
            }
            JointKind::Angle { degrees } => {
                let between = dm.cross(df).length().atan2(dm.dot(df));
                out.push((between - f64::from(degrees).to_radians()) * ARM_MM);
            }
        }
    }
}

impl WorkbenchFeature for JointFeature {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(JOINT_KIND)
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
        Vec::new()
    }

    fn name(&self) -> &str {
        self.kind.label()
    }
}
