//! Fitting marked or grown scan regions and storing their measured elements.

use crate::element::{Element, Fitted};
use crate::{element, next_number, tolerance};
use core_document::{BodyId, CommandError, WorkbenchRuntimeContext};
use kernel_api::{FitKind, FitRequest, GrowRequest};
use serde_json::{Value, json};

/// A command's `method`: least squares when it gives none.
pub(super) fn method_arg(a: &core_document::Args) -> Result<kernel_api::FitMethod, CommandError> {
    use kernel_api::FitMethod as M;
    Ok(match a.opt_string("method")? {
        None => M::LeastSquares,
        Some(m) => match m.to_lowercase().replace([' ', '-'], "_").as_str() {
            "least_squares" | "gaussian" => M::LeastSquares,
            "minimum_zone" | "chebyshev" => M::MinimumZone,
            "inscribed" | "maximum_inscribed" => M::Inscribed,
            "circumscribed" | "minimum_circumscribed" => M::Circumscribed,
            _ => {
                return Err(CommandError::bad(
                    "method",
                    "is least_squares, minimum_zone, inscribed or circumscribed",
                ));
            }
        },
    })
}

pub(super) fn kind_named(name: &str) -> Option<FitKind> {
    FitKind::ALL
        .into_iter()
        .find(|k| element::kind_word(*k).eq_ignore_ascii_case(name))
}

/// Fit `kind` to the marked part of `scan` and keep it as an element.
pub(super) fn fit_marked(
    ctx: &mut WorkbenchRuntimeContext,
    kind: FitKind,
    cutoff: Option<f64>,
    method: kernel_api::FitMethod,
) -> Result<(core_document::FeatureId, Element), String> {
    let marking = ctx
        .marking
        .clone()
        .ok_or("nothing is marked: mark part of a scan first (Mark surface… in its menu)")?;
    let scan = marking.body;
    let (mesh, _) = ctx
        .document
        .local_geometry(scan)
        .ok_or("the marked scan has no mesh")?;
    let region = element::runs(&marking.vertices);
    let picked: Vec<usize> = element::vertices(&region)
        .map(|v| v as usize)
        .filter(|v| *v < mesh.positions.len())
        .collect();
    let point = |v: usize| mesh.positions[v].map(f64::from);
    let points: Vec<[f64; 3]> = picked.iter().map(|v| point(*v)).collect();
    let normals = if mesh.normals.len() == mesh.positions.len() {
        picked
            .iter()
            .map(|v| mesh.normals[*v].map(f64::from))
            .collect()
    } else {
        Vec::new()
    };
    let kernel = ctx.kernel.ok_or("no kernel to fit with")?;
    let answer = kernel
        .fit(&FitRequest {
            kind,
            points: points.clone(),
            normals,
            sigma_cutoff: cutoff,
            fixed_radius: None,
        })
        .map_err(|e| e.to_string())?;
    keep(
        ctx,
        Fit {
            scan,
            kind,
            cutoff,
            method,
            region,
            revision: marking.revision,
        },
        &points,
        answer,
    )
}

/// Grow `kind` over `scan` from `at` and the `more` seeds beside it (in
/// the scan's own frame), fit it and keep it as an element.
#[allow(clippy::too_many_arguments)]
pub(super) fn grow_at(
    ctx: &mut WorkbenchRuntimeContext,
    kind: FitKind,
    scan: BodyId,
    at: [f64; 3],
    more: &[[f64; 3]],
    cutoff: Option<f64>,
    peel: bool,
    method: kernel_api::FitMethod,
) -> Result<(core_document::FeatureId, Element), String> {
    if ctx.document.body_scan(scan).is_none() {
        return Err("that body is not a scan".into());
    }
    let (mesh, _) = ctx
        .document
        .local_geometry(scan)
        .ok_or("the scan has no mesh")?;
    let revision = ctx
        .document
        .imported_geometry(scan)
        .map_or(0, |g| g.revision);
    let kernel = ctx.kernel.ok_or("no kernel to fit with")?;
    let grown = kernel
        .grow(
            &mesh,
            &GrowRequest {
                kind,
                seeds: std::iter::once(at).chain(more.iter().copied()).collect(),
                sigma_cutoff: cutoff,
                normal_limit_deg: None,
                peel,
                max_triangles: None,
            },
        )
        .map_err(|e| e.to_string())?;
    if grown.seeds_in_region.first() == Some(&false) {
        return Err(format!(
            "the surface where you clicked is not a {}",
            element::kind_word(kind).to_lowercase()
        ));
    }
    let mut flags = vec![false; mesh.positions.len()];
    for v in &grown.vertices {
        if let Some(f) = flags.get_mut(*v as usize) {
            *f = true;
        }
    }
    let region = element::runs(&flags);
    let points: Vec<[f64; 3]> = grown
        .vertices
        .iter()
        .filter_map(|v| mesh.positions.get(*v as usize))
        .map(|p| p.map(f64::from))
        .collect();
    keep(
        ctx,
        Fit {
            scan,
            kind,
            cutoff,
            method,
            region,
            revision,
        },
        &points,
        grown.fit,
    )
}

