//! Fitting and measuring: shapes fitted to points, region growing,
//! distances, registration, thickness, orientation, shadows and symmetry.

use serde::{Deserialize, Serialize};

use crate::*;

/// How a shape is fitted to measured points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FitMethod {
    /// Orthogonal least squares: the mean shape.
    #[default]
    LeastSquares,
    /// The middle of the narrowest zone holding every point (flatness,
    /// straightness, circularity, cylindricity, sphericity).
    MinimumZone,
    /// The largest circle or cylinder with every point outside it (a
    /// hole's mating size).
    Inscribed,
    /// The smallest circle or cylinder with every point inside it (a
    /// boss's mating size).
    Circumscribed,
}

/// What shape a set of measured points is fitted to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FitKind {
    Plane,
    Sphere,
    Cylinder,
    Cone,
    Torus,
    /// A circle in space: its plane and its round together.
    Circle,
    Line,
}

impl FitKind {
    pub const ALL: [FitKind; 7] = [
        FitKind::Plane,
        FitKind::Sphere,
        FitKind::Cylinder,
        FitKind::Cone,
        FitKind::Torus,
        FitKind::Circle,
        FitKind::Line,
    ];
}

/// Points to fit a shape to, and how.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FitRequest {
    pub kind: FitKind,
    /// In millimetres, in whatever frame the answer is wanted in.
    pub points: Vec<[f64; 3]>,
    /// A normal per point, out of the material, or empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub normals: Vec<[f64; 3]>,
    /// Points farther than this many sigmas from the fit are left out and
    /// it is fitted again, until what is kept stops changing; `None` keeps
    /// every point.
    #[serde(default)]
    pub sigma_cutoff: Option<f64>,
    /// A sphere, cylinder or circle's radius held, only the rest fitted.
    #[serde(default)]
    pub fixed_radius: Option<f64>,
}

/// A fitted shape, in the request's frame. Directions are unit length.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FittedShape {
    Plane {
        origin: [f64; 3],
        normal: [f64; 3],
    },
    Sphere {
        centre: [f64; 3],
        radius: f64,
    },
    Cylinder {
        point: [f64; 3],
        axis: [f64; 3],
        radius: f64,
    },
    /// `half_angle` in radians, between the axis and the wall.
    Cone {
        apex: [f64; 3],
        axis: [f64; 3],
        half_angle: f64,
    },
    Torus {
        centre: [f64; 3],
        axis: [f64; 3],
        major: f64,
        minor: f64,
    },
    Circle {
        centre: [f64; 3],
        normal: [f64; 3],
        radius: f64,
    },
    Line {
        point: [f64; 3],
        direction: [f64; 3],
    },
}

/// A least-squares fit and what it rests on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FitAnswer {
    pub shape: FittedShape,
    /// RMS distance of the points used from the shape.
    pub sigma: f64,
    /// The most negative and most positive signed distance of the points
    /// used: the form error is their difference.
    pub form: (f64, f64),
    /// Which points the fit used, one per point given.
    pub used: Vec<bool>,
    /// Each point's signed distance from the shape, used or not.
    pub residuals: Vec<f64>,
    pub converged: bool,
}

/// A region to grow over a mesh from points near it, and how.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrowRequest {
    /// The surface to grow: a plane, sphere, cylinder, cone or torus.
    pub kind: FitKind,
    /// Points on or near the mesh the region starts from.
    pub seeds: Vec<[f64; 3]>,
    /// Points farther from the fit than this many sigmas are left out and
    /// stop growth; the kernel's own gate when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sigma_cutoff: Option<f64>,
    /// The largest turn, in degrees, between a triangle's normal and the
    /// surface's for the triangle to join; the kernel's default when
    /// `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal_limit_deg: Option<f64>,
    /// Take off the rim while it turns away from the surface (a face's
    /// rounds).
    #[serde(default)]
    pub peel: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_triangles: Option<usize>,
}

/// A grown region and its fit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrowAnswer {
    /// The fit; its `used` and `residuals` go with `vertices`.
    pub fit: FitAnswer,
    /// The mesh's triangles in the region.
    pub triangles: Vec<u32>,
    /// The mesh vertex each of the fit's points is.
    pub vertices: Vec<u32>,
    /// Whether each seed's triangle ended up in the region.
    pub seeds_in_region: Vec<bool>,
}

/// Points to measure to a reference surface, and how.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DistanceRequest {
    pub points: Vec<[f64; 3]>,
    /// One per point, or none; needed for `facing_limit_deg`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub normals: Vec<[f64; 3]>,
    /// A point with nothing nearer reads unmeasured; any distance when
    /// `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Only surface turned within this many degrees of a point's own
    /// normal counts for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facing_limit_deg: Option<f64>,
}

