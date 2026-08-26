//! Shape → `TriMesh` conversion: deflection mapping, per-face triangulation
//! with color keying, normal-aware cross-face welding and boundary edges.
//!
//! The weld and boundary-edge algorithms are direct ports of the previous
//! kernel-independent C++ implementations, so viewport shading and outline
//! behaviour are unchanged across the kernel swap.

use std::collections::HashMap;

use kernel_api::{KernelError, KernelResult, LinearDeflectionMode, TessellationSettings, TriMesh};
use ogeom::algo::{shape_bounds, vertex_bounds};
use ogeom::core::parallel::map_ordered;
use ogeom::core::Tolerances;
use ogeom::math::Point;
use ogeom::mesh::{triangulate_face, Deflection};
use ogeom::topo::Triangulation;
use ogeom::topo::{explore, Filter, Model, Shape, ShapeType};
use tracing::warn;

pub const WHITE: [f32; 3] = [1.0, 1.0, 1.0];

/// Whether a mesh may spread its faces across threads.
///
/// `Inline` when the caller is already inside a parallel loop: nesting two
/// `map_ordered` passes would multiply the thread count instead of sharing it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Faces {
    Wide,
    Inline,
}

/// What triangulating one face produced. Kept as data rather than handled on
/// the spot so the parallel pass stays pure and every message is reported in
/// face order.
enum FaceWork {
    Meshed(Box<Triangulation>),
    /// This face alone could not be triangulated; the rest still mesh.
    Failed(String),
    /// The whole job was cancelled.
    Cancelled(String),
}

pub fn tolerances() -> Tolerances {
    Tolerances::millimetres()
}

/// Deserialize one native-format blob (single root) and mesh it.
pub fn tessellate_blob(
    brep_blob: &[u8],
    face_colors: &[[f32; 3]],
    detail: &TessellationSettings,
    faces: Faces,
) -> KernelResult<TriMesh> {
    let (model, root) = read_blob(brep_blob)?;
    mesh_shape_with(&model, &root, face_colors, detail, faces)
        .map_err(|e| KernelError::Other(anyhow::anyhow!("tessellation failed: {e}")))
}

/// Deserialize a native-format blob that carries exactly one root shape.
pub fn read_blob(brep_blob: &[u8]) -> KernelResult<(Model, Shape)> {
    let text = std::str::from_utf8(brep_blob)
        .map_err(|_| KernelError::InvalidInput("shape snapshot is not valid UTF-8".into()))?;
    let (model, roots) = ogeom::io::native::read(text)
        .map_err(|e| KernelError::InvalidInput(format!("shape snapshot failed to parse: {e}")))?;
    let root = roots
        .into_iter()
        .next()
        .ok_or_else(|| KernelError::InvalidInput("shape snapshot holds no shape".into()))?;
    Ok((model, root))
}

/// Serialize one shape into native-format blob bytes.
pub fn write_blob(model: &Model, root: &Shape) -> KernelResult<Vec<u8>> {
    let options = ogeom::io::native::WriteOptions {
        triangulations: false,
    };
    let text = ogeom::io::native::write(model, std::slice::from_ref(root), options)
        .map_err(|e| KernelError::Other(anyhow::anyhow!("shape serialization failed: {e}")))?;
    Ok(text.into_bytes())
}

/// A finite bounding box for a shape.
///
/// `shape_bounds` bounds each face's *carrier* surface, and a STEP-imported
/// face can sit on an unbounded plane — the result balloons to ±1e9. When the
/// carrier bound is absurd (or empty), fall back to the vertex bound, which
/// is finite and only underestimates curved bulges.
pub fn robust_bounds(model: &Model, shape: &Shape) -> Option<(Point, Point)> {
    const SANE: f64 = 1.0e7;
    let carrier = shape_bounds(model, shape, tolerances())
        .ok()
        .and_then(|b| Some((b.low()?, b.high()?)));
    if let Some((lo, hi)) = carrier {
        let sane = [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z]
            .iter()
            .all(|v| v.is_finite() && v.abs() < SANE);
        if sane {
            return Some((lo, hi));
        }
    }
    vertex_bounds(model, shape, tolerances())
        .ok()
        .and_then(|b| Some((b.low()?, b.high()?)))
}

