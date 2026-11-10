//! What a body's build reads beyond its own features: the datums and
//! sketches that follow its solid, and the bodies its booleans and borrows
//! take as inputs.

use super::design_features_of_body;
use core_document::BuildPlan;
use core_document::{BodyId, Document, FeatureId, WorkbenchFeature};

use crate::feature::{BorrowSource, DesignFeature};

/// The datums of `body` that stand on its solid, and its sketches that
/// project its solid's edges, each with what it asks of the solid summed
/// up. A datum is read as it was made: what a build found is the answer,
/// never the question. A sketch asks onto its plane as evaluation left it.
pub(super) fn following_datums(document: &Document, body: BodyId) -> Vec<(FeatureId, u64)> {
    use std::hash::{Hash, Hasher};
    let mut asking: Vec<(FeatureId, std::collections::hash_map::DefaultHasher)> = Vec::new();
    let mut add = |id: FeatureId, asked: String| match asking.iter_mut().find(|(f, _)| *f == id) {
        Some((_, hasher)) => asked.hash(hasher),
        None => {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            asked.hash(&mut hasher);
            asking.push((id, hasher));
        }
    };
    for (id, _, probes) in datums_asking(document, body) {
        add(id, serde_json::to_string(&probes).unwrap_or_default());
    }
    for (id, _, asked) in sketches_projecting(document, body) {
        add(id, serde_json::to_string(&asked).unwrap_or_default());
    }
    asking.into_iter().map(|(id, h)| (id, h.finish())).collect()
}

/// The sketches of `body` that take geometry from its own solid: id, place
/// in the history and what each projects, onto its plane as evaluation
/// left it (a sketch on a face where the face is).
fn sketches_projecting(
    document: &Document,
    body: BodyId,
) -> Vec<(FeatureId, u64, Vec<kernel_api::ProjectionProbe>)> {
    let mut sketches: Vec<(FeatureId, u64, Vec<kernel_api::ProjectionProbe>)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| {
            n.body == Some(body) && !n.suppressed && n.workbench_id.as_str() == wb_sketch::KIND
        })
        .filter_map(|(id, n)| {
            let data = document.feature_values(*id).unwrap_or(&n.data);
            let asked = wb_sketch::SketchFeature::from_json(data)
                .ok()?
                .own_projections(body);
            (!asked.is_empty()).then_some((*id, n.seq, asked))
        })
        .collect();
    sketches.sort_by_key(|(id, seq, _)| (*seq, *id));
    sketches
}

/// What the sketches of `body` project of its solid, each of the solid the
/// features before the sketch make.
pub fn sketch_projections(
    document: &Document,
    body: BodyId,
    plan: &BuildPlan,
) -> Vec<core_document::PlanProjection> {
    let seq_of = |id: &FeatureId| document.get_feature_meta(*id).map(|n| n.seq);
    sketches_projecting(document, body)
        .into_iter()
        .flat_map(|(feature, seq, asked)| {
            let after_op = plan
                .op_features
                .iter()
                .take_while(|f| seq_of(f).is_some_and(|s| s < seq))
                .count();
            asked
                .into_iter()
                .map(move |probe| core_document::PlanProjection {
                    feature,
                    projection: kernel_api::ChainProjection { after_op, probe },
                })
        })
        .collect()
}

/// Tell each sketch of `body` placed on a face a borrow lends where the
/// face is now, as the borrow finds it on its source, the way a build
/// answers a sketch on the body's own solid.
pub(super) fn answer_lent_faces(document: &mut Document, body: BodyId) {
    let lent: Vec<(FeatureId, wb_sketch::FaceSupport)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.body == Some(body) && n.workbench_id.as_str() == wb_sketch::KIND)
        .filter_map(|(id, n)| {
            let face = wb_sketch::SketchFeature::from_json(&n.data).ok()?.face?;
            face.lent_by.is_some().then_some((*id, face))
        })
        .collect();
    for (sketch, face) in lent {
        let Some(lent_by) = face.lent_by else {
            continue;
        };
        let answer = crate::borrow::lent_face(document, lent_by.borrow, lent_by.index)
            .map(|(point, normal)| kernel_api::ProbeAnswer::Face {
                point: point.map(f64::from),
                normal: normal.map(f64::from),
                surface: kernel_api::FaceSurface::Plane {
                    origin: point,
                    normal,
                },
            })
            .ok_or_else(|| "the borrow no longer lends a flat face there".to_string());
        document.set_probed_references(
            sketch,
            core_document::ProbedReferences {
                probes: vec![face.probe()],
                answers: vec![answer],
            },
        );
    }
}

