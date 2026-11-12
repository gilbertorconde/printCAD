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
        .param("degrees", ParamKind::Number, "How far, degrees")
        .note(
            "On a hinge the angle moves on by `degrees`: a drive holding it moves with it, \
             else the hinge is free again afterwards, where it was turned to. A hinge a \
             coupling drives turns its driver to get there.",
        )
        .note(
            "Other joints turn the body about their normal or axis through the joint's point, \
             the joint carried with it so it holds the body there. A ground is refused.",
        )
        .see_also("asm.flip")
        .see_also("asm.travel")
        .example(
            "A mated block turned a quarter turn on its face",
            &format!(
                "{BOX}{FACING}{}",
                r#"
            local base = box(0, 20, 20, 5)
            local lid = box(5, 10, 4, 3)
            assert(#pc.doc.rebuild() == 0)
            local j = pc.asm.mate{body = lid, face = facing(lid, -1),
              other = base, other_face = facing(base, 1)}
            pc.asm.turn{joint = j, degrees = 90}
            local q = pc.asm.placement{body = lid}.rotation
            assert(math.abs(math.deg(2 * math.acos(q[4])) - 90) < 1e-3, "a quarter turn")
            assert(math.abs(q[3]) > 0.7, "about Z")
            assert(math.abs(facing(lid, -1).point[3] - 5) < 1e-4, "still on the top")
                "#
            ),
        ),
        turn,
    );
    t.add(
        CommandSpec::new(
            "asm.flip",
            "Turn a joint's body over, half a turn across the joint's axis or normal",
        )
        .param("joint", ParamKind::Id, "The joint")
        .note(
            "The joint is carried with the body, so it holds it turned over. On a mate that \
             is `flip` in `asm.set`: a body resting on a face ends on its far side, inside \
             the body it rested on.",
        )
        .see_also("asm.turn")
        .see_also("asm.set")
        .example(
            "A wheel turned over on its hinge",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
            local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
            local w = pc.sketch.new{plane = "XY"}
            pc.sketch.circle{sketch = w, x = 30, y = 0, radius = 10}
            local wheel = pc.doc.feature{id = pc.design.pad{sketch = w, length = 4}}.body
            assert(#pc.doc.rebuild() == 0)
            local function axis(x)
              return {axis = {point = {x, 0, 0}, direction = {0, 0, 1}}}
            end
            local h = pc.asm.hinge{body = wheel, face = axis(30),
              other = post, other_face = axis(0), offset = 3}
            pc.asm.flip{joint = h}
            local q = pc.asm.placement{body = wheel}.rotation
            assert(math.abs(q[4]) < 1e-4, "half a turn")
            assert(math.abs(q[3]) < 1e-4, "about an axis square to the hinge's")
            "#,
        ),
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
        )
        .note(
            "The turn the joint keeps is turned 90° about the other end's axis or normal: a \
             hinge's or an alignment's zero, a slider's or a fixed joint's held turn. The \
             body turns with it, and what the joint reads stays as it was: a hinge held at \
             30° is held at 30° from its new zero, its limits turned round with it.",
        )
        .note(
            "A joint that keeps no turn of its own is refused. A mate's turn in its plane is \
             free, so `asm.turn` turns a mated body; it is the same for the other joints.",
        )
        .see_also("asm.turn")
        .see_also("asm.travel")
        .example(
            "A hinge held at 30° whose zero is turned a quarter turn",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
            local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
            local w = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = w, x = 0, y = -2, width = 20, height = 4}
            local arm = pc.doc.feature{id = pc.design.pad{sketch = w, length = 2}}.body
            assert(#pc.doc.rebuild() == 0)
            local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
            local h = pc.asm.hinge{body = arm, face = z, other = post, other_face = z,
              drive = 30}
            pc.asm.turn90{joint = h}
            assert(math.abs(pc.asm.travel{joint = h} - 30) < 1e-3, "still 30° on the hinge")
            local q = pc.asm.placement{body = arm}.rotation
            assert(math.abs(math.deg(2 * math.acos(q[4])) - 120) < 1e-2,
              "the arm a quarter turn further round")
            "#,
        ),
        turn90,
    );
    t.add(
        CommandSpec::new("asm.placement", "Where a body sits")
            .param("body", ParamKind::Id, "")
            .returns("{translation, rotation}, rotation a quaternion {x, y, z, w}")
            .read_only()
            .note(
                "`translation` is in mm, in world space: where the body's own origin is, \
                 turned by `rotation`.",
            )
            .see_also("asm.place")
            .see_also("asm.move")
            .example(
                "A body's placement before and after a move",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
                local body = pc.doc.feature{id = pc.design.pad{sketch = s, length = 3}}.body
                assert(#pc.doc.rebuild() == 0)
                local p = pc.asm.placement{body = body}
                assert(p.translation[1] == 0 and p.rotation[4] == 1, "where it was modelled")
                pc.asm.move{body = body, by = {5, 0, 0}, turn = 90}
                p = pc.asm.placement{body = body}
                assert(p.translation[1] == 5 and math.abs(p.rotation[3] - math.sqrt(0.5)) < 1e-6)
                "#,
            ),
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
            )
            .note(
                "It sets the placement outright; what is left out keeps its value. The \
                 quaternion is normalised, and a zero one is refused.",
            )
            .note(
                "It does not solve: joints catch up at the next `asm.solve`. The other \
                 bodies of a rigid component move with it.",
            )
            .note(
                "A drag that carries a geared hinge past a whole turn records the \
                 couplings' new counts in `turns`, so a replay counts as the drag did.",
            )
            .see_also("asm.move")
            .see_also("asm.placement")
            .see_also("asm.solve")
            .example(
                "A body lifted and turned",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
                local body = pc.doc.feature{id = pc.design.pad{sketch = s, length = 3}}.body
                assert(#pc.doc.rebuild() == 0)
                pc.asm.place{body = body, translation = {0, 0, 20}, rotation = {0, 0, 1, 1}}
                local p = pc.asm.placement{body = body}
                assert(p.translation[3] == 20)
                local half = math.sqrt(0.5)
                assert(math.abs(p.rotation[3] - half) < 1e-6, "the quaternion is normalised")
                pc.asm.place{body = body, translation = {1, 2, 3}}
                p = pc.asm.placement{body = body}
                assert(math.abs(p.rotation[3] - half) < 1e-6, "the rotation left out is kept")
                "#,
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
            )
            .note(
                "It turns first, `turn` degrees about `axis` through `about`, then steps by \
                 `by`, mm in world space; both add to where the body is.",
            )
            .note(
                "It does not solve: joints catch up at the next `asm.solve`. The other \
                 bodies of a rigid component move with it.",
            )
            .see_also("asm.place")
            .see_also("asm.solve")
            .example(
                "A plate turned about its own centre",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 20}
                local body = pc.doc.feature{id = pc.design.pad{sketch = s, length = 3}}.body
                assert(#pc.doc.rebuild() == 0)
                pc.asm.move{body = body, turn = 90, about = {10, 10, 0}}
                local t = pc.asm.placement{body = body}.translation
                assert(math.abs(t[1] - 20) < 1e-4 and math.abs(t[2]) < 1e-4,
                  "turned in place about its centre")
                pc.asm.move{body = body, by = {0, 0, 5}}
                t = pc.asm.placement{body = body}.translation
                assert(math.abs(t[3] - 5) < 1e-4, "steps add up")
                "#,
            ),
        move_body,
    );
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