/// Points to bring onto a reference surface by a rigid motion, and how.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub points: Vec<[f64; 3]>,
    /// One per point, or none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub normals: Vec<[f64; 3]>,
    /// Try every way of matching the points' principal axes with the
    /// reference's as well as the points where they are, and keep the best.
    #[serde(default)]
    pub search: bool,
    /// Pairs farther apart than this are not used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_distance: Option<f64>,
    /// Pair a point only with reference surface turned within this many
    /// degrees of its own normal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facing_limit_deg: Option<f64>,
}

/// A rigid motion that brings points onto a reference, and how well.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Registration {
    /// Row-major, the last row left out: the points' frame to where they
    /// fit.
    pub transform: [[f64; 4]; 3],
    /// RMS distance of the pairs kept, mm.
    pub rms: f64,
    pub inliers: usize,
    pub sampled: usize,
    pub converged: bool,
    /// Motions the fit cannot pin down (a cylinder turning about its axis,
    /// a plane sliding along itself), each a turn about `about` with a
    /// slide; empty when the fit is held every way.
    #[serde(default)]
    pub free: Vec<FreeMotion>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FreeMotion {
    pub about: [f64; 3],
    pub rotation: [f64; 3],
    pub translation: [f64; 3],
}

/// Points on a part whose wall thickness is read, and how.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ThicknessRequest {
    pub points: Vec<[f64; 3]>,
    /// Outward, one per point.
    pub normals: Vec<[f64; 3]>,
    /// The largest ball touching the point inside the material, in place
    /// of rays.
    #[serde(default)]
    pub sphere: bool,
    /// Rays spread within this many degrees of the inward normal, the
    /// shortest kept; along the normal alone at 0.
    #[serde(default)]
    pub cone_deg: f64,
    #[serde(default)]
    pub rays: u32,
    /// Surfaces within this many degrees of edge-on to a ray are passed
    /// over.
    #[serde(default)]
    pub facing_limit_deg: f64,
    /// A wall thicker than this reads unmeasured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

/// What distances are measured to.
#[derive(Debug, Clone, Copy)]
pub enum DistanceTarget<'a> {
    /// Triangles, closed or not.
    Mesh(&'a TriMesh),
    /// A native-format solid, meshed finely by curvature for the purpose.
    Shape(&'a [u8]),
}

/// A scan's proposed upright frame, expressed in its own coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Orientation {
    /// Bounding-box centre on the standing face.
    pub origin: [f64; 3],
    pub x_axis: [f64; 3],
    pub z_axis: [f64; 3],
    /// What supports the proposal, including a principal-axis fallback.
    pub said: String,
}

/// One connected polyline of a mesh's section by a plane, in the mesh's
/// own frame.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SectionChain {
    /// In order along the chain; a closed chain does not repeat its first
    /// point.
    pub points: Vec<[f64; 3]>,
    /// At each point, the unit direction in the plane pointing out of the
    /// material.
    pub outward: Vec<[f64; 3]>,
    pub closed: bool,
}

/// The shadow a mesh casts on a plane, and an outline fitted around it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowRequest {
    /// The plane, in the mesh's frame; the answer's points are along its
    /// `x_axis` and `y_axis` from its `origin`.
    pub plane: ProfilePlane,
    /// The outline's least clearance from the shadow, mm; negative inside
    /// it, a press fit.
    pub play: f64,
    /// Half the outline's band: every point of it lies between `play` and
    /// `play + 2 tolerance` outside the shadow.
    pub tolerance: f64,
    /// Corners turning less than this, degrees, take the largest tangent
    /// arc that fits the band.
    pub round_below_deg: f64,
    /// Holes and notches no wider across than this, mm, are filled before
    /// the outline is fitted (its inner corners rounded to half of it);
    /// 0 fills nothing.
    #[serde(default)]
    pub fill_below: f64,
}

/// What a mesh's shadow is: its loops as traced (outer boundaries
/// counter-clockwise, holes clockwise) and the outline of lines, arcs and
/// circles fitted around them.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ShadowAnswer {
    pub silhouette: Vec<Vec<[f64; 2]>>,
    pub outline: Vec<ProfileWire>,
}

/// How a chain of points is fitted with lines and arcs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChainFit {
    /// Every point lies within this distance of its piece, mm.
    pub tolerance: f64,
    /// Pieces turning against each other meet where their curves cross,
    /// bringing back a corner the points round off.
    pub corners: bool,
    /// Lines within this many degrees of the plane's axes are turned onto
    /// them where they still hold their points.
    pub axis_snap_deg: Option<f64>,
    /// An arc bowing from its chord by no more than this is taken as
    /// lines, mm.
    pub min_bow: f64,
    pub closed: bool,
}

/// A scan's mirror plane, expressed in its own coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymmetryPlane {
    pub point: [f64; 3],
    pub normal: [f64; 3],
    /// RMS mirror residual in millimetres.
    pub rms: f64,
}
