//! Questions asked of a built solid and their answers: projected edges,
//! sections, drawings, probes, holes, frames along paths and construction
//! geometry.

use serde::{Deserialize, Serialize};

use crate::*;

/// An edge of a solid projected onto a plane, in the plane's own 2D
/// coordinates (along its `x_axis` and `y_axis`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProjectedEdge {
    /// The edge runs along the plane's normal, so it projects to a point.
    Point([f64; 2]),
    /// A segment.
    Line { start: [f64; 2], end: [f64; 2] },
    /// A circle or circular arc, `centre + radius (cos t, sin t)`, turning
    /// counter-clockwise from `range.0` to `range.1`; a full circle when the
    /// range spans a turn.
    Circle {
        centre: [f64; 2],
        radius: f64,
        range: (f64, f64),
    },
    /// An ellipse or elliptical arc, `centre + major cos t + minor sin t`,
    /// `minor` the major axis turned a quarter counter-clockwise and scaled
    /// by `ratio`, from `range.0` to `range.1`.
    Ellipse {
        centre: [f64; 2],
        major: [f64; 2],
        ratio: f64,
        range: (f64, f64),
    },
    /// A piece of a parabola, `vertex + axis s² / (4 focal) + side s`,
    /// `axis` a unit vector the way it opens and `side` it turned a quarter
    /// counter-clockwise, from `s = range.0` to `range.1`.
    Parabola {
        vertex: [f64; 2],
        axis: [f64; 2],
        focal: f64,
        range: (f64, f64),
    },
    /// A piece of one branch of a hyperbola, `centre + major cosh s +
    /// minor sinh s`, `major` (centre to the branch's vertex) square to
    /// `minor`, from `s = range.0` to `range.1`.
    Hyperbola {
        centre: [f64; 2],
        major: [f64; 2],
        minor: [f64; 2],
        range: (f64, f64),
    },
    /// A B-spline, as the kernel projects it: `degree`, its clamped
    /// `knots` (`control_points + degree + 1` of them), and a weight per
    /// control point when rational (empty when every weight is one).
    Spline {
        degree: u32,
        knots: Vec<f64>,
        control_points: Vec<[f64; 2]>,
        weights: Vec<f64>,
    },
    /// Any other curve, as points along it: exact at each point, straight
    /// between.
    Polyline(Vec<[f64; 2]>),
}

/// One curve where a plane crosses a shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionCurve {
    /// The exact curve, in the plane's own 2D coordinates.
    pub curve: ProjectedEdge,
    /// Points along it, in the shape's own frame, in the order its run
    /// goes.
    pub points: Vec<[f64; 3]>,
    /// The run goes against the way `curve` runs.
    #[serde(default)]
    pub reversed: bool,
}

/// Curves where a plane crosses a shape, end to end: a closed loop round
/// a solid's cut, or an open run where a sheet's edge stops it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionRun {
    pub curves: Vec<SectionCurve>,
    pub closed: bool,
}

/// One edge of a face's boundary loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoopEdge {
    /// Points along it, in the shape's own frame, in the order the loop
    /// walks it.
    pub points: Vec<[f64; 3]>,
    /// The edge a round face meets itself along: it turns up twice in
    /// its loop, once each way.
    #[serde(default)]
    pub seam: bool,
}

/// The curves and texts of a 2D drawing file, in the drawing's own
/// coordinates.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Drawing2d {
    /// Millimetres per drawing unit, when the drawing names its unit.
    pub unit_mm: Option<f64>,
    /// Every curve, in the order the file has them.
    pub curves: Vec<DrawingCurve>,
    /// Every line of text, in the order the file has them.
    #[serde(default)]
    pub texts: Vec<DrawingText>,
}

/// One curve of a drawing, and whether the drawing marks it hidden
/// (a hidden layer, or a dashed line type).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawingCurve {
    pub hidden: bool,
    pub shape: DrawingShape,
    /// The layer it is drawn on; empty for the drawing's default layer.
    #[serde(default)]
    pub layer: String,
}

/// A line of text on a drawing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawingText {
    /// The start of its baseline.
    pub at: [f64; 2],
    /// The height of its capitals.
    pub height: f64,
    /// Its baseline's direction, radians counter-clockwise from x.
    pub angle: f64,
    pub text: String,
    /// The layer it is written on; empty for the drawing's default layer.
    #[serde(default)]
    pub layer: String,
}

/// The kinds of curve a drawing carries. Angles are radians, arcs run
/// counter-clockwise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DrawingShape {
    /// Points in order, each segment a line, or an arc where the bulge of
    /// the segment starting at that point (the tangent of a quarter of its
    /// included angle, positive counter-clockwise) is not zero. `bulges`
    /// is as long as `points`, or empty for straight segments only.
    Polyline {
        points: Vec<[f64; 2]>,
        bulges: Vec<f64>,
        closed: bool,
    },
    Arc {
        centre: [f64; 2],
        radius: f64,
        start_angle: f64,
        end_angle: f64,
    },
    Circle {
        centre: [f64; 2],
        radius: f64,
    },
    /// Points `centre + major cos t + ratio perp(major) sin t` for `t`
    /// from `start_param` to `end_param`; the whole ellipse when those span
    /// a full turn.
    Ellipse {
        centre: [f64; 2],
        major: [f64; 2],
        ratio: f64,
        start_param: f64,
        end_param: f64,
    },
    /// A spline, as points along the exact curve.
    Spline {
        points: Vec<[f64; 2]>,
        closed: bool,
    },
}

