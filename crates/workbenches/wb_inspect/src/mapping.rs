//! What the map tools make and answer: comparisons against a reference,
//! maps against an element, wall thickness, clearance and overhang.

use crate::{
    clearance, compare, compare::Comparison, element::Element, element_map, making::element_at,
    maps, overhang, thickness,
};
use core_document::{BodyId, Document, WorkbenchRuntimeContext};
use serde_json::{Value, json};

/// The scan and the reference a Compare takes: the selected body and the
/// one other body that has geometry, whichever of them is the scan.
pub(crate) fn compare_pair(ctx: &WorkbenchRuntimeContext) -> Option<(BodyId, BodyId)> {
    let document = &*ctx.document;
    let shaped: Vec<BodyId> = document
        .bodies()
        .iter()
        .map(|b| b.id)
        .filter(|b| document.imported_geometry(*b).is_some())
        .collect();
    let is_scan = |b: BodyId| document.body_scan(b).is_some();
    let selected = ctx
        .selected_body_id
        .map(BodyId)
        .filter(|b| shaped.contains(b));
    let pick = |scan: bool| -> Option<BodyId> {
        if let Some(b) = selected.filter(|b| is_scan(*b) == scan) {
            return Some(b);
        }
        let mut of_kind = shaped.iter().copied().filter(|b| is_scan(*b) == scan);
        let first = of_kind.next()?;
        of_kind.next().is_none().then_some(first)
    };
    Some((pick(true)?, pick(false)?))
}

/// Keep a comparison of `scan` with `reference`, measured at once.
pub(crate) fn add_comparison(
    ctx: &mut WorkbenchRuntimeContext,
    maps: &maps::Maps,
    c: Comparison,
) -> Result<(core_document::FeatureId, Value), String> {
    if ctx.document.body_scan(c.scan).is_none() {
        return Err("the first body is not a scan".into());
    }
    if c.scan == c.reference {
        return Err("a scan is compared with another body".into());
    }
    let (id, c, values) = maps::add(ctx, maps, c)?;
    Ok((id, comparison_answer(id, &c, &values)))
}

/// The comparison `id` with its inputs and distances: those measured
/// already when they are current, else measured now.
pub(crate) fn comparison_values(
    ctx: &WorkbenchRuntimeContext,
    maps: &maps::Maps,
    id: core_document::FeatureId,
) -> Result<(Comparison, compare::Inputs, std::sync::Arc<[f32]>), String> {
    let node = ctx
        .document
        .feature_tree()
        .get_node(id)
        .filter(|n| n.workbench_id.as_str() == compare::KIND)
        .ok_or("that is not a comparison")?;
    let _ = node;
    let c = ctx
        .document
        .feature_as::<Comparison>(id)
        .ok_or("that comparison cannot be read")?;
    let inputs = compare::inputs(ctx.document, &c)?;
    if let Some(values) = maps.values(id, inputs.key) {
        return Ok((c, inputs, values));
    }
    let kernel = ctx.kernel.ok_or("no kernel to measure with")?;
    let values = compare::measure(kernel, &inputs, &c)?;
    maps.store(id, inputs.key, values.clone());
    Ok((c, inputs, values))
}

/// The longest side of `mesh`'s bounding box.
pub(crate) fn longest_side(mesh: &kernel_api::TriMesh) -> f64 {
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for p in &mesh.positions {
        for i in 0..3 {
            low[i] = low[i].min(p[i]);
            high[i] = high[i].max(p[i]);
        }
    }
    (0..3)
        .map(|i| f64::from(high[i] - low[i]))
        .fold(0.0, f64::max)
}

/// The body a wall-thickness tool reads: the selected one with geometry.
pub(crate) fn thickness_body(ctx: &WorkbenchRuntimeContext) -> Option<BodyId> {
    ctx.selected_body_id
        .map(BodyId)
        .filter(|b| ctx.document.local_geometry(*b).is_some())
}

