// printCAD OCCT shim: minimal C ABI for STEP import + tessellation.
//
// Memory ownership: all output arrays are heap-allocated by the shim using
// `std::malloc` and must be released by the caller via `printcad_occt_free_*`
// helpers. This avoids cross-allocator issues between the Rust and C++ sides.

#ifndef PRINTCAD_OCCT_STEP_LOADER_H
#define PRINTCAD_OCCT_STEP_LOADER_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// One imported body. `positions`, `normals` are flat arrays of length
// `vertex_count * 3`. `indices` is a flat array of length `index_count`.
// `edges` is a flat list of vertex-index pairs (length `edge_count * 2`)
// describing face-boundary line segments and is computed before any vertex
// welding so hard CAD edges stay visible. `colors` is flat RGB linear 0..1,
// length `vertex_count * 3`, parallel to positions. `name` may be null when no STEP
// product label was attached.
typedef struct PrintcadOcctBody {
    char* name;
    float* positions;
    float* normals;
    float* colors;
    uint32_t* indices;
    uint32_t* edges;
    size_t vertex_count;
    size_t index_count;
    size_t edge_count;
} PrintcadOcctBody;

// One imported hierarchy node reconstructed from XCAF labels/components.
// `parent_id` is -1 for roots. `kind`: 0=assembly, 1=part, 2=instance.
// `body_index` points into the companion body array when this node owns
// renderable payload, or -1 when it is a pure container.
// `local_transform` is row-major 4x4.
typedef struct PrintcadOcctImportNode {
    uint64_t id;
    int64_t parent_id;
    char* name;
    int kind;
    int visible;
    int64_t body_index;
    int has_local_transform;
    float local_transform[16];
} PrintcadOcctImportNode;

typedef struct PrintcadOcctImportResult {
    PrintcadOcctBody* bodies;
    size_t body_count;
    PrintcadOcctImportNode* nodes;
    size_t node_count;
    char* error;
} PrintcadOcctImportResult;

// Read a STEP/STP file and tessellate every solid/shell face.
// `linear_deflection_mode`: 0 = bbox-scaled chord deflection
// (linear_value is the dimensionless mesh-deviation multiplier, typically 0.2);
// 1 = absolute OCCT linear deflection in model units (chord height in mm).
// `angular_deflection_rad` controls angular sampling.
// When `weld_cross_face` is non-zero the shim merges coincident vertices
// across face boundaries whose face normals are within
// `weld_angle_threshold_rad` of each other (so hard CAD edges remain
// distinct). Pass 0 to skip welding entirely.
// When `generate_boundary_edges` is 0, face-boundary line segments are not
// computed (faster on large meshes).
// On success, `error` is null and `bodies`/`body_count` are populated.
// On failure, `error` is a non-null malloc'd string and `bodies` is null.
PrintcadOcctImportResult printcad_occt_import_step(
    const char* utf8_path,
    int linear_deflection_mode,
    double linear_value,
    double angular_deflection_rad,
    int weld_cross_face,
    double weld_angle_threshold_rad,
    int generate_boundary_edges);

// Fast STEP import: read/transfer + XCAF colour snapshot per body.
// When `serialize_brep` is non-zero: `BRepTools::Write` each body (`brep_blob`);
// mesh fields are null and tessellation is deferred (slower on huge models).
// When `serialize_brep` is zero: skip BRep write; mesh in memory and fill
// `mesh_*` (session-fast path; `brep_blob` null).
typedef struct PrintcadOcctBrepBody {
    char* name;
    uint8_t* brep_blob;
    size_t brep_len;
    float bbox_min[3];
    float bbox_max[3];
    float* face_colors;
    size_t face_count;
    float* mesh_positions;
    float* mesh_normals;
    float* mesh_colors;
    uint32_t* mesh_indices;
    uint32_t* mesh_edges;
    size_t mesh_vertex_count;
    size_t mesh_index_count;
    size_t mesh_edge_count;
} PrintcadOcctBrepBody;

typedef struct PrintcadOcctBrepImportResult {
    PrintcadOcctBrepBody* bodies;
    size_t body_count;
    PrintcadOcctImportNode* nodes;
    size_t node_count;
    char* error;
} PrintcadOcctBrepImportResult;

