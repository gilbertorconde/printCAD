//! Projection caching and reference following for sketch geometry.

use crate::sketch::SketchPlane;
use crate::{
    SketchFeature, external, external_ref, feature, params, placed_plane, sketch, sketch_placement,
    stored_sketch,
};
use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// What `source` comes to on the sketch plane `plane` (as the scene has
/// it), in the sketch's own coordinates: its edge projected onto the plane,
/// or the curves where its face crosses it.
pub(crate) fn project_source(
    ctx: &WorkbenchRuntimeContext,
    plane: &SketchPlane,
    source: &sketch::ExternalSource,
) -> Result<Vec<kernel_api::ProjectedEdge>, String> {
    project_source_in(ctx.document, ctx.kernel, plane, source)
}

/// [`project_source`] with the document and kernel given.
pub(super) fn project_source_in(
    document: &core_document::Document,
    kernel: Option<&dyn kernel_api::KernelQueries>,
    plane: &SketchPlane,
    source: &sketch::ExternalSource,
) -> Result<Vec<kernel_api::ProjectedEdge>, String> {
    if let Some(reference) = source.reference {
        return external_ref::project(document, plane, reference);
    }
    let kernel = kernel.ok_or("no kernel to project with")?;
    let body = BodyId(source.body);
    let brep = document
        .imported_brep_blob(body)
        .ok_or("that body has no solid shape")?;
    // The kernel works in the body's own frame: the plane goes there too,
    // and the projection keeps the sketch's own coordinates.
    let local = placed_plane(plane, &document.body_placement(body).inverse());
    let f = |v: [f64; 3]| v.map(f64::from);
    let plane = kernel_api::ProfilePlane {
        origin: f(local.origin),
        x_axis: f(local.x_axis),
        y_axis: f(local.y_axis),
        normal: f(local.normal),
    };
    if source.section {
        let curves = kernel
            .section_face(brep, f(source.point), &plane)
            .map_err(|e| e.to_string())?;
        if curves.is_empty() {
            return Err("the face does not cross the sketch plane".to_string());
        }
        return Ok(curves);
    }
    // A named edge is found by its faces' names wherever it went, the
    // point it was picked at after.
    let near = (source.faces != [0; 2])
        .then(|| {
            let [a, b] = source.faces;
            outline_point_near(document, body, source.point, |names| {
                names == [a, b] || names == [b, a]
            })
        })
        .flatten()
        .map_or(source.point, |(point, _)| point);
    kernel
        .project_edge(brep, f(near), &plane)
        .map(|edge| vec![edge])
        .map_err(|e| e.to_string())
}

/// `source` with the names of the faces its edge runs between, read from
/// its body's mesh where the edge nearest its point is, when it is an
/// edge of a solid and has none yet.
pub(crate) fn with_edge_names(
    document: &core_document::Document,
    mut source: sketch::ExternalSource,
) -> sketch::ExternalSource {
    if source.reference.is_none() && !source.section && source.faces == [0; 2] {
        source.faces = outline_point_near(document, BodyId(source.body), source.point, |_| true)
            .map(|(_, names)| names)
            .unwrap_or_default();
    }
    source
}

