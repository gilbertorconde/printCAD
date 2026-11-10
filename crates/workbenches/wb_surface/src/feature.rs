//! The Surface workbench's features, as the document keeps them: what each
//! step is built from (sketches by id, edges and faces of the body as picks
//! in its own frame, edges of other bodies as picks in theirs) and its
//! settings.

use core_document::{BodyId, FeatureId, WorkbenchFeature, WorkbenchId};
use kernel_api::{Continuity, TopoName};
use serde::{Deserialize, Serialize};

/// The kind every surface feature carries.
pub const KIND: &str = "wb.surface";

/// A curve a step is built from: every chain of a sketch, an edge of the
/// body, or an edge of another body, picked in that body's frame and
/// carried to where it sits.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CurveRef {
    Sketch(FeatureId),
    Edge(EdgePick),
    BodyEdge { body: BodyId, edge: EdgePick },
}

/// An edge of the body, as a point beside it and the way it runs there, in
/// the body's own frame, with the names of the faces it runs between.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EdgePick {
    pub point: [f32; 3],
    pub direction: [f32; 3],
    #[serde(
        default,
        deserialize_with = "kernel_api::naming::names_from_numbers_or_text"
    )]
    pub faces: [TopoName; 2],
    /// Its length when picked, to name it by.
    #[serde(default)]
    pub length: f32,
}

/// A face of the body, as a point on it and its normal there, in the
/// body's own frame, with its name.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FacePick {
    pub point: [f32; 3],
    /// Its normal where picked; a face is found by its name and point, so
    /// a curved face, which has no one normal, may leave it out.
    #[serde(default)]
    pub normal: [f32; 3],
    #[serde(
        default,
        deserialize_with = "kernel_api::naming::name_from_number_or_text"
    )]
    pub name: TopoName,
}

/// Which way an extrusion runs.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum Direction {
    /// Square to the first sketch's plane.
    #[default]
    SketchNormal,
    X,
    Y,
    Z,
    Custom([f32; 3]),
}

/// The axis a revolution turns about.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum Axis {
    /// The first sketch's vertical axis, through its origin.
    #[default]
    SketchVertical,
    /// The first sketch's horizontal axis, through its origin.
    SketchHorizontal,
    X,
    Y,
    Z,
    Custom {
        origin: [f32; 3],
        direction: [f32; 3],
    },
}

/// A plane a mirror reflects in or a trim cuts by, in the body's frame.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum PlaneRef {
    #[default]
    YZ,
    XZ,
    XY,
    Custom {
        origin: [f32; 3],
        normal: [f32; 3],
    },
}

