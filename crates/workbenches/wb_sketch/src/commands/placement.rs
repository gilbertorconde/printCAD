//! Making a sketch and placing it: on a base plane, a datum or a face, on a
//! plane of its own, and carbon copies, merges and mirrors of other sketches.

use core_document::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, FeatureId,
    ParamKind, WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::feature::SketchFeature;
use crate::sketch::{GeometryElement, Sketch};

use super::{
    add_sketch, custom_plane, editing, ids_of, made_since, placed, placing, reading, sketch,
};
use crate::SketchWorkbench;

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<SketchWorkbench>) {
    t.add(
        placing(CommandSpec::new(
            "sketch.new",
            "Make an empty sketch on a base plane",
        ))
        .optional(
            "generator",
            ParamKind::String,
            "gear, sprocket, shaft or keyway: the sketch is that generator's, at its default \
             numbers, centred on the plane's origin",
        )
        .returns("the sketch's id"),
        new_command,
    );
    t.add_free(
        sketch(
            CommandSpec::new(
                "sketch.attachment",
                "Move a sketch off what it was placed on: along its normal, across it, \
                 turned about it",
            )
            .optional("offset", ParamKind::Number, "Along the normal, mm")
            .optional("shift", ParamKind::List, "Across the plane, {x, y} in mm")
            .optional("turn", ParamKind::Number, "About the normal, degrees"),
        ),
        editing(attachment),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.set_plane",
            "Move the sketch onto another plane, its geometry kept in its own coordinates",
        ))
        .param("normal", ParamKind::List, "The plane's normal, {x, y, z}")
        .optional("origin", ParamKind::List, "Its origin, {x, y, z}")
        .optional(
            "x_axis",
            ParamKind::List,
            "The sketch's X direction, {x, y, z}",
        ),
        editing(set_plane),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.mirror_sketch",
            "A new sketch on the same plane: this one's geometry mirrored across its Y axis",
        ))
        .returns("the new sketch's id"),
        reading(mirror_sketch_command),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.merge",
            "A new sketch holding this one's geometry and other sketches', mapped onto its plane",
        ))
        .param("with", ParamKind::List, "The other sketches")
        .returns("the new sketch's id"),
        reading(merge_command),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.carbon_copy",
            "Copy another sketch's geometry into this one, mapped onto its plane",
        ))
        .param("from", ParamKind::Id, "The sketch to copy")
        .returns("{elements, constraints}: what it made"),
        editing(carbon_copy_command),
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "placement.md", include_str!("placement.md"), &[])
}

fn new_command(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let made_by = match a.opt_string("generator")? {
        Some(kind) => Some(crate::generator::Generator::named(kind).ok_or_else(|| {
            CommandError::bad("generator", "must be gear, sprocket, shaft or keyway")
        })?),
        None => None,
    };
    let name = match (a.opt_string("name")?, &made_by) {
        (Some(name), _) => name.to_string(),
        (None, Some(made_by)) => {
            crate::SketchWorkbench::next_generated_name(ctx.document, made_by.base_name())
        }
        (None, None) => crate::SketchWorkbench::next_sketch_name(ctx.document),
    };
    let id = add_sketch(&a, ctx, Sketch::new(name), made_by)?;
    Ok(json!(id.0.to_string()))
}

fn mirror_sketch_command(
    _a: &Args,
    sketch_id: FeatureId,
    _feature: SketchFeature,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    mirror_sketch(ctx.document, sketch_id).map(|id| json!(id.0.to_string()))
}

fn merge_command(
    a: &Args,
    sketch_id: FeatureId,
    _feature: SketchFeature,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let with = feature_ids(args.get("with"), "with")?;
    merge(ctx.document, sketch_id, &with).map(|id| json!(id.0.to_string()))
}

