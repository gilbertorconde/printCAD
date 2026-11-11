//! What a chain build answers: the solid and its mesh, the preview, and the
//! error naming the failing op.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::*;

/// Error from executing a body's solid-op chain, attributed to the failing
/// step so the caller can mark the matching feature.
#[derive(Debug, Clone, Default, Error)]
#[error("op {op_index}: {message}")]
pub struct ChainError {
    pub op_index: usize,
    pub message: String,
    /// The edges a refused fillet or chamfer took, and those it could not
    /// round or bevel.
    pub edges: EdgeRefusal,
}

/// The edges of a fillet or chamfer the kernel refused.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EdgeRefusal {
    /// Every edge the op took, in its order, tangent chains followed: what
    /// it rounds without the failing ones is these less those.
    pub request: Vec<EdgeProbe>,
    /// The edges that fail.
    pub failing: Vec<FailingEdge>,
    /// The largest share of the asked size at which every edge of the
    /// request builds, when it does at some: the size times it fits.
    #[serde(default)]
    pub fits: Option<f64>,
}

/// An edge a fillet or chamfer could not round or bevel: refused alone, or
/// in a group of the op's edges meeting at a vertex.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FailingEdge {
    /// Its place among the edges the op rounds, from zero.
    pub index: usize,
    /// How many edges the op rounds.
    pub of: usize,
    /// A pick that names it in the body's own frame: a point halfway
    /// along, the way it runs there and its faces' names.
    pub probe: EdgeProbe,
    /// Points along it in the body's own frame, to draw it by.
    pub points: Vec<[f64; 3]>,
    /// Why the kernel refused it.
    pub reason: String,
    /// The largest share of the asked size at which it builds on its own,
    /// when it does at some.
    #[serde(default)]
    pub fits: Option<f64>,
}

/// Result of executing a body's solid-op chain.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SolidBuildResult {
    /// Native-format shape snapshot of the final solid (for later
    /// re-tessellation, persistence, and downstream booleans).
    pub brep_blob: Vec<u8>,
    /// Render mesh of the final solid.
    pub mesh: TriMesh,
    /// Axis-aligned bounds in millimetres.
    pub bounds_mm: Option<([f32; 3], [f32; 3])>,
    /// What the feature being edited does, when the build was asked for it.
    pub preview: Option<Box<FeaturePreview>>,
    /// The answers to the chain's probes, in the order they were asked.
    pub probes: Vec<Result<ProbeAnswer, String>>,
    /// The curves the chain's projections came to, in the order they were
    /// asked.
    #[serde(default)]
    pub projections: Vec<Result<Vec<ProjectedEdge>, String>>,
    /// What an op built but should be told: a reference whose name was
    /// lost, found again by where it was picked. In op order.
    #[serde(default)]
    pub warnings: Vec<OpWarning>,
    /// The same for the probes, by their order in `probes`: a face or edge
    /// found by where it was picked after its name was lost.
    #[serde(default)]
    pub probe_warnings: Vec<OpWarning>,
    /// The same for the projections, by their order in `projections`.
    #[serde(default)]
    pub projection_warnings: Vec<OpWarning>,
    /// The largest size the previewed fillet or chamfer builds at, when the
    /// build previewed one: what a drag of its size stops at.
    #[serde(default)]
    pub size_limit: Option<SizeLimit>,
}

/// The largest size a fillet or chamfer builds at on the solid before it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct SizeLimit {
    /// The op it is the limit of.
    pub op_index: usize,
    /// [`SolidOp::size_limit_key`] of the op it was worked out for.
    pub key: u64,
    /// Its first size at the limit, mm: a fillet's radius, a chamfer's
    /// distance (with two distances, the first at their common scale).
    /// `None` where no size short of the solid's own fails.
    pub largest: Option<f64>,
}

/// A warning on one op of a chain that built (or, in
/// [`SolidBuildResult::probe_warnings`] and `projection_warnings`, on one
/// probe or projection, `op_index` then being its place in their list).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OpWarning {
    pub op_index: usize,
    pub message: String,
}

/// What a feature being edited does to its body, beside the body without
/// it: the view while it is edited.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeaturePreview {
    /// The body to show meanwhile: as it stood before a feature that adds,
    /// after one that cuts. None when an adding feature is the body's
    /// first.
    pub shown: Option<Box<SolidBuildResult>>,
    /// The feature's own tool solid, meshed: the material it adds or takes.
    pub tool: TriMesh,
    /// The feature cuts.
    pub cuts: bool,
}
