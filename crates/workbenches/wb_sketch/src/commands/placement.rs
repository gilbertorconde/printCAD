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
        .returns("the sketch's id")
        .note(
            "Without `body` the sketch goes in the selected body, else in a new one: in a \
             script two sketches made without `body` land in two bodies, and a pocket or a \
             hole from the second is refused for want of material. Give `body` from \
             `pc.doc.feature{id = ...}.body` or `pc.doc.new_body`.",
        )
        .note(
            "`body` takes any body, a surface body too: that is how a sketch starts in a \
             surface body, as the Surface bench's Create sketch does.",
        )
        .note(
            "XY faces +Z, YZ faces +X and XZ faces -Y, so a pad from an XZ sketch grows \
             toward -Y and `offset` moves an XZ sketch toward -Y. The sketch's x and y run \
             along the plane's two letters (on XZ, y is world Z). Lower case names are \
             taken too.",
        )
        .note(
            "With `generator` it is named after it (Sprocket, Sprocket_1, ...) and \
             `sketch.generator` sets its numbers, as `design.sprocket` and its kin do on a \
             base plane or a picked face.",
        )
        .see_also("doc.new_body")
        .see_also("sketch.rect")
        .see_also("design.datum")
        .example(
            "Two sketches in one body",
            r#"
            local body = pc.doc.new_body{name = "Bracket"}
            local base = pc.sketch.new{body = body, plane = "XY"}
            local side = pc.sketch.new{body = body, plane = "XZ", offset = 5}
            assert(pc.doc.feature{id = base}.body == body)
            assert(pc.doc.feature{id = side}.body == body)
            assert(#pc.doc.bodies() == 1, "both sketches went in the one body")
            "#,
        ),
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
        )
        .note(
            "A sketch on a datum moves on the datum and follows it; any other moves from the \
             plane, face or attachment it was placed on, and a face or attachment still \
             takes it along. Placing the sketch again (`sketch.set_plane`) drops the offset.",
        )
        .note(
            "Each value given replaces the one the sketch had rather than adding to it: \
             `offset = 5` twice leaves it 5 mm off.",
        )
        .note(
            "`shift` runs along the plane's own x and y; `turn` is degrees about its normal.",
        )
        .see_also("sketch.new")
        .see_also("design.datum")
        .see_also("sketch.set_plane")
        .example(
            "A sketch set 5 mm off its datum",
            r#"
            local body = pc.doc.new_body{name = "Plate"}
            local datum = pc.design.datum{body = body, kind = "plane", plane = "XY", offset = {0, 0, 10}}
            local s = pc.sketch.new{body = body, on = datum}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 4, height = 2}
            pc.sketch.attachment{sketch = s, offset = 5}
            pc.sketch.attachment{sketch = s, offset = 5}
            pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = body}
            assert(math.abs(m.min[3] - 15) < 1e-6, "5 mm off the datum at 10, however often it is set")
            "#,
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
        )
        .note(
            "The geometry keeps its sketch coordinates and moves with the plane. Without \
             `x_axis` the sketch's x is a direction square to the normal chosen for it (+Y \
             for a normal along X), so give `x_axis` to know which way the geometry lies.",
        )
        .note(
            "The plane given is fixed: a sketch made on a datum stops following it, and \
             `sketch.attachment` refuses it from then on.",
        )
        .see_also("sketch.attachment")
        .see_also("sketch.new")
        .example(
            "A sketch moved onto a plane facing +X",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 4}
            pc.sketch.set_plane{sketch = s, normal = {1, 0, 0}, origin = {5, 0, 0}, x_axis = {0, 1, 0}}
            local pad = pc.design.pad{sketch = s, length = 2}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.min[1] - 5) < 1e-6 and math.abs(m.max[1] - 7) < 1e-6, "padded along +X from x = 5")
            assert(math.abs(m.max[2] - 10) < 1e-6 and math.abs(m.max[3] - 4) < 1e-6, "sketch x along Y, y along Z")
            "#,
        ),
        editing(set_plane),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.mirror_sketch",
            "A new sketch on the same plane: this one's geometry mirrored across its Y axis",
        ))
        .returns("the new sketch's id")
        .note(
            "It makes a new sketch, named after this one with \"mirror\", in the same body; \
             this one is left as it is. The mirror is across the sketch's own Y axis: x \
             becomes -x.",
        )
        .note(
            "For both halves in one profile, `sketch.merge` the two, or mirror within one \
             sketch with `sketch.draw`'s mirror tool.",
        )
        .see_also("sketch.merge")
        .see_also("sketch.draw")
        .example(
            "A rectangle mirrored across the Y axis",
            r#"
            local body = pc.doc.new_body{name = "Wing"}
            local s = pc.sketch.new{body = body, plane = "XY"}
            pc.sketch.rect{sketch = s, x = 5, y = 0, width = 10, height = 5}
            local mirrored = pc.sketch.mirror_sketch{sketch = s}
            assert(pc.doc.feature{id = mirrored}.body == body)
            assert(#pc.sketch.geometry{sketch = s} == 8, "the original keeps only its own")
            pc.design.pad{sketch = mirrored, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = body}
            assert(math.abs(m.min[1] + 15) < 1e-6 and math.abs(m.max[1] + 5) < 1e-6, "across the sketch's Y axis")
            "#,
        ),
        reading(mirror_sketch_command),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.merge",
            "A new sketch holding this one's geometry and other sketches', mapped onto its plane",
        ))
        .param("with", ParamKind::List, "The other sketches")
        .returns("the new sketch's id")
        .note(
            "It makes a new sketch, named after this one with \"merged\", on this one's plane \
             and in its body; the sketches merged are left as they are. Constraints come \
             with the geometry; under a turn or a mirror those stated against the sketch \
             axes follow it (swapped on a quarter turn, made parallel or perpendicular to \
             one another at any other angle). Projected geometry stays projected from its \
             solid, sketch or datum.",
        )
        .note(
            "Each sketch in `with` must lie on a plane parallel to this one (\"its plane is \
             not parallel to this one\"); its geometry is laid onto this plane, the distance \
             between them dropped.",
        )
        .see_also("sketch.carbon_copy")
        .see_also("sketch.mirror_sketch")
        .example(
            "Two sketches merged and padded as one",
            r#"
            local body = pc.doc.new_body{name = "Pair"}
            local left = pc.sketch.new{body = body, plane = "XY"}
            pc.sketch.rect{sketch = left, x = -15, y = 0, width = 10, height = 5}
            local right = pc.sketch.new{body = body, plane = "XY", offset = 3}
            pc.sketch.circle{sketch = right, x = 10, y = 2, radius = 2}
            local merged = pc.sketch.merge{sketch = left, with = {right}}
            assert(#pc.sketch.geometry{sketch = merged} == 10)
            pc.design.pad{sketch = merged, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = body}
            assert(math.abs(m.volume - (50 + math.pi * 4)) < 0.01, m.volume)
            assert(math.abs(m.max[3] - 1) < 1e-6, "on the first sketch's plane")
            "#,
        ),
        reading(merge_command),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.carbon_copy",
            "Copy another sketch's geometry into this one, mapped onto its plane",
        ))
        .param("from", ParamKind::Id, "The sketch to copy")
        .returns("{elements, constraints}: what it made")
        .note(
            "The geometry comes with its constraints, as geometry of this sketch: free to \
             edit, not tied to the sketch it came from. Under a turn or a mirror the \
             constraints stated against the sketch axes follow it, and projected geometry \
             stays projected from its solid, sketch or datum.",
        )
        .note(
            "`from` must lie on a plane parallel to this one (\"its plane is not parallel to \
             this one\"); a sketch cannot copy itself.",
        )
        .see_also("sketch.merge")
        .see_also("sketch.external_from")
        .example(
            "A rectangle copied onto a sketch above it",
            r#"
            local body = pc.doc.new_body{name = "Stack"}
            local base = pc.sketch.new{body = body, plane = "XY"}
            pc.sketch.rect{sketch = base, x = 0, y = 0, width = 10, height = 5}
            local top = pc.sketch.new{body = body, plane = "XY", offset = 5}
            local made = pc.sketch.carbon_copy{sketch = top, from = base}
            assert(#made.elements == 8 and #made.constraints == 4, "the lines, their ends and their constraints")
            pc.design.pad{sketch = top, length = 2}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = body}
            assert(math.abs(m.volume - 100) < 1e-3 and math.abs(m.min[3] - 5) < 1e-6)
            "#,
        ),
        editing(carbon_copy_command),
    );
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
