//! OpenCASCADE-backed implementation of the [`Kernel`] trait.
//!
//! The current scope is intentionally minimal: STEP/STP files can be imported
//! and tessellated for viewport display. Boolean/feature operations remain to
//! be wired in as the kernel matures.

mod ffi;
mod step_header;

use std::ffi::{CStr, CString};
use std::path::Path;
use std::time::Instant;

use kernel_api::{
    BodyHandle, BooleanOp, ExtrudeOp, ImportedBody, ImportedModel, ImportedNode, ImportedNodeKind,
    Kernel, KernelError, KernelResult, LinearDeflectionMode, ProfilePlane, ProfileSegment,
    RebuildRequest, RebuildResponse, SolidBuildResult, TessellationSettings, TriMesh,
};
use tracing::info;

use std::os::raw::{c_double, c_int};

fn tessellation_linear_for_ffi(detail: &TessellationSettings) -> (c_int, c_double) {
    match detail.linear_deflection_mode {
        LinearDeflectionMode::BboxScaled => (0, detail.mesh_deviation.max(0.001) as c_double),
        LinearDeflectionMode::AbsoluteMm => (1, detail.chord_tolerance.max(0.001) as c_double),
    }
}

fn boundary_edges_for_ffi(detail: &TessellationSettings) -> c_int {
    if detail.generate_boundary_edges {
        1
    } else {
        0
    }
}

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

    /// Tessellates one imported BRep body using a pre-snapshotted per-face RGB
    /// table (same order as the OCCT face explorer on the original shape).
    pub fn tessellate_step_brep(
        &self,
        brep_blob: &[u8],
        face_colors: &[[f32; 3]],
        detail: &TessellationSettings,
    ) -> KernelResult<TriMesh> {
        if !self.initialized {
            return Err(KernelError::NotInitialized);
        }
        tessellate_brep_internal(brep_blob, face_colors, detail)
    }

    /// Read + transfer + `BRepMesh` + extract in one synchronous shot (legacy path).
    /// Useful for tests comparing triangle counts against the deferred pipeline.
    pub fn import_step_full_mesh(
        &mut self,
        path: &Path,
        detail: &TessellationSettings,
    ) -> KernelResult<ImportedModel> {
        self.initialize()?;
        import_step_full_mesh_internal(path, detail)
    }

    /// Execute a body's extrude chain: the first op must be NewSolid; each
    /// subsequent op fuses/cuts against the accumulated solid. Returns the
    /// final solid's BRep snapshot + render mesh.
    pub fn execute_extrude_chain(
        &mut self,
        ops: &[ExtrudeOp],
        detail: &TessellationSettings,
    ) -> Result<SolidBuildResult, KernelError> {
        self.initialize()?;
        execute_extrude_chain_internal(ops, detail)
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
        import_step_brep_only_internal(path, detail)
    }
}

fn tessellate_brep_internal(
    brep_blob: &[u8],
    face_colors: &[[f32; 3]],
    detail: &TessellationSettings,
) -> KernelResult<TriMesh> {
    let (linear_mode, linear_value) = tessellation_linear_for_ffi(detail);
    let angular = (detail.angular_tolerance_deg.max(0.5) as f64).to_radians();
    let weld_cross_face = if detail.weld_cross_face { 1 } else { 0 };
    let weld_angle_rad = (detail.weld_angle_threshold_deg.max(0.0) as f64).to_radians();
    let gen_edges = boundary_edges_for_ffi(detail);

    let mut flat_colors: Vec<f32> = Vec::with_capacity(face_colors.len() * 3);
    for c in face_colors {
        flat_colors.extend_from_slice(c);
    }

    let result = unsafe {
        ffi::printcad_occt_tessellate_brep(
            brep_blob.as_ptr(),
            brep_blob.len(),
            flat_colors.as_ptr(),
            face_colors.len(),
            linear_mode,
            linear_value,
            angular,
            weld_cross_face,
            weld_angle_rad,
            gen_edges,
        )
    };

    if !result.error.is_null() {
        let message = unsafe { CStr::from_ptr(result.error) }
            .to_string_lossy()
            .into_owned();
        unsafe { ffi::printcad_occt_free_result(result) };
        return Err(KernelError::Import(message));
    }

    let mesh = if result.body_count > 0 && !result.bodies.is_null() {
        let body = unsafe { &*result.bodies };
        tri_mesh_from_occt_body(body, detail)
    } else {
        TriMesh::default()
    };

    unsafe { ffi::printcad_occt_free_result(result) };

    if mesh.positions.is_empty() {
        return Err(KernelError::Import(
            "tessellation returned an empty mesh".into(),
        ));
    }

    Ok(mesh)
}

