use kernel_api::{
    BodyHandle, Kernel, KernelError, KernelResult, RebuildRequest, RebuildResponse,
    TessellationSettings, TriMesh,
};
use tracing::info;

/// Placeholder OCCT-backed kernel implementation.
pub struct OcctKernel {
    initialized: bool,
}

impl Default for OcctKernel {
    fn default() -> Self {
        Self::new()
    }
}

impl OcctKernel {
    pub fn new() -> Self {
        Self { initialized: false }
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }
}

impl Kernel for OcctKernel {
    fn name(&self) -> &str {
        "OpenCascade"
    }

    fn initialize(&mut self) -> KernelResult<()> {
        if !self.initialized {
            info!("Initializing OCCT kernel (stub)");
            // Actual FFI wires will live here once bindings land.
            self.initialized = true;
        }
        Ok(())
    }

    fn rebuild(&mut self, request: &RebuildRequest) -> KernelResult<RebuildResponse> {
        if !self.initialized {
            return Err(KernelError::NotInitialized);
        }

        let generated_handles = request
            .dirty_features
            .iter()
            .enumerate()
            .map(|(index, _)| BodyHandle(index as u64 + 1))
            .collect();

        Ok(RebuildResponse {
            updated_bodies: generated_handles,
            diagnostics: vec!["OCCT kernel stub executed".to_string()],
        })
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
}
