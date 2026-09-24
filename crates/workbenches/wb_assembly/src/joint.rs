//! A joint: how one body sits against another. Each end is an anchor on a
//! face, kept in its own body's frame, so a joint holds however either body
//! is later moved or rebuilt.

use core_document::{
    BodyId, BodyPlacement, DocumentResult, EdgeRef, FaceRef, FeatureError, FeatureId,
    WorkbenchFeature, WorkbenchId,
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
    /// The body stays where it is: everything else is placed against it.
    /// It names no other body and holds no anchors.
    Ground,
    /// Two axes as one, `offset` millimetres apart along them: a hinge's
    /// pin in its knuckle. Only the turn about the axis is left, which
    /// `drive` can hold or keep within limits: its angle, in degrees, is
    /// the turn from `zero`, the body's turn in the other's frame when the
    /// joint was made.
    Hinge {
        offset: f32,
        #[serde(default = "unturned")]
        zero: [f64; 4],
        #[serde(default)]
        drive: Drive,
    },
    /// Two axes as one, the turn about them held as it was made: a
    /// drawer's runner. Only the slide along the axis is left, which
    /// `drive` can hold or keep within limits: its position, in
    /// millimetres, is how far along the axis the first sits from the
    /// second.
    Slider {
        turn: [f64; 4],
        #[serde(default)]
        drive: Drive,
    },
    /// The body held to the other exactly as it sat when the joint was
    /// made (`turn` and `shift` in the other body's frame).
    Fixed { turn: [f64; 4], shift: [f64; 3] },
    /// Two flat faces parallel, facing either way; where they sit is left.
    Parallel,
    /// Two flat faces square to each other; where they sit is left.
    Perpendicular,
    /// Two flat faces `offset` millimetres apart along the second's
    /// normal, however they are turned.
    Distance { offset: f32 },
    /// A flat face against a round one of `radius`: the round face's axis
    /// parallel to the flat face, a radius off it, on its outer side.
    Tangent { radius: f32 },
}

/// What is done with the one motion a hinge or a slider leaves: held at
/// `to`, or kept between `limits` (low, high), or left free.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Drive {
    #[serde(default)]
    pub to: Option<f32>,
    #[serde(default)]
    pub limits: Option<[f32; 2]>,
}

impl Drive {
    /// Its residual, when it holds anything, for a motion at `now`;
    /// `angular` motions are in degrees and wrap round.
    fn residual(&self, now: f64, angular: bool, out: &mut Vec<f64>) {
        let apart = |target: f32| {
            let d = now - f64::from(target);
            if angular {
                (d + 180.0).rem_euclid(360.0) - 180.0
            } else {
                d
            }
        };
        let scale = if angular {
            ARM_MM * std::f64::consts::PI / 180.0
        } else {
            1.0
        };
        if let Some(to) = self.to {
            out.push(apart(to) * scale);
        } else if let Some([low, high]) = self.limits {
            let (below, above) = (apart(low), apart(high));
            out.push(if below < 0.0 {
                below * scale
            } else if above > 0.0 {
                above * scale
            } else {
                0.0
            });
        }
    }
}

fn unturned() -> [f64; 4] {
    DQuat::IDENTITY.to_array()
}

/// A body's turn and shift in another body's frame.
pub fn relative(moving: &Rigid, fixed: &Rigid) -> (DQuat, DVec3) {
    let back = fixed.rotation.inverse();
    (
        (back * moving.rotation).normalize(),
        back * (moving.translation - fixed.translation),
    )
}

/// The turn from `from` to `to`, as a rotation vector the short way round.
fn turn_between(from: DQuat, to: DQuat) -> DVec3 {
    let d = (from.inverse() * to).normalize();
    let d = if d.w < 0.0 { -d } else { d };
    d.to_scaled_axis()
}

/// A stored turn as a quaternion.
pub fn quat(turn: [f64; 4]) -> DQuat {
    DQuat::from_array(turn).normalize()
}

