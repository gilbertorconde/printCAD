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
// welding so hard CAD edges stay visible. `name` may be null when no STEP
// product label was attached.
typedef struct PrintcadOcctBody {
    char* name;
    float* positions;
    float* normals;
    uint32_t* indices;
    uint32_t* edges;
    size_t vertex_count;
    size_t index_count;
    size_t edge_count;
} PrintcadOcctBody;

typedef struct PrintcadOcctImportResult {
    PrintcadOcctBody* bodies;
    size_t body_count;
    char* error;
} PrintcadOcctImportResult;

// Read a STEP/STP file and tessellate every solid/shell face.
// `linear_deflection` and `angular_deflection_rad` control mesh density.
// When `weld_cross_face` is non-zero the shim merges coincident vertices
// across face boundaries whose face normals are within
// `weld_angle_threshold_rad` of each other (so hard CAD edges remain
// distinct). Pass 0 to skip welding entirely.
// On success, `error` is null and `bodies`/`body_count` are populated.
// On failure, `error` is a non-null malloc'd string and `bodies` is null.
PrintcadOcctImportResult printcad_occt_import_step(
    const char* utf8_path,
    double linear_deflection,
    double angular_deflection_rad,
    int weld_cross_face,
    double weld_angle_threshold_rad);

// Free helpers — every output buffer must be released exactly once.
void printcad_occt_free_string(char* str);
void printcad_occt_free_result(PrintcadOcctImportResult result);

#ifdef __cplusplus
}
#endif

#endif // PRINTCAD_OCCT_STEP_LOADER_H