/// The point of `body`'s outline, in its own mesh and frame, nearest
/// `point` on an edge whose faces' names `takes` accepts, and those names.
/// `None` when its mesh has no edges it accepts.
fn outline_point_near(
    document: &core_document::Document,
    body: BodyId,
    point: [f64; 3],
    takes: impl Fn([kernel_api::TopoName; 2]) -> bool,
) -> Option<([f64; 3], [kernel_api::TopoName; 2])> {
    let (mesh, _) = document.local_geometry(body)?;
    let at = glam::DVec3::from_array(point);
    let position = |i: u32| glam::Vec3::from_array(mesh.positions[i as usize]).as_dvec3();
    let mut best: Option<(f64, glam::DVec3, [kernel_api::TopoName; 2])> = None;
    for (pair, edge) in mesh.edges.as_chunks::<2>().0.iter().zip(&mesh.edge_ids) {
        let names = mesh
            .edge_faces
            .get(*edge as usize)
            .copied()
            .unwrap_or_default();
        if !takes(names) {
            continue;
        }
        let (a, b) = (position(pair[0]), position(pair[1]));
        let along = b - a;
        let t = if along.length_squared() > 0.0 {
            ((at - a).dot(along) / along.length_squared()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let on = a + along * t;
        let distance = on.distance(at);
        if best.is_none_or(|(d, _, _)| distance < d) {
            best = Some((distance, on, names));
        }
    }
    best.map(|(_, on, names)| (on.to_array(), names))
}

/// A solid's edge or face projected onto a sketch plane: the curves it
/// comes to, or why it could not be projected.
pub(super) type Projection = Result<Vec<kernel_api::ProjectedEdge>, String>;

/// What projecting `source` onto `plane` (where the sketch's body sits)
/// gives depends on: the source, the plane, and the source's solid and
/// where its body sits.
pub(super) fn projection_key(
    document: &core_document::Document,
    plane: &SketchPlane,
    source: &sketch::ExternalSource,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{source:?} {plane:?}").hash(&mut hasher);
    let body = BodyId(source.body);
    document
        .imported_geometry(body)
        .map(|g| g.revision)
        .hash(&mut hasher);
    format!("{:?}", document.body_placement(body)).hash(&mut hasher);
    hasher.finish()
}

/// Whether `body`'s solid as it stands is built, in part, from `sketch`:
/// a feature taking from it, directly or through others, in the body's
/// history up to its tip. Following that solid's edges would chase the
/// sketch's own change.
pub(super) fn solid_built_from(
    document: &core_document::Document,
    sketch: FeatureId,
    body: BodyId,
) -> bool {
    let tree = document.feature_tree();
    let tip = document
        .bodies()
        .iter()
        .find(|b| b.id == body)
        .and_then(|b| b.tip)
        .and_then(|tip| tree.get_node(tip))
        .map(|n| n.seq);
    let mut seen = HashSet::new();
    let mut next = tree.dependents(sketch);
    while let Some(id) = next.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(node) = tree.get_node(id)
            && node.body == Some(body)
            && tip.is_none_or(|tip| node.seq <= tip)
        {
            return true;
        }
        next.extend(tree.dependents(id));
    }
    false
}

/// What sketch `id`'s lost external elements were worked out from: its
/// plane, its external elements' sources and where its body sits, the
/// solids they were projected from as they stand, what its body's build
/// projected for it, and for elements from other sketches or datums the
/// document's edit count. `None` when it has no external geometry.
pub(super) fn external_stamp(document: &core_document::Document, id: FeatureId) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let values = document.feature_values(id)?;
    let external = values.get("sketch")?.get("external")?;
    let sources: HashMap<Uuid, sketch::ExternalSource> =
        serde_json::from_value(external.clone()).ok()?;
    if sources.is_empty() {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    core_document::data_revision(external).hash(&mut hasher);
    values
        .get("plane")
        .map(core_document::data_revision)
        .hash(&mut hasher);
    format!("{:?}", sketch_placement(document, id)).hash(&mut hasher);
    let mut bodies: Vec<Uuid> = sources
        .values()
        .filter(|s| s.reference.is_none())
        .map(|s| s.body)
        .collect();
    bodies.sort();
    bodies.dedup();
    for body in bodies {
        let body = BodyId(body);
        document
            .imported_geometry(body)
            .map(|g| g.revision)
            .hash(&mut hasher);
        format!("{:?}", document.body_placement(body)).hash(&mut hasher);
    }
    if sources.values().any(|s| s.reference.is_some()) {
        document.mutation_seq().hash(&mut hasher);
    }
    format!("{:?}", document.projected_references(id)).hash(&mut hasher);
    Some(hasher.finish())
}

/// The external elements of sketch `id` whose source cannot be projected
/// again: an edge or face gone from its solid, a sketch element or datum
/// deleted. Solids' edges are passed over without a kernel.
pub(super) fn lost_external_elements(
    document: &core_document::Document,
    kernel: Option<&dyn kernel_api::KernelQueries>,
    id: FeatureId,
) -> Vec<Uuid> {
    let Some(feature) = stored_sketch(document, id) else {
        return Vec::new();
    };
    let plane = placed_plane(&feature.plane, &sketch_placement(document, id));
    let own_body = document.get_feature_meta(id).and_then(|n| n.body);
    let built = document.projected_references(id);
    external::groups(&feature.sketch)
        .into_iter()
        .filter(|(source, _)| {
            // From its own body: what the build projected where the sketch
            // stands, once it has.
            if source.reference.is_none() && own_body == Some(BodyId(source.body)) {
                let probe = feature::projection_probe(&feature.plane, source);
                if let Some(answer) = built.and_then(|b| b.answer(&probe)) {
                    return answer.is_err();
                }
                if solid_built_from(document, id, BodyId(source.body)) {
                    return false;
                }
            }
            (source.reference.is_some() || kernel.is_some())
                && project_source_in(document, kernel, &plane, source).is_err()
        })
        .flat_map(|(_, group)| group)
        .collect()
}

/// Move the sketch's geometry from other sketches' elements and datums to
/// where they stand in `values_of` (the working data of this evaluation),
/// in place: an element that is now another kind of curve, or one gone,
/// waits for the sketch to be edited. Answers whether anything moved.
pub(super) fn follow_references(
    feature: &mut SketchFeature,
    id: FeatureId,
    values_of: &dyn Fn(FeatureId) -> Option<serde_json::Value>,
    document: &core_document::Document,
) -> bool {
    let groups: Vec<_> = external::groups(&feature.sketch)
        .into_iter()
        .filter(|(source, _)| source.reference.is_some())
        .collect();
    if groups.is_empty() {
        return false;
    }
    // Another sketch as it builds: solved for what formulas gave it, once
    // however many of its elements come here.
    let solved: std::cell::RefCell<HashMap<FeatureId, Option<serde_json::Value>>> =
        Default::default();
    let settled = |other: FeatureId| -> Option<serde_json::Value> {
        if let Some(known) = solved.borrow().get(&other) {
            return known.clone();
        }
        let mut data = values_of(other);
        let is_sketch = document
            .get_feature_meta(other)
            .is_some_and(|n| n.workbench_id.as_str() == "wb.sketch");
        if is_sketch && let Some(data) = data.as_mut() {
            params::settle(data);
        }
        solved.borrow_mut().insert(other, data.clone());
        data
    };
    let placed = placed_plane(&feature.plane, &sketch_placement(document, id));
    let before = serde_json::to_value(&feature.sketch.geometry).ok();
    for (source, group) in groups {
        let Some(reference) = source.reference else {
            continue;
        };
        let Ok(projected) = external_ref::project_with(document, &settled, &placed, reference)
        else {
            continue;
        };
        let kept = feature.sketch.geometry.clone();
        if !external::refresh_in_place(&mut feature.sketch, &group, &projected) {
            feature.sketch.geometry = kept;
        }
    }
    serde_json::to_value(&feature.sketch.geometry).ok() != before
}