/// A sheet fitted to a scan, in the body's frame: what its points cover,
/// before it is extended.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum FitSheet {
    /// A plane through `origin` with `normal`, over `across` along `u`
    /// and along `normal × u`.
    Plane {
        origin: [f64; 3],
        normal: [f64; 3],
        u: [f64; 3],
        across: [[f64; 2]; 2],
    },
    /// A cylinder about the line through `point` along `axis`, over
    /// `along` of it.
    Cylinder {
        point: [f64; 3],
        axis: [f64; 3],
        radius: f64,
        along: [f64; 2],
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SurfaceFeature {
    Extrude {
        curves: Vec<CurveRef>,
        #[serde(default)]
        direction: Direction,
        length: f32,
        #[serde(default)]
        symmetric: bool,
        #[serde(default)]
        reversed: bool,
    },
    Revolve {
        curves: Vec<CurveRef>,
        #[serde(default)]
        axis: Axis,
        angle_deg: f32,
    },
    PlanarFill {
        curves: Vec<CurveRef>,
    },
    Fill {
        boundary: Vec<CurveRef>,
        #[serde(default)]
        continuity: Continuity,
    },
    Ruled {
        first: Option<CurveRef>,
        second: Option<CurveRef>,
    },
    Loft {
        sections: Vec<CurveRef>,
        #[serde(default)]
        closed: bool,
        /// Curves the surface follows between the sections, each crossing
        /// every section once.
        #[serde(default)]
        guides: Vec<CurveRef>,
    },
    Sweep {
        profile: Vec<CurveRef>,
        path: Vec<CurveRef>,
        /// A second rail: the profile runs from the path to it and is
        /// swept along both, scaled to the width between them.
        #[serde(default)]
        second_rail: Vec<CurveRef>,
    },
    Offset {
        faces: Vec<FacePick>,
        distance: f32,
    },
    Extend {
        edges: Vec<EdgePick>,
        length: f32,
        #[serde(default)]
        continuity: Continuity,
    },
    Blend {
        first: Option<EdgePick>,
        second: Option<EdgePick>,
        #[serde(default)]
        continuity: Continuity,
    },
    Split {
        faces: Vec<FacePick>,
        curves: Vec<CurveRef>,
    },
    Sew {
        /// Edges this far apart (mm) are joined too; 0 joins only edges
        /// that meet.
        #[serde(default)]
        gap: f32,
    },
    /// A round of `radius` along picked edges where two faces of the
    /// body's sheets meet.
    Fillet {
        edges: Vec<EdgePick>,
        radius: f32,
    },
    /// A round of `radius` between two faces of the body that need share
    /// no edge, each cut back to where the round meets it. The ball rolls
    /// on the side a face's normal points to, the other when flipped.
    FilletFaces {
        first: Option<FacePick>,
        second: Option<FacePick>,
        radius: f32,
        #[serde(default)]
        flip_first: bool,
        #[serde(default)]
        flip_second: bool,
    },
    Thicken {
        thickness: f32,
        #[serde(default)]
        both_sides: bool,
    },
    Trim {
        #[serde(default)]
        plane: PlaneRef,
        #[serde(default)]
        offset: f32,
        /// Keep the side the plane's normal points away from.
        #[serde(default)]
        flip: bool,
    },
    Mirror {
        #[serde(default)]
        plane: PlaneRef,
        #[serde(default)]
        offset: f32,
    },
    /// The body's sheets cut by each other and kept where they lie on
    /// `scan`, sewn into a shell, a closed one a solid.
    TrimScan {
        #[serde(default)]
        scan: Option<BodyId>,
        /// How near the scan a face must lie, mm; three times the largest
        /// fit's σ (at least 0.05) when `None`.
        #[serde(default)]
        tolerance: Option<f32>,
        /// How much of a face's area must lie that near, from 0 to 1.
        #[serde(default = "half")]
        share: f32,
    },
    /// A network of B-spline patches fitted to a scan at once, each
    /// bounded by curves of the network and meeting its neighbours across
    /// them without a gap, tangent unless the curve is a crease.
    FitNetwork {
        /// The network's first curves: each sketch's chains, then edges.
        #[serde(default)]
        curves: Vec<CurveRef>,
        /// Curves after those, as points along each in the body's frame.
        #[serde(default)]
        polylines: Vec<Vec<[f32; 3]>>,
        /// Each patch's sides in order round it, as curve numbers from 1,
        /// negative where the curve runs against the walk, 0 for a side
        /// shrunk to a point.
        #[serde(default)]
        patches: Vec<[i32; 4]>,
        /// Additional patches, each side a chain of nonzero signed curve
        /// numbers in walking order, or empty for a side shrunk to a point.
        #[serde(default)]
        chained: Vec<[Vec<i32>; 4]>,
        /// Parameter-correction passes after refinement.
        #[serde(default)]
        corrections: u32,
        /// The scan it is fitted to.
        #[serde(default)]
        scan: Option<BodyId>,
        #[serde(default = "network_tolerance")]
        tolerance: f32,
        /// The fairing weight that settles what the points leave free.
        #[serde(default = "network_smoothing")]
        smoothing: f32,
        /// Curves, numbered from 1, the patches meet across creased.
        #[serde(default)]
        creases: Vec<u32>,
    },
    FitScanFree {
        #[serde(default)]
        points: Vec<[f32; 3]>,
        /// The scan it was read from.
        #[serde(default)]
        scan: Option<BodyId>,
        /// How near the points the sheet keeps, mm.
        #[serde(default = "free_tolerance")]
        tolerance: f32,
        /// The region spread across triangles turning less than this,
        /// degrees.
        #[serde(default = "free_turn")]
        turn_deg: f32,
    },
    /// A plane or cylinder fitted to a region of a scan, extended past
    /// the region by `extend` of its size so neighbouring fits cross.
    FitScan {
        #[serde(default)]
        sheet: Option<FitSheet>,
        /// The scan it was fitted on.
        #[serde(default)]
        scan: Option<BodyId>,
        /// The fit's RMS distance from the scan's points, mm.
        #[serde(default)]
        sigma: f32,
        #[serde(default = "default_extend")]
        extend: f32,
    },
}

fn default_extend() -> f32 {
    0.35
}

fn half() -> f32 {
    0.5
}

fn free_tolerance() -> f32 {
    0.05
}

fn free_turn() -> f32 {
    8.0
}

fn network_tolerance() -> f32 {
    0.05
}

fn network_smoothing() -> f32 {
    1e-4
}

/// What each kind of step is, for the toolbar, the tree and the commands:
/// its tool id, its name, its icon and whether the kernel builds it yet.
pub struct Kind {
    pub tool: &'static str,
    pub label: &'static str,
    /// What the step's command does, in a line.
    pub summary: &'static str,
    pub icon: &'static str,
    /// What the tool does once the kernel can: `None` when it builds now.
    pub waits: Option<&'static str>,
}

pub const KINDS: &[Kind] = &[
    Kind {
        tool: "surface.extrude",
        label: "Extruded surface",
        summary: "Extrude curves into a surface",
        icon: "surface-extrude",
        waits: None,
    },
    Kind {
        tool: "surface.revolve",
        label: "Revolved surface",
        summary: "Revolve curves about an axis into a surface",
        icon: "surface-revolve",
        waits: None,
    },
    Kind {
        tool: "surface.planar",
        label: "Planar surface",
        summary: "Fill closed flat loops with a planar surface",
        icon: "surface-planar",
        waits: None,
    },
    Kind {
        tool: "surface.fill",
        label: "Filling",
        summary: "Fill the hole curves close with a surface",
        icon: "surface-fill",
        waits: None,
    },
    Kind {
        tool: "surface.ruled",
        label: "Ruled surface",
        summary: "Span two curves with straight lines",
        icon: "surface-ruled",
        waits: None,
    },
    Kind {
        tool: "surface.loft",
        label: "Lofted surface",
        summary: "Loft a surface through sections in order",
        icon: "surface-loft",
        waits: None,
    },
    Kind {
        tool: "surface.sweep",
        label: "Swept surface",
        summary: "Sweep a profile along a path into a surface",
        icon: "surface-sweep",
        waits: None,
    },
    Kind {
        tool: "surface.offset",
        label: "Offset surface",
        summary: "Copy faces at a distance along their normals",
        icon: "surface-offset",
        waits: None,
    },
    Kind {
        tool: "surface.extend",
        label: "Extend surface",
        summary: "Extend faces past picked edges",
        icon: "surface-extend",
        waits: None,
    },
    Kind {
        tool: "surface.blend",
        label: "Blend surface",
        summary: "Bridge two edges with a surface",
        icon: "surface-blend",
        waits: None,
    },
    Kind {
        tool: "surface.split",
        label: "Split surface",
        summary: "Split faces along curves",
        icon: "surface-split",
        waits: None,
    },
    Kind {
        tool: "surface.sew",
        label: "Sew",
        summary: "Sew the body's surfaces together",
        icon: "surface-sew",
        waits: None,
    },
    Kind {
        tool: "surface.fillet",
        label: "Surface fillet",
        summary: "Round edges where two faces of a surface meet",
        icon: "surface-fillet",
        waits: None,
    },
    Kind {
        tool: "surface.fillet_faces",
        label: "Fillet between surfaces",
        summary: "Round between two faces that share no edge",
        icon: "fillet",
        waits: None,
    },
    Kind {
        tool: "surface.thicken",
        label: "Thicken",
        summary: "Thicken the body's surfaces into solids",
        icon: "surface-thicken",
        waits: None,
    },
    Kind {
        tool: "surface.trim",
        label: "Trim by plane",
        summary: "Keep what of the body lies on one side of a plane",
        icon: "surface-trim",
        waits: None,
    },
    Kind {
        tool: "surface.mirror",
        label: "Mirror",
        summary: "Add the body's reflection in a plane",
        icon: "surface-mirror",
        waits: None,
    },
    Kind {
        tool: "surface.fit_scan",
        label: "Fitted surface",
        summary: "Fit a plane or cylinder to a region of a scan",
        icon: "surface-planar",
        waits: None,
    },
    Kind {
        tool: "surface.trim_scan",
        label: "Trim by the scan",
        summary: "Keep the pieces of the surfaces that lie on a scan",
        icon: "surface-trim",
        waits: None,
    },
    Kind {
        tool: "surface.fit_scan_spline",
        label: "Fitted free-form surface",
        summary: "Fit a B-spline surface to a region of a scan of any shape",
        icon: "surface-fill",
        waits: None,
    },
    Kind {
        tool: "surface.fit_network",
        label: "Patch network",
        summary: "Fit a network of tangent patches to a scan",
        icon: "surface-fill",
        waits: None,
    },
];

impl SurfaceFeature {
    /// The step a tool makes, with nothing picked yet.
    pub fn for_tool(tool: &str) -> Option<Self> {
        Some(match tool {
            "surface.extrude" => SurfaceFeature::Extrude {
                curves: Vec::new(),
                direction: Direction::SketchNormal,
                length: 10.0,
                symmetric: false,
                reversed: false,
            },
            "surface.revolve" => SurfaceFeature::Revolve {
                curves: Vec::new(),
                axis: Axis::SketchVertical,
                angle_deg: 360.0,
            },
            "surface.planar" => SurfaceFeature::PlanarFill { curves: Vec::new() },
            "surface.fill" => SurfaceFeature::Fill {
                boundary: Vec::new(),
                continuity: Continuity::G0,
            },
            "surface.ruled" => SurfaceFeature::Ruled {
                first: None,
                second: None,
            },
            "surface.loft" => SurfaceFeature::Loft {
                sections: Vec::new(),
                closed: false,
                guides: Vec::new(),
            },
            "surface.sweep" => SurfaceFeature::Sweep {
                profile: Vec::new(),
                path: Vec::new(),
                second_rail: Vec::new(),
            },
            "surface.offset" => SurfaceFeature::Offset {
                faces: Vec::new(),
                distance: 1.0,
            },
            "surface.extend" => SurfaceFeature::Extend {
                edges: Vec::new(),
                length: 5.0,
                continuity: Continuity::G1,
            },
            "surface.blend" => SurfaceFeature::Blend {
                first: None,
                second: None,
                continuity: Continuity::G1,
            },
            "surface.split" => SurfaceFeature::Split {
                faces: Vec::new(),
                curves: Vec::new(),
            },
            "surface.sew" => SurfaceFeature::Sew { gap: 0.0 },
            "surface.fillet" => SurfaceFeature::Fillet {
                edges: Vec::new(),
                radius: 2.0,
            },
            "surface.fillet_faces" => SurfaceFeature::FilletFaces {
                first: None,
                second: None,
                radius: 2.0,
                flip_first: false,
                flip_second: false,
            },
            "surface.thicken" => SurfaceFeature::Thicken {
                thickness: 2.0,
                both_sides: false,
            },
            "surface.trim" => SurfaceFeature::Trim {
                plane: PlaneRef::YZ,
                offset: 0.0,
                flip: false,
            },
            "surface.mirror" => SurfaceFeature::Mirror {
                plane: PlaneRef::YZ,
                offset: 0.0,
            },
            "surface.fit_network" => SurfaceFeature::FitNetwork {
                curves: Vec::new(),
                polylines: Vec::new(),
                patches: Vec::new(),
                chained: Vec::new(),
                corrections: 0,
                scan: None,
                tolerance: network_tolerance(),
                smoothing: network_smoothing(),
                creases: Vec::new(),
            },
            "surface.fit_scan_spline" => SurfaceFeature::FitScanFree {
                points: Vec::new(),
                scan: None,
                tolerance: free_tolerance(),
                turn_deg: free_turn(),
            },
            "surface.trim_scan" => SurfaceFeature::TrimScan {
                scan: None,
                tolerance: None,
                share: half(),
            },
            "surface.fit_scan" => SurfaceFeature::FitScan {
                sheet: None,
                scan: None,
                sigma: 0.0,
                extend: default_extend(),
            },
            _ => return None,
        })
    }

    /// The tool that makes this kind of step.
    pub fn tool(&self) -> &'static str {
        match self {
            SurfaceFeature::Extrude { .. } => "surface.extrude",
            SurfaceFeature::Revolve { .. } => "surface.revolve",
            SurfaceFeature::PlanarFill { .. } => "surface.planar",
            SurfaceFeature::Fill { .. } => "surface.fill",
            SurfaceFeature::Ruled { .. } => "surface.ruled",
            SurfaceFeature::Loft { .. } => "surface.loft",
            SurfaceFeature::Sweep { .. } => "surface.sweep",
            SurfaceFeature::Offset { .. } => "surface.offset",
            SurfaceFeature::Extend { .. } => "surface.extend",
            SurfaceFeature::Blend { .. } => "surface.blend",
            SurfaceFeature::Split { .. } => "surface.split",
            SurfaceFeature::Sew { .. } => "surface.sew",
            SurfaceFeature::Fillet { .. } => "surface.fillet",
            SurfaceFeature::FilletFaces { .. } => "surface.fillet_faces",
            SurfaceFeature::Thicken { .. } => "surface.thicken",
            SurfaceFeature::Trim { .. } => "surface.trim",
            SurfaceFeature::Mirror { .. } => "surface.mirror",
            SurfaceFeature::FitScan { .. } => "surface.fit_scan",
            SurfaceFeature::TrimScan { .. } => "surface.trim_scan",
            SurfaceFeature::FitScanFree { .. } => "surface.fit_scan_spline",
            SurfaceFeature::FitNetwork { .. } => "surface.fit_network",
        }
    }

    pub fn kind(&self) -> &'static Kind {
        let tool = self.tool();
        KINDS
            .iter()
            .find(|k| k.tool == tool)
            .expect("every step has a kind")
    }

    /// Every curve the step reads, in order.
    pub fn curves(&self) -> Vec<CurveRef> {
        match self {
            SurfaceFeature::Extrude { curves, .. }
            | SurfaceFeature::Revolve { curves, .. }
            | SurfaceFeature::PlanarFill { curves }
            | SurfaceFeature::Split { curves, .. } => curves.clone(),
            SurfaceFeature::Fill { boundary, .. } => boundary.clone(),
            SurfaceFeature::Loft {
                sections, guides, ..
            } => sections.iter().chain(guides).copied().collect(),
            SurfaceFeature::Ruled { first, second } => {
                first.iter().chain(second).copied().collect()
            }
            SurfaceFeature::Sweep {
                profile,
                path,
                second_rail,
            } => profile
                .iter()
                .chain(path)
                .chain(second_rail)
                .copied()
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The other bodies whose edges the step reads.
    pub fn edge_bodies(&self) -> Vec<BodyId> {
        let mut out = Vec::new();
        for curve in self.curves() {
            if let CurveRef::BodyEdge { body, .. } = curve
                && !out.contains(&body)
            {
                out.push(body);
            }
        }
        out
    }

    /// The sketches the step reads.
    pub fn sketches(&self) -> Vec<FeatureId> {
        let mut out = Vec::new();
        for curve in self.curves() {
            if let CurveRef::Sketch(id) = curve
                && !out.contains(&id)
            {
                out.push(id);
            }
        }
        out
    }

    /// Whether the step is built from curves, so a tool makes it only with
    /// one picked.
    pub fn curves_needed(&self) -> bool {
        matches!(
            self,
            SurfaceFeature::Extrude { .. }
                | SurfaceFeature::Revolve { .. }
                | SurfaceFeature::PlanarFill { .. }
                | SurfaceFeature::Fill { .. }
                | SurfaceFeature::Ruled { .. }
                | SurfaceFeature::Loft { .. }
                | SurfaceFeature::Sweep { .. }
        )
    }

    /// Take out the curves the step is built from, for new ones to go in.
    pub fn clear_curves(&mut self) {
        match self {
            Self::Extrude { curves, .. }
            | Self::Revolve { curves, .. }
            | Self::PlanarFill { curves }
            | Self::Split { curves, .. } => curves.clear(),
            Self::Fill { boundary, .. } => boundary.clear(),
            Self::Loft {
                sections, guides, ..
            } => {
                sections.clear();
                guides.clear();
            }
            Self::Ruled { first, second } => {
                *first = None;
                *second = None;
            }
            Self::Sweep {
                profile,
                path,
                second_rail,
            } => {
                profile.clear();
                path.clear();
                second_rail.clear();
            }
            Self::Offset { .. }
            | Self::Extend { .. }
            | Self::Blend { .. }
            | Self::Sew { .. }
            | Self::Fillet { .. }
            | Self::FilletFaces { .. }
            | Self::Thicken { .. }
            | Self::Trim { .. }
            | Self::Mirror { .. }
            | Self::FitScan { .. }
            | Self::FitScanFree { .. }
            | Self::TrimScan { .. } => {}
            Self::FitNetwork {
                curves, polylines, ..
            } => {
                curves.clear();
                polylines.clear();
            }
        }
    }

    /// What the step still needs picked before it can build, in the words
    /// of the command field that gives it.
    pub fn missing(&self) -> Option<&'static str> {
        match self {
            Self::Extrude { curves, .. }
            | Self::Revolve { curves, .. }
            | Self::PlanarFill { curves } => curves
                .is_empty()
                .then_some("`sketches`: the curves to build from"),
            Self::Fill { boundary, .. } => boundary
                .is_empty()
                .then_some("`sketches` or `boundary`: the curves that close the hole"),
            Self::Ruled { first, second } => (first.is_none() || second.is_none())
                .then_some("`sketches`: two curves, the first and the second"),
            Self::Loft { sections, .. } => {
                (sections.len() < 2).then_some("`sketches`: two sections or more, in order")
            }
            Self::Sweep { profile, path, .. } => (profile.is_empty() || path.is_empty())
                .then_some("`sketches`: the profile, then the path"),
            Self::Offset { faces, .. } => {
                faces.is_empty().then_some("`faces`: the faces to offset")
            }
            Self::Split { faces, curves } => (faces.is_empty() || curves.is_empty()).then_some(
                "`faces` and `sketches`: the faces to split and the curves to split them along",
            ),
            Self::Extend { edges, .. } => edges
                .is_empty()
                .then_some("`edges`: the edges to extend past"),
            Self::Fillet { edges, .. } => edges.is_empty().then_some("`edges`: the edges to round"),
            Self::FilletFaces { first, second, .. } => (first.is_none() || second.is_none())
                .then_some("`first` and `second`: the two faces to round between"),
            Self::Blend { first, second, .. } => (first.is_none() || second.is_none())
                .then_some("`first` and `second`: the two edges to bridge"),
            Self::Sew { .. } | Self::Thicken { .. } | Self::Trim { .. } | Self::Mirror { .. } => {
                None
            }
            Self::FitScan { sheet, .. } => sheet
                .is_none()
                .then_some("`scan` and `at`: where on a scan to fit"),
            Self::FitScanFree { points, .. } => points
                .is_empty()
                .then_some("`scan` and `at`: where on a scan to fit"),
            Self::FitNetwork {
                curves,
                polylines,
                patches,
                chained,
                scan,
                ..
            } => {
                if scan.is_none() {
                    Some("`scan`: the scan to fit")
                } else if curves.is_empty() && polylines.is_empty() {
                    Some("`sketches`, `curves` or `polylines`: the network's curves")
                } else if patches.is_empty() && chained.is_empty() {
                    Some("`patches` or `chained`: each patch's sides as curve numbers")
                } else {
                    None
                }
            }
            Self::TrimScan { scan, .. } => scan
                .is_none()
                .then_some("`scan`: the scan whose surface the faces are kept on"),
        }
    }

    /// Whether the step adds a sheet of its own, rather than working on
    /// what the body has.
    pub fn constructs(&self) -> bool {
        !matches!(
            self,
            SurfaceFeature::Sew { .. }
                | SurfaceFeature::Fillet { .. }
                | SurfaceFeature::FilletFaces { .. }
                | SurfaceFeature::Thicken { .. }
                | SurfaceFeature::Trim { .. }
                | SurfaceFeature::Mirror { .. }
                | SurfaceFeature::Split { .. }
                | SurfaceFeature::Extend { .. }
                | SurfaceFeature::TrimScan { .. }
        )
    }

    /// Whether the step reads what its body holds, so it needs a body to
    /// go in: every step that works on the body's surfaces, and an offset
    /// or a blend, built from the body's faces and edges.
    pub fn needs_body(&self) -> bool {
        !self.constructs() || matches!(self, Self::Offset { .. } | Self::Blend { .. })
    }
}

