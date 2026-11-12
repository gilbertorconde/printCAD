//! Where a body sits: reading and setting its placement, moving it, and
//! turning or flipping it about a joint.

use core_document::{
    Args, BodyId, BodyPlacement, CommandArgs, CommandError, CommandResult, CommandSpec,
    CommandTable, FeatureId, ParamKind, WorkbenchRuntimeContext,
};
use glam::{Quat, Vec3};
use serde_json::{Value, json};

use crate::joint::{JointKind, Rigid};
use crate::solve::joints;

use super::{BOX, FACING, single, solved};

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable) {
    t.add(
        CommandSpec::new(
            "asm.turn",
            "Turn a joint's body about the joint's axis or normal, the joint keeping it there",
        )
        .param("joint", ParamKind::Id, "The joint")
        .param("degrees", ParamKind::Number, "How far, degrees"),
        turn,
    );
    t.add(
        CommandSpec::new(
            "asm.flip",
            "Turn a joint's body over, half a turn across the joint's axis or normal",
        )
        .param("joint", ParamKind::Id, "The joint"),
        flip,
    );
    t.add(
        CommandSpec::new(
            "asm.turn90",
            "Turn a joint's own turn a quarter turn, its body following",
        )
        .param(
            "joint",
            ParamKind::Id,
            "The hinge, alignment, slider or fixed joint",
        )
        .optional(
            "back",
            ParamKind::Bool,
            "Turn the other way round (false when left out)",
        ),
        turn90,
    );
    t.add(
        CommandSpec::new("asm.placement", "Where a body sits")
            .param("body", ParamKind::Id, "")
            .returns("{translation, rotation}, rotation a quaternion {x, y, z, w}")
            .read_only(),
        placement,
    );
    t.add(
        CommandSpec::new("asm.place", "Put a body at a placement")
            .param("body", ParamKind::Id, "")
            .optional("translation", ParamKind::List, "{x, y, z} in mm")
            .optional("rotation", ParamKind::List, "A quaternion {x, y, z, w}")
            .optional(
                "turns",
                ParamKind::List,
                "Couplings' whole turns to store with it, {{coupling = id, turns = n}, ...}",
            ),
        place,
    );
    t.add(
        CommandSpec::new("asm.move", "Move a body by a step and a turn")
            .param("body", ParamKind::Id, "")
            .optional("by", ParamKind::List, "{x, y, z} in mm")
            .optional("turn", ParamKind::Number, "Degrees about `axis`")
            .optional("axis", ParamKind::List, "{x, y, z}; Z when left out")
            .optional(
                "about",
                ParamKind::List,
                "The point the turn is about, {x, y, z}; the origin when left out",
            ),
        move_body,
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands, the
/// examples' setup as `{{box}}` and `{{facing}}`.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(
        &PROSE,
        "placing.md",
        include_str!("placing.md"),
        &[("box", BOX), ("facing", FACING)],
    )
}

/// The couplings' counts `asm.place` was given in `turns`.
fn turn_counts(a: &Args) -> Result<Vec<(FeatureId, i64)>, CommandError> {
    let Some(list) = a.0.get("turns").filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let bad = || CommandError::bad("turns", "is a list of {coupling = id, turns = n}");
    list.as_array()
        .ok_or_else(bad)?
        .iter()
        .map(|item| {
            let id = item
                .get("coupling")
                .and_then(Value::as_str)
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .ok_or_else(bad)?;
            let turns = item
                .get("turns")
                .and_then(Value::as_f64)
                .filter(|n| n.fract() == 0.0)
                .ok_or_else(bad)?;
            Ok((FeatureId(id), turns as i64))
        })
        .collect()
}

fn turn(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    turn_or_flip(args, ctx, false)
}

fn flip(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    turn_or_flip(args, ctx, true)
}

/// Turn a joint's body by `degrees`, or `flip` it over.
fn turn_or_flip(
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
    flip: bool,
) -> CommandResult {
    let a = Args(args);
    let joint = FeatureId(a.id("joint")?);
    let found = joints(ctx.document)
        .into_iter()
        .find(|j| j.id == joint && j.feature.kind != JointKind::Ground)
        .ok_or_else(|| CommandError::bad("joint", "is not a joint between two bodies"))?;
    let degrees = if flip { 0.0 } else { a.number("degrees")? };
    if !flip && matches!(found.feature.kind, JointKind::Hinge { .. }) {
        let release = advance(ctx, &found, degrees, 0)?;
        let answer = solved(ctx, Value::Null);
        // A drive held only for the solve goes; nothing moves with
        // it, the bodies being where it held them.
        if let Some((joint, data)) = release {
            ctx.document
                .update_feature_data(joint, data)
                .map_err(|e| CommandError::failed(e.to_string()))?;
            ctx.document.clear_feature_dirty(joint);
        }
        return answer;
    }
    turn_joint(ctx, &found, degrees, flip)?;
    solved(ctx, Value::Null)
}

fn placement(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let placement = ctx.document.body_placement(a.body(ctx.document, "body")?);
    Ok(json!({
        "translation": placement.translation,
        "rotation": placement.rotation,
    }))
}

fn place(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let body = a.body(ctx.document, "body")?;
    let now = ctx.document.body_placement(body);
    let translation = a
        .opt_vec3("translation")?
        .map_or(Vec3::from(now.translation), single);
    let rotation = match a.0.get("rotation") {
        Some(v) if !v.is_null() => quaternion(v)?,
        _ => now.quat(),
    };
    crate::components::move_with_unit(
        ctx.document,
        body,
        BodyPlacement::new(rotation, translation),
    );
    crate::coupling::set_turn_counts(ctx.document, &turn_counts(&a)?);
    Ok(Value::Null)
}

