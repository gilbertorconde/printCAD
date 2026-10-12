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

/// The other end of a joint held to the world: the origin, its planes and
/// axes, which never move. No body has this id.
pub const WORLD: BodyId = BodyId(uuid::Uuid::nil());

/// The world's planes and axes, as anchors, by name.
pub const ORIGIN: [(&str, Anchor); 6] = [
    (
        "XY plane",
        Anchor::Plane {
            point: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
        },
    ),
    (
        "XZ plane",
        Anchor::Plane {
            point: [0.0; 3],
            normal: [0.0, -1.0, 0.0],
        },
    ),
    (
        "YZ plane",
        Anchor::Plane {
            point: [0.0; 3],
            normal: [1.0, 0.0, 0.0],
        },
    ),
    (
        "X axis",
        Anchor::Axis {
            point: [0.0; 3],
            direction: [1.0, 0.0, 0.0],
        },
    ),
    (
        "Y axis",
        Anchor::Axis {
            point: [0.0; 3],
            direction: [0.0, 1.0, 0.0],
        },
    ),
    (
        "Z axis",
        Anchor::Axis {
            point: [0.0; 3],
            direction: [0.0, 0.0, 1.0],
        },
    ),
];

/// What a joint holds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum JointKind {
    /// Two flat faces against each other, `offset` millimetres apart. With
    /// `flip` they face the same way instead, as a shelf and the floor
    /// under it do.
    Mate { flip: bool, offset: f32 },
    /// Two round faces on one axis: a pin in a hole, a shaft in a bearing.
    /// The moving body may still turn about the axis and slide along it;
    /// `turn` can hold or limit the turn (degrees from `zero`, the body's
    /// turn in the other's frame when the joint was made) and `slide` the
    /// slide (mm along the axis from the other's anchor).
    Align {
        #[serde(default = "unturned")]
        zero: [f64; 4],
        #[serde(default)]
        turn: Drive,
        #[serde(default)]
        slide: Drive,
    },
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
    /// Two points as one: a ball in its socket. The body may turn every
    /// way about the point.
    Ball,
    /// A cross between two yokes: the two pins' axes meet at one point and
    /// stay square to each other. The body may turn about either pin.
    Universal,
    /// A pin in a slot: the moving point stays on the fixed line, free to
    /// slide along it and turn every way.
    Slot,
    /// A point following an edge of the other body, any shape of edge:
    /// free to run along it and turn every way.
    Path,
    /// A follower on a cam: the moving point stays `radius` off the other
    /// body's face (a roller's radius; 0 for a point), on its outer side.
    Cam { radius: f32 },
    /// A tab centred in a slot: the moving body's two faces (`moving` and
    /// the first of `second`) centred between the other's two, all four
    /// parallel. The tab slides along the slot and turns in it.
    Width,
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

