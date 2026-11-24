//! Shape → `TriMesh` conversion: deflection mapping, per-face triangulation
//! with color keying, normal-aware cross-face welding and boundary edges.

use std::collections::HashMap;

use kernel_api::{
    FaceSurface, KernelError, KernelResult, LinearDeflectionMode, TessellationSettings, TriMesh,
};
use ogeom::algo::shape_bounds;
use ogeom::core::Tolerances;
use ogeom::core::parallel::map_ordered;
use ogeom::geom::SurfaceGeometry;
use ogeom::math::Point;
use ogeom::mesh::{Deflection, face_meshes_for, polyline_of_edge};
use ogeom::topo::NodeData;
use ogeom::topo::{Filter, Model, Shape, ShapeType, explore, explore_unique};
use tracing::warn;

pub const WHITE: [f32; 3] = [1.0, 1.0, 1.0];

pub fn tolerances() -> Tolerances {
    Tolerances::millimetres()
}

/// Deserialize one native-format blob (single root) and mesh it.
pub fn tessellate_blob(
    brep_blob: &[u8],
    face_colors: &[[f32; 3]],
    detail: &TessellationSettings,
) -> KernelResult<TriMesh> {
    let (model, root) = read_blob(brep_blob)?;
    mesh_shape(&model, &root, face_colors, detail)
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

/// A box surely holding a shape: a trimmed face adds only its surface's
/// bulge past its boundary, so a curved face's box may stand a little past
/// it (see [`solid_bounds`] for the smallest).
pub fn guaranteed_bounds(model: &Model, shape: &Shape) -> Option<(Point, Point)> {
    shape_bounds(model, shape, tolerances())
        .ok()
        .and_then(|b| Some((b.low()?, b.high()?)))
}

/// A built solid's bounds: the smallest box holding it, where the mesh's
/// stop at its chords. The mesh's when the kernel cannot tell.
pub fn solid_bounds(model: &Model, shape: &Shape, mesh: &TriMesh) -> Option<([f32; 3], [f32; 3])> {
    ogeom::algo::tight_bounds(model, shape, tolerances())
        .ok()
        .and_then(|b| Some((b.low()?, b.high()?)))
        .map(|(lo, hi)| {
            (
                [lo.x as f32, lo.y as f32, lo.z as f32],
                [hi.x as f32, hi.y as f32, hi.z as f32],
            )
        })
        .or_else(|| mesh.bounds())
}

/// The absolute chord deflection for a shape under the current settings.
///
/// Bbox-scaled mode is `(dx + dy + dz) / 300 × mesh_deviation`.
pub fn chord_for(model: &Model, shape: &Shape, detail: &TessellationSettings) -> f64 {
    match detail.linear_deflection_mode {
        LinearDeflectionMode::AbsoluteMm => f64::from(detail.chord_tolerance.max(0.001)),
        LinearDeflectionMode::BboxScaled => {
            let mult = f64::from(detail.mesh_deviation.max(0.001));
            let extent_sum = guaranteed_bounds(model, shape)
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

pub(crate) fn deflection_for(
    model: &Model,
    shape: &Shape,
    detail: &TessellationSettings,
) -> Deflection {
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
///
/// The faces go wide across threads, or stay on the calling thread when it
/// is already one of the kernel's workers (an import meshing its bodies
/// side by side).
pub fn mesh_shape(
    model: &Model,
    root: &Shape,
    face_colors: &[[f32; 3]],
    detail: &TessellationSettings,
) -> KernelResult<TriMesh> {
    mesh_faces(model, root, face_colors, detail, None, None)
}

/// As [`mesh_shape`], each face carrying its name from `names` and each
/// edge the names of the faces it runs between.
pub(crate) fn mesh_named(
    model: &Model,
    root: &Shape,
    detail: &TessellationSettings,
    names: &crate::naming::NameMap,
    reuse: Option<&mut crate::reuse::FaceMeshes>,
) -> KernelResult<TriMesh> {
    mesh_faces(model, root, &[], detail, Some(names), reuse)
}

fn mesh_faces(
    model: &Model,
    root: &Shape,
    face_colors: &[[f32; 3]],
    detail: &TessellationSettings,
    names: Option<&crate::naming::NameMap>,
    mut reuse: Option<&mut crate::reuse::FaceMeshes>,
) -> KernelResult<TriMesh> {
    let tol = tolerances();
    let deflection = deflection_for(model, root, detail);

    // A detail, not a context: this runs once per body, from every worker
    // thread during an import, and a context would reset the status display
    // hundreds of times a second.
    let face_count = explore(model, root, Filter::OfType(ShapeType::Face)).map_or(0, |f| f.len());
    crate::progress::detail(format_args!("Meshing {face_count} faces"));

    // Every face drawn to the chords the faces agree to draw their shared
    // edges to (a face narrower than a few chords draws its edges finer,
    // and the faces across must draw them the same or the weld finds
    // nothing to join), in face order, at any thread count. A face the
    // build before drew as it is now is taken from what that build kept.
    let kept = reuse.as_deref().map(crate::reuse::FaceMeshes::meshes);
    let (computed, chords) = face_meshes_for(model, root, deflection, kept, tol).map_err(|e| {
        if matches!(e, ogeom::core::OgeomError::Cancelled) {
            KernelError::Other(anyhow::anyhow!("meshing stopped: {e}"))
        } else {
            KernelError::Other(anyhow::anyhow!("meshing failed: {e}"))
        }
    })?;
    // A cancelled mesh is not a partial mesh: say so and hand back nothing
    // rather than half a body.
    if let Some(e) = computed.iter().find_map(|(_, m)| {
        m.as_ref()
            .err()
            .filter(|e| matches!(e, ogeom::core::OgeomError::Cancelled))
    }) {
        return Err(KernelError::Other(anyhow::anyhow!("meshing stopped: {e}")));
    }
    let faces: Vec<Shape> = computed.iter().map(|(face, _)| face.clone()).collect();

    let keys = reuse
        .as_ref()
        .map(|_| crate::reuse::FaceKeys::new(model, &faces, deflection));
    if let (Some(meshes), Some(keys)) = (reuse.as_deref_mut(), keys.as_ref()) {
        meshes.drew(keys);
        // The shape's box, the union of its faces': a face's box kept from
        // the build before where it is the same face, measured where not.
        let boxes: Vec<Option<(Point, Point)>> = map_ordered(&faces, |i, face| {
            keys.faces[i]
                .and_then(|k| meshes.face_box(k))
                .unwrap_or_else(|| {
                    ogeom::algo::tight_bounds(model, face, tol)
                        .ok()
                        .and_then(|b| Some((b.low()?, b.high()?)))
                })
        });
        let mut bounds: Option<(Point, Point)> = None;
        let mut whole = true;
        for ((_, mesh), drawn) in computed.iter().zip(&boxes) {
            match (mesh, drawn) {
                (Ok(_), Some((lo, hi))) => {
                    bounds = Some(match bounds {
                        None => (*lo, *hi),
                        Some((a, b)) => (
                            Point::new(a.x.min(lo.x), a.y.min(lo.y), a.z.min(lo.z)),
                            Point::new(b.x.max(hi.x), b.y.max(hi.y), b.z.max(hi.z)),
                        ),
                    });
                }
                _ => whole = false,
            }
        }
        meshes.set_bounds(bounds.filter(|_| whole));
        meshes.replace_boxes(
            keys.faces
                .iter()
                .zip(boxes)
                .filter_map(|(key, drawn)| Some(((*key)?, drawn)))
                .collect(),
        );
        tracing::debug!(
            target: "printcad.chain",
            "meshing keeps {} faces for {}",
            meshes.len(),
            faces.len()
        );
    }

    let mut mesh = TriMesh::default();
    // Per successfully meshed face: average normal and the id each vertex
    // belongs to, for the normal-aware weld.
    let mut face_normals: Vec<[f32; 3]> = Vec::new();
    let mut vertex_face: Vec<u32> = Vec::new();
    // A face the kernel cannot triangulate is dropped so the rest of the body
    // still draws, but silently dropping it would leave a hole nobody
    // accounts for, so the count is reported below.
    let mut skipped = 0usize;
    // What the mesher could not mesh as asked, by face, reported below.
    let mut folds: Vec<(usize, Point)> = Vec::new();
    let mut capped: Vec<(usize, Point)> = Vec::new();

    for (i, (face_shape, work)) in computed.iter().enumerate() {
        let tri = match work {
            Ok(tri) => tri,
            Err(e) => {
                if skipped == 0 {
                    warn!(target: "printcad.kernel", face = i, "face failed to triangulate: {e}");
                }
                skipped += 1;
                continue;
            }
        };
        for finding in &tri.findings {
            match finding {
                ogeom::topo::MeshFinding::SurfaceFolds { point, .. } => folds.push((i, *point)),
                ogeom::topo::MeshFinding::RepairCapped { point, .. } => capped.push((i, *point)),
                _ => {}
            }
        }
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
        let average = if len > 1e-12 {
            [
                (acc[0] / len) as f32,
                (acc[1] / len) as f32,
                (acc[2] / len) as f32,
            ]
        } else {
            [0.0, 0.0, 1.0]
        };
        face_normals.push(average);
        mesh.face_surfaces
            .push(face_surface(model, face_shape, average));
        if let Some(names) = names {
            mesh.face_names.push(names.primary(face_shape));
            mesh.face_other_names.push(names.others_of(face_shape));
            mesh.face_makers.push(names.maker_of(face_shape));
        }

        for (k, t) in tri.triangles.iter().enumerate() {
            let corners = tri.triangle_normals(k);
            for (j, v) in t.iter().enumerate() {
                // A cone's tip or a sphere's pole is one vertex with a
                // normal per corner: a corner whose normal is not its
                // vertex's gets a vertex of its own, so the fan shades
                // along each ruling.
                let own = corners.map(|c| c[j]).filter(|n| {
                    tri.normals
                        .get(*v as usize)
                        .is_some_and(|m| n.dot(*m) < 1.0 - 1e-9)
                });
                let index = match own {
                    Some(n) => {
                        let p = tri.positions[*v as usize];
                        mesh.positions.push([p.x as f32, p.y as f32, p.z as f32]);
                        mesh.normals.push([n.x as f32, n.y as f32, n.z as f32]);
                        mesh.colors.push(color);
                        vertex_face.push(face_id);
                        mesh.positions.len() as u32 - 1
                    }
                    None => base + v,
                };
                mesh.indices.push(index);
            }
            mesh.faces.push(face_id);
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

    if let Some((face, at)) = folds.first() {
        warn!(
            target: "printcad.kernel",
            faces = folds.len(),
            face,
            "{} face(s) fold over on themselves, the first at ({:.3}, {:.3}, {:.3}): no flat \
             triangle there faces the way the surface does on every side",
            folds.len(),
            at.x,
            at.y,
            at.z
        );
    }
    if let Some((face, at)) = capped.first() {
        warn!(
            target: "printcad.kernel",
            faces = capped.len(),
            face,
            "{} face(s) stopped refining before meeting the chord, the first near \
             ({:.3}, {:.3}, {:.3}): the mesh may stand off the face there by more than asked",
            capped.len(),
            at.x,
            at.y,
            at.z
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

    if detail.weld_cross_face && !mesh.indices.is_empty() {
        let threshold = f64::from(detail.weld_angle_threshold_deg.max(0.0))
            .to_radians()
            .cos() as f32;
        weld_vertices(&mut mesh, &vertex_face, &face_normals, threshold);
    }

    // The outline is the kernel's own edges, each drawn to the chord the
    // faces agreed on, and each segment names its edge so a click on one
    // segment can mean the whole edge. Only when the kernel cannot walk
    // the edges does the outline fall back to the triangle boundaries,
    // which name nothing.
    if detail.generate_boundary_edges {
        let outlines = keys.as_ref().zip(reuse);
        match kernel_edge_outline(model, root, &chords, deflection, tol, outlines) {
            Ok(outline) if !outline.positions.is_empty() => {
                let base = mesh.positions.len() as u32;
                let count = outline.positions.len();
                mesh.positions.extend(outline.positions);
                mesh.normals
                    .extend(std::iter::repeat_n([0.0, 0.0, 1.0], count));
                if !mesh.colors.is_empty() {
                    mesh.colors.extend(std::iter::repeat_n(WHITE, count));
                }
                mesh.edges = outline.segments.iter().map(|i| base + i).collect();
                mesh.edge_ids = outline.edge_ids;
                if let Ok(edges) = explore_unique(model, root, ShapeType::Edge) {
                    mesh.edge_curves = edge_curves(model, &edges, tol);
                    if let Some(names) = names {
                        mesh.edge_faces = crate::naming::edge_faces(model, names, &edges);
                    }
                }
            }
            Ok(_) => mesh.edges = extract_boundary_edges(&mesh.indices),
            Err(e) => {
                warn!(target: "printcad.kernel", "edge outline failed ({e}); drawing triangle boundaries");
                mesh.edges = extract_boundary_edges(&mesh.indices);
            }
        }
    }
    // A solid that brought no colours of its own carries none, so it draws
    // in the body's colour rather than as white.
    if face_colors.is_empty() {
        mesh.colors.clear();
    }
    if mesh.face_other_names.iter().all(Vec::is_empty) {
        mesh.face_other_names.clear();
    }
    Ok(mesh)
}

/// The kernel's edges as polylines: their points, the segments over them
/// (index pairs into those points) and the edge each segment belongs to,
/// numbered in the kernel's own exploration order.
struct EdgeOutline {
    positions: Vec<[f32; 3]>,
    segments: Vec<u32>,
    edge_ids: Vec<u32>,
}

/// With `reuse`, an edge drawn alike by the build before keeps its
/// polyline, and the edges drawn now are kept for the next.
fn kernel_edge_outline(
    model: &Model,
    root: &Shape,
    chords: &ogeom::mesh::EdgeChords,
    deflection: Deflection,
    tol: Tolerances,
    reuse: Option<(&crate::reuse::FaceKeys, &mut crate::reuse::FaceMeshes)>,
) -> KernelResult<EdgeOutline> {
    let mut drawn = std::collections::HashMap::new();
    let edges = explore_unique(model, root, ShapeType::Edge)
        .map_err(|e| KernelError::Other(anyhow::anyhow!("edge exploration failed: {e}")))?;
    let mut outline = EdgeOutline {
        positions: Vec::new(),
        segments: Vec::new(),
        edge_ids: Vec::new(),
    };
    for (edge_id, edge) in edges.iter().enumerate() {
        let chord = chords
            .get(&edge.node().index())
            .copied()
            .unwrap_or(deflection.chord);
        let drawn_to = Deflection {
            chord: chord.min(deflection.chord),
            ..deflection
        };
        let key = reuse
            .as_ref()
            .and_then(|(keys, _)| keys.edge(model, edge, drawn_to));
        let kept = key.and_then(|k| reuse.as_ref().and_then(|(_, m)| m.edge(k)));
        let points = match kept {
            Some(points) => points,
            None => match polyline_of_edge(model, edge, drawn_to, tol) {
                Ok(points) => std::sync::Arc::new(points),
                Err(e) => {
                    warn!(target: "printcad.kernel", edge = edge_id, "edge polyline failed: {e}");
                    continue;
                }
            },
        };
        if let Some(key) = key {
            drawn.insert(key, std::sync::Arc::clone(&points));
        }
        if points.len() < 2 {
            continue;
        }
        let base = outline.positions.len() as u32;
        outline
            .positions
            .extend(points.iter().map(|p| [p.x as f32, p.y as f32, p.z as f32]));
        for i in 1..points.len() as u32 {
            outline.segments.push(base + i - 1);
            outline.segments.push(base + i);
            outline.edge_ids.push(edge_id as u32);
        }
    }
    if let Some((_, meshes)) = reuse {
        meshes.replace_edges(drawn);
    }
    Ok(outline)
}

/// Each of `edges`' curves and lengths, in their order, placed as the
/// edges are. They go wide, a length off a spline being an integral, or
/// stay on the calling thread when it is already one of the kernel's
/// workers.
fn edge_curves(model: &Model, edges: &[Shape], tol: Tolerances) -> Vec<kernel_api::EdgeCurve> {
    map_ordered(edges, |_, edge| edge_curve(model, edge, tol))
}

/// What `edge`'s curve is, in the shape's frame, and its arc length. An
/// edge with no curve (a pole) is another curve of no length.
pub(crate) fn edge_curve(model: &Model, edge: &Shape, tol: Tolerances) -> kernel_api::EdgeCurve {
    use kernel_api::{EdgeCurve, EdgeKind};
    use ogeom::geom::{Curve, Transformable as _};
    use ogeom::topo::EdgeRepr;
    let Some(NodeData::Edge(data)) = model.node(edge).map(|n| n.data()) else {
        return EdgeCurve::default();
    };
    if data.degenerate {
        return EdgeCurve::default();
    }
    let Some(EdgeRepr::Curve3d {
        curve,
        location,
        range,
    }) = data.curve3d()
    else {
        return EdgeCurve::default();
    };
    let placed = (|| {
        let geometry = model.geometry().curve(*curve)?;
        let local = location.composed(model.datums()).ok()?;
        let placement = edge.transform(model.datums()).ok()?;
        geometry
            .transformed(&local, tol)
            .ok()?
            .transformed(&placement, tol)
            .ok()
    })();
    let Some(curve) = placed else {
        return EdgeCurve::default();
    };
    let f = |p: Point| [p.x as f32, p.y as f32, p.z as f32];
    let integrated = || ogeom::algo::curve_length(&curve, *range, tol).unwrap_or(f64::NAN);
    // A trim runs on its basis's parameter, so its basis says what it is.
    let mut basis = &curve;
    while let Curve::Trimmed(trim) = basis {
        basis = trim.basis();
    }
    let (kind, length) = match basis {
        Curve::Line(_) => (EdgeKind::Line, integrated()),
        Curve::Circle(c) => {
            let circle = c.circle();
            let n = circle.frame().z().vector();
            (
                EdgeKind::Circle {
                    center: f(circle.centre()),
                    normal: [n.x as f32, n.y as f32, n.z as f32],
                    radius: circle.radius() as f32,
                },
                circle.radius() * (range.1 - range.0).abs(),
            )
        }
        _ => (EdgeKind::Other, integrated()),
    };
    EdgeCurve {
        kind,
        length: length as f32,
    }
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
    for tri in indices.as_chunks::<3>().0 {
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
    for tri in indices.as_chunks::<3>().0 {
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

/// What a face's surface is, placed as the face is. A plane's normal is
/// the one the face's triangles face, whichever way the surface's own frame
/// points.
pub(crate) fn face_surface(model: &Model, face: &Shape, facing: [f32; 3]) -> FaceSurface {
    let Some(node) = model.node(face) else {
        return FaceSurface::Other;
    };
    let NodeData::Face(data) = node.data() else {
        return FaceSurface::Other;
    };
    let (Some(surface), Ok(placement)) = (
        model.geometry().surface(data.surface),
        face.transform(model.datums()),
    ) else {
        return FaceSurface::Other;
    };
    let point = |p: Point| {
        let p = placement.apply(p);
        [p.x as f32, p.y as f32, p.z as f32]
    };
    let direction = |d: ogeom::math::Direction| {
        let v = placement.apply_vector(d.vector());
        let len = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt().max(1e-12);
        [(v.x / len) as f32, (v.y / len) as f32, (v.z / len) as f32]
    };
    match surface {
        SurfaceGeometry::Plane(plane) => {
            let frame = plane.plane().frame();
            let mut normal = direction(frame.z());
            let dot: f32 = (0..3).map(|k| normal[k] * facing[k]).sum();
            if dot < 0.0 {
                normal = normal.map(|c| -c);
            }
            FaceSurface::Plane {
                origin: point(frame.origin()),
                normal,
            }
        }
        SurfaceGeometry::Cylinder(cylinder) => {
            let c = cylinder.cylinder();
            FaceSurface::Cylinder {
                origin: point(c.frame().origin()),
                axis: direction(c.frame().z()),
                radius: c.radius() as f32,
            }
        }
        SurfaceGeometry::Cone(cone) => {
            let c = cone.cone();
            FaceSurface::Cone {
                apex: point(c.apex()),
                axis: direction(c.frame().z()),
                half_angle: c.half_angle().abs() as f32,
            }
        }
        SurfaceGeometry::Sphere(sphere) => {
            let s = sphere.sphere();
            FaceSurface::Sphere {
                center: point(s.centre()),
                radius: s.radius() as f32,
            }
        }
        SurfaceGeometry::Torus(torus) => {
            let t = torus.torus();
            FaceSurface::Torus {
                center: point(t.centre()),
                axis: direction(t.frame().z()),
                major_radius: t.major_radius() as f32,
                minor_radius: t.minor_radius() as f32,
            }
        }
        _ => FaceSurface::Other,
    }
}

/// Merge vertices at one point (the crate's `SAME_POINT`) whose colors match exactly and whose owning
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

    let key_of = |p: &[f32; 3]| crate::mesh_intake::SAME_POINT.key(*p);

    struct BucketEntry {
        canonical_index: u32,
        face_normal: [f32; 3],
        color_packed: u32,
    }

    let mut buckets: HashMap<[i64; 3], Vec<BucketEntry>> = HashMap::with_capacity(old_vertex_count);
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
            colors: vec![WHITE; 4],
            ..TriMesh::default()
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
