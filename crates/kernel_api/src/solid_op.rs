//! Solid operations as plain data: profiles, sweeps, primitives, dress-ups,
//! patterns, surface steps and the op enum a chain runs.

use serde::{Deserialize, Serialize};

use crate::*;

/// One segment of a closed 2D profile wire, in sketch-plane coordinates
/// (millimetres). Arcs are encoded as three on-curve points so consumers
/// never have to agree on a winding convention.
#[cfg_attr(not(feature = "exhaustive"), non_exhaustive)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProfileSegment {
    Line {
        start: [f64; 2],
        end: [f64; 2],
    },
    /// Circular arc through three points (start → mid → end).
    Arc {
        start: [f64; 2],
        mid: [f64; 2],
        end: [f64; 2],
    },
    Circle {
        center: [f64; 2],
        radius: f64,
    },
    /// Full ellipse. `major` is the vector from the center to a major-axis
    /// vertex; the minor radius is `|major| * ratio` with `ratio` in (0, 1].
    Ellipse {
        center: [f64; 2],
        major: [f64; 2],
        ratio: f64,
    },
    /// Elliptical arc between two parameter angles (radians, counter-clockwise
    /// in the ellipse frame where the major axis is at parameter 0).
    EllipseArc {
        center: [f64; 2],
        major: [f64; 2],
        ratio: f64,
        start_param: f64,
        end_param: f64,
    },
    /// Cubic B-spline through the given control points. A periodic spline
    /// closes smoothly on itself; an open one runs first → last control point.
    BSpline {
        control_points: Vec<[f64; 2]>,
        periodic: bool,
    },
    /// A B-spline of any degree over its own knots, rational when it has
    /// weights: an exact conic arc, a spline drawn through points. An open
    /// one's knots are clamped (`control points + degree + 1` of them), so
    /// it runs first → last control point; a periodic one is a uniform ring
    /// over its control points closing smoothly on itself, and its knots
    /// are not used.
    Nurbs {
        degree: u32,
        knots: Vec<f64>,
        control_points: Vec<[f64; 2]>,
        /// One per control point; empty when every weight is one.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        weights: Vec<f64>,
        #[serde(default)]
        periodic: bool,
    },
}

/// What a [`SolidOp::PipeThrough`] runs along.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PipePath {
    /// A sketch's wire.
    Profile(Profile),
    /// Edges of the running solid, joined end to end.
    Edges(Vec<EdgeProbe>),
    /// Edges of another solid (a native-format snapshot, moved by
    /// `transform` into the body's frame), joined end to end; the probes
    /// are in the body's frame.
    EdgesOf {
        shape: Vec<u8>,
        transform: Option<Box<[[f64; 4]; 4]>>,
        edges: Vec<EdgeProbe>,
    },
}

/// One section of a [`SolidOp::LoftThrough`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LoftSection {
    Profile(Profile),
    /// The flat face of the running solid the probe finds, by its outer
    /// and inner boundaries.
    Face(FaceProbe),
    /// A point the loft closes to, in the body's frame.
    Point([f64; 3]),
}

/// A closed loop of profile segments. Consecutive segments share endpoints;
/// a single `Circle` segment is a wire by itself.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProfileWire {
    pub segments: Vec<ProfileSegment>,
    /// The name of each segment, one per segment, by what it was drawn from
    /// (a sketch element): the faces a feature sweeps from it are named by
    /// it, so they keep their names while dimensions change. Empty, or a
    /// zero, where a segment has no such name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<TopoName>,
}

impl ProfileWire {
    /// A wire of `segments` with no names.
    pub fn new(segments: Vec<ProfileSegment>) -> Self {
        Self {
            segments,
            names: Vec::new(),
        }
    }

    /// The name of segment `index`, when it has one.
    pub fn name_of(&self, index: usize) -> Option<TopoName> {
        self.names.get(index).copied().filter(|n| *n != 0)
    }
}

/// The plane a profile lives on, in world coordinates (millimetres).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProfilePlane {
    pub origin: [f64; 3],
    pub x_axis: [f64; 3],
    pub y_axis: [f64; 3],
    pub normal: [f64; 3],
}

/// The XY plane through the origin.
impl Default for ProfilePlane {
    fn default() -> Self {
        Self {
            origin: [0.0; 3],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 1.0, 0.0],
            normal: [0.0, 0.0, 1.0],
        }
    }
}

/// How an extrusion combines with the body's existing solid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BooleanOp {
    /// Replace / start the body's solid (first feature).
    NewSolid,
    /// Union with the existing solid (Pad on an existing body).
    Fuse,
    /// Subtract from the existing solid (Pocket).
    Cut,
    /// Keep only what the existing solid and the tool share.
    Common,
}

/// A set of closed wires on one plane. The largest-area wire is the outer
/// boundary, the rest become holes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub plane: ProfilePlane,
    pub wires: Vec<ProfileWire>,
}