/// The datums of `body`, and its sketches placed on its faces, with
/// references to find again on its solid: id, place in the history and
/// the probes.
fn datums_asking(
    document: &Document,
    body: BodyId,
) -> Vec<(FeatureId, u64, Vec<kernel_api::ShapeProbe>)> {
    let mut datums: Vec<(FeatureId, u64, Vec<kernel_api::ShapeProbe>)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.body == Some(body) && !n.suppressed)
        .filter_map(|(id, n)| {
            let probes = match n.workbench_id.as_str() {
                core_document::DATUM_KIND => core_document::DatumFeature::from_json(&n.data)
                    .ok()?
                    .probes(),
                // A primitive attached by a mode asks what the attachment
                // asks.
                // A feature starting on a face of the solid asks where the
                // face is.
                crate::KIND => match DesignFeature::from_json(&n.data).ok()? {
                    DesignFeature::Primitive {
                        attached: Some(attached),
                        ..
                    } => attached.probes(),
                    feature => vec![feature.start_face()?.probe()],
                },
                wb_sketch::KIND => {
                    let sketch = wb_sketch::SketchFeature::from_json(&n.data).ok()?;
                    // Attached by a mode, it asks what the attachment asks.
                    match sketch.attached {
                        Some(attached) => attached.probes(),
                        // A face a borrow lends is on another body's solid:
                        // `answer_lent_faces` finds it, not this body's build.
                        None => sketch
                            .face
                            .iter()
                            .filter(|face| face.lent_by.is_none())
                            .map(|face| face.probe())
                            .collect(),
                    }
                }
                _ => return None,
            };
            (!probes.is_empty()).then_some((*id, n.seq, probes))
        })
        .collect();
    datums.sort_by_key(|(id, seq, _)| (*seq, *id));
    datums
}

/// What the datums of `body` ask of its solid, each of the solid the
/// features before it make.
pub fn datum_probes(
    document: &Document,
    body: BodyId,
    plan: &BuildPlan,
) -> Vec<core_document::PlanProbe> {
    let seq_of = |id: &FeatureId| document.get_feature_meta(*id).map(|n| n.seq);
    datums_asking(document, body)
        .into_iter()
        .flat_map(|(feature, seq, probes)| {
            let after_op = plan
                .op_features
                .iter()
                .take_while(|f| seq_of(f).is_some_and(|s| s < seq))
                .count();
            probes
                .into_iter()
                .map(move |probe| core_document::PlanProbe {
                    feature,
                    probe: kernel_api::ChainProbe { after_op, probe },
                })
        })
        .collect()
}

/// The Booleans and the Clone of `body`'s history that are not
/// suppressed, each with the body whose solid it takes: a Boolean's tool,
/// a Clone's source. The body builds after them and again when they do.
fn boolean_tools(document: &Document, body: BodyId) -> Vec<(FeatureId, BodyId)> {
    design_features_of_body(document, body)
        .into_iter()
        .filter(|(id, _)| !document.get_feature_meta(*id).is_some_and(|n| n.suppressed))
        .flat_map(|(id, feature)| match feature {
            DesignFeature::BodyBoolean {
                tool_body,
                more_tools,
                ..
            } => std::iter::once(tool_body)
                .chain(more_tools)
                .map(|tool| (id, tool))
                .collect::<Vec<_>>(),
            DesignFeature::Clone { source } => vec![(id, source)],
            _ => Vec::new(),
        })
        .collect()
}

