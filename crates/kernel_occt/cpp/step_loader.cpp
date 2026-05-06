// printCAD OCCT shim: STEP import + per-face triangulation pipeline.
//
// Implementation notes:
//   - Each top-level shape produced by `STEPControl_Reader::TransferRoots` is
//     emitted as one `PrintcadOcctBody`.
//   - Vertices are first emitted per-face (no welding across face
//     boundaries) so face-boundary edges and per-face normals can be
//     computed unambiguously.
//   - Per-vertex normals are computed by averaging adjacent triangle normals
//     within the face (smooth shading within a face).
//   - Triangle winding is flipped when the face orientation is REVERSED so
//     the outward normals match face orientation.
//   - Boundary edges are extracted *before* welding using the standard
//     "edge appears in only one triangle of a per-face buffer" rule and then
//     remapped through the welding table so they keep pointing at the
//     correct welded vertices.
//   - When cross-face welding is enabled the shim merges vertices that share
//     a quantized position *and* whose face normals are within a
//     configurable angle threshold. This typically collapses the 4-6×
//     duplication produced by per-face emission while leaving genuine sharp
//     edges intact (since their face normals differ by more than the
//     threshold).

#include "step_loader.h"

#include <BRep_Tool.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <Bnd_Box.hxx>
#include <BRepBndLib.hxx>
#include <Poly_Triangulation.hxx>
#include <Poly_Triangle.hxx>
#include <STEPControl_Reader.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_Orientation.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp_Explorer.hxx>
#include <TopLoc_Location.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Iterator.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <string>
#include <unordered_map>
#include <utility>
#include <vector>