/// The absolute chord deflection for a shape under the current settings.
///
/// Bbox-scaled mode replicates the previous kernel's formula —
/// `(dx + dy + dz) / 300 × mesh_deviation` — so triangle density is visually
/// unchanged across the swap.
pub fn chord_for(model: &Model, shape: &Shape, detail: &TessellationSettings) -> f64 {
    match detail.linear_deflection_mode {
        LinearDeflectionMode::AbsoluteMm => f64::from(detail.chord_tolerance.max(0.001)),
        LinearDeflectionMode::BboxScaled => {
            let mult = f64::from(detail.mesh_deviation.max(0.001));
            let extent_sum = robust_bounds(model, shape)
                .map(|(lo, hi)| (hi.x - lo.x) + (hi.y - lo.y) + (hi.z - lo.z))
                .unwrap_or(0.0);
            if extent_sum <= 0.0 {
                0.1
            } else {
                (extent_sum / 300.0) * mult
            }
        }
    }
}

fn deflection_for(model: &Model, shape: &Shape, detail: &TessellationSettings) -> Deflection {
    let chord = chord_for(model, shape, detail).max(1e-6);
    let angular = f64::from(detail.angular_tolerance_deg.max(0.5)).to_radians();
    Deflection {
        chord,
        angular,
        ..Deflection::default()
    }
}

/// Mesh a shape into a `TriMesh`, coloring vertices per face in exploration
/// order from `face_colors` (white when the table is empty or short).
pub fn mesh_shape(
    model: &Model,
    root: &Shape,
    face_colors: &[[f32; 3]],
    detail: &TessellationSettings,
) -> KernelResult<TriMesh> {
    mesh_shape_with(model, root, face_colors, detail, Faces::Wide)
}

