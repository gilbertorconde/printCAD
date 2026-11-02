//! The Inspection workbench: measuring a scan. Its first tools fit a plane,
//! sphere, cylinder, cone, torus, circle or line to the part of a scan the
//! user marked (Mark surface…), by orthogonal least squares with points
//! beyond three sigmas left out, and keep the result as an element under
//! the scan: its size, its sigma, its form error, and the lines that draw
//! it no larger than measured. Dimensions between elements read
//! distances, angles and diameters off them, each with a nominal and a
//! band it passes or fails.

pub mod construction;
pub mod dimension;
pub mod element;

use construction::{Construction, Made};
use core_document::{
    BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, Document, FeatureInfo,
    FeatureNode, InputResult, ParamKind, PassiveGeometry, PassiveTint, ToolDescriptor,
    WorkbenchContext, WorkbenchDescriptor, WorkbenchFeature, WorkbenchInputEvent,
    WorkbenchRuntimeContext,
};
use dimension::{DimKind, Dimension};
use element::{Element, Fitted};
use kernel_api::{FitKind, FitRequest, GrowRequest};
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

/// How deep constructions on constructions are followed.
const DEEPEST: usize = 32;

#[derive(Default)]
pub struct InspectionWorkbench {
    /// The fit a click on a scan makes, while nothing is marked.
    armed: Option<FitKind>,
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

fn kind_named(name: &str) -> Option<FitKind> {
    FitKind::ALL
        .into_iter()
        .find(|k| element::kind_word(*k).eq_ignore_ascii_case(name))
}

/// Fit `kind` to the marked part of `scan` and keep it as an element.
fn fit_marked(
    ctx: &mut WorkbenchRuntimeContext,
    kind: FitKind,
    cutoff: Option<f64>,
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
        scan,
        kind,
        cutoff,
        region,
        marking.revision,
        &points,
        answer,
    )
}

/// Grow `kind` over `scan` from `at` (in the scan's own frame), fit it and
/// keep it as an element.
fn grow_at(
    ctx: &mut WorkbenchRuntimeContext,
    kind: FitKind,
    scan: BodyId,
    at: [f64; 3],
    cutoff: Option<f64>,
    peel: bool,
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
                seeds: vec![at],
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
        ctx, scan, kind, cutoff, region, revision, &points, grown.fit,
    )
}