/// Where a one-directional extrusion stops.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExtrudeTermination {
    /// Fixed length in millimetres.
    Blind { distance: f64 },
    /// Extend well past the base solid's bounding box in this direction.
    ThroughAll,
    /// Stop on the given world-space plane, shifted by `offset` along its
    /// normal (a picked planar face plus an offset-to-face value).
    UpToPlane {
        point: [f64; 3],
        normal: [f64; 3],
        offset: f64,
    },
    /// Stop on the base solid's face named `name`, else the one nearest
    /// `point` (a picked face, where it was picked), exactly on its surface
    /// whatever its shape, pushed `offset` out along its outward normal.
    /// With no base solid, or no face of it there, the plane through
    /// `point` square to `normal`.
    UpToFace {
        point: [f64; 3],
        normal: [f64; 3],
        offset: f64,
        #[serde(default)]
        name: TopoName,
    },
    /// Stop at the first face of the base solid hit along the extrusion
    /// direction, on its surface.
    ToFirst,
    /// Stop at the last face of the base solid hit along the extrusion
    /// direction, on its surface.
    ToLast,
    /// Stop on a set of the base solid's faces: every line of the sweep
    /// ends at the first of them it meets, moved `offset` further along the
    /// sweep (a negative offset stops short).
    UpToShape { faces: Vec<FaceProbe>, offset: f64 },
    /// Stop on a face of another shape: the face of `shape` (a native-format
    /// snapshot, moved by `transform`, a rigid row-major 4×4 matrix, into
    /// the body's frame) nearest `point`, exactly on its surface whatever
    /// its shape, pushed `offset` out along its outward normal.
    UpToFaceOf {
        shape: Vec<u8>,
        transform: Option<Box<[[f64; 4]; 4]>>,
        point: [f64; 3],
        offset: f64,
    },
    /// Stop on a set of faces as `UpToShape` does, some of them the base
    /// solid's (`faces`) and some of other shapes (`others`, faces other
    /// bodies lend this one). Needs no base solid when every face is
    /// another shape's.
    UpToShapeOf {
        faces: Vec<FaceProbe>,
        others: Vec<FacesOf>,
        offset: f64,
    },
}

/// Faces of another shape: the faces of `shape` (a native-format snapshot,
/// moved by `transform`, a rigid row-major 4×4 matrix, into the body's
/// frame) nearest each of `points`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FacesOf {
    pub shape: Vec<u8>,
    pub transform: Option<Box<[[f64; 4]; 4]>>,
    pub points: Vec<[f64; 3]>,
}

/// Where a revolution stops turning.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum RevolveTermination {
    /// Through the sweep's own angles.
    #[default]
    Angle,
    /// On the first face of the base solid the profile's centre meets as it
    /// turns.
    ToFirst,
    /// On the last face of the base solid the profile's centre meets as it
    /// turns.
    ToLast,
    /// On the base solid's face nearest the probe.
    UpToFace(FaceProbe),
}

/// How a profile is swept into a solid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SweepKind {
    /// Linear extrusion along `plane.normal` (or `direction` when set).
    Extrude {
        termination: ExtrudeTermination,
        /// Independent termination for the opposite side of the sketch plane
        /// (two-sided extrusion). `None` extrudes one side only.
        second_side: Option<ExtrudeTermination>,
        /// Extrude half the blind distance to each side of the sketch plane.
        symmetric: bool,
        /// Swap which side of the sketch plane is "forward".
        reversed: bool,
        /// Draft angle applied along the sweep; positive widens the far end.
        taper_deg: f64,
        /// Custom world-space sweep direction; `None` uses the plane normal.
        direction: Option<[f64; 3]>,
    },
    /// Revolution about an axis lying IN the sketch plane, given in sketch
    /// 2D coordinates (point + direction). `angle_deg` in (0, 360].
    Revolve {
        axis_origin: [f64; 2],
        axis_dir: [f64; 2],
        angle_deg: f64,
        /// Independent sweep angle for the opposite rotation direction.
        second_angle_deg: Option<f64>,
        /// Center the sweep on the sketch plane (half the angle each way).
        midplane: bool,
        reversed: bool,
        /// Where the turn stops; anything but `Angle` takes the angle from
        /// what the profile meets and leaves the angles above unused.
        #[serde(default)]
        termination: RevolveTermination,
    },
    /// Sweep the profile along a helix whose axis lies in the sketch plane.
    /// A cone angle moves every point away from the axis by tan(angle) of
    /// every pitch it climbs.
    Helix {
        axis_origin: [f64; 2],
        axis_dir: [f64; 2],
        pitch: f64,
        height: f64,
        left_handed: bool,
        cone_angle_deg: f64,
        reversed: bool,
        /// How many turns, when given: the pitch is then `height / turns`
        /// and `pitch` is not read. A height of 0 with turns and growth is
        /// a flat spiral.
        #[serde(default)]
        turns: Option<f64>,
        /// How far every point moves away from the axis per turn, when
        /// given, in place of `cone_angle_deg`.
        #[serde(default)]
        growth: Option<f64>,
    },
    /// Sweep the profile along a helix whose axis runs square to the sketch
    /// plane through `axis_origin` (sketch 2D coordinates): every point of
    /// the profile climbs its own helix about it, the profile staying
    /// level. The rest reads as for `Helix`.
    HelixNormal {
        axis_origin: [f64; 2],
        pitch: f64,
        height: f64,
        left_handed: bool,
        cone_angle_deg: f64,
        reversed: bool,
        #[serde(default)]
        turns: Option<f64>,
        #[serde(default)]
        growth: Option<f64>,
    },
}