fn profile_segment_to_ffi(segment: &ProfileSegment) -> ffi::PcadProfileSegment {
    match *segment {
        ProfileSegment::Line { start, end } => ffi::PcadProfileSegment {
            kind: 0,
            d: [start[0], start[1], end[0], end[1], 0.0, 0.0],
        },
        ProfileSegment::Arc { start, mid, end } => ffi::PcadProfileSegment {
            kind: 1,
            d: [start[0], start[1], mid[0], mid[1], end[0], end[1]],
        },
        ProfileSegment::Circle { center, radius } => ffi::PcadProfileSegment {
            kind: 2,
            d: [center[0], center[1], radius, 0.0, 0.0, 0.0],
        },
    }
}

fn profile_plane_to_ffi(plane: &ProfilePlane) -> ffi::PcadProfilePlane {
    ffi::PcadProfilePlane {
        origin: plane.origin,
        x_axis: plane.x_axis,
        y_axis: plane.y_axis,
        normal: plane.normal,
    }
}

fn boolean_op_to_ffi(op: BooleanOp) -> i32 {
    match op {
        BooleanOp::NewSolid => 0,
        BooleanOp::Fuse => 1,
        BooleanOp::Cut => 2,
    }
}

fn execute_extrude_chain_internal(
    ops: &[ExtrudeOp],
    detail: &TessellationSettings,
) -> KernelResult<SolidBuildResult> {
    if ops.is_empty() {
        return Err(KernelError::InvalidInput("extrude chain is empty".into()));
    }
    if ops[0].op != BooleanOp::NewSolid {
        return Err(KernelError::InvalidInput(
            "first extrude op in a chain must be NewSolid".into(),
        ));
    }
    if let Some(offset) = ops[1..].iter().position(|op| op.op == BooleanOp::NewSolid) {
        return Err(KernelError::InvalidInput(format!(
            "extrude op {} is NewSolid but only the first op in a chain may be",
            offset + 1
        )));
    }

    let (linear_mode, linear_value) = tessellation_linear_for_ffi(detail);
    let angular = (detail.angular_tolerance_deg.max(0.5) as f64).to_radians();
    let weld_cross_face = if detail.weld_cross_face { 1 } else { 0 };
    let weld_angle_rad = (detail.weld_angle_threshold_deg.max(0.0) as f64).to_radians();
    let gen_edges = boundary_edges_for_ffi(detail);

    let mut current_blob: Vec<u8> = Vec::new();
    let mut final_mesh = TriMesh::default();

    for (index, extrude) in ops.iter().enumerate() {
        if extrude.wires.is_empty() {
            return Err(KernelError::InvalidInput(format!(
                "extrude op {index} has no profile wires"
            )));
        }

        // Segment arrays must outlive the FFI call; wires borrow into them.
        let segment_storage: Vec<Vec<ffi::PcadProfileSegment>> = extrude
            .wires
            .iter()
            .map(|wire| wire.segments.iter().map(profile_segment_to_ffi).collect())
            .collect();
        let ffi_wires: Vec<ffi::PcadProfileWire> = segment_storage
            .iter()
            .map(|segments| ffi::PcadProfileWire {
                segments: segments.as_ptr(),
                count: segments.len(),
            })
            .collect();
        let plane = profile_plane_to_ffi(&extrude.plane);

        let is_last = index + 1 == ops.len();
        let (base_ptr, base_len) = if index == 0 {
            (std::ptr::null(), 0)
        } else {
            (current_blob.as_ptr(), current_blob.len())
        };

        let result = unsafe {
            ffi::printcad_occt_extrude_profile(
                base_ptr,
                base_len,
                &plane,
                ffi_wires.as_ptr(),
                ffi_wires.len(),
                extrude.distance,
                boolean_op_to_ffi(extrude.op),
                if is_last { 1 } else { 0 },
                linear_mode,
                linear_value,
                angular,
                weld_cross_face,
                weld_angle_rad,
                gen_edges,
            )
        };

        if !result.error.is_null() {
            let message = unsafe { CStr::from_ptr(result.error) }
                .to_string_lossy()
                .into_owned();
            unsafe { ffi::printcad_occt_free_extrude_result(result) };
            return Err(KernelError::InvalidInput(format!(
                "extrude op {index}: {message}"
            )));
        }
        if result.brep_blob.is_null() || result.brep_len == 0 {
            unsafe { ffi::printcad_occt_free_extrude_result(result) };
            return Err(KernelError::InvalidInput(format!(
                "extrude op {index} returned no BRep snapshot"
            )));
        }

        current_blob =
            unsafe { std::slice::from_raw_parts(result.brep_blob, result.brep_len) }.to_vec();
        if is_last {
            final_mesh = TriMesh {
                positions: copy_vec3_array(result.mesh_positions, result.mesh_vertex_count),
                normals: copy_vec3_array(result.mesh_normals, result.mesh_vertex_count),
                indices: copy_u32_array(result.mesh_indices, result.mesh_index_count),
                edges: copy_u32_array(result.mesh_edges, result.mesh_edge_count * 2),
                colors: Vec::new(),
            };
        }
        unsafe { ffi::printcad_occt_free_extrude_result(result) };
    }

    if final_mesh.positions.is_empty() || final_mesh.indices.is_empty() {
        return Err(KernelError::InvalidInput(
            "extrude chain produced an empty render mesh".into(),
        ));
    }

    let bounds_mm = final_mesh.bounds();
    Ok(SolidBuildResult {
        brep_blob: current_blob,
        mesh: final_mesh,
        bounds_mm,
    })
}