/// As [`mesh_shape`], saying whether the face pass may go wide.
pub fn mesh_shape_with(
    model: &Model,
    root: &Shape,
    face_colors: &[[f32; 3]],
    detail: &TessellationSettings,
    faces_threading: Faces,
) -> KernelResult<TriMesh> {
    let tol = tolerances();
    let deflection = deflection_for(model, root, detail);
    let faces = explore(model, root, Filter::OfType(ShapeType::Face))
        .map_err(|e| KernelError::Other(anyhow::anyhow!("face exploration failed: {e}")))?;

    crate::progress::context(format_args!("Meshing {} faces", faces.len()));

    // Triangulating a face only reads the model, so the faces go wide; the
    // buffers are then filled in face order, which is what keeps the output
    // identical at any thread count.
    let one_face = |face: &Shape| -> FaceWork {
        if let Err(e) = crate::progress::checkpoint() {
            return FaceWork::Cancelled(e);
        }
        match triangulate_face(model, face, deflection, tol) {
            Ok(tri) => FaceWork::Meshed(Box::new(tri)),
            Err(e) => FaceWork::Failed(e.to_string()),
        }
    };
    let computed: Vec<FaceWork> = match faces_threading {
        Faces::Wide => map_ordered(&faces, |_, face| one_face(face)),
        Faces::Inline => faces.iter().map(one_face).collect(),
    };

    let mut mesh = TriMesh::default();
    // Per successfully meshed face: average normal and the id each vertex
    // belongs to, for the normal-aware weld.
    let mut face_normals: Vec<[f32; 3]> = Vec::new();
    let mut vertex_face: Vec<u32> = Vec::new();
    // A face the kernel cannot triangulate is dropped so the rest of the body
    // still draws — but silently dropping it would leave a hole nobody
    // accounts for, so the count is reported below.
    let mut skipped = 0usize;

    for (i, work) in computed.into_iter().enumerate() {
        let tri = match work {
            FaceWork::Meshed(tri) => *tri,
            FaceWork::Failed(e) => {
                if skipped == 0 {
                    warn!(target: "printcad.kernel", face = i, "face failed to triangulate: {e}");
                }
                skipped += 1;
                continue;
            }
            // A cancelled mesh is not a partial mesh: say so and hand back
            // nothing rather than half a body.
            FaceWork::Cancelled(e) => {
                return Err(KernelError::Other(anyhow::anyhow!("meshing stopped: {e}")))
            }
        };
        if tri.triangles.is_empty() || tri.positions.is_empty() {
            continue;
        }
        let color = face_colors.get(i).copied().unwrap_or(WHITE);
        let base = mesh.positions.len() as u32;
        let face_id = face_normals.len() as u32;

        for p in &tri.positions {
            mesh.positions.push([p.x as f32, p.y as f32, p.z as f32]);
            mesh.colors.push(color);
            vertex_face.push(face_id);
        }
        for n in &tri.normals {
            mesh.normals.push([n.x as f32, n.y as f32, n.z as f32]);
        }

        // Area-weighted average triangle normal stands in for "the face's
        // normal" when the weld decides whether two coincident vertices belong
        // to the same smooth group.
        let mut acc = [0.0f64; 3];
        for t in &tri.triangles {
            let [a, b, c] = t.map(|i| tri.positions[i as usize]);
            let n = (b - a).cross(c - a);
            acc[0] += n.x;
            acc[1] += n.y;
            acc[2] += n.z;
        }
        let len = (acc[0] * acc[0] + acc[1] * acc[1] + acc[2] * acc[2]).sqrt();
        face_normals.push(if len > 1e-12 {
            [
                (acc[0] / len) as f32,
                (acc[1] / len) as f32,
                (acc[2] / len) as f32,
            ]
        } else {
            [0.0, 0.0, 1.0]
        });

        for t in &tri.triangles {
            mesh.indices.push(base + t[0]);
            mesh.indices.push(base + t[1]);
            mesh.indices.push(base + t[2]);
        }
    }

    if skipped > 0 {
        warn!(
            target: "printcad.kernel",
            skipped,
            faces = faces.len(),
            "{skipped} of {} faces could not be triangulated; this body draws with gaps",
            faces.len()
        );
    }

    if !face_colors.is_empty() && face_colors.len() != faces.len() {
        warn!(
            target: "printcad.kernel",
            expected = faces.len(),
            got = face_colors.len(),
            "face color table does not match face count; extra faces rendered white"
        );
    }

    let mut edges = if detail.generate_boundary_edges {
        extract_boundary_edges(&mesh.indices)
    } else {
        Vec::new()
    };

    if detail.weld_cross_face && !mesh.indices.is_empty() {
        let threshold = f64::from(detail.weld_angle_threshold_deg.max(0.0))
            .to_radians()
            .cos() as f32;
        let remap = weld_vertices(&mut mesh, &vertex_face, &face_normals, threshold);
        for v in &mut edges {
            *v = remap[*v as usize];
        }
        let mut filtered = Vec::with_capacity(edges.len());
        for pair in edges.chunks_exact(2) {
            if pair[0] != pair[1] {
                filtered.extend_from_slice(pair);
            }
        }
        edges = filtered;
    }

    mesh.edges = edges;
    Ok(mesh)
}