/// How a pipe's section turns as it runs down its path.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum PipeFrame {
    /// Rotation-minimizing: the section neither twists nor kinks where the
    /// path bends.
    #[default]
    RotationMinimizing,
    /// The path's own curvature frame, turning with it.
    Frenet,
    /// The section turns about the path so that one direction of it keeps
    /// pointing at a second path running beside the first.
    Auxiliary { path: Profile },
    /// The section keeps a fixed binormal: the world-space direction square
    /// to the path that the section's frame holds all the way.
    Binormal { direction: [f64; 3] },
    /// The section keeps its orientation in space: carried along the path
    /// without ever turning, as it is drawn.
    Fixed,
}

impl PipeFrame {
    /// The frame an older `frenet` flag named.
    pub fn from_frenet(frenet: bool) -> Self {
        if frenet {
            PipeFrame::Frenet
        } else {
            PipeFrame::RotationMinimizing
        }
    }
}

/// Reads a [`PipeFrame`] or the `frenet` flag it stands in for.
pub fn deserialize_pipe_frame<'de, D>(deserializer: D) -> Result<PipeFrame, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stored {
        Flag(bool),
        Frame(PipeFrame),
    }
    Ok(match Stored::deserialize(deserializer)? {
        Stored::Flag(frenet) => PipeFrame::from_frenet(frenet),
        Stored::Frame(frame) => frame,
    })
}

/// How a pipe's section turns a sharp corner of its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PipeCorner {
    /// The section is carried into the corner and sheared onto the plane
    /// bisecting it: the two legs meet on that mitre.
    #[default]
    Transformed,
    /// Each leg runs on straight past the corner and the two are fused,
    /// leaving the outside of the corner square.
    Right,
    /// The section turns about the corner, the outside of it rounded.
    Round,
}

/// Parametric primitive shapes (dimensions in millimetres, angles degrees).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PrimitiveKind {
    Box {
        length: f64,
        width: f64,
        height: f64,
    },
    Cylinder {
        radius: f64,
        height: f64,
        angle_deg: f64,
    },
    Sphere {
        radius: f64,
        /// Latitude range and longitude sweep (three-angle parameterization).
        angle1_deg: f64,
        angle2_deg: f64,
        angle3_deg: f64,
    },
    Cone {
        radius1: f64,
        radius2: f64,
        height: f64,
        angle_deg: f64,
    },
    Torus {
        radius1: f64,
        radius2: f64,
        angle1_deg: f64,
        angle2_deg: f64,
        angle3_deg: f64,
    },
    /// An ellipsoid, cut as a sphere is: between two latitudes about its
    /// third axis and through a sweep round it.
    Ellipsoid {
        radius1: f64,
        radius2: f64,
        radius3: f64,
        #[serde(default = "south_pole")]
        angle1_deg: f64,
        #[serde(default = "north_pole")]
        angle2_deg: f64,
        #[serde(default = "full_turn")]
        angle3_deg: f64,
    },
    /// A regular prism, its top shifted by the skew angles: leaning that
    /// far from its axis toward its x and its y.
    Prism {
        sides: u32,
        circumradius: f64,
        height: f64,
        #[serde(default)]
        skew_x_deg: f64,
        #[serde(default)]
        skew_y_deg: f64,
    },
    /// A hollow cylinder: a wall between the radii `inner` and `outer`
    /// about its axis, `height` tall. The inner radius is above zero and
    /// below the outer.
    Tube { outer: f64, inner: f64, height: f64 },
    /// Box with an independently sized top rectangle (zero top spans give a
    /// pyramid). Spans are along local X (`x2*`) and Z (`z2*`) at height Y.
    Wedge {
        xmin: f64,
        xmax: f64,
        ymin: f64,
        ymax: f64,
        zmin: f64,
        zmax: f64,
        x2min: f64,
        x2max: f64,
        z2min: f64,
        z2max: f64,
    },
}

fn south_pole() -> f64 {
    -90.0
}

fn north_pole() -> f64 {
    90.0
}

fn full_turn() -> f64 {
    360.0
}