fn import_step_brep_only_internal(
    path: &Path,
    detail: &TessellationSettings,
) -> KernelResult<ImportedModel> {
    let rust_total = Instant::now();

    let path_str = path.to_str().ok_or_else(|| {
        KernelError::InvalidInput(format!("STEP path is not valid UTF-8: {}", path.display()))
    })?;
    let c_path = CString::new(path_str).map_err(|err| {
        KernelError::InvalidInput(format!("STEP path contains a NUL byte: {err}"))
    })?;

    let header_start = Instant::now();
    let source_unit = step_header::detect_step_unit_from_path(path).ok().flatten();
    let header_ms = header_start.elapsed().as_secs_f64() * 1000.0;

    let (linear_mode, linear_value) = tessellation_linear_for_ffi(detail);
    let angular = (detail.angular_tolerance_deg.max(0.5) as f64).to_radians();
    let weld_cross_face = if detail.weld_cross_face { 1 } else { 0 };
    let weld_angle_rad = (detail.weld_angle_threshold_deg.max(0.0) as f64).to_radians();
    let serialize_brep = if detail.persist_brep_snapshot { 1 } else { 0 };
    let gen_edges = boundary_edges_for_ffi(detail);

    let ffi_start = Instant::now();
    let result = unsafe {
        ffi::printcad_occt_import_step_brep(
            c_path.as_ptr(),
            serialize_brep,
            linear_mode,
            linear_value,
            angular,
            weld_cross_face,
            weld_angle_rad,
            gen_edges,
        )
    };
    let occt_ffi_ms = ffi_start.elapsed().as_secs_f64() * 1000.0;

    if !result.error.is_null() {
        let message = unsafe { CStr::from_ptr(result.error) }
            .to_string_lossy()
            .into_owned();
        unsafe { ffi::printcad_occt_free_brep_import_result(result) };
        return Err(KernelError::Import(message));
    }

    let copy_start = Instant::now();
    let nodes = imported_nodes_from_brep_result(&result);
    let mut bodies = Vec::with_capacity(result.body_count);
    if result.body_count > 0 && !result.bodies.is_null() {
        let raw_bodies = unsafe { std::slice::from_raw_parts(result.bodies, result.body_count) };
        for body in raw_bodies {
            let name = if body.name.is_null() {
                None
            } else {
                Some(
                    unsafe { CStr::from_ptr(body.name) }
                        .to_string_lossy()
                        .into_owned(),
                )
            };
            let brep_blob = if body.brep_blob.is_null() || body.brep_len == 0 {
                Vec::new()
            } else {
                unsafe { std::slice::from_raw_parts(body.brep_blob, body.brep_len) }.to_vec()
            };
            let face_colors = copy_vec3_array(body.face_colors, body.face_count);
            let bounds_mm = Some((body.bbox_min, body.bbox_max));
            let mesh = tri_mesh_from_brep_inline(body, detail);
            bodies.push(ImportedBody {
                name,
                mesh,
                brep_blob,
                face_colors,
                bounds_mm,
            });
        }
    }
    let rust_copy_ms = copy_start.elapsed().as_secs_f64() * 1000.0;
    unsafe { ffi::printcad_occt_free_brep_import_result(result) };

    let rust_total_ms = rust_total.elapsed().as_secs_f64() * 1000.0;
    info!(
        path = %path.display(),
        header_scan_ms = format!("{header_ms:.2}"),
        occt_brep_ffi_ms = format!("{occt_ffi_ms:.2}"),
        rust_brep_copy_ms = format!("{rust_copy_ms:.2}"),
        rust_import_total_ms = format!("{rust_total_ms:.2}"),
        "STEP fast import (BRep) timing (Rust; C++ stderr: [printcad_import_brep_cpp])"
    );

    let note = if detail.persist_brep_snapshot {
        "BRep snapshot serialized; tessellation may follow in worker"
    } else {
        "session import (mesh inline; BRep snapshot skipped)"
    };
    info!(
        "Imported STEP `{}`: {} bodies ({note}), source unit {:?}",
        path.display(),
        bodies.len(),
        source_unit,
    );

    Ok(ImportedModel {
        bodies,
        nodes,
        source_unit,
    })
}

