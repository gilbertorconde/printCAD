//! Moving features through histories: borrowed geometry frozen or mapped,
//! a feature moved to another body or duplicated.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, FeatureId, ParamKind,
    WorkbenchContext, WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::Value;

use crate::feature::DesignFeature;

/// `design.freeze`: a borrow takes its source as it is now, or follows it
/// again.
fn freeze(a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let id = FeatureId(a.id("feature")?);
    let not_a_borrow = || CommandError::bad("feature", "is not borrowed geometry");
    let body = ctx
        .document
        .get_feature_meta(id)
        .and_then(|n| n.body)
        .ok_or_else(not_a_borrow)?;
    let Some(DesignFeature::Borrow {
        source, options, ..
    }) = ctx
        .document
        .get_feature_data(id)
        .and_then(|d| DesignFeature::from_json(d).ok())
    else {
        return Err(not_a_borrow());
    };
    let frozen = if a.opt_bool("frozen")?.unwrap_or(true) {
        Some(
            crate::borrow::freeze(ctx.document, ctx.kernel, body, &source, &options)
                .map_err(CommandError::failed)?,
        )
    } else {
        None
    };
    let feature = DesignFeature::Borrow {
        source,
        frozen,
        options,
    };
    ctx.document
        .update_feature_data(id, feature.to_json())
        .map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document
        .set_feature_dependencies(id, feature.dependencies());
    ctx.document.mark_feature_dirty(id);
    Ok(Value::Null)
}

fn move_to_body(a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let id = FeatureId(a.id("feature")?);
    let body = BodyId(a.id("body")?);
    move_feature(ctx, id, body)
        .map(|moved| Value::from(moved.iter().map(|f| f.0.to_string()).collect::<Vec<_>>()))
        .map_err(CommandError::failed)
}

/// Copy `id` into `body`, its own when `None`.
pub(crate) fn duplicate(
    ctx: &mut WorkbenchRuntimeContext,
    id: FeatureId,
    body: Option<BodyId>,
) -> Result<Vec<FeatureId>, String> {
    let copied = crate::clipboard::Clipboard::copy(ctx.document, id)
        .ok_or("the feature belongs to no body")?;
    let body = match body {
        Some(body) if ctx.document.body(body).is_some() => body,
        Some(_) => return Err("no such body".into()),
        None => ctx
            .document
            .get_feature_meta(id)
            .and_then(|n| n.body)
            .ok_or("the feature belongs to no body")?,
    };
    Ok(copied.paste(ctx.document, body))
}

/// Move `id` into `body` and rebuild both bodies.
pub(crate) fn move_feature(
    ctx: &mut WorkbenchRuntimeContext,
    id: FeatureId,
    body: BodyId,
) -> Result<Vec<FeatureId>, String> {
    let from = ctx
        .document
        .get_feature_meta(id)
        .and_then(|n| n.body)
        .ok_or("the feature belongs to no body")?;
    let moved = ctx.document.move_feature_to_body(id, body)?;
    crate::invalidate_body(ctx.document, from);
    crate::invalidate_body(ctx.document, body);
    Ok(moved)
}

pub(super) fn owns(id: &str) -> bool {
    matches!(
        id,
        "design.freeze" | "design.map_sketch" | "design.move_to_body" | "design.duplicate"
    )
}

