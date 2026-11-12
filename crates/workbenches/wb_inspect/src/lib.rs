//! The Inspection workbench: measuring a scan. Its first tools fit a plane,
//! sphere, cylinder, cone, torus, circle or line to the part of a scan the
//! user marked (Mark surface…), by orthogonal least squares with points
//! beyond three sigmas left out, and keep the result as an element under
//! the scan: its size, its sigma, its form error, and the lines that draw
//! it no larger than measured. Dimensions between elements read
//! distances, angles and diameters off them, each with a nominal and a
//! band it passes or fails.

// Without the egui feature the crate is the headless subset its dependents'
// tests and the kernel bench use; the panel-only state and helpers go unused there.
#![cfg_attr(not(feature = "egui"), allow(dead_code, unused_imports))]

pub mod align;
mod align_commands;
mod best_fit;
pub mod clearance;
mod commands;
pub mod compare;
pub mod compensation;
pub mod construction;
pub mod dimension;
pub mod element;
pub mod element_map;
pub mod export;
pub mod extract;
mod fitting;
use fitting::{answer_of, fit_marked, grow_at, kind_named, method_arg};
pub mod hotspots;
pub mod maps;
pub mod overhang;
mod picks;
pub mod pins;
mod pmi;
pub mod report;
use report::report_of;
pub mod section;
pub mod shadow;
pub mod thickness;
pub mod tolerance;
mod tolerance_read;
mod tolerance_task;
mod vec;
pub mod volume;

use compare::Comparison;
use construction::{Construction, Made};
use core_document::units::show_mm_signed;
use core_document::{
    BodyId, CommandArgs, CommandError, CommandResult, Document, FeatureInfo, FeatureNode,
    InputResult, PassiveGeometry, PassiveTint, ToolDescriptor, WorkbenchContext,
    WorkbenchDescriptor, WorkbenchFeature, WorkbenchInputEvent, WorkbenchRuntimeContext,
};
use dimension::{DimKind, Dimension};
use element::Element;
#[cfg(test)]
use element::Fitted;
use glam::DVec3;
use kernel_api::FitKind;
#[cfg(test)]
use kernel_api::FitRequest;
use serde_json::{Value, json};

/// The workbench's id.
pub const ID: &str = "wb.inspect";

/// Points farther from the fit than this many sigmas are left out.
const DEFAULT_CUTOFF: f64 = 3.0;

const FIT_COMMAND: &str = "inspect.fit";
const REPORT_COMMAND: &str = "inspect.report";
const DIMENSION_COMMAND: &str = "inspect.dimension";
const CONSTRUCT_COMMAND: &str = "inspect.construct";
const GROW_COMMAND: &str = "inspect.grow";
const COMPARE_COMMAND: &str = "inspect.compare";
const COMPARE_TOOL: &str = "inspect.compare";
const BEST_FIT: &str = "inspect.best_fit";
const RESET_ALIGNMENT: &str = "inspect.reset_alignment";
const MOVE_COMMAND: &str = "inspect.move";
const HOT_SPOTS: &str = "inspect.hot_spots";
const VOLUME_ERRORS: &str = "inspect.volume_errors";
const THICKNESS: &str = "inspect.walls";
const OVERHANG: &str = "inspect.overhang";
const CLEARANCE: &str = "inspect.clearance";
const ELEMENT_SOLID: &str = "inspect.element_solid";
const EXTRACT: &str = "inspect.extract";
const TOLERANCE: &str = "inspect.tolerance";
const SAVE_REPORT: &str = "inspect.save_report";
const ELEMENT_MAP: &str = "inspect.element_map";
const PIN: &str = "inspect.pin";
const SECTION: &str = "inspect.section";
const COMPENSATION: &str = "inspect.compensation";
const EXPORT_STEP: &str = "inspect.export_step";
const SAVE_STEP: &str = "inspect.save_step";
const EXPORT_IGES: &str = "inspect.export_iges";
const SAVE_IGES: &str = "inspect.save_iges";
const SHADOW: &str = "inspect.shadow";

/// How far from the cursor a reading looks for a scan point, mm.
const READING_REACH: f64 = 2.0;

/// The limit a tolerance made from the toolbar starts at, mm.
const DEFAULT_LIMIT: f64 = 0.1;
const MAP_EXPORT: &str = "inspect.map_export";

/// Hot spots labelled on the part, the largest first.
const LABELLED_SPOTS: usize = 5;
/// The fewest vertices a labelled hot spot has.
const SPOT_LEAST: usize = 3;

/// A point element's cross across, in logical pixels.
const POINT_CROSS_PT: f32 = 14.0;

/// How deep constructions on constructions are followed.
const DEEPEST: usize = 32;

/// What the bench holds for the document on screen, moved with its tab.
#[derive(Default)]
struct Tab {
    /// The fit a click on a scan makes, while nothing is marked.
    armed: Option<FitKind>,
    /// A click on a mapped scan pins a reading there.
    pinning: bool,
    /// The clicks of an element in progress, pinned until it is made.
    picks: picks::Picks,
    tolerance_task: Option<tolerance_task::Task>,
    /// The best fit the toolbar started, while it runs and until applied.
    fitting: best_fit::Fitting,
    /// Every map's values, measured away from the window.
    maps: maps::Maps,
    tolerance_readings: tolerance_read::Readings,
}

#[derive(Default)]
pub struct InspectionWorkbench {
    tab: Tab,
    /// Every section's cuts, by what they were cut from.
    cuts: section::Cuts,
    /// The kernel the host hands the bench, kept for drawing sections.
    kernel: std::sync::OnceLock<&'static dyn kernel_api::KernelQueries>,
}

/// Move `scan` to `after` and keep how as an alignment; the answer says
/// where it went.
fn record_alignment(
    ctx: &mut WorkbenchRuntimeContext,
    scan: BodyId,
    how: align::How,
    after: core_document::BodyPlacement,
) -> Result<Value, String> {
    if ctx.document.body_scan(scan).is_none() {
        return Err("only a scan is aligned".into());
    }
    let before = ctx.document.body_placement(scan);
    let a = align::Alignment {
        name: numbered(ctx.document, "Alignment"),
        scan,
        how,
        before,
        after,
    };
    // The step that can refuse goes first, so a refusal moves nothing.
    let id = ctx
        .document
        .add_feature_in_body(a.clone(), a.name.clone(), Some(scan))
        .map_err(|e| e.to_string())?;
    ctx.document.set_body_placement(scan, after);
    ctx.log_info(format!("{}: {}", a.how.word(), a.how.summary()));
    let mut out = json!({
        "alignment": id.0.to_string(),
        "name": a.name,
        "translation": after.translation,
        "rotation": after.rotation,
    });
    if let align::How::BestFit {
        rms, inliers, free, ..
    } = a.how
    {
        out["rms"] = json!(rms);
        out["inliers"] = json!(inliers);
        out["free"] = json!(free);
    }
    Ok(out)
}

fn best_fit_how(reference: BodyId, search: bool, fit: &kernel_api::Registration) -> align::How {
    align::How::BestFit {
        marked: false,
        reference,
        search,
        rms: fit.rms,
        inliers: fit.inliers,
        sampled: fit.sampled,
        free: fit.free.len(),
        converged: fit.converged,
    }
}