/// Edges used by exactly one triangle, as flat index pairs (the mesh outline).
pub fn extract_boundary_edges(indices: &[u32]) -> Vec<u32> {
    if indices.len() < 3 {
        return Vec::new();
    }
    struct Record {
        count: u32,
    }
    let mut edges: HashMap<(u32, u32), Record> = HashMap::with_capacity(indices.len());
    let bump = |a: u32, b: u32, edges: &mut HashMap<(u32, u32), Record>| {
        if a == b {
            return;
        }
        let key = if a < b { (a, b) } else { (b, a) };
        edges
            .entry(key)
            .and_modify(|r| r.count += 1)
            .or_insert(Record { count: 1 });
    };
    for tri in indices.chunks_exact(3) {
        bump(tri[0], tri[1], &mut edges);
        bump(tri[1], tri[2], &mut edges);
        bump(tri[2], tri[0], &mut edges);
    }
    // Emit in triangle order rather than map order: a boundary edge belongs
    // to exactly one triangle, so a second pass yields each once, and the
    // result does not depend on how the map happened to hash.
    let mut out = Vec::with_capacity(edges.len() * 2);
    let emit = |a: u32, b: u32, out: &mut Vec<u32>| {
        if a == b {
            return;
        }
        let key = if a < b { (a, b) } else { (b, a) };
        if edges.get(&key).is_some_and(|record| record.count == 1) {
            out.push(a);
            out.push(b);
        }
    };
    for tri in indices.chunks_exact(3) {
        emit(tri[0], tri[1], &mut out);
        emit(tri[1], tri[2], &mut out);
        emit(tri[2], tri[0], &mut out);
    }
    out
}

fn pack_rgb_key(rgb: &[f32; 3]) -> u32 {
    let ch = |v: f32| -> u32 { ((f64::from(v).clamp(0.0, 1.0) * 255.0).round() as u32) & 0xff };
    (ch(rgb[0]) << 16) | (ch(rgb[1]) << 8) | ch(rgb[2])
}