/// A connected volume where a scan and a solid differ the same way, in the
/// solid's frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VolumeRegion {
    /// In the solid and out of the scan; else in the scan and out of the
    /// solid.
    pub missing: bool,
    /// In mm³.
    pub volume: f64,
    /// How far apart the two surfaces stand across it, in mm.
    pub depth: f64,
    /// Wider than twice its depth: a face standing off, rather than a boss
    /// or a pocket.
    pub layer: bool,
    pub min: [f64; 3],
    pub max: [f64; 3],
    /// It rests on a hole of the scan, or is too shallow for its volume to
    /// be read.
    pub unsure: bool,
}

/// The solid two shapes share: its volume, its centre and its mesh, in
/// the first shape's frame.
#[derive(Debug, Clone)]
pub struct Overlap {
    pub volume_mm3: f64,
    pub centre_mm: [f64; 3],
    pub mesh: TriMesh,
}

/// How near two shapes come: the distance between their boundaries and a
/// nearest point on each, in the first shape's frame. Zero where the
/// boundaries touch or cross.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gap {
    pub distance_mm: f64,
    pub on_a: [f64; 3],
    pub on_b: [f64; 3],
}

/// A stretch of a region's medial axis, in the profile plane's own 2D
/// coordinates: points along one branch with the clearance at each, the
/// radius of the largest disc centred there that stays inside the region.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MedialPath {
    pub points: Vec<[f64; 2]>,
    /// One per point, in millimetres.
    pub clearance: Vec<f64>,
    /// Whether the first and the last point end the axis on the boundary
    /// (a corner, or a rounded end's centre) rather than meet other
    /// branches: towards such an end the clearance falls without the
    /// region getting thinner.
    pub boundary_ends: [bool; 2],
}

/// Where a region is narrowest, and how narrow: the smallest clearance on
/// its medial axis away from the branch ends at its corners (where the
/// clearance runs down to nothing without the region getting any thinner).
/// The wall there is twice the clearance.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Narrowest {
    pub at: [f64; 2],
    pub clearance: f64,
}

/// The medial axis of one region of a profile: an outer wire and its holes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MedialRegion {
    /// The profile's wires that bound it, by index, the outer one first.
    pub wires: Vec<usize>,
    pub paths: Vec<MedialPath>,
    /// `None` only when the axis has no place away from the corners.
    pub narrowest: Option<Narrowest>,
}

/// A picked face: its name, and a point on it with its outward normal.
/// A rebuild finds the face by its name, the point choosing among the
/// pieces of a face that split; without a name (or when no face bears it
/// any more), the face nearest the point.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct FaceProbe {
    pub point: [f64; 3],
    pub normal: [f64; 3],
    #[serde(default)]
    pub name: TopoName,
}

/// A question about a solid a reference stands on, named geometrically so
/// it can be asked again of the solid a rebuild makes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ShapeProbe {
    /// The face named `name`, else the face nearest `point` (among faces
    /// as near, the one whose outward normal agrees best with `normal`),
    /// and where on it `point` falls.
    Face {
        point: [f64; 3],
        normal: [f64; 3],
        #[serde(default)]
        name: TopoName,
    },
    /// The edge between faces named `faces`, else the edge nearest `point`
    /// that runs along `direction` there (a zero direction takes the
    /// nearest edge whichever way it runs).
    Edge {
        point: [f64; 3],
        direction: [f64; 3],
        #[serde(default)]
        faces: [TopoName; 2],
        /// A share along the edge from its start (0 to 1) whose point and
        /// tangent the answer also gives.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        along: Option<f64>,
    },
    /// The solid's centre of mass and principal axes of inertia.
    Mass,
}

/// What a solid answers to a [`ShapeProbe`], in the solid's own frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ProbeAnswer {
    /// The point of the face nearest the probe's, the face's outward normal
    /// there, and the face's surface.
    Face {
        point: [f64; 3],
        normal: [f64; 3],
        surface: FaceSurface,
    },
    /// The point of the edge nearest the probe's and the way the edge runs
    /// there, its two ends and its middle (halfway along its parameter,
    /// which is halfway along an arc), and the circle it runs round when
    /// it is a circle or an arc of one.
    Edge {
        point: [f64; 3],
        direction: [f64; 3],
        start: [f64; 3],
        end: [f64; 3],
        middle: [f64; 3],
        circle: Option<ProbedCircle>,
        /// The point and tangent the probe's share along gives, when it
        /// asked for one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        along: Option<([f64; 3], [f64; 3])>,
    },
    /// The centre of mass and the principal axes of inertia, the axis of
    /// the smallest moment first.
    Mass {
        centre: [f64; 3],
        axes: [[f64; 3]; 3],
    },
}