/// A right-handed placement frame in world coordinates (millimetres).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub origin: [f64; 3],
    pub x_axis: [f64; 3],
    pub z_axis: [f64; 3],
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            origin: [0.0; 3],
            x_axis: [1.0, 0.0, 0.0],
            z_axis: [0.0, 0.0, 1.0],
        }
    }
}

/// Which edges of the current solid a dress-up applies to. Edges are
/// identified geometrically: by a face they border (sample point on the
/// face) or by a point near the edge itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EdgeSelection {
    All,
    OfFaces(Vec<[f64; 3]>),
    /// The edges bordering the picked faces, each found by its name when
    /// it has one.
    OfPickedFaces(Vec<FaceProbe>),
    Near(Vec<[f64; 3]>),
    /// Edges picked one by one: each the nearest edge to its point that
    /// runs along its direction there.
    Picked(Vec<EdgeProbe>),
}

/// A picked edge, as a point beside it and the way it runs there, and the
/// names of the two faces it runs between. A rebuild finds the edge
/// between faces of those names, the point choosing among several; without
/// names, the nearest edge. A zero direction leaves the way open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct EdgeProbe {
    pub point: [f64; 3],
    pub direction: [f64; 3],
    #[serde(default)]
    pub faces: [TopoName; 2],
}

/// How a thickness's walls meet across an edge of the solid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThicknessJoin {
    /// The walls run on until they meet: sharp corners.
    #[default]
    Intersection,
    /// The walls round about the edge, the wall thickness its radius.
    Arc,
}

impl ThicknessJoin {
    pub const ALL: [ThicknessJoin; 2] = [ThicknessJoin::Intersection, ThicknessJoin::Arc];

    pub fn label(&self) -> &'static str {
        match self {
            ThicknessJoin::Intersection => "Intersection",
            ThicknessJoin::Arc => "Arc",
        }
    }
}

/// Chamfer sizing, mirroring the three standard input styles.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ChamferSpec {
    EqualDistance { distance: f64 },
    TwoDistances { distance1: f64, distance2: f64 },
    DistanceAngle { distance: f64, angle_deg: f64 },
}

/// Boolean between the running solid and an external tool solid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoolKind {
    Fuse,
    Cut,
    Common,
}

