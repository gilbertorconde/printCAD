//! The Inspection workbench: measuring a scan. Its first tools fit a plane,
//! sphere, cylinder, cone, torus, circle or line to the part of a scan the
//! user marked (Mark surface…), by orthogonal least squares with points
//! beyond three sigmas left out, and keep the result as an element under
//! the scan: its size, its sigma, its form error, and the lines that draw
//! it no larger than measured. Dimensions between elements read
//! distances, angles and diameters off them, each with a nominal and a
//! band it passes or fails.

pub mod align;
mod align_commands;
pub mod clearance;
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
pub mod volume;

use compare::Comparison;
use construction::{Construction, Made};
use core_document::units::{show_mm, show_mm_signed};
use core_document::{
    BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, Document, FeatureInfo,
    FeatureNode, InputResult, ParamKind, PassiveGeometry, PassiveTint, ToolDescriptor,
    WorkbenchContext, WorkbenchDescriptor, WorkbenchFeature, WorkbenchInputEvent,
    WorkbenchRuntimeContext,
};
use dimension::{DimKind, Dimension};
use element::Element;
#[cfg(test)]
use element::Fitted;
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

/// A best fit running away from the window, and what it came to.
struct Fitting {
    scan: BodyId,
    reference: BodyId,
    search: bool,
    answer: Option<Result<(core_document::BodyPlacement, kernel_api::Registration), String>>,
}

/// A point element's cross across, in logical pixels.
const POINT_CROSS_PT: f32 = 14.0;

/// How deep constructions on constructions are followed.
const DEEPEST: usize = 32;

#[derive(Default)]
pub struct InspectionWorkbench {
    /// The fit a click on a scan makes, while nothing is marked.
    armed: Option<FitKind>,
    /// A click on a mapped scan pins a reading there.
    pinning: bool,
    /// The clicks of an element in progress, pinned until it is made.
    picks: picks::Picks,
    /// Every comparison's distances, measured away from the window.
    maps: maps::Maps,
    tolerance_readings: tolerance_read::Readings,
    tolerance_task: Option<tolerance_task::Task>,
    /// The best fit the toolbar started, while it runs and until applied.
    fitting: std::sync::Arc<std::sync::Mutex<Option<Fitting>>>,
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
    let number = ctx
        .document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == align::KIND && n.body == Some(scan))
        .count()
        + 1;
    let a = align::Alignment {
        name: format!("Alignment {number}"),
        scan,
        how,
        before,
        after,
    };
    ctx.document.set_body_placement(scan, after);
    let id = ctx
        .document
        .add_feature_in_body(a.clone(), a.name.clone(), Some(scan))
        .map_err(|e| e.to_string())?;
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
    mut c: Comparison,
) -> Result<(core_document::FeatureId, Value), String> {
    if ctx.document.body_scan(c.scan).is_none() {
        return Err("the first body is not a scan".into());
    }
    if c.scan == c.reference {
        return Err("a scan is compared with another body".into());
    }
    let number = ctx
        .document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == compare::KIND && n.body == Some(c.scan))
        .count()
        + 1;
    c.name = format!("Compare {number}");
    let kernel = ctx.kernel.ok_or("no kernel to measure with")?;
    let inputs = compare::inputs(ctx.document, &c)?;
    let values = compare::measure(kernel, &inputs, &c)?;
    let id = ctx
        .document
        .add_feature_in_body(c.clone(), c.name.clone(), Some(c.scan))
        .map_err(|e| e.to_string())?;
    maps.store(id, inputs.key, values.clone());
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
        .feature_values(id)
        .and_then(|d| Comparison::from_json(d).ok())
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
    mut t: thickness::WallThickness,
) -> Result<Value, String> {
    let number = ctx
        .document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == thickness::KIND && n.body == Some(t.body))
        .count()
        + 1;
    t.name = format!("Thickness {number}");
    let kernel = ctx.kernel.ok_or("no kernel to measure with")?;
    let inputs = thickness::inputs(ctx.document, &t)?;
    let values = thickness::measure(kernel, &inputs, &t)?;
    let id = ctx
        .document
        .add_feature_in_body(t.clone(), t.name.clone(), Some(t.body))
        .map_err(|e| e.to_string())?;
    maps.store(id, inputs.key, values.clone());
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
    mut c: clearance::Clearance,
) -> Result<Value, String> {
    if c.reach <= 0.0 {
        return Err("the reach is a distance above zero".into());
    }
    let number = ctx
        .document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == clearance::KIND)
        .count()
        + 1;
    c.name = format!("Fit {number}");
    let kernel = ctx.kernel.ok_or("no kernel to measure with")?;
    let inputs = clearance::inputs(ctx.document, &c)?;
    let values = clearance::measure(kernel, &inputs, &c)?;
    let id = ctx
        .document
        .add_feature_in_body(c.clone(), c.name.clone(), Some(c.part))
        .map_err(|e| e.to_string())?;
    maps.store(id, inputs.key, values.clone());
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

/// The map a fit check paints: interference red, wide gaps blue.
fn fit_map(
    name: &str,
    id: core_document::FeatureId,
    key: u64,
    c: &clearance::Clearance,
    r: &clearance::Reading,
    values: std::sync::Arc<[f32]>,
    ctx: &WorkbenchRuntimeContext,
) -> core_document::ScalarMap {
    let (none, unit) = (ctx.sketch_palette.inactive, ctx.document.display_unit());
    let ramp = metrology::ramp::Ramp::Jet;
    let (wide_cap, tight_cap) = ramp.caps();
    let mut stats = vec![(
        "Interfering".to_string(),
        format!("{:.1} %", r.interfering * 100.0),
    )];
    if let Some(d) = r.deepest {
        stats.push(("Deepest".to_string(), show_mm(d, unit)));
    }
    if let Some(g) = r.narrowest {
        stats.push(("Narrowest gap".to_string(), show_mm(g, unit)));
    }
    if let (Some(g), Some(share)) = (c.min_gap, r.too_tight) {
        stats.push((format!("Under {g}"), format!("{:.1} %", share * 100.0)));
    }
    core_document::ScalarMap {
        body: c.part,
        revision: key ^ id.0.as_u64_pair().1 ^ c.min_gap.unwrap_or(0.0).to_bits(),
        values,
        range: [-r.range as f32, r.range as f32],
        bands: 0,
        ramp: (0..32)
            .map(|i| ramp.at(1.0 - f64::from(i) / 31.0))
            .collect(),
        under: tight_cap,
        over: wide_cap,
        none,
        legend: core_document::MapLegend {
            title: format!("{name} · clearance"),
            unit: "mm".to_string(),
            stats,
            histogram: r.histogram.counts.iter().map(|n| *n as u32).collect(),
            tolerance: c.min_gap.map(|g| (0.0, g as f32)),
        },
    }
}