impl WorkbenchFeature for SurfaceFeature {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(KIND)
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        self.sketches()
    }

    fn name(&self) -> &str {
        self.kind().label
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_makes_its_own_kind() {
        for kind in KINDS.iter().filter(|k| k.waits.is_none()) {
            let feature = SurfaceFeature::for_tool(kind.tool).expect(kind.tool);
            assert_eq!(feature.tool(), kind.tool);
            let back = SurfaceFeature::from_json(&feature.to_json()).unwrap();
            assert_eq!(back, feature);
        }
    }

    /// Optional surface inputs have defaults when absent from a document.
    #[test]
    fn optional_surface_inputs_default_when_absent() {
        let id = FeatureId::new();
        let loft = serde_json::json!({"Loft": {"sections": [{"Sketch": id}], "closed": false}});
        assert!(matches!(
            SurfaceFeature::from_json(&loft).unwrap(),
            SurfaceFeature::Loft { guides, .. } if guides.is_empty()
        ));
        let sweep = serde_json::json!({"Sweep": {"profile": [], "path": []}});
        assert!(matches!(
            SurfaceFeature::from_json(&sweep).unwrap(),
            SurfaceFeature::Sweep { second_rail, .. } if second_rail.is_empty()
        ));
        let network = serde_json::json!({"FitNetwork": {"patches": [[1, 2, 3, 4]]}});
        assert!(matches!(
            SurfaceFeature::from_json(&network).unwrap(),
            SurfaceFeature::FitNetwork { chained, corrections: 0, patches, .. }
                if chained.is_empty() && patches == vec![[1, 2, 3, 4]]
        ));
    }

    /// A guide and a second rail are curves the step reads, so their
    /// sketches are what it depends on.
    #[test]
    fn guides_and_rails_are_read_curves() {
        let (a, b, g) = (FeatureId::new(), FeatureId::new(), FeatureId::new());
        let loft = SurfaceFeature::Loft {
            sections: vec![CurveRef::Sketch(a), CurveRef::Sketch(b)],
            closed: false,
            guides: vec![CurveRef::Sketch(g)],
        };
        assert_eq!(loft.dependencies(), vec![a, b, g]);
        let sweep = SurfaceFeature::Sweep {
            profile: vec![CurveRef::Sketch(a)],
            path: vec![CurveRef::Sketch(b)],
            second_rail: vec![CurveRef::Sketch(g)],
        };
        assert_eq!(sweep.dependencies(), vec![a, b, g]);
    }

    #[test]
    fn a_step_depends_on_the_sketches_it_reads_once_each() {
        let a = FeatureId::new();
        let b = FeatureId::new();
        let loft = SurfaceFeature::Loft {
            sections: vec![
                CurveRef::Sketch(a),
                CurveRef::Sketch(b),
                CurveRef::Sketch(a),
            ],
            closed: false,
            guides: Vec::new(),
        };
        assert_eq!(loft.dependencies(), vec![a, b]);
    }
}