/// One step in a body's build history. Shape-producing steps (`Sweep`,
/// `Loft`, `Pipe`, `Primitive`) carry a [`BooleanOp`]; the remaining steps
/// modify or combine the running solid directly.
///
/// A package builds these, so a variant keeps its fields for as long as
/// the contract's minor version holds: a new option comes as a new
/// variant, or as a new struct that derives `Default`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SolidOp {
    /// Start the chain from a snapshot of another solid, in the native
    /// format `SolidBuildResult::brep_blob` carries. Only ever the first op.
    Shape { brep: Vec<u8> },
    Sweep {
        profile: Profile,
        kind: SweepKind,
        op: BooleanOp,
    },
    /// Extrude a flat face of the running solid: its outer and inner
    /// boundaries are the profile and its outward normal the sketch
    /// normal. `kind` is an `Extrude`.
    SweepFace {
        face: FaceProbe,
        kind: SweepKind,
        op: BooleanOp,
    },
    /// Extrude a flat face of another shape (native-format `shape`, moved by
    /// `transform` into this body's frame): the face of it at `point`, a
    /// face another body lends this one.
    SweepFaceOf {
        shape: Vec<u8>,
        transform: Option<Box<[[f64; 4]; 4]>>,
        point: [f64; 3],
        kind: SweepKind,
        op: BooleanOp,
    },
    /// Revolve a profile until it meets a face of another shape
    /// (native-format `shape`, moved by `transform` into this body's frame):
    /// the face of it nearest `point`, a face another body lends this one.
    /// `kind` is a `Revolve`, whose own termination is not read.
    RevolveToFaceOf {
        profile: Profile,
        kind: SweepKind,
        shape: Vec<u8>,
        transform: Option<Box<[[f64; 4]; 4]>>,
        point: [f64; 3],
        op: BooleanOp,
    },
    /// Skin through two or more sections (in order), each a profile, a
    /// flat face of the running solid (its boundary) or a point, which
    /// only the first or last may be.
    LoftThrough {
        sections: Vec<LoftSection>,
        ruled: bool,
        closed: bool,
        op: BooleanOp,
    },
    /// Skin through two or more section profiles (in order).
    Loft {
        sections: Vec<Profile>,
        ruled: bool,
        closed: bool,
        op: BooleanOp,
    },
    /// Sweep a profile along an open or closed spine wire from another
    /// sketch, the section turning as `frame` says and turning its corners
    /// as `corner` says. With `sections`, the section changes shape down
    /// the path, passing through each of them where it crosses the path,
    /// in order, the profile first.
    Pipe {
        profile: Profile,
        spine: Profile,
        #[serde(default, alias = "frenet", deserialize_with = "deserialize_pipe_frame")]
        frame: PipeFrame,
        #[serde(default)]
        corner: PipeCorner,
        #[serde(default)]
        sections: Vec<Profile>,
        op: BooleanOp,
    },
    /// A pipe whose profile may be a flat face of the running solid, whose
    /// path may be edges of a solid, and whose last section may be a point
    /// it closes to; the rest reads as for `Pipe`.
    PipeThrough {
        profile: LoftSection,
        path: PipePath,
        #[serde(default)]
        frame: PipeFrame,
        #[serde(default)]
        corner: PipeCorner,
        #[serde(default)]
        sections: Vec<LoftSection>,
        op: BooleanOp,
    },
    Primitive {
        kind: PrimitiveKind,
        placement: Placement,
        op: BooleanOp,
    },
    /// Round the selected edges; with `follow_tangent`, every edge meeting
    /// one of them tangentially joins the selection, and so on along the
    /// chain.
    Fillet {
        radius: f64,
        edges: EdgeSelection,
        #[serde(default)]
        follow_tangent: bool,
    },
    /// Round between two faces of the running solid, which need share no
    /// edge: the ball rolling between them, its round filling or taking off
    /// what lies between it and the faces' meeting (a step across a groove).
    BlendFaces { radius: f64, faces: [FaceProbe; 2] },
    /// Bevel the selected edges, taking tangent chains as a fillet does.
    Chamfer {
        spec: ChamferSpec,
        flip: bool,
        edges: EdgeSelection,
        #[serde(default)]
        follow_tangent: bool,
    },
    /// Tilt the selected faces by `angle_deg` about their intersection with
    /// the neutral plane.
    Draft {
        angle_deg: f64,
        neutral_point: [f64; 3],
        neutral_normal: [f64; 3],
        pull_dir: Option<[f64; 3]>,
        faces: Vec<[f64; 3]>,
        /// The name of each face, one per point, found by name first; empty
        /// or zero where a face has none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        face_names: Vec<TopoName>,
    },
    /// Merge adjacent faces of the solid that lie on one plane: the split
    /// a fuse or cut leaves where two pieces meet flush.
    Refine,
    /// Offset faces of the running solid along their outward normals by
    /// `distance` (negative into the material), the faces around them
    /// following on their own surfaces.
    OffsetFaces {
        faces: Vec<[f64; 3]>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        face_names: Vec<TopoName>,
        distance: f64,
    },
    /// Move faces of the running solid rigidly by `transform` (row-major,
    /// a translation or a rotation), the faces around them following.
    MoveFaces {
        faces: Vec<[f64; 3]>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        face_names: Vec<TopoName>,
        transform: [[f64; 4]; 4],
    },
    /// Remove faces of the running solid and close the openings from the
    /// neighbouring faces' own geometry: a hole, a boss or a fillet taken
    /// away. Faces as points on them, named where they have names.
    RemoveFaces {
        faces: Vec<[f64; 3]>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        face_names: Vec<TopoName>,
    },
    /// Hollow the solid, removing the faces sampled by `open_faces`.
    Thickness {
        value: f64,
        open_faces: Vec<[f64; 3]>,
        /// The name of each open face, one per point; empty or zero where
        /// a face has none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        open_face_names: Vec<TopoName>,
        inward: bool,
        /// How the walls meet where the solid's faces meet.
        #[serde(default)]
        join: ThicknessJoin,
        /// The walls stand on both sides of the solid's faces, `value` each
        /// way, rather than on the one `inward` says.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        both_sides: bool,
    },
    /// Re-apply earlier steps' tool solids (or the whole current solid when
    /// `originals` is empty) under each transform, fusing additive tools and
    /// cutting subtractive ones. `originals` holds chain indices of earlier
    /// shape-producing steps.
    Transform {
        transforms: Vec<[[f64; 4]; 4]>,
        originals: Vec<usize>,
    },
    /// [`SolidOp::Transform`] with the originals themselves left out: the
    /// steps `originals` names build their tools without adding or cutting
    /// them, so only the transformed copies go in. With `originals` empty
    /// the running solid gives way to its copies.
    TransformCopies {
        transforms: Vec<[[f64; 4]; 4]>,
        originals: Vec<usize>,
    },
    /// [`SolidOp::Transform`] whose transforms the chain works out from
    /// `steps` where the op stands, so a step may follow an edge of the
    /// solid as it is there. Occurrence 0 is the original; with it among
    /// `skipped`, only the copies go in, as [`SolidOp::TransformCopies`].
    Pattern {
        steps: Vec<PatternStep>,
        originals: Vec<usize>,
        /// Occurrences left out, by index: 0 the original, then each copy
        /// in the order the steps make them.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        skipped: Vec<u32>,
    },
    /// Boolean against an external solid (another body's shape snapshot,
    /// ogeom native-format text bytes).
    Boolean {
        tool_brep: Vec<u8>,
        kind: BoolKind,
        /// Where the tool sits in this chain's frame, when the two bodies
        /// are placed differently: a rigid row-major 4×4 matrix.
        #[serde(default)]
        tool_transform: Option<[[f64; 4]; 4]>,
    },
    /// A surface step: a sheet added to the body, or the body's sheets
    /// joined, trimmed or mirrored.
    Surface(SurfaceOp),
    /// `op` with its booleans run at a stated tolerance: geometry within
    /// `tolerance` mm of touching counts as touching, so faces that nearly
    /// meet are joined rather than left with a gap or a sliver between
    /// them. The op keeps its place in the chain, as a pattern's
    /// `originals` count it.
    WithTolerance { tolerance: f64, op: Box<SolidOp> },
    /// A sketch carried along its plane's normal onto faces of the running
    /// solid, each face split along the curves where they land; `mode`
    /// says whether the regions the curves enclose then stand out of the
    /// faces or sink into them by `height`.
    Project {
        profile: Profile,
        faces: Vec<FaceProbe>,
        mode: ProjectMode,
        height: f64,
    },
    /// A sketch wrapped onto the curved surface its `faces` lie on: the
    /// sketch's middle lands where a straight projection along its plane's
    /// normal puts it, and the rest is laid onto the surface from there.
    /// A cylinder or cone is unrolled, every length kept, the way across
    /// the axis turning about it and the way along the axis following it;
    /// any other surface takes it as a flat sheet laid by geodesic polar
    /// coordinates, lengths out from the middle kept and lengths across
    /// them stretched by the surface's curvature. Otherwise as
    /// [`SolidOp::Project`].
    Wrap {
        profile: Profile,
        faces: Vec<FaceProbe>,
        mode: ProjectMode,
        height: f64,
    },
    /// Keep the running solid's material on one side of a plane: the side
    /// `normal` points to from `origin`. A plane that leaves nothing on
    /// that side fails, saying so.
    HalfSpace { origin: [f64; 3], normal: [f64; 3] },
    /// Grow the running solid by `distance` everywhere along its faces'
    /// outward normals, or shrink it with a negative one: every face moved
    /// that far and the faces around it following, its topology kept.
    OffsetSolid { distance: f64 },
    /// Scale the running solid by `factors` along the body's X, Y and Z
    /// about `centre`, or about the middle of its bounding box when that
    /// is `None`. Equal factors keep its surfaces' kinds; unequal ones
    /// restate them as the kernel's general transform does.
    Scale {
        factors: [f64; 3],
        centre: Option<[f64; 3]>,
    },
    /// Keep one piece of the running shape: of its solids, the one
    /// nearest `near`, the others left out.
    KeepPiece { near: [f64; 3] },
}