impl JointKind {
    pub fn label(&self) -> &'static str {
        match self {
            JointKind::Mate { .. } => "Mate",
            JointKind::Align => "Align",
            JointKind::Angle { .. } => "Angle",
            JointKind::Ground => "Ground",
            JointKind::Hinge { .. } => "Hinge",
            JointKind::Slider { .. } => "Slider",
            JointKind::Fixed { .. } => "Fixed",
            JointKind::Parallel => "Parallel",
            JointKind::Perpendicular => "Perpendicular",
            JointKind::Distance { .. } => "Distance",
            JointKind::Tangent { .. } => "Tangent",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            JointKind::Mate { .. } => "joint-mate",
            JointKind::Align => "joint-align",
            JointKind::Angle { .. } => "constraint-angle",
            JointKind::Ground => "constraint-lock",
            JointKind::Hinge { .. } => "revolution",
            JointKind::Slider { .. } => "linear-pattern",
            JointKind::Fixed { .. } => "constraint-block",
            JointKind::Parallel => "constraint-parallel",
            JointKind::Perpendicular => "constraint-perpendicular",
            JointKind::Distance { .. } => "constraint-distance",
            JointKind::Tangent { .. } => "constraint-tangent",
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

    /// An edge as an axis: a circle's (a hole's rim), or a straight edge's
    /// own line.
    pub fn axis_of_edge(edge: &EdgeRef) -> Anchor {
        match edge.circle {
            Some(c) => Anchor::Axis {
                point: c.center,
                direction: c.normal,
            },
            None => Anchor::Axis {
                point: edge.point,
                direction: edge.direction,
            },
        }
    }

    /// The radius of the round face a pick landed on, when it has one.
    pub fn radius_of(face: &FaceRef) -> Option<f32> {
        match face.surface? {
            FaceSurface::Cylinder { radius, .. } => Some(radius),
            _ => None,
        }
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
    /// Where a hinge or a slider has got to: the hinge's angle in degrees
    /// (-180 to 180) or the slider's position in millimetres.
    pub fn travel(&self, moving: &Rigid, fixed: &Rigid) -> Option<f64> {
        match self.kind {
            JointKind::Hinge { zero, .. } => {
                // The turn since the joint was made, in the other body's
                // frame, and how much of it is about the axis there.
                let (now, _) = relative(moving, fixed);
                let mut d = (now * quat(zero).inverse()).normalize();
                if d.w < 0.0 {
                    d = -d;
                }
                let (_, axis) = self.fixed.parts();
                let along = DVec3::new(d.x, d.y, d.z).dot(axis.normalize_or_zero());
                Some((2.0 * along.atan2(d.w)).to_degrees())
            }
            JointKind::Slider { .. } => {
                let (pm, _) = self.moving.placed(moving);
                let (pf, df) = self.fixed.placed(fixed);
                Some((pm - pf).dot(df))
            }
            _ => None,
        }
    }

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
            // Grounding fixes the body rather than asking anything of it.
            JointKind::Ground => {}
            JointKind::Hinge { offset, drive, .. } => {
                out.extend(dm.cross(df).to_array().map(|c| c * ARM_MM));
                out.extend((pm - pf).cross(df).to_array());
                out.push((pm - pf).dot(df) - f64::from(offset));
                if let Some(angle) = self.travel(moving, fixed) {
                    drive.residual(angle, true, out);
                }
            }
            JointKind::Slider { turn, drive } => {
                drive.residual((pm - pf).dot(df), false, out);
                out.extend((pm - pf).cross(df).to_array());
                let (now, _) = relative(moving, fixed);
                out.extend(turn_between(quat(turn), now).to_array().map(|c| c * ARM_MM));
            }
            JointKind::Fixed { turn, shift } => {
                let (now, at) = relative(moving, fixed);
                out.extend(turn_between(quat(turn), now).to_array().map(|c| c * ARM_MM));
                out.extend((at - DVec3::from_array(shift)).to_array());
            }
            JointKind::Parallel => {
                out.extend(dm.cross(df).to_array().map(|c| c * ARM_MM));
            }
            JointKind::Perpendicular => out.push(dm.dot(df) * ARM_MM),
            JointKind::Distance { offset } => {
                out.push((pm - pf).dot(df) - f64::from(offset));
            }
            JointKind::Tangent { radius } => {
                // Whichever end is the flat face.
                let (plane_point, normal, axis_point, axis) = match self.moving {
                    Anchor::Plane { .. } => (pm, dm, pf, df),
                    Anchor::Axis { .. } => (pf, df, pm, dm),
                };
                out.push(axis.dot(normal) * ARM_MM);
                out.push((axis_point - plane_point).dot(normal) - f64::from(radius));
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

/// What a joint takes on each of its two bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Takes {
    Flat,
    /// A round face, or an edge: a circle's axis or a straight edge's line.
    Round,
    /// Anything: the joint holds the bodies, not the faces.
    Any,
    /// A flat face on one and a round one on the other, either way round.
    FlatAndRound,
}

/// A tool that makes a joint from a face on each of two bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JointTool {
    Mate,
    Align,
    Angle,
    Hinge,
    Slider,
    Fixed,
    Parallel,
    Perpendicular,
    Distance,
    Tangent,
}

impl JointTool {
    pub const ALL: [JointTool; 10] = [
        JointTool::Mate,
        JointTool::Align,
        JointTool::Angle,
        JointTool::Hinge,
        JointTool::Slider,
        JointTool::Fixed,
        JointTool::Parallel,
        JointTool::Perpendicular,
        JointTool::Distance,
        JointTool::Tangent,
    ];

    /// Its tool and command id.
    pub fn command(self) -> &'static str {
        match self {
            JointTool::Mate => "asm.mate",
            JointTool::Align => "asm.align",
            JointTool::Angle => "asm.angle",
            JointTool::Hinge => "asm.hinge",
            JointTool::Slider => "asm.slider",
            JointTool::Fixed => "asm.fix",
            JointTool::Parallel => "asm.parallel",
            JointTool::Perpendicular => "asm.perpendicular",
            JointTool::Distance => "asm.distance",
            JointTool::Tangent => "asm.tangent",
        }
    }

    pub fn of_command(id: &str) -> Option<JointTool> {
        Self::ALL.into_iter().find(|t| t.command() == id)
    }

    /// The tool that makes a joint of this kind; `None` for a ground.
    pub fn of_kind(kind: &JointKind) -> Option<JointTool> {
        Some(match kind {
            JointKind::Mate { .. } => JointTool::Mate,
            JointKind::Align => JointTool::Align,
            JointKind::Angle { .. } => JointTool::Angle,
            JointKind::Ground => return None,
            JointKind::Hinge { .. } => JointTool::Hinge,
            JointKind::Slider { .. } => JointTool::Slider,
            JointKind::Fixed { .. } => JointTool::Fixed,
            JointKind::Parallel => JointTool::Parallel,
            JointKind::Perpendicular => JointTool::Perpendicular,
            JointKind::Distance { .. } => JointTool::Distance,
            JointKind::Tangent { .. } => JointTool::Tangent,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            JointTool::Mate => "Mate faces",
            JointTool::Align => "Align axes",
            JointTool::Angle => "Angle between faces",
            JointTool::Hinge => "Hinge",
            JointTool::Slider => "Slider",
            JointTool::Fixed => "Fix together",
            JointTool::Parallel => "Parallel faces",
            JointTool::Perpendicular => "Perpendicular faces",
            JointTool::Distance => "Distance between faces",
            JointTool::Tangent => "Tangent faces",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            JointTool::Mate => "joint-mate",
            JointTool::Align => "joint-align",
            JointTool::Angle => "constraint-angle",
            JointTool::Hinge => "revolution",
            JointTool::Slider => "linear-pattern",
            JointTool::Fixed => "constraint-block",
            JointTool::Parallel => "constraint-parallel",
            JointTool::Perpendicular => "constraint-perpendicular",
            JointTool::Distance => "constraint-distance",
            JointTool::Tangent => "constraint-tangent",
        }
    }

    pub fn shortcut(self) -> &'static str {
        match self {
            JointTool::Mate => "M",
            JointTool::Align => "A",
            JointTool::Angle => "N",
            JointTool::Hinge => "H",
            JointTool::Slider => "L",
            JointTool::Fixed => "X",
            JointTool::Parallel => "R",
            JointTool::Perpendicular => "Shift+R",
            JointTool::Distance => "D",
            JointTool::Tangent => "T",
        }
    }

    /// What the joint does, in a sentence.
    pub fn summary(self) -> &'static str {
        match self {
            JointTool::Mate => "Put two flat faces against each other",
            JointTool::Align => "Put two round faces on one axis",
            JointTool::Angle => "Hold two flat faces at an angle",
            JointTool::Hinge => "Put two axes on one line: the body can only turn about it",
            JointTool::Slider => {
                "Put two axes on one line without turning: the body can only slide along it"
            }
            JointTool::Fixed => "Hold a body to another where it sits",
            JointTool::Parallel => "Keep two flat faces parallel",
            JointTool::Perpendicular => "Keep two flat faces square to each other",
            JointTool::Distance => "Keep two flat faces a distance apart",
            JointTool::Tangent => "Rest a round face on a flat one",
        }
    }