/// The circle an edge runs round.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProbedCircle {
    pub centre: [f64; 3],
    pub normal: [f64; 3],
    pub radius: f64,
}

/// A probe asked part way through a solid-op chain: of the solid the first
/// `after_op` ops make.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChainProbe {
    pub after_op: usize,
    pub probe: ShapeProbe,
}

/// A solid's edge projected onto a plane, or the curves where one of its
/// faces crosses the plane: what a sketch takes from the solid it stands
/// on, asked again of the solid a rebuild makes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectionProbe {
    /// A point on the edge (on the face, for a section), in the solid's
    /// own frame.
    pub point: [f64; 3],
    /// The names of the two faces the edge runs between: the edge is found
    /// by them first, `point` choosing among its pieces, and by `point`
    /// alone without them or when no edge runs between them.
    #[serde(default)]
    pub faces: [TopoName; 2],
    /// The face nearest `point` is cut by the plane, in place of an edge
    /// projected onto it.
    #[serde(default)]
    pub section: bool,
    /// The plane, in the solid's own frame: the curves come in its own 2D
    /// coordinates.
    pub plane: ProfilePlane,
}

/// A projection asked part way through a solid-op chain: of the solid the
/// first `after_op` ops make.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChainProjection {
    pub after_op: usize,
    pub probe: ProjectionProbe,
}

/// The centre line of a pipe-like solid between two of its faces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CentreLine {
    /// Points along it, from the first face to the second, in the shape's
    /// frame.
    pub points: Vec<[f64; 3]>,
    /// Its length, in millimetres.
    pub length: f64,
    /// The largest distance measured from the centroid of a section cut
    /// square to the line to the line itself.
    pub deviation: f64,
    /// It is one straight segment.
    pub straight: bool,
}

/// A round hole found in a solid: a full bore, open at one end (blind)
/// or both (through), closed by a flat bottom or a drill point.
#[derive(Debug, Clone, PartialEq)]
pub struct RecognizedHole {
    /// The centre of its opening, on its axis.
    pub entry: [f64; 3],
    /// Along its axis, into the material.
    pub direction: [f64; 3],
    pub diameter: f64,
    /// The bore's length from the opening, not counting a drill point.
    pub depth: f64,
    /// Open at both ends.
    pub through: bool,
    /// The included angle of the cone that ends a blind hole, degrees;
    /// `None` for a flat bottom or a through hole.
    pub drill_point_deg: Option<f64>,
    /// A point on each face the hole is made of (its bore, its bottom),
    /// with that face's outward normal.
    pub faces: Vec<([f64; 3], [f64; 3])>,
}

/// Geometry questions a workbench may ask while it runs, answered by the
/// kernel at once. Shapes arrive as the snapshot bytes the document keeps.
/// How two faces of a shape meet across an edge they share: a point
/// halfway along it, in the shape's own frame, the largest gap between the
/// edge and the faces, the largest angle between the faces' normals along
/// it (zero where they meet tangent), and the largest difference in how
/// sharply they bend square to it (1/mm; zero where they meet curvature
/// continuous, `None` where it could not be read).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EdgeContinuity {
    pub point: [f64; 3],
    pub gap: f64,
    pub angle_deg: f64,
    #[serde(default)]
    pub curvature: Option<f64>,
}

/// A face of a shape, named by a point inside it, and points on it, each
/// with the normal its curvature is signed against.
pub type FacePoints = ([f64; 3], Vec<([f64; 3], [f64; 3])>);

/// A point on a path and the way the path runs there.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PathFrame {
    pub point: [f64; 3],
    /// A unit vector along the path, the way it is walked.
    pub tangent: [f64; 3],
}

/// Stations along a path, by arc length: what is repeated along a curve
/// stands at them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathFrames {
    /// The path's whole length, mm.
    pub length: f64,
    /// Whether the path ends where it starts.
    pub closed: bool,
    /// One per station asked for, in the order asked.
    pub frames: Vec<PathFrame>,
}

/// Reference geometry with no topology: what STEP and IGES files carry as
/// construction geometry beside (or instead of) a part's solid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConstructionShape {
    Point([f64; 3]),
    /// An axis placement: `z` its axis, `x` its reference direction.
    Axis {
        origin: [f64; 3],
        z: [f64; 3],
        x: [f64; 3],
    },
    /// Straight pieces through the points; closed back to the first.
    Polyline {
        points: Vec<[f64; 3]>,
        closed: bool,
    },
    Circle {
        centre: [f64; 3],
        normal: [f64; 3],
        radius: f64,
    },
    /// A plane over `u` along `x_axis` and `v` along `normal × x_axis`
    /// from `origin`.
    Plane {
        origin: [f64; 3],
        normal: [f64; 3],
        x_axis: [f64; 3],
        u: [f64; 2],
        v: [f64; 2],
    },
}

/// A named item of construction geometry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstructionItem {
    pub name: String,
    pub shape: ConstructionShape,
}

/// Construction geometry written as IGES, with any omitted geometry or names reported.
#[derive(Debug, Clone, Default)]
pub struct IgesExport {
    pub text: String,
    pub warnings: Vec<String>,
}