/// What a [`SolidOp::Project`] makes of the faces a sketch lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ProjectMode {
    /// The faces split along the sketch's curves, the solid's shape
    /// unchanged: lines later features and face colours can use.
    #[default]
    Split,
    /// The regions the curves enclose raised out of the faces.
    Emboss,
    /// The regions the curves enclose sunk into the faces.
    Deboss,
}

impl ProjectMode {
    pub const ALL: [ProjectMode; 3] =
        [ProjectMode::Split, ProjectMode::Emboss, ProjectMode::Deboss];

    pub fn label(&self) -> &'static str {
        match self {
            ProjectMode::Split => "Split lines",
            ProjectMode::Emboss => "Emboss",
            ProjectMode::Deboss => "Deboss",
        }
    }
}

/// A curve a surface is built from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CurveSource {
    /// A chain of a sketch's curves, open or closed, on its plane.
    Sketch {
        plane: ProfilePlane,
        wire: ProfileWire,
    },
    /// An edge of the body's own shape as it stands where the step comes,
    /// found as a fillet's picked edges are.
    Edge(EdgeProbe),
    /// An edge of another body's shape: `brep` is that body's snapshot,
    /// `edge` finds the edge in its own frame, and `transform` (a rigid
    /// row-major 4×4 matrix) carries it to where it sits in this chain's
    /// frame, when the two bodies are placed differently.
    BodyEdge {
        brep: Vec<u8>,
        edge: EdgeProbe,
        #[serde(default)]
        transform: Option<Box<[[f64; 4]; 4]>>,
    },
}

/// How a surface meets the faces beside it across a shared edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Continuity {
    /// Touching: no gap, a crease allowed.
    #[default]
    G0,
    /// Tangent: no crease.
    G1,
    /// Curvature continuous: no change in how sharply it bends.
    G2,
}

