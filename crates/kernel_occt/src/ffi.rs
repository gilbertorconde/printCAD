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
    pub(crate) error: *mut c_char,
}

extern "C" {
    pub(crate) fn printcad_occt_import_step(
        utf8_path: *const c_char,
        linear_deflection: c_double,
        angular_deflection_rad: c_double,
        weld_cross_face: c_int,
        weld_angle_threshold_rad: c_double,
    ) -> PrintcadOcctImportResult;

    pub(crate) fn printcad_occt_free_result(result: PrintcadOcctImportResult);
}
