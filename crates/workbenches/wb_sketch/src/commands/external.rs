//! External geometry: edges, faces, sketches and datums projected in, their
//! intersections with the plane, and whether they count in profiles.

use core_document::command::vec3_of;
use core_document::{
    Args, CommandError, CommandResult, CommandSpec, CommandTable, FeatureId, ParamKind,
    WorkbenchRuntimeContext,
};
use serde_json::Value;
use uuid::Uuid;

use crate::sketch::{GeometryElement, Sketch, SketchPlane};

use super::editing;
use super::{ids, ids_of, made_since, sketch};
use crate::SketchWorkbench;
use crate::feature::SketchFeature;

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<SketchWorkbench>) {
    t.add_free(
        sketch(
            CommandSpec::new(
                "sketch.external_from",
                "Bring another sketch's curves and points, or a datum, into this sketch as \
                 external geometry that follows them",
            )
            .param("from", ParamKind::Id, "A sketch or a datum")
            .optional(
                "counts",
                ParamKind::Bool,
                "true: it counts in the profile, as drawn geometry does; false (the default): \
                 it only guides the sketch",
            )
            .returns("the external elements made"),
        ),
        editing(external_from),
    );
    t.add_free(
        sketch(
            CommandSpec::new(
                "sketch.external_defining",
                "Count external geometry in the sketch's profiles, or leave it only guiding",
            )
            .param("items", ParamKind::List, "External elements' ids")
            .optional(
                "on",
                ParamKind::Bool,
                "true counts them (the default), false stops",
            ),
        ),
        editing(external_defining),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.external",
            "Project edges of solids into the sketch as fixed references",
        ))
        .param(
            "edges",
            ParamKind::List,
            "Each {body, point, direction}: a point on the edge and its direction, \
             in the body's own frame",
        )
        .optional(
            "counts",
            ParamKind::Bool,
            "true: it counts in the profile, as drawn geometry does; false (the default): it \
             only guides the sketch",
        )
        .returns("{elements}: what it made"),
        editing(external),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.intersection",
            "Add where faces of solids cross the sketch plane, as fixed references",
        ))
        .param(
            "faces",
            ParamKind::List,
            "Each {body, point, normal}: a point on the face and its normal there, \
             in the body's own frame",
        )
        .optional(
            "counts",
            ParamKind::Bool,
            "true: it counts in the profile, as drawn geometry does; false (the default): it \
             only guides the sketch",
        )
        .returns("{elements}: what it made"),
        editing(intersection),
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "external.md", include_str!("external.md"), &[])
}

fn external_from(
    a: &Args,
    sketch_id: FeatureId,
    feature: &mut SketchFeature,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let from = FeatureId(a.id("from")?);
    if from == sketch_id {
        return Err(CommandError::bad("from", "must be another sketch"));
    }
    let sources = reference_sources(ctx.document, from)
        .ok_or_else(|| CommandError::bad("from", "must be a sketch or a datum"))?;
    let sources = counting(a, sources)?;
    let before = ids_of(&feature.sketch);
    let placed = crate::placed_plane(
        &feature.plane,
        &crate::sketch_placement(ctx.document, sketch_id),
    );
    add_external(ctx, &placed, &mut feature.sketch, &sources).map_err(CommandError::failed)?;
    let made = made_since(&feature.sketch, &before);
    Ok(made)
}

fn external_defining(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let items = ids(args.get("items"), "items", &feature.sketch)?;
    // The points of an external curve go with it: the list
    // `sketch.external` answers with is taken as it comes.
    let on_external: std::collections::HashSet<Uuid> = feature
        .sketch
        .geometry
        .iter()
        .filter(|g| feature.sketch.external.contains_key(&g.id()))
        .flat_map(Sketch::curve_point_ids)
        .collect();
    let items: Vec<Uuid> = items
        .into_iter()
        .filter(|i| feature.sketch.external.contains_key(i) || !on_external.contains(i))
        .collect();
    if let Some(bad) = items
        .iter()
        .find(|i| !feature.sketch.external.contains_key(i))
    {
        return Err(CommandError::bad(
            "items",
            format!("{bad} is not external geometry"),
        ));
    }
    set_external_defining(
        &mut feature.sketch,
        &items,
        a.opt_bool("on")?.unwrap_or(true),
    );
    Ok(Value::Null)
}

fn intersection(
    a: &Args,
    sketch_id: FeatureId,
    feature: &mut SketchFeature,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let faces = counting(a, section_sources(args.get("faces"))?)?;
    let before = ids_of(&feature.sketch);
    let placed = crate::placed_plane(
        &feature.plane,
        &crate::sketch_placement(ctx.document, sketch_id),
    );
    add_external(ctx, &placed, &mut feature.sketch, &faces).map_err(CommandError::failed)?;
    let made = made_since(&feature.sketch, &before);
    Ok(made)
}

fn external(
    a: &Args,
    sketch_id: FeatureId,
    feature: &mut SketchFeature,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let edges = counting(a, external_sources(args.get("edges"))?)?;
    let before = ids_of(&feature.sketch);
    let placed = crate::placed_plane(
        &feature.plane,
        &crate::sketch_placement(ctx.document, sketch_id),
    );
    let added =
        add_external(ctx, &placed, &mut feature.sketch, &edges).map_err(CommandError::failed)?;
    if added == 0 {
        return Err(CommandError::failed("no edge could be projected"));
    }
    let made = made_since(&feature.sketch, &before);
    Ok(made)
}

