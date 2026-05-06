//! OpenCASCADE-backed implementation of the [`Kernel`] trait.
//!
//! The current scope is intentionally minimal: STEP/STP files can be imported
//! and tessellated for viewport display. Boolean/feature operations remain to
//! be wired in as the kernel matures.

mod ffi;
mod step_header;

use std::ffi::{CStr, CString};
use std::path::Path;

use kernel_api::{
    BodyHandle, ImportedBody, ImportedModel, Kernel, KernelError, KernelResult, RebuildRequest,
    RebuildResponse, TessellationSettings, TriMesh,
};
use tracing::info;

pub use step_header::detect_step_unit;

/// OpenCASCADE-backed kernel.
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
            info!("Initializing OCCT kernel");
            self.initialized = true;
        }
        Ok(())
    }

    fn rebuild(&mut self, _request: &RebuildRequest) -> KernelResult<RebuildResponse> {
        if !self.initialized {
            return Err(KernelError::NotInitialized);
        }
        // Feature recomputation is not yet implemented for the OCCT kernel.
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
        // Bodies aren't tracked yet; STEP imports return their meshes inline.
        Ok(TriMesh::default())
    }

    fn import_step(
        &mut self,
        path: &Path,
        detail: &TessellationSettings,
    ) -> KernelResult<ImportedModel> {
        self.initialize()?;
        import_step_internal(path, detail)
    }
}

fn import_step_internal(path: &Path, detail: &TessellationSettings) -> KernelResult<ImportedModel> {
    let path_str = path.to_str().ok_or_else(|| {
        KernelError::InvalidInput(format!(
            "STEP path is not valid UTF-8: {}",
            path.display()
        ))
    })?;
    let c_path = CString::new(path_str).map_err(|err| {
        KernelError::InvalidInput(format!("STEP path contains a NUL byte: {err}"))
    })?;

    // Probe the STEP HEADER section for its declared length unit before
    // handing the file to OCCT (OCCT itself converts geometry to mm regardless,
    // so this is a pure introspection step for the UI).
    let source_unit = step_header::detect_step_unit_from_path(path).ok().flatten();

    let linear = detail.chord_tolerance.max(0.001) as f64;
    let angular = (detail.angular_tolerance_deg.max(0.5) as f64).to_radians();
    let weld_cross_face = if detail.weld_cross_face { 1 } else { 0 };
    let weld_angle_rad = (detail.weld_angle_threshold_deg.max(0.0) as f64).to_radians();

    // SAFETY: the C++ shim allocates result buffers itself and we always pass
    // the result through `printcad_occt_free_result` after copying its data.
    let result = unsafe {
        ffi::printcad_occt_import_step(
            c_path.as_ptr(),
            linear,
            angular,
            weld_cross_face,
            weld_angle_rad,
        )
    };

    if !result.error.is_null() {
        let message = unsafe { CStr::from_ptr(result.error) }
            .to_string_lossy()
            .into_owned();
        unsafe { ffi::printcad_occt_free_result(result) };
        return Err(KernelError::Import(message));
    }

    let mut bodies = Vec::with_capacity(result.body_count);
    if result.body_count > 0 && !result.bodies.is_null() {
        let raw_bodies = unsafe { std::slice::from_raw_parts(result.bodies, result.body_count) };
        for body in raw_bodies {
            let positions = copy_vec3_array(body.positions, body.vertex_count);
            let normals = copy_vec3_array(body.normals, body.vertex_count);
            let indices = copy_u32_array(body.indices, body.index_count);
            // Boundary edges are computed inside the C++ shim *before*
            // welding (so face boundaries survive even when seam vertices
            // get merged) and then remapped through the welding table. We
            // simply copy them out here.
            let edges = copy_u32_array(body.edges, body.edge_count * 2);

            // Vertex/triangle ratio profiling. With cross-face welding
            // active the ratio should drop close to 1 on dense models;
            // logging the post-weld ratio doubles as a sanity check that
            // the welder is actually firing.
            let triangle_count = indices.len() / 3;
            if triangle_count > 0 {
                let ratio = positions.len() as f64 / triangle_count as f64;
                let label = body
                    .name
                    .is_null()
                    .then(|| "<unnamed>".to_string())
                    .unwrap_or_else(|| {
                        unsafe { CStr::from_ptr(body.name) }
                            .to_string_lossy()
                            .into_owned()
                    });
                tracing::info!(
                    body = %label,
                    vertices = positions.len(),
                    triangles = triangle_count,
                    boundary_edges = edges.len() / 2,
                    welded = detail.weld_cross_face,
                    weld_angle_deg = detail.weld_angle_threshold_deg,
                    ratio = format!("{:.2}", ratio),
                    "STEP body tessellation profile (vertex/triangle ratio)"
                );
            }

            let mesh = TriMesh {
                positions,
                normals,
                indices,
                edges,
            };
            let name = if body.name.is_null() {
                None
            } else {
                Some(unsafe { CStr::from_ptr(body.name) }.to_string_lossy().into_owned())
            };
            bodies.push(ImportedBody { name, mesh });
        }
    }

    unsafe { ffi::printcad_occt_free_result(result) };

    info!(
        "Imported STEP `{}`: {} bodies, {} triangles, source unit {:?}",
        path.display(),
        bodies.len(),
        bodies
            .iter()
            .map(|b| b.mesh.indices.len() / 3)
            .sum::<usize>(),
        source_unit,
    );

    Ok(ImportedModel {
        bodies,
        source_unit,
    })
}

fn copy_vec3_array(ptr: *const f32, vertex_count: usize) -> Vec<[f32; 3]> {
    if ptr.is_null() || vertex_count == 0 {
        return Vec::new();
    }
    let scalar_count = vertex_count * 3;
    let slice = unsafe { std::slice::from_raw_parts(ptr, scalar_count) };
    let mut out = Vec::with_capacity(vertex_count);
    for i in 0..vertex_count {
        let base = i * 3;
        out.push([slice[base], slice[base + 1], slice[base + 2]]);
    }
    out
}

fn copy_u32_array(ptr: *const u32, count: usize) -> Vec<u32> {
    if ptr.is_null() || count == 0 {
        return Vec::new();
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, count) };
    slice.to_vec()
}

// Boundary-edge extraction now lives in the C++ shim (`step_loader.cpp`)
// so it can run before vertex welding and have its output remapped through
// the welding table. The Rust-side hash/sort fallbacks were removed when
// that move landed; the C++ algorithm is the single source of truth.
