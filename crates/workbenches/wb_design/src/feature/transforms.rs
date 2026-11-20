//! The settings of scaling, mirrors and patterns: their centres, planes,
//! axes, paths and fills.

use super::*;

/// The point a Scale scales about, in the body's own frame.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Walk)]
pub enum ScaleCentre {
    /// The body's origin.
    #[default]
    Origin,
    /// The middle of the solid's bounding box where the scale stands.
    BodyCentre,
    /// A datum's origin: a datum point, or a coordinate system's origin.
    Datum { datum: FeatureId },
}

impl ScaleCentre {
    pub fn label(&self) -> &'static str {
        match self {
            ScaleCentre::Origin => "Origin",
            ScaleCentre::BodyCentre => "The body's centre",
            ScaleCentre::Datum { .. } => "Datum",
        }
    }
}

pub(super) fn unit_factors() -> [f32; 3] {
    [1.0; 3]
}

/// A mirror/pattern reference plane.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Walk)]
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
        #[walk(setting)]
        plane: Option<core_document::BasePlane>,
    },
    /// A sketch's plane, or the plane through one of its axes square to
    /// it.
    Sketch {
        sketch: FeatureId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[walk(setting)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Describe)]
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
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Walk)]
pub enum PatternAxis {
    X,
    #[default]
    Y,
    Z,
    Custom {
        #[walk(setting)]
        origin: [f32; 3],
        #[walk(setting)]
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
        #[walk(setting)]
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
}

/// One step of a multi-transform (each applies to every result of the
/// previous step).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Walk)]
pub enum TransformStep {
    Linear {
        axis: PatternAxis,
        #[walk(length("length", "Length"))]
        length: f32,
        #[walk(count("occurrences", "Occurrences"))]
        occurrences: u32,
    },
    Polar {
        axis: PatternAxis,
        #[walk(angle("angle", "Angle"))]
        angle_deg: f32,
        #[walk(count("occurrences", "Occurrences"))]
        occurrences: u32,
    },
    Mirror {
        plane: MirrorPlane,
    },
    /// Scaled about `center`, in the body's own frame: its origin when
    /// none is given.
    Scale {
        #[walk(number("factor", "Factor"))]
        factor: f32,
        #[serde(default)]
        #[walk(setting)]
        center: [f32; 3],
        #[walk(count("occurrences", "Occurrences"))]
        occurrences: u32,
    },
    /// Rings about `axis`: ring `i` moved `i × spacing` along `outward`,
    /// then spread evenly round the axis, `counts[i]` occurrences in it,
    /// the original first in the first ring.
    Circular {
        axis: PatternAxis,
        outward: PatternAxis,
        #[walk(length("ring_spacing", "Ring spacing"))]
        spacing: f32,
        #[walk(setting)]
        counts: Vec<u32>,
    },
    /// An occurrence at every point of `sketch` after its first, moved by
    /// how far that point is from the first, where the original stands.
    Points {
        sketch: FeatureId,
    },
    /// Occurrences along a path, as a path pattern places them.
    Path {
        path: PatternPath,
        #[serde(flatten)]
        layout: PathSpacing,
    },
}

/// What a path pattern runs along, in its body's own frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Walk)]
pub enum PatternPath {
    /// One of a sketch's chains of curves (its construction left out), by
    /// its place among them, walked from its first curve's start.
    Sketch {
        sketch: FeatureId,
        #[serde(default)]
        #[walk(setting)]
        chain: usize,
    },
    /// An edge of the solid, found again on the solid as it stands where
    /// the pattern does, walked from its curve's start.
    Edge(EdgePick),
}

impl PatternPath {
    /// The feature the path follows, when it is a sketch's.
    pub fn reference(&self) -> Option<FeatureId> {
        match self {
            PatternPath::Sketch { sketch, .. } => Some(*sketch),
            PatternPath::Edge(_) => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            PatternPath::Sketch { .. } => "Sketch",
            PatternPath::Edge(_) => "Picked edge",
        }
    }
}

/// How a path pattern's occurrences are spaced along its path, its fields
/// written beside the pattern's own.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Walk)]
#[serde(default)]
pub struct PathSpacing {
    /// Occurrences, the original counted.
    #[walk(count("occurrences", "Occurrences"))]
    pub occurrences: u32,
    /// `spacing` is the distance between neighbours along the path; off,
    /// the occurrences spread over its whole length (round it evenly when
    /// it closes).
    #[walk(setting)]
    pub spacing_mode: bool,
    /// Millimetres between neighbours along the path, with `spacing_mode`.
    #[walk(length("spacing", "Spacing"))]
    pub spacing: f32,
    /// Walk the path from its end.
    #[walk(setting)]
    pub reversed: bool,
    /// Each copy turns as the path does from its start to the copy's place.
    #[walk(setting)]
    pub align: bool,
}

impl Default for PathSpacing {
    fn default() -> Self {
        Self {
            occurrences: 4,
            spacing_mode: false,
            spacing: 10.0,
            reversed: false,
            align: false,
        }
    }
}

/// What a fill pattern covers, in its body's own frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Walk)]
pub enum FillArea {
    /// The area a sketch's curves close, or the regions of it named; its
    /// lattice runs along the sketch's x axis.
    Sketch {
        sketch: FeatureId,
        #[serde(default)]
        #[walk(setting)]
        regions: Vec<String>,
    },
    /// A flat face of the solid, found again where the pattern stands:
    /// the area its outer edge closes. Its lattice runs along the body's
    /// x axis laid on the face (its y axis on a face square to x).
    Face(FacePick),
}

impl FillArea {
    pub fn label(&self) -> &'static str {
        match self {
            FillArea::Sketch { .. } => "Sketch",
            FillArea::Face(_) => "Picked face",
        }
    }
}

/// How a fill pattern's places repeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Describe)]
pub enum FillKind {
    /// Rows of places, `spacing` apart along a row and `row_spacing` from
    /// row to row.
    #[default]
    Grid,
    /// Each place `spacing` from its six neighbours, every other row moved
    /// half a spacing along.
    Honeycomb,
}

impl FillKind {
    pub const ALL: [FillKind; 2] = [FillKind::Grid, FillKind::Honeycomb];

    pub fn label(&self) -> &'static str {
        match self {
            FillKind::Grid => "Grid",
            FillKind::Honeycomb => "Honeycomb",
        }
    }
}

pub(super) fn default_fill_rows() -> f32 {
    10.0
}
