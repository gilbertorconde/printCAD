//! Raw FFI declarations for the OpenCASCADE C++ shim.
//!
//! The matching C definitions live in `cpp/step_loader.h`. Memory ownership is
//! intentionally simple: every output buffer is allocated with `malloc` on the
//! C++ side and must be released by calling `printcad_occt_free_result`.

use std::os::raw::{c_char, c_double, c_int};

#[repr(C)]
pub(crate) struct PrintcadOcctBody {
    pub(crate) name: *mut c_char,
    pub(crate) positions: *mut f32,
    pub(crate) normals: *mut f32,
    pub(crate) colors: *mut f32,
    pub(crate) indices: *mut u32,
    pub(crate) edges: *mut u32,
    pub(crate) vertex_count: usize,
    pub(crate) index_count: usize,
    pub(crate) edge_count: usize,
}

#[repr(C)]
pub(crate) struct PrintcadOcctImportResult {
    pub(crate) bodies: *mut PrintcadOcctBody,
    pub(crate) body_count: usize,
    pub(crate) nodes: *mut PrintcadOcctImportNode,
    pub(crate) node_count: usize,
    pub(crate) error: *mut c_char,
}

#[repr(C)]
pub(crate) struct PrintcadOcctImportNode {
    pub(crate) id: u64,
    pub(crate) parent_id: i64,
    pub(crate) name: *mut c_char,
    pub(crate) kind: c_int,
    pub(crate) visible: c_int,
    pub(crate) body_index: i64,
    pub(crate) has_local_transform: c_int,
    pub(crate) local_transform: [f32; 16],
}

#[repr(C)]
pub(crate) struct PrintcadOcctBrepBody {
    pub(crate) name: *mut c_char,
    pub(crate) brep_blob: *mut u8,
    pub(crate) brep_len: usize,
    pub(crate) bbox_min: [f32; 3],
    pub(crate) bbox_max: [f32; 3],
    pub(crate) face_colors: *mut f32,
    pub(crate) face_count: usize,
    pub(crate) mesh_positions: *mut f32,
    pub(crate) mesh_normals: *mut f32,
    pub(crate) mesh_colors: *mut f32,
    pub(crate) mesh_indices: *mut u32,
    pub(crate) mesh_edges: *mut u32,
    pub(crate) mesh_vertex_count: usize,
    pub(crate) mesh_index_count: usize,
    pub(crate) mesh_edge_count: usize,
}

#[repr(C)]
pub(crate) struct PrintcadOcctBrepImportResult {
    pub(crate) bodies: *mut PrintcadOcctBrepBody,
    pub(crate) body_count: usize,
    pub(crate) nodes: *mut PrintcadOcctImportNode,
    pub(crate) node_count: usize,
    pub(crate) error: *mut c_char,
}

extern "C" {
    pub(crate) fn printcad_occt_import_step(
        utf8_path: *const c_char,
        linear_deflection_mode: c_int,
        linear_value: c_double,
        angular_deflection_rad: c_double,
        weld_cross_face: c_int,
        weld_angle_threshold_rad: c_double,
        generate_boundary_edges: c_int,
    ) -> PrintcadOcctImportResult;

    pub(crate) fn printcad_occt_import_step_brep(
        utf8_path: *const c_char,
        serialize_brep: c_int,
        linear_deflection_mode: c_int,
        linear_value: c_double,
        angular_deflection_rad: c_double,
        weld_cross_face: c_int,
        weld_angle_threshold_rad: c_double,
        generate_boundary_edges: c_int,
    ) -> PrintcadOcctBrepImportResult;

    pub(crate) fn printcad_occt_tessellate_brep(
        brep_bytes: *const u8,
        brep_len: usize,
        face_colors: *const f32,
        face_color_count: usize,
        linear_deflection_mode: c_int,
        linear_value: c_double,
        angular_deflection_rad: c_double,
        weld_cross_face: c_int,
        weld_angle_threshold_rad: c_double,
        generate_boundary_edges: c_int,
    ) -> PrintcadOcctImportResult;

    pub(crate) fn printcad_occt_free_result(result: PrintcadOcctImportResult);

    pub(crate) fn printcad_occt_free_brep_import_result(result: PrintcadOcctBrepImportResult);
}