/// What an element is fitted from, and how.
struct Fit {
    scan: BodyId,
    kind: FitKind,
    cutoff: Option<f64>,
    method: kernel_api::FitMethod,
    region: Vec<[u32; 2]>,
    revision: u64,
}

/// Keep a fit of `points` on `fit.scan` as an element: the least-squares
/// `answer` sets which points are used, and a zone or mating method then
/// fits those; the minimum zone of the points used is kept as its form.
fn keep(
    ctx: &mut WorkbenchRuntimeContext,
    fit: Fit,
    points: &[[f64; 3]],
    mut answer: kernel_api::FitAnswer,
) -> Result<(core_document::FeatureId, Element), String> {
    let Fit {
        scan,
        kind,
        cutoff,
        method,
        region,
        revision,
    } = fit;
    let used: Vec<[f64; 3]> = points
        .iter()
        .zip(&answer.used)
        .filter(|(_, u)| **u)
        .map(|(p, _)| *p)
        .collect();
    let kernel = ctx.kernel.ok_or("no kernel to fit with")?;
    let zone = tolerance::Characteristic::of_form(kind).and_then(|_| {
        kernel
            .zone_fit(kind, kernel_api::FitMethod::MinimumZone, &used)
            .ok()
    });
    if method != kernel_api::FitMethod::LeastSquares {
        let fitted = match (&zone, method) {
            (Some(z), kernel_api::FitMethod::MinimumZone) => z.clone(),
            _ => kernel
                .zone_fit(kind, method, &used)
                .map_err(|e| e.to_string())?,
        };
        answer.shape = fitted.shape;
        answer.sigma = fitted.sigma;
        answer.form = fitted.form;
    }
    let number = next_number(ctx.document, scan, element::kind_word(kind));
    let name = format!("{} {number}", element::kind_word(kind));
    let element = Element {
        method,
        name: name.clone(),
        scan,
        kind,
        cutoff,
        region,
        revision,
        result: Fitted {
            zone: zone.map(|z| element::FormZone {
                width: z.form.1 - z.form.0,
                optimal: z.converged,
            }),
            span: element::span_of(&answer.shape, &used),
            shape: answer.shape,
            sigma: answer.sigma,
            form: answer.form,
            used: used.len(),
            points: points.len(),
        },
    };
    let id = ctx
        .document
        .add_feature_in_body(element.clone(), name, Some(scan))
        .map_err(|e| e.to_string())?;
    Ok((id, element))
}

pub(super) fn answer_of(id: core_document::FeatureId, e: &Element) -> Value {
    let fitted = &e.result;
    json!({
        "element": id.0.to_string(),
        "name": e.name,
        "kind": element::kind_word(e.kind).to_lowercase(),
        "shape": fitted.shape,
        "sigma": fitted.sigma,
        "form": fitted.form.1 - fitted.form.0,
        "zone": fitted.zone,
        "method": e.method,
        "diameters": fitted.diameters().map(|(lo, hi)| json!({"min": lo, "max": hi})),
        "used": fitted.used,
        "points": fitted.points,
    })
}