/// The steps of surface modelling. A constructive step (every one but
/// `Sew`, `TrimByPlane` and `Mirror`) adds its sheet to the body beside
/// what is there; the rest act on all of the body's shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SurfaceOp {
    /// Each curve swept straight along `direction` by `length`, both ways
    /// by half of it when `symmetric`.
    Extrude {
        curves: Vec<CurveSource>,
        direction: [f64; 3],
        length: f64,
        #[serde(default)]
        symmetric: bool,
    },
    /// Each curve turned by `angle_deg` about the axis through `origin`
    /// along `axis`.
    Revolve {
        curves: Vec<CurveSource>,
        origin: [f64; 3],
        axis: [f64; 3],
        angle_deg: f64,
    },
    /// The flat face closed planar loops bound, a loop inside another a
    /// hole in it.
    PlanarFill { curves: Vec<CurveSource> },
    /// The surface curves meeting end to end bound, meeting the faces the
    /// boundary edges belong to with `continuity`.
    Fill {
        boundary: Vec<CurveSource>,
        #[serde(default)]
        continuity: Continuity,
    },
    /// The surface of straight lines between two curves, end to end.
    Ruled {
        first: CurveSource,
        second: CurveSource,
    },
    /// A surface through section curves in order, closing back to the first
    /// when `closed`.
    Loft {
        sections: Vec<CurveSource>,
        #[serde(default)]
        closed: bool,
    },
    /// A profile curve swept along a path.
    Sweep {
        profile: Vec<CurveSource>,
        path: Vec<CurveSource>,
        #[serde(default)]
        frame: PipeFrame,
    },
    /// A surface through section curves in order that also follows guide
    /// curves, each crossing every section once: one face.
    GuidedLoft {
        sections: Vec<CurveSource>,
        guides: Vec<CurveSource>,
    },
    /// A profile running from the first rail's start to the second's,
    /// swept along both: at each point along them it is turned and scaled
    /// so its ends ride the rails.
    SweepTwoRails {
        profile: Vec<CurveSource>,
        first_rail: Vec<CurveSource>,
        second_rail: Vec<CurveSource>,
    },
    /// Faces carried off along their normals by `distance`, as new faces.
    Offset {
        faces: Vec<FaceProbe>,
        distance: f64,
    },
    /// Faces carried past picked edges of their boundary by `length`.
    Extend {
        edges: Vec<EdgeProbe>,
        length: f64,
        #[serde(default)]
        continuity: Continuity,
    },
    /// A surface bridging two edges, meeting each edge's face with
    /// `continuity`.
    Blend {
        first: EdgeProbe,
        second: EdgeProbe,
        #[serde(default)]
        continuity: Continuity,
    },
    /// Picked faces split along curves projected onto them.
    Split {
        faces: Vec<FaceProbe>,
        curves: Vec<CurveSource>,
    },
    /// The body's faces joined where their edges meet, or come within
    /// `gap` of each other (0 for exactly); a shell that closes becomes a
    /// solid.
    Sew {
        #[serde(default)]
        gap: f64,
    },
    /// A round of `radius` along picked edges where two faces of a sheet
    /// meet.
    Fillet { edges: Vec<EdgeProbe>, radius: f64 },
    /// A round of `radius` between two faces of the body that need share
    /// no edge, such as two separate surfaces, each cut back to where it
    /// meets the round. The ball rolls on the side each face's normal
    /// points to, on the other where `flip` says so.
    FilletFaces {
        first: FaceProbe,
        second: FaceProbe,
        radius: f64,
        #[serde(default)]
        flip: [bool; 2],
    },
    /// The body's sheets given `thickness` along their normals (both ways
    /// by half when `both_sides`): a solid.
    Thicken {
        thickness: f64,
        #[serde(default)]
        both_sides: bool,
    },
    /// The body's sheets cut by the plane through `origin`, the side
    /// `normal` points to kept.
    TrimByPlane { origin: [f64; 3], normal: [f64; 3] },
    /// The body's shape and its reflection in the plane through `origin`
    /// square to `normal`.
    Mirror { origin: [f64; 3], normal: [f64; 3] },
    /// The body's sheets cut by each other, the faces lying on `scan`
    /// kept (at least `share` of each face's area within `tolerance` of
    /// it) and sewn: a closed shell becomes a solid. The scan's
    /// triangles, `scan` vertices taken three by three from `triangles`,
    /// are in the body's frame.
    TrimByScan {
        scan: Vec<[f32; 3]>,
        triangles: Vec<u32>,
        tolerance: f64,
        share: f64,
    },
    /// A free-form (B-spline) sheet fitted to scattered `points`, such as
    /// a region of a scan, within `tolerance` of them where it can.
    FitScattered {
        points: Vec<[f64; 3]>,
        tolerance: f64,
    },
    /// A network of B-spline patches fitted to `points` at once, each
    /// patch bounded by curves of the network and meeting its neighbours
    /// across them without a gap, tangent unless the curve is a crease.
    /// The network's curves are `curves`, one chain each, then
    /// `polylines`; a patch names its sides in order round it as curve
    /// numbers from 1, negative where the curve runs against the walk and 0
    /// for a side shrunk to a point.
    FitNetwork {
        curves: Vec<CurveSource>,
        polylines: Vec<Vec<[f64; 3]>>,
        #[serde(default)]
        patches: Vec<[i32; 4]>,
        /// Additional patches with each side a chain in walking order.
        /// An empty side is shrunk to a point; chain entries are nonzero
        /// signed curve numbers, as in `patches`.
        #[serde(default)]
        chained: Vec<[Vec<i32>; 4]>,
        /// Parameter-correction passes after refinement.
        #[serde(default)]
        corrections: u32,
        points: Vec<[f64; 3]>,
        tolerance: f64,
        smoothing: f64,
        /// The curves, numbered from 1, the patches meet across creased.
        creases: Vec<u32>,
    },
}