/// A joint's kind as stored. A bare `"Align"` reads as an alignment with
/// nothing held, so documents that store the alignment as a plain word
/// keep loading.
fn kind_or_word<'de, D: serde::Deserializer<'de>>(de: D) -> Result<JointKind, D::Error> {
    let value = serde_json::Value::deserialize(de)?;
    if value.as_str() == Some("Align") {
        return Ok(JointKind::Align {
            zero: unturned(),
            turn: Drive::default(),
            slide: Drive::default(),
        });
    }
    serde_json::from_value(value).map_err(serde::de::Error::custom)
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
    /// An alignment made where the bodies stand unturned, neither motion
    /// held.
    pub fn align() -> Self {
        JointKind::Align {
            zero: unturned(),
            turn: Drive::default(),
            slide: Drive::default(),
        }
    }

    /// Whether the joint keeps a turn of its own that a quarter turn can
    /// move: a hinge's or an alignment's zero, a slider's or a fixed
    /// joint's held turn.
    pub fn keeps_a_turn(&self) -> bool {
        matches!(
            self,
            JointKind::Hinge { .. }
                | JointKind::Align { .. }
                | JointKind::Slider { .. }
                | JointKind::Fixed { .. }
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            JointKind::Mate { .. } => "Mate",
            JointKind::Align { .. } => "Align",
            JointKind::Angle { .. } => "Angle",
            JointKind::Ground => "Ground",
            JointKind::Hinge { .. } => "Hinge",
            JointKind::Slider { .. } => "Slider",
            JointKind::Fixed { .. } => "Fixed",
            JointKind::Parallel => "Parallel",
            JointKind::Perpendicular => "Perpendicular",
            JointKind::Distance { .. } => "Distance",
            JointKind::Tangent { .. } => "Tangent",
            JointKind::Ball => "Ball",
            JointKind::Universal => "Universal",
            JointKind::Slot => "Slot",
            JointKind::Path => "Path",
            JointKind::Cam { .. } => "Cam",
            JointKind::Width => "Width",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            JointKind::Mate { .. } => "joint-mate",
            JointKind::Align { .. } => "joint-align",
            JointKind::Angle { .. } => "constraint-angle",
            JointKind::Ground => "constraint-lock",
            JointKind::Hinge { .. } => "revolution",
            JointKind::Slider { .. } => "linear-pattern",
            JointKind::Fixed { .. } => "constraint-block",
            JointKind::Parallel => "constraint-parallel",
            JointKind::Perpendicular => "constraint-perpendicular",
            JointKind::Distance { .. } => "constraint-distance",
            JointKind::Tangent { .. } => "constraint-tangent",
            JointKind::Ball => "point",
            JointKind::Universal => "constraint-perpendicular",
            JointKind::Slot => "constraint-point-on-object",
            JointKind::Path => "additive-pipe",
            JointKind::Cam { .. } => "constraint-tangent",
            JointKind::Width => "constraint-symmetric",
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
    /// A point: a sphere's centre, a circle's centre, a point picked on a
    /// face. It has no direction.
    Point { point: [f32; 3] },
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

    /// The point a pick names: a sphere's centre, else where on the face
    /// it landed.
    pub fn point_of(face: &FaceRef) -> Anchor {
        match face.surface {
            Some(FaceSurface::Sphere { center, .. }) => Anchor::Point { point: center },
            _ => Anchor::Point { point: face.point },
        }
    }

    /// The point an edge pick names: a circle's centre, else where on the
    /// edge it landed.
    pub fn point_of_edge(edge: &EdgeRef) -> Anchor {
        Anchor::Point {
            point: edge.circle.map_or(edge.point, |c| c.center),
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
            Anchor::Point { point } => Anchor::Point {
                point: placement.point(point),
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
            Anchor::Point { point } => (point, [0.0; 3]),
        };
        (
            DVec3::from_array(p.map(f64::from)),
            DVec3::from_array(d.map(f64::from)).normalize_or_zero(),
        )
    }
}

/// `p` moved `by` along the unit of `direction`.
fn shifted(p: [f32; 3], direction: [f32; 3], by: f32) -> [f32; 3] {
    let d = glam::Vec3::from_array(direction).normalize_or_zero();
    (glam::Vec3::from_array(p) + d * by).to_array()
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

impl Rigid {
    /// This placement after `step`.
    pub fn then(&self, step: &Rigid) -> Rigid {
        Rigid {
            rotation: (step.rotation * self.rotation).normalize(),
            translation: step.rotation * self.translation + step.translation,
        }
    }
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
    #[serde(deserialize_with = "kind_or_word")]
    pub kind: JointKind,
    /// On the body that moves: the body the feature belongs to.
    pub moving: Anchor,
    /// The body it is held against, which it follows.
    pub other_body: BodyId,
    pub fixed: Anchor,
    /// The names of the faces the two ends were picked on (`moving`,
    /// `fixed`), which a rebuilt body is searched for; 0 for an end with
    /// none.
    #[serde(default, skip_serializing_if = "unnamed")]
    pub names: [kernel_api::TopoName; 2],
    /// How far each end (`moving`, `fixed`) is moved along its own
    /// direction, a flat face's normal or an axis, before the joint holds
    /// it, in millimetres.
    #[serde(default)]
    pub ends: [f32; 2],
    /// What a path or a cam runs on, in the other body's frame: the edge
    /// as a polyline, or the face as triangles, three corners each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shape: Vec<[f32; 3]>,
    /// A width's second face on each body (`moving`, `fixed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub second: Option<[Anchor; 2]>,
}

fn unnamed(names: &[kernel_api::TopoName; 2]) -> bool {
    names == &[0, 0]
}

/// Scales a direction mismatch against a distance: a turn this many
/// millimetres out at arm's length weighs as much as a millimetre.
const ARM_MM: f64 = 50.0;

impl JointFeature {
    /// The joint as the solver holds it: each end moved by its offset.
    pub fn with_ends_moved(&self) -> JointFeature {
        let moved = |anchor: Anchor, by: f32| match anchor {
            Anchor::Plane { point, normal } => Anchor::Plane {
                point: shifted(point, normal, by),
                normal,
            },
            Anchor::Axis { point, direction } => Anchor::Axis {
                point: shifted(point, direction, by),
                direction,
            },
            Anchor::Point { .. } => anchor,
        };
        JointFeature {
            moving: moved(self.moving, self.ends[0]),
            fixed: moved(self.fixed, self.ends[1]),
            ends: [0.0; 2],
            ..self.clone()
        }
    }

    /// Where a hinge or a slider has got to: the hinge's angle in degrees
    /// (-180 to 180) or the slider's position in millimetres.
    pub fn travel(&self, moving: &Rigid, fixed: &Rigid) -> Option<f64> {
        match self.kind {
            JointKind::Hinge { zero, .. } => Some(self.turned(zero, moving, fixed)),
            JointKind::Slider { .. } => {
                let (pm, _) = self.moving.placed(moving);
                let (pf, df) = self.fixed.placed(fixed);
                Some((pm - pf).dot(df))
            }
            _ => None,
        }
    }

    /// The turn about the other end's axis since `zero`, degrees from
    /// -180 to 180.
    pub fn turned(&self, zero: [f64; 4], moving: &Rigid, fixed: &Rigid) -> f64 {
        // The turn since the joint was made, in the other body's frame,
        // and how much of it is about the axis there.
        let (now, _) = relative(moving, fixed);
        let mut d = (now * quat(zero).inverse()).normalize();
        if d.w < 0.0 {
            d = -d;
        }
        let (_, axis) = self.fixed.parts();
        let along = DVec3::new(d.x, d.y, d.z).dot(axis.normalize_or_zero());
        (2.0 * along.atan2(d.w)).to_degrees()
    }

    /// The moving body moved from `before` to `after`, the other body at
    /// `fixed`: what the joint holds follows it (a held turn, a fixed
    /// shift, an angle, a distance, the side a mate's faces are on), so
    /// the joint holds the body at `after`.
    pub fn carried(&mut self, before: &Rigid, after: &Rigid, fixed: &Rigid) {
        let (was, _) = relative(before, fixed);
        let (now, shift) = relative(after, fixed);
        let step = (now * was.inverse()).normalize();
        let turn = |q: &mut [f64; 4]| *q = (step * quat(*q)).normalize().to_array();
        let (_, dm) = self.moving.placed(after);
        let (_, df) = self.fixed.placed(fixed);
        let apart = self.apart(after, fixed);
        match &mut self.kind {
            JointKind::Hinge { zero, .. } | JointKind::Align { zero, .. } => turn(zero),
            JointKind::Slider { turn: held, .. } => turn(held),
            JointKind::Fixed {
                turn: held,
                shift: s,
            } => {
                turn(held);
                *s = shift.to_array();
            }
            JointKind::Angle { degrees } => {
                *degrees = dm.cross(df).length().atan2(dm.dot(df)).to_degrees() as f32;
            }
            JointKind::Distance { offset } => *offset = apart as f32,
            JointKind::Mate { flip, .. } => *flip = dm.dot(df) > 0.0,
            JointKind::Ground
            | JointKind::Parallel
            | JointKind::Perpendicular
            | JointKind::Tangent { .. }
            | JointKind::Ball
            | JointKind::Universal
            | JointKind::Slot
            | JointKind::Path
            | JointKind::Cam { .. }
            | JointKind::Width => {}
        }
    }

    /// How far apart the two ends are, by what they are: along a flat
    /// face's normal from it (either end's, the fixed one's first), from a
    /// point to an axis, between two parallel axes (a centre distance), or
    /// between two points.
    pub fn apart(&self, moving: &Rigid, fixed: &Rigid) -> f64 {
        let (pm, dm) = self.moving.placed(moving);
        let (pf, df) = self.fixed.placed(fixed);
        // Where the two coincide the length has no slope to follow: a
        // hair's breadth off, along a line square to the axis, it has.
        let length = |v: DVec3, across: DVec3| {
            if v.length() < 1e-6 {
                (v + across * 1e-6).length()
            } else {
                v.length()
            }
        };
        match (self.moving, self.fixed) {
            (_, Anchor::Plane { .. }) => (pm - pf).dot(df),
            (Anchor::Plane { .. }, _) => (pf - pm).dot(dm),
            (_, Anchor::Axis { .. }) => {
                let off = pm - pf;
                length(off - df * off.dot(df), df.any_orthonormal_vector())
            }
            (Anchor::Axis { .. }, _) => {
                let off = pf - pm;
                length(off - dm * off.dot(dm), dm.any_orthonormal_vector())
            }
            (Anchor::Point { .. }, Anchor::Point { .. }) => length(pm - pf, DVec3::X),
        }
    }

    /// The step that turns the moving body by `degrees` about the other
    /// end's direction, through where the moving end takes hold; with
    /// `over`, half a turn about a line square to it instead.
    pub fn turning_step(&self, moving: &Rigid, fixed: &Rigid, degrees: f64, over: bool) -> Rigid {
        let (pm, _) = self.moving.placed(moving);
        let (_, df) = self.fixed.placed(fixed);
        let rotation = if over {
            DQuat::from_axis_angle(df.any_orthonormal_vector(), std::f64::consts::PI)
        } else {
            DQuat::from_axis_angle(df, degrees.to_radians())
        };
        Rigid {
            rotation,
            translation: pm - rotation * pm,
        }
    }

    /// Where an alignment has got to: its turn in degrees and its slide in
    /// millimetres.
    pub fn align_travel(&self, moving: &Rigid, fixed: &Rigid) -> Option<(f64, f64)> {
        let JointKind::Align { zero, .. } = self.kind else {
            return None;
        };
        let (pm, _) = self.moving.placed(moving);
        let (pf, df) = self.fixed.placed(fixed);
        Some((self.turned(zero, moving, fixed), (pm - pf).dot(df)))
    }

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
            JointKind::Align { zero, turn, slide } => {
                out.extend(dm.cross(df).to_array().map(|c| c * ARM_MM));
                out.extend((pm - pf).cross(df).to_array());
                slide.residual((pm - pf).dot(df), false, out);
                if turn.to.is_some() || turn.limits.is_some() {
                    turn.residual(self.turned(zero, moving, fixed), true, out);
                }
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
                out.push(self.apart(moving, fixed) - f64::from(offset));
            }
            JointKind::Ball => out.extend((pm - pf).to_array()),
            JointKind::Universal => {
                out.extend((pm - pf).to_array());
                out.push(dm.dot(df) * ARM_MM);
            }
            JointKind::Slot => {
                let off = pm - pf;
                out.extend((off - df * off.dot(df)).to_array());
            }
            JointKind::Path => {
                let line: Vec<DVec3> = self
                    .shape
                    .iter()
                    .map(|p| {
                        fixed.rotation * DVec3::from_array(p.map(f64::from)) + fixed.translation
                    })
                    .collect();
                match crate::shapes::nearest_on_polyline(pm, &line) {
                    Some(q) => out.extend((pm - q).to_array()),
                    None => out.extend([0.0; 3]),
                }
            }
            JointKind::Cam { radius } => {
                let corners: Vec<DVec3> = self
                    .shape
                    .iter()
                    .map(|p| {
                        fixed.rotation * DVec3::from_array(p.map(f64::from)) + fixed.translation
                    })
                    .collect();
                match crate::shapes::nearest_on_triangles(pm, &corners) {
                    Some((q, n)) => out.push((pm - q).dot(n) - f64::from(radius)),
                    None => out.push(0.0),
                }
            }
            JointKind::Width => {
                out.extend(dm.cross(df).to_array().map(|c| c * ARM_MM));
                if let Some([m2, f2]) = self.second {
                    let (qm, _) = m2.placed(moving);
                    let (qf, _) = f2.placed(fixed);
                    out.push(((pm + qm) * 0.5 - (pf + qf) * 0.5).dot(df));
                }
            }
            JointKind::Tangent { radius } => {
                // Whichever end is the flat face.
                let (plane_point, normal, axis_point, axis) = match self.moving {
                    Anchor::Plane { .. } => (pm, dm, pf, df),
                    Anchor::Axis { .. } | Anchor::Point { .. } => (pf, df, pm, dm),
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
    /// A point on each: a sphere's or a circle's centre, a point of a face.
    Point,
    /// Something with a direction on each: a flat face's normal, a round
    /// face's or an edge's axis.
    Directed,
    /// A flat face, an axis or a point on each.
    Anything,
    /// A point on the moving body and an axis (a line) on the other.
    PointAndLine,
    /// A point on the moving body and an edge of the other.
    PointAndEdge,
    /// A point (or a roller's axis) on the moving body and a face of the
    /// other.
    PointAndFace,
}

/// A tool that makes a joint from a pick on each of two bodies.
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
    Ball,
    Universal,
    Slot,
    Path,
    Cam,
    Width,
}

impl JointTool {
    pub const ALL: [JointTool; 16] = [
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
        JointTool::Ball,
        JointTool::Universal,
        JointTool::Slot,
        JointTool::Path,
        JointTool::Cam,
        JointTool::Width,
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
            JointTool::Ball => "asm.ball",
            JointTool::Universal => "asm.universal",
            JointTool::Slot => "asm.slot",
            JointTool::Path => "asm.path",
            JointTool::Cam => "asm.cam",
            JointTool::Width => "asm.width",
        }
    }

    pub fn of_command(id: &str) -> Option<JointTool> {
        Self::ALL.into_iter().find(|t| t.command() == id)
    }

    /// Its command without the `asm.` in front: `mate`, `hinge`, `fix`.
    pub fn word(self) -> &'static str {
        self.command().trim_start_matches("asm.")
    }

    pub fn of_word(word: &str) -> Option<JointTool> {
        Self::ALL.into_iter().find(|t| t.word() == word)
    }

    /// Whether it takes `anchor`, after `first` when that is picked.
    pub fn takes_anchor(self, anchor: &Anchor, first: Option<Anchor>) -> bool {
        let flat = matches!(anchor, Anchor::Plane { .. });
        let axis = matches!(anchor, Anchor::Axis { .. });
        let point = matches!(anchor, Anchor::Point { .. });
        match (self.takes(), first) {
            (Takes::Flat, _) => flat,
            (Takes::Round, _) => axis,
            (Takes::Point, _) => point,
            (Takes::Directed, _) => !point,
            (Takes::Any, _) | (Takes::Anything, _) => true,
            (Takes::PointAndLine, None) => point,
            (Takes::PointAndLine, Some(_)) => axis,
            (Takes::PointAndEdge | Takes::PointAndFace, _) => point,
            (Takes::FlatAndRound, None) => !point,
            (Takes::FlatAndRound, Some(first)) => {
                !point && matches!(first, Anchor::Plane { .. }) != flat
            }
        }
    }

    /// Whether it takes `a` on the moving body, then `b` on the other.
    pub fn fits(self, a: &Anchor, b: &Anchor) -> bool {
        self.takes_anchor(a, None) && self.takes_anchor(b, Some(*a))
    }

    /// The tool that makes a joint of this kind; `None` for a ground.
    pub fn of_kind(kind: &JointKind) -> Option<JointTool> {
        Some(match kind {
            JointKind::Mate { .. } => JointTool::Mate,
            JointKind::Align { .. } => JointTool::Align,
            JointKind::Angle { .. } => JointTool::Angle,
            JointKind::Ground => return None,
            JointKind::Hinge { .. } => JointTool::Hinge,
            JointKind::Slider { .. } => JointTool::Slider,
            JointKind::Fixed { .. } => JointTool::Fixed,
            JointKind::Parallel => JointTool::Parallel,
            JointKind::Perpendicular => JointTool::Perpendicular,
            JointKind::Distance { .. } => JointTool::Distance,
            JointKind::Tangent { .. } => JointTool::Tangent,
            JointKind::Ball => JointTool::Ball,
            JointKind::Universal => JointTool::Universal,
            JointKind::Slot => JointTool::Slot,
            JointKind::Path => JointTool::Path,
            JointKind::Cam { .. } => JointTool::Cam,
            JointKind::Width => JointTool::Width,
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
            JointTool::Ball => "Ball joint",
            JointTool::Universal => "Universal joint",
            JointTool::Slot => "Pin in a slot",
            JointTool::Path => "Along a path",
            JointTool::Cam => "Cam and follower",
            JointTool::Width => "Centred in a slot",
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
            JointTool::Ball => "point",
            JointTool::Universal => "constraint-perpendicular",
            JointTool::Slot => "constraint-point-on-object",
            JointTool::Path => "additive-pipe",
            JointTool::Cam => "constraint-tangent",
            JointTool::Width => "constraint-symmetric",
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
            JointTool::Ball => "Shift+B",
            JointTool::Universal => "Shift+U",
            JointTool::Slot => "Shift+S",
            JointTool::Path => "Shift+P",
            JointTool::Cam => "Shift+C",
            JointTool::Width => "Shift+W",
        }
    }

    /// What the joint does, in a sentence.
    pub fn summary(self) -> &'static str {
        match self {
            JointTool::Mate => "Put two flat faces against each other",
            JointTool::Align => "Put two round faces on one axis",
            JointTool::Angle => "Hold two faces or axes at an angle",
            JointTool::Hinge => "Put two axes on one line: the body can only turn about it",
            JointTool::Slider => {
                "Put two axes on one line without turning: the body can only slide along it"
            }
            JointTool::Fixed => "Hold a body to another where it sits",
            JointTool::Parallel => "Keep two faces or axes parallel",
            JointTool::Perpendicular => "Keep two faces or axes square to each other",
            JointTool::Distance => {
                "Keep two faces, axes or points a distance apart: along a face, from an axis, \
                 between axes or points"
            }
            JointTool::Tangent => "Rest a round face on a flat one",
            JointTool::Ball => "Put two points together: the body can turn every way about them",
            JointTool::Universal => {
                "Cross two yokes' pins at one point, square to each other: the body turns \
                 about either"
            }
            JointTool::Slot => "Keep a point on a line: a pin sliding in a slot",
            JointTool::Path => "Keep a point on an edge of any shape: it runs along it",
            JointTool::Cam => "Keep a follower on a cam's face, a roller's radius off it",
            JointTool::Width => "Centre a tab's two faces between a slot's two walls",
        }
    }

    pub fn takes(self) -> Takes {
        match self {
            JointTool::Mate => Takes::Flat,
            JointTool::Angle | JointTool::Parallel | JointTool::Perpendicular => Takes::Directed,
            JointTool::Distance => Takes::Anything,
            JointTool::Align | JointTool::Hinge | JointTool::Slider => Takes::Round,
            JointTool::Fixed => Takes::Any,
            JointTool::Tangent => Takes::FlatAndRound,
            JointTool::Ball => Takes::Point,
            JointTool::Universal => Takes::Round,
            JointTool::Slot => Takes::PointAndLine,
            JointTool::Path => Takes::PointAndEdge,
            JointTool::Cam => Takes::PointAndFace,
            JointTool::Width => Takes::Flat,
        }
    }

    /// How many faces it takes on each body: two for a width.
    pub fn picks_each(self) -> usize {
        if self == JointTool::Width { 2 } else { 1 }
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
            (Takes::Point, false) => {
                "Click a ball, a round edge or a point of a face on the body to move"
            }
            (Takes::Point, true) => "Click the point it sits in, on another body",
            (Takes::Directed, false) => "Click a face or an edge on the body to move",
            (Takes::Directed, true) => "Click the face or edge it keeps to, on another body",
            (Takes::Anything, false) => "Click a face, an edge or a ball on the body to move",
            (Takes::Anything, true) => "Click what it keeps its distance from, on another body",
            (Takes::PointAndLine, false) => "Click the pin: a round edge, a ball or a point",
            (Takes::PointAndLine, true) => "Click the slot's line: an edge or a round face",
            (Takes::PointAndEdge, false) => "Click the point that runs: a round edge, a ball",
            (Takes::PointAndEdge, true) => "Click the edge it runs along, on another body",
            (Takes::PointAndFace, false) => "Click the follower: a roller, a ball or a point",
            (Takes::PointAndFace, true) => "Click the cam's face, on another body",
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
            Takes::Point => "This joint takes points: a ball, a round edge's centre, a face",
            Takes::Directed => "This joint takes faces or edges with a direction",
            Takes::Anything => "Click a face, an edge or a ball",
            Takes::PointAndLine => "This joint takes a point, then a line: an edge or an axis",
            Takes::PointAndEdge => "This joint takes a point, then an edge",
            Takes::PointAndFace => "This joint takes a point, then a face",
        }
    }

    /// The joint made from two anchors where the bodies sit now: settings
    /// the tool does not ask for start at what the bodies make now, so
    /// making the joint moves nothing it need not. `radius` is the round
    /// face's for a tangent, the roller's for a cam.
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
            JointTool::Align => JointKind::Align {
                zero: turn.to_array(),
                turn: Drive::default(),
                slide: Drive::default(),
            },
            JointTool::Angle => JointKind::Angle {
                degrees: moving.angle_to(at, fixed, fixed_at),
            },
            JointTool::Hinge => JointKind::Hinge {
                offset: 0.0,
                zero: turn.to_array(),
                drive: Drive::default(),
            },
            // The turn it keeps is the one it has once its axis lies along
            // the other's, the shortest way round: kept as it is now, a
            // body whose axis points elsewhere would never come onto it.
            JointTool::Slider => {
                let (_, dm) = moving.placed(at);
                let (_, df) = fixed.placed(fixed_at);
                let (dm, df) = (dm.normalize_or_zero(), df.normalize_or_zero());
                let onto = if dm.dot(df) < 0.0 { -df } else { df };
                let lined_up = Rigid {
                    rotation: (glam::DQuat::from_rotation_arc(dm, onto) * at.rotation).normalize(),
                    translation: at.translation,
                };
                JointKind::Slider {
                    turn: relative(&lined_up, fixed_at).0.to_array(),
                    drive: Drive::default(),
                }
            }
            JointTool::Fixed => JointKind::Fixed {
                turn: turn.to_array(),
                shift: shift.to_array(),
            },
            JointTool::Parallel => JointKind::Parallel,
            JointTool::Perpendicular => JointKind::Perpendicular,
            JointTool::Distance => {
                let probe = JointFeature {
                    second: None,
                    shape: Vec::new(),
                    ends: [0.0; 2],
                    kind: JointKind::Distance { offset: 0.0 },
                    moving: *moving,
                    other_body: BodyId(uuid::Uuid::nil()),
                    fixed: *fixed,
                    names: [0; 2],
                };
                JointKind::Distance {
                    offset: probe.apart(at, fixed_at) as f32,
                }
            }
            JointTool::Tangent => JointKind::Tangent { radius },
            JointTool::Ball => JointKind::Ball,
            JointTool::Universal => JointKind::Universal,
            JointTool::Slot => JointKind::Slot,
            JointTool::Path => JointKind::Path,
            JointTool::Cam => JointKind::Cam { radius },
            JointTool::Width => JointKind::Width,
        }
    }
}
