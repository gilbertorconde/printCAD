//! Mesh edits as plain data: the steps of a mesh body's history and what
//! each reports.

use serde::{Deserialize, Serialize};

use crate::*;

/// Part of a mesh, by where its points are: a face is in it when all
/// three of its corners are.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MeshRegion {
    All,
    Box {
        min: [f64; 3],
        max: [f64; 3],
    },
    Ball {
        centre: [f64; 3],
        radius: f64,
    },
    /// Vertices of the history's start mesh, by number (a marking).
    Vertices(Vec<u32>),
}

/// One step of a mesh's history, in the mesh's own frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(not(feature = "exhaustive"), non_exhaustive)]
pub enum MeshOp {
    /// Pieces whose bounding box's diagonal is shorter than this removed.
    RemoveIslands { smaller_than: f64 },
    /// The region's faces removed, the hole left open.
    Delete { region: MeshRegion },
    /// The region's faces turned over.
    Flip { region: MeshRegion },
    /// Fill open rims whose longest span is at most `holes_up_to`, or the
    /// nearest such rim to `near` when given. Existing vertices stay fixed.
    Fill {
        holes_up_to: f64,
        #[serde(default)]
        near: Option<[f64; 3]>,
        #[serde(default)]
        fairing: MeshFairing,
    },
    /// Project the region onto local quadrics, with a bounded movement.
    Smooth {
        region: MeshRegion,
        bumps_up_to: f64,
        move_at_most: f64,
        #[serde(default)]
        keep_edges_deg: Option<f64>,
    },
    /// Move faces by a normal distance; a marking gets a joining wall.
    Offset { region: MeshRegion, distance: f64 },
    /// The boolean `operation` of the closed mesh (first) and a bounded
    /// solid in the mesh's frame, the solid meshed to `chord` first. An
    /// open mesh, and an operand it does not meet, are refused.
    Boolean {
        operand: MeshOperand,
        operation: BooleanOp,
        chord: f64,
    },
    /// A closed mesh hollowed: its surface kept, an inner wall `thickness`
    /// inside it bounding a sealed cavity. An open sheet thickened to the
    /// `side` given, rim walls closing it.
    Shell {
        thickness: f64,
        #[serde(default)]
        side: MeshSide,
    },
    /// Decimate and measure both ways at the 99.9th percentile. The
    /// triangle limit takes priority over the tolerance; the report says
    /// how far the resulting surface lies from its input.
    Reduce {
        tolerance: f64,
        triangle_limit: usize,
    },
    /// Cut by the plane through `origin` square to `normal`, the side
    /// `normal` points to kept when `keep_above`; with `close`, every loop
    /// of the cut capped flat, gaps in its rim up to `bridge` closed.
    Cut {
        origin: [f64; 3],
        normal: [f64; 3],
        keep_above: bool,
        close: bool,
        bridge: f64,
    },
}

/// A bounded solid a mesh boolean takes, as it stood when the step was
/// made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MeshOperand {
    /// A solid in the kernel's native text format, moved by the row-major
    /// rigid `transform` into the frame it is used in.
    Shape {
        brep: String,
        transform: [[f64; 4]; 4],
    },
    /// A primitive at its placement.
    Primitive {
        kind: PrimitiveKind,
        placement: Placement,
    },
}

/// Which side of an open sheet a shell thickens: inward is against the
/// way its triangles face.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum MeshSide {
    Inward,
    Outward,
    #[default]
    Both,
}

/// A solid trimmed where a scan breaks off: the scan's triangles lying
/// deeper than `depth` inside the solid are the break, which is extended
/// past the solid, thickened by `gap` to each side and cut away. Both in
/// the solid's frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrimRequest {
    /// The solid, in the kernel's native format.
    pub solid: Vec<u8>,
    pub scan: TriMesh,
    pub depth: f64,
    pub gap: f64,
    /// Keep what lies on the side of the break away from the scan (the
    /// piece); otherwise the side toward it.
    pub away: bool,
}

/// A trimmed solid and how it meets the scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrimAnswer {
    /// The trimmed solid, as a `.brep` file holds it.
    pub brep: Vec<u8>,
    /// Its volume, mm³.
    pub volume: f64,
    /// The area of its faces that lie along the break, mm².
    pub mating_area: f64,
    /// The distance from the break's points to the mating faces: median
    /// and the range holding the middle 90 %, mm.
    pub gap_median: f64,
    pub gap_range: [f64; 2],
}

/// How a hole's added vertices settle.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum MeshFairing {
    #[default]
    Curved,
    Flat,
}

/// Measurements made by a mesh step, in millimetres.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MeshStepReport {
    Cut {
        capped_loops: usize,
        bridged: usize,
        open_loops: usize,
    },
    /// `volume` in mm³; `cut_triangles` the result's triangles the other
    /// operand cut.
    Boolean {
        cut_triangles: usize,
        volume: f64,
    },
    /// `hollow` for a closed part, otherwise a sheet thickened.
    Shell {
        hollow: bool,
        volume: f64,
    },
    Smooth {
        max: f64,
        rms: f64,
    },
    Reduce {
        within_share: f64,
        p999: f64,
        furthest: [f64; 3],
        max: f64,
        rms: f64,
        target_met: bool,
    },
}

/// What one step of a mesh history did.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MeshStep {
    /// Faces after it.
    pub faces: usize,
    pub note: String,
    #[serde(default)]
    pub report: Option<MeshStepReport>,
}

/// A mesh history built: the mesh at its end, what each step did, and the
/// step that failed, when one did (the mesh is then the one before it).
#[derive(Debug, Clone, Default)]
pub struct MeshChainAnswer {
    pub mesh: TriMesh,
    /// The start vertex behind each displayed vertex, where the kernel
    /// preserves it. Added surface has no source vertex.
    pub sources: Vec<Option<u32>>,
    pub steps: Vec<MeshStep>,
    pub failed: Option<(usize, String)>,
}
