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
        )
        .note(
            "Every curve and loose point of the other sketch comes, its construction left \
             out; a datum comes as one element.",
        )
        .note(
            "It only guides unless `counts = true`: a sketch holding guides alone has no \
             profile, and a pad of it fails at `pc.doc.rebuild()`.",
        )
        .note(
            "It returns {elements, constraints}: the curves and the points at their ends. \
             Only the curves are external geometry, which `pc.sketch.geometry` marks \
             `external`.",
        )
        .note("`from` must be a sketch other than this one, or a datum; anything else is refused.")
        .note(
            "The elements follow their source without this sketch being opened: when the \
             other sketch or the datum moves (by hand or by a formula), `pc.sketch.geometry` \
             reads them where it now is and the next `pc.doc.rebuild()` builds from that. A \
             source curve that becomes another kind of curve, or is deleted, is caught up \
             with when this sketch is next edited.",
        )
        .note(
            "What the sketch already holds comes once: bringing the same sketch again adds \
             only what is new in it, and is refused (\"... already in the sketch ...\") when \
             nothing is.",
        )
        .see_also("sketch.external_defining")
        .see_also("sketch.carbon_copy")
        .example(
            "A circle from the sketch below, counted, padded and following its source",
            r#"
            local body = pc.doc.new_body{name = "Boss"}
            local base = pc.sketch.new{body = body, plane = "XY"}
            local circle = pc.sketch.circle{sketch = base, x = 0, y = 0, radius = 5}
            local radius = pc.sketch.constrain{sketch = base, kind = "radius", items = {circle}, value = 5}
            local top = pc.sketch.new{body = body, plane = "XY", offset = 10}
            local made = pc.sketch.external_from{sketch = top, from = base, counts = true}
            assert(#made.elements == 2, "the circle and its centre")
            pc.design.pad{sketch = top, length = 3}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = body}
            assert(math.abs(m.volume - math.pi * 25 * 3) < 0.01, m.volume)
            assert(math.abs(m.min[3] - 10) < 1e-6)
            pc.sketch.set_value{sketch = base, constraint = radius[1], value = 6}
            assert(#pc.doc.rebuild() == 0)
            m = pc.doc.measure{body = body}
            assert(math.abs(m.volume - math.pi * 36 * 3) < 0.01, "the copy follows: " .. m.volume)
            "#,
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
        )
        .note(
            "Every item must be external geometry, or the call is refused (\"... is not \
             external geometry\"); the elements `pc.sketch.geometry` marks `external` are. \
             The points at an external curve's ends and centre go with their curve, so the \
             `elements` `sketch.external`, `sketch.external_from` and `sketch.intersection` \
             return can be passed as they come.",
        )
        .note(
            "Counting takes an element out of construction; `on = false` makes it a guide \
             again.",
        )
        .see_also("sketch.external_from")
        .see_also("sketch.external")
        .example(
            "A guide made to count",
            r#"
            local body = pc.doc.new_body{name = "Boss"}
            local base = pc.sketch.new{body = body, plane = "XY"}
            pc.sketch.circle{sketch = base, x = 0, y = 0, radius = 5}
            local top = pc.sketch.new{body = body, plane = "XY", offset = 10}
            local made = pc.sketch.external_from{sketch = top, from = base}
            local pad = pc.design.pad{sketch = top, length = 3}
            assert(#pc.doc.rebuild() == 1, "a guide alone is no profile")
            pc.sketch.external_defining{sketch = top, items = made.elements}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = body}
            assert(math.abs(m.volume - math.pi * 25 * 3) < 0.01, m.volume)
            "#,
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
        .returns("{elements}: what it made")
        .note(
            "Each edge is a point on it and its direction there, in the body's own frame; \
             `pc.doc.edges` gives both (where the body sits, the same until it is moved). \
             The body must be built first (`pc.doc.rebuild()`), else it is refused (\"that \
             body has no solid shape\").",
        )
        .note(
            "It only guides unless `counts = true`; counted, the projected edges close a \
             profile as drawn lines do. An edge square to the sketch plane projects to a \
             point.",
        )
        .note(
            "It returns {elements, constraints}: the curves and the points at their ends, \
             the curves marked `external` by `pc.sketch.geometry`.",
        )
        .note(
            "The projection follows its solid: rebuilt, the edge is found again by the faces \
             it runs between (else by the point) and projected anew, the sketch unopened. \
             An edge of the sketch's own body is taken from the solid as it stands where the \
             sketch is in the history, so what the sketch builds never moves it; another \
             body's solid built from the sketch is projected again only when the sketch is \
             opened.",
        )
        .note(
            "A point names an edge only within a tenth of the body's diagonal of it; one \
             farther from every edge is refused (\"no edge near ...\"). An edge the sketch \
             already holds comes once: picked again, at any point along it, it is passed \
             over, and a call that brings nothing new is refused (\"... already in the \
             sketch ...\").",
        )
        .see_also("sketch.intersection")
        .see_also("doc.edges")
        .see_also("sketch.external_defining")
        .example(
            "A block's top edges projected and padded higher",
            r#"
            local body = pc.doc.new_body{name = "Block"}
            local base = pc.sketch.new{body = body, plane = "XY"}
            pc.sketch.rect{sketch = base, x = 0, y = 0, width = 20, height = 10}
            pc.design.pad{sketch = base, length = 5}
            assert(#pc.doc.rebuild() == 0)
            local top = pc.sketch.new{body = body, plane = "XY", offset = 5}
            local made = pc.sketch.external{sketch = top, counts = true, edges = {
              {body = body, point = {10, 0, 5}, direction = {1, 0, 0}},
              {body = body, point = {20, 5, 5}, direction = {0, 1, 0}},
              {body = body, point = {10, 10, 5}, direction = {1, 0, 0}},
              {body = body, point = {0, 5, 5}, direction = {0, 1, 0}},
            }}
            assert(#made.elements > 0)
            pc.design.pad{sketch = top, length = 3}
            assert(#pc.doc.rebuild() == 0, "the four top edges close a profile")
            assert(math.abs(pc.doc.measure{body = body}.volume - 20 * 10 * 8) < 1e-3)
            "#,
        ),
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
        .returns("{elements}: what it made")
        .note(
            "Each face is a point on it and its outward normal there, in the body's own \
             frame; `pc.doc.faces` gives both. The body must be built first.",
        )
        .note(
            "A face the sketch plane does not cross adds nothing. What it adds only guides \
             unless `counts = true`.",
        )
        .note("It returns {elements, constraints}: the curves and the points at their ends.")
        .see_also("sketch.external")
        .see_also("doc.faces")
        .example(
            "Where a block's top face crosses a sketch through its middle",
            r#"
            local body = pc.doc.new_body{name = "Block"}
            local base = pc.sketch.new{body = body, plane = "XY"}
            pc.sketch.rect{sketch = base, x = 0, y = 0, width = 20, height = 10}
            pc.design.pad{sketch = base, length = 5}
            assert(#pc.doc.rebuild() == 0)
            local cut = pc.sketch.new{body = body, plane = "XZ", offset = -5}
            local made = pc.sketch.intersection{sketch = cut, faces = {{body = body, point = {10, 5, 5}, normal = {0, 0, 1}}}}
            assert(#made.elements == 3, "a line and its two ends")
            for _, e in ipairs(pc.sketch.geometry{sketch = cut}) do
              if e.kind == "line" then
                assert(e.external and e.points[1][2] == 5 and e.points[2][2] == 5, "the top face, at sketch y = 5")
              end
            end
            "#,
        ),
        editing(intersection),
    );
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