/// Each Boolean or Clone of `body` with what the solids it takes are now,
/// all of them in one sum: a Boolean with several tools is out of date
/// when any of them is. A tool that takes this body as a tool in turn
/// follows nothing: the Boolean is refused, and the two would rebuild
/// each other forever.
pub(super) fn boolean_inputs(document: &Document, body: BodyId) -> Vec<(FeatureId, u64)> {
    use std::hash::{Hash, Hasher};
    let mut out: Vec<(FeatureId, std::collections::hash_map::DefaultHasher)> = Vec::new();
    for (feature, tool) in boolean_tools(document, body) {
        if tools_reach(document, tool, body) {
            continue;
        }
        let inputs = tool_inputs(document, body, tool);
        match out.iter_mut().find(|(f, _)| *f == feature) {
            Some((_, hasher)) => inputs.hash(hasher),
            None => {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                inputs.hash(&mut hasher);
                out.push((feature, hasher));
            }
        }
    }
    out.into_iter().map(|(f, h)| (f, h.finish())).collect()
}

/// Each live borrow of `body` with what it follows now. A borrow of a
/// body built from this one in turn follows nothing: the two would rebuild
/// each other forever, and what stands on it says so instead.
pub(super) fn borrow_inputs(document: &Document, body: BodyId) -> Vec<(FeatureId, u64)> {
    crate::borrow::borrows_of_body(document, body)
        .into_iter()
        .filter(|(_, borrow)| borrow.frozen.is_none())
        .filter(|(_, borrow)| match &borrow.source {
            BorrowSource::Solid { body: source, .. } => !bodies_reach(document, *source, body),
            BorrowSource::Sketch(_) => true,
        })
        .map(|(id, borrow)| (id, crate::borrow::inputs(document, body, &borrow.source)))
        .collect()
}

/// The bodies whose solids `body` is built against: the tool bodies of its
/// Booleans and the sources of the faces and edges it borrows live.
pub(super) fn followed_bodies(document: &Document, body: BodyId) -> Vec<BodyId> {
    let mut followed: Vec<BodyId> = boolean_tools(document, body)
        .into_iter()
        .map(|(_, tool)| tool)
        .collect();
    for (_, borrow) in crate::borrow::borrows_of_body(document, body) {
        if let (None, BorrowSource::Solid { body: source, .. }) = (&borrow.frozen, &borrow.source) {
            followed.push(*source);
        }
    }
    followed
}

/// Whether `to` is among the bodies `from` is built against, or theirs,
/// and so on: a body that reaches itself this way can never be built.
pub(crate) fn bodies_reach(document: &Document, from: BodyId, to: BodyId) -> bool {
    let mut stack = vec![from];
    let mut seen: Vec<BodyId> = Vec::new();
    while let Some(body) = stack.pop() {
        if body == to {
            return true;
        }
        if seen.contains(&body) {
            continue;
        }
        seen.push(body);
        stack.extend(followed_bodies(document, body));
    }
    false
}

/// Whether `to` is among the tool bodies `from` takes, or theirs, and so
/// on: a body that reaches itself this way can never be built.
pub(super) fn tools_reach(document: &Document, from: BodyId, to: BodyId) -> bool {
    let mut stack = vec![from];
    let mut seen: Vec<BodyId> = Vec::new();
    while let Some(body) = stack.pop() {
        if body == to {
            return true;
        }
        if seen.contains(&body) {
            continue;
        }
        seen.push(body);
        stack.extend(
            boolean_tools(document, body)
                .into_iter()
                .map(|(_, tool)| tool),
        );
    }
    false
}

/// What a Boolean of `body` is built against, summed up: the tool body's
/// solid (its geometry revision, none before it has one) and where the
/// tool sits relative to the body.
fn tool_inputs(document: &Document, body: BodyId, tool: BodyId) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    document
        .imported_geometry(tool)
        .map(|g| g.revision)
        .hash(&mut hasher);
    let relative = document
        .body_placement(body)
        .inverse()
        .after(&document.body_placement(tool));
    for row in relative.rows() {
        for value in row {
            value.to_bits().hash(&mut hasher);
        }
    }
    hasher.finish()
}