namespace {

// Owned mesh buffer used while accumulating face triangulations. Vertices
// here are in pre-weld, per-face form; welding happens once per body in
// `weld_buffer`.
struct MeshBuffer {
    std::vector<float> positions;
    std::vector<float> normals;
    std::vector<uint32_t> indices;
    // Range [face_starts[i], face_starts[i+1]) of `indices` belongs to face
    // `i`. We need this for both boundary-edge extraction and welding (which
    // needs to know the face normal driving each vertex).
    std::vector<size_t> face_starts;
    // One entry per face: averaged outward face normal. Used by the welding
    // pass to decide whether two vertices that share a position should be
    // merged.
    std::vector<std::array<float, 3>> face_normals;
};

char* duplicate_to_malloc(const std::string& input) {
    char* out = static_cast<char*>(std::malloc(input.size() + 1));
    if (out == nullptr) {
        return nullptr;
    }
    std::memcpy(out, input.data(), input.size());
    out[input.size()] = '\0';
    return out;
}

PrintcadOcctImportResult make_error(const std::string& message) {
    PrintcadOcctImportResult result{};
    result.bodies = nullptr;
    result.body_count = 0;
    result.error = duplicate_to_malloc(message);
    return result;
}

// Tessellate a single face into the mesh buffer. Returns true if any
// triangles were appended. The caller is responsible for closing the face by
// recording `face_starts` *after* calling this.
bool append_face(const TopoDS_Face& face, MeshBuffer& out) {
    TopLoc_Location location;
    Handle(Poly_Triangulation) triangulation = BRep_Tool::Triangulation(face, location);
    if (triangulation.IsNull()) {
        return false;
    }

    const int node_count = triangulation->NbNodes();
    const int triangle_count = triangulation->NbTriangles();
    if (node_count <= 0 || triangle_count <= 0) {
        return false;
    }

    const gp_Trsf& trsf = location.Transformation();
    const bool reversed = face.Orientation() == TopAbs_REVERSED;

    const uint32_t base_index = static_cast<uint32_t>(out.positions.size() / 3);

    out.positions.reserve(out.positions.size() + static_cast<size_t>(node_count) * 3);
    for (int i = 1; i <= node_count; ++i) {
        gp_Pnt p = triangulation->Node(i);
        p.Transform(trsf);
        out.positions.push_back(static_cast<float>(p.X()));
        out.positions.push_back(static_cast<float>(p.Y()));
        out.positions.push_back(static_cast<float>(p.Z()));
    }

    out.normals.resize(out.positions.size(), 0.0f);

    // Track aggregate face normal for the welding pass.
    double face_nx = 0.0;
    double face_ny = 0.0;
    double face_nz = 0.0;

    out.indices.reserve(out.indices.size() + static_cast<size_t>(triangle_count) * 3);
    for (int t = 1; t <= triangle_count; ++t) {
        const Poly_Triangle& tri = triangulation->Triangle(t);
        int n1, n2, n3;
        tri.Get(n1, n2, n3);
        if (reversed) {
            std::swap(n2, n3);
        }
        const uint32_t a = base_index + static_cast<uint32_t>(n1 - 1);
        const uint32_t b = base_index + static_cast<uint32_t>(n2 - 1);
        const uint32_t c = base_index + static_cast<uint32_t>(n3 - 1);
        out.indices.push_back(a);
        out.indices.push_back(b);
        out.indices.push_back(c);

        const float ax = out.positions[a * 3 + 0];
        const float ay = out.positions[a * 3 + 1];
        const float az = out.positions[a * 3 + 2];
        const float bx = out.positions[b * 3 + 0];
        const float by = out.positions[b * 3 + 1];
        const float bz = out.positions[b * 3 + 2];
        const float cx = out.positions[c * 3 + 0];
        const float cy = out.positions[c * 3 + 1];
        const float cz = out.positions[c * 3 + 2];

        const float ux = bx - ax, uy = by - ay, uz = bz - az;
        const float vx = cx - ax, vy = cy - ay, vz = cz - az;
        const float nx = uy * vz - uz * vy;
        const float ny = uz * vx - ux * vz;
        const float nz = ux * vy - uy * vx;

        out.normals[a * 3 + 0] += nx;
        out.normals[a * 3 + 1] += ny;
        out.normals[a * 3 + 2] += nz;
        out.normals[b * 3 + 0] += nx;
        out.normals[b * 3 + 1] += ny;
        out.normals[b * 3 + 2] += nz;
        out.normals[c * 3 + 0] += nx;
        out.normals[c * 3 + 1] += ny;
        out.normals[c * 3 + 2] += nz;

        face_nx += nx;
        face_ny += ny;
        face_nz += nz;
    }

    // Normalise the per-vertex normals we just touched.
    for (size_t i = base_index * 3; i < out.normals.size(); i += 3) {
        const float nx = out.normals[i];
        const float ny = out.normals[i + 1];
        const float nz = out.normals[i + 2];
        const float len = std::sqrt(nx * nx + ny * ny + nz * nz);
        if (len > 1e-8f) {
            out.normals[i] = nx / len;
            out.normals[i + 1] = ny / len;
            out.normals[i + 2] = nz / len;
        } else {
            out.normals[i] = 0.0f;
            out.normals[i + 1] = 0.0f;
            out.normals[i + 2] = 1.0f;
        }
    }

    const double face_len = std::sqrt(face_nx * face_nx + face_ny * face_ny + face_nz * face_nz);
    std::array<float, 3> face_normal{0.0f, 0.0f, 1.0f};
    if (face_len > 1e-12) {
        face_normal[0] = static_cast<float>(face_nx / face_len);
        face_normal[1] = static_cast<float>(face_ny / face_len);
        face_normal[2] = static_cast<float>(face_nz / face_len);
    }
    out.face_normals.push_back(face_normal);

    return true;
}

// Boundary edges: any triangle edge that appears in only one triangle of
// the *per-face* buffer. With the C++ shim emitting one vertex range per
// face this exactly produces the face boundaries (internal-edge twins live
// in the same face range, boundary edges only see one triangle).
//
// Returned in pre-weld vertex space; the welding pass remaps them
// afterwards.
std::vector<uint32_t> extract_boundary_edges(const std::vector<uint32_t>& indices) {
    if (indices.size() < 3) {
        return {};
    }

    struct EdgeRecord {
        uint32_t a;
        uint32_t b;
        uint32_t count;
    };

    // Hash on (min,max) of the two endpoints so opposite-winding pairs
    // collide and cancel.
    struct PairHash {
        size_t operator()(const std::pair<uint32_t, uint32_t>& p) const noexcept {
            return std::hash<uint64_t>{}((static_cast<uint64_t>(p.first) << 32) | p.second);
        }
    };

    std::unordered_map<std::pair<uint32_t, uint32_t>, EdgeRecord, PairHash> edges;
    edges.reserve(indices.size());

    auto bump = [&](uint32_t a, uint32_t b) {
        if (a == b) {
            return;
        }
        std::pair<uint32_t, uint32_t> key = a < b ? std::make_pair(a, b) : std::make_pair(b, a);
        auto it = edges.find(key);
        if (it == edges.end()) {
            edges.emplace(key, EdgeRecord{a, b, 1});
        } else {
            it->second.count += 1;
        }
    };

    for (size_t i = 0; i + 2 < indices.size(); i += 3) {
        const uint32_t a = indices[i];
        const uint32_t b = indices[i + 1];
        const uint32_t c = indices[i + 2];
        bump(a, b);
        bump(b, c);
        bump(c, a);
    }

    std::vector<uint32_t> out;
    out.reserve(edges.size() * 2);
    for (const auto& kv : edges) {
        if (kv.second.count == 1) {
            out.push_back(kv.second.a);
            out.push_back(kv.second.b);
        }
    }
    return out;
}

// Returns the face index that owns a given pre-weld vertex. `face_starts`
// stores the running prefix of indices, so a vertex's owning face is the
// face whose vertex range covers it. Since `append_face` emits all of a
// face's vertices contiguously *before* its triangles get pushed, we can
// recover the owning face from a parallel `vertex_face` table built once
// per body.
//
// We build that table directly during welding rather than searching here.

// Cross-face vertex welding.
//
// Inputs:
//   - `mesh.positions` / `mesh.normals` / `mesh.indices` in pre-weld form.
//   - `vertex_face[i]` = face id for old vertex `i`.
//   - `mesh.face_normals[f]` = average outward normal for face `f`.
//   - `angle_cos_threshold` = cos(angle threshold). Two face normals merge
//     iff their dot is ≥ this value.
//
// Outputs:
//   - `mesh.positions` / `mesh.normals` are rewritten in-place to the
//     welded vertex set. Welded normals are the (renormalised) average of
//     the contributing per-face normals.
//   - `mesh.indices` is rewritten in-place to point at welded indices.
//   - Returns the remap table (old → new) so boundary edges can be
//     translated through it too.
std::vector<uint32_t> weld_vertices(
    MeshBuffer& mesh,
    const std::vector<uint32_t>& vertex_face,
    float angle_cos_threshold) {
    const size_t old_vertex_count = mesh.positions.size() / 3;

    // Quantize positions into integer cells so coincident points hash the
    // same. ~1e-5 mm grid is well below typical CAD precision but still
    // numerically safe inside an int64 key.
    constexpr double kQuantize = 1.0e5;

    struct PositionKey {
        int64_t x, y, z;
        bool operator==(const PositionKey& other) const noexcept {
            return x == other.x && y == other.y && z == other.z;
        }
    };
    struct PositionKeyHash {
        size_t operator()(const PositionKey& k) const noexcept {
            const uint64_t hx = static_cast<uint64_t>(k.x);
            const uint64_t hy = static_cast<uint64_t>(k.y);
            const uint64_t hz = static_cast<uint64_t>(k.z);
            uint64_t h = hx;
            h = h * 0x9E3779B97F4A7C15ULL ^ hy;
            h = h * 0x9E3779B97F4A7C15ULL ^ hz;
            return static_cast<size_t>(h);
        }
    };

    auto key_of = [](const float* p) {
        return PositionKey{
            static_cast<int64_t>(std::llround(static_cast<double>(p[0]) * kQuantize)),
            static_cast<int64_t>(std::llround(static_cast<double>(p[1]) * kQuantize)),
            static_cast<int64_t>(std::llround(static_cast<double>(p[2]) * kQuantize)),
        };
    };

    // Per-bucket entry: a candidate canonical vertex with its driving face
    // normal. Multiple entries may share a position when the angle threshold
    // forced a split.
    struct BucketEntry {
        uint32_t canonical_index;
        std::array<float, 3> face_normal;
    };

    std::unordered_map<PositionKey, std::vector<BucketEntry>, PositionKeyHash> buckets;
    buckets.reserve(old_vertex_count);

    std::vector<float> new_positions;
    std::vector<double> normal_accum;     // double precision averaging
    std::vector<uint32_t> normal_weights; // contributors per canonical
    new_positions.reserve(mesh.positions.size());
    normal_accum.reserve(mesh.normals.size());
    normal_weights.reserve(old_vertex_count);

    std::vector<uint32_t> remap(old_vertex_count, 0);

    for (size_t i = 0; i < old_vertex_count; ++i) {
        const float* pos = &mesh.positions[i * 3];
        const float* nrm = &mesh.normals[i * 3];
        const uint32_t fid = vertex_face[i];
        const std::array<float, 3>& face_n = mesh.face_normals[fid];

        const PositionKey key = key_of(pos);
        auto& bucket = buckets[key];

        // Look for a compatible canonical (face normal within threshold).
        uint32_t canonical = UINT32_MAX;
        for (auto& entry : bucket) {
            const float dot = entry.face_normal[0] * face_n[0]
                            + entry.face_normal[1] * face_n[1]
                            + entry.face_normal[2] * face_n[2];
            if (dot >= angle_cos_threshold) {
                canonical = entry.canonical_index;
                // Pull the entry's stored face normal toward the new face
                // so the next neighbour test compares against the running
                // mean, which behaves well on smooth-curved surfaces.
                const float w = static_cast<float>(normal_weights[canonical]);
                const float inv = 1.0f / (w + 1.0f);
                entry.face_normal = {
                    (entry.face_normal[0] * w + face_n[0]) * inv,
                    (entry.face_normal[1] * w + face_n[1]) * inv,
                    (entry.face_normal[2] * w + face_n[2]) * inv,
                };
                const float len = std::sqrt(
                    entry.face_normal[0] * entry.face_normal[0]
                    + entry.face_normal[1] * entry.face_normal[1]
                    + entry.face_normal[2] * entry.face_normal[2]);
                if (len > 1e-8f) {
                    entry.face_normal[0] /= len;
                    entry.face_normal[1] /= len;
                    entry.face_normal[2] /= len;
                }
                break;
            }
        }

        if (canonical == UINT32_MAX) {
            canonical = static_cast<uint32_t>(new_positions.size() / 3);
            new_positions.push_back(pos[0]);
            new_positions.push_back(pos[1]);
            new_positions.push_back(pos[2]);
            normal_accum.push_back(static_cast<double>(nrm[0]));
            normal_accum.push_back(static_cast<double>(nrm[1]));
            normal_accum.push_back(static_cast<double>(nrm[2]));
            normal_weights.push_back(1);
            bucket.push_back(BucketEntry{canonical, face_n});
        } else {
            normal_accum[canonical * 3 + 0] += static_cast<double>(nrm[0]);
            normal_accum[canonical * 3 + 1] += static_cast<double>(nrm[1]);
            normal_accum[canonical * 3 + 2] += static_cast<double>(nrm[2]);
            normal_weights[canonical] += 1;
        }

        remap[i] = canonical;
    }

    // Finalise normals: average then renormalise.
    std::vector<float> new_normals;
    new_normals.resize(new_positions.size(), 0.0f);
    const size_t new_vertex_count = new_positions.size() / 3;
    for (size_t i = 0; i < new_vertex_count; ++i) {
        const double w = static_cast<double>(normal_weights[i]);
        const double inv = w > 0.0 ? 1.0 / w : 0.0;
        const double nx = normal_accum[i * 3 + 0] * inv;
        const double ny = normal_accum[i * 3 + 1] * inv;
        const double nz = normal_accum[i * 3 + 2] * inv;
        const double len = std::sqrt(nx * nx + ny * ny + nz * nz);
        if (len > 1e-12) {
            new_normals[i * 3 + 0] = static_cast<float>(nx / len);
            new_normals[i * 3 + 1] = static_cast<float>(ny / len);
            new_normals[i * 3 + 2] = static_cast<float>(nz / len);
        } else {
            new_normals[i * 3 + 0] = 0.0f;
            new_normals[i * 3 + 1] = 0.0f;
            new_normals[i * 3 + 2] = 1.0f;
        }
    }

    // Rewrite indices through the remap.
    for (auto& idx : mesh.indices) {
        idx = remap[idx];
    }

    mesh.positions = std::move(new_positions);
    mesh.normals = std::move(new_normals);
    return remap;
}

bool finalize_body(
    MeshBuffer& buffer,
    const std::string& name,
    const std::vector<uint32_t>& edges,
    std::vector<PrintcadOcctBody>& out) {
    if (buffer.indices.empty() || buffer.positions.empty()) {
        return false;
    }

    PrintcadOcctBody body{};
    body.name = name.empty() ? nullptr : duplicate_to_malloc(name);

    const size_t vertex_count = buffer.positions.size() / 3;
    body.vertex_count = vertex_count;
    body.index_count = buffer.indices.size();
    body.edge_count = edges.size() / 2;

    body.positions = static_cast<float*>(std::malloc(buffer.positions.size() * sizeof(float)));
    body.normals = static_cast<float*>(std::malloc(buffer.normals.size() * sizeof(float)));
    body.indices = static_cast<uint32_t*>(std::malloc(buffer.indices.size() * sizeof(uint32_t)));
    body.edges = edges.empty()
                     ? nullptr
                     : static_cast<uint32_t*>(std::malloc(edges.size() * sizeof(uint32_t)));

    if (body.positions == nullptr || body.normals == nullptr || body.indices == nullptr
        || (!edges.empty() && body.edges == nullptr)) {
        std::free(body.positions);
        std::free(body.normals);
        std::free(body.indices);
        std::free(body.edges);
        std::free(body.name);
        return false;
    }
    std::memcpy(body.positions, buffer.positions.data(), buffer.positions.size() * sizeof(float));
    std::memcpy(body.normals, buffer.normals.data(), buffer.normals.size() * sizeof(float));
    std::memcpy(body.indices, buffer.indices.data(), buffer.indices.size() * sizeof(uint32_t));
    if (!edges.empty()) {
        std::memcpy(body.edges, edges.data(), edges.size() * sizeof(uint32_t));
    }
    out.push_back(body);
    return true;
}

void process_top_level(
    const TopoDS_Shape& shape,
    std::vector<PrintcadOcctBody>& out,
    int index,
    bool weld_cross_face,
    float weld_angle_cos_threshold) {
    MeshBuffer buffer;
    buffer.face_starts.push_back(0);

    for (TopExp_Explorer it(shape, TopAbs_FACE); it.More(); it.Next()) {
        const TopoDS_Face& face = TopoDS::Face(it.Current());
        if (append_face(face, buffer)) {
            buffer.face_starts.push_back(buffer.indices.size());
        }
    }

    // Boundary edges (computed in pre-weld vertex space).
    std::vector<uint32_t> edges = extract_boundary_edges(buffer.indices);

    if (weld_cross_face && !buffer.indices.empty()) {
        // Build vertex_face: for each pre-weld vertex, which face owns it.
        const size_t vertex_count = buffer.positions.size() / 3;
        std::vector<uint32_t> vertex_face(vertex_count, 0);

        // Walk per-face index ranges and stamp each referenced vertex with
        // its face id. Vertex ranges between faces don't strictly need to be
        // contiguous after `append_face` returns, but the loop below handles
        // both cases since it uses indices to find the touched vertices.
        const size_t face_count = buffer.face_normals.size();
        for (size_t f = 0; f < face_count; ++f) {
            const size_t start = buffer.face_starts[f];
            const size_t end = buffer.face_starts[f + 1];
            for (size_t i = start; i < end; ++i) {
                vertex_face[buffer.indices[i]] = static_cast<uint32_t>(f);
            }
        }

        std::vector<uint32_t> remap =
            weld_vertices(buffer, vertex_face, weld_angle_cos_threshold);

        // Translate boundary edges through the remap.
        for (auto& v : edges) {
            v = remap[v];
        }

        // After welding some boundary edges may collapse to zero-length
        // (rare, but possible if welding merged the two endpoints). Filter
        // those out.
        std::vector<uint32_t> filtered;
        filtered.reserve(edges.size());
        for (size_t i = 0; i + 1 < edges.size(); i += 2) {
            if (edges[i] != edges[i + 1]) {
                filtered.push_back(edges[i]);
                filtered.push_back(edges[i + 1]);
            }
        }
        edges = std::move(filtered);
    }

    std::string name = "Body " + std::to_string(index + 1);
    finalize_body(buffer, name, edges, out);
}

} // namespace

