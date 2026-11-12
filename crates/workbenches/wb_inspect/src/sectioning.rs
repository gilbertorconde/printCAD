//! Sections and shadows: their inputs, the plane a new one cuts along,
//! their cuts kept by what they were cut from, and their answers.

use crate::{InspectionWorkbench, element, making::element_at, section, shadow, vec};
use core_document::{BodyId, CommandError, Document};
use glam::DVec3;
use serde_json::{Value, json};

/// A section's settings, the scan's mesh it cuts and a key that changes
/// with either.
fn section_inputs(
    document: &Document,
    id: core_document::FeatureId,
) -> Option<(section::Section, std::sync::Arc<kernel_api::TriMesh>, u64)> {
    let s = document.feature_as::<section::Section>(id)?;
    let (mesh, _) = document.local_geometry(s.scan)?;
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        document
            .imported_geometry(s.scan)
            .map(|g| g.revision)
            .hash(&mut h);
        serde_json::to_string(&s).unwrap_or_default().hash(&mut h);
        h.finish()
    };
    Some((s, mesh, key))
}

/// The plane a new section cuts along: across an element (or through its
/// axis, at `angle` degrees about it), else a coordinate plane through the
/// scan's middle. Returns the scan, origin, normal and half-plane side, in
/// the scan's frame.
#[allow(clippy::type_complexity)]
pub(crate) fn section_plane(
    document: &Document,
    element: Option<core_document::FeatureId>,
    scan: Option<BodyId>,
    plane: &str,
    through: bool,
    angle: f64,
) -> Result<(BodyId, [f64; 3], [f64; 3], Option<[f64; 3]>), CommandError> {
    let mid = |t: [f64; 2]| (t[0] + t[1]) / 2.0;
    if let Some(id) = element {
        let e = element_at(document, id)
            .ok_or_else(|| CommandError::bad("element", "is not an element"))?;
        // The middle of the axis's span; an axis with no direction has none.
        let axis_at = |point: [f64; 3], axis: [f64; 3]| {
            let (p, a) = (DVec3::from(point), DVec3::from(axis));
            match e.result.span {
                element::Span::Axis(t) if vec::unit(a).is_some() => {
                    (p + a * (mid(t) / a.length())).to_array()
                }
                _ => point,
            }
        };
        let (origin, axis) = match e.result.shape {
            kernel_api::FittedShape::Plane { origin, normal } => {
                if through {
                    return Err(CommandError::bad("through", "needs an axis, not a plane"));
                }
                let origin = match e.result.span {
                    element::Span::Patch { u, across } => {
                        let u = DVec3::from(u);
                        let v = DVec3::from(normal).cross(u);
                        (DVec3::from(origin) + (u * mid(across[0]) + v * mid(across[1]))).to_array()
                    }
                    _ => origin,
                };
                return Ok((e.scan, origin, normal, None));
            }
            kernel_api::FittedShape::Cylinder { point, axis, .. } => (axis_at(point, axis), axis),
            kernel_api::FittedShape::Cone { apex, axis, .. } => (axis_at(apex, axis), axis),
            kernel_api::FittedShape::Line { point, direction } => {
                (axis_at(point, direction), direction)
            }
            kernel_api::FittedShape::Circle { centre, normal, .. } => (centre, normal),
            kernel_api::FittedShape::Torus { centre, axis, .. } => (centre, axis),
            _ => {
                return Err(CommandError::bad(
                    "element",
                    "has no direction to cut across: pick a plane, an axis or a circle",
                ));
            }
        };
        if !through {
            return Ok((e.scan, origin, axis, None));
        }
        let axis = DVec3::from(axis);
        let (u, v) = vec::basis(axis)
            .ok_or_else(|| CommandError::bad("element", "has no direction to cut through"))?;
        let (sin, cos) = angle.to_radians().sin_cos();
        let along = u * cos + v * sin;
        return Ok((
            e.scan,
            origin,
            axis.cross(along).to_array(),
            Some(along.to_array()),
        ));
    }
    let scan = scan.ok_or_else(|| CommandError::bad("scan", "or an element is needed"))?;
    let (mesh, _) = document
        .local_geometry(scan)
        .ok_or_else(|| CommandError::bad("scan", "has no mesh"))?;
    let (lo, hi) = mesh
        .bounds()
        .ok_or_else(|| CommandError::bad("scan", "has no mesh"))?;
    let (lo, hi) = (lo.map(f64::from), hi.map(f64::from));
    let world = match plane {
        "xy" | "XY" => [0.0, 0.0, 1.0],
        "xz" | "XZ" => [0.0, 1.0, 0.0],
        "yz" | "YZ" => [1.0, 0.0, 0.0],
        _ => return Err(CommandError::bad("plane", "is xy, xz or yz")),
    };
    let normal = document
        .body_placement(scan)
        .inverse()
        .direction(world)
        .map(f64::from);
    Ok((
        scan,
        ((DVec3::from(lo) + DVec3::from(hi)) * 0.5).to_array(),
        normal,
        None,
    ))
}

/// The shadow features of `body`, with their evaluated data.
pub(crate) fn shadows_on(
    document: &Document,
    body: BodyId,
) -> Vec<(core_document::FeatureId, shadow::Shadow)> {
    let mut found: Vec<_> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == shadow::KIND && n.body == Some(body))
        .filter_map(|(id, n)| {
            let s = document.feature_as::<shadow::Shadow>(*id)?;
            Some((n.seq, *id, s))
        })
        .collect();
    found.sort_by_key(|(seq, id, _)| (*seq, *id));
    found.into_iter().map(|(_, id, s)| (id, s)).collect()
}

pub(crate) fn shadow_answer(
    id: core_document::FeatureId,
    body: BodyId,
    s: &shadow::Shadow,
    made: &shadow::Made,
) -> Value {
    json!({
        "shadow": id.0.to_string(),
        "body": body.0.to_string(),
        "name": s.name,
        "loops": made.outline.len(),
        "area": made.area,
        "band": made.band,
        "depth": made.depth,
        "error": made.error,
    })
}

/// What a section reads, cut by cut.
pub(crate) fn section_answer(
    id: core_document::FeatureId,
    s: &section::Section,
    cuts: &[section::Cut],
) -> Value {
    json!({
        "section": id.0.to_string(),
        "name": s.name,
        "cuts": cuts.iter().map(|c| json!({
            "offset": c.offset,
            "chains": c.chains.len(),
            "closed": c.closed(),
            "length": c.length(),
            "area": c.area(s.normal),
        })).collect::<Vec<_>>(),
        "tracks": section::tracks(cuts, s.normal).iter().map(|t| json!({
            "cuts": t.cuts,
            "from": t.from,
            "to": t.to,
            "tilt": t.tilt_deg,
            "draft": t.draft_deg,
            "radius": t.radius,
            "roundness": t.roundness,
        })).collect::<Vec<_>>(),
    })
}

impl InspectionWorkbench {
    /// A section's cuts as the scan stands, cut now if they are not kept.
    pub(crate) fn cuts_of(
        &self,
        document: &Document,
        id: core_document::FeatureId,
    ) -> Option<(section::Section, std::sync::Arc<Vec<section::Cut>>, u64)> {
        let (s, mesh, key) = section_inputs(document, id)?;
        if let Some(cuts) = self.cuts.get(id, key) {
            return Some((s, cuts, key));
        }
        let kernel = *self.kernel.get()?;
        let cuts = section::cut(kernel, &mesh, &s).ok()?;
        let cuts = self.cuts.store(id, key, cuts);
        Some((s, cuts, key))
    }
}