PrintcadOcctBrepImportResult printcad_occt_import_step_brep(
    const char* utf8_path,
    int serialize_brep,
    int linear_deflection_mode,
    double linear_value,
    double angular_deflection_rad,
    int weld_cross_face,
    double weld_angle_threshold_rad,
    int generate_boundary_edges);

// Tessellate one body from a BRepTools binary + per-face RGB snapshot (same
// face order as `TopExp_Explorer` over `TopAbs_FACE`). Fills one entry in
// `PrintcadOcctImportResult::bodies` or returns `error`.
// `linear_deflection_mode`: 0 = bbox-scaled (linear_value = mesh deviation, e.g. 0.2),
//                           1 = absolute linear deflection in model units.
PrintcadOcctImportResult printcad_occt_tessellate_brep(
    const uint8_t* brep_bytes,
    size_t brep_len,
    const float* face_colors,
    size_t face_color_count,
    int linear_deflection_mode,
    double linear_value,
    double angular_deflection_rad,
    int weld_cross_face,
    double weld_angle_threshold_rad,
    int generate_boundary_edges);

// ---- Sketch-profile extrusion (pad/pocket) ----

// One profile segment in sketch-plane (u, v) millimetre coordinates.
// kind 0 = line:   d = { start_u, start_v, end_u, end_v, 0, 0 }
// kind 1 = arc:    d = { start_u, start_v, mid_u, mid_v, end_u, end_v }
//                  (three on-curve points: start -> mid -> end)
// kind 2 = circle: d = { center_u, center_v, radius, 0, 0, 0 }
typedef struct PcadProfileSegment {
    int32_t kind;
    double d[6];
} PcadProfileSegment;

// A closed loop of consecutive segments (a single circle is a wire by itself).
typedef struct PcadProfileWire {
    const PcadProfileSegment* segments;
    size_t count;
} PcadProfileWire;

// World-space plane the profile lives on (millimetres):
// 3D point = origin + u * x_axis + v * y_axis.
typedef struct PcadProfilePlane {
    double origin[3];
    double x_axis[3];
    double y_axis[3];
    double normal[3];
} PcadProfilePlane;

// Result of one extrude step. On success `error` is null and `brep_blob` holds
// the `BRepTools::Write` snapshot of the resulting solid; the `mesh_*` arrays
// are populated only when `want_mesh != 0` (same layout as `PrintcadOcctBody`,
// no per-vertex colours). On failure `error` is a malloc'd string and every
// other field is null/zero.
typedef struct PrintcadOcctExtrudeResult {
    uint8_t* brep_blob;
    size_t brep_len;
    float* mesh_positions;
    float* mesh_normals;
    uint32_t* mesh_indices;
    uint32_t* mesh_edges;
    size_t mesh_vertex_count;
    size_t mesh_index_count;
    size_t mesh_edge_count;
    char* error;
} PrintcadOcctExtrudeResult;

// Extrude a closed sketch profile into a prism and combine it with an optional
// base solid. `op`: 0 = new solid (base_brep must be NULL), 1 = fuse,
// 2 = cut (base_brep required for 1/2). `distance` is measured along
// `plane->normal` and may be negative to extrude backwards. The largest-area
// wire is the outer boundary; the remaining wires become holes. Tessellation
// parameters mirror `printcad_occt_tessellate_brep` and are only used when
// `want_mesh != 0`.
PrintcadOcctExtrudeResult printcad_occt_extrude_profile(
    const uint8_t* base_brep,
    size_t base_brep_len,
    const PcadProfilePlane* plane,
    const PcadProfileWire* wires,
    size_t wire_count,
    double distance,
    int32_t op,
    int want_mesh,
    int linear_deflection_mode,
    double linear_value,
    double angular_deflection_rad,
    int weld_cross_face,
    double weld_angle_threshold_rad,
    int generate_boundary_edges);

// Free helpers — every output buffer must be released exactly once.
void printcad_occt_free_string(char* str);
void printcad_occt_free_extrude_result(PrintcadOcctExtrudeResult result);
void printcad_occt_free_result(PrintcadOcctImportResult result);
void printcad_occt_free_brep_import_result(PrintcadOcctBrepImportResult result);

#ifdef __cplusplus
}
#endif

#endif // PRINTCAD_OCCT_STEP_LOADER_H
