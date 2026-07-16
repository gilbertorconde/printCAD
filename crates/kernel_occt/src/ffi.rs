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

/// One profile segment in sketch-plane (u, v) millimetre coordinates.
/// `kind` 0 = line (`d = [su, sv, eu, ev, 0, 0]`), 1 = arc through three
/// on-curve points (`d = [su, sv, mu, mv, eu, ev]`), 2 = circle
/// (`d = [cu, cv, radius, 0, 0, 0]`), 3 = ellipse
/// (`d = [cu, cv, mu, mv, ratio, 0]`), 4 = ellipse arc (d as 3, `extra` =
/// start/end params), 5 = B-spline (`d[0]` = periodic, `extra` = flat uv
/// control points).
#[repr(C)]
pub(crate) struct PcadProfileSegment {
    pub(crate) kind: i32,
    pub(crate) d: [c_double; 6],
    pub(crate) extra: *const c_double,
    pub(crate) extra_count: usize,
}

#[repr(C)]
pub(crate) struct PcadProfileWire {
    pub(crate) segments: *const PcadProfileSegment,
    pub(crate) count: usize,
}

#[repr(C)]
pub(crate) struct PcadProfilePlane {
    pub(crate) origin: [c_double; 3],
    pub(crate) x_axis: [c_double; 3],
    pub(crate) y_axis: [c_double; 3],
    pub(crate) normal: [c_double; 3],
}

/// Result of one solid-op step: `brep_blob` always on success, `tool_blob`
/// when the op produced a standalone tool solid, `mesh_*` arrays only when
/// requested. Freed via `printcad_occt_free_sweep_result`.
#[repr(C)]
pub(crate) struct PrintcadOcctSweepResult {
    pub(crate) brep_blob: *mut u8,
    pub(crate) brep_len: usize,
    pub(crate) tool_blob: *mut u8,
    pub(crate) tool_len: usize,
    pub(crate) mesh_positions: *mut f32,
    pub(crate) mesh_normals: *mut f32,
    pub(crate) mesh_indices: *mut u32,
    pub(crate) mesh_edges: *mut u32,
    pub(crate) mesh_vertex_count: usize,
    pub(crate) mesh_index_count: usize,
    pub(crate) mesh_edge_count: usize,
    pub(crate) error: *mut c_char,
}

/// Tessellation request shared by every solid-op entry point.
#[repr(C)]
pub(crate) struct PcadMeshOptions {
    pub(crate) want_mesh: c_int,
    pub(crate) linear_deflection_mode: c_int,
    pub(crate) linear_value: c_double,
    pub(crate) angular_deflection_rad: c_double,
    pub(crate) weld_cross_face: c_int,
    pub(crate) weld_angle_threshold_rad: c_double,
    pub(crate) generate_boundary_edges: c_int,
}

/// Extrusion termination: `kind` 0 = blind, 1 = through-all, 2 = up-to-plane,
/// 3 = to-first, 4 = to-last.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct PcadTermination {
    pub(crate) kind: i32,
    pub(crate) distance: c_double,
    pub(crate) plane_point: [c_double; 3],
    pub(crate) plane_normal: [c_double; 3],
    pub(crate) offset: c_double,
}

/// Sweep description: `kind` 0 = extrude, 1 = revolve, 2 = helix.
#[repr(C)]
pub(crate) struct PcadSweepDesc {
    pub(crate) kind: i32,
    pub(crate) term: PcadTermination,
    pub(crate) term2: PcadTermination,
    pub(crate) has_term2: i32,
    pub(crate) symmetric: i32,
    pub(crate) reversed: i32,
    pub(crate) taper_deg: c_double,
    pub(crate) direction: [c_double; 3],
    pub(crate) has_direction: i32,
    pub(crate) axis_origin: [c_double; 2],
    pub(crate) axis_dir: [c_double; 2],
    pub(crate) angle_deg: c_double,
    pub(crate) angle2_deg: c_double,
    pub(crate) has_angle2: i32,
    pub(crate) midplane: i32,
    pub(crate) pitch: c_double,
    pub(crate) height: c_double,
    pub(crate) cone_angle_deg: c_double,
    pub(crate) left_handed: i32,
}