/// Keep a wall-thickness map of `t.body` and read it at once.
pub(crate) fn add_thickness(
    ctx: &mut WorkbenchRuntimeContext,
    maps: &maps::Maps,
    t: thickness::WallThickness,
) -> Result<Value, String> {
    let (id, t, values) = maps::add(ctx, maps, t)?;
    let r = thickness::read(&values, &t);
    let s = r.summary;
    Ok(json!({
        "thickness": id.0.to_string(),
        "name": t.name,
        "measured": s.map_or(0, |s| s.count),
        "unmeasured": s.map_or(values.len(), |s| s.unmeasured),
        "mean": s.map(|s| s.mean),
        "min": s.map(|s| s.min),
        "max": s.map(|s| s.max),
        "range": r.range,
        "thinner": r.thinner,
    }))
}

/// Keep a fit check of `c.part` in `c.housing` and read it at once.
pub(crate) fn add_clearance(
    ctx: &mut WorkbenchRuntimeContext,
    maps: &maps::Maps,
    c: clearance::Clearance,
) -> Result<Value, String> {
    if c.reach <= 0.0 {
        return Err("the reach is a distance above zero".into());
    }
    let (id, c, values) = maps::add(ctx, maps, c)?;
    let r = clearance::read(&values, &c);
    Ok(json!({
        "clearance": id.0.to_string(),
        "name": c.name,
        "measured": r.summary.map_or(0, |s| s.count),
        "unmeasured": r.summary.map_or(values.len(), |s| s.unmeasured),
        "interfering": r.interfering,
        "deepest": r.deepest,
        "narrowest": r.narrowest,
        "too_tight": r.too_tight,
    }))
}

/// Keep an overhang map of `o.body` and read it at once.
pub(crate) fn add_overhang(
    ctx: &mut WorkbenchRuntimeContext,
    maps: &maps::Maps,
    o: overhang::Overhang,
) -> Result<Value, String> {
    if !(0.0..90.0).contains(&o.limit_deg) {
        return Err("the limit is an angle from 0 to 90 degrees past vertical".into());
    }
    let (id, o, values) = maps::add(ctx, maps, o)?;
    let mesh = ctx
        .document
        .imported_geometry(o.body)
        .map(|g| std::sync::Arc::clone(&g.mesh))
        .ok_or("the body has no geometry to read")?;
    let (share, steepest) = overhang::read(&mesh, &values, o.limit_deg);
    Ok(json!({
        "overhang": id.0.to_string(),
        "name": o.name,
        "limit": o.limit_deg,
        "share": share,
        "steepest": steepest,
    }))
}

/// What an element map is measured from now: the scan's mesh, the
/// element, and a key that changes with either or the map's settings.
pub(crate) fn element_map_inputs(
    document: &Document,
    m: &element_map::ElementMap,
) -> Option<(std::sync::Arc<kernel_api::TriMesh>, Element, u64)> {
    let e = element_at(document, m.element)?;
    let (mesh, _) = document.local_geometry(m.scan)?;
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        document
            .imported_geometry(m.scan)
            .map(|g| g.revision)
            .hash(&mut h);
        serde_json::to_string(&e.result)
            .unwrap_or_default()
            .hash(&mut h);
        format!("{}{}", m.facing_limit_deg, m.flip).hash(&mut h);
        h.finish()
    };
    Some((mesh, e, key))
}

fn comparison_answer(id: core_document::FeatureId, c: &Comparison, values: &[f32]) -> Value {
    let r = compare::read(values, c);
    let s = r.summary;
    json!({
        "comparison": id.0.to_string(),
        "name": c.name,
        "scan": c.scan.0.to_string(),
        "reference": c.reference.0.to_string(),
        "range": r.range,
        "measured": s.map_or(0, |s| s.count),
        "unmeasured": s.map_or(values.len(), |s| s.unmeasured),
        "mean": s.map(|s| s.mean),
        "rms": s.map(|s| s.rms),
        "sigma": s.map(|s| s.sigma),
        "min": s.map(|s| s.min),
        "max": s.map(|s| s.max),
        "within": r.within,
    })
}
