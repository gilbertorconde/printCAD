use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

/// Convenience alias for kernel fallible operations.
pub type KernelResult<T> = Result<T, KernelError>;

/// Handle to a body managed by the kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BodyHandle(pub u64);

/// Request describing which features or bodies must be recomputed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RebuildRequest {
    /// Feature identifiers that triggered the rebuild.
    pub dirty_features: Vec<String>,
    /// Whether dependent features should be recomputed automatically.
    pub propagate: bool,
}

/// Response returned for every rebuild invocation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RebuildResponse {
    /// Bodies that were modified or regenerated.
    pub updated_bodies: Vec<BodyHandle>,
    /// Kernel provided diagnostics or warnings.
    pub diagnostics: Vec<String>,
}

/// Parameters controlling tessellation quality for viewport rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TessellationSettings {
    pub chord_tolerance: f32,
    pub angular_tolerance_deg: f32,
    /// When true, the kernel collapses vertices that share a position across
    /// multiple faces *and* whose face normals are within
    /// [`Self::weld_angle_threshold_deg`] of each other. This typically
    /// shrinks vertex counts by 4–6× on dense CAD models without flattening
    /// genuine sharp edges, since dissimilar face normals stay separate.
    #[serde(default = "default_weld_cross_face")]
    pub weld_cross_face: bool,
    /// Maximum angle between two face normals at a shared position for them
    /// to be merged into a single welded vertex. Above this angle the kernel
    /// keeps them separate so hard CAD edges stay crisp under shading.
    /// Defaults to 30° (common cross-face weld preset).
    #[serde(default = "default_weld_angle_threshold_deg")]
    pub weld_angle_threshold_deg: f32,
}

fn default_weld_cross_face() -> bool {
    true
}

fn default_weld_angle_threshold_deg() -> f32 {
    30.0
}

impl Default for TessellationSettings {
    fn default() -> Self {
        Self {
            chord_tolerance: 0.1,
            angular_tolerance_deg: 20.0,
            weld_cross_face: default_weld_cross_face(),
            weld_angle_threshold_deg: default_weld_angle_threshold_deg(),
        }
    }
}

/// Triangular mesh generated from kernel bodies for viewports and export.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TriMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// Optional line-list of feature edges that should be drawn as outlines on
    /// top of the shaded surface (e.g. face boundaries from a STEP import).
    /// Stored as flat pairs of indices into [`Self::positions`]; the slice
    /// length is therefore always a multiple of two. Empty when the source has
    /// no edge information available.
    #[serde(default)]
    pub edges: Vec<u32>,
    /// Optional per-vertex linear RGB albedo in 0..1 (same length as [`Self::positions`] when set).
    /// Empty means the renderer uses the body tint from push constants only.
    #[serde(default)]
    pub colors: Vec<[f32; 3]>,
}

impl TriMesh {
    /// Compute axis-aligned bounding box `(min, max)` of the mesh, if non-empty.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        if self.positions.is_empty() {
            return None;
        }
        let mut min = self.positions[0];
        let mut max = self.positions[0];
        for &[x, y, z] in &self.positions {
            if x < min[0] {
                min[0] = x;
            }
            if y < min[1] {
                min[1] = y;
            }
            if z < min[2] {
                min[2] = z;
            }
            if x > max[0] {
                max[0] = x;
            }
            if y > max[1] {
                max[1] = y;
            }
            if z > max[2] {
                max[2] = z;
            }
        }
        Some((min, max))
    }
}

/// A single body produced by an external import (e.g. STEP).
///
/// The kernel returns one entry per top-level solid/shell encountered in the
/// source file, along with a tessellated [`TriMesh`] suitable for rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedBody {
    /// Optional name extracted from the source file (e.g. STEP product label).
    pub name: Option<String>,
    /// Tessellated mesh ready for the viewport.
    pub mesh: TriMesh,
}

/// Length unit declared by an imported source file.
///
/// This is a thin mirror of `core_document::Unit` that lives in `kernel_api`
/// to avoid pulling document-side types into the kernel crate. App code is
/// responsible for translating it into the document's display unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LengthUnit {
    Millimetre,
    Centimetre,
    Metre,
    Inch,
    Foot,
}

/// Result of importing an external CAD file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportedModel {
    /// One entry per top-level body in the source file.
    pub bodies: Vec<ImportedBody>,
    /// Length unit declared by the source file, when detectable. Geometry in
    /// `bodies` is *always* expressed in millimetres regardless — this field
    /// is purely informational and used by the UI to pick a display unit.
    #[serde(default)]
    pub source_unit: Option<LengthUnit>,
}

/// Trait implemented by any geometry kernel that can serve the application.
pub trait Kernel: Send {
    /// Human-friendly identifier for logging purposes.
    fn name(&self) -> &str;

    /// Called once before any geometry work happens.
    fn initialize(&mut self) -> KernelResult<()>;

    /// Recompute dirty features/bodies and return the affected handles.
    fn rebuild(&mut self, request: &RebuildRequest) -> KernelResult<RebuildResponse>;

    /// Produce a triangular mesh for the provided body handle.
    fn tessellate(&self, body: BodyHandle, detail: &TessellationSettings) -> KernelResult<TriMesh>;

    /// Read a STEP/STP file from disk and return tessellated bodies.
    ///
    /// Implementations that do not support STEP import should return
    /// [`KernelError::Unsupported`].
    fn import_step(
        &mut self,
        _path: &Path,
        _detail: &TessellationSettings,
    ) -> KernelResult<ImportedModel> {
        Err(KernelError::Unsupported(
            "STEP import is not implemented by this kernel".into(),
        ))
    }
}

/// Standardized error type for kernel interactions.
#[derive(Debug, Error)]
pub enum KernelError {
    #[error("kernel initialization failed: {0}")]
    Initialization(String),
    #[error("kernel not initialized")]
    NotInitialized,
    #[error("operation unsupported: {0}")]
    Unsupported(String),
    #[error("invalid kernel input: {0}")]
    InvalidInput(String),
    #[error("import failed: {0}")]
    Import(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
