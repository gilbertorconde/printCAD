//! The kernel itself: its trait, its error and its result, with the rebuild
//! stubs kept until 0.2.

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

use crate::*;

/// Convenience alias for kernel fallible operations.
pub type KernelResult<T> = Result<T, KernelError>;

/// Handle to a body managed by the kernel.
#[deprecated(since = "0.1.6", note = "never called; removed in 0.2")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BodyHandle(pub u64);

/// Request describing which features or bodies must be recomputed.
#[deprecated(since = "0.1.6", note = "never called; removed in 0.2")]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RebuildRequest {
    /// Feature identifiers that triggered the rebuild.
    pub dirty_features: Vec<String>,
    /// Whether dependent features should be recomputed automatically.
    pub propagate: bool,
}

/// Response returned for every rebuild invocation.
#[deprecated(since = "0.1.6", note = "never called; removed in 0.2")]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RebuildResponse {
    /// Bodies that were modified or regenerated.
    #[allow(deprecated)]
    pub updated_bodies: Vec<BodyHandle>,
    /// Kernel provided diagnostics or warnings.
    pub diagnostics: Vec<String>,
}

/// Trait implemented by any geometry kernel that can serve the application.
#[allow(deprecated)]
pub trait Kernel: Send {
    /// Human-friendly identifier for logging purposes.
    fn name(&self) -> &str;

    /// Called once before any geometry work happens.
    fn initialize(&mut self) -> KernelResult<()>;

    /// Recompute dirty features/bodies and return the affected handles.
    #[deprecated(since = "0.1.6", note = "never called; removed in 0.2")]
    fn rebuild(&mut self, _request: &RebuildRequest) -> KernelResult<RebuildResponse> {
        Err(KernelError::Unsupported("rebuild".into()))
    }

    /// Produce a triangular mesh for the provided body handle.
    #[deprecated(since = "0.1.6", note = "never called; removed in 0.2")]
    fn tessellate(
        &self,
        _body: BodyHandle,
        _detail: &TessellationSettings,
    ) -> KernelResult<TriMesh> {
        Err(KernelError::Unsupported("tessellate".into()))
    }

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