extern "C" PrintcadOcctImportResult printcad_occt_import_step(
    const char* utf8_path,
    double linear_deflection,
    double angular_deflection_rad,
    int weld_cross_face,
    double weld_angle_threshold_rad) {
    if (utf8_path == nullptr) {
        return make_error("STEP path is null");
    }

    try {
        STEPControl_Reader reader;
        const IFSelect_ReturnStatus status = reader.ReadFile(utf8_path);
        if (status != IFSelect_RetDone) {
            return make_error(std::string("STEP read failed (status ") + std::to_string(static_cast<int>(status)) + ")");
        }

        const Standard_Integer transferred = reader.TransferRoots();
        if (transferred <= 0) {
            return make_error("STEP file contained no transferable roots");
        }

        const Standard_Integer shape_count = reader.NbShapes();
        if (shape_count <= 0) {
            return make_error("STEP file produced no shapes");
        }

        std::vector<TopoDS_Shape> top_level;
        top_level.reserve(static_cast<size_t>(shape_count));
        for (Standard_Integer i = 1; i <= shape_count; ++i) {
            top_level.push_back(reader.Shape(i));
        }

        for (auto& shape : top_level) {
            BRepMesh_IncrementalMesh mesher(
                shape,
                linear_deflection > 0.0 ? linear_deflection : 0.5,
                /*isRelative=*/Standard_False,
                angular_deflection_rad > 0.0 ? angular_deflection_rad : 0.5,
                /*isInParallel=*/Standard_True);
            mesher.Perform();
        }

        // Precompute the cosine of the welding threshold once per import.
        const float weld_angle_cos =
            static_cast<float>(std::cos(std::max(0.0, weld_angle_threshold_rad)));
        const bool do_weld = weld_cross_face != 0;

        std::vector<PrintcadOcctBody> bodies;
        bodies.reserve(top_level.size());
        for (size_t i = 0; i < top_level.size(); ++i) {
            process_top_level(
                top_level[i], bodies, static_cast<int>(i), do_weld, weld_angle_cos);
        }

        if (bodies.empty()) {
            return make_error("STEP file produced no triangulated geometry");
        }

        PrintcadOcctImportResult result{};
        result.body_count = bodies.size();
        result.bodies = static_cast<PrintcadOcctBody*>(
            std::malloc(bodies.size() * sizeof(PrintcadOcctBody)));
        if (result.bodies == nullptr) {
            for (auto& body : bodies) {
                std::free(body.positions);
                std::free(body.normals);
                std::free(body.indices);
                std::free(body.edges);
                std::free(body.name);
            }
            return make_error("Out of memory while building STEP import result");
        }
        std::memcpy(result.bodies, bodies.data(), bodies.size() * sizeof(PrintcadOcctBody));
        result.error = nullptr;
        return result;
    } catch (Standard_Failure const& ex) {
        return make_error(std::string("OCCT exception: ") + ex.GetMessageString());
    } catch (std::exception const& ex) {
        return make_error(std::string("std::exception: ") + ex.what());
    } catch (...) {
        return make_error("Unknown exception while importing STEP file");
    }
}

extern "C" void printcad_occt_free_string(char* str) {
    if (str != nullptr) {
        std::free(str);
    }
}

extern "C" void printcad_occt_free_result(PrintcadOcctImportResult result) {
    for (size_t i = 0; i < result.body_count; ++i) {
        PrintcadOcctBody& body = result.bodies[i];
        std::free(body.positions);
        std::free(body.normals);
        std::free(body.indices);
        std::free(body.edges);
        std::free(body.name);
    }
    std::free(result.bodies);
    std::free(result.error);
}
