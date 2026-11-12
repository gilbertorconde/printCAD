//! Elements, constructions, dimensions and tolerances: found as the
//! document holds them, made from the selection, and answered as JSON.

use crate::{
    construction, construction::Construction, construction::Made, dimension, dimension::DimKind,
    dimension::Dimension, element, element::Element, extract, tolerance, tolerance_read, vec,
};
use core_document::{CommandError, Document, WorkbenchFeature, WorkbenchRuntimeContext};
use glam::DVec3;
use serde_json::{Value, json};

/// How deep constructions on constructions are followed.
const DEEPEST: usize = 32;

/// The tolerance `id` is and what it reads.
pub(crate) fn tolerance_at(
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

pub(crate) fn tolerance_answer(
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
pub(crate) fn characteristic_for(
    picked: &[Element],
) -> Option<(tolerance::Characteristic, Option<f64>)> {
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
pub(crate) fn add_tolerance(
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

/// `word 1`, `word 2`, ...: the next name of that kind no feature has.
pub(crate) fn numbered(document: &Document, word: &str) -> String {
    document.new_object_name(word, core_document::Numbering::Counted)
}

/// The element `id` is, fitted or constructed, with the values the
/// document holds for it.
pub(crate) fn element_at(document: &Document, id: core_document::FeatureId) -> Option<Element> {
    built_at(document, id, 0).ok()
}

/// The element `id` is, a construction worked out from its sources.
pub(crate) fn built_at(
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
pub(crate) fn add_construction(
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

pub(crate) fn construction_answer(
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
pub(crate) fn shape_revision(e: &Element) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(&e.result.shape)
        .unwrap_or_default()
        .hash(&mut h);
    format!("{:?}", e.result.span).hash(&mut h);
    h.finish()
}

/// The dimension `id` is and what it reads now.
pub(crate) fn dimension_at(
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
pub(crate) fn selected_elements(ctx: &WorkbenchRuntimeContext) -> Vec<core_document::FeatureId> {
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
pub(crate) fn element_solid(
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
pub(crate) fn add_dimension(
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

pub(crate) fn dimension_answer(
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

/// The feature ids listed under `name`.
pub(crate) fn ids_in(
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