/// `sources` counting in the profile or only guiding, as the call's
/// `counts` says (guiding when it says nothing).
fn counting(
    a: &Args,
    mut sources: Vec<crate::sketch::ExternalSource>,
) -> Result<Vec<crate::sketch::ExternalSource>, CommandError> {
    let counts = a.opt_bool("counts")?.unwrap_or(false);
    for source in &mut sources {
        source.defining = counts;
    }
    Ok(sources)
}

/// Edges named as `{body, point, direction}`, in each body's own frame.
fn external_sources(
    value: Option<&Value>,
) -> Result<Vec<crate::sketch::ExternalSource>, CommandError> {
    let bad = || CommandError::bad("edges", "must be a list of {body, point, direction}");
    let list = value.and_then(Value::as_array).ok_or_else(bad)?;
    list.iter()
        .map(|edge| {
            let body = edge
                .get("body")
                .and_then(Value::as_str)
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(bad)?;
            let v = |name: &str| vec3_of(edge.get(name), "edges");
            Ok(crate::sketch::ExternalSource {
                body,
                point: v("point")?,
                direction: v("direction")?,
                section: false,
                defining: false,
                reference: None,
                faces: [0; 2],
            })
        })
        .collect()
}

/// Faces named as `{body, point, normal}`, in each body's own frame.
fn section_sources(
    value: Option<&Value>,
) -> Result<Vec<crate::sketch::ExternalSource>, CommandError> {
    let bad = || CommandError::bad("faces", "must be a list of {body, point, normal}");
    let list = value.and_then(Value::as_array).ok_or_else(bad)?;
    list.iter()
        .map(|face| {
            let body = face
                .get("body")
                .and_then(Value::as_str)
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(bad)?;
            let v = |name: &str| vec3_of(face.get(name), "faces");
            Ok(crate::sketch::ExternalSource {
                body,
                point: v("point")?,
                direction: v("normal")?,
                section: true,
                defining: false,
                reference: None,
                faces: [0; 2],
            })
        })
        .collect()
}

/// Bring `edges` (edges to project, faces to cut) onto `plane` (the
/// sketch's, where its body sits) and add what they come to as external
/// geometry. How many elements came.
pub(crate) fn add_external(
    ctx: &WorkbenchRuntimeContext,
    plane: &SketchPlane,
    sketch: &mut Sketch,
    edges: &[crate::sketch::ExternalSource],
) -> Result<usize, String> {
    let mut added = 0;
    let mut repeated = 0;
    let mut last_error = None;
    for source in edges {
        let source = &crate::with_edge_names(ctx.document, *source);
        match crate::project_source(ctx, plane, source) {
            Ok(curves) => {
                // An edge or element the sketch already holds comes once.
                let held = crate::external::groups(sketch)
                    .iter()
                    .any(|(_, group)| crate::external::already_holds(sketch, group, &curves));
                if held {
                    repeated += 1;
                    continue;
                }
                for curve in &curves {
                    added += crate::external::add(sketch, curve, *source);
                }
            }
            Err(why) => last_error = Some(why),
        }
    }
    match (added, last_error) {
        (0, Some(why)) => Err(why),
        (0, None) if repeated > 0 => Err(match repeated {
            1 => "that is already in the sketch as external geometry".to_string(),
            _ => "those are already in the sketch as external geometry".to_string(),
        }),
        _ => Ok(added),
    }
}

/// What `from` brings as external geometry: each curve and loose point of a
/// sketch (its construction left out), or a datum; `None` for anything
/// else.
pub(crate) fn reference_sources(
    document: &core_document::Document,
    from: FeatureId,
) -> Option<Vec<crate::sketch::ExternalSource>> {
    use crate::sketch::{ExternalReference, ExternalSource};
    let node = document.get_feature_meta(from)?;
    if node.workbench_id.as_str() == core_document::DATUM_KIND {
        return Some(vec![ExternalSource::of_reference(
            ExternalReference::Datum { datum: from.0 },
        )]);
    }
    let other = crate::stored_sketch(document, from)?;
    let sketch = &other.sketch;
    let on_curves: std::collections::HashSet<Uuid> = sketch
        .geometry
        .iter()
        .flat_map(Sketch::curve_point_ids)
        .collect();
    Some(
        sketch
            .geometry
            .iter()
            .filter(|g| !sketch.is_construction(g.id()) && !sketch.is_external(g.id()))
            .filter(|g| !matches!(g, GeometryElement::Point(p) if on_curves.contains(&p.id)))
            .map(|g| {
                ExternalSource::of_reference(ExternalReference::SketchElement {
                    sketch: from.0,
                    element: g.id(),
                })
            })
            .collect(),
    )
}

/// Mark `items` of the sketch's external geometry as counting in its
/// profiles, or not.
pub(crate) fn set_external_defining(sketch: &mut Sketch, items: &[Uuid], on: bool) {
    for id in items {
        if let Some(source) = sketch.external.get_mut(id) {
            source.defining = on;
            // Counting in the profile is what makes it normal geometry.
            if on {
                sketch.construction.remove(id);
            }
        }
    }
}