pub(super) fn configure(context: &mut WorkbenchContext) {
    context.register_command(
        CommandSpec::new(
            "design.freeze",
            "Freeze borrowed geometry as it is now, or let it follow its source again",
        )
        .param("feature", ParamKind::Id, "The borrow")
        .optional(
            "frozen",
            ParamKind::Bool,
            "true (the default) takes the source as it is now; false follows it again",
        )
        .returns("nothing")
        .note("Any feature other than a borrow is refused.")
        .see_also("design.borrow")
        .example(
            "A borrowed sketch frozen, then followed again",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local body = pc.doc.new_body{}
            local lent = pc.design.borrow{body = body, sketch = s}
            pc.design.pad{sketch = lent, length = 3}
            pc.design.freeze{feature = lent}
            -- A rectangle added to the source after the freeze is not taken.
            pc.sketch.rect{sketch = s, x = 30, y = 0, width = 10, height = 10}
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = body}.volume - 600) < 1e-3)
            pc.design.freeze{feature = lent, frozen = false}
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = body}.volume - 900) < 1e-3, "following it again")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "design.map_sketch",
            "Put an existing sketch on a flat face its body borrows from another body; the \
             sketch then follows the face",
        )
        .param("sketch", ParamKind::Id, "The sketch")
        .param(
            "borrow",
            ParamKind::Id,
            "A borrow of faces in the sketch's body",
        )
        .optional(
            "index",
            ParamKind::Integer,
            "Which of the borrow's faces, from 0; its first flat one when left out",
        )
        .returns("nothing")
        .note(
            "The sketch keeps its geometry in its own coordinates and moves with the plane; \
             a datum, mode or face it stood on before is left.",
        )
        .note(
            "A borrow of another body, a curved face, or a face the lender has not built yet \
             (`pc.doc.rebuild()` first) is refused.",
        )
        .see_also("design.borrow")
        .see_also("sketch.set_plane")
        .example(
            "A post's sketch moved onto a face another body lends",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 20}
            local block = pc.design.pad{sketch = s, length = 10}
            local lender = pc.doc.feature{id = block}.body
            assert(#pc.doc.rebuild() == 0)
            local body = pc.doc.new_body{}
            local top = pc.design.borrow{body = body, from = lender,
              faces = {{point = {10, 10, 10}, normal = {0, 0, 1}}}}
            local post = pc.sketch.new{body = body, plane = "XY"}
            pc.sketch.circle{sketch = post, x = 10, y = 10, radius = 3}
            pc.design.pad{sketch = post, length = 5}
            pc.design.map_sketch{sketch = post, borrow = top}
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = body}.min[3] - 10) < 1e-6, "on the lent top")
            -- The lender grows: the post rises with its top.
            pc.design.set{feature = block, length = 15}
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = body}.min[3] - 15) < 1e-6, "follows the top")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "design.move_to_body",
            "Move a feature into another body's history, with the sketch and datums only it uses",
        )
        .param("feature", ParamKind::Id, "The feature")
        .param("body", ParamKind::Id, "The body it goes to, in at its tip")
        .returns("the ids of the features moved, the given one last")
        .note(
            "It is refused when the feature reads a sketch other features read too; \
             `design.duplicate` with `body` copies it there with a sketch of its own.",
        )
        .see_also("design.duplicate")
        .see_also("doc.new_body")
        .example(
            "A boss moved into a body of its own",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            local top = pc.sketch.new{body = body, plane = "XY", offset = 5}
            pc.sketch.rect{sketch = top, x = 0, y = 0, width = 5, height = 5}
            local boss = pc.design.pad{sketch = top, length = 5}
            local other = pc.doc.new_body{}
            local moved = pc.design.move_to_body{feature = boss, body = other}
            -- The boss went with the sketch only it used.
            assert(#moved == 2 and moved[1] == top and moved[2] == boss)
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = body}.volume - 1000) < 1e-3)
            assert(math.abs(pc.doc.measure{body = other}.volume - 125) < 1e-3)
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "design.duplicate",
            "Make a copy of a feature, with its own copies of the sketches and datums it reads",
        )
        .param("feature", ParamKind::Id, "The feature")
        .optional(
            "body",
            ParamKind::Id,
            "The body the copy goes in, at its tip (the feature's own when left out)",
        )
        .returns("the ids of the features made, the copy of the given one last")
        .note(
            "The copy is made exactly where the feature is, so in the same body it adds \
             nothing to the solid until `design.set` changes it.",
        )
        .see_also("design.set")
        .see_also("design.move_to_body")
        .example(
            "A pad copied and turned to grow the other way",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            local made = pc.design.duplicate{feature = pad}
            -- A sketch of its own and the copied pad, last.
            assert(#made == 2 and made[1] ~= s)
            local copy = made[2]
            pc.design.set{feature = copy, reversed = true}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = body}
            assert(math.abs(m.volume - 1000) < 1e-3 and math.abs(m.min[3] + 5) < 1e-6, m.volume)
            "#,
        ),
    );
}

pub(super) fn run(
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    if id == "design.freeze" {
        return freeze(&a, ctx);
    }
    if id == "design.move_to_body" {
        return move_to_body(&a, ctx);
    }
    if id == "design.map_sketch" {
        let sketch = FeatureId(a.id("sketch")?);
        let borrow = FeatureId(a.id("borrow")?);
        let index = match a.opt_number("index")? {
            Some(i) if i < 0.0 => return Err(CommandError::bad("index", "must be 0 or more")),
            Some(i) => i as usize,
            None => crate::borrow::flat_faces_of_body(
                ctx.document,
                ctx.document
                    .get_feature_meta(borrow)
                    .and_then(|n| n.body)
                    .ok_or_else(|| CommandError::bad("borrow", "is not a borrow"))?,
            )
            .into_iter()
            .find(|(r, _)| r.borrow == borrow)
            .map(|(r, _)| r.index)
            .ok_or_else(|| CommandError::bad("borrow", "lends no flat face"))?,
        };
        crate::borrow::map_sketch(
            ctx.document,
            sketch,
            crate::feature::BorrowedRef { borrow, index },
        )
        .map_err(CommandError::failed)?;
        return Ok(Value::Null);
    }
    if id == "design.duplicate" {
        let feature = FeatureId(a.id("feature")?);
        let body = a.opt_id("body")?.map(BodyId);
        return duplicate(ctx, feature, body)
            .map(|made| Value::from(made.iter().map(|f| f.0.to_string()).collect::<Vec<_>>()))
            .map_err(CommandError::failed);
    }
    Err(CommandError::Unknown(id.to_string()))
}