/// The scan and the reference a Compare takes: the selected body and the
/// one other body that has geometry, whichever of them is the scan.
fn compare_pair(ctx: &WorkbenchRuntimeContext) -> Option<(BodyId, BodyId)> {
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
fn add_comparison(
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
fn comparison_values(
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
fn longest_side(mesh: &kernel_api::TriMesh) -> f64 {
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
fn thickness_body(ctx: &WorkbenchRuntimeContext) -> Option<BodyId> {
    ctx.selected_body_id
        .map(BodyId)
        .filter(|b| ctx.document.local_geometry(*b).is_some())
}

/// Keep a wall-thickness map of `t.body` and read it at once.
fn add_thickness(
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
fn add_clearance(
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
fn add_overhang(
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

/// The tolerance `id` is and what it reads.
fn tolerance_at(
    document: &Document,
    id: core_document::FeatureId,
    readings: &tolerance_read::Readings,
    kernel: Option<&'static dyn kernel_api::KernelQueries>,
    wait: bool,
) -> Option<(tolerance::Tolerance, Result<tolerance::Evaluation, String>)> {
    let t = document.feature_as::<tolerance::Tolerance>(id)?;
    let reading = readings.read(document, id, &t, kernel, wait);
    Some((t, reading))
}

fn tolerance_answer(
    id: core_document::FeatureId,
    t: &tolerance::Tolerance,
    reading: &Result<tolerance::Evaluation, String>,
) -> Value {
    let mut out = json!({
        "tolerance": id.0.to_string(),
        "name": t.name,
        "characteristic": t.characteristic.key(),
        "element": t.element.0.to_string(),
        "datum": t.datum.map(|d| d.0.to_string()),
        "datums": t.references(),
        "basic": t.basic,
        "material": t.material,
        "unequal": t.unequal,
        "section": t.section,
        "limit": t.limit,
    });
    match reading {
        Ok(e) => {
            out["value"] = json!(e.value);
            out["pass"] = json!(t.accepts(e));
            out["bonus"] = json!(e.bonus);
            out["allowed"] = json!(t.limit + e.bonus);
            out["datum_shift"] = json!(e.datum_shift);
            out["datum_labels"] = json!(e.datums);
            if let Some(note) = &e.note {
                out["note"] = json!(note);
            }
        }
        Err(why) => out["error"] = json!(why),
    }
    out
}

/// The characteristic the toolbar reads for `picked`: a form tolerance for
/// one element, for two the attitude the first holds to the second.
fn characteristic_for(picked: &[Element]) -> Option<(tolerance::Characteristic, Option<f64>)> {
    use kernel_api::FittedShape as S;
    use tolerance::Characteristic as C;
    // An axis or a normal with no direction suggests nothing.
    let direction = |e: &Element| {
        match e.result.shape {
            S::Plane { normal, .. } => Some((DVec3::from(normal), true)),
            S::Cylinder { axis, .. }
            | S::Cone { axis, .. }
            | S::Line {
                direction: axis, ..
            } => Some((DVec3::from(axis), false)),
            _ => None,
        }
        .filter(|(d, _)| vec::unit(*d).is_some())
    };
    match picked {
        [one] => C::of_form(one.kind).map(|c| (c, None)),
        [a, b] => {
            let centre = |e: &Element| {
                matches!(
                    e.result.shape,
                    S::Sphere { .. } | S::Circle { .. } | S::Torus { .. }
                )
            };
            if centre(a) {
                return Some((C::Concentricity, None));
            }
            let ((da, plane_a), (db, plane_b)) = (direction(a)?, direction(b)?);
            let cos = (da.dot(db) / (da.length() * db.length())).clamp(-1.0, 1.0);
            let angle = cos.abs().acos().to_degrees();
            Some(match (plane_a, plane_b) {
                (true, true) if angle < 5.0 => (C::Parallelism, None),
                (true, true) if angle > 85.0 => (C::Perpendicularity, None),
                (true, true) => (C::Angularity, Some(cos.acos().to_degrees().round())),
                (false, false) if angle < 5.0 => (C::Coaxiality, None),
                (false, true) if angle < 5.0 => (C::Perpendicularity, None),
                (true, false) if angle < 5.0 => (C::Perpendicularity, None),
                (true, false) => (C::Parallelism, None),
                _ => (C::Parallelism, None),
            })
        }
        _ => None,
    }
}

/// Keep a tolerance on `t.element` and read it at once.
fn add_tolerance(
    ctx: &mut WorkbenchRuntimeContext,
    mut t: tolerance::Tolerance,
) -> Result<Value, String> {
    let e = element_at(ctx.document, t.element).ok_or("the element is not an element")?;
    t.scan = e.scan;
    t.name = numbered(ctx.document, t.characteristic.word());
    let reading = tolerance_read::evaluate(
        ctx.document,
        &t,
        ctx.kernel.map(|k| k as &dyn kernel_api::KernelQueries),
    )?;
    let id = ctx
        .document
        .add_feature_in_body(t.clone(), t.name.clone(), Some(e.scan))
        .map_err(|e| e.to_string())?;
    Ok(tolerance_answer(id, &t, &Ok(reading)))
}

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
fn section_plane(
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
fn shadows_on(
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

fn shadow_answer(
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

fn write_construction(
    kernel: &dyn kernel_api::KernelQueries,
    items: &[kernel_api::ConstructionItem],
    iges: bool,
) -> kernel_api::KernelResult<(Vec<u8>, Vec<String>)> {
    if iges {
        let written = kernel.write_construction_iges("Inspection", items)?;
        Ok((written.text.into_bytes(), written.warnings))
    } else {
        kernel
            .write_construction_step("Inspection", items)
            .map(|bytes| (bytes, Vec::new()))
    }
}

/// `shape` of `e` as construction geometry, in the scan's frame; an
/// element with no direction gives none.
fn construction_of(e: &Element) -> Vec<(String, kernel_api::ConstructionShape)> {
    construction_items(e).unwrap_or_default()
}

fn construction_items(e: &Element) -> Option<Vec<(String, kernel_api::ConstructionShape)>> {
    use kernel_api::{ConstructionShape as S, FittedShape as F};
    let name = e.name.clone();
    let unit = |a: [f64; 3]| vec::unit(a.into());
    let along = |at: [f64; 3], d: DVec3, t: f64| (DVec3::from(at) + d * t).to_array();
    let axis = |name: &str, at: [f64; 3], a: [f64; 3]| {
        Some((
            format!("{name} axis"),
            S::Axis {
                origin: at,
                z: unit(a)?.to_array(),
                x: vec::basis(a.into())?.0.to_array(),
            },
        ))
    };
    let span = match e.result.span {
        element::Span::Axis(t) => Some(t),
        _ => None,
    };
    Some(match (e.result.shape.clone(), e.result.span) {
        (F::Plane { origin, normal }, element::Span::Patch { u, across }) => vec![(
            name,
            S::Plane {
                origin,
                normal: unit(normal)?.to_array(),
                x_axis: u,
                u: across[0],
                v: across[1],
            },
        )],
        (F::Plane { origin, normal }, _) => vec![(
            name,
            S::Plane {
                origin,
                normal: unit(normal)?.to_array(),
                x_axis: vec::basis(normal.into())?.0.to_array(),
                u: [-10.0, 10.0],
                v: [-10.0, 10.0],
            },
        )],
        (
            F::Cylinder {
                point,
                axis: a,
                radius,
            },
            _,
        ) => {
            let d = unit(a)?;
            let [t0, t1] = span.unwrap_or([0.0, 0.0]);
            let mut out = vec![axis(&name, along(point, d, t0), d.to_array())?];
            for (end, t) in [("start", t0), ("end", t1)] {
                out.push((
                    format!("{name} {end}"),
                    S::Circle {
                        centre: along(point, d, t),
                        normal: d.to_array(),
                        radius,
                    },
                ));
            }
            out
        }
        (
            F::Cone {
                apex,
                axis: a,
                half_angle,
            },
            _,
        ) => {
            let d = unit(a)?;
            let mut out = vec![axis(&name, apex, d.to_array())?];
            if let Some([t0, t1]) = span {
                for (end, t) in [("start", t0), ("end", t1)] {
                    let radius = (t * half_angle.tan()).abs();
                    if radius > 0.0 {
                        out.push((
                            format!("{name} {end}"),
                            S::Circle {
                                centre: along(apex, d, t),
                                normal: d.to_array(),
                                radius,
                            },
                        ));
                    }
                }
            }
            out
        }
        (F::Sphere { centre, .. }, _) => vec![(name, S::Point(centre))],
        (
            F::Torus {
                centre,
                axis: a,
                major,
                ..
            },
            _,
        ) => vec![
            axis(&name, centre, a)?,
            (
                format!("{name} spine"),
                S::Circle {
                    centre,
                    normal: unit(a)?.to_array(),
                    radius: major,
                },
            ),
        ],
        (
            F::Circle {
                centre,
                normal,
                radius,
            },
            _,
        ) => vec![(
            name,
            S::Circle {
                centre,
                normal: unit(normal)?.to_array(),
                radius,
            },
        )],
        (F::Line { point, direction }, _) => {
            let d = unit(direction)?;
            let [t0, t1] = span.unwrap_or([-10.0, 10.0]);
            vec![(
                name,
                S::Polyline {
                    points: vec![along(point, d, t0), along(point, d, t1)],
                    closed: false,
                },
            )]
        }
    })
}

/// `shape` moved by a body's placement.
fn placed_shape(
    rows: [[f64; 4]; 4],
    shape: kernel_api::ConstructionShape,
) -> kernel_api::ConstructionShape {
    use kernel_api::ConstructionShape as S;
    let p = |v: [f64; 3]| -> [f64; 3] {
        std::array::from_fn(|r| {
            rows[r][0] * v[0] + rows[r][1] * v[1] + rows[r][2] * v[2] + rows[r][3]
        })
    };
    let d = |v: [f64; 3]| -> [f64; 3] {
        std::array::from_fn(|r| rows[r][0] * v[0] + rows[r][1] * v[1] + rows[r][2] * v[2])
    };
    match shape {
        S::Point(a) => S::Point(p(a)),
        S::Axis { origin, z, x } => S::Axis {
            origin: p(origin),
            z: d(z),
            x: d(x),
        },
        S::Polyline { points, closed } => S::Polyline {
            points: points.into_iter().map(p).collect(),
            closed,
        },
        S::Circle {
            centre,
            normal,
            radius,
        } => S::Circle {
            centre: p(centre),
            normal: d(normal),
            radius,
        },
        S::Plane {
            origin,
            normal,
            x_axis,
            u,
            v,
        } => S::Plane {
            origin: p(origin),
            normal: d(normal),
            x_axis: d(x_axis),
            u,
            v,
        },
    }
}

/// What a section reads, cut by cut.
fn section_answer(
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

/// What an element map is measured from now: the scan's mesh, the
/// element, and a key that changes with either or the map's settings.
fn element_map_inputs(
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

fn tool_id(kind: FitKind) -> String {
    format!("inspect.fit_{}", element::kind_word(kind).to_lowercase())
}

fn kind_of_tool(id: &str) -> Option<FitKind> {
    FitKind::ALL.into_iter().find(|k| tool_id(*k) == id)
}

fn dim_tool_id(kind: DimKind) -> String {
    format!("inspect.{}", kind.word().to_lowercase())
}

fn dim_kind_of_tool(id: &str) -> Option<DimKind> {
    DimKind::ALL.into_iter().find(|k| dim_tool_id(*k) == id)
}

/// `word 1`, `word 2`, ...: the next name of that kind no feature has.
pub(crate) fn numbered(document: &Document, word: &str) -> String {
    document.new_object_name(word, core_document::Numbering::Counted)
}

fn construct_tool_id(made: Made) -> String {
    format!("inspect.construct_{}", made.word().to_lowercase())
}

fn made_of_tool(id: &str) -> Option<Made> {
    Made::ALL.into_iter().find(|m| construct_tool_id(*m) == id)
}

/// The element `id` is, fitted or constructed, with the values the
/// document holds for it.
fn element_at(document: &Document, id: core_document::FeatureId) -> Option<Element> {
    built_at(document, id, 0).ok()
}

/// The element `id` is, a construction worked out from its sources.
fn built_at(
    document: &Document,
    id: core_document::FeatureId,
    depth: usize,
) -> Result<Element, String> {
    let gone = || "an element it is made from is gone".to_string();
    let node = document.feature_tree().get_node(id).ok_or_else(gone)?;
    let data = document.feature_values(id).ok_or_else(gone)?;
    match node.workbench_id.as_str() {
        element::KIND => Element::from_json(data).map_err(|e| e.to_string()),
        construction::KIND if depth < DEEPEST => {
            let c = Construction::from_json(data).map_err(|e| e.to_string())?;
            let from = c
                .from
                .iter()
                .map(|f| built_at(document, *f, depth + 1))
                .collect::<Result<Vec<_>, _>>()?;
            construction::build(&c, &from)
        }
        _ => Err(gone()),
    }
}

/// Make `made` from `from` and keep it as a constructed element.
fn add_construction(
    ctx: &mut WorkbenchRuntimeContext,
    made: Made,
    from: Vec<core_document::FeatureId>,
    offset: f64,
) -> Result<(core_document::FeatureId, Value), String> {
    let found: Vec<Element> = from
        .iter()
        .map(|id| element_at(ctx.document, *id).ok_or(format!("{} is not an element", id.0)))
        .collect::<Result<_, _>>()?;
    let scan = found
        .first()
        .ok_or("pick the elements it is made from")?
        .scan;
    if found.iter().any(|e| e.scan != scan) {
        return Err("the elements are on different scans".into());
    }
    let c = Construction {
        name: numbered(ctx.document, made.word()),
        scan,
        made,
        from,
        offset,
    };
    let built = construction::build(&c, &found)?;
    let id = ctx
        .document
        .add_feature_in_body(c.clone(), c.name.clone(), Some(scan))
        .map_err(|e| e.to_string())?;
    Ok((id, construction_answer(id, &c, &Ok(built))))
}

fn construction_answer(
    id: core_document::FeatureId,
    c: &Construction,
    built: &Result<Element, String>,
) -> Value {
    let mut out = json!({
        "element": id.0.to_string(),
        "name": c.name,
        "made": c.made.word().to_lowercase(),
        "from": c.from.iter().map(|e| e.0.to_string()).collect::<Vec<_>>(),
        "offset": c.offset,
    });
    match built {
        Ok(e) => {
            out["shape"] = match e.result.shape {
                kernel_api::FittedShape::Sphere {
                    centre,
                    radius: 0.0,
                } => {
                    json!({"Point": {"at": centre}})
                }
                ref shape => json!(shape),
            }
        }
        Err(why) => out["error"] = json!(why),
    }
    out
}

/// A number that changes whenever `e`'s shape does.
fn shape_revision(e: &Element) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(&e.result.shape)
        .unwrap_or_default()
        .hash(&mut h);
    format!("{:?}", e.result.span).hash(&mut h);
    h.finish()
}

/// The dimension `id` is and what it reads now.
fn dimension_at(
    document: &Document,
    id: core_document::FeatureId,
) -> Option<(Dimension, Result<dimension::Reading, String>)> {
    let dim = document.feature_as::<Dimension>(id)?;
    let elements: Option<Vec<Element>> = dim
        .elements
        .iter()
        .map(|e| element_at(document, *e))
        .collect();
    let reading = match elements {
        Some(elements) => dimension::read(dim.kind, &elements),
        None => Err("an element it was measured between is gone".to_string()),
    };
    Some((dim, reading))
}

/// The elements a dimension tool would take: the tree's selection.
fn selected_elements(ctx: &WorkbenchRuntimeContext) -> Vec<core_document::FeatureId> {
    let ids: Vec<_> = if ctx.selected_features.is_empty() {
        ctx.active_document_object.into_iter().collect()
    } else {
        ctx.selected_features.clone()
    };
    ids.into_iter()
        .filter(|id| element_at(ctx.document, *id).is_some())
        .collect()
}

/// The primitive `id` stands for, as `design.primitive` takes it:
/// `{kind, placement}` in the document's frame.
fn element_solid(
    document: &Document,
    id: core_document::FeatureId,
    thickness: f64,
) -> Result<Value, String> {
    let element = element_at(document, id).ok_or("not an element of this document")?;
    let rows = document.body_placement(element.scan).rows();
    let (kind, placement) = extract::primitive(&element, &rows, thickness)?;
    Ok(json!({"kind": kind, "placement": placement, "name": element.name}))
}

/// Measure `kind` between `elements` and keep it as a dimension.
fn add_dimension(
    ctx: &mut WorkbenchRuntimeContext,
    kind: DimKind,
    elements: Vec<core_document::FeatureId>,
    band: Option<(f64, f64, f64)>,
) -> Result<(core_document::FeatureId, Value), String> {
    let found: Vec<Element> = elements
        .iter()
        .map(|id| element_at(ctx.document, *id).ok_or(format!("{} is not an element", id.0)))
        .collect::<Result<_, _>>()?;
    if found.iter().any(|e| e.scan != found[0].scan) {
        return Err("the elements are on different scans".into());
    }
    let reading = dimension::read(kind, &found)?;
    let scan = found[0].scan;
    let name = numbered(ctx.document, kind.word());
    let (nominal, plus, minus) =
        band.unwrap_or((dimension::design_value(kind, reading.value), 0.0, 0.0));
    let dim = Dimension {
        name: name.clone(),
        scan,
        kind,
        elements,
        nominal,
        plus,
        minus,
    };
    let id = ctx
        .document
        .add_feature_in_body(dim.clone(), name, Some(scan))
        .map_err(|e| e.to_string())?;
    Ok((id, dimension_answer(id, &dim, &Ok(reading))))
}

fn dimension_answer(
    id: core_document::FeatureId,
    dim: &Dimension,
    reading: &Result<dimension::Reading, String>,
) -> Value {
    let mut out = json!({
        "dimension": id.0.to_string(),
        "name": dim.name,
        "kind": dim.kind.word().to_lowercase(),
        "elements": dim.elements.iter().map(|e| e.0.to_string()).collect::<Vec<_>>(),
        "nominal": dim.nominal,
        "plus": dim.plus,
        "minus": dim.minus,
    });
    match reading {
        Ok(r) => {
            out["value"] = json!(r.value);
            out["deviation"] = json!(r.value - dim.nominal);
            if let Some(d) = r.deltas {
                out["deltas"] = json!({"x": d[0], "y": d[1], "z": d[2]});
            }
            if let Some((lo, hi)) = r.range {
                out["range"] = json!({"min": lo, "max": hi});
            }
            if let Some(pass) = dim.passes(r.value) {
                out["pass"] = json!(pass);
            }
        }
        Err(why) => out["error"] = json!(why),
    }
    out
}

/// Move labels down until none covers another, in the order given.
fn spread(labels: &mut [core_document::ScreenSpaceLabel]) {
    let half = |l: &core_document::ScreenSpaceLabel| {
        (
            l.text.chars().count() as f32 * l.size * 0.32 + 6.0,
            l.size * 0.9 + 3.0,
        )
    };
    for i in 1..labels.len() {
        let (w, h) = half(&labels[i]);
        for _ in 0..labels.len() {
            let hit = labels[..i].iter().find(|o| {
                let (ow, oh) = half(o);
                (o.pos[0] - labels[i].pos[0]).abs() < w + ow - 0.5
                    && (o.pos[1] - labels[i].pos[1]).abs() < h + oh - 0.5
            });
            match hit {
                Some(o) => labels[i].pos[1] = o.pos[1] + half(o).1 + h,
                None => break,
            }
        }
    }
}

/// The feature ids listed under `name`.
fn ids_in(
    a: &core_document::Args,
    name: &str,
) -> Result<Vec<core_document::FeatureId>, CommandError> {
    a.0.get(name)
        .and_then(Value::as_array)
        .ok_or_else(|| CommandError::bad(name, "must be a list of ids"))?
        .iter()
        .map(|v| {
            v.as_str()
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .map(core_document::FeatureId)
                .ok_or_else(|| CommandError::bad(name, "must be a list of ids"))
        })
        .collect()
}

impl InspectionWorkbench {
    /// Apply the running best fit once its answer is in, while the scan
    /// and reference are still as it fitted them; a stale answer is
    /// dropped with a word in the log.
    fn land_best_fit(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let (job, answer) = match self.tab.fitting.landed(ctx.document) {
            None => return,
            Some(best_fit::Landed::Stale) => {
                ctx.log_warn("Best fit dropped: the scan or its reference changed while it ran");
                return;
            }
            Some(best_fit::Landed::Fresh(landed)) => *landed,
        };
        let (after, fit) = match answer {
            Ok(fitted) => fitted,
            Err(why) => return ctx.log_warn(format!("Best fit: {why}")),
        };
        let how = best_fit_how(job.reference, job.search, &fit);
        match record_alignment(ctx, job.scan, how, after) {
            Ok(answer) => {
                let mut args = CommandArgs::new();
                args.insert("scan".into(), json!(job.scan.0.to_string()));
                args.insert("reference".into(), json!(job.reference.0.to_string()));
                ctx.record(BEST_FIT, args, answer);
            }
            Err(why) => ctx.log_warn(why),
        }
    }

    /// The open tolerance task's panel, as widgets: its settings, then
    /// what it reads.
    pub(crate) fn task_widgets(&self, ctx: &WorkbenchRuntimeContext) -> Vec<bench_api::Widget> {
        let Some((id, mut t)) = self.tolerance_in_task(ctx) else {
            return Vec::new();
        };
        let mut widgets = tolerance_task::walk(ctx.document, &mut t, None);
        let text = |text: String| bench_api::Widget::Text { text, mono: false };
        match self
            .tab
            .tolerance_readings
            .read(ctx.document, id, &t, ctx.kernel, false)
        {
            Ok(e) => {
                widgets.push(text(t.label(&e, ctx.document.display_unit())));
                widgets.extend(e.note.clone().map(text));
            }
            Err(why) => widgets.push(text(why)),
        }
        vec![
            bench_api::Stack {
                children: widgets,
                tight: true,
                ..Default::default()
            }
            .into(),
        ]
    }

    /// A change to the open tolerance task's panel, written to the
    /// tolerance at once.
    pub(crate) fn task_event(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        event: &bench_api::PanelEvent,
    ) {
        let Some((id, mut t)) = self.tolerance_in_task(ctx) else {
            return;
        };
        let before = t.clone();
        tolerance_task::walk(ctx.document, &mut t, Some(event));
        if t != before {
            let _ = ctx.document.update_feature_data(id, t.to_json());
            ctx.document.set_feature_dependencies(id, t.dependencies());
        }
    }

    /// The tolerance the open task edits, as the document holds it.
    fn tolerance_in_task(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> Option<(core_document::FeatureId, tolerance::Tolerance)> {
        let id = self.tab.tolerance_task.as_ref()?.id;
        let data = ctx.document.get_feature_data(id)?;
        tolerance::Tolerance::from_json(data).ok().map(|t| (id, t))
    }

    /// Every element, constructed element, section cut and shadow outline
    /// (of `only` when given) as construction geometry where it stands.
    fn construction_items(
        &self,
        document: &Document,
        only: Option<BodyId>,
    ) -> Vec<kernel_api::ConstructionItem> {
        let mut nodes: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.visible)
            .collect();
        nodes.sort_by_key(|(id, n)| (n.seq, **id));
        let mut out = Vec::new();
        let mut push = |rows, name: String, shape| {
            out.push(kernel_api::ConstructionItem {
                name,
                shape: placed_shape(rows, shape),
            })
        };
        for (id, node) in nodes {
            let kind = node.workbench_id.as_str();
            if kind == element::KIND || kind == construction::KIND {
                let Some(e) = element_at(document, *id) else {
                    continue;
                };
                if only.is_some_and(|b| b != e.scan) {
                    continue;
                }
                let rows = document.body_placement(e.scan).rows();
                for (name, shape) in construction_of(&e) {
                    push(rows, name, shape);
                }
            } else if kind == section::KIND {
                let Some((s, cuts, _)) = self.cuts_of(document, *id) else {
                    continue;
                };
                if only.is_some_and(|b| b != s.scan) {
                    continue;
                }
                let rows = document.body_placement(s.scan).rows();
                for c in cuts.iter() {
                    for (k, chain) in c.chains.iter().enumerate() {
                        push(
                            rows,
                            format!("{} {:+.2} #{}", node.name, c.offset, k + 1),
                            kernel_api::ConstructionShape::Polyline {
                                points: chain.points.clone(),
                                closed: chain.closed,
                            },
                        );
                    }
                }
            } else if kind == shadow::KIND {
                let Some(s) = document.feature_as::<shadow::Shadow>(*id) else {
                    continue;
                };
                let Some(made) = s.made.as_ref().filter(|_| only.is_none_or(|b| b == s.scan))
                else {
                    continue;
                };
                let p = made.plane;
                for (k, wire) in made.outline.iter().enumerate() {
                    let points = shadow::samples(wire)
                        .map(|at| shadow::on_plane(&p, at))
                        .collect();
                    // The outline is in the document's frame already.
                    push(
                        core_document::BodyPlacement::default().rows(),
                        format!("{} #{}", node.name, k + 1),
                        kernel_api::ConstructionShape::Polyline {
                            points,
                            closed: true,
                        },
                    );
                }
            }
        }
        out
    }

    /// A section's cuts as the scan stands, cut now if they are not kept.
    fn cuts_of(
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

    /// The map shown on `scan`, as `scalar_maps` picks it (comparisons,
    /// then maps against elements, then wall thickness, the latest of each
    /// first), with its values when they are measured.
    fn shown_map(
        &self,
        document: &Document,
        scan: BodyId,
    ) -> Option<(core_document::FeatureId, std::sync::Arc<[f32]>)> {
        self.tab
            .maps
            .shown::<Comparison>(document, scan)
            .or_else(|| {
                self.tab
                    .maps
                    .shown::<element_map::ElementMap>(document, scan)
            })
            .or_else(|| {
                self.tab
                    .maps
                    .shown::<thickness::WallThickness>(document, scan)
            })
            .flatten()
    }

    /// The shown map's value under the cursor, and where, in the scan's
    /// frame.
    fn hovered_reading(&self, ctx: &WorkbenchRuntimeContext) -> Option<(BodyId, f64, [f64; 3])> {
        let scan = ctx.hovered_body_id.map(BodyId)?;
        let face = ctx.hovered_face?;
        let (_, values) = self.shown_map(ctx.document, scan)?;
        let (mesh, _) = ctx.document.local_geometry(scan)?;
        let at = ctx
            .document
            .body_placement(scan)
            .inverse()
            .point(face.point)
            .map(f64::from);
        Some((scan, pins::read(&mesh, &values, at, READING_REACH)?, at))
    }

    /// A click or a key while the alignment by points is picking; `None`
    /// leaves the event to the rest.
    fn align_input(
        &mut self,
        event: &WorkbenchInputEvent,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> Option<InputResult> {
        match event {
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                ..
            } => {
                let body = BodyId(ctx.hovered_body_id?);
                let at = ctx
                    .document
                    .body_placement(body)
                    .inverse()
                    .point(ctx.hovered_face.as_ref()?.point)
                    .map(f64::from);
                let scan = ctx.document.body_scan(body).is_some();
                self.tab.picks.pick_pair_point(body, scan, at);
                Some(InputResult::consumed())
            }
            WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Enter,
            } => {
                let Some(args) = self.tab.picks.align_args(ctx.document) else {
                    ctx.log_warn(format!(
                        "align by points takes {} pairs on one scan; {} so far",
                        picks::PAIRS_NEEDED,
                        self.tab.picks.pairs()
                    ));
                    return Some(InputResult::consumed());
                };
                match align_commands::run(align_commands::POINTS, &args, ctx) {
                    Ok(answer) => {
                        ctx.record(align_commands::POINTS, args, answer);
                        self.tab.picks.clear();
                    }
                    Err(why) => ctx.log_warn(why.to_string()),
                }
                Some(InputResult::consumed())
            }
            _ => None,
        }
    }

    fn armed_status(&self, _ctx: &WorkbenchRuntimeContext) -> Option<core_document::StatusItems> {
        let kind = self.tab.armed?;
        Some(core_document::StatusItems {
            mode: Some(format!(
                "Click a scan {} to fit a {} there · Shift-click pins one more seed · Esc to stop",
                if matches!(kind, FitKind::Circle | FitKind::Line) {
                    "by a sharp edge"
                } else {
                    "on a surface"
                },
                element::kind_word(kind).to_lowercase()
            )),
            ..Default::default()
        })
    }
}

impl core_document::Workbench for InspectionWorkbench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(
            ID,
            "Inspection",
            "Measure a scan: fit shapes to its surface and read their size and form",
        )
        .icon("measure")
        .feature_kinds([
            shadow::KIND,
            section::KIND,
            pins::KIND,
            element_map::KIND,
            tolerance::KIND,
            thickness::KIND,
            overhang::KIND,
            clearance::KIND,
            element::KIND,
            dimension::KIND,
            construction::KIND,
            compare::KIND,
            align::KIND,
        ])
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        align_commands::configure(context);
        for kind in FitKind::ALL {
            context.register_tool(
                ToolDescriptor::new_action(
                    tool_id(kind),
                    format!("Fit {}", element::kind_word(kind).to_lowercase()),
                    Some("fit"),
                )
                .icon(element::icon(kind)),
            );
        }
        context.register_tool(
            ToolDescriptor::new_action(BEST_FIT, "Best fit to reference", Some("align"))
                .icon("joint-align"),
        );
        context.register_tool(
            ToolDescriptor::new_action(RESET_ALIGNMENT, "Reset alignment", Some("align"))
                .icon("home"),
        );
        context.register_tool(
            ToolDescriptor::new_action(SECTION, "Section", Some("section")).icon("clipping-plane"),
        );
        context.register_tool(
            ToolDescriptor::new_action(SHADOW, "Shadow outline", Some("section")).icon("cut"),
        );
        context.register_tool(
            ToolDescriptor::new_action(PIN, "Pin reading", Some("compare")).icon("datum-point"),
        );
        context.register_tool(
            ToolDescriptor::new_action(ELEMENT_MAP, "Map against element", Some("compare"))
                .icon("constraint-distance"),
        );
        context.register_tool(
            ToolDescriptor::new_action(SAVE_REPORT, "Save report…", Some("report")).icon("save"),
        );
        context.register_tool(
            ToolDescriptor::new_action(SAVE_STEP, "Save elements as STEP…", Some("report"))
                .icon("save"),
        );
        context.register_tool(
            ToolDescriptor::new_action(SAVE_IGES, "Save elements as IGES…", Some("report"))
                .icon("save"),
        );
        context.register_tool(
            ToolDescriptor::new_action(TOLERANCE, "Tolerance", Some("dimension"))
                .icon("constraint-perpendicular"),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                THICKNESS,
                "Wall thickness",
                Some(core_document::INSPECT_CATEGORY),
            )
            .icon("thickness"),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                EXTRACT,
                "Element as body",
                Some(core_document::INSPECT_CATEGORY),
            )
            .icon("body"),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                CLEARANCE,
                "Fit check",
                Some(core_document::INSPECT_CATEGORY),
            )
            .icon("thickness"),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                OVERHANG,
                "Overhangs",
                Some(core_document::INSPECT_CATEGORY),
            )
            .icon("thickness"),
        );
        context.register_tool(
            ToolDescriptor::new_action(COMPARE_TOOL, "Compare with reference", Some("compare"))
                .icon("measure"),
        );
        for made in Made::ALL {
            context.register_tool(
                ToolDescriptor::new_action(
                    construct_tool_id(made),
                    format!("Construct {}", made.word().to_lowercase()),
                    Some("construct"),
                )
                .icon(made.icon()),
            );
        }
        for kind in DimKind::ALL {
            context.register_tool(
                ToolDescriptor::new_action(dim_tool_id(kind), kind.word(), Some("dimension"))
                    .icon(kind.icon()),
            );
        }
        commands::configure(context);
    }

    fn feature_info(&self, node: &FeatureNode) -> FeatureInfo {
        if node.workbench_id.as_str() == shadow::KIND {
            return FeatureInfo {
                icon: "cut",
                kind_label: "Shadow outline".to_string(),
                family_label: "Inspection shadow".to_string(),
                builds_solid: true,
            };
        }
        if node.workbench_id.as_str() == section::KIND {
            return FeatureInfo {
                icon: "clipping-plane",
                kind_label: "Section".to_string(),
                family_label: "Inspection section".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == pins::KIND {
            return FeatureInfo {
                icon: "datum-point",
                kind_label: "Pinned reading".to_string(),
                family_label: "Inspection pin".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == element_map::KIND {
            return FeatureInfo {
                icon: "constraint-distance",
                kind_label: "Map against an element".to_string(),
                family_label: "Inspection map".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == tolerance::KIND {
            let label = tolerance::Tolerance::from_json(&node.data)
                .map(|t| format!("{} tolerance", t.characteristic.word()))
                .unwrap_or_else(|_| "Tolerance".to_string());
            return FeatureInfo {
                icon: "constraint-perpendicular",
                kind_label: label,
                family_label: "Inspection tolerance".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == thickness::KIND {
            return FeatureInfo {
                icon: "thickness",
                kind_label: "Wall thickness".to_string(),
                family_label: "Inspection map".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == clearance::KIND {
            return FeatureInfo {
                icon: "thickness",
                kind_label: "Fit check".to_string(),
                family_label: "Inspection map".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == overhang::KIND {
            return FeatureInfo {
                icon: "thickness",
                kind_label: "Overhangs".to_string(),
                family_label: "Inspection map".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == align::KIND {
            let label = align::Alignment::from_json(&node.data)
                .map(|a| format!("{} · {}", a.how.word(), a.how.summary()))
                .unwrap_or_else(|_| "Alignment".to_string());
            return FeatureInfo {
                icon: "move-geometry",
                kind_label: label,
                family_label: "Inspection alignment".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == compare::KIND {
            return FeatureInfo {
                icon: "measure",
                kind_label: "Comparison with a reference".to_string(),
                family_label: "Inspection map".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == construction::KIND {
            let Ok(c) = Construction::from_json(&node.data) else {
                return FeatureInfo::fallback(node);
            };
            return FeatureInfo {
                icon: c.made.icon(),
                kind_label: format!("Constructed {}", c.made.word().to_lowercase()),
                family_label: "Inspection element".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == dimension::KIND {
            let Ok(dim) = Dimension::from_json(&node.data) else {
                return FeatureInfo::fallback(node);
            };
            return FeatureInfo {
                icon: dim.kind.icon(),
                kind_label: format!("{} dimension", dim.kind.word()),
                family_label: "Inspection dimension".to_string(),
                builds_solid: false,
            };
        }
        let Ok(element) = Element::from_json(&node.data) else {
            return FeatureInfo::fallback(node);
        };
        FeatureInfo {
            icon: element::icon(element.kind),
            kind_label: format!(
                "{} · {}",
                element::kind_word(element.kind),
                element.result.summary()
            ),
            family_label: "Inspection element".to_string(),
            builds_solid: false,
        }
    }

    fn passive_geometry(
        &self,
        document: &Document,
        id: core_document::FeatureId,
        node: &FeatureNode,
    ) -> Option<PassiveGeometry> {
        if node.workbench_id.as_str() == shadow::KIND {
            let s = document.feature_as::<shadow::Shadow>(id)?;
            let made = s.made?;
            return Some(PassiveGeometry {
                mesh: shadow::lines(&made),
                revision: made.key,
                tint: PassiveTint::External,
                regions: Vec::new(),
            });
        }
        if node.workbench_id.as_str() == section::KIND {
            let (_, cuts, key) = self.cuts_of(document, id)?;
            return Some(PassiveGeometry {
                mesh: section::lines(&cuts),
                revision: key,
                tint: PassiveTint::External,
                regions: Vec::new(),
            });
        }
        let data = document.feature_values(id)?;
        if node.workbench_id.as_str() == dimension::KIND {
            let (_, reading) = dimension_at(document, id)?;
            let reading = reading.ok()?;
            return Some(PassiveGeometry {
                mesh: dimension::line(&reading),
                revision: core_document::data_revision(data)
                    ^ reading.value.to_bits()
                    ^ reading.from[0].to_bits(),
                tint: PassiveTint::External,
                regions: Vec::new(),
            });
        }
        if node.workbench_id.as_str() == construction::KIND {
            let built = element_at(document, id)?;
            return Some(PassiveGeometry {
                mesh: built.result.lines(),
                revision: core_document::data_revision(data) ^ shape_revision(&built),
                tint: PassiveTint::External,
                regions: Vec::new(),
            });
        }
        let element = Element::from_json(data).ok()?;
        Some(PassiveGeometry {
            mesh: element.result.lines(),
            revision: core_document::data_revision(data),
            tint: PassiveTint::External,
            regions: Vec::new(),
        })
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        if tool_id == align_commands::POINTS {
            let bodies = ctx.document.bodies();
            return bodies.len() > 1
                && bodies
                    .iter()
                    .any(|b| ctx.document.body_scan(b.id).is_some());
        }
        if align_commands::owns(tool_id) {
            return align_commands::selected_args(tool_id, ctx).is_some();
        }
        if let Some(kind) = dim_kind_of_tool(tool_id) {
            return selected_elements(ctx).len() == kind.operands();
        }
        if tool_id == COMPARE_TOOL {
            return compare_pair(ctx).is_some();
        }
        if tool_id == PIN {
            return ctx
                .document
                .bodies()
                .iter()
                .any(|b| self.shown_map(ctx.document, b.id).is_some());
        }
        if tool_id == ELEMENT_MAP {
            return selected_elements(ctx).len() == 1;
        }
        if tool_id == SHADOW {
            return match selected_elements(ctx).as_slice() {
                [one] => element_at(ctx.document, *one).is_some_and(|e| {
                    !matches!(e.result.shape, kernel_api::FittedShape::Sphere { .. })
                }),
                [] => ctx
                    .selected_body_id
                    .is_some_and(|b| ctx.document.body_scan(BodyId(b)).is_some()),
                _ => false,
            };
        }
        if tool_id == SECTION {
            return match selected_elements(ctx).as_slice() {
                [one] => element_at(ctx.document, *one).is_some_and(|e| {
                    !matches!(e.result.shape, kernel_api::FittedShape::Sphere { .. })
                }),
                [] => ctx
                    .selected_body_id
                    .is_some_and(|b| ctx.document.body_scan(BodyId(b)).is_some()),
                _ => false,
            };
        }
        if matches!(tool_id, SAVE_STEP | SAVE_IGES) {
            return !self.construction_items(ctx.document, None).is_empty();
        }
        if tool_id == SAVE_REPORT {
            return ctx
                .document
                .feature_tree()
                .all_nodes()
                .any(|(_, n)| n.workbench_id.as_str() == element::KIND);
        }
        if tool_id == TOLERANCE {
            let picked: Vec<Element> = selected_elements(ctx)
                .into_iter()
                .filter_map(|id| element_at(ctx.document, id))
                .collect();
            return characteristic_for(&picked).is_some();
        }
        if tool_id == CLEARANCE {
            return compare_pair(ctx).is_some();
        }
        if tool_id == EXTRACT {
            let picked = selected_elements(ctx);
            return picked.len() == 1 && element_solid(ctx.document, picked[0], 2.0).is_ok();
        }
        if tool_id == THICKNESS || tool_id == OVERHANG {
            return thickness_body(ctx).is_some();
        }
        if tool_id == BEST_FIT {
            // While a fit runs, a click stops it.
            return self.tab.fitting.running() || compare_pair(ctx).is_some();
        }
        if tool_id == RESET_ALIGNMENT {
            return compare_pair(ctx)
                .map(|(scan, _)| scan)
                .or_else(|| ctx.selected_body_id.map(BodyId))
                .is_some_and(|b| {
                    ctx.document.body_scan(b).is_some()
                        && !ctx.document.body_placement(b).is_identity()
                });
        }
        if let Some(made) = made_of_tool(tool_id) {
            let picked: Vec<Element> = selected_elements(ctx)
                .into_iter()
                .filter_map(|id| element_at(ctx.document, id))
                .collect();
            let trial = Construction {
                name: String::new(),
                scan: BodyId(uuid::Uuid::nil()),
                made,
                from: Vec::new(),
                offset: 0.0,
            };
            return construction::build(&trial, &picked).is_ok();
        }
        match kind_of_tool(tool_id) {
            Some(_) => {
                ctx.current_marking().is_some()
                    || ctx
                        .document
                        .bodies()
                        .iter()
                        .any(|b| ctx.document.body_scan(b.id).is_some())
            }
            None => true,
        }
    }

    fn tool_toggled(&self, tool_id: &str) -> bool {
        (tool_id == PIN && self.tab.pinning)
            || (tool_id == BEST_FIT && self.tab.fitting.running())
            || (tool_id == align_commands::POINTS && self.tab.picks.aligning)
            || (self.tab.armed.is_some() && kind_of_tool(tool_id) == self.tab.armed)
    }

    fn parameters(&self, node: &FeatureNode) -> Vec<core_document::evaluate::Parameter> {
        use core_document::{evaluate::Parameter, expr::Dim};
        if node.workbench_id.as_str() == construction::KIND {
            return match Construction::from_json(&node.data) {
                Ok(c) if c.made == Made::Plane && c.from.len() == 1 => {
                    vec![Parameter::new("offset", "Offset", Dim::LENGTH, "/offset")]
                }
                _ => Vec::new(),
            };
        }
        if node.workbench_id.as_str() == tolerance::KIND {
            return vec![Parameter::new("limit", "Limit", Dim::LENGTH, "/limit")];
        }
        if node.workbench_id.as_str() == clearance::KIND {
            return vec![
                Parameter::new("min_gap", "Smallest gap", Dim::LENGTH, "/min_gap"),
                Parameter::new("reach", "Reach", Dim::LENGTH, "/reach"),
            ];
        }
        if node.workbench_id.as_str() == overhang::KIND {
            return vec![Parameter::new("limit", "Limit", Dim::ANGLE, "/limit_deg")];
        }
        if node.workbench_id.as_str() == shadow::KIND {
            return vec![
                Parameter::new("play", "Play", Dim::LENGTH, "/play"),
                Parameter::new("tolerance", "Tolerance", Dim::LENGTH, "/tolerance"),
                Parameter::new("round_below", "Round below", Dim::ANGLE, "/round_below_deg"),
                Parameter::new("fill_below", "Fill below", Dim::LENGTH, "/fill_below"),
            ];
        }
        if node.workbench_id.as_str() == section::KIND {
            return vec![
                Parameter::new("offset", "Offset", Dim::LENGTH, "/offset"),
                Parameter::count("count", "Cuts", "/count"),
                Parameter::new("step", "Step", Dim::LENGTH, "/step"),
            ];
        }
        if node.workbench_id.as_str() != dimension::KIND {
            return Vec::new();
        }
        let Ok(dim) = Dimension::from_json(&node.data) else {
            return Vec::new();
        };
        let unit = match dim.kind {
            DimKind::Angle => Dim::ANGLE,
            _ => Dim::LENGTH,
        };
        vec![
            Parameter::new("nominal", "Nominal", unit, "/nominal"),
            Parameter::new("plus", "Plus", unit, "/plus"),
            Parameter::new("minus", "Minus", unit, "/minus"),
        ]
    }

    fn delete_feature(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        id: core_document::FeatureId,
    ) -> bool {
        // Everything made or measured from it goes with it, however deep.
        let mut going = vec![id];
        loop {
            let more: Vec<_> =
                ctx.document
                    .feature_tree()
                    .all_nodes()
                    .filter(|(f, _)| !going.contains(f))
                    .filter(|(_, n)| {
                        let on = match n.workbench_id.as_str() {
                            dimension::KIND => Dimension::from_json(&n.data).map(|d| d.elements),
                            pins::KIND => pins::Pin::from_json(&n.data).map(|p| p.dependencies()),
                            element_map::KIND => element_map::ElementMap::from_json(&n.data)
                                .map(|m| m.dependencies()),
                            tolerance::KIND => {
                                tolerance::Tolerance::from_json(&n.data).map(|t| t.dependencies())
                            }
                            construction::KIND => Construction::from_json(&n.data).map(|c| c.from),
                            _ => return false,
                        };
                        on.is_ok_and(|on| on.iter().any(|f| going.contains(f)))
                    })
                    .map(|(f, _)| *f)
                    .collect();
            if more.is_empty() {
                break;
            }
            going.extend(more);
        }
        for f in going.iter().skip(1).rev() {
            let _ = ctx.document.remove_feature(*f);
        }
        ctx.document.remove_feature(id).is_ok()
    }

    fn edit_feature(&mut self, ctx: &mut WorkbenchRuntimeContext, id: core_document::FeatureId) {
        if let Some(node) = ctx.document.feature_tree().get_node(id)
            && node.workbench_id.as_str() == tolerance::KIND
            && let Ok(t) = tolerance::Tolerance::from_json(&node.data)
        {
            self.tab.tolerance_task = Some(tolerance_task::Task {
                id,
                opening: Some(t),
            });
        }
    }

    fn editing_feature(&self) -> Option<core_document::FeatureId> {
        self.tab.tolerance_task.as_ref().map(|t| t.id)
    }

    fn tab_state(&mut self) -> Option<&mut dyn core_document::TabSlot> {
        Some(&mut self.tab)
    }

    fn task(&self, _ctx: &WorkbenchRuntimeContext) -> Option<core_document::TaskInfo> {
        self.tab.tolerance_task.as_ref()?;
        Some(core_document::TaskInfo {
            title: "Geometric tolerance".into(),
            icon: "constraint-perpendicular",
            confirmable: true,
            stepwise: false,
            ..Default::default()
        })
    }

    #[cfg(feature = "egui")]
    fn ui_task_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: core_document::TaskRequest,
    ) -> core_document::TaskOutcome {
        use core_document::TaskOutcome;
        let Some(task) = self.tab.tolerance_task.as_ref() else {
            return TaskOutcome::Open;
        };
        let id = task.id;
        let Some(data) = ctx.document.get_feature_data(id) else {
            self.tab.tolerance_task = None;
            return TaskOutcome::Cancelled;
        };
        // Cancel's edits are taken back by the host.
        if request.cancel {
            self.tab.tolerance_task = None;
            return TaskOutcome::Cancelled;
        }
        if tolerance::Tolerance::from_json(data).is_err() {
            return TaskOutcome::Open;
        }
        let widgets = self.task_widgets(ctx);
        let out = core_document::panel::show(
            ui,
            egui::Id::new("tolerance_task"),
            &widgets,
            Some(&*ctx.document),
        );
        let by_formula = out.write_formulas(ctx);
        for event in out.events_besides(&by_formula) {
            self.task_event(ctx, event);
        }
        if request.accept
            && let Some((id, t)) = self.tolerance_in_task(ctx)
            && let Some(task) = &self.tab.tolerance_task
        {
            let reading = self
                .tab
                .tolerance_readings
                .read(ctx.document, id, &t, ctx.kernel, false);
            if reading.is_ok() {
                let args = tolerance_task::args(&t, task.opening.as_ref().map(|_| id));
                ctx.record(TOLERANCE, args, tolerance_answer(id, &t, &reading));
                self.tab.tolerance_task = None;
                return TaskOutcome::Accepted {
                    label: "Geometric tolerance".into(),
                };
            }
        }
        TaskOutcome::Open
    }

    fn busy(&self) -> bool {
        self.tab.maps.busy() || self.tab.tolerance_readings.busy() || self.tab.fitting.running()
    }

    fn on_frame(&mut self, _dt: f32, ctx: &mut WorkbenchRuntimeContext) {
        if let Some(kernel) = ctx.kernel {
            let _ = self.kernel.set(kernel);
        }
        self.land_best_fit(ctx);
    }

    fn on_background_frame(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.land_best_fit(ctx);
    }

    fn viewport(&self, ctx: &WorkbenchRuntimeContext) -> core_document::ViewportOutput {
        core_document::ViewportOutput {
            marks: self.screen_marks(ctx, ctx.active_document_object),
            labels: self.screen_labels(ctx, ctx.active_document_object),
            scalar_maps: self.scalar_maps(ctx),
            status: self.status_items(ctx),
            faded: self.faded_bodies(ctx),
            ..Default::default()
        }
    }

    /// A shown comparison's reference draws see-through, so the scan's map
    /// shows where the reference would cover it.
    /// A shadow's outline follows the scan's mesh, where it stands and the
    /// shadow's settings.
    fn derive_on_geometry(
        &self,
        node: &FeatureNode,
        values: &mut Value,
        document: &Document,
        kernel: Option<&dyn kernel_api::KernelQueries>,
    ) -> bool {
        if node.workbench_id.as_str() != shadow::KIND {
            return false;
        }
        let (Some(kernel), Ok(s)) = (kernel, shadow::Shadow::from_json(values)) else {
            return false;
        };
        if s.made
            .as_ref()
            .is_some_and(|m| m.key == shadow::key(document, &s))
        {
            return false;
        }
        let made = shadow::make(document, kernel, &s);
        values["made"] = serde_json::to_value(made).unwrap_or_default();
        true
    }

    fn rebuild_jobs(&self, document: &mut Document) -> Vec<core_document::RebuildJob> {
        let dirty: Vec<BodyId> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == shadow::KIND && n.dirty)
            .filter_map(|(_, n)| n.body)
            .collect();
        let mut jobs = Vec::new();
        for body in dirty {
            if jobs
                .iter()
                .any(|j: &core_document::RebuildJob| j.body == body)
            {
                continue;
            }
            let mut plan = core_document::BuildPlan::default();
            for (id, s) in shadows_on(document, body) {
                document.clear_feature_dirty(id);
                let ops = shadow::ops(&s);
                plan.op_features.extend(ops.iter().map(|_| id));
                plan.ops.extend(ops);
            }
            jobs.push(core_document::RebuildJob {
                body,
                plan: Ok(plan),
            });
        }
        jobs
    }

    fn invalidate_body(&self, document: &mut Document, body: BodyId) {
        for (id, _) in shadows_on(document, body) {
            document.mark_feature_dirty(id);
        }
    }

    fn invalidate_all(&self, document: &mut Document) {
        let all: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == shadow::KIND)
            .map(|(id, _)| *id)
            .collect();
        for id in all {
            document.mark_feature_dirty(id);
        }
    }

    /// Every element, constructed element, dimension and tolerance as one
    /// table, the tally above it, the scans they are on pictured.
    fn semantic_pmi(&self, document: &Document, body: BodyId) -> core_document::SemanticPmi {
        pmi::of_body(document, body)
    }

    fn report(&self, document: &Document) -> Option<core_document::BenchReport> {
        let report = report_of(
            document,
            None,
            &self.tab.tolerance_readings,
            self.kernel.get().copied(),
            true,
        );
        let rows = report::rows(&report);
        if rows.is_empty() {
            return None;
        }
        let mut bodies: Vec<BodyId> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == element::KIND)
            .filter_map(|(_, n)| n.body)
            .collect();
        bodies.sort();
        bodies.dedup();
        Some(core_document::BenchReport {
            title: "Inspection".to_string(),
            summary: vec![format!(
                "{} passed, {} failed",
                report["passed"], report["failed"]
            )],
            tables: vec![core_document::ReportTable {
                heading: "Elements, dimensions and tolerances".to_string(),
                columns: [
                    "Section",
                    "Name",
                    "What",
                    "Value",
                    "Target",
                    "Deviation",
                    "Result",
                    "Note",
                ]
                .map(String::from)
                .to_vec(),
                rows: rows.into_iter().map(|r| r.to_vec()).collect(),
            }],
            bodies,
        })
    }

    fn feature_solid(
        &self,
        document: &Document,
        id: core_document::FeatureId,
    ) -> Option<kernel_api::MeshOperand> {
        let e = element_at(document, id)?;
        e.result.solid(document.body_placement(e.scan).rows())
    }

    /// A section offers its first cut's plane, a fitted or constructed
    /// plane its own, where the scan stands.
    fn feature_plane(
        &self,
        document: &Document,
        id: core_document::FeatureId,
        node: &FeatureNode,
    ) -> Option<kernel_api::ProfilePlane> {
        let (scan, origin, normal) = if node.workbench_id.as_str() == section::KIND {
            let s = document.feature_as::<section::Section>(id)?;
            let n = DVec3::from(s.normal);
            vec::unit(n)?;
            let shift = n * (s.offset / n.length());
            (s.scan, (DVec3::from(s.origin) + shift).to_array(), s.normal)
        } else {
            let e = element_at(document, id)?;
            match e.result.shape {
                kernel_api::FittedShape::Plane { origin, normal } => (e.scan, origin, normal),
                _ => return None,
            }
        };
        let rows = document.body_placement(scan).rows();
        let place = |p: [f64; 3], w: f64| -> [f64; 3] {
            std::array::from_fn(|r| {
                rows[r][0] * p[0] + rows[r][1] * p[1] + rows[r][2] * p[2] + w * rows[r][3]
            })
        };
        // A plane with no normal offers nothing to sketch on.
        let n = vec::unit(normal.into())?;
        let (x, y) = vec::basis(n)?;
        Some(kernel_api::ProfilePlane {
            origin: place(origin, 1.0),
            normal: place(n.to_array(), 0.0),
            x_axis: place(x.to_array(), 0.0),
            y_axis: place(y.to_array(), 0.0),
        })
    }

    fn on_input(
        &mut self,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        if let (WorkbenchInputEvent::ToolActivated, Some(id @ (SECTION | SHADOW))) =
            (event, active_tool)
        {
            let mut args = CommandArgs::new();
            match (selected_elements(ctx).first(), ctx.selected_body_id) {
                (Some(element), _) => {
                    args.insert("element".into(), json!(element.0.to_string()));
                }
                (None, Some(scan)) => {
                    args.insert("scan".into(), json!(scan.to_string()));
                }
                (None, None) => return InputResult::consumed(),
            }
            match self.run_command(id, &args, ctx) {
                Ok(answer) => ctx.record(id, args, answer),
                Err(why) => ctx.log_warn(why.to_string()),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(ELEMENT_MAP)) = (event, active_tool) {
            let Some(&element) = selected_elements(ctx).first() else {
                return InputResult::consumed();
            };
            let mut args = CommandArgs::new();
            args.insert("element".into(), json!(element.0.to_string()));
            match self.run_command(ELEMENT_MAP, &args, ctx) {
                Ok(answer) => ctx.record(ELEMENT_MAP, args, answer),
                Err(why) => ctx.log_warn(why.to_string()),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(id @ (SAVE_STEP | SAVE_IGES))) =
            (event, active_tool)
        {
            let iges = id == SAVE_IGES;
            let (kind, extension) = if iges {
                ("IGES", "igs")
            } else {
                ("STEP", "step")
            };
            let items = self.construction_items(ctx.document, None);
            let written = ctx
                .kernel
                .ok_or_else(|| "no kernel to write with".to_string())
                .and_then(|k| write_construction(k, &items, iges).map_err(|e| e.to_string()));
            match written {
                Ok((contents, warnings)) => {
                    for warning in warnings {
                        ctx.log_warn(format!("Save elements as {kind}: {warning}"));
                    }
                    ctx.request(core_document::HostRequest::SaveFile {
                        name: "inspection".to_string(),
                        kind: kind.to_string(),
                        extension: extension.to_string(),
                        contents,
                    });
                }
                Err(why) => ctx.log_warn(format!("Save elements as {kind}: {why}")),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(SAVE_REPORT)) = (event, active_tool) {
            let report = report_of(
                ctx.document,
                None,
                &self.tab.tolerance_readings,
                ctx.kernel,
                true,
            );
            ctx.request(core_document::HostRequest::SaveFile {
                name: "inspection report".to_string(),
                kind: "CSV".to_string(),
                extension: "csv".to_string(),
                contents: report::csv(&report).into_bytes(),
            });
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(TOLERANCE)) = (event, active_tool) {
            let ids = selected_elements(ctx);
            let picked: Vec<Element> = ids
                .iter()
                .filter_map(|id| element_at(ctx.document, *id))
                .collect();
            let Some((characteristic, angle_deg)) = characteristic_for(&picked) else {
                ctx.log_warn("select an element, or an element and its datum");
                return InputResult::consumed();
            };
            let t = tolerance::Tolerance {
                name: String::new(),
                scan: picked[0].scan,
                characteristic,
                element: ids[0],
                datum: ids.get(1).copied(),
                limit: DEFAULT_LIMIT,
                angle_deg,
                ..Default::default()
            };
            match add_tolerance(ctx, t) {
                Ok(answer) => {
                    let id = core_document::FeatureId(
                        answer["tolerance"].as_str().unwrap().parse().unwrap(),
                    );
                    self.tab.tolerance_task = Some(tolerance_task::Task { id, opening: None });
                }
                Err(why) => ctx.log_warn(why),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(THICKNESS)) = (event, active_tool) {
            let Some(body) = thickness_body(ctx) else {
                ctx.log_warn("select the body to read");
                return InputResult::consumed();
            };
            let t = thickness::WallThickness {
                name: String::new(),
                body,
                sphere: false,
                min_wall: None,
                max: None,
            };
            match add_thickness(ctx, &self.tab.maps, t) {
                Ok(answer) => {
                    let mut args = CommandArgs::new();
                    args.insert("body".into(), json!(body.0.to_string()));
                    ctx.log_info(format!(
                        "{}: {:.3} to {:.3} mm",
                        answer["name"].as_str().unwrap_or_default(),
                        answer["min"].as_f64().unwrap_or(f64::NAN),
                        answer["max"].as_f64().unwrap_or(f64::NAN)
                    ));
                    ctx.record(THICKNESS, args, answer);
                }
                Err(why) => ctx.log_warn(why),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(EXTRACT)) = (event, active_tool) {
            let picked = selected_elements(ctx);
            let made = match picked.as_slice() {
                [id] => element_solid(ctx.document, *id, 2.0),
                _ => Err("select one fitted surface".to_string()),
            };
            match made {
                Ok(solid) => {
                    let name = solid["name"].as_str().unwrap_or("Element").to_string();
                    let body = ctx.document.create_body(None);
                    ctx.document.rename_body(body, name.clone());
                    let mut args = CommandArgs::new();
                    args.insert("name".into(), json!(name));
                    ctx.record("doc.new_body", args, json!(body.0.to_string()));
                    let mut args = CommandArgs::new();
                    args.insert("body".into(), json!(body.0.to_string()));
                    args.insert("kind".into(), solid["kind"].clone());
                    args.insert("placement".into(), solid["placement"].clone());
                    ctx.request(core_document::HostRequest::RunCommand {
                        id: "design.primitive".into(),
                        args,
                    });
                    ctx.request(core_document::HostRequest::SelectBody(body));
                    ctx.request(core_document::HostRequest::JournalLabel(format!(
                        "{name} as body"
                    )));
                }
                Err(why) => ctx.log_warn(why),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(CLEARANCE)) = (event, active_tool) {
            let Some((part, housing)) = compare_pair(ctx) else {
                ctx.log_warn("select the scanned part; the solid it fits into is found");
                return InputResult::consumed();
            };
            let c = clearance::Clearance {
                name: String::new(),
                part,
                housing,
                min_gap: None,
                reach: clearance::DEFAULT_REACH,
            };
            match add_clearance(ctx, &self.tab.maps, c) {
                Ok(answer) => {
                    let mut args = CommandArgs::new();
                    args.insert("part".into(), json!(part.0.to_string()));
                    args.insert("housing".into(), json!(housing.0.to_string()));
                    ctx.log_info(format!(
                        "{}: {:.1} % interfering, narrowest gap {}",
                        answer["name"].as_str().unwrap_or_default(),
                        answer["interfering"].as_f64().unwrap_or(0.0) * 100.0,
                        answer["narrowest"]
                            .as_f64()
                            .map_or("none".to_string(), |g| format!("{g:.3} mm"))
                    ));
                    ctx.record(CLEARANCE, args, answer);
                }
                Err(why) => ctx.log_warn(why),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(OVERHANG)) = (event, active_tool) {
            let Some(body) = thickness_body(ctx) else {
                ctx.log_warn("select the body to read");
                return InputResult::consumed();
            };
            let o = overhang::Overhang {
                name: String::new(),
                body,
                limit_deg: 45.0,
            };
            match add_overhang(ctx, &self.tab.maps, o) {
                Ok(answer) => {
                    let mut args = CommandArgs::new();
                    args.insert("body".into(), json!(body.0.to_string()));
                    ctx.log_info(format!(
                        "{}: {:.1} % of the surface past 45°, steepest {:.0}°",
                        answer["name"].as_str().unwrap_or_default(),
                        answer["share"].as_f64().unwrap_or(0.0) * 100.0,
                        answer["steepest"].as_f64().unwrap_or(0.0)
                    ));
                    ctx.record(OVERHANG, args, answer);
                }
                Err(why) => ctx.log_warn(why),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(align_commands::POINTS)) =
            (event, active_tool)
        {
            let aligning = !self.tab.picks.aligning;
            self.tab.picks.clear();
            self.tab.picks.aligning = aligning;
            self.tab.armed = None;
            self.tab.pinning = false;
            return InputResult::consumed();
        }
        if self.tab.picks.aligning
            && let Some(done) = self.align_input(event, ctx)
        {
            return done;
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(id)) = (event, active_tool)
            && align_commands::owns(id)
        {
            if let Some(args) = align_commands::selected_args(id, ctx) {
                match align_commands::run(id, &args, ctx) {
                    Ok(answer) => ctx.record(id, args, answer),
                    Err(why) => ctx.log_warn(why.to_string()),
                }
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(BEST_FIT)) = (event, active_tool) {
            if self.tab.fitting.cancel() {
                ctx.log_info("Best fit stopped");
                return InputResult::consumed();
            }
            let (Some((scan, reference)), Some(kernel)) = (compare_pair(ctx), ctx.kernel) else {
                ctx.log_warn("select the reference body, with one scan in the document");
                return InputResult::consumed();
            };
            let inputs = match compare::inputs_for(ctx.document, scan, reference) {
                Ok(inputs) => inputs,
                Err(why) => {
                    ctx.log_warn(why);
                    return InputResult::consumed();
                }
            };
            let placed = ctx.document.body_placement(reference);
            let reply = self.tab.fitting.begin(ctx.document, scan, reference, true);
            std::thread::spawn(move || {
                reply.send(align::best_fit(kernel, &inputs, placed, true));
            });
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(RESET_ALIGNMENT)) = (event, active_tool) {
            let scan = compare_pair(ctx)
                .map(|(scan, _)| scan)
                .or_else(|| ctx.selected_body_id.map(BodyId));
            if let Some(scan) = scan {
                match record_alignment(
                    ctx,
                    scan,
                    align::How::Reset,
                    core_document::BodyPlacement::IDENTITY,
                ) {
                    Ok(answer) => {
                        let mut args = CommandArgs::new();
                        args.insert("scan".into(), json!(scan.0.to_string()));
                        ctx.record(RESET_ALIGNMENT, args, answer);
                    }
                    Err(why) => ctx.log_warn(why),
                }
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(COMPARE_TOOL)) = (event, active_tool) {
            let Some((scan, reference)) = compare_pair(ctx) else {
                ctx.log_warn("select the reference body, with one scan in the document");
                return InputResult::consumed();
            };
            let c = Comparison {
                name: String::new(),
                scan,
                reference,
                range: None,
                bands: 0,
                ramp: metrology::ramp::Ramp::Jet.name().to_string(),
                tolerance: None,
                max: None,
                facing_limit_deg: Some(90.0),
                volume_cell: None,
            };
            match add_comparison(ctx, &self.tab.maps, c) {
                Ok((_, answer)) => {
                    let mut args = CommandArgs::new();
                    args.insert("scan".into(), json!(scan.0.to_string()));
                    args.insert("reference".into(), json!(reference.0.to_string()));
                    ctx.log_info(format!(
                        "{}: RMS {:.4} mm over {} points",
                        answer["name"].as_str().unwrap_or_default(),
                        answer["rms"].as_f64().unwrap_or(f64::NAN),
                        answer["measured"]
                    ));
                    ctx.record(COMPARE_COMMAND, args, answer);
                }
                Err(why) => ctx.log_warn(why),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(made)) =
            (event, active_tool.and_then(made_of_tool))
        {
            let from = selected_elements(ctx);
            match add_construction(ctx, made, from.clone(), 0.0) {
                Ok((_, answer)) => {
                    let mut args = CommandArgs::new();
                    args.insert("made".into(), json!(made.word().to_lowercase()));
                    args.insert(
                        "from".into(),
                        json!(from.iter().map(|e| e.0.to_string()).collect::<Vec<_>>()),
                    );
                    ctx.log_info(format!(
                        "{} made",
                        answer["name"].as_str().unwrap_or_default()
                    ));
                    ctx.record(CONSTRUCT_COMMAND, args, answer);
                }
                Err(why) => ctx.log_warn(why),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(kind)) =
            (event, active_tool.and_then(dim_kind_of_tool))
        {
            let elements = selected_elements(ctx);
            match add_dimension(ctx, kind, elements.clone(), None) {
                Ok((_, answer)) => {
                    let mut args = CommandArgs::new();
                    args.insert("kind".into(), json!(kind.word().to_lowercase()));
                    args.insert(
                        "elements".into(),
                        json!(elements.iter().map(|e| e.0.to_string()).collect::<Vec<_>>()),
                    );
                    ctx.log_info(format!(
                        "{}: {}",
                        answer["name"].as_str().unwrap_or_default(),
                        answer["value"]
                            .as_f64()
                            .map_or(String::new(), |v| format!("{v:.3} {}", kind.unit()))
                    ));
                    ctx.record(DIMENSION_COMMAND, args, answer);
                }
                Err(why) => ctx.log_warn(why),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(PIN)) = (event, active_tool) {
            self.tab.pinning = !self.tab.pinning;
            self.tab.armed = None;
            return InputResult::consumed();
        }
        if self.tab.pinning
            && let WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                ..
            } = event
        {
            if let Some((scan, _, at)) = self.hovered_reading(ctx) {
                let mut args = CommandArgs::new();
                args.insert("scan".into(), json!(scan.0.to_string()));
                args.insert("at".into(), json!(at));
                match self.run_command(PIN, &args, ctx) {
                    Ok(answer) => ctx.record(PIN, args, answer),
                    Err(why) => ctx.log_warn(why.to_string()),
                }
                return InputResult::consumed();
            }
            return InputResult::ignored();
        }
        match event {
            WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Escape,
            } if self.tab.fitting.running() => {
                self.tab.fitting.cancel();
                ctx.log_info("Best fit stopped");
                return InputResult::consumed();
            }
            WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Escape,
            } if self.tab.armed.is_some()
                || self.tab.pinning
                || self.tab.picks.aligning
                || self.tab.picks.any() =>
            {
                self.tab.armed = None;
                self.tab.pinning = false;
                self.tab.picks.clear();
                return InputResult::consumed();
            }
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                ..
            } => {
                let Some(kind) = self.tab.armed else {
                    return InputResult::ignored();
                };
                let Some(scan) = ctx
                    .hovered_body_id
                    .map(BodyId)
                    .filter(|b| ctx.document.body_scan(*b).is_some())
                else {
                    return InputResult::ignored();
                };
                let Some(face) = ctx.hovered_face else {
                    return InputResult::ignored();
                };
                let at = ctx
                    .document
                    .body_placement(scan)
                    .inverse()
                    .point(face.point)
                    .map(f64::from);
                // Shift pins the click as one more seed; a plain click
                // grows the fit from every pin and itself.
                if ctx.shift_down {
                    self.tab.picks.pin_seed(scan, at);
                    return InputResult::consumed();
                }
                let more = self.tab.picks.take_seeds(scan);
                match grow_at(
                    ctx,
                    kind,
                    scan,
                    at,
                    &more,
                    Some(DEFAULT_CUTOFF),
                    false,
                    kernel_api::FitMethod::LeastSquares,
                ) {
                    Ok((id, element)) => {
                        let mut args = CommandArgs::new();
                        args.insert(
                            "kind".into(),
                            json!(element::kind_word(kind).to_lowercase()),
                        );
                        args.insert("scan".into(), json!(scan.0.to_string()));
                        args.insert("at".into(), json!(at));
                        if !more.is_empty() {
                            args.insert("more".into(), json!(more));
                        }
                        ctx.record(GROW_COMMAND, args, answer_of(id, &element));
                        ctx.log_info(format!("{}: {}", element.name, element.result.summary()));
                    }
                    Err(why) => ctx.log_warn(why),
                }
                return InputResult::consumed();
            }
            _ => {}
        }
        let (WorkbenchInputEvent::ToolActivated, Some(kind)) =
            (event, active_tool.and_then(kind_of_tool))
        else {
            return InputResult::ignored();
        };
        if ctx.current_marking().is_none() {
            self.tab.armed = (self.tab.armed != Some(kind)).then_some(kind);
            self.tab.picks.clear();
            return InputResult::consumed();
        }
        match fit_marked(
            ctx,
            kind,
            Some(DEFAULT_CUTOFF),
            kernel_api::FitMethod::LeastSquares,
        ) {
            Ok((id, element)) => {
                let mut args = CommandArgs::new();
                args.insert(
                    "kind".into(),
                    json!(element::kind_word(kind).to_lowercase()),
                );
                ctx.record(FIT_COMMAND, args, answer_of(id, &element));
                ctx.log_info(format!("{}: {}", element.name, element.result.summary()));
            }
            Err(why) => ctx.log_warn(why),
        }
        InputResult::consumed()
    }

    fn run_command(
        &mut self,
        id: &str,
        args: &CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> CommandResult {
        if align_commands::owns(id) {
            return align_commands::run(id, args, ctx);
        }
        if let Some(kernel) = ctx.kernel {
            let _ = self.kernel.set(kernel);
        }
        commands::run(self, id, args, ctx)
    }
}

impl InspectionWorkbench {
    fn status_items(&self, ctx: &WorkbenchRuntimeContext) -> Option<core_document::StatusItems> {
        if self.tab.fitting.running() {
            return Some(core_document::StatusItems {
                mode: Some("Fitting the scan onto its reference… · Esc to stop".to_string()),
                ..Default::default()
            });
        }
        if self.tab.picks.aligning {
            let n = self.tab.picks.pairs();
            return Some(core_document::StatusItems {
                mode: Some(format!(
                    "Align by points: click a feature on the scan, then the same on the \
                     reference · {n} pair{} · Enter aligns from {} · Esc to stop",
                    if n == 1 { "" } else { "s" },
                    picks::PAIRS_NEEDED
                )),
                ..Default::default()
            });
        }
        self.armed_status(ctx)
    }

    fn faded_bodies(&self, ctx: &WorkbenchRuntimeContext) -> Vec<BodyId> {
        let document = &*ctx.document;
        document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == compare::KIND && n.visible)
            .filter_map(|(id, _)| document.feature_as::<Comparison>(*id).map(|c| c.reference))
            .collect()
    }

    fn scalar_maps(&self, ctx: &WorkbenchRuntimeContext) -> Vec<core_document::ScalarMap> {
        let Some(kernel) = ctx.kernel else {
            return Vec::new();
        };
        let _ = self.kernel.set(kernel);
        let mut maps = Vec::new();
        self.tab
            .maps
            .paint_all::<Comparison>(ctx, kernel, &mut maps);
        self.tab
            .maps
            .paint_all::<element_map::ElementMap>(ctx, kernel, &mut maps);
        self.tab
            .maps
            .paint_all::<thickness::WallThickness>(ctx, kernel, &mut maps);
        self.tab
            .maps
            .paint_all::<clearance::Clearance>(ctx, kernel, &mut maps);
        self.tab
            .maps
            .paint_all::<overhang::Overhang>(ctx, kernel, &mut maps);
        maps
    }

    /// Every point element, a cross the same size at any zoom, and the
    /// clicks of an element in progress, pinned.
    fn screen_marks(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<core_document::FeatureId>,
    ) -> Vec<core_document::ScreenSpaceMark> {
        let document = &*ctx.document;
        let mut marks: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| {
                n.visible && matches!(n.workbench_id.as_str(), ID | construction::KIND)
            })
            .filter_map(|(id, _)| {
                let e = element_at(document, *id)?;
                let kernel_api::FittedShape::Sphere { centre, radius } = e.result.shape else {
                    return None;
                };
                if radius != 0.0 || !document.imported_body_effective_visible(e.scan) {
                    return None;
                }
                let world = document
                    .body_placement(e.scan)
                    .point(centre.map(|c| c as f32));
                let (x, y) = ctx.world_to_viewport_on(Some(e.scan.0), world)?;
                Some(core_document::ScreenSpaceMark::crosshair(
                    [x, y],
                    POINT_CROSS_PT * ctx.pixels_per_point,
                    ctx.sketch_palette.external,
                ))
            })
            .collect();
        marks.extend(self.tab.picks.drawn(ctx).0);
        marks
    }

    fn screen_labels(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<core_document::FeatureId>,
    ) -> Vec<core_document::ScreenSpaceLabel> {
        let palette = &ctx.sketch_palette;
        let document = &*ctx.document;
        let mut labels = self.tab.picks.drawn(ctx).1;
        let mut put = |body: BodyId, at: [f64; 3], text: String, color: [f32; 3]| {
            if !document.imported_body_effective_visible(body) {
                return;
            }
            let world = document.body_placement(body).point(at.map(|c| c as f32));
            if let Some((x, y)) = ctx.world_to_viewport(world) {
                labels.push(core_document::ScreenSpaceLabel {
                    pos: [x, y],
                    text,
                    color,
                    size: 11.0,
                    background: true,
                    mono: true,
                });
            }
        };
        let mut dims: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == dimension::KIND && n.visible)
            .collect();
        dims.sort_by_key(|(id, n)| (n.seq, **id));
        for (id, _) in dims {
            let Some((dim, Ok(reading))) = dimension_at(document, *id) else {
                continue;
            };
            let color = match dim.passes(reading.value) {
                Some(true) => palette.fully_constrained,
                Some(false) => palette.conflict,
                None => palette.constraint,
            };
            let middle = ((DVec3::from(reading.from) + DVec3::from(reading.to)) * 0.5).to_array();
            put(
                dim.scan,
                middle,
                dim.label(&reading, ctx.document.display_unit()),
                color,
            );
        }
        let mut sections: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == section::KIND && n.visible)
            .collect();
        sections.sort_by_key(|(id, n)| (n.seq, **id));
        for (id, node) in sections {
            let Some((s, cuts, _)) = self.cuts_of(document, *id) else {
                continue;
            };
            // A stack's top cut says how its loops run through it.
            let tracked = section::tracks(&cuts, s.normal);
            let summary = match tracked.as_slice() {
                [] => String::new(),
                [t] => format!(
                    " · draft {:+.2}° · tilt {:.2}° · R {:.2}→{:.2}",
                    t.draft_deg + 0.0,
                    t.tilt_deg,
                    t.radius[0],
                    t.radius[1]
                ),
                many => format!(" · {} loops tracked", many.len()),
            };
            for (k, c) in cuts.iter().enumerate() {
                let Some(at) = c
                    .chains
                    .iter()
                    .max_by(|a, b| a.points.len().cmp(&b.points.len()))
                    .and_then(|chain| chain.points.first())
                else {
                    continue;
                };
                let mut text = format!("{} · {:.1} mm", node.name, c.length());
                if s.count > 1 {
                    text = format!("{} {:+.1} · {:.1} mm", node.name, c.offset, c.length());
                }
                if c.closed() > 0 {
                    text.push_str(&format!(" · {:.1} mm²", c.area(s.normal)));
                }
                if k + 1 == cuts.len() {
                    text.push_str(&summary);
                }
                put(s.scan, *at, text, palette.constraint);
            }
        }
        for (_, node) in document.feature_tree().all_nodes() {
            if node.workbench_id.as_str() != shadow::KIND || !node.visible {
                continue;
            }
            let Some(body) = node.body else {
                continue;
            };
            let Some((_, s)) = shadows_on(document, body).into_iter().next() else {
                continue;
            };
            let Some(made) = s.made.as_ref() else {
                continue;
            };
            let Some(at) = made
                .outline
                .first()
                .and_then(|w| w.segments.first())
                .map(|seg| {
                    let [u, v] = match *seg {
                        kernel_api::ProfileSegment::Line { start, .. }
                        | kernel_api::ProfileSegment::Arc { start, .. } => start,
                        kernel_api::ProfileSegment::Circle { center, radius } => {
                            [center[0] + radius, center[1]]
                        }
                        _ => [0.0; 2],
                    };
                    shadow::on_plane(&made.plane, [u, v])
                })
            else {
                continue;
            };
            let loops = made.outline.len();
            let text = format!(
                "{} · {loops} loop{} · {:.1} mm² · clear {:.2}–{:.2} mm",
                node.name,
                if loops == 1 { "" } else { "s" },
                made.area,
                made.band[0],
                made.band[1]
            );
            put(
                body,
                ctx.document
                    .body_placement(body)
                    .inverse()
                    .point(at.map(|c| c as f32))
                    .map(f64::from),
                text,
                palette.constraint,
            );
        }
        let mut tols: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == tolerance::KIND && n.visible)
            .collect();
        tols.sort_by_key(|(id, n)| (n.seq, **id));
        for (id, _) in tols {
            let Some((t, Ok(e))) = tolerance_at(
                document,
                *id,
                &self.tab.tolerance_readings,
                self.kernel.get().copied(),
                false,
            ) else {
                continue;
            };
            let color = if t.accepts(&e) {
                palette.fully_constrained
            } else {
                palette.conflict
            };
            put(
                t.scan,
                e.at,
                t.label(&e, ctx.document.display_unit()),
                color,
            );
        }
        for (_, node) in document.feature_tree().all_nodes() {
            if node.workbench_id.as_str() != pins::KIND || !node.visible {
                continue;
            }
            let Ok(pin) = pins::Pin::from_json(&node.data) else {
                continue;
            };
            let Some((map, values)) = self.shown_map(document, pin.scan) else {
                continue;
            };
            if map != pin.map {
                continue;
            }
            let Some((mesh, _)) = document.local_geometry(pin.scan) else {
                continue;
            };
            let text = pins::read(&mesh, &values, pin.at, READING_REACH).map_or_else(
                || "unmeasured".to_string(),
                |v| show_mm_signed(v, ctx.document.display_unit()),
            );
            put(pin.scan, pin.at, text, palette.constraint);
        }
        if let Some((scan, value, at)) = self.hovered_reading(ctx) {
            put(
                scan,
                at,
                show_mm_signed(value, ctx.document.display_unit()),
                palette.selected,
            );
        }
        // The largest hot spots of each shown comparison with a tolerance.
        for (id, node) in document.feature_tree().all_nodes() {
            if node.workbench_id.as_str() != compare::KIND || !node.visible {
                continue;
            }
            let Some(c) = document.feature_as::<Comparison>(*id) else {
                continue;
            };
            let (Some(tolerance), Ok(inputs)) = (c.tolerance, compare::inputs(document, &c)) else {
                continue;
            };
            let Some(spots) = self.tab.maps.hot_spots(*id, inputs.key, tolerance, || {
                self.tab
                    .maps
                    .values(*id, inputs.key)
                    .map(|v| hotspots::find(inputs.scan_mesh(), &v, tolerance, SPOT_LEAST))
            }) else {
                continue;
            };
            for spot in spots.iter().take(LABELLED_SPOTS) {
                let color = if spot.side > 0 {
                    palette.conflict
                } else {
                    palette.constraint
                };
                put(
                    c.scan,
                    spot.at,
                    format!("{:+.3} mm · {:.0} mm²", spot.extreme, spot.area),
                    color,
                );
            }
        }
        // The largest volume errors of each shown comparison that reads them.
        for (id, node) in document.feature_tree().all_nodes() {
            if node.workbench_id.as_str() != compare::KIND || !node.visible {
                continue;
            }
            let Some(c) = document.feature_as::<Comparison>(*id) else {
                continue;
            };
            let (Some(cell), Ok(inputs)) = (c.volume_cell, compare::inputs(document, &c)) else {
                continue;
            };
            let key = inputs.key;
            let back = inputs.scan_to_reference().inverse();
            let Some(found) = self.tab.maps.volumes(*id, key, cell) else {
                if let Some(kernel) = self.kernel.get() {
                    self.tab.maps.request_volumes(*id, inputs, cell, *kernel);
                }
                continue;
            };
            let Ok(regions) = found else {
                continue;
            };
            for r in regions.iter().take(LABELLED_SPOTS) {
                let at = back.point(volume::centre(r).map(|v| v as f32));
                let color = if r.missing {
                    palette.constraint
                } else {
                    palette.conflict
                };
                put(
                    c.scan,
                    at.map(f64::from),
                    volume::label(r, document.display_unit()),
                    color,
                );
            }
        }
        spread(&mut labels);
        labels
    }
}

#[cfg(test)]
mod gdt_tests;

#[cfg(test)]
mod best_fit_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::Workbench;
    use std::sync::Arc;

    /// A kernel that fits every request as a unit sphere and keeps all
    /// points but the last.
    struct UnitSphere;

    impl kernel_api::KernelQueries for UnitSphere {
        fn project_edge(
            &self,
            _brep: &[u8],
            _near: [f64; 3],
            _plane: &kernel_api::ProfilePlane,
        ) -> kernel_api::KernelResult<kernel_api::ProjectedEdge> {
            Err(kernel_api::KernelError::Unsupported("project_edge".into()))
        }

        /// A 2 mm square, outlined by a circle of radius 2.
        fn mesh_shadow(
            &self,
            _mesh: &kernel_api::TriMesh,
            _request: &kernel_api::ShadowRequest,
        ) -> kernel_api::KernelResult<kernel_api::ShadowAnswer> {
            Ok(kernel_api::ShadowAnswer {
                silhouette: vec![vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]],
                outline: vec![kernel_api::ProfileWire::new(vec![
                    kernel_api::ProfileSegment::Circle {
                        center: [0.0; 2],
                        radius: 2.0,
                    },
                ])],
            })
        }

        /// A 2 mm square about the axis at each offset.
        fn mesh_sections(
            &self,
            _mesh: &kernel_api::TriMesh,
            origin: [f64; 3],
            _normal: [f64; 3],
            offsets: &[f64],
        ) -> kernel_api::KernelResult<Vec<Vec<kernel_api::SectionChain>>> {
            Ok(offsets
                .iter()
                .map(|z| {
                    let z = origin[2] + z;
                    vec![kernel_api::SectionChain {
                        points: vec![
                            [-1.0, -1.0, z],
                            [1.0, -1.0, z],
                            [1.0, 1.0, z],
                            [-1.0, 1.0, z],
                        ],
                        outward: vec![[0.0; 3]; 4],
                        closed: true,
                    }]
                })
                .collect())
        }

        fn fit(&self, request: &FitRequest) -> kernel_api::KernelResult<kernel_api::FitAnswer> {
            let n = request.points.len();
            Ok(kernel_api::FitAnswer {
                shape: kernel_api::FittedShape::Sphere {
                    centre: [0.0; 3],
                    radius: 1.0,
                },
                sigma: 0.01,
                form: (-0.02, 0.03),
                used: (0..n).map(|i| i + 1 < n).collect(),
                residuals: vec![0.0; n],
                converged: true,
            })
        }
    }

    /// Every tool names an icon the set has. A fit takes the marked part of
    /// a scan, keeps it as an element under the scan with the vertices it
    /// rests on and what the kernel made of them, records itself as a
    /// command, and the report lists it; with nothing marked the tool says
    /// so and makes nothing. A dimension on it passes its band and goes
    /// when the element is deleted. Datums square the scan, and picked
    /// point pairs bring it where a reference has it.
    #[test]
    fn a_fit_of_the_marked_surface_becomes_an_element() {
        let mut context = WorkbenchContext::default();
        InspectionWorkbench::default().configure(&mut context);
        assert_eq!(
            context.tools().len(),
            FitKind::ALL.len() + DimKind::ALL.len() + Made::ALL.len() + 19
        );
        for tool in context.tools() {
            let icon = tool.icon.expect("an icon");
            assert!(ui_kit::icon::exists(icon), "{icon}");
        }
        assert!(ui_kit::icon::exists("measure"));

        let mut document = Document::new("Scan");
        let scan = document.create_body(Some("Scan".into()));
        let mesh = kernel_api::TriMesh {
            positions: vec![
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [-1.0, 0.0, 0.0],
            ],
            normals: vec![
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [-1.0, 0.0, 0.0],
            ],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        document.set_imported_geometry(
            scan,
            core_document::ImportedGeometry {
                mesh: Arc::new(mesh),
                source_asset: None,
                revision: 0,
                bounds_mm: None,
                brep_blob_path: None,
                mesh_path: None,
                face_colors_path: None,
                health: None,
            },
        );
        let mut bench = InspectionWorkbench::default();
        let mut ctx = WorkbenchRuntimeContext::new(&mut document, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        ctx.kernel = Some(&UnitSphere);
        let tool = tool_id(FitKind::Sphere);
        assert!(!bench.is_tool_enabled(&tool, &ctx));
        bench.on_input(&WorkbenchInputEvent::ToolActivated, Some(&tool), &mut ctx);
        let outcome = core_document::HookOutcome::take(&mut ctx);
        assert!(outcome.recorded.is_empty(), "nothing marked, nothing made");

        ctx.marking = Some(core_document::MarkedSurface {
            body: scan,
            revision: 0,
            vertices: Arc::new(vec![true, true, false, true]),
        });
        // A marking of a mesh the scan no longer has fits nothing: the
        // tool arms instead.
        ctx.marking.as_mut().unwrap().revision += 1;
        bench.tab.armed = None;
        assert!(!bench.is_tool_enabled(&tool, &ctx));
        bench.on_input(&WorkbenchInputEvent::ToolActivated, Some(&tool), &mut ctx);
        assert!(
            core_document::HookOutcome::take(&mut ctx)
                .recorded
                .is_empty()
        );
        assert!(bench.tab.armed.take().is_some());
        ctx.marking.as_mut().unwrap().revision -= 1;
        assert!(bench.is_tool_enabled(&tool, &ctx));
        bench.on_input(&WorkbenchInputEvent::ToolActivated, Some(&tool), &mut ctx);
        assert_eq!(
            core_document::HookOutcome::take(&mut ctx).recorded[0].id,
            FIT_COMMAND
        );
        let report = bench
            .run_command(REPORT_COMMAND, &CommandArgs::new(), &mut ctx)
            .unwrap();
        let first = &report["elements"][0];
        assert_eq!(first["name"], "Sphere 1");
        assert_eq!(
            (first["points"].clone(), first["used"].clone()),
            (json!(3), json!(2))
        );
        assert!((first["form"].as_f64().unwrap() - 0.05).abs() < 1e-12);
        let id = core_document::FeatureId(
            uuid::Uuid::parse_str(first["element"].as_str().unwrap()).unwrap(),
        );
        let data = ctx.document.feature_values(id).unwrap();
        let element = Element::from_json(data).unwrap();
        assert_eq!(
            (element.scan, element.region.clone()),
            (scan, vec![[0, 2], [3, 4]])
        );
        assert_eq!(element.result.lines().edges.len() / 2, 3 * 48);

        let mut args = CommandArgs::new();
        args.insert("kind".into(), json!("diameter"));
        args.insert("elements".into(), json!([first["element"]]));
        args.insert("plus".into(), json!(0.1));
        let dim = bench
            .run_command(DIMENSION_COMMAND, &args, &mut ctx)
            .unwrap();
        assert_eq!(
            (dim["value"].clone(), dim["pass"].clone()),
            (json!(2.0), json!(true))
        );
        let report = bench
            .run_command(REPORT_COMMAND, &CommandArgs::new(), &mut ctx)
            .unwrap();
        assert_eq!(
            (report["passed"].clone(), report["failed"].clone()),
            (json!(1), json!(0))
        );
        assert!(bench.delete_feature(&mut ctx, id));
        let report = bench
            .run_command(REPORT_COMMAND, &CommandArgs::new(), &mut ctx)
            .unwrap();
        assert_eq!(report["dimensions"], json!([]), "gone with its element");

        let asset = ctx.document.add_asset(core_document::AssetReference::new(
            "assets/scan.stl",
            core_document::AssetType::Stl,
            json!({}),
        ));
        let mut geometry = ctx.document.imported_geometry(scan).unwrap().clone();
        geometry.source_asset = Some(asset);
        ctx.document.set_imported_geometry(scan, geometry);
        assert!(ctx.document.set_body_scan(scan, Some(Default::default())));
        let before = align::by_numbers([5.0, 8.0, 3.0], [10.0, 20.0, 30.0]);
        ctx.document.set_body_placement(scan, before);
        let mut ids = Vec::new();
        for normal in [[0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            let mut datum = element.clone();
            datum.kind = FitKind::Plane;
            datum.result.shape = kernel_api::FittedShape::Plane {
                origin: [1.0, 2.0, 3.0],
                normal,
            };
            ids.push(
                ctx.document
                    .add_feature_in_body(datum, "Datum".into(), Some(scan))
                    .unwrap(),
            );
        }
        ctx.selected_features = ids.clone();
        assert!(bench.is_tool_enabled(align_commands::DATUMS, &ctx));
        bench.on_input(
            &WorkbenchInputEvent::ToolActivated,
            Some(align_commands::DATUMS),
            &mut ctx,
        );
        let outcome = core_document::HookOutcome::take(&mut ctx);
        assert_eq!(outcome.recorded.last().unwrap().id, align_commands::DATUMS);
        let (_, node) = ctx
            .document
            .feature_tree()
            .all_nodes()
            .find(|(_, n)| n.workbench_id.as_str() == align::KIND)
            .unwrap();
        let alignment = align::Alignment::from_json(&node.data).unwrap();
        assert_eq!(alignment.before, before);
        assert_eq!(alignment.after.point([1.0, 2.0, 3.0]), [0.0; 3]);
        assert!(
            matches!(alignment.how, align::How::Datums { primary, secondary, zero, .. }
            if [primary, secondary, zero] == ids.as_slice())
        );
        ctx.selected_features.swap(0, 1);
        assert!(bench.is_tool_enabled(align_commands::DATUMS, &ctx));
        ctx.selected_features[1] = ctx.selected_features[0];
        assert!(!bench.is_tool_enabled(align_commands::DATUMS, &ctx));

        // Points picked on the scan where it stands and where they are on
        // a reference bring it exactly where the reference has it.
        let now = ctx.document.body_placement(scan);
        let target = align::by_numbers([-4.0, 2.0, 7.0], [15.0, -30.0, 60.0]);
        let picked = [
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [0.0, 5.0, 2.0],
            [3.0, 3.0, 8.0],
        ];
        let pairs: Vec<Value> = picked
            .iter()
            .map(|p| json!({"scan": now.point(*p), "reference": target.point(*p)}))
            .collect();
        let mut args = CommandArgs::new();
        args.insert("scan".into(), json!(scan.0.to_string()));
        args.insert("pairs".into(), json!(pairs));
        let answer = bench
            .run_command(align_commands::POINTS, &args, &mut ctx)
            .unwrap();
        assert!(answer["rms"].as_f64().unwrap() < 1e-4);
        let placed = ctx.document.body_placement(scan);
        for p in picked {
            let (a, b) = (placed.point(p), target.point(p));
            assert!((0..3).all(|k| (a[k] - b[k]).abs() < 1e-3), "{a:?} {b:?}");
        }
        assert_eq!(answer["name"], "Alignment 2");

        // Another scan's alignment is named apart from the first scan's.
        let other = ctx.document.create_body(Some("Other".into()));
        let geometry = ctx.document.imported_geometry(scan).unwrap().clone();
        ctx.document.set_imported_geometry(other, geometry);
        assert!(ctx.document.set_body_scan(other, Some(Default::default())));
        let mut args = CommandArgs::new();
        args.insert("scan".into(), json!(other.0.to_string()));
        let answer = bench.run_command(MOVE_COMMAND, &args, &mut ctx).unwrap();
        assert_eq!(answer["name"], "Alignment 3");
    }

    /// A stack of sections across a scan is kept in the scan's frame, reads
    /// each cut's loops, length and area, draws every chain, and cuts again
    /// when its count changes, and offers its first cut to sketch on. A
    /// shadow reads its area and clearance, its
    /// outline is derived once per change of what it is made from, and its
    /// body's plan stands the outline up the scan's height; a STEP export
    /// holds them all, a cylinder as its axis and rims.
    #[test]
    fn sections_and_shadows_read_and_draw_the_scan() {
        let mut document = Document::new("Scan");
        let scan = document.create_body(Some("Scan".into()));
        document.set_imported_geometry(
            scan,
            core_document::ImportedGeometry {
                mesh: Arc::new(kernel_api::TriMesh {
                    positions: vec![[-1.0, -1.0, 0.0], [1.0, 1.0, 10.0], [1.0, -1.0, 0.0]],
                    indices: vec![0, 1, 2],
                    ..Default::default()
                }),
                source_asset: None,
                revision: 0,
                bounds_mm: None,
                brep_blob_path: None,
                mesh_path: None,
                face_colors_path: None,
                health: None,
            },
        );
        let mut bench = InspectionWorkbench::default();
        let mut ctx = WorkbenchRuntimeContext::new(&mut document, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        ctx.kernel = Some(&UnitSphere);
        let mut args = CommandArgs::new();
        args.insert("scan".into(), json!(scan.0.to_string()));
        args.insert("offset".into(), json!(-4.0));
        args.insert("count".into(), json!(3));
        args.insert("step".into(), json!(4.0));
        let answer = bench.run_command(SECTION, &args, &mut ctx).unwrap();
        let cuts = answer["cuts"].as_array().unwrap();
        assert_eq!(cuts.len(), 3);
        for (cut, offset) in cuts.iter().zip([-4.0, 0.0, 4.0]) {
            assert_eq!(cut["offset"], json!(offset));
            assert_eq!(
                (cut["chains"].clone(), cut["closed"].clone()),
                (json!(1), json!(1))
            );
            assert!((cut["length"].as_f64().unwrap() - 8.0).abs() < 1e-12);
            assert!((cut["area"].as_f64().unwrap() - 4.0).abs() < 1e-12);
        }
        let id = core_document::FeatureId(answer["section"].as_str().unwrap().parse().unwrap());
        let (_, node) = document
            .feature_tree()
            .all_nodes()
            .find(|(f, _)| **f == id)
            .unwrap();
        let drawn = bench
            .passive_geometry(&document, id, &node.clone())
            .unwrap();
        assert_eq!(drawn.mesh.edges.len(), 3 * 4 * 2);
        assert_eq!(
            drawn.mesh.positions[0][2], 1.0,
            "the middle of the scan, less 4"
        );
        let offered = bench.feature_plane(&document, id, &node.clone()).unwrap();
        assert_eq!(
            (offered.origin[2], offered.normal),
            (1.0, [0.0, 0.0, 1.0]),
            "its first cut"
        );

        let mut args = CommandArgs::new();
        args.insert("scan".into(), json!(scan.0.to_string()));
        let mut ctx = WorkbenchRuntimeContext::new(&mut document, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        ctx.kernel = Some(&UnitSphere);
        let shadow = bench.run_command(SHADOW, &args, &mut ctx).unwrap();
        assert_eq!(
            (shadow["loops"].clone(), shadow["area"].clone()),
            (json!(1), json!(4.0))
        );
        let band = shadow["band"].as_array().unwrap();
        assert!((band[0].as_f64().unwrap() - (2.0 - 2f64.sqrt())).abs() < 0.01);
        assert!((band[1].as_f64().unwrap() - 1.0).abs() < 0.01);
        assert_eq!(
            shadow["depth"],
            json!(10.0),
            "the scan's height above its foot"
        );
        let block = BodyId(shadow["body"].as_str().unwrap().parse().unwrap());
        let sid = core_document::FeatureId(shadow["shadow"].as_str().unwrap().parse().unwrap());
        let (_, node) = document
            .feature_tree()
            .all_nodes()
            .find(|(f, _)| **f == sid)
            .unwrap();
        let (node, mut values) = (node.clone(), node.data.clone());
        assert!(bench.derive_on_geometry(&node, &mut values, &document, Some(&UnitSphere)));
        assert!(!bench.derive_on_geometry(&node, &mut values, &document, Some(&UnitSphere)));
        let made = shadow::Shadow::from_json(&values).unwrap();
        match shadow::ops(&made).as_slice() {
            [
                kernel_api::SolidOp::Sweep {
                    profile,
                    kind: kernel_api::SweepKind::Extrude { termination, .. },
                    ..
                },
            ] => {
                assert_eq!(profile.wires.len(), 1);
                assert_eq!(
                    *termination,
                    kernel_api::ExtrudeTermination::Blind { distance: 10.0 }
                );
                assert_eq!(profile.plane.normal, [0.0, 0.0, 1.0]);
            }
            other => panic!("{other:?}"),
        }
        // What a STEP export of the inspection holds: every cut's chain
        // (the shadow's outline joins once evaluation works it out).
        let items = bench.construction_items(&document, None);
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Section 1 -4.00 #1",
                "Section 1 +0.00 #1",
                "Section 1 +4.00 #1"
            ]
        );
        let e = Element {
            method: Default::default(),
            name: "Cylinder 1".into(),
            scan,
            kind: FitKind::Cylinder,
            cutoff: None,
            region: Vec::new(),
            revision: 0,
            result: Fitted {
                zone: None,
                shape: kernel_api::FittedShape::Cylinder {
                    point: [0.0; 3],
                    axis: [0.0, 0.0, 2.0],
                    radius: 5.0,
                },
                sigma: 0.0,
                form: (0.0, 0.0),
                used: 0,
                points: 0,
                span: element::Span::Axis([1.0, 9.0]),
            },
        };
        let shapes = construction_of(&e);
        assert_eq!(shapes.len(), 3, "its axis and two rims");
        assert!(matches!(
            shapes[2].1,
            kernel_api::ConstructionShape::Circle {
                centre: [0.0, 0.0, 9.0],
                radius: 5.0,
                ..
            }
        ));
        bench.invalidate_body(&mut document, block);
        let jobs = bench.rebuild_jobs(&mut document);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].body, block);

        let mut data = document.feature_values(id).unwrap().clone();
        data["count"] = json!(1);
        document.update_feature_data(id, data).unwrap();
        let (_, node) = document
            .feature_tree()
            .all_nodes()
            .find(|(f, _)| **f == id)
            .unwrap();
        let drawn = bench
            .passive_geometry(&document, id, &node.clone())
            .unwrap();
        assert_eq!(drawn.mesh.edges.len(), 4 * 2);
    }

    /// Each click of an element in progress is pinned and numbered 1..n,
    /// the same size on screen at any zoom: an armed fit's Shift-clicks
    /// until Escape puts them away, an alignment's points by pair (the
    /// reference's beside the scan's) until Enter makes it.
    #[test]
    fn clicks_in_progress_are_pinned_and_numbered_until_made() {
        let mut document = Document::new("Pins");
        let mesh = |dx: f32| kernel_api::TriMesh {
            positions: vec![[dx, 0.0, 0.0], [dx + 1.0, 0.0, 0.0], [dx, 1.0, 0.0]],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        let mut body = |name: &str, dx: f32| {
            let id = document.create_body(Some(name.into()));
            document.set_imported_geometry(
                id,
                core_document::ImportedGeometry {
                    mesh: Arc::new(mesh(dx)),
                    source_asset: None,
                    revision: 0,
                    bounds_mm: None,
                    brep_blob_path: None,
                    mesh_path: None,
                    face_colors_path: None,
                    health: None,
                },
            );
            id
        };
        let (scan, reference) = (body("Scan", 0.0), body("Reference", 5.0));
        let asset = document.add_asset(core_document::AssetReference::new(
            "assets/scan.stl",
            core_document::AssetType::Stl,
            json!({}),
        ));
        let mut geometry = document.imported_geometry(scan).unwrap().clone();
        geometry.source_asset = Some(asset);
        document.set_imported_geometry(scan, geometry);
        assert!(document.set_body_scan(scan, Some(Default::default())));
        let mut bench = InspectionWorkbench::default();
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut document, [0.0; 3], [0.0; 3], (0, 0, 400, 400));
        ctx.pixels_per_point = 2.0;
        let zoom = |k: f32| {
            [
                [k, 0.0, 0.0, 0.0],
                [0.0, k, 0.0, 0.0],
                [0.0, 0.0, 0.1, 0.0],
                [0.0, 0.0, 0.5, 1.0],
            ]
        };
        ctx.view_proj = Some(zoom(0.05));
        let shown = |bench: &InspectionWorkbench, ctx: &WorkbenchRuntimeContext| {
            let numbers: Vec<String> = bench
                .screen_labels(ctx, None)
                .into_iter()
                .map(|l| l.text)
                .collect();
            let radii: Vec<f32> = bench
                .screen_marks(ctx, None)
                .into_iter()
                .map(|m| match m.kind {
                    core_document::MarkKind::Dot { radius } => radius,
                    _ => 0.0,
                })
                .collect();
            (numbers, radii)
        };
        let click = |bench: &mut InspectionWorkbench,
                     ctx: &mut WorkbenchRuntimeContext,
                     body: BodyId,
                     p: [f32; 3]| {
            ctx.hovered_body_id = Some(body.0);
            ctx.hovered_face = Some(core_document::FaceRef {
                point: p,
                normal: [0.0, 0.0, 1.0],
                surface: None,
                name: 0,
            });
            let press = WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                viewport_pos: (0.0, 0.0),
            };
            assert!(bench.on_input(&press, None, ctx).consumed);
        };
        let key = |bench: &mut InspectionWorkbench, ctx: &mut WorkbenchRuntimeContext, key| {
            bench.on_input(&WorkbenchInputEvent::KeyPress { key }, None, ctx);
        };

        let plane = tool_id(FitKind::Plane);
        bench.on_input(&WorkbenchInputEvent::ToolActivated, Some(&plane), &mut ctx);
        ctx.shift_down = true;
        for x in [0.1, 0.4, 0.2] {
            click(&mut bench, &mut ctx, scan, [x, 0.2, 0.0]);
        }
        let (numbers, radii) = shown(&bench, &ctx);
        assert_eq!(numbers, ["1", "2", "3"]);
        ctx.view_proj = Some(zoom(4.0));
        assert_eq!(shown(&bench, &ctx).1, radii, "the same size zoomed in");
        assert!(radii.iter().all(|r| *r == radii[0] && *r > 0.0));
        key(&mut bench, &mut ctx, core_document::KeyCode::Escape);
        assert!(shown(&bench, &ctx).0.is_empty(), "Escape puts them away");

        ctx.shift_down = false;
        ctx.view_proj = Some(zoom(0.05));
        let tool = align_commands::POINTS;
        assert!(bench.is_tool_enabled(tool, &ctx));
        bench.on_input(&WorkbenchInputEvent::ToolActivated, Some(tool), &mut ctx);
        for (i, p) in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
            .into_iter()
            .enumerate()
        {
            if i == 1 {
                // Both bodies move between the clicks (an undo, another
                // client): the pins go with them. They park with their
                // tab.
                let place = |t| align::by_numbers(t, [0.0; 3]);
                ctx.document
                    .set_body_placement(scan, place([0.0, 3.0, 0.0]));
                ctx.document
                    .set_body_placement(reference, place([0.0, 0.0, 1.0]));
                let parked = bench.suspend_session();
                bench.resume_session(None);
                assert!(shown(&bench, &ctx).0.is_empty() && !bench.tab.picks.aligning);
                bench.resume_session(parked);
                assert_eq!(shown(&bench, &ctx).0, ["1", "1"]);
            }
            let at = |body| ctx.document.body_placement(body);
            let (on_scan, on_reference) = (
                at(scan).point(p),
                at(reference).point([p[0] + 5.0, p[1], p[2]]),
            );
            click(&mut bench, &mut ctx, scan, on_scan);
            click(&mut bench, &mut ctx, reference, on_reference);
        }
        assert_eq!(shown(&bench, &ctx).0, ["1", "2", "3", "1", "2", "3"]);
        core_document::HookOutcome::take(&mut ctx);
        key(&mut bench, &mut ctx, core_document::KeyCode::Enter);
        let outcome = core_document::HookOutcome::take(&mut ctx);
        assert_eq!(outcome.recorded.len(), 1, "the alignment is made");
        assert!(shown(&bench, &ctx).0.is_empty());
        let moved = ctx.document.body_placement(scan).point([0.0, 0.0, 0.0]);
        let want = [5.0, 0.0, 1.0];
        assert!(
            (0..3).all(|k| (moved[k] - want[k]).abs() < 1e-3),
            "{moved:?}"
        );
    }
}