fn turn90(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let joint = FeatureId(a.id("joint")?);
    let found = joints(ctx.document)
        .into_iter()
        .find(|j| j.id == joint)
        .ok_or_else(|| CommandError::bad("joint", "is not a joint"))?;
    if !found.feature.kind.keeps_a_turn() {
        return Err(CommandError::bad(
            "joint",
            "keeps no turn of its own; asm.turn turns its body",
        ));
    }
    if found.feature.fixed.parts().1 == glam::DVec3::ZERO {
        return Err(CommandError::bad(
            "joint",
            "holds to a point, with no axis or normal to turn about",
        ));
    }
    let degrees = if a.opt_bool("back")? == Some(true) {
        -90.0
    } else {
        90.0
    };
    turn_joint(ctx, &found, degrees, false)?;
    solved(ctx, Value::Null)
}

fn move_body(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let body = a.body(ctx.document, "body")?;
    let by = a.opt_vec3("by")?.map_or(Vec3::ZERO, single);
    let axis = a.opt_vec3("axis")?.map_or(Vec3::Z, single);
    if axis.length() < 1e-9 {
        return Err(CommandError::bad("axis", "must not be zero"));
    }
    let about = a.opt_vec3("about")?.map_or(Vec3::ZERO, single);
    let turn = Quat::from_axis_angle(
        axis.normalize(),
        (a.opt_number("turn")?.unwrap_or(0.0) as f32).to_radians(),
    );
    // Turn about `about`, then step: p -> turn (p - about) + about + by.
    let step = BodyPlacement::new(turn, about - turn * about + by);
    let now = ctx.document.body_placement(body);
    crate::components::move_with_unit(ctx.document, body, step.after(&now));
    Ok(Value::Null)
}

/// Turn a joint's moving body by `degrees` about the joint's direction,
/// or half a turn over with `over`, and carry the joint's settings with
/// it so it holds the body there.
pub(crate) fn turn_joint(
    ctx: &mut WorkbenchRuntimeContext,
    joint: &crate::Joint,
    degrees: f64,
    over: bool,
) -> Result<(), CommandError> {
    let at = |b: BodyId| -> Rigid { ctx.document.body_placement(b).into() };
    let (before, fixed) = (at(joint.body), at(joint.feature.other_body));
    let step = joint.feature.turning_step(&before, &fixed, degrees, over);
    let after = before.then(&step);
    let mut feature = joint.feature.clone();
    feature.carried(&before, &after, &fixed);
    ctx.document
        .update_feature_data(joint.id, core_document::WorkbenchFeature::to_json(&feature))
        .map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document.clear_feature_dirty(joint.id);
    ctx.document.set_body_placement(joint.body, after.into());
    Ok(())
}

/// Move a hinge's or a slider's motion on by `amount` (degrees or mm),
/// its drive holding it there for the solve that follows: a held drive's
/// value moves, else a drive is set where the motion now reaches, and
/// answered, with the joint's data before it, to be taken off after the
/// solve. A joint a coupling drives moves its driver instead, by what
/// brings it `amount` along.
fn advance(
    ctx: &mut WorkbenchRuntimeContext,
    joint: &crate::Joint,
    amount: f64,
    depth: usize,
) -> Result<Option<(FeatureId, Value)>, CommandError> {
    if depth < 8
        && let Some(coupling) = crate::coupling::driving(ctx.document, joint.id)
        && let Some(driver) = joints(ctx.document)
            .into_iter()
            .find(|j| j.id == coupling.driver)
    {
        let step = coupling
            .driver_step(amount, crate::coupling::angular(&driver.feature))
            .ok_or_else(|| CommandError::failed("the coupling moves nothing"))?;
        return advance(ctx, &driver, step, depth + 1);
    }
    let at = |b: BodyId| -> Rigid { ctx.document.body_placement(b).into() };
    let now = joint
        .feature
        .travel(&at(joint.body), &at(joint.feature.other_body))
        .ok_or_else(|| CommandError::bad("joint", "is not a hinge or a slider"))?;
    let before = core_document::WorkbenchFeature::to_json(&joint.feature);
    let mut feature = joint.feature.clone();
    let released = match &mut feature.kind {
        JointKind::Hinge { drive, .. } | JointKind::Slider { drive, .. } => match &mut drive.to {
            Some(to) => {
                *to += amount as f32;
                None
            }
            None => {
                drive.to = Some((now + amount) as f32);
                Some((joint.id, before))
            }
        },
        _ => return Err(CommandError::bad("joint", "is not a hinge or a slider")),
    };
    ctx.document
        .update_feature_data(joint.id, core_document::WorkbenchFeature::to_json(&feature))
        .map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document.clear_feature_dirty(joint.id);
    Ok(released)
}

fn quaternion(value: &Value) -> Result<Quat, CommandError> {
    let bad = || CommandError::bad("rotation", "must be a quaternion {x, y, z, w}");
    let q = match value {
        Value::Array(v) if v.len() == 4 => {
            [v[0].as_f64(), v[1].as_f64(), v[2].as_f64(), v[3].as_f64()]
        }
        Value::Object(m) => ["x", "y", "z", "w"].map(|k| m.get(k).and_then(Value::as_f64)),
        _ => return Err(bad()),
    };
    let q = Quat::from_xyzw(
        q[0].ok_or_else(bad)? as f32,
        q[1].ok_or_else(bad)? as f32,
        q[2].ok_or_else(bad)? as f32,
        q[3].ok_or_else(bad)? as f32,
    );
    if q.length() < 1e-6 {
        return Err(bad());
    }
    Ok(q.normalize())
}