/// Keep an overhang map of `o.body` and read it at once.
fn add_overhang(
    ctx: &mut WorkbenchRuntimeContext,
    mut o: overhang::Overhang,
) -> Result<Value, String> {
    if !(0.0..90.0).contains(&o.limit_deg) {
        return Err("the limit is an angle from 0 to 90 degrees past vertical".into());
    }
    let mesh = ctx
        .document
        .imported_geometry(o.body)
        .map(|g| std::sync::Arc::clone(&g.mesh))
        .ok_or("the body has no geometry to read")?;
    let number = ctx
        .document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == overhang::KIND && n.body == Some(o.body))
        .count()
        + 1;
    o.name = format!("Overhang {number}");
    let values = overhang::measure(&mesh, overhang::BED);
    let (share, steepest) = overhang::read(&mesh, &values, o.limit_deg);
    let id = ctx
        .document
        .add_feature_in_body(o.clone(), o.name.clone(), Some(o.body))
        .map_err(|e| e.to_string())?;
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
    let t = tolerance::Tolerance::from_json(document.feature_values(id)?).ok()?;
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
    let direction = |e: &Element| match e.result.shape {
        S::Plane { normal, .. } => Some((normal, true)),
        S::Cylinder { axis, .. }
        | S::Cone { axis, .. }
        | S::Line {
            direction: axis, ..
        } => Some((axis, false)),
        _ => None,
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
            let len = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let cos = ((da[0] * db[0] + da[1] * db[1] + da[2] * db[2]) / (len(da) * len(db)))
                .clamp(-1.0, 1.0);
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
    let number = ctx
        .document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == tolerance::KIND && n.body == Some(e.scan))
        .count()
        + 1;
    t.name = format!("{} {number}", t.characteristic.word());
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
    let s = section::Section::from_json(document.feature_values(id)?).ok()?;
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
    use element::{add, scale};
    let mid = |t: [f64; 2]| (t[0] + t[1]) / 2.0;
    if let Some(id) = element {
        let e = element_at(document, id)
            .ok_or_else(|| CommandError::bad("element", "is not an element"))?;
        let axis_at = |point: [f64; 3], axis: [f64; 3]| match e.result.span {
            element::Span::Axis(t) => {
                let len = element::dot(axis, axis).sqrt().max(1e-12);
                add(point, scale(axis, mid(t) / len))
            }
            _ => point,
        };
        let (origin, axis) = match e.result.shape {
            kernel_api::FittedShape::Plane { origin, normal } => {
                if through {
                    return Err(CommandError::bad("through", "needs an axis, not a plane"));
                }
                let origin = match e.result.span {
                    element::Span::Patch { u, across } => {
                        let v = element::cross(normal, u);
                        add(
                            origin,
                            add(scale(u, mid(across[0])), scale(v, mid(across[1]))),
                        )
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
        let (u, v) = element::basis(axis);
        let (sin, cos) = angle.to_radians().sin_cos();
        let along = add(scale(u, cos), scale(v, sin));
        return Ok((e.scan, origin, element::cross(axis, along), Some(along)));
    }
    let scan = scan.ok_or_else(|| CommandError::bad("scan", "or an element is needed"))?;
    let (mesh, _) = document
        .local_geometry(scan)
        .ok_or_else(|| CommandError::bad("scan", "has no mesh"))?;
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for p in &mesh.positions {
        for k in 0..3 {
            lo[k] = lo[k].min(f64::from(p[k]));
            hi[k] = hi[k].max(f64::from(p[k]));
        }
    }
    if mesh.positions.is_empty() {
        return Err(CommandError::bad("scan", "has no mesh"));
    }
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
    Ok((scan, scale(add(lo, hi), 0.5), normal, None))
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
            let s = shadow::Shadow::from_json(document.feature_values(*id)?).ok()?;
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

/// `shape` of `e` as construction geometry, in the scan's frame.
fn construction_of(e: &Element) -> Vec<(String, kernel_api::ConstructionShape)> {
    use element::{add, scale};
    use kernel_api::{ConstructionShape as S, FittedShape as F};
    let name = e.name.clone();
    let unit = |a: [f64; 3]| scale(a, 1.0 / element::dot(a, a).sqrt().max(1e-12));
    let axis = |name: &str, at: [f64; 3], a: [f64; 3]| {
        (
            format!("{name} axis"),
            S::Axis {
                origin: at,
                z: unit(a),
                x: element::basis(a).0,
            },
        )
    };
    let span = match e.result.span {
        element::Span::Axis(t) => Some(t),
        _ => None,
    };
    match (e.result.shape.clone(), e.result.span) {
        (F::Plane { origin, normal }, element::Span::Patch { u, across }) => vec![(
            name,
            S::Plane {
                origin,
                normal: unit(normal),
                x_axis: u,
                u: across[0],
                v: across[1],
            },
        )],
        (F::Plane { origin, normal }, _) => vec![(
            name,
            S::Plane {
                origin,
                normal: unit(normal),
                x_axis: element::basis(normal).0,
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
            let a = unit(a);
            let [t0, t1] = span.unwrap_or([0.0, 0.0]);
            let mut out = vec![axis(&name, add(point, scale(a, t0)), a)];
            for (end, t) in [("start", t0), ("end", t1)] {
                out.push((
                    format!("{name} {end}"),
                    S::Circle {
                        centre: add(point, scale(a, t)),
                        normal: a,
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
            let a = unit(a);
            let mut out = vec![axis(&name, apex, a)];
            if let Some([t0, t1]) = span {
                for (end, t) in [("start", t0), ("end", t1)] {
                    let radius = (t * half_angle.tan()).abs();
                    if radius > 0.0 {
                        out.push((
                            format!("{name} {end}"),
                            S::Circle {
                                centre: add(apex, scale(a, t)),
                                normal: a,
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
            axis(&name, centre, a),
            (
                format!("{name} spine"),
                S::Circle {
                    centre,
                    normal: unit(a),
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
                normal: unit(normal),
                radius,
            },
        )],
        (F::Line { point, direction }, _) => {
            let d = unit(direction);
            let [t0, t1] = span.unwrap_or([-10.0, 10.0]);
            vec![(
                name,
                S::Polyline {
                    points: vec![add(point, scale(d, t0)), add(point, scale(d, t1))],
                    closed: false,
                },
            )]
        }
    }
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

/// The next free number for an element named `word` on `scan`, fitted and
/// constructed counted together so no two share a name.
fn next_number(document: &Document, scan: BodyId, word: &str) -> usize {
    document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.body == Some(scan))
        .filter(|(_, n)| match n.workbench_id.as_str() {
            element::KIND => {
                Element::from_json(&n.data).is_ok_and(|e| element::kind_word(e.kind) == word)
            }
            construction::KIND => {
                Construction::from_json(&n.data).is_ok_and(|c| c.made.word() == word)
            }
            _ => false,
        })
        .count()
        + 1
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
    let number = next_number(ctx.document, scan, made.word());
    let c = Construction {
        name: format!("{} {number}", made.word()),
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
    let dim = Dimension::from_json(document.feature_values(id)?).ok()?;
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
    let number = ctx
        .document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == dimension::KIND && n.body == Some(scan))
        .filter(|(_, n)| Dimension::from_json(&n.data).is_ok_and(|d| d.kind == kind))
        .count()
        + 1;
    let name = format!("{} {number}", kind.word());
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
                let Some(s) = document
                    .feature_values(*id)
                    .and_then(|d| shadow::Shadow::from_json(d).ok())
                else {
                    continue;
                };
                let Some(made) = s.made.as_ref().filter(|_| only.is_none_or(|b| b == s.scan))
                else {
                    continue;
                };
                let p = made.plane;
                for (k, wire) in made.outline.iter().enumerate() {
                    let points = shadow::samples(wire)
                        .map(|[u, v]| {
                            element::add(
                                p.origin,
                                element::add(
                                    element::scale(p.x_axis, u),
                                    element::scale(p.y_axis, v),
                                ),
                            )
                        })
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
        let mut nodes: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.body == Some(scan) && n.visible)
            .filter(|(_, n)| {
                matches!(
                    n.workbench_id.as_str(),
                    compare::KIND | element_map::KIND | thickness::KIND
                )
            })
            .collect();
        let rank = |kind: &str| match kind {
            compare::KIND => 0,
            element_map::KIND => 1,
            _ => 2,
        };
        nodes.sort_by_key(|(id, n)| {
            (
                rank(n.workbench_id.as_str()),
                std::cmp::Reverse(n.seq),
                **id,
            )
        });
        let (id, node) = nodes.first()?;
        let data = document.feature_values(**id)?;
        let key = match node.workbench_id.as_str() {
            compare::KIND => {
                compare::inputs(document, &Comparison::from_json(data).ok()?)
                    .ok()?
                    .key
            }
            element_map::KIND => {
                element_map_inputs(document, &element_map::ElementMap::from_json(data).ok()?)?.2
            }
            _ => {
                thickness::inputs(document, &thickness::WallThickness::from_json(data).ok()?)
                    .ok()?
                    .key
            }
        };
        Some((**id, self.maps.values(**id, key)?))
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
                let at = ctx.hovered_face.as_ref()?.point.map(f64::from);
                let scan = ctx.document.body_scan(body).is_some();
                self.picks.pick_pair_point(body, scan, at);
                Some(InputResult::consumed())
            }
            WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Enter,
            } => {
                let Some(args) = self.picks.align_args() else {
                    ctx.log_warn(format!(
                        "align by points takes {} pairs on one scan; {} so far",
                        picks::PAIRS_NEEDED,
                        self.picks.pairs()
                    ));
                    return Some(InputResult::consumed());
                };
                match align_commands::run(align_commands::POINTS, &args, ctx) {
                    Ok(answer) => {
                        ctx.record(align_commands::POINTS, args, answer);
                        self.picks.clear();
                    }
                    Err(why) => ctx.log_warn(why.to_string()),
                }
                Some(InputResult::consumed())
            }
            _ => None,
        }
    }

    fn armed_status(&self, _ctx: &WorkbenchRuntimeContext) -> Option<core_document::StatusItems> {
        let kind = self.armed?;
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
        context.register_command(
            CommandSpec::new(SECTION, "Cut a scan by a plane, or a stack of planes")
                .optional(
                    "element",
                    ParamKind::Id,
                    "Cut across this plane, axis or circle",
                )
                .optional("scan", ParamKind::Id, "Without an element: the scan to cut")
                .optional(
                    "plane",
                    ParamKind::String,
                    "Without an element: xy, xz or yz through the scan's middle; xy",
                )
                .optional(
                    "offset",
                    ParamKind::Number,
                    "Move the first cut along the normal, mm; 0",
                )
                .optional("count", ParamKind::Number, "How many cuts; 1")
                .optional("step", ParamKind::Number, "Between cuts, mm; 5")
                .optional(
                    "through",
                    ParamKind::Bool,
                    "A half-section containing the element's axis",
                )
                .optional(
                    "angle",
                    ParamKind::Number,
                    "Of a half-section about the axis, degrees; 0",
                )
                .returns("{section, name, cuts = {{offset, chains, closed, length, area}}, tracks = {{cuts, from, to, tilt, draft, radius = {first, last}, roundness}}}")
                .note(
                    "The plane is kept in the scan's frame as it is when made: aligning the scan \
                     carries it, refitting the element does not move it. Chains shorter than \
                     1 mm are dropped; area is the material inside the closed chains.",
                ),
        );
        context.register_tool(
            ToolDescriptor::new_action(SHADOW, "Shadow outline", Some("section")).icon("cut"),
        );
        context.register_command(
            CommandSpec::new(
                SHADOW,
                "Outline a scan's shadow clear of it, and stand it up as a block",
            )
            .optional(
                "element",
                ParamKind::Id,
                "Look along this plane's normal or this axis",
            )
            .optional("scan", ParamKind::Id, "Without an element: the scan")
            .optional(
                "plane",
                ParamKind::String,
                "Without an element: look along the normal of xy, xz or yz; xy",
            )
            .optional(
                "play",
                ParamKind::Number,
                "The least clearance, mm; 1, 0 exact, negative a press fit",
            )
            .optional(
                "tolerance",
                ParamKind::Number,
                "Half the band the outline keeps to, mm; 0.1",
            )
            .optional(
                "round_below",
                ParamKind::Number,
                "Round corners turning less than this, degrees; 30",
            )
            .optional(
                "fill_below",
                ParamKind::Number,
                "Fill holes and notches no wider than this, mm, rounding inner corners to \
                 half of it; 0, none",
            )
            .optional(
                "block",
                ParamKind::Bool,
                "Stand the outline up as a block; true",
            )
            .optional(
                "height",
                ParamKind::Number,
                "The block's height, mm; the part's thickness",
            )
            .returns("{shadow, body, name, loops, area, band = {least, largest}, depth, error}")
            .note(
                "The outline is lines, arcs and circles, every point of it between play and \
                     play + 2 × tolerance outside the shadow; holes through the part stay holes. \
                     It lives on a body of its own whose solid is the block, and follows the scan \
                     when the scan is moved or its settings change.",
            ),
        );
        context.register_tool(
            ToolDescriptor::new_action(PIN, "Pin reading", Some("compare")).icon("datum-point"),
        );
        context.register_command(
            CommandSpec::new(PIN, "Pin a map's reading on the part")
                .param("scan", ParamKind::Id, "The scan the map is shown on")
                .param(
                    "at",
                    ParamKind::Any,
                    "{x, y, z}: where, in the scan's own frame",
                )
                .returns("{pin, name, value}")
                .note(
                    "The pin reads the map shown on the scan at its nearest point, again \
                       whenever the map changes; its label shows on the part.",
                ),
        );
        context.register_tool(
            ToolDescriptor::new_action(ELEMENT_MAP, "Map against element", Some("compare"))
                .icon("constraint-distance"),
        );
        context.register_command(
            CommandSpec::new(
                ELEMENT_MAP,
                "Map a scan's distance from one of its elements",
            )
            .param(
                "element",
                ParamKind::Id,
                "A plane, cylinder, cone or sphere",
            )
            .optional(
                "tolerance",
                ParamKind::Number,
                "± in mm, counted in the legend",
            )
            .optional("flip", ParamKind::Bool, "Count the other side positive")
            .optional(
                "facing",
                ParamKind::Number,
                "Leave points turned further than this many degrees from the shape unmeasured; 60",
            )
            .returns("{map, name, measured, mean, rms, min, max}")
            .note(
                "Every scan point within the element's drawn extent is measured to its ideal \
                     shape, positive where the material faces out as the scan's normals say: is \
                     this face flat, is this bore round. No reference or alignment is needed.",
            ),
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
        context.register_command(
            CommandSpec::new(COMPENSATION, "How a print came out per axis, and what scales it back")
                .param("scan", ParamKind::Id, "The scanned print, aligned onto its model (Best fit)")
                .param("reference", ParamKind::Id, "The body it was printed from")
                .optional("tie_xy", ParamKind::Bool, "One stretch for X and Y together, Z its own")
                .returns("{stretch = {x, y, z}, compensation = {x, y, z}, shift = {x, y, z}, holes, bosses, flare, warp, rms_before, rms_after, used, points}")
                .read_only()
                .note(
                    "Each scan point's distance to the model along its normal is explained as each \
                     axis stretching about the model's middle and a leftover shift, by least \
                     squares; points past three times the fit's RMS (support remains, defects) \
                     are left out. `stretch` is each axis's stretch less one in percent (-0.4 a \
                     print 0.4 % small); `compensation` the factors that scale the model so the \
                     next print comes out as drawn.",
                )
                .note(
                    "The model is read as it stood on the bed, +Z up. Scan points on the walls \
                     of the model's round holes and bosses add each kind's radius error beyond \
                     the stretch: `holes` and `bosses` are {radius_error, offset, points}, nil \
                     with too few points, `offset` the radius change to draw them with (a hole \
                     printing 0.1 mm small in radius reads offset 0.1). The side walls of the \
                     first 0.6 mm are left out of the fit; `flare` is how far they stand out \
                     past it (elephant's foot). `warp` reads the down-facing points within 3 mm \
                     of the bottom: {flatness, lift, at}, the bottom's peak-to-peak and how \
                     high its highest part stands over its middle height, and where.",
                )
                .see_also("inspect.best_fit"),
        );
        context.register_command(
            CommandSpec::new(
                EXPORT_STEP,
                "Write the elements as STEP construction geometry",
            )
            .param("path", ParamKind::String, "The file to write")
            .optional("scan", ParamKind::Id, "Only this scan's")
            .returns("{path, items}")
            .agent_always_asks()
            .note(
                "One part holding no solid, its construction geometry named as the tree \
                     names it: fitted and constructed planes as bounded planes, cylinders, cones \
                     and tori as their axes and rims, spheres and points as points, circles, \
                     lines, every section's chains and shadow outlines as curves, where the scan \
                     stands.",
            ),
        );
        context.register_command(
            CommandSpec::new(
                EXPORT_IGES,
                "Write the elements as IGES construction geometry",
            )
            .param("path", ParamKind::String, "The file to write")
            .optional("scan", ParamKind::Id, "Only this scan's")
            .returns("{path, items, warnings}")
            .agent_always_asks()
            .note(
                "Writes the same geometry as inspect.export_step, at the scan's placement, \
                 with the IGES construction flag (06) and no solid. `items` counts the items \
                 submitted to the writer; `warnings` lists any geometry left out or names \
                 carried only in part. Names containing non-ASCII characters, commas or \
                 semicolons keep only a short label.",
            )
            .see_also(EXPORT_STEP)
            .example(
                "A section saved as IGES with its naming warning",
                r#"
                    local drum = pc.doc.new_body{}
                    pc.design.primitive{body = drum, variant = "cylinder",
                      kind = {Cylinder = {radius = 5, height = 10, angle_deg = 360}}}
                    assert(#pc.doc.rebuild() == 0)
                    local path = os.tmpname() .. ".stl"
                    pc.file.export{path = path, bodies = {drum}}
                    pc.file.import{path = path, scan = true}
                    pc.doc.rebuild()
                    os.remove(path)
                    local scan
                    for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
                    local cut = pc.inspect.section{scan = scan}
                    pc.doc.rename{id = cut.section, name = "Section, centre"}
                    local out = os.tmpname() .. ".igs"
                    local saved = pc.inspect.export_iges{path = out, scan = scan}
                    os.remove(out)
                    assert(saved.path == out and saved.items == 1)
                    assert(#saved.warnings == 1)
                    assert(saved.warnings[1]:find("Section, centre", 1, true))
                "#,
            ),
        );
        context.register_tool(
            ToolDescriptor::new_action(TOLERANCE, "Tolerance", Some("dimension"))
                .icon("constraint-perpendicular"),
        );
        context.register_command(
            CommandSpec::new(
                TOLERANCE,
                "Keep a geometric tolerance on an element",
            )
            .param(
                "characteristic",
                ParamKind::String,
                "flatness, straightness, circularity, cylindricity, sphericity, \
                     parallelism, perpendicularity, angularity, coaxiality, concentricity, position, \
                     surface_profile, line_profile, circular_runout, total_runout or symmetry",
            )
            .param("element", ParamKind::Id, "The element toleranced")
            .optional("datum", ParamKind::Id, "The element it is taken against")
            .param(
                "limit",
                ParamKind::Number,
                "The widest zone that passes, mm",
            )
            .optional(
                "angle",
                ParamKind::Number,
                "Angularity's basic angle, degrees",
            )
            .optional("tolerance", ParamKind::Id, "Replace this tolerance rather than creating one")
            .optional("datums", ParamKind::List, "Ordered {element, modifier = mmc or lmc, size} references, up to three; legacy datum stays primary")
            .optional("basic", ParamKind::List, "True position {x, y, z} in the datum frame, mm")
            .optional("basic_direction", ParamKind::List, "Zone axis or plane normal in the datum frame; {0, 0, 1}")
            .optional("material", ParamKind::String, "mmc or lmc on position")
            .optional("material_size", ParamKind::Number, "Material boundary size in mm; otherwise the diameter dimension supplies it")
            .optional("unequal", ParamKind::Number, "Unequally disposed profile zone: signed offset of its centre, mm")
            .optional("section", ParamKind::Id, "Section whose chains a line profile measures")
            .optional("comparison", ParamKind::Id, "Comparison supplying the nominal reference body")
            .optional("reference_face", ParamKind::Integer, "Face index in the reference supplying the basic nominal (plane, cylinder or sphere)")
            .optional("opposed", ParamKind::Id, "Other plane of a slot or symmetry measurement")
            .optional("section_width", ParamKind::Number, "Circular runout station-band width in mm")
            .returns("{tolerance, name, characteristic, element, datum, datums, limit, value, bonus, allowed, datum_shift, pass, note}")
            .note(
                "A form tolerance reads the element's own band: a plane's flatness, a \
                     line's straightness, a circle's circularity, a cylinder's cylindricity, a \
                     sphere's sphericity. An orientation tolerance reads a plane at its measured \
                     points (a constructed plane at its drawn corners, said in `note`), an axis \
                     over its fitted length and a centre as a point, against the datum's \
                     attitude. Form uses the fitted minimum zone when available. Position reads \
                     axis ends or a centre in a cylindrical zone (diameter), a plane in two \
                     parallel planes. The primary datum sets Z, the next transverse datum X. \
                     Basic offsets and nominal faces use that frame. Profiles with datums hold \
                     the scan in this frame against a reference in matching coordinates; without \
                     datums they retain the current best-fit placement. The profile limit is the \
                     full zone width. Circular runout reads sampled station bands; total reads \
                     the whole region, removing a cone's nominal slope. Position bonus is the \
                     departure of the fitted size from material_size; use an inscribed fit for \
                     holes or circumscribed for bosses. Datum shifts are reported, never applied.",
            )
            .example("Datum position, profiles, runout and a median plane", r#"
                local model = pc.doc.new_body{}
                pc.design.primitive{body = model, variant = "cylinder",
                  kind = {Cylinder = {radius = 5, height = 10, angle_deg = 360}}}
                assert(#pc.doc.rebuild() == 0)
                local path = os.tmpname() .. ".stl"
                pc.file.export{path = path, bodies = {model}}
                pc.doc.set_visible{id = model, visible = false}
                pc.file.import{path = path, scan = true}
                pc.doc.rebuild()
                os.remove(path)
                local scan
                for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
                local axis = pc.inspect.grow{kind = "cylinder", scan = scan, at = {5,0,5}, method = "circumscribed"}.element
                local bottom = pc.inspect.grow{kind = "plane", scan = scan, at = {0,0,0}}.element
                local top = pc.inspect.grow{kind = "plane", scan = scan, at = {0,0,10}}.element
                local middle = pc.inspect.construct{made = "plane", from = {bottom,top}}.element
                local comparison = pc.inspect.compare{scan = scan, reference = model}.comparison
                local section = pc.inspect.section{scan = scan}.section
                local position = pc.inspect.tolerance{characteristic = "position", element = axis,
                  datums = {{element = bottom}}, basic = {0,0,0}, limit = 0.05,
                  material = "mmc", material_size = 10.2}
                assert(position.pass and position.value < 0.01 and position.bonus > 0.19)
                for _, kind in ipairs({"surface_profile", "line_profile"}) do
                  local reading = pc.inspect.tolerance{characteristic = kind, element = axis,
                    comparison = comparison, section = section, limit = 0.1, unequal = 0.01}
                  assert(reading.pass and reading.value < 0.1)
                end
                for _, kind in ipairs({"circular_runout", "total_runout"}) do
                  local reading = pc.inspect.tolerance{characteristic = kind, element = axis,
                    datum = axis, limit = 0.05, section_width = 0.1}
                  assert(reading.pass and reading.value < 0.05)
                end
                local symmetry = pc.inspect.tolerance{characteristic = "symmetry", element = bottom,
                  opposed = top, datum = middle, limit = 0.05}
                assert(symmetry.pass and symmetry.value < 0.01)
                assert(#pc.inspect.report{}.tolerances == 6)
            "#),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                THICKNESS,
                "Wall thickness",
                Some(core_document::INSPECT_CATEGORY),
            )
            .icon("thickness"),
        );
        context.register_command(
            CommandSpec::new(THICKNESS, "Map how thin or thick a body's walls are")
                .param("body", ParamKind::Id, "Any body with geometry: a scan or a solid")
                .optional("sphere", ParamKind::Bool, "The largest ball that fits, in place of rays")
                .optional("min_wall", ParamKind::Number, "Count the walls thinner than this, mm")
                .optional("max", ParamKind::Number, "Walls thicker than this read unmeasured; a fifth of the body's diagonal when left out")
                .returns("{thickness, name, measured, unmeasured, mean, min, max, range, thinner}")
                .note(
                    "Read inward from every vertex of the body's mesh, along its normal; a \
                     solid is read from its exact surfaces. Surfaces within 60° of edge-on to a \
                     ray are passed over. The map paints thin walls hot while the Inspection \
                     bench is active.",
                ),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                EXTRACT,
                "Element as body",
                Some(core_document::INSPECT_CATEGORY),
            )
            .icon("body"),
        );
        context.register_command(
            CommandSpec::new(
                ELEMENT_SOLID,
                "The solid primitive a fitted element stands for",
            )
            .param(
                "element",
                ParamKind::Id,
                "A fitted plane, cylinder, cone, sphere or torus",
            )
            .optional(
                "thickness",
                ParamKind::Number,
                "A plane's slab, under its fitted patch, mm; 2",
            )
            .returns(
                "{kind, placement, name}: what pc.design.primitive takes, in the document's frame",
            )
            .read_only()
            .note(
                "A cylinder or cone covers the length its points cover along its axis, a \
                     plane becomes a slab over the patch its points cover, sunk under it \
                     (against its normal), and a sphere or torus is whole. Hand `kind` and \
                     `placement` to pc.design.primitive in a new body to model from it; the \
                     Element as body tool does both. A circle or line has no solid.",
            )
            .see_also("design.primitive")
            .example(
                "A scanned cylinder fitted and made a body again",
                r#"
                    local drum = pc.doc.new_body{}
                    pc.design.primitive{body = drum, variant = "cylinder",
                      kind = {Cylinder = {radius = 5, height = 10, angle_deg = 360}}}
                    assert(#pc.doc.rebuild() == 0)
                    local path = os.tmpname() .. ".stl"
                    pc.file.export{path = path, bodies = {drum}}
                    pc.doc.set_visible{id = drum, visible = false}
                    pc.file.import{path = path, scan = true}
                    pc.doc.rebuild()
                    os.remove(path)
                    local scan
                    for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
                    local fit = pc.inspect.grow{kind = "cylinder", scan = scan, at = {5, 0, 5}}
                    local s = pc.inspect.element_solid{element = fit.element}
                    local copy = pc.doc.new_body{name = s.name}
                    pc.design.primitive{body = copy, kind = s.kind, placement = s.placement}
                    assert(#pc.doc.rebuild() == 0)
                    local v = pc.doc.measure{body = copy}.volume
                    assert(math.abs(v - math.pi * 25 * 10) < 0.02 * math.pi * 25 * 10, v)
                    "#,
            ),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                CLEARANCE,
                "Fit check",
                Some(core_document::INSPECT_CATEGORY),
            )
            .icon("thickness"),
        );
        context.register_command(
            CommandSpec::new(CLEARANCE, "Map a part's room in what it fits into")
                .param("part", ParamKind::Id, "The part the map paints: a scan or a solid")
                .param("housing", ParamKind::Id, "What it fits into: a solid or a scan")
                .optional("min_gap", ParamKind::Number, "Count the points with less room than this, mm")
                .optional("reach", ParamKind::Number, "Points farther than this from the housing read unmeasured, mm; 5 when left out")
                .returns("{clearance, name, measured, unmeasured, interfering, deepest, narrowest, too_tight}")
                .note(
                    "The two are measured as they sit now: place the part in the housing first \
                     (an alignment, the Assembly's joints or a placement). Each of the part's \
                     points reads its distance to the housing's surface, positive across a gap \
                     and negative inside the housing's material. `interfering` is the share of \
                     measured points inside it and `deepest` how far, `narrowest` the smallest \
                     gap when nothing interferes, and `too_tight` the share with less room than \
                     `min_gap`. The map paints interference red while the Inspection bench is \
                     active.",
                )
                .example(
                    "A 10 mm cube in a 12 mm square pocket has 1 mm all round; moved 1.5 mm, \
                     it presses 0.5 mm into one wall.",
                    "local box = pc.doc.new_body{}\n\
                     pc.design.primitive{body = box, kind = {Box = {length = 30, width = 30, height = 10}}}\n\
                     pc.design.subtractive_primitive{body = box, kind = {Box = {length = 12, width = 12, height = 20}},\n\
                       placement = {origin = {9, 9, 2}, x_axis = {1, 0, 0}, z_axis = {0, 0, 1}}}\n\
                     local part = pc.doc.new_body{}\n\
                     pc.design.primitive{body = part, kind = {Box = {length = 10, width = 10, height = 10}}}\n\
                     assert(#pc.doc.rebuild() == 0)\n\
                     pc.doc.set_body{body = part, translation = {10, 10, 4}}\n\
                     local fit = pc.inspect.clearance{part = part, housing = box, min_gap = 0.5}\n\
                     assert(fit.interfering == 0 and math.abs(fit.narrowest - 1) < 1e-6, fit.narrowest)\n\
                     pc.doc.set_body{body = part, translation = {11.5, 10, 4}}\n\
                     local tight = pc.inspect.clearance{part = part, housing = box}\n\
                     assert(tight.interfering > 0 and math.abs(tight.deepest - 0.5) < 1e-6, tight.deepest)",
                ),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                OVERHANG,
                "Overhangs",
                Some(core_document::INSPECT_CATEGORY),
            )
            .icon("thickness"),
        );
        context.register_command(
            CommandSpec::new(OVERHANG, "Map a body's overhangs as it would print")
                .param("body", ParamKind::Id, "Any body with geometry, standing as it would print (+Z up)")
                .optional("limit", ParamKind::Number, "The steepest overhang printed without support, degrees past vertical; 45 when left out")
                .returns("{overhang, name, limit, share, steepest}")
                .note(
                    "Each point of the mesh facing down reads how far it leans out past \
                     vertical: 0 a wall, 90 a ceiling. Surface facing up, and the face the body \
                     stands on at its lowest point, read unmeasured. `share` is the part of the \
                     body's area past the limit, which needs support; `steepest` the worst \
                     overhang, in degrees. The map paints while the Inspection bench is active.",
                )
                .example(
                    "A box standing on the bed needs no support; on its edge, half of it does.",
                    "local body = pc.doc.new_body{}\n\
                     pc.design.primitive{body = body}\n\
                     assert(#pc.doc.rebuild() == 0)\n\
                     local o = pc.inspect.overhang{body = body}\n\
                     assert(o.share == 0 and o.limit == 45, o.share)\n\
                     local q = math.pi / 8\n\
                     pc.doc.set_body{body = body, rotation = {x = math.sin(q), y = 0, z = 0, w = math.cos(q)}}\n\
                     local tilted = pc.inspect.overhang{body = body, limit = 40}\n\
                     assert(math.abs(tilted.steepest - 45) < 1e-3, tilted.steepest)",
                ),
        );
        context.register_command(
            CommandSpec::new(
                HOT_SPOTS,
                "Where a comparison is out of tolerance, as patches",
            )
            .param("comparison", ParamKind::Id, "The comparison")
            .optional(
                "tolerance",
                ParamKind::Number,
                "± in mm; the comparison's own when left out",
            )
            .optional(
                "least",
                ParamKind::Integer,
                "Leave out patches of fewer vertices; 3",
            )
            .returns(
                "a list, largest first, of {side, count, area, centre, min, max, mean, \
                     extreme, at, kind}",
            )
            .note(
                "Vertices beyond the tolerance are joined with neighbours on the same side \
                     across the mesh's edges. `side` is 1 outside the reference and -1 inside; \
                     `kind` is `feature` for a patch no more than 40 times as wide as it is deep, \
                     else `texture`. Positions are in the scan's frame.",
            )
            .read_only(),
        );
        context.register_command(
            CommandSpec::new(VOLUME_ERRORS, "Missing and excess material of a comparison")
                .param("comparison", ParamKind::Id, "The comparison; its reference a solid")
                .optional(
                    "cell",
                    ParamKind::Number,
                    "The grid's cell in mm; a two-hundredth of the scan's longest side",
                )
                .optional("off", ParamKind::Bool, "Stop reading them; false")
                .returns(
                    "{cell, regions}: a list, largest first, of {side, form, volume, depth, \
                     centre, min, max, unsure}",
                )
                .note(
                    "The scan is read as a solid on a grid of cubic cells and set against the \
                     reference. `side` is `missing` where the reference has material the scan \
                     lacks and `excess` where the scan has more; `form` is `layer` for a face \
                     standing off, `lump` for a boss or a pocket. `volume` is in mm³, `depth` in \
                     mm, how far apart the surfaces stand across the region. Holes in the scan \
                     read as the reference; a region on one, or shallower than two cells, is \
                     `unsure`, and one shallower than a cell is not reported. Positions are in \
                     the reference's frame. The comparison keeps the cell, and the largest \
                     regions are labelled on the part while the Inspection bench is active.",
                )
                .see_also(HOT_SPOTS)
                .example(
                    "A print two millimetres short of its part reads a missing layer",
                    r#"
                    local part = pc.doc.new_body{}
                    pc.design.primitive{body = part, kind = {Box = {length = 20, width = 20, height = 12}}}
                    local short = pc.doc.new_body{}
                    pc.design.primitive{body = short, kind = {Box = {length = 20, width = 20, height = 10}}}
                    assert(#pc.doc.rebuild() == 0)
                    local path = os.tmpname() .. ".stl"
                    pc.file.export{path = path, bodies = {short}}
                    pc.doc.set_visible{id = short, visible = false}
                    pc.file.import{path = path, scan = true}
                    pc.doc.rebuild()
                    os.remove(path)
                    local scan
                    for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
                    local c = pc.inspect.compare{scan = scan, reference = part}
                    local v = pc.inspect.volume_errors{comparison = c.comparison, cell = 0.5}
                    local r = v.regions[1]
                    assert(r.side == "missing" and r.form == "layer", r.side)
                    assert(math.abs(r.volume - 800) < 80 and math.abs(r.depth - 2) < 0.3, r.volume)
                    "#,
                ),
        );
        context.register_command(
            CommandSpec::new(MAP_EXPORT, "Write a comparison's distances to a file")
                .param("comparison", ParamKind::Id, "The comparison")
                .param("path", ParamKind::String, "Where to write")
                .optional(
                    "format",
                    ParamKind::String,
                    "csv, ply (binary, a deviation per vertex) or npy; from the path's \
                     extension when left out",
                )
                .returns("{path, points}")
                .note(
                    "Points are in the scan's own frame; an unmeasured point's distance is \
                       NaN, blank in CSV.",
                ),
        );
        context.register_command(
            CommandSpec::new(BEST_FIT, "Move a scan onto a reference body, best fit")
                .param("scan", ParamKind::Id, "The scan; only it moves")
                .param("reference", ParamKind::Id, "The body it should match")
                .optional(
                    "search",
                    ParamKind::Bool,
                    "Also try every way of matching the scan's principal axes with the \
                     reference's, for a scan nowhere near; true when left out",
                )
                .optional(
                    "marked",
                    ParamKind::Bool,
                    "A local fine fit: only the scan's marked part is fitted (a datum face, a \
                     region the part should sit on), the search off unless asked",
                )
                .returns("{alignment, name, translation, rotation, rms, inliers, free}")
                .note(
                    "Rigid and point to plane, pairs past three times the median distance left \
                     out. `free` counts the motions the fit could not pin down (a cylinder \
                     turning about its axis): above 0 the scan may sit anywhere along them. The \
                     scan's placement changes in one undoable step and an alignment row records \
                     how.",
                ),
        );
        context.register_command(
            CommandSpec::new(MOVE_COMMAND, "Move and turn a scan by numbers")
                .param("scan", ParamKind::Id, "The scan")
                .optional("translate", ParamKind::Any, "{x, y, z} in mm")
                .optional(
                    "rotate",
                    ParamKind::Any,
                    "{x, y, z} in degrees: about X, then Y, then Z, through the origin",
                )
                .returns("{alignment, name, translation, rotation}")
                .note("The motion is applied to where the scan sits now."),
        );
        context.register_command(
            CommandSpec::new(
                RESET_ALIGNMENT,
                "Put a scan back where the scanner placed it",
            )
            .param("scan", ParamKind::Id, "The scan")
            .returns("{alignment, name, translation, rotation}"),
        );
        context.register_tool(
            ToolDescriptor::new_action(COMPARE_TOOL, "Compare with reference", Some("compare"))
                .icon("measure"),
        );
        context.register_command(
            CommandSpec::new(
                COMPARE_COMMAND,
                "Compare a scan with a reference body, point by point",
            )
            .param("scan", ParamKind::Id, "The scan")
            .param("reference", ParamKind::Id, "The body it should be: a solid or a mesh")
            .optional("tolerance", ParamKind::Number, "± in mm: the share within it is counted")
            .optional("range", ParamKind::Number, "The colour scale's half-width in mm; worked out when left out")
            .optional("bands", ParamKind::Integer, "Steps of flat colour; 0, continuous, when left out")
            .optional("ramp", ParamKind::String, "jet, viridis, magma, inferno or plasma")
            .optional("max", ParamKind::Number, "Leave points farther than this from the reference unmeasured")
            .optional("facing", ParamKind::Number, "Measure only to reference surface within this many degrees of a point's own normal; 90 when left out")
            .returns("{comparison, name, scan, reference, range, measured, unmeasured, mean, rms, sigma, min, max, within}")
            .note(
                "A distance is positive where the scan stands outside the reference, negative \
                 inside. A solid reference is measured exactly as it is meshed for the purpose, \
                 finely; a mesh one as its triangles. Both are measured where they sit, so align \
                 the scan first. The map is drawn on the scan while the Inspection bench is \
                 active.",
            ),
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
        context.register_command(
            CommandSpec::new(
                CONSTRUCT_COMMAND,
                "Make a point, line, plane or circle from elements",
            )
            .param("made", ParamKind::String, "point, line, plane or circle")
            .param(
                "from",
                ParamKind::List,
                "The ids of the elements it is made from, fitted or constructed",
            )
            .optional(
                "offset",
                ParamKind::Number,
                "A plane made from one plane lies this far along its normal; 0 when left out",
            )
            .returns("{element, name, made, from, offset, shape}: the shape in the scan's frame")
            .note(
                "A point is two centres' midpoint or where an axis crosses a plane; a line runs \
                 through two centres or where two planes meet; a plane runs through three \
                 centres, midway between two planes or offset from one; a circle is where a \
                 plane cuts a sphere, or a cylinder within 5° of square. A sphere, circle or \
                 torus gives its centre, a cylinder, cone, circle or line its axis. The \
                 construction follows its sources when they are refitted, and dimensions take \
                 it as they take a fitted element.",
            ),
        );
        for kind in DimKind::ALL {
            context.register_tool(
                ToolDescriptor::new_action(dim_tool_id(kind), kind.word(), Some("dimension"))
                    .icon(kind.icon()),
            );
        }
        context.register_command(
            CommandSpec::new(
                FIT_COMMAND,
                "Fit a shape to the marked part of a scan and keep it as an element",
            )
            .param(
                "kind",
                ParamKind::String,
                "plane, sphere, cylinder, cone, torus, circle or line",
            )
            .optional(
                "cutoff",
                ParamKind::Number,
                "Leave out points farther than this many sigmas, fitting again until the \
                 set kept stops changing; 3 when left out, 0 keeps every point",
            )
            .optional(
                "method",
                ParamKind::String,
                "least_squares (the default), minimum_zone, inscribed or circumscribed",
            )
            .returns(
                "{element, name, kind, shape, sigma, form, zone, method, diameters, used, \
                 points}: in mm, the shape in the scan's own frame",
            )
            .note(
                "It fits what is marked (`pc.scan.mark`, or Mark surface… in the scan's \
                 menu), by orthogonal least squares. `sigma` is the RMS distance of the points \
                 used from the shape, `form` the width of the band they fill, `diameters` the \
                 smallest and largest a round shape reaches.",
            )
            .note(
                "Least squares picks the points used; `method` then fits those another way: \
                 `minimum_zone` to the middle of the narrowest zone holding them (a plane, \
                 line, circle, cylinder or sphere), `inscribed` to the largest circle or \
                 cylinder clear of them (a hole's mating size), `circumscribed` to the \
                 smallest round them (a boss's). `zone` is the minimum zone's width of the \
                 points used, {width, optimal}, the form a flatness, straightness, \
                 circularity, cylindricity or sphericity tolerance reads; `optimal` is false \
                 where the zone of a line, cylinder or sphere is only certified locally.",
            ),
        );
        context.register_command(
            CommandSpec::new(
                DIMENSION_COMMAND,
                "Measure a distance, angle or diameter between elements",
            )
            .param("kind", ParamKind::String, "distance, angle or diameter")
            .param(
                "elements",
                ParamKind::List,
                "The elements' ids: two for a distance or an angle, one for a diameter",
            )
            .optional(
                "nominal",
                ParamKind::Number,
                "What it should read; the nearest design value when left out",
            )
            .optional(
                "plus",
                ParamKind::Number,
                "How far above the nominal it may read",
            )
            .optional(
                "minus",
                ParamKind::Number,
                "How far below the nominal it may read",
            )
            .returns(
                "{dimension, name, kind, elements, nominal, plus, minus, value, deviation, \
                 deltas, range, pass}",
            )
            .note(
                "A distance is measured from the middle of the first element: between centres \
                 (spheres, circles) with `deltas` along X, Y and Z; from a centre to an axis \
                 or a plane; between near-parallel axes as their offset, otherwise where they \
                 come closest; between planes or an axis and a plane only within 3° of \
                 parallel, planes with the `range` their surfaces give. An angle between axes \
                 or an axis and a plane reads 0–90°, between planes' normals 0–180°. \
                 `pass` is there once `plus` or `minus` is above zero. A dimension reads its \
                 elements again whenever they change; `doc.set_value` sets its nominal, plus \
                 and minus.",
            ),
        );
        context.register_command(
            CommandSpec::new(
                GROW_COMMAND,
                "Fit a surface grown, or an edge traced, from a point on a scan",
            )
            .param(
                "kind",
                ParamKind::String,
                "plane, sphere, cylinder, cone or torus",
            )
            .param("scan", ParamKind::Id, "The scan")
            .param(
                "at",
                ParamKind::Any,
                "{x, y, z}: a point on or near the surface, in the scan's own frame",
            )
            .optional(
                "cutoff",
                ParamKind::Number,
                "Points farther than this many sigmas stop the growth and are left out; 3 \
                 when left out",
            )
            .optional(
                "more",
                ParamKind::List,
                "More {x, y, z} points in the scan's frame the region grows from as well \
                 (the clicks pinned with Shift before the last)",
            )
            .optional(
                "peel",
                ParamKind::Bool,
                "Take off the region's rim while it turns away (a face's rounds)",
            )
            .optional(
                "method",
                ParamKind::String,
                "least_squares (the default), minimum_zone, inscribed or circumscribed",
            )
            .returns("as inspect.fit")
            .note(
                "The region starts at the triangle nearest `at` and takes each neighbour whose \
                 points lie on the fitted surface and whose normal agrees with it, fitting \
                 again as it grows. A circle or a line is traced instead along the sharp edge \
                 nearest `at` (a hole's rim, a box's edge) until the edge turns away or meets \
                 another. It is refused where what is at `at` is not of the kind asked.",
            )
            .example(
                "A scanned bore's mating size sits inside its mean size",
                r#"
                    local drum = pc.doc.new_body{}
                    pc.design.primitive{body = drum, variant = "cylinder",
                      kind = {Cylinder = {radius = 5, height = 10, angle_deg = 360}}}
                    assert(#pc.doc.rebuild() == 0)
                    local path = os.tmpname() .. ".stl"
                    pc.file.export{path = path, bodies = {drum}}
                    pc.doc.set_visible{id = drum, visible = false}
                    pc.file.import{path = path, scan = true}
                    pc.doc.rebuild()
                    os.remove(path)
                    local scan
                    for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
                    local mean = pc.inspect.grow{kind = "cylinder", scan = scan, at = {5, 0, 5}}
                    local inside = pc.inspect.grow{kind = "cylinder", scan = scan,
                      at = {5, 0, 5}, method = "inscribed"}
                    assert(inside.method == "inscribed")
                    assert(inside.shape.Cylinder.radius <= mean.shape.Cylinder.radius + 1e-9)
                    assert(mean.zone.width <= mean.form + 1e-9, mean.zone.width)
                    "#,
            ),
        );
        context.register_command(
            CommandSpec::new(
                REPORT_COMMAND,
                "Every element and dimension of the inspection",
            )
            .optional("scan", ParamKind::Id, "Only this scan's")
            .optional(
                "format",
                ParamKind::String,
                "json (the answer as a table), csv or text",
            )
            .optional(
                "path",
                ParamKind::String,
                "Write it to this file; its extension picks the format when `format` is left out",
            )
            .returns(
                "{elements, constructions, dimensions, tolerances, passed, failed}, each as \
                 the command making it answers; CSV or text as a string; {path} when written",
            )
            .read_only(),
        );
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
            let s = shadow::Shadow::from_json(document.feature_values(id)?).ok()?;
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
            let idle = self.fitting.lock().is_ok_and(|f| f.is_none());
            return idle && compare_pair(ctx).is_some();
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
                ctx.marking.is_some()
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
        (tool_id == PIN && self.pinning)
            || (tool_id == align_commands::POINTS && self.picks.aligning)
            || (self.armed.is_some() && kind_of_tool(tool_id) == self.armed)
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
            self.tolerance_task = Some(tolerance_task::Task {
                id,
                opening: Some(t),
            });
        }
    }

    fn editing_feature(&self) -> Option<core_document::FeatureId> {
        self.tolerance_task.as_ref().map(|t| t.id)
    }

    fn suspend_session(&mut self) -> Option<Box<dyn std::any::Any + Send>> {
        self.tolerance_task
            .take()
            .map(|t| Box::new(t) as Box<dyn std::any::Any + Send>)
    }

    fn resume_session(&mut self, state: Option<Box<dyn std::any::Any + Send>>) {
        self.tolerance_task = state
            .and_then(|s| s.downcast::<tolerance_task::Task>().ok())
            .map(|s| *s);
    }

    fn task(&self, _ctx: &WorkbenchRuntimeContext) -> Option<core_document::TaskInfo> {
        self.tolerance_task.as_ref()?;
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
        let Some(task) = self.tolerance_task.as_ref() else {
            return TaskOutcome::Open;
        };
        let id = task.id;
        let Some(data) = ctx.document.get_feature_data(id) else {
            self.tolerance_task = None;
            return TaskOutcome::Cancelled;
        };
        let Ok(mut t) = tolerance::Tolerance::from_json(data) else {
            return TaskOutcome::Open;
        };
        if request.cancel {
            if let Some(opening) = &task.opening {
                let _ = ctx.document.update_feature_data(id, opening.to_json());
                ctx.document
                    .set_feature_dependencies(id, opening.dependencies());
            } else {
                let _ = ctx.document.remove_feature(id);
            }
            self.tolerance_task = None;
            return TaskOutcome::Cancelled;
        }
        let before = t.clone();
        tolerance_task::draw(ui, ctx.document, &mut t);
        if t != before {
            let _ = ctx.document.update_feature_data(id, t.to_json());
            ctx.document.set_feature_dependencies(id, t.dependencies());
        }
        let reading = self
            .tolerance_readings
            .read(ctx.document, id, &t, ctx.kernel, false);
        match &reading {
            Ok(e) => {
                ui.label(t.label(e, ctx.document.display_unit()));
                if let Some(note) = &e.note {
                    ui.label(note);
                }
            }
            Err(why) => {
                ui.label(why);
            }
        }
        if request.accept && reading.is_ok() {
            ctx.record(
                TOLERANCE,
                tolerance_task::args(&t, task.opening.as_ref().map(|_| id)),
                tolerance_answer(id, &t, &reading),
            );
            self.tolerance_task = None;
            return TaskOutcome::Accepted {
                label: "Geometric tolerance".into(),
            };
        }
        TaskOutcome::Open
    }

    fn busy(&self) -> bool {
        self.maps.busy()
            || self.tolerance_readings.busy()
            || self.fitting.lock().is_ok_and(|f| f.is_some())
    }

    fn on_frame(&mut self, _dt: f32, ctx: &mut WorkbenchRuntimeContext) {
        if let Some(kernel) = ctx.kernel {
            let _ = self.kernel.set(kernel);
        }
        let done = {
            let Ok(mut fitting) = self.fitting.lock() else {
                return;
            };
            if fitting.as_ref().is_some_and(|f| f.answer.is_some()) {
                fitting.take()
            } else {
                None
            }
        };
        let Some(Fitting {
            scan,
            reference,
            search,
            answer: Some(answer),
        }) = done
        else {
            return;
        };
        match answer {
            Ok((after, fit)) => {
                match record_alignment(ctx, scan, best_fit_how(reference, search, &fit), after) {
                    Ok(answer) => {
                        let mut args = CommandArgs::new();
                        args.insert("scan".into(), json!(scan.0.to_string()));
                        args.insert("reference".into(), json!(reference.0.to_string()));
                        ctx.record(BEST_FIT, args, answer);
                    }
                    Err(why) => ctx.log_warn(why),
                }
            }
            Err(why) => ctx.log_warn(format!("Best fit: {why}")),
        }
    }

    fn status_items(&self, ctx: &WorkbenchRuntimeContext) -> Option<core_document::StatusItems> {
        if self.fitting.lock().is_ok_and(|f| f.is_some()) {
            return Some(core_document::StatusItems {
                mode: Some("Fitting the scan onto its reference…".to_string()),
                ..Default::default()
            });
        }
        if self.picks.aligning {
            let n = self.picks.pairs();
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
            &self.tolerance_readings,
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
            let s = section::Section::from_json(document.feature_values(id)?).ok()?;
            let len = element::dot(s.normal, s.normal).sqrt();
            let shift = element::scale(s.normal, s.offset / len.max(1e-12));
            (s.scan, element::add(s.origin, shift), s.normal)
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
        let len = element::dot(normal, normal).sqrt().max(1e-12);
        let n = element::scale(normal, 1.0 / len);
        let (x, y) = element::basis(n);
        Some(kernel_api::ProfilePlane {
            origin: place(origin, 1.0),
            normal: place(n, 0.0),
            x_axis: place(x, 0.0),
            y_axis: place(y, 0.0),
        })
    }

    fn faded_bodies(&self, ctx: &WorkbenchRuntimeContext) -> Vec<BodyId> {
        let document = &*ctx.document;
        document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == compare::KIND && n.visible)
            .filter_map(|(id, _)| {
                Comparison::from_json(document.feature_values(*id)?)
                    .ok()
                    .map(|c| c.reference)
            })
            .collect()
    }

    fn scalar_maps(&self, ctx: &WorkbenchRuntimeContext) -> Vec<core_document::ScalarMap> {
        let document = &*ctx.document;
        let Some(kernel) = ctx.kernel else {
            return Vec::new();
        };
        let _ = self.kernel.set(kernel);
        let mut comparisons: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == compare::KIND && n.visible)
            .collect();
        comparisons.sort_by_key(|(id, n)| (std::cmp::Reverse(n.seq), **id));
        let mut maps = Vec::new();
        for (id, node) in comparisons {
            let Some(c) = document
                .feature_values(*id)
                .and_then(|d| Comparison::from_json(d).ok())
            else {
                continue;
            };
            if maps
                .iter()
                .any(|m: &core_document::ScalarMap| m.body == c.scan)
                || !document.imported_body_effective_visible(c.scan)
            {
                continue;
            }
            let Ok(inputs) = compare::inputs(document, &c) else {
                continue;
            };
            let key = inputs.key;
            let Some(values) = self.maps.values(*id, key) else {
                self.maps.request(*id, inputs, c, kernel);
                continue;
            };
            let Some(r) = self.maps.reading(*id, key, &c) else {
                continue;
            };
            let ramp = c.ramp();
            let (under, over) = ramp.caps();
            let range = r.range as f32;
            let mut stats = Vec::new();
            if let Some(s) = r.summary {
                for (name, v) in [
                    ("Mean", s.mean),
                    ("RMS", s.rms),
                    ("σ", s.sigma),
                    ("Min", s.min),
                    ("Max", s.max),
                ] {
                    stats.push((
                        name.to_string(),
                        show_mm_signed(v, ctx.document.display_unit()),
                    ));
                }
                if s.unmeasured > 0 {
                    stats.push(("Unmeasured".to_string(), s.unmeasured.to_string()));
                }
            }
            if let (Some(t), Some(w)) = (c.tolerance, r.within) {
                stats.push((format!("Within ±{t}"), format!("{:.1} %", w * 100.0)));
            }
            maps.push(core_document::ScalarMap {
                body: c.scan,
                revision: key ^ id.0.as_u64_pair().0,
                values,
                range: [-range, range],
                bands: c.bands,
                ramp: (0..32).map(|i| ramp.at(f64::from(i) / 31.0)).collect(),
                under,
                over,
                none: ctx.sketch_palette.inactive,
                legend: core_document::MapLegend {
                    title: format!("{} · deviation", node.name),
                    unit: "mm".to_string(),
                    stats,
                    histogram: r.histogram.counts.iter().map(|n| *n as u32).collect(),
                    tolerance: c.tolerance.map(|t| (-t as f32, t as f32)),
                },
            });
        }
        let mut against: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == element_map::KIND && n.visible)
            .collect();
        against.sort_by_key(|(id, n)| (std::cmp::Reverse(n.seq), **id));
        for (id, node) in against {
            let Some(m) = document
                .feature_values(*id)
                .and_then(|d| element_map::ElementMap::from_json(d).ok())
            else {
                continue;
            };
            if maps
                .iter()
                .any(|x: &core_document::ScalarMap| x.body == m.scan)
                || !document.imported_body_effective_visible(m.scan)
            {
                continue;
            }
            let Some((mesh, e, key)) = element_map_inputs(document, &m) else {
                continue;
            };
            let Some(values) = self.maps.values(*id, key) else {
                let (id, job) = (*id, m.clone());
                self.maps.run(id, key, move || {
                    Ok(element_map::measure(&mesh, &e, &job).into())
                });
                continue;
            };
            let as_comparison = Comparison {
                name: m.name.clone(),
                scan: m.scan,
                reference: m.scan,
                range: None,
                bands: 0,
                ramp: metrology::ramp::Ramp::Jet.name().to_string(),
                tolerance: m.tolerance,
                max: None,
                facing_limit_deg: None,
                volume_cell: None,
            };
            let r = compare::read(&values, &as_comparison);
            let ramp = metrology::ramp::Ramp::Jet;
            let (under, over) = ramp.caps();
            let range = r.range as f32;
            let mut stats = Vec::new();
            if let Some(s) = r.summary {
                for (name, v) in [
                    ("Mean", s.mean),
                    ("RMS", s.rms),
                    ("Min", s.min),
                    ("Max", s.max),
                ] {
                    stats.push((
                        name.to_string(),
                        show_mm_signed(v, ctx.document.display_unit()),
                    ));
                }
            }
            if let (Some(t), Some(w)) = (m.tolerance, r.within) {
                stats.push((format!("Within ±{t}"), format!("{:.1} %", w * 100.0)));
            }
            maps.push(core_document::ScalarMap {
                body: m.scan,
                revision: key ^ id.0.as_u64_pair().0,
                values,
                range: [-range, range],
                bands: 0,
                ramp: (0..32).map(|i| ramp.at(f64::from(i) / 31.0)).collect(),
                under,
                over,
                none: ctx.sketch_palette.inactive,
                legend: core_document::MapLegend {
                    title: format!("{} · from {}", node.name, e.name),
                    unit: "mm".to_string(),
                    stats,
                    histogram: r.histogram.counts.iter().map(|n| *n as u32).collect(),
                    tolerance: m.tolerance.map(|t| (-t as f32, t as f32)),
                },
            });
        }
        let mut walls: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == thickness::KIND && n.visible)
            .collect();
        walls.sort_by_key(|(id, n)| (std::cmp::Reverse(n.seq), **id));
        for (id, node) in walls {
            let Some(t) = document
                .feature_values(*id)
                .and_then(|d| thickness::WallThickness::from_json(d).ok())
            else {
                continue;
            };
            if maps
                .iter()
                .any(|m: &core_document::ScalarMap| m.body == t.body)
                || !document.imported_body_effective_visible(t.body)
            {
                continue;
            }
            let Ok(inputs) = thickness::inputs(document, &t) else {
                continue;
            };
            let key = inputs.key;
            let Some(values) = self.maps.values(*id, key) else {
                let (id, job) = (*id, t.clone());
                self.maps
                    .run(id, key, move || thickness::measure(kernel, &inputs, &job));
                continue;
            };
            let r = thickness::read(&values, &t);
            let ramp = metrology::ramp::Ramp::Jet;
            // Thin walls hot: the ramp runs red at the thin end.
            let (thick_cap, thin_cap) = ramp.caps();
            let mut stats = Vec::new();
            if let Some(s) = r.summary {
                for (name, v) in [("Mean", s.mean), ("Thinnest", s.min), ("Thickest", s.max)] {
                    stats.push((name.to_string(), show_mm(v, ctx.document.display_unit())));
                }
                if s.unmeasured > 0 {
                    stats.push(("Unmeasured".to_string(), s.unmeasured.to_string()));
                }
            }
            if let (Some(m), Some(share)) = (t.min_wall, r.thinner) {
                stats.push((
                    format!("Thinner than {m}"),
                    format!("{:.1} %", share * 100.0),
                ));
            }
            maps.push(core_document::ScalarMap {
                body: t.body,
                revision: key ^ id.0.as_u64_pair().1,
                values,
                range: [r.range[0] as f32, r.range[1] as f32],
                bands: 0,
                ramp: (0..32)
                    .map(|i| ramp.at(1.0 - f64::from(i) / 31.0))
                    .collect(),
                under: thin_cap,
                over: thick_cap,
                none: ctx.sketch_palette.inactive,
                legend: core_document::MapLegend {
                    title: format!("{} · wall thickness", node.name),
                    unit: "mm".to_string(),
                    stats,
                    histogram: r.histogram.counts.iter().map(|n| *n as u32).collect(),
                    tolerance: t.min_wall.map(|m| (r.range[0] as f32, m as f32)),
                },
            });
        }
        let mut fits: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == clearance::KIND && n.visible)
            .collect();
        fits.sort_by_key(|(id, n)| (std::cmp::Reverse(n.seq), **id));
        for (id, node) in fits {
            let Some(c) = document
                .feature_values(*id)
                .and_then(|d| clearance::Clearance::from_json(d).ok())
            else {
                continue;
            };
            if maps
                .iter()
                .any(|m: &core_document::ScalarMap| m.body == c.part)
                || !document.imported_body_effective_visible(c.part)
            {
                continue;
            }
            let Ok(inputs) = clearance::inputs(document, &c) else {
                continue;
            };
            let key = inputs.key;
            let Some(values) = self.maps.values(*id, key) else {
                let (id, job) = (*id, c.clone());
                self.maps
                    .run(id, key, move || clearance::measure(kernel, &inputs, &job));
                continue;
            };
            let r = clearance::read(&values, &c);
            maps.push(fit_map(&node.name, *id, key, &c, &r, values, ctx));
        }
        let mut overhangs: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == overhang::KIND && n.visible)
            .collect();
        overhangs.sort_by_key(|(id, n)| (std::cmp::Reverse(n.seq), **id));
        for (id, node) in overhangs {
            let Some(o) = document
                .feature_values(*id)
                .and_then(|d| overhang::Overhang::from_json(d).ok())
            else {
                continue;
            };
            let Some(geometry) = document.imported_geometry(o.body) else {
                continue;
            };
            if maps
                .iter()
                .any(|m: &core_document::ScalarMap| m.body == o.body)
                || !document.imported_body_effective_visible(o.body)
            {
                continue;
            }
            maps.push(overhang::map(
                &geometry.mesh,
                geometry.revision ^ id.0.as_u64_pair().1 ^ o.limit_deg.to_bits(),
                &o,
                &node.name,
                ctx.sketch_palette.inactive,
            ));
        }
        maps
    }

    /// Every point element, a cross the same size at any zoom, and the
    /// clicks of an element in progress, pinned.
    fn get_screen_space_marks(
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
        marks.extend(self.picks.drawn(ctx).0);
        marks
    }

    fn get_screen_space_labels(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<core_document::FeatureId>,
    ) -> Vec<core_document::ScreenSpaceLabel> {
        let palette = &ctx.sketch_palette;
        let document = &*ctx.document;
        let mut labels = self.picks.drawn(ctx).1;
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
            let middle = element::scale(element::add(reading.from, reading.to), 0.5);
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
                    let p = made.plane;
                    element::add(
                        p.origin,
                        element::add(element::scale(p.x_axis, u), element::scale(p.y_axis, v)),
                    )
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
                &self.tolerance_readings,
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
            let Some(c) = document
                .feature_values(*id)
                .and_then(|d| Comparison::from_json(d).ok())
            else {
                continue;
            };
            let (Some(tolerance), Ok(inputs)) = (c.tolerance, compare::inputs(document, &c)) else {
                continue;
            };
            let Some(spots) = self.maps.hot_spots(*id, inputs.key, tolerance, || {
                self.maps
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
            let Some(c) = document
                .feature_values(*id)
                .and_then(|d| Comparison::from_json(d).ok())
            else {
                continue;
            };
            let (Some(cell), Ok(inputs)) = (c.volume_cell, compare::inputs(document, &c)) else {
                continue;
            };
            let key = inputs.key;
            let back = inputs.scan_to_reference().inverse();
            let Some(found) = self.maps.volumes(*id, key, cell) else {
                if let Some(kernel) = self.kernel.get() {
                    self.maps.request_volumes(*id, inputs, cell, *kernel);
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
                &self.tolerance_readings,
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
                    self.tolerance_task = Some(tolerance_task::Task { id, opening: None });
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
            match add_thickness(ctx, &self.maps, t) {
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
            match add_clearance(ctx, &self.maps, c) {
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
            match add_overhang(ctx, o) {
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
            let aligning = !self.picks.aligning;
            self.picks.clear();
            self.picks.aligning = aligning;
            self.armed = None;
            self.pinning = false;
            return InputResult::consumed();
        }
        if self.picks.aligning
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
            if let Ok(mut fitting) = self.fitting.lock() {
                *fitting = Some(Fitting {
                    scan,
                    reference,
                    search: true,
                    answer: None,
                });
            }
            let slot = self.fitting.clone();
            std::thread::spawn(move || {
                let answer = align::best_fit(kernel, &inputs, placed, true);
                if let Ok(mut fitting) = slot.lock()
                    && let Some(f) = fitting.as_mut()
                {
                    f.answer = Some(answer);
                }
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
            match add_comparison(ctx, &self.maps, c) {
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
            self.pinning = !self.pinning;
            self.armed = None;
            return InputResult::consumed();
        }
        if self.pinning
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
            } if self.armed.is_some()
                || self.pinning
                || self.picks.aligning
                || self.picks.any() =>
            {
                self.armed = None;
                self.pinning = false;
                self.picks.clear();
                return InputResult::consumed();
            }
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                ..
            } => {
                let Some(kind) = self.armed else {
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
                    self.picks.pin_seed(scan, at);
                    return InputResult::consumed();
                }
                let more = self.picks.take_seeds(scan);
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
        if ctx.marking.is_none() {
            self.armed = (self.armed != Some(kind)).then_some(kind);
            self.picks.clear();
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
        let a = core_document::Args(args);
        match id {
            SHADOW => {
                let (scan, _, normal, _) = section_plane(
                    ctx.document,
                    a.opt_id("element")?.map(core_document::FeatureId),
                    a.opt_id("scan")?.map(BodyId),
                    a.opt_string("plane")?.unwrap_or("xy"),
                    false,
                    0.0,
                )?;
                let (mesh, _) = ctx
                    .document
                    .local_geometry(scan)
                    .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
                let (origin, x_axis) = shadow::foot(&mesh, normal)
                    .ok_or_else(|| CommandError::failed("the scan has no points"))?;
                let tolerance = a.opt_number("tolerance")?.unwrap_or(0.1);
                if tolerance <= 0.0 {
                    return Err(CommandError::bad("tolerance", "is above zero"));
                }
                let kernel = ctx
                    .kernel
                    .ok_or_else(|| CommandError::failed("no kernel to outline with"))?;
                let number = ctx
                    .document
                    .feature_tree()
                    .all_nodes()
                    .filter(|(_, n)| n.workbench_id.as_str() == shadow::KIND)
                    .count()
                    + 1;
                let s = shadow::Shadow {
                    name: format!("Shadow {number}"),
                    scan,
                    origin,
                    normal,
                    x_axis,
                    play: a.opt_number("play")?.unwrap_or(1.0),
                    tolerance,
                    round_below_deg: a.opt_number("round_below")?.unwrap_or(30.0),
                    fill_below: a.opt_number("fill_below")?.unwrap_or(0.0).max(0.0),
                    block: a.opt_bool("block")?.unwrap_or(true),
                    height: a.opt_number("height")?,
                    made: None,
                };
                let made = shadow::make(ctx.document, kernel, &s);
                if let Some(why) = &made.error {
                    return Err(CommandError::failed(why.clone()));
                }
                let body = ctx.document.create_body(Some(s.name.clone()));
                let id = ctx
                    .document
                    .add_feature_in_body(s.clone(), s.name.clone(), Some(body))
                    .map_err(|e| CommandError::failed(e.to_string()))?;
                Ok(shadow_answer(id, body, &s, &made))
            }
            COMPENSATION => {
                let scan = BodyId(a.id("scan")?);
                let reference = BodyId(a.id("reference")?);
                let kernel = ctx
                    .kernel
                    .ok_or_else(|| CommandError::failed("no kernel to measure with"))?;
                let inputs = compare::inputs_for(ctx.document, scan, reference)
                    .map_err(CommandError::failed)?;
                let (points, normals) = inputs.points();
                if normals.len() != points.len() {
                    return Err(CommandError::failed(
                        "the scan has no normals to measure along",
                    ));
                }
                let (ref_mesh, _) = ctx
                    .document
                    .local_geometry(reference)
                    .ok_or_else(|| CommandError::failed("the reference has no mesh"))?;
                let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
                for p in &ref_mesh.positions {
                    for k in 0..3 {
                        lo[k] = lo[k].min(f64::from(p[k]));
                        hi[k] = hi[k].max(f64::from(p[k]));
                    }
                }
                let diagonal = (0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f64>().sqrt();
                let distances = kernel
                    .signed_distance(
                        inputs.target(),
                        &kernel_api::DistanceRequest {
                            points: points.clone(),
                            normals: normals.clone(),
                            max: Some(diagonal / 20.0),
                            facing_limit_deg: Some(60.0),
                        },
                    )
                    .map_err(|e| CommandError::failed(e.to_string()))?;
                let centre = std::array::from_fn(|k| (lo[k] + hi[k]) / 2.0);
                let roles = compensation::roles(&ref_mesh, &points, &normals, lo[2]);
                let fit = metrology::shrink::fit(
                    &points,
                    &normals,
                    &distances,
                    &roles,
                    centre,
                    a.opt_bool("tie_xy")? == Some(true),
                )
                .ok_or_else(|| {
                    CommandError::failed(
                        "the scan's surface does not face enough ways to tell its axes apart",
                    )
                })?;
                let xyz = |v: [f64; 3]| json!({"x": v[0], "y": v[1], "z": v[2]});
                let on = |role| roles.iter().filter(|r| **r == role).count();
                let round = |error: Option<f64>, role| {
                    error.map(|e| json!({"radius_error": e, "offset": -e, "points": on(role)}))
                };
                let warp = compensation::warp(&points, &normals, lo[2]);
                Ok(json!({
                    "stretch": xyz(fit.stretch.map(|e| e * 100.0)),
                    "compensation": xyz(fit.compensation()),
                    "shift": xyz(fit.shift),
                    "holes": round(fit.hole, metrology::shrink::Role::Hole),
                    "bosses": round(fit.boss, metrology::shrink::Role::Boss),
                    "flare": compensation::flare(&fit, &points, &normals, &distances, &roles),
                    "warp": warp.map(|w| json!({
                        "flatness": w.flatness,
                        "lift": w.lift,
                        "at": xyz(w.at),
                    })),
                    "rms_before": fit.rms[0],
                    "rms_after": fit.rms[1],
                    "used": fit.used,
                    "points": fit.points,
                }))
            }
            EXPORT_STEP | EXPORT_IGES => {
                let items = self.construction_items(ctx.document, a.opt_id("scan")?.map(BodyId));
                let kernel = ctx
                    .kernel
                    .ok_or_else(|| CommandError::failed("no kernel to write with"))?;
                let (bytes, warnings) = write_construction(kernel, &items, id == EXPORT_IGES)
                    .map_err(|e| CommandError::failed(e.to_string()))?;
                let path = a.string("path")?;
                std::fs::write(path, bytes)
                    .map_err(|e| CommandError::failed(format!("{path}: {e}")))?;
                let mut answer = json!({"path": path, "items": items.len()});
                if id == EXPORT_IGES {
                    answer["warnings"] = json!(warnings);
                }
                Ok(answer)
            }
            SECTION => {
                let element = a.opt_id("element")?.map(core_document::FeatureId);
                let (scan, origin, normal, half) = section_plane(
                    ctx.document,
                    element,
                    a.opt_id("scan")?.map(BodyId),
                    a.opt_string("plane")?.unwrap_or("xy"),
                    a.opt_bool("through")? == Some(true),
                    a.opt_number("angle")?.unwrap_or(0.0),
                )?;
                let count = a.opt_number("count")?.unwrap_or(1.0);
                if !(1.0..=200.0).contains(&count) {
                    return Err(CommandError::bad("count", "is 1 to 200"));
                }
                let number = ctx
                    .document
                    .feature_tree()
                    .all_nodes()
                    .filter(|(_, n)| n.workbench_id.as_str() == section::KIND)
                    .count()
                    + 1;
                let s = section::Section {
                    name: format!("Section {number}"),
                    scan,
                    origin,
                    normal,
                    offset: a.opt_number("offset")?.unwrap_or(0.0),
                    count: count.round() as u32,
                    step: a.opt_number("step")?.unwrap_or(5.0),
                    half,
                };
                let kernel = ctx
                    .kernel
                    .ok_or_else(|| CommandError::failed("no kernel to cut with"))?;
                let (mesh, _) = ctx
                    .document
                    .local_geometry(scan)
                    .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
                let cuts = section::cut(kernel, &mesh, &s).map_err(CommandError::failed)?;
                let id = ctx
                    .document
                    .add_feature_in_body(s.clone(), s.name.clone(), Some(scan))
                    .map_err(|e| CommandError::failed(e.to_string()))?;
                Ok(section_answer(id, &s, &cuts))
            }
            FIT_COMMAND => {
                let kind = kind_named(a.string("kind")?).ok_or_else(|| {
                    CommandError::bad(
                        "kind",
                        "is plane, sphere, cylinder, cone, torus, circle or line",
                    )
                })?;
                let cutoff = match a.opt_number("cutoff")? {
                    Some(k) if k <= 0.0 => None,
                    Some(k) => Some(k),
                    None => Some(DEFAULT_CUTOFF),
                };
                let (id, element) =
                    fit_marked(ctx, kind, cutoff, method_arg(&a)?).map_err(CommandError::failed)?;
                Ok(answer_of(id, &element))
            }
            PIN => {
                let scan = BodyId(a.id("scan")?);
                let at = a.0.get("at").cloned().unwrap_or(Value::Null);
                let coord = |k: &str, i: usize| {
                    at.get(k)
                        .or_else(|| at.get(i))
                        .and_then(Value::as_f64)
                        .ok_or_else(|| CommandError::bad("at", "needs {x, y, z}"))
                };
                let at = [coord("x", 0)?, coord("y", 1)?, coord("z", 2)?];
                let (map, values) = self
                    .shown_map(ctx.document, scan)
                    .ok_or_else(|| CommandError::failed("no map is shown on that scan"))?;
                let (mesh, _) = ctx
                    .document
                    .local_geometry(scan)
                    .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
                let value = pins::read(&mesh, &values, at, READING_REACH);
                let number = ctx
                    .document
                    .feature_tree()
                    .all_nodes()
                    .filter(|(_, n)| n.workbench_id.as_str() == pins::KIND && n.body == Some(scan))
                    .count()
                    + 1;
                let pin = pins::Pin {
                    name: format!("Pin {number}"),
                    scan,
                    map,
                    at,
                };
                let id = ctx
                    .document
                    .add_feature_in_body(pin.clone(), pin.name.clone(), Some(scan))
                    .map_err(|e| CommandError::failed(e.to_string()))?;
                Ok(json!({"pin": id.0.to_string(), "name": pin.name, "value": value}))
            }
            ELEMENT_MAP => {
                let element = core_document::FeatureId(a.id("element")?);
                let e = element_at(ctx.document, element)
                    .ok_or_else(|| CommandError::bad("element", "is not an element"))?;
                let number = ctx
                    .document
                    .feature_tree()
                    .all_nodes()
                    .filter(|(_, n)| n.workbench_id.as_str() == element_map::KIND)
                    .count()
                    + 1;
                let m = element_map::ElementMap {
                    name: format!("Map {number}"),
                    scan: e.scan,
                    element,
                    facing_limit_deg: a.opt_number("facing")?.unwrap_or(60.0),
                    flip: a.opt_bool("flip")? == Some(true),
                    tolerance: a.opt_number("tolerance")?,
                };
                let (mesh, e, key) = element_map_inputs(ctx.document, &m)
                    .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
                let values: std::sync::Arc<[f32]> = element_map::measure(&mesh, &e, &m).into();
                let id = ctx
                    .document
                    .add_feature_in_body(m.clone(), m.name.clone(), Some(m.scan))
                    .map_err(|e| CommandError::failed(e.to_string()))?;
                self.maps.store(id, key, values.clone());
                let wide: Vec<f64> = values.iter().map(|v| f64::from(*v)).collect();
                let s = metrology::stats::summary(&wide);
                Ok(json!({
                    "map": id.0.to_string(),
                    "name": m.name,
                    "measured": s.map_or(0, |s| s.count),
                    "mean": s.map(|s| s.mean),
                    "rms": s.map(|s| s.rms),
                    "min": s.map(|s| s.min),
                    "max": s.map(|s| s.max),
                }))
            }
            TOLERANCE => {
                let characteristic = tolerance::Characteristic::ALL
                    .into_iter()
                    .find(|c| {
                        c.key().eq_ignore_ascii_case(
                            &a.string("characteristic").unwrap_or("").replace(' ', "_"),
                        )
                    })
                    .ok_or_else(|| {
                        CommandError::bad("characteristic", "is not a tolerance characteristic")
                    })?;
                let datum = a.opt_id("datum")?.map(core_document::FeatureId);
                let fields =
                    serde_json::to_value(args).map_err(|e| CommandError::failed(e.to_string()))?;
                let mut t: tolerance::Tolerance = serde_json::from_value(json!({
                    "name": "", "scan": uuid::Uuid::nil(), "characteristic": characteristic,
                    "element": a.id("element")?, "datum": datum, "limit": a.number("limit")?,
                }))
                .map_err(|e| CommandError::failed(e.to_string()))?;
                let mut data = t.to_json();
                for key in [
                    "datums",
                    "basic",
                    "basic_direction",
                    "material",
                    "material_size",
                    "unequal",
                    "section",
                    "comparison",
                    "reference_face",
                    "opposed",
                    "section_width",
                ] {
                    if let Some(v) = fields.get(key) {
                        data[key] = v.clone();
                    }
                }
                t = tolerance::Tolerance::from_json(&data)
                    .map_err(|e| CommandError::failed(e.to_string()))?;
                t.angle_deg = a.opt_number("angle")?;
                if let Some(id) = a.opt_id("tolerance")? {
                    let id = core_document::FeatureId(id);
                    let old = ctx
                        .document
                        .feature_tree()
                        .get_node(id)
                        .ok_or_else(|| CommandError::bad("tolerance", "is gone"))?;
                    if old.workbench_id.as_str() != tolerance::KIND {
                        return Err(CommandError::bad("tolerance", "is not a tolerance"));
                    }
                    let body = old.body;
                    t.name = old.name.clone();
                    t.scan = element_at(ctx.document, t.element)
                        .ok_or_else(|| CommandError::bad("element", "is gone"))?
                        .scan;
                    if body != Some(t.scan) {
                        return Err(CommandError::bad(
                            "element",
                            "must stay on the tolerance's scan",
                        ));
                    }
                    let reading = tolerance_read::evaluate(
                        ctx.document,
                        &t,
                        ctx.kernel.map(|k| k as &dyn kernel_api::KernelQueries),
                    )
                    .map_err(CommandError::failed)?;
                    ctx.document
                        .update_feature_data(id, t.to_json())
                        .map_err(|e| CommandError::failed(e.to_string()))?;
                    ctx.document.set_feature_dependencies(id, t.dependencies());
                    return Ok(tolerance_answer(id, &t, &Ok(reading)));
                }
                add_tolerance(ctx, t).map_err(CommandError::failed)
            }
            THICKNESS => {
                let t = thickness::WallThickness {
                    name: String::new(),
                    body: BodyId(a.id("body")?),
                    sphere: a.opt_bool("sphere")? == Some(true),
                    min_wall: a.opt_number("min_wall")?,
                    max: a.opt_number("max")?,
                };
                add_thickness(ctx, &self.maps, t).map_err(CommandError::failed)
            }
            ELEMENT_SOLID => {
                let id = core_document::FeatureId(a.id("element")?);
                let thickness = a.opt_number("thickness")?.unwrap_or(2.0);
                element_solid(ctx.document, id, thickness).map_err(CommandError::failed)
            }
            CLEARANCE => {
                let c = clearance::Clearance {
                    name: String::new(),
                    part: BodyId(a.id("part")?),
                    housing: BodyId(a.id("housing")?),
                    min_gap: a.opt_number("min_gap")?,
                    reach: a.opt_number("reach")?.unwrap_or(clearance::DEFAULT_REACH),
                };
                add_clearance(ctx, &self.maps, c).map_err(CommandError::failed)
            }
            OVERHANG => {
                let o = overhang::Overhang {
                    name: String::new(),
                    body: BodyId(a.id("body")?),
                    limit_deg: a.opt_number("limit")?.unwrap_or(45.0),
                };
                add_overhang(ctx, o).map_err(CommandError::failed)
            }
            HOT_SPOTS => {
                let id = core_document::FeatureId(a.id("comparison")?);
                let (c, inputs, values) =
                    comparison_values(ctx, &self.maps, id).map_err(CommandError::failed)?;
                let tolerance = a.opt_number("tolerance")?.or(c.tolerance).ok_or_else(|| {
                    CommandError::bad("tolerance", "is needed: the comparison has none")
                })?;
                let least = a
                    .opt_number("least")?
                    .map_or(SPOT_LEAST, |n| n.max(1.0) as usize);
                Ok(json!(hotspots::find(
                    inputs.scan_mesh(),
                    &values,
                    tolerance,
                    least
                )))
            }
            VOLUME_ERRORS => {
                let id = core_document::FeatureId(a.id("comparison")?);
                let mut c = ctx
                    .document
                    .feature_values(id)
                    .filter(|_| {
                        ctx.document
                            .feature_tree()
                            .get_node(id)
                            .is_some_and(|n| n.workbench_id.as_str() == compare::KIND)
                    })
                    .and_then(|d| Comparison::from_json(d).ok())
                    .ok_or_else(|| CommandError::bad("comparison", "is not a comparison"))?;
                if a.opt_bool("off")?.unwrap_or(false) {
                    c.volume_cell = None;
                    ctx.document
                        .update_feature_data(id, c.to_json())
                        .map_err(|e| CommandError::failed(e.to_string()))?;
                    return Ok(json!({ "cell": Value::Null, "regions": [] }));
                }
                let inputs = compare::inputs(ctx.document, &c).map_err(CommandError::failed)?;
                let cell = match a.opt_number("cell")? {
                    Some(cell) if cell.is_finite() && cell > 0.0 => cell,
                    Some(_) => return Err(CommandError::bad("cell", "is a positive distance")),
                    None => volume::default_cell(longest_side(inputs.scan_mesh())),
                };
                let kernel = ctx
                    .kernel
                    .ok_or_else(|| CommandError::failed("no kernel to measure with"))?;
                let regions =
                    volume::measure(kernel, &inputs, cell).map_err(CommandError::failed)?;
                let answer = volume::answer(&regions, cell);
                if c.volume_cell != Some(cell) {
                    c.volume_cell = Some(cell);
                    ctx.document
                        .update_feature_data(id, c.to_json())
                        .map_err(|e| CommandError::failed(e.to_string()))?;
                }
                let key = compare::inputs(ctx.document, &c)
                    .map_err(CommandError::failed)?
                    .key;
                self.maps
                    .store_volumes(id, key, cell, std::sync::Arc::new(regions));
                Ok(answer)
            }
            MAP_EXPORT => {
                let id = core_document::FeatureId(a.id("comparison")?);
                let path = std::path::PathBuf::from(a.string("path")?);
                let format = match a.opt_string("format")? {
                    Some(f) => f.to_lowercase(),
                    None => path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("csv")
                        .to_lowercase(),
                };
                let (_, inputs, values) =
                    comparison_values(ctx, &self.maps, id).map_err(CommandError::failed)?;
                let bytes = export::write(inputs.scan_mesh(), &values, &format)
                    .map_err(|why| CommandError::bad("format", why))?;
                std::fs::write(&path, bytes)
                    .map_err(|e| CommandError::failed(format!("{}: {e}", path.display())))?;
                Ok(json!({"path": path.display().to_string(), "points": values.len()}))
            }
            BEST_FIT => {
                let scan = BodyId(a.id("scan")?);
                let reference = BodyId(a.id("reference")?);
                let marked = a.opt_bool("marked")? == Some(true);
                let search = a.opt_bool("search")?.unwrap_or(!marked);
                let keep = if marked {
                    let marking =
                        ctx.marking
                            .as_ref()
                            .filter(|m| m.body == scan)
                            .ok_or_else(|| {
                                CommandError::failed("mark the part of the scan to fit on")
                            })?;
                    Some(marking.vertices.clone())
                } else {
                    None
                };
                let kernel = ctx
                    .kernel
                    .ok_or_else(|| CommandError::failed("no kernel to fit with"))?;
                let inputs = compare::inputs_for(ctx.document, scan, reference)
                    .map_err(CommandError::failed)?;
                let placed = ctx.document.body_placement(reference);
                let (after, fit) = align::best_fit_of(
                    kernel,
                    &inputs,
                    placed,
                    search,
                    keep.as_deref().map(Vec::as_slice),
                )
                .map_err(CommandError::failed)?;
                let mut how = best_fit_how(reference, search, &fit);
                if let align::How::BestFit { marked: m, .. } = &mut how {
                    *m = marked;
                }
                record_alignment(ctx, scan, how, after).map_err(CommandError::failed)
            }
            MOVE_COMMAND => {
                let scan = BodyId(a.id("scan")?);
                let triple = |name: &str| -> Result<[f64; 3], CommandError> {
                    let v = a.0.get(name).cloned().unwrap_or(Value::Null);
                    if v.is_null() {
                        return Ok([0.0; 3]);
                    }
                    let get = |k: &str, i: usize| {
                        v.get(k)
                            .or_else(|| v.get(i))
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0)
                    };
                    Ok([get("x", 0), get("y", 1), get("z", 2)])
                };
                let (translate, rotate_deg) = (triple("translate")?, triple("rotate")?);
                let after = align::by_numbers(translate, rotate_deg)
                    .after(&ctx.document.body_placement(scan));
                record_alignment(
                    ctx,
                    scan,
                    align::How::Numbers {
                        translate,
                        rotate_deg,
                    },
                    after,
                )
                .map_err(CommandError::failed)
            }
            RESET_ALIGNMENT => {
                let scan = BodyId(a.id("scan")?);
                record_alignment(
                    ctx,
                    scan,
                    align::How::Reset,
                    core_document::BodyPlacement::IDENTITY,
                )
                .map_err(CommandError::failed)
            }
            COMPARE_COMMAND => {
                let c = Comparison {
                    name: String::new(),
                    scan: BodyId(a.id("scan")?),
                    reference: BodyId(a.id("reference")?),
                    range: a.opt_number("range")?,
                    bands: a.opt_number("bands")?.map_or(0, |b| b.max(0.0) as u32),
                    ramp: a
                        .opt_string("ramp")?
                        .unwrap_or(metrology::ramp::Ramp::Jet.name())
                        .to_string(),
                    tolerance: a.opt_number("tolerance")?,
                    max: a.opt_number("max")?,
                    facing_limit_deg: Some(a.opt_number("facing")?.unwrap_or(90.0)),
                    volume_cell: None,
                };
                let (_, answer) =
                    add_comparison(ctx, &self.maps, c).map_err(CommandError::failed)?;
                Ok(answer)
            }
            GROW_COMMAND => {
                let kind = kind_named(a.string("kind")?).ok_or_else(|| {
                    CommandError::bad(
                        "kind",
                        "is plane, sphere, cylinder, cone, torus, circle or line",
                    )
                })?;
                let scan = BodyId(a.id("scan")?);
                let at = a.0.get("at").cloned().unwrap_or(Value::Null);
                let coord = |k: &str, i: usize| {
                    at.get(k)
                        .or_else(|| at.get(i))
                        .and_then(Value::as_f64)
                        .ok_or_else(|| CommandError::bad("at", "needs {x, y, z}"))
                };
                let at = [coord("x", 0)?, coord("y", 1)?, coord("z", 2)?];
                let cutoff = match a.opt_number("cutoff")? {
                    Some(k) if k <= 0.0 => None,
                    Some(k) => Some(k),
                    None => Some(DEFAULT_CUTOFF),
                };
                let peel = a.opt_bool("peel")? == Some(true);
                let more = match a.0.get("more") {
                    Some(Value::Array(list)) => list
                        .iter()
                        .map(|p| {
                            let c = |k: &str, i: usize| {
                                p.get(k).or_else(|| p.get(i)).and_then(Value::as_f64)
                            };
                            match (c("x", 0), c("y", 1), c("z", 2)) {
                                (Some(x), Some(y), Some(z)) => Ok([x, y, z]),
                                _ => Err(CommandError::bad("more", "lists {x, y, z} points")),
                            }
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    _ => Vec::new(),
                };
                let (id, element) =
                    grow_at(ctx, kind, scan, at, &more, cutoff, peel, method_arg(&a)?)
                        .map_err(CommandError::failed)?;
                Ok(answer_of(id, &element))
            }
            CONSTRUCT_COMMAND => {
                let made = Made::ALL
                    .into_iter()
                    .find(|m| {
                        m.word()
                            .eq_ignore_ascii_case(a.string("made").unwrap_or(""))
                    })
                    .ok_or_else(|| CommandError::bad("made", "is point, line, plane or circle"))?;
                let from = ids_in(&a, "from")?;
                let offset = a.opt_number("offset")?.unwrap_or(0.0);
                let (_, answer) =
                    add_construction(ctx, made, from, offset).map_err(CommandError::failed)?;
                Ok(answer)
            }
            DIMENSION_COMMAND => {
                let kind = DimKind::ALL
                    .into_iter()
                    .find(|k| {
                        k.word()
                            .eq_ignore_ascii_case(a.string("kind").unwrap_or(""))
                    })
                    .ok_or_else(|| CommandError::bad("kind", "is distance, angle or diameter"))?;
                let elements = ids_in(&a, "elements")?;
                let band = match (
                    a.opt_number("nominal")?,
                    a.opt_number("plus")?,
                    a.opt_number("minus")?,
                ) {
                    (None, None, None) => None,
                    (nominal, plus, minus) => {
                        let found: Vec<Element> = elements
                            .iter()
                            .filter_map(|id| element_at(ctx.document, *id))
                            .collect();
                        let nominal = match nominal {
                            Some(n) => n,
                            None => dimension::design_value(
                                kind,
                                dimension::read(kind, &found)
                                    .map_err(CommandError::failed)?
                                    .value,
                            ),
                        };
                        Some((
                            nominal,
                            plus.unwrap_or(0.0).abs(),
                            minus.unwrap_or(0.0).abs(),
                        ))
                    }
                };
                let (_, answer) =
                    add_dimension(ctx, kind, elements, band).map_err(CommandError::failed)?;
                Ok(answer)
            }
            REPORT_COMMAND => {
                let only = a.opt_id("scan")?.map(BodyId);
                let report = report_of(
                    ctx.document,
                    only,
                    &self.tolerance_readings,
                    ctx.kernel,
                    true,
                );
                let path = a.opt_string("path")?.map(std::path::PathBuf::from);
                let format = match a.opt_string("format")? {
                    Some(f) => f.to_lowercase(),
                    None => path
                        .as_ref()
                        .and_then(|p| p.extension()?.to_str().map(str::to_lowercase))
                        .unwrap_or_else(|| "json".to_string()),
                };
                let written = match format.as_str() {
                    "json" => serde_json::to_string_pretty(&report).unwrap_or_default(),
                    "csv" => report::csv(&report),
                    "text" | "txt" => report::text(&report, "Inspection report"),
                    _ => return Err(CommandError::bad("format", "is json, csv or text")),
                };
                match path {
                    Some(path) => {
                        std::fs::write(&path, written).map_err(|e| {
                            CommandError::failed(format!("{}: {e}", path.display()))
                        })?;
                        Ok(json!({"path": path.display().to_string()}))
                    }
                    None if format == "json" => Ok(report),
                    None => Ok(json!(written)),
                }
            }
            _ => Err(CommandError::Unknown(id.to_string())),
        }
    }
}

#[cfg(test)]
mod gdt_tests;

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
                .get_screen_space_labels(ctx, None)
                .into_iter()
                .map(|l| l.text)
                .collect();
            let radii: Vec<f32> = bench
                .get_screen_space_marks(ctx, None)
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
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            click(&mut bench, &mut ctx, scan, p);
            click(&mut bench, &mut ctx, reference, [p[0] + 5.0, p[1], p[2]]);
        }
        assert_eq!(shown(&bench, &ctx).0, ["1", "2", "3", "1", "2", "3"]);
        core_document::HookOutcome::take(&mut ctx);
        key(&mut bench, &mut ctx, core_document::KeyCode::Enter);
        let outcome = core_document::HookOutcome::take(&mut ctx);
        assert_eq!(outcome.recorded.len(), 1, "the alignment is made");
        assert!(shown(&bench, &ctx).0.is_empty());
        let moved = ctx.document.body_placement(scan).point([0.0, 0.0, 0.0]);
        assert!((moved[0] - 5.0).abs() < 1e-3, "{moved:?}");
    }
}