/// Merge coincident vertices whose colors match exactly and whose owning
/// faces' normals agree within the angle threshold; vertex normals average
/// across the merged set so smooth surfaces shade smoothly while hard CAD
/// edges stay crisp. Returns the old→new index remap.
pub fn weld_vertices(
    mesh: &mut TriMesh,
    vertex_face: &[u32],
    face_normals: &[[f32; 3]],
    angle_cos_threshold: f32,
) -> Vec<u32> {
    let old_vertex_count = mesh.positions.len();
    if mesh.colors.len() != old_vertex_count {
        mesh.colors.resize(old_vertex_count, WHITE);
    }

    const QUANTIZE: f64 = 1.0e5;
    let key_of = |p: &[f32; 3]| -> (i64, i64, i64) {
        (
            (f64::from(p[0]) * QUANTIZE).round() as i64,
            (f64::from(p[1]) * QUANTIZE).round() as i64,
            (f64::from(p[2]) * QUANTIZE).round() as i64,
        )
    };

    struct BucketEntry {
        canonical_index: u32,
        face_normal: [f32; 3],
        color_packed: u32,
    }

    let mut buckets: HashMap<(i64, i64, i64), Vec<BucketEntry>> =
        HashMap::with_capacity(old_vertex_count);
    let mut new_positions: Vec<[f32; 3]> = Vec::with_capacity(old_vertex_count);
    let mut new_colors: Vec<[f32; 3]> = Vec::with_capacity(old_vertex_count);
    let mut normal_accum: Vec<[f64; 3]> = Vec::with_capacity(old_vertex_count);
    let mut normal_weights: Vec<u32> = Vec::with_capacity(old_vertex_count);
    let mut remap = vec![0u32; old_vertex_count];

    for i in 0..old_vertex_count {
        let pos = mesh.positions[i];
        let nrm = mesh.normals[i];
        let col = mesh.colors[i];
        let face_n = face_normals[vertex_face[i] as usize];
        let pcol = pack_rgb_key(&col);

        let bucket = buckets.entry(key_of(&pos)).or_default();
        let mut canonical = u32::MAX;
        for entry in bucket.iter_mut() {
            if entry.color_packed != pcol {
                continue;
            }
            let dot = entry.face_normal[0] * face_n[0]
                + entry.face_normal[1] * face_n[1]
                + entry.face_normal[2] * face_n[2];
            if dot >= angle_cos_threshold {
                canonical = entry.canonical_index;
                // Fold this face's normal into the bucket entry so chains of
                // slightly-turning faces (cylinder facets) keep welding.
                let w = normal_weights[canonical as usize] as f32;
                let inv = 1.0 / (w + 1.0);
                let mut folded = [
                    (entry.face_normal[0] * w + face_n[0]) * inv,
                    (entry.face_normal[1] * w + face_n[1]) * inv,
                    (entry.face_normal[2] * w + face_n[2]) * inv,
                ];
                let len =
                    (folded[0] * folded[0] + folded[1] * folded[1] + folded[2] * folded[2]).sqrt();
                if len > 1e-8 {
                    folded = [folded[0] / len, folded[1] / len, folded[2] / len];
                }
                entry.face_normal = folded;
                break;
            }
        }

        if canonical == u32::MAX {
            canonical = new_positions.len() as u32;
            new_positions.push(pos);
            new_colors.push(col);
            normal_accum.push([f64::from(nrm[0]), f64::from(nrm[1]), f64::from(nrm[2])]);
            normal_weights.push(1);
            bucket.push(BucketEntry {
                canonical_index: canonical,
                face_normal: face_n,
                color_packed: pcol,
            });
        } else {
            let acc = &mut normal_accum[canonical as usize];
            acc[0] += f64::from(nrm[0]);
            acc[1] += f64::from(nrm[1]);
            acc[2] += f64::from(nrm[2]);
            normal_weights[canonical as usize] += 1;
        }

        remap[i] = canonical;
    }

    let mut new_normals: Vec<[f32; 3]> = Vec::with_capacity(new_positions.len());
    for (acc, w) in normal_accum.iter().zip(&normal_weights) {
        let inv = if *w > 0 { 1.0 / f64::from(*w) } else { 0.0 };
        let (nx, ny, nz) = (acc[0] * inv, acc[1] * inv, acc[2] * inv);
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        new_normals.push(if len > 1e-12 {
            [(nx / len) as f32, (ny / len) as f32, (nz / len) as f32]
        } else {
            [0.0, 0.0, 1.0]
        });
    }

    for idx in &mut mesh.indices {
        *idx = remap[*idx as usize];
    }
    mesh.positions = new_positions;
    mesh.normals = new_normals;
    mesh.colors = new_colors;
    remap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_edges_of_two_triangles_sharing_one_edge() {
        // Quad split into two triangles: the diagonal (1,2) is interior.
        let indices = [0u32, 1, 2, 1, 3, 2];
        let mut edges = extract_boundary_edges(&indices);
        assert_eq!(edges.len(), 8);
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        while let (Some(b), Some(a)) = (edges.pop(), edges.pop()) {
            pairs.push(if a < b { (a, b) } else { (b, a) });
        }
        pairs.sort_unstable();
        assert_eq!(pairs, vec![(0, 1), (0, 2), (1, 3), (2, 3)]);
    }

    #[test]
    fn weld_merges_same_group_and_splits_hard_edges() {
        // Two faces meeting at a shared edge position; face normals differ by
        // 90°, so under a 30° threshold the seam vertices must NOT merge, while
        // duplicates within one face must.
        let mut mesh = TriMesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 0.0, 0.0], // duplicate of #1, same face group
                [1.0, 0.0, 0.0], // duplicate on the other face
            ],
            normals: vec![
                [0.0, 0.0, 1.0],
                [0.0, 0.0, 1.0],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 0.0],
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            edges: Vec::new(),
            colors: vec![WHITE; 4],
        };
        let vertex_face = [0u32, 0, 0, 1];
        let face_normals = [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0]];
        let threshold = 30f32.to_radians().cos();
        let remap = weld_vertices(&mut mesh, &vertex_face, &face_normals, threshold);
        assert_eq!(remap[1], remap[2], "same-face duplicates weld");
        assert_ne!(remap[1], remap[3], "hard edge stays split");
        assert_eq!(mesh.positions.len(), 3);
    }
}
