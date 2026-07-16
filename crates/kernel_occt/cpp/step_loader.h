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

// ---- Sketch-profile solid sweeps (pad/pocket/revolve/groove) ----

// One profile segment in sketch-plane (u, v) millimetre coordinates.
// kind 0 = line:    d = { start_u, start_v, end_u, end_v, 0, 0 }
// kind 1 = arc:     d = { start_u, start_v, mid_u, mid_v, end_u, end_v }
//                   (three on-curve points: start -> mid -> end)
// kind 2 = circle:  d = { center_u, center_v, radius, 0, 0, 0 }
// kind 3 = ellipse: d = { center_u, center_v, major_u, major_v, ratio, 0 }
//                   (major = vector center -> major vertex, minor = |major|*ratio)
// kind 4 = ellipse arc: d as kind 3, extra = { start_param, end_param } (radians)
// kind 5 = cubic B-spline: d[0] != 0 => periodic, extra = flat (u, v) control points
typedef struct PcadProfileSegment {
    int32_t kind;
    double d[6];
    const double* extra;
    size_t extra_count;
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

// Result of one solid-op step. On success `error` is null, `brep_blob` holds
// the `BRepTools::Write` snapshot of the resulting solid, and `tool_blob`
// (when the op produced a standalone tool solid before the boolean) holds the
// raw tool shape for later re-use by patterns. The `mesh_*` arrays are
// populated only when meshing was requested (same layout as
// `PrintcadOcctBody`, no per-vertex colours). On failure `error` is a
// malloc'd string and every other field is null/zero.
typedef struct PrintcadOcctSweepResult {
    uint8_t* brep_blob;
    size_t brep_len;
    uint8_t* tool_blob;
    size_t tool_len;
    float* mesh_positions;
    float* mesh_normals;
    uint32_t* mesh_indices;
    uint32_t* mesh_edges;
    size_t mesh_vertex_count;
    size_t mesh_index_count;
    size_t mesh_edge_count;
    char* error;
} PrintcadOcctSweepResult;

// Tessellation request shared by every solid-op entry point. Parameters
// mirror `printcad_occt_tessellate_brep`; the mesh is only produced when
// `want_mesh != 0`.
typedef struct PcadMeshOptions {
    int want_mesh;
    int linear_deflection_mode;
    double linear_value;
    double angular_deflection_rad;
    int weld_cross_face;
    double weld_angle_threshold_rad;
    int generate_boundary_edges;
} PcadMeshOptions;

// Where a one-directional extrusion stops.
// kind 0 = blind (`distance` mm), 1 = through-all (past the base bbox),
// 2 = up-to-plane (`plane_point`/`plane_normal` shifted by `offset`),
// 3 = to-first / 4 = to-last planar base face hit along the sweep direction.
typedef struct PcadTermination {
    int32_t kind;
    double distance;
    double plane_point[3];
    double plane_normal[3];
    double offset;
} PcadTermination;

// How a profile becomes a solid. `kind`: 0 = extrude, 1 = revolve, 2 = helix.
// Extrude uses term/term2 (+`has_term2` for two-sided), `symmetric`,
// `reversed`, `taper_deg`, and optional custom `direction`. Revolve uses the
// sketch-plane axis, `angle_deg` (+optional `angle2_deg`), `midplane`,
// `reversed`. Helix uses the axis plus `pitch`/`height`/`cone_angle_deg`/
// `left_handed`.
typedef struct PcadSweepDesc {
    int32_t kind;
    PcadTermination term;
    PcadTermination term2;
    int32_t has_term2;
    int32_t symmetric;
    int32_t reversed;
    double taper_deg;
    double direction[3];
    int32_t has_direction;
    double axis_origin[2];
    double axis_dir[2];
    double angle_deg;
    double angle2_deg;
    int32_t has_angle2;
    int32_t midplane;
    double pitch;
    double height;
    double cone_angle_deg;
    int32_t left_handed;
} PcadSweepDesc;

// `op` for every shape-producing entry point below:
// 0 = new solid (base_brep must be NULL), 1 = fuse, 2 = cut
// (base required for 1/2).

// Sweep a closed sketch profile into a solid (extrude / revolve / helix) and
// combine it with the optional base. The largest-area wire is the outer
// boundary; the remaining wires become holes.
PrintcadOcctSweepResult printcad_occt_solid_sweep(
    const uint8_t* base_brep,
    size_t base_brep_len,
    const PcadProfilePlane* plane,
    const PcadProfileWire* wires,
    size_t wire_count,
    const PcadSweepDesc* desc,
    int32_t op,
    const PcadMeshOptions* mesh);

// Loft through 2+ section profiles (planes[i] owns wires starting at
// wire_offsets[i], wire_counts[i] wires). Hole wires loft pairwise (by
// descending area) only when every section has the same wire count.
// `closed != 0` loops the last section back to the first.
PrintcadOcctSweepResult printcad_occt_solid_loft(
    const uint8_t* base_brep,
    size_t base_brep_len,
    const PcadProfilePlane* planes,
    const PcadProfileWire* wires,
    const size_t* wire_offsets,
    const size_t* wire_counts,
    size_t section_count,
    int ruled,
    int closed,
    int32_t op,
    const PcadMeshOptions* mesh);

// Sweep a profile along a spine wire from another sketch. The spine may be
// open or closed; `frenet != 0` uses a Frenet frame, otherwise the corrected
// frame.
PrintcadOcctSweepResult printcad_occt_solid_pipe(
    const uint8_t* base_brep,
    size_t base_brep_len,
    const PcadProfilePlane* profile_plane,
    const PcadProfileWire* profile_wires,
    size_t profile_wire_count,
    const PcadProfilePlane* spine_plane,
    const PcadProfileWire* spine_wire,
    int frenet,
    int32_t op,
    const PcadMeshOptions* mesh);

// Parametric primitive. `placement` = origin[3], x_axis[3], z_axis[3].
// kind / params:
// 0 box{l,w,h} 1 cylinder{r,h,angle} 2 sphere{r,a1,a2,a3} 3 cone{r1,r2,h,angle}
// 4 torus{r1,r2,a1,a2,a3} 5 ellipsoid{r1,r2,r3} 6 prism{sides,circumradius,h}
// 7 wedge{xmin,xmax,ymin,ymax,zmin,zmax,x2min,x2max,z2min,z2max}
// (angles degrees).
PrintcadOcctSweepResult printcad_occt_solid_primitive(
    const uint8_t* base_brep,
    size_t base_brep_len,
    int32_t kind,
    const double* params,
    size_t param_count,
    const double placement[9],
    int32_t op,
    const PcadMeshOptions* mesh);

// Fillet or chamfer edges of the base solid, selected geometrically.
// `kind`: 0 = fillet (params = {radius}), 1 = chamfer (params = {mode, d1,
// d2_or_angle_deg, flip}; mode 0 equal-distance, 1 two-distances,
// 2 distance+angle). `selection_mode`: 0 = all edges, 1 = edges of the faces
// nearest each xyz in `points`, 2 = the single edge nearest each point.
PrintcadOcctSweepResult printcad_occt_dressup(
    const uint8_t* base_brep,
    size_t base_brep_len,
    int32_t kind,
    const double* params,
    size_t param_count,
    int32_t selection_mode,
    const double* points,
    size_t point_count,
    const PcadMeshOptions* mesh);

// Tilt the faces nearest each xyz in `face_points` by `angle_deg` about the
// neutral plane. `pull_dir` may be NULL (defaults to the neutral normal).
PrintcadOcctSweepResult printcad_occt_draft(
    const uint8_t* base_brep,
    size_t base_brep_len,
    double angle_deg,
    const double neutral_point[3],
    const double neutral_normal[3],
    const double* pull_dir,
    const double* face_points,
    size_t face_point_count,
    const PcadMeshOptions* mesh);

// Hollow the base solid to a wall of `value` mm, removing the faces nearest
// each xyz in `face_points`. `inward != 0` keeps the outer surface in place.
PrintcadOcctSweepResult printcad_occt_thickness(
    const uint8_t* base_brep,
    size_t base_brep_len,
    double value,
    int inward,
    const double* face_points,
    size_t face_point_count,
    const PcadMeshOptions* mesh);

// One tool solid re-applied by a pattern.
typedef struct PcadToolSolid {
    const uint8_t* brep;
    size_t len;
    int32_t subtractive;
} PcadToolSolid;

// Re-apply the tool solids under each transform (row-major 4x4, `transforms`
// holds 16 * transform_count doubles): additive tools fuse into the base,
// subtractive tools cut. Non-rigid transforms (e.g. scaling) are supported.
PrintcadOcctSweepResult printcad_occt_pattern(
    const uint8_t* base_brep,
    size_t base_brep_len,
    const PcadToolSolid* tools,
    size_t tool_count,
    const double* transforms,
    size_t transform_count,
    const PcadMeshOptions* mesh);

// Boolean between the base and an external tool solid.
// `kind`: 0 = fuse, 1 = cut, 2 = common.
PrintcadOcctSweepResult printcad_occt_boolean(
    const uint8_t* base_brep,
    size_t base_brep_len,
    const uint8_t* tool_brep,
    size_t tool_len,
    int32_t kind,
    const PcadMeshOptions* mesh);

// Free helpers — every output buffer must be released exactly once.
void printcad_occt_free_string(char* str);
void printcad_occt_free_sweep_result(PrintcadOcctSweepResult result);
void printcad_occt_free_result(PrintcadOcctImportResult result);
void printcad_occt_free_brep_import_result(PrintcadOcctBrepImportResult result);

#ifdef __cplusplus
}
#endif

#endif // PRINTCAD_OCCT_STEP_LOADER_H