impl SurfaceOp {
    /// Whether the step makes a new sheet (and so may begin a chain).
    pub fn constructs(&self) -> bool {
        !matches!(
            self,
            SurfaceOp::Sew { .. }
                | SurfaceOp::Fillet { .. }
                | SurfaceOp::FilletFaces { .. }
                | SurfaceOp::Thicken { .. }
                | SurfaceOp::TrimByPlane { .. }
                | SurfaceOp::Mirror { .. }
                | SurfaceOp::Split { .. }
                | SurfaceOp::Extend { .. }
                | SurfaceOp::TrimByScan { .. }
        )
    }
}

/// One step of a [`SolidOp::Pattern`]: each applies to every occurrence of
/// the steps before it, the original among them, which stands first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PatternStep {
    /// A copy under each transform (rigid or not, row-major), the
    /// original, untransformed, left out.
    Transforms(Vec<[[f64; 4]; 4]>),
    /// Copies along a path.
    Path(PathLayout),
}

/// The path a [`PathLayout`] runs along, in the chain's frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PatternPath {
    /// A chain of curves on a plane, walked from its first segment's
    /// start.
    Wire {
        plane: ProfilePlane,
        wire: ProfileWire,
    },
    /// The edge of the solid, where the step stands, that the probe finds,
    /// walked from its curve's start.
    Edge(EdgeProbe),
}

/// Occurrences at stations along a path by arc length, the original at
/// the first: occurrence k is moved from where the first station stands
/// to where the k-th does, turned as the path turns between them when
/// `align`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathLayout {
    pub path: PatternPath,
    /// Occurrences, the original counted.
    pub count: u32,
    /// Millimetres between neighbours along the path; `None` spreads them
    /// over the whole of it, evenly round it when it closes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing: Option<f64>,
    /// Walk the path from its end.
    #[serde(default)]
    pub reversed: bool,
    /// Turn each copy as the path's direction turns from the first
    /// station to its own, carried along the path without twisting.
    #[serde(default)]
    pub align: bool,
}

impl SolidOp {
    /// The op itself, out of any [`SolidOp::WithTolerance`] around it.
    pub fn inner(&self) -> &SolidOp {
        match self {
            SolidOp::WithTolerance { op, .. } => op.inner(),
            op => op,
        }
    }

    /// The tolerance the op's booleans run at, when one is stated: the
    /// outermost [`SolidOp::WithTolerance`]'s.
    pub fn boolean_tolerance(&self) -> Option<f64> {
        match self {
            SolidOp::WithTolerance { tolerance, .. } => Some(*tolerance),
            _ => None,
        }
    }

    /// The op with its booleans run at `tolerance`, or as it is with none.
    pub fn with_boolean_tolerance(self, tolerance: Option<f64>) -> SolidOp {
        match tolerance {
            Some(tolerance) => SolidOp::WithTolerance {
                tolerance,
                op: Box::new(self),
            },
            None => self,
        }
    }

    /// Whether the step makes a shape of its own, so a chain may begin with
    /// it: a new solid, or a surface step that adds a sheet.
    pub fn starts_shape(&self) -> bool {
        match self.inner() {
            SolidOp::Surface(op) => op.constructs(),
            _ => self.boolean_op() == Some(BooleanOp::NewSolid),
        }
    }

    /// The boolean role of a shape-producing step; `None` for modifiers.
    pub fn boolean_op(&self) -> Option<BooleanOp> {
        match self.inner() {
            SolidOp::Sweep { op, .. }
            | SolidOp::SweepFace { op, .. }
            | SolidOp::SweepFaceOf { op, .. }
            | SolidOp::RevolveToFaceOf { op, .. }
            | SolidOp::Loft { op, .. }
            | SolidOp::LoftThrough { op, .. }
            | SolidOp::PipeThrough { op, .. }
            | SolidOp::Pipe { op, .. }
            | SolidOp::Primitive { op, .. } => Some(*op),
            // A snapshot is a solid in itself: it begins a chain.
            SolidOp::Shape { .. } => Some(BooleanOp::NewSolid),
            _ => None,
        }
    }
}