    pub fn takes(self) -> Takes {
        match self {
            JointTool::Mate
            | JointTool::Angle
            | JointTool::Parallel
            | JointTool::Perpendicular
            | JointTool::Distance => Takes::Flat,
            JointTool::Align | JointTool::Hinge | JointTool::Slider => Takes::Round,
            JointTool::Fixed => Takes::Any,
            JointTool::Tangent => Takes::FlatAndRound,
        }
    }

    /// What to click next.
    pub fn prompt(self, first_done: bool) -> &'static str {
        match (self.takes(), first_done) {
            (Takes::Flat, false) => "Click a flat face on the body to move",
            (Takes::Flat, true) => "Click the flat face it keeps to, on another body",
            (Takes::Round, false) => "Click a round face or an edge on the body to move",
            (Takes::Round, true) => {
                "Click the round face or edge it lines up with, on another body"
            }
            (Takes::Any, false) => "Click the body to move",
            (Takes::Any, true) => "Click the body it is held to",
            (Takes::FlatAndRound, false) => "Click a flat or a round face on the body to move",
            (Takes::FlatAndRound, true) => "Click the face it rests against, on another body",
        }
    }

    /// Why a pick was turned down.
    pub fn refusal(self) -> &'static str {
        match self.takes() {
            Takes::Flat => "This joint takes flat faces",
            Takes::Round => "This joint takes round faces or edges: a hole, a pin, a rim",
            Takes::Any => "Click a face of the body",
            Takes::FlatAndRound => {
                "This joint takes a flat face on one body and a round one on the other"
            }
        }
    }

    /// The joint made from two anchors where the bodies sit now: settings
    /// the tool does not ask for start at what the bodies make now, so
    /// making the joint moves nothing it need not. `radius` is the round
    /// face's, for a tangent.
    pub fn joint(
        self,
        moving: &Anchor,
        at: &Rigid,
        fixed: &Anchor,
        fixed_at: &Rigid,
        radius: f32,
    ) -> JointKind {
        let (turn, shift) = relative(at, fixed_at);
        match self {
            JointTool::Mate => JointKind::Mate {
                flip: false,
                offset: 0.0,
            },
            JointTool::Align => JointKind::Align,
            JointTool::Angle => JointKind::Angle {
                degrees: moving.angle_to(at, fixed, fixed_at),
            },
            JointTool::Hinge => JointKind::Hinge {
                offset: 0.0,
                zero: turn.to_array(),
                drive: Drive::default(),
            },
            JointTool::Slider => JointKind::Slider {
                turn: turn.to_array(),
                drive: Drive::default(),
            },
            JointTool::Fixed => JointKind::Fixed {
                turn: turn.to_array(),
                shift: shift.to_array(),
            },
            JointTool::Parallel => JointKind::Parallel,
            JointTool::Perpendicular => JointKind::Perpendicular,
            JointTool::Distance => {
                let (pm, _) = moving.placed(at);
                let (pf, df) = fixed.placed(fixed_at);
                JointKind::Distance {
                    offset: (pm - pf).dot(df) as f32,
                }
            }
            JointTool::Tangent => JointKind::Tangent { radius },
        }
    }
}
