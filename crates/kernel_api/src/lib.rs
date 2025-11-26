use serde::{Deserialize, Serialize};
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
}

impl Default for TessellationSettings {
    fn default() -> Self {
        Self {
            chord_tolerance: 0.1,
            angular_tolerance_deg: 20.0,
        }
    }
}

/// Triangular mesh generated from kernel bodies for viewports and export.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TriMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
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
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
