//! ogeom-backed implementation of the [`Kernel`] trait.
//!
//! Pure-Rust kernel adapter: STEP import, deferred
//! tessellation of persisted shape snapshots, and solid-op chain execution
//! all go through the `ogeom` kernel. Persisted shape blobs are ogeom
//! native-format text bytes (`ogeom::io::native`).

mod chain;
mod import;
mod ops;
mod profile;
pub mod progress;
mod tess;

pub use progress::CONTEXT_PREFIX;
// The host installs a progress watch around each job; re-exported here so
// app code never depends on `ogeom` directly.
pub use ogeom::core::progress::{watched, Canceller, Stage, Watch};

/// Announce a stage as the kernel would, for host-side tests of a sink.
#[doc(hidden)]
pub fn stage_for_test(name: &str) {
    ogeom::core::progress::stage(name);
}

use std::path::Path;

use kernel_api::{
    BodyHandle, ChainError, ImportedModel, Kernel, KernelError, KernelResult, RebuildRequest,
    RebuildResponse, SolidBuildResult, SolidOp, TessellationSettings, TriMesh,
};
use tracing::info;

/// ogeom-backed kernel.
pub struct OgeomKernel {
    initialized: bool,
}

impl Default for OgeomKernel {
    fn default() -> Self {
        Self::new()
    }
}

impl OgeomKernel {
    pub fn new() -> Self {
        Self { initialized: false }
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Tessellates one imported body from its native-format snapshot using a
    /// pre-snapshotted per-face RGB table (same order as `ogeom::topo::explore`
    /// over the deserialized root shape).
    pub fn tessellate_step_brep(
        &self,
        brep_blob: &[u8],
        face_colors: &[[f32; 3]],
        detail: &TessellationSettings,
    ) -> KernelResult<TriMesh> {
        if !self.initialized {
            return Err(KernelError::NotInitialized);
        }
        tess::tessellate_blob(brep_blob, face_colors, detail, tess::Faces::Wide)
    }

    /// Tessellate many snapshots at once, spreading the *bodies* across
    /// threads and keeping each body's faces on one.
    ///
    /// An assembly is usually many small bodies, where per-body overhead —
    /// parsing the snapshot above all — dominates the face work, so this is
    /// the pass that matters for large imports. Results come back in input
    /// order, one per job, so a failure is attributed to its own body.
    pub fn tessellate_step_breps(
        &self,
        jobs: &[(&[u8], &[[f32; 3]])],
        detail: &TessellationSettings,
    ) -> KernelResult<Vec<KernelResult<TriMesh>>> {
        if !self.initialized {
            return Err(KernelError::NotInitialized);
        }
        Ok(ogeom::core::parallel::map_ordered(
            jobs,
            |_, (blob, face_colors)| {
                tess::tessellate_blob(blob, face_colors, detail, tess::Faces::Inline)
            },
        ))
    }

    /// Read + tessellate in one synchronous shot (legacy path). Useful for
    /// tests comparing meshes against the deferred pipeline.
    pub fn import_step_full_mesh(
        &mut self,
        path: &Path,
        detail: &TessellationSettings,
    ) -> KernelResult<ImportedModel> {
        self.initialize()?;
        import::import_step(path, detail, true)
    }

    /// Execute a body's solid-op chain: the first op must be a
    /// shape-producing NewSolid; each subsequent op fuses/cuts a new tool
    /// solid against the accumulated solid or modifies it directly
    /// (dress-ups, patterns, booleans). Errors are attributed to the failing
    /// op's chain index. Returns the final solid's native snapshot + render
    /// mesh.
    pub fn execute_solid_chain(
        &mut self,
        ops: &[SolidOp],
        detail: &TessellationSettings,
    ) -> Result<SolidBuildResult, ChainError> {
        self.initialize().map_err(|e| ChainError {
            op_index: 0,
            message: e.to_string(),
        })?;
        chain::execute(ops, detail)
    }
}

impl Kernel for OgeomKernel {
    fn name(&self) -> &str {
        "ogeom"
    }

    fn initialize(&mut self) -> KernelResult<()> {
        if !self.initialized {
            info!("Initializing ogeom kernel");
            self.initialized = true;
        }
        Ok(())
    }

    fn rebuild(&mut self, _request: &RebuildRequest) -> KernelResult<RebuildResponse> {
        if !self.initialized {
            return Err(KernelError::NotInitialized);
        }
        Ok(RebuildResponse::default())
    }

    fn tessellate(
        &self,
        _body: BodyHandle,
        _detail: &TessellationSettings,
    ) -> KernelResult<TriMesh> {
        if !self.initialized {
            return Err(KernelError::NotInitialized);
        }
        Ok(TriMesh::default())
    }

    fn import_step(
        &mut self,
        path: &Path,
        detail: &TessellationSettings,
    ) -> KernelResult<ImportedModel> {
        self.initialize()?;
        import::import_step(path, detail, false)
    }
}