/// Keep a fit of `points` on `scan` as an element.
#[allow(clippy::too_many_arguments)]
fn keep(
    ctx: &mut WorkbenchRuntimeContext,
    scan: BodyId,
    kind: FitKind,
    cutoff: Option<f64>,
    region: Vec<[u32; 2]>,
    revision: u64,
    points: &[[f64; 3]],
    answer: kernel_api::FitAnswer,
) -> Result<(core_document::FeatureId, Element), String> {
    let used: Vec<[f64; 3]> = points
        .iter()
        .zip(&answer.used)
        .filter(|(_, u)| **u)
        .map(|(p, _)| *p)
        .collect();
    let number = next_number(ctx.document, scan, element::kind_word(kind));
    let name = format!("{} {number}", element::kind_word(kind));
    let element = Element {
        name: name.clone(),
        scan,
        kind,
        cutoff,
        region,
        revision,
        result: Fitted {
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

fn answer_of(id: core_document::FeatureId, e: &Element) -> Value {
    let fitted = &e.result;
    json!({
        "element": id.0.to_string(),
        "name": e.name,
        "kind": element::kind_word(e.kind).to_lowercase(),
        "shape": fitted.shape,
        "sigma": fitted.sigma,
        "form": fitted.form.1 - fitted.form.0,
        "diameters": fitted.diameters().map(|(lo, hi)| json!({"min": lo, "max": hi})),
        "used": fitted.used,
        "points": fitted.points,
    })
}

impl core_document::Workbench for InspectionWorkbench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(
            ID,
            "Inspection",
            "Measure a scan: fit shapes to its surface and read their size and form",
        )
        .icon("measure")
        .feature_kinds([element::KIND, dimension::KIND, construction::KIND])
    }

    fn configure(&self, context: &mut WorkbenchContext) {
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
            .returns(
                "{element, name, kind, shape, sigma, form, diameters, used, points}: in mm, \
                 the shape in the scan's own frame",
            )
            .note(
                "It fits what is marked (`pc.scan.mark`, or Mark surface… in the scan's \
                 menu), by orthogonal least squares. `sigma` is the RMS distance of the points \
                 used from the shape, `form` the width of the band they fill, `diameters` the \
                 smallest and largest a round shape reaches.",
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
                "peel",
                ParamKind::Bool,
                "Take off the region's rim while it turns away (a face's rounds)",
            )
            .returns("as inspect.fit")
            .note(
                "The region starts at the triangle nearest `at` and takes each neighbour whose \
                 points lie on the fitted surface and whose normal agrees with it, fitting \
                 again as it grows. A circle or a line is traced instead along the sharp edge \
                 nearest `at` (a hole's rim, a box's edge) until the edge turns away or meets \
                 another. It is refused where what is at `at` is not of the kind asked.",
            ),
        );
        context.register_command(
            CommandSpec::new(
                REPORT_COMMAND,
                "Every element and dimension of the inspection",
            )
            .optional("scan", ParamKind::Id, "Only this scan's")
            .returns(
                "{elements, dimensions, passed, failed}: each as inspect.fit and \
                     inspect.dimension answer it",
            )
            .read_only(),
        );
    }

    fn feature_info(&self, node: &FeatureNode) -> FeatureInfo {
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
                region: None,
            });
        }
        if node.workbench_id.as_str() == construction::KIND {
            let built = element_at(document, id)?;
            return Some(PassiveGeometry {
                mesh: built.result.lines(),
                revision: core_document::data_revision(data) ^ shape_revision(&built),
                tint: PassiveTint::External,
                region: None,
            });
        }
        let element = Element::from_json(data).ok()?;
        Some(PassiveGeometry {
            mesh: element.result.lines(),
            revision: core_document::data_revision(data),
            tint: PassiveTint::External,
            region: None,
        })
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        if let Some(kind) = dim_kind_of_tool(tool_id) {
            return selected_elements(ctx).len() == kind.operands();
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
        self.armed.is_some() && kind_of_tool(tool_id) == self.armed
    }

    fn status_items(&self, _ctx: &WorkbenchRuntimeContext) -> Option<core_document::StatusItems> {
        let kind = self.armed?;
        Some(core_document::StatusItems {
            mode: Some(format!(
                "Click a scan {} to fit a {} there · Esc to stop",
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
            let more: Vec<_> = ctx
                .document
                .feature_tree()
                .all_nodes()
                .filter(|(f, _)| !going.contains(f))
                .filter(|(_, n)| {
                    let on = match n.workbench_id.as_str() {
                        dimension::KIND => Dimension::from_json(&n.data).map(|d| d.elements),
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

    fn get_screen_space_labels(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<core_document::FeatureId>,
    ) -> Vec<core_document::ScreenSpaceLabel> {
        let palette = &ctx.sketch_palette;
        let document = &*ctx.document;
        let mut labels = Vec::new();
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
            put(dim.scan, middle, dim.label(&reading), color);
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
        match event {
            WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Escape,
            } if self.armed.is_some() => {
                self.armed = None;
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
                match grow_at(ctx, kind, scan, at, Some(DEFAULT_CUTOFF), false) {
                    Ok((id, element)) => {
                        let mut args = CommandArgs::new();
                        args.insert(
                            "kind".into(),
                            json!(element::kind_word(kind).to_lowercase()),
                        );
                        args.insert("scan".into(), json!(scan.0.to_string()));
                        args.insert("at".into(), json!(at));
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
            return InputResult::consumed();
        }
        match fit_marked(ctx, kind, Some(DEFAULT_CUTOFF)) {
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
        let a = core_document::Args(args);
        match id {
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
                let (id, element) = fit_marked(ctx, kind, cutoff).map_err(CommandError::failed)?;
                Ok(answer_of(id, &element))
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
                let (id, element) =
                    grow_at(ctx, kind, scan, at, cutoff, peel).map_err(CommandError::failed)?;
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
                let document = &*ctx.document;
                let mut ordered: Vec<_> = document
                    .feature_tree()
                    .all_nodes()
                    .filter(|(_, n)| only.is_none() || n.body == only)
                    .collect();
                ordered.sort_by_key(|(id, n)| (n.seq, **id));
                let nodes = || ordered.iter().copied();
                let elements: Vec<Value> = nodes()
                    .filter(|(_, n)| n.workbench_id.as_str() == element::KIND)
                    .filter_map(|(id, _)| Some(answer_of(*id, &element_at(document, *id)?)))
                    .collect();
                let constructions: Vec<Value> = nodes()
                    .filter(|(_, n)| n.workbench_id.as_str() == construction::KIND)
                    .filter_map(|(id, _)| {
                        let c = Construction::from_json(document.feature_values(*id)?).ok()?;
                        Some(construction_answer(*id, &c, &built_at(document, *id, 0)))
                    })
                    .collect();
                let dimensions: Vec<Value> = nodes()
                    .filter(|(_, n)| n.workbench_id.as_str() == dimension::KIND)
                    .filter_map(|(id, _)| {
                        let (dim, reading) = dimension_at(document, *id)?;
                        Some(dimension_answer(*id, &dim, &reading))
                    })
                    .collect();
                let count = |want: bool| {
                    dimensions
                        .iter()
                        .filter(|d| d["pass"].as_bool() == Some(want))
                        .count()
                };
                Ok(json!({
                    "passed": count(true),
                    "failed": count(false),
                    "elements": elements,
                    "constructions": constructions,
                    "dimensions": dimensions,
                }))
            }
            _ => Err(CommandError::Unknown(id.to_string())),
        }
    }
}

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
    /// when the element is deleted.
    #[test]
    fn a_fit_of_the_marked_surface_becomes_an_element() {
        let mut context = WorkbenchContext::default();
        InspectionWorkbench::default().configure(&mut context);
        assert_eq!(
            context.tools().len(),
            FitKind::ALL.len() + DimKind::ALL.len() + Made::ALL.len()
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
    }
}