fn attachment(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let shift = match args.get("shift").filter(|v| !v.is_null()) {
        Some(v) => Some(
            v.as_array()
                .filter(|p| p.len() == 2)
                .and_then(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
                .ok_or_else(|| CommandError::bad("shift", "must be {x, y}"))?,
        ),
        None => None,
    };
    let (offset, turn) = (a.opt_number("offset")?, a.opt_number("turn")?);
    // Off a plane, a face or an attachment: moved from where it was
    // placed, which the old offset gives back.
    let Some(mut support) = feature.support.clone() else {
        let old = feature.offset;
        let mut new = old;
        new.offset = offset.unwrap_or(old.offset);
        new.shift = shift.unwrap_or(old.shift);
        new.turn = turn.unwrap_or(old.turn);
        let plane = new.apply(old.undo(feature.plane));
        feature.plane = plane;
        feature.sketch.plane = plane;
        feature.offset = new;
        return Ok(Value::Null);
    };
    if let Some(v) = offset {
        support.offset = v;
    }
    if let Some(v) = shift {
        support.shift = v;
    }
    if let Some(v) = turn {
        support.turn = v;
    }
    let values = ctx
        .document
        .feature_values(support.datum)
        .cloned()
        .ok_or_else(|| CommandError::failed("the datum is gone"))?;
    let plane = support
        .plane_from(&values)
        .ok_or_else(|| CommandError::failed("the datum is not a plane"))?;
    feature.plane = plane;
    feature.sketch.plane = plane;
    feature.support = Some(support);
    Ok(Value::Null)
}

fn set_plane(
    a: &Args,
    sketch_id: FeatureId,
    feature: &mut SketchFeature,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let plane = custom_plane(a)?;
    feature.plane = plane;
    feature.sketch.plane = plane;
    feature.offset = Default::default();
    // A plane set outright leaves the datum the sketch followed.
    if feature.support.take().is_some() {
        ctx.document
            .set_feature_dependencies(sketch_id, feature.dependencies());
    }
    Ok(Value::Null)
}

fn carbon_copy_command(
    a: &Args,
    sketch_id: FeatureId,
    feature: &mut SketchFeature,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let from = FeatureId(a.id("from")?);
    let before = ids_of(&feature.sketch);
    carbon_copy(ctx.document, sketch_id, &mut feature.sketch, from)
        .map_err(CommandError::failed)?;
    let made = made_since(&feature.sketch, &before);
    Ok(made)
}

/// Sketch ids named in a list.
fn feature_ids(value: Option<&Value>, name: &str) -> Result<Vec<FeatureId>, CommandError> {
    value
        .and_then(Value::as_array)
        .ok_or_else(|| CommandError::bad(name, "must be a list of sketch ids"))?
        .iter()
        .map(|v| {
            v.as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .map(FeatureId)
                .ok_or_else(|| CommandError::bad(name, "must be a list of sketch ids"))
        })
        .collect()
}

/// Copy sketch `from`'s geometry into `target`, the geometry of sketch
/// `target_id`, mapped from its plane onto the target's (both where their
/// bodies sit). How many elements and constraints came.
pub(crate) fn carbon_copy(
    document: &core_document::Document,
    target_id: FeatureId,
    target: &mut Sketch,
    from: FeatureId,
) -> Result<(usize, usize), String> {
    let onto = placed(document, target_id).ok_or("the sketch is not in this document")?;
    let source = placed(document, from).ok_or("`from` is not a sketch of this document")?;
    if from == target_id {
        return Err("a sketch cannot copy itself".to_string());
    }
    let xf = crate::plane_map(&source.plane, &onto.plane)?;
    Ok(crate::copy_sketch_into(&source.sketch, target, &xf))
}

/// A new sketch on `sketch`'s plane and body holding its geometry and
/// `with`'s, each mapped onto the plane.
pub(crate) fn merge(
    document: &mut core_document::Document,
    sketch: FeatureId,
    with: &[FeatureId],
) -> Result<FeatureId, CommandError> {
    let base =
        placed(document, sketch).ok_or_else(|| CommandError::bad("sketch", "is not a sketch"))?;
    let mut merged = Sketch::new(format!("{} merged", base.sketch.name));
    crate::copy_sketch_into(
        &base.sketch,
        &mut merged,
        &crate::tools::Similarity::translation(glam::DVec2::ZERO),
    );
    for other in with {
        let from = placed(document, *other)
            .ok_or_else(|| CommandError::bad("with", "holds something that is not a sketch"))?;
        let xf = crate::plane_map(&from.plane, &base.plane).map_err(CommandError::failed)?;
        crate::copy_sketch_into(&from.sketch, &mut merged, &xf);
    }
    new_beside(document, sketch, merged)
}

/// A new sketch on `sketch`'s plane and body: its geometry mirrored across
/// the sketch's Y axis.
pub(crate) fn mirror_sketch(
    document: &mut core_document::Document,
    sketch: FeatureId,
) -> Result<FeatureId, CommandError> {
    let base = crate::stored_sketch(document, sketch)
        .ok_or_else(|| CommandError::bad("sketch", "is not a sketch"))?;
    let mut mirrored = Sketch::new(format!("{} mirror", base.sketch.name));
    let all: std::collections::HashSet<Uuid> = base
        .sketch
        .geometry
        .iter()
        .map(GeometryElement::id)
        .collect();
    let xf = crate::tools::Similarity::mirror_about(glam::DVec2::ZERO, glam::DVec2::Y);
    crate::tools::copy_from(&base.sketch, &mut mirrored, &all, &xf);
    new_beside(document, sketch, mirrored)
}

/// Add `made` as a new sketch on `beside`'s stored plane and in its body.
fn new_beside(
    document: &mut core_document::Document,
    beside: FeatureId,
    mut made: Sketch,
) -> Result<FeatureId, CommandError> {
    let stored = crate::stored_sketch(document, beside)
        .ok_or_else(|| CommandError::bad("sketch", "is not a sketch"))?;
    let body = document.get_feature_meta(beside).and_then(|n| n.body);
    made.plane = stored.plane;
    let name = made.name.clone();
    document
        .add_feature_in_body(SketchFeature::new(made, stored.plane), name, body)
        .map_err(|e| CommandError::failed(e.to_string()))
}