fn import_step_full_mesh_internal(
    path: &Path,
    detail: &TessellationSettings,
) -> KernelResult<ImportedModel> {
    let rust_total = Instant::now();

    let path_str = path.to_str().ok_or_else(|| {
        KernelError::InvalidInput(format!("STEP path is not valid UTF-8: {}", path.display()))
    })?;
    let c_path = CString::new(path_str).map_err(|err| {
        KernelError::InvalidInput(format!("STEP path contains a NUL byte: {err}"))
    })?;

    let header_start = Instant::now();
    let source_unit = step_header::detect_step_unit_from_path(path).ok().flatten();
    let header_ms = header_start.elapsed().as_secs_f64() * 1000.0;

    let (linear_mode, linear_value) = tessellation_linear_for_ffi(detail);
    let angular = (detail.angular_tolerance_deg.max(0.5) as f64).to_radians();
    let weld_cross_face = if detail.weld_cross_face { 1 } else { 0 };
    let weld_angle_rad = (detail.weld_angle_threshold_deg.max(0.0) as f64).to_radians();
    let gen_edges = boundary_edges_for_ffi(detail);

    let ffi_start = Instant::now();
    let result = unsafe {
        ffi::printcad_occt_import_step(
            c_path.as_ptr(),
            linear_mode,
            linear_value,
            angular,
            weld_cross_face,
            weld_angle_rad,
            gen_edges,
        )
    };
    let occt_ffi_ms = ffi_start.elapsed().as_secs_f64() * 1000.0;

    if !result.error.is_null() {
        let message = unsafe { CStr::from_ptr(result.error) }
            .to_string_lossy()
            .into_owned();
        unsafe { ffi::printcad_occt_free_result(result) };
        return Err(KernelError::Import(message));
    }

    let copy_start = Instant::now();
    let nodes = imported_nodes_from_import_result(&result);
    let mut bodies = Vec::with_capacity(result.body_count);
    if result.body_count > 0 && !result.bodies.is_null() {
        let raw_bodies = unsafe { std::slice::from_raw_parts(result.bodies, result.body_count) };
        for body in raw_bodies {
            let mesh = tri_mesh_from_occt_body(body, detail);
            let name = if body.name.is_null() {
                None
            } else {
                Some(
                    unsafe { CStr::from_ptr(body.name) }
                        .to_string_lossy()
                        .into_owned(),
                )
            };
            let bounds_mm = mesh.bounds();
            bodies.push(ImportedBody {
                name,
                mesh,
                brep_blob: Vec::new(),
                face_colors: Vec::new(),
                bounds_mm,
            });
        }
    }

    let rust_mesh_copy_ms = copy_start.elapsed().as_secs_f64() * 1000.0;

    unsafe { ffi::printcad_occt_free_result(result) };

    let rust_total_ms = rust_total.elapsed().as_secs_f64() * 1000.0;

    info!(
        path = %path.display(),
        header_scan_ms = format!("{header_ms:.2}"),
        occt_ffi_ms = format!("{occt_ffi_ms:.2}"),
        rust_mesh_copy_ms = format!("{rust_mesh_copy_ms:.2}"),
        rust_import_total_ms = format!("{rust_total_ms:.2}"),
        "STEP full import timing (Rust; C++ stderr: [printcad_import_cpp])"
    );

    info!(
        "Imported STEP `{}` (full mesh): {} bodies, {} triangles, source unit {:?}",
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
        nodes,
        source_unit,
    })
}

fn imported_nodes_from_brep_result(
    result: &ffi::PrintcadOcctBrepImportResult,
) -> Vec<ImportedNode> {
    let mut nodes = Vec::new();
    if result.node_count > 0 && !result.nodes.is_null() {
        let raw_nodes = unsafe { std::slice::from_raw_parts(result.nodes, result.node_count) };
        nodes.extend(raw_nodes.iter().map(imported_node_from_ffi));
    }

    if nodes.is_empty() && result.body_count > 0 {
        // Backward-compat fallback while C++ hierarchy export rolls out.
        nodes.extend((0..result.body_count).map(|idx| ImportedNode {
            id: (idx + 1) as u64,
            parent_id: None,
            name: Some(format!("Body {}", idx + 1)),
            kind: ImportedNodeKind::Part,
            visible: true,
            body_index: Some(idx),
            local_transform: None,
        }));
    }
    nodes
}

