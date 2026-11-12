//! Linked copies of a body, mirrored copies, and replacing a body's shape.

use core_document::{
    Args, BodyId, BodyPlacement, CommandArgs, CommandError, CommandResult, CommandSpec,
    CommandTable, ParamKind, WorkbenchRuntimeContext,
};
use glam::{Quat, Vec3};
use serde_json::{Value, json};

use super::{BOX, FACING, copy_step, single, solved, vector};

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable) {
    t.add(
        CommandSpec::new(
            "asm.copy",
            "Insert linked copies of a body: each takes its shape and follows it, placed on its own",
        )
        .param("body", ParamKind::Id, "The body to copy")
        .optional("count", ParamKind::Number, "How many (1 when left out)")
        .optional(
            "step",
            ParamKind::List,
            "{x, y, z}: how far each copy sits from the one before, mm; beside it along X \
             when left out",
        )
        .optional(
            "around",
            ParamKind::Any,
            "{point = {x, y, z}, direction = {x, y, z}, angle}: the copies turned about this \
             axis instead, spread evenly over `angle` degrees (360 when left out)",
        )
        .returns("the copies' ids"), copy);
    t.add(
        CommandSpec::new(
            "asm.mirror",
            "Insert a linked copy that is a body's mirror image, following every change to it",
        )
        .param("body", ParamKind::Id, "The body to mirror")
        .param(
            "point",
            ParamKind::List,
            "A point of the mirror plane, {x, y, z}, in the world",
        )
        .param("normal", ParamKind::List, "The plane's normal, {x, y, z}")
        .returns("the mirrored copy's id"),
        mirror,
    );
    t.add(
        CommandSpec::new(
            "asm.replace",
            "Put another body in a body's place, with its joints found again on the new body's faces",
        )
        .param("body", ParamKind::Id, "The body to replace; it is hidden")
        .param("with", ParamKind::Id, "The body that takes its place")
        .returns(
            "{kept, unmatched}: the joints whose ends were found on the new body, and those \
             that were not",
        ), replace);
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands, the
/// examples' setup as `{{box}}` and `{{facing}}`.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(
        &PROSE,
        "copies.md",
        include_str!("copies.md"),
        &[("box", BOX), ("facing", FACING)],
    )
}

fn copy(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let source = a.body(ctx.document, "body")?;
    let count = a.opt_number("count")?.unwrap_or(1.0).clamp(1.0, 500.0) as usize;
    let step = a.opt_vec3("step")?.map(single);
    let made = match a.0.get("around").filter(|v| !v.is_null()) {
        Some(around) => {
            let point = vector(around.get("point").unwrap_or(&json!([0, 0, 0])), "around")?;
            let direction = vector(around.get("direction").unwrap_or(&Value::Null), "around")?;
            if direction.length() < 1e-9 {
                return Err(CommandError::bad("around", "needs a direction"));
            }
            let angle = around.get("angle").and_then(Value::as_f64).unwrap_or(360.0) as f32;
            insert_copies_around(ctx, source, count, (point, direction.normalize(), angle))
        }
        None => insert_copies(ctx, source, count, step),
    }
    .ok_or_else(|| CommandError::bad("body", core_document::command::NOT_A_BODY))?;
    Ok(Value::from(
        made.iter().map(|b| b.0.to_string()).collect::<Vec<_>>(),
    ))
}

fn replace(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let old = a.body(ctx.document, "body")?;
    let new = BodyId(a.id("with")?);
    let report = crate::replace::replace(ctx.document, old, new).map_err(CommandError::failed)?;
    solved(
        ctx,
        json!({"kept": report.kept, "unmatched": report.unmatched}),
    )
}

fn mirror(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let source = a.body(ctx.document, "body")?;
    let point = single(a.vec3("point")?);
    let normal = single(a.vec3("normal")?);
    if normal.length() < 1e-9 {
        return Err(CommandError::bad("normal", "must not be zero"));
    }
    let plane = core_document::MirrorPlane {
        point: point.to_array(),
        normal: normal.normalize().to_array(),
    };
    let copy = ctx
        .document
        .create_mirrored_copy(source, plane, None)
        .ok_or_else(|| CommandError::bad("body", "cannot be mirrored: a mirror of a mirror"))?;
    Ok(json!(copy.0.to_string()))
}

/// `count` linked copies of `source`, each `step` (world) on from the
/// one before, the first `step` from the source; their ids.
pub(crate) fn insert_copies(
    ctx: &mut WorkbenchRuntimeContext,
    source: BodyId,
    count: usize,
    step: Option<Vec3>,
) -> Option<Vec<BodyId>> {
    let step = step.unwrap_or_else(|| copy_step(ctx.document, source));
    let from = ctx.document.body_placement(source);
    let mut made = Vec::with_capacity(count);
    for i in 1..=count {
        let copy = ctx.document.create_linked_copy(source, None)?;
        let placed = BodyPlacement::new(from.quat(), from.offset() + step * i as f32);
        ctx.document.set_body_placement(copy, placed);
        made.push(copy);
    }
    Some(made)
}

/// `count` linked copies of `source` turned about the axis through
/// `point` along `direction`, spread evenly over `angle` degrees: a whole
/// turn shares it with the source, a part turn ends on its far end.
pub(crate) fn insert_copies_around(
    ctx: &mut WorkbenchRuntimeContext,
    source: BodyId,
    count: usize,
    (point, direction, angle): (Vec3, Vec3, f32),
) -> Option<Vec<BodyId>> {
    let full = (angle.abs() - 360.0).abs() < 1e-3;
    let each = if full {
        angle / (count + 1) as f32
    } else {
        angle / count.max(1) as f32
    };
    let from = ctx.document.body_placement(source);
    let mut made = Vec::with_capacity(count);
    for i in 1..=count {
        let turn = Quat::from_axis_angle(direction, (each * i as f32).to_radians());
        let step = BodyPlacement::new(turn, point - turn * point);
        let copy = ctx.document.create_linked_copy(source, None)?;
        ctx.document.set_body_placement(copy, step.after(&from));
        made.push(copy);
    }
    Some(made)
}