/// One tool solid re-applied by a pattern.
#[repr(C)]
pub(crate) struct PcadToolSolid {
    pub(crate) brep: *const u8,
    pub(crate) len: usize,
    pub(crate) subtractive: i32,
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

    pub(crate) fn printcad_occt_solid_sweep(
        base_brep: *const u8,
        base_brep_len: usize,
        plane: *const PcadProfilePlane,
        wires: *const PcadProfileWire,
        wire_count: usize,
        desc: *const PcadSweepDesc,
        op: i32,
        mesh: *const PcadMeshOptions,
    ) -> PrintcadOcctSweepResult;

    pub(crate) fn printcad_occt_solid_loft(
        base_brep: *const u8,
        base_brep_len: usize,
        planes: *const PcadProfilePlane,
        wires: *const PcadProfileWire,
        wire_offsets: *const usize,
        wire_counts: *const usize,
        section_count: usize,
        ruled: c_int,
        closed: c_int,
        op: i32,
        mesh: *const PcadMeshOptions,
    ) -> PrintcadOcctSweepResult;

    pub(crate) fn printcad_occt_solid_pipe(
        base_brep: *const u8,
        base_brep_len: usize,
        profile_plane: *const PcadProfilePlane,
        profile_wires: *const PcadProfileWire,
        profile_wire_count: usize,
        spine_plane: *const PcadProfilePlane,
        spine_wire: *const PcadProfileWire,
        frenet: c_int,
        op: i32,
        mesh: *const PcadMeshOptions,
    ) -> PrintcadOcctSweepResult;

    pub(crate) fn printcad_occt_solid_primitive(
        base_brep: *const u8,
        base_brep_len: usize,
        kind: i32,
        params: *const c_double,
        param_count: usize,
        placement: *const c_double,
        op: i32,
        mesh: *const PcadMeshOptions,
    ) -> PrintcadOcctSweepResult;

    pub(crate) fn printcad_occt_dressup(
        base_brep: *const u8,
        base_brep_len: usize,
        kind: i32,
        params: *const c_double,
        param_count: usize,
        selection_mode: i32,
        points: *const c_double,
        point_count: usize,
        mesh: *const PcadMeshOptions,
    ) -> PrintcadOcctSweepResult;

    pub(crate) fn printcad_occt_draft(
        base_brep: *const u8,
        base_brep_len: usize,
        angle_deg: c_double,
        neutral_point: *const c_double,
        neutral_normal: *const c_double,
        pull_dir: *const c_double,
        face_points: *const c_double,
        face_point_count: usize,
        mesh: *const PcadMeshOptions,
    ) -> PrintcadOcctSweepResult;

    pub(crate) fn printcad_occt_thickness(
        base_brep: *const u8,
        base_brep_len: usize,
        value: c_double,
        inward: c_int,
        face_points: *const c_double,
        face_point_count: usize,
        mesh: *const PcadMeshOptions,
    ) -> PrintcadOcctSweepResult;

    pub(crate) fn printcad_occt_pattern(
        base_brep: *const u8,
        base_brep_len: usize,
        tools: *const PcadToolSolid,
        tool_count: usize,
        transforms: *const c_double,
        transform_count: usize,
        mesh: *const PcadMeshOptions,
    ) -> PrintcadOcctSweepResult;

    pub(crate) fn printcad_occt_boolean(
        base_brep: *const u8,
        base_brep_len: usize,
        tool_brep: *const u8,
        tool_len: usize,
        kind: i32,
        mesh: *const PcadMeshOptions,
    ) -> PrintcadOcctSweepResult;

    pub(crate) fn printcad_occt_free_result(result: PrintcadOcctImportResult);

    pub(crate) fn printcad_occt_free_brep_import_result(result: PrintcadOcctBrepImportResult);

    pub(crate) fn printcad_occt_free_sweep_result(result: PrintcadOcctSweepResult);
}