fn imported_nodes_from_import_result(result: &ffi::PrintcadOcctImportResult) -> Vec<ImportedNode> {
    if result.node_count > 0 && !result.nodes.is_null() {
        let raw_nodes = unsafe { std::slice::from_raw_parts(result.nodes, result.node_count) };
        return raw_nodes.iter().map(imported_node_from_ffi).collect();
    }
    (0..result.body_count)
        .map(|idx| ImportedNode {
            id: (idx + 1) as u64,
            parent_id: None,
            name: Some(format!("Body {}", idx + 1)),
            kind: ImportedNodeKind::Part,
            visible: true,
            body_index: Some(idx),
            local_transform: None,
        })
        .collect()
}

fn imported_node_from_ffi(raw: &ffi::PrintcadOcctImportNode) -> ImportedNode {
    let name = if raw.name.is_null() {
        None
    } else {
        Some(
            unsafe { CStr::from_ptr(raw.name) }
                .to_string_lossy()
                .trim()
                .to_string(),
        )
        .filter(|s| !s.is_empty())
    };
    let kind = match raw.kind {
        0 => ImportedNodeKind::Assembly,
        2 => ImportedNodeKind::Instance,
        _ => ImportedNodeKind::Part,
    };
    let body_index = if raw.body_index >= 0 {
        Some(raw.body_index as usize)
    } else {
        None
    };
    let parent_id = if raw.parent_id >= 0 {
        Some(raw.parent_id as u64)
    } else {
        None
    };
    let local_transform = if raw.has_local_transform != 0 {
        Some([
            [
                raw.local_transform[0],
                raw.local_transform[1],
                raw.local_transform[2],
                raw.local_transform[3],
            ],
            [
                raw.local_transform[4],
                raw.local_transform[5],
                raw.local_transform[6],
                raw.local_transform[7],
            ],
            [
                raw.local_transform[8],
                raw.local_transform[9],
                raw.local_transform[10],
                raw.local_transform[11],
            ],
            [
                raw.local_transform[12],
                raw.local_transform[13],
                raw.local_transform[14],
                raw.local_transform[15],
            ],
        ])
    } else {
        None
    };

    ImportedNode {
        id: raw.id,
        parent_id,
        name,
        kind,
        visible: raw.visible != 0,
        body_index,
        local_transform,
    }
}

fn tri_mesh_from_brep_inline(
    body: &ffi::PrintcadOcctBrepBody,
    detail: &TessellationSettings,
) -> TriMesh {
    if body.mesh_vertex_count == 0 || body.mesh_positions.is_null() {
        return TriMesh::default();
    }
    let occt = ffi::PrintcadOcctBody {
        name: std::ptr::null_mut(),
        positions: body.mesh_positions,
        normals: body.mesh_normals,
        colors: body.mesh_colors,
        indices: body.mesh_indices,
        edges: body.mesh_edges,
        vertex_count: body.mesh_vertex_count,
        index_count: body.mesh_index_count,
        edge_count: body.mesh_edge_count,
    };
    tri_mesh_from_occt_body(&occt, detail)
}

fn tri_mesh_from_occt_body(body: &ffi::PrintcadOcctBody, detail: &TessellationSettings) -> TriMesh {
    let positions = copy_vec3_array(body.positions, body.vertex_count);
    let normals = copy_vec3_array(body.normals, body.vertex_count);
    let mut colors = copy_vec3_array(body.colors, body.vertex_count);
    if colors.len() != positions.len() && !positions.is_empty() {
        colors = vec![[1.0, 1.0, 1.0]; positions.len()];
    }
    let indices = copy_u32_array(body.indices, body.index_count);
    let edges = copy_u32_array(body.edges, body.edge_count * 2);

    let triangle_count = indices.len() / 3;
    if triangle_count > 0 {
        let ratio = positions.len() as f64 / triangle_count as f64;
        let label = if body.name.is_null() {
            "<unnamed>".to_string()
        } else {
            unsafe { CStr::from_ptr(body.name) }
                .to_string_lossy()
                .into_owned()
        };
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

    TriMesh {
        positions,
        normals,
        indices,
        edges,
        colors,
    }
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
