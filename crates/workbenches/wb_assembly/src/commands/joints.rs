//! Joints and couplings: making one, changing its settings, grounding, solving,
//! and what holds, what does not and the freedom left.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandProse, CommandResult, CommandSpec,
    CommandTable, FeatureId, ParamKind, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};
use std::sync::OnceLock;

use crate::coupling::{COUPLING_KIND, Coupling, Gearing};
use crate::joint::{Anchor, Drive, JOINT_KIND, JointFeature, JointKind, JointTool, Takes};
use crate::solve::joints;

use super::{
    BOX, FACING, ground_first, joints_holding, next_name, placements, rejoined, set_grounded,
    solved, vector,
};

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable) {
    let joint = |id: &str, summary: &str, face: &str| {
        CommandSpec::new(id, summary)
            .param("body", ParamKind::Id, "The body that moves")
            .param("face", ParamKind::Any, face)
            .param(
                "other",
                ParamKind::Id,
                "The body it is held against; the nil id (all zeros) for the world origin, \
                 its faces then in world space",
            )
            .param("other_face", ParamKind::Any, face)
            .optional("name", ParamKind::String, "Its name in the tree")
    };
    for tool in JointTool::ALL {
        let face = match tool.takes() {
            Takes::Flat => "A flat face, {point, normal}, as pc.doc.faces lists it",
            Takes::Round => {
                "A round face, {axis = {point, direction}}, as pc.doc.faces lists it; \
                 an edge's line or circle axis goes the same way"
            }
            Takes::Any => "Any face, as pc.doc.faces lists it; the body's origin when left out",
            Takes::FlatAndRound => {
                "A flat face {point, normal} on one body and a round face {axis, radius} \
                 on the other, either way round"
            }
            Takes::Point => {
                "A point: a ball's {centre}, or {point} alone, as pc.doc.faces lists them"
            }
            Takes::Directed => {
                "A flat face {point, normal} or a round face or edge {axis = {point, direction}}"
            }
            Takes::Anything => {
                "A flat face {point, normal}, a round face or edge {axis}, or a point \
                 ({centre} of a ball, or {point} alone)"
            }
            Takes::PointAndLine => {
                "The pin: a point ({centre} or {point}) on the moving body; the slot: a line \
                 {axis = {point, direction}} on the other"
            }
            Takes::PointAndEdge => {
                "The point that runs ({centre} or {point}); on the other body, a {point} on \
                 the edge it runs along"
            }
            Takes::PointAndFace => {
                "The follower ({centre} or {point}); on the other body, a {point} on the cam's face"
            }
        };
        let spec = if tool == JointTool::Fixed {
            CommandSpec::new(tool.command(), tool.summary())
                .param("body", ParamKind::Id, "The body that moves")
                .optional("face", ParamKind::Any, face)
                .param("other", ParamKind::Id, "The body it is held against")
                .optional("other_face", ParamKind::Any, face)
                .optional("name", ParamKind::String, "Its name in the tree")
        } else {
            joint(tool.command(), tool.summary(), face)
        };
        let spec = match tool {
            JointTool::Mate => spec
                .optional("offset", ParamKind::Number, "The gap between them, mm")
                .optional(
                    "flip",
                    ParamKind::Bool,
                    "Face the same way instead of at each other",
                ),
            JointTool::Angle => spec.optional(
                "degrees",
                ParamKind::Number,
                "Between their outward normals; the angle they make now when left out",
            ),
            JointTool::Hinge => spec
                .optional(
                    "offset",
                    ParamKind::Number,
                    "How far along the axis the first sits from the second, mm",
                )
                .optional("drive", ParamKind::Any, DRIVE)
                .optional("limits", ParamKind::Any, LIMITS),
            JointTool::Slider => spec.optional("drive", ParamKind::Any, DRIVE).optional(
                "limits",
                ParamKind::Any,
                LIMITS,
            ),
            JointTool::Align => align_drives(spec),
            JointTool::Distance => spec.optional(
                "offset",
                ParamKind::Number,
                "Along the second face's normal, mm; the distance they are now when left out",
            ),
            JointTool::Tangent => spec.optional(
                "radius",
                ParamKind::Number,
                "The round face's radius, mm; the face's own when left out",
            ),
            JointTool::Cam => spec.optional(
                "radius",
                ParamKind::Number,
                "The follower's roller radius, mm; 0 for a point follower",
            ),
            JointTool::Width => spec
                .param("face2", ParamKind::Any, "The tab's other flat face")
                .param("other_face2", ParamKind::Any, "The slot's other wall"),
            _ => spec,
        };
        t.add_by_id(
            spec.returns("the joint's id")
                .note(
                    "`body` moves and `other` stays. The document's first joint also grounds \
                     `other` (as `asm.ground` does), unless it is the world: the nil id, all \
                     zeros, whose faces are given in world space.",
                )
                .note(
                    "Faces are given where the bodies sit now, in world space, as \
                     `pc.doc.faces` lists them; the joint keeps them in each body's own frame \
                     and solves as it is made. A joint that cannot hold with the others is not \
                     made: the call fails naming the joints in conflict.",
                )
                .see_also("asm.set"),
            |id, args, ctx| make_joint(id, &Args(args), ctx),
        );
    }
    t.add(
        CommandSpec::new(
            "asm.couple",
            "Tie two joints' motions together: gears or a belt between two hinges, a \
             rack and pinion or a screw between a hinge and a slider",
        )
        .param("driver", ParamKind::Id, "The hinge or slider that leads")
        .param("driven", ParamKind::Id, "The hinge or slider that follows")
        .optional("gearing", ParamKind::String, GEARING)
        .optional("ratio", ParamKind::Number, RATIO)
        .optional(
            "reverse",
            ParamKind::Bool,
            "The driven joint moves the other way",
        )
        .optional("name", ParamKind::String, "Its name in the tree")
        .returns("the coupling's id"),
        couple,
    );
    let set = CommandSpec::new(
        "asm.set",
        "Change a joint's gap, side, angle or radius, or a coupling's joints and ratio",
    )
    .param("joint", ParamKind::Id, "A joint or a coupling")
    .optional("gearing", ParamKind::String, GEARING)
    .optional("ratio", ParamKind::Number, RATIO)
    .optional(
        "reverse",
        ParamKind::Bool,
        "A coupling's driven joint moves the other way",
    )
    .optional("driver", ParamKind::Id, "A coupling's leading joint")
    .optional("driven", ParamKind::Id, "A coupling's following joint")
    .optional(
        "offset",
        ParamKind::Number,
        "A mate's gap, a hinge's height or a distance, mm",
    )
    .optional("flip", ParamKind::Bool, "A mate's side")
    .optional("degrees", ParamKind::Number, "An angle joint's angle")
    .optional("radius", ParamKind::Number, "A tangent's radius, mm")
    .optional("drive", ParamKind::Any, DRIVE)
    .optional("limits", ParamKind::Any, LIMITS)
    .optional(
        "kind",
        ParamKind::String,
        "A joint's new kind (mate, align, hinge, ...), from where the bodies stand",
    )
    .optional(
        "face",
        ParamKind::Any,
        "The moving body's face, picked afresh, as the joint's command takes it",
    )
    .optional(
        "other",
        ParamKind::Id,
        "The body it is held against, picked afresh",
    )
    .optional(
        "other_face",
        ParamKind::Any,
        "The other body's face, picked afresh",
    )
    .optional(
        "moving_end",
        ParamKind::Number,
        "How far the moving end sits along its own normal or axis, mm",
    )
    .optional(
        "fixed_end",
        ParamKind::Number,
        "How far the fixed end sits along its own normal or axis, mm",
    );
    t.add(align_drives(set), set_joint);
    t.add(
        CommandSpec::new(
            "asm.redundant",
            "The joints that hold nothing a body's other joints do not",
        )
        .returns("a list of {joint, name}")
        .read_only(),
        redundant,
    );
    t.add(
        CommandSpec::new("asm.select_joints", "Select every joint that holds a body")
            .param("body", ParamKind::Id, "The body")
            .returns("a list of {joint, name}, in history order")
            .read_only(),
        select_joints,
    );
    t.add(
        CommandSpec::new(
            "asm.unmet",
            "The joints and couplings that do not hold where the bodies sit",
        )
        .returns("a list of {joint, name}, in history order")
        .read_only(),
        unmet,
    );
    t.add(
        CommandSpec::new(
            "asm.travel",
            "Where a hinge or a slider has got to: the hinge's angle in degrees, \
             the slider's position in mm",
        )
        .param("joint", ParamKind::Id, "")
        .returns("a number")
        .read_only(),
        travel,
    );
    t.add(
        CommandSpec::new(
            "asm.ground",
            "Keep a body where it is: the bodies joined to it are placed against it",
        )
        .param("body", ParamKind::Id, "")
        .optional(
            "grounded",
            ParamKind::Bool,
            "false lets it move again (true by default)",
        )
        .returns("the ground joint's id, or nil when it was taken away"),
        ground,
    );
    t.add(
        CommandSpec::new(
            "asm.freedom",
            "What each jointed body may still do: the motions its joints leave open",
        )
        .optional("body", ParamKind::Id, "Only this body")
        .returns(
            "a list of {body, free, motions}, each motion {turn = {axis, through}} \
             or {slide = direction}, with at_limit true where a limit lets it go one \
             way only",
        )
        .read_only(),
        freedom,
    );
    t.add(
        CommandSpec::new("asm.solve", "Place every body its joints hold")
            .returns("what moved, in words"),
        solve,
    );
    t.explain(prose());
}

/// `asm.set` of a joint, or of a coupling.
fn set_joint(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    if ctx
        .document
        .get_feature_meta(FeatureId(a.id("joint")?))
        .is_some_and(|n| n.workbench_id.as_str() == COUPLING_KIND)
    {
        return set_coupling(&a, ctx);
    }
    let joint = FeatureId(a.id("joint")?);
    let not_a_joint = || CommandError::bad("joint", "is not a joint");
    let node = ctx
        .document
        .get_feature_meta(joint)
        .filter(|n| n.workbench_id.as_str() == JOINT_KIND)
        .ok_or_else(not_a_joint)?;
    let mut feature: JointFeature =
        serde_json::from_value(node.data.clone()).map_err(|_| not_a_joint())?;
    // Other faces, another body or another kind first: the settings
    // below apply to the joint they make.
    let tool = match a.opt_string("kind")? {
        Some(word) => Some(JointTool::of_word(word).ok_or_else(|| {
            CommandError::bad("kind", "must be a joint's word: mate, align, hinge, ...")
        })?),
        None => None,
    };
    if tool.is_some()
        || ["face", "other", "other_face"]
            .iter()
            .any(|k| a.0.contains_key(*k))
    {
        let moving_body = node.body.ok_or_else(not_a_joint)?;
        let tool = tool
            .or_else(|| JointTool::of_kind(&feature.kind))
            .ok_or_else(|| CommandError::bad("joint", "a ground takes no faces"))?;
        let other = match a.opt_id("other")? {
            Some(id) => BodyId(id),
            None => feature.other_body,
        };
        if !(other == crate::WORLD || ctx.document.body(other).is_some()) || other == moving_body {
            return Err(CommandError::bad(
                "other",
                "must be another body of the document",
            ));
        }
        let local = |name: &str, body: BodyId, kept: Anchor| match a.0.get(name) {
            Some(v) => Ok::<_, CommandError>(
                anchor_of(Some(v), name, tool.takes())?
                    .moved(&ctx.document.body_placement(body).inverse()),
            ),
            None => Ok(kept),
        };
        let moving = local("face", moving_body, feature.moving)?;
        let fixed = local("other_face", other, feature.fixed)?;
        let radius = a
            .opt_number("radius")?
            .map(|r| r as f32)
            .or_else(|| {
                ["face", "other_face"]
                    .iter()
                    .find_map(|n| a.0.get(*n)?.get("radius")?.as_f64())
                    .map(|r| r as f32)
            })
            .or(match feature.kind {
                JointKind::Tangent { radius } => Some(radius),
                _ => None,
            })
            .unwrap_or(0.0);
        let kept = feature.names;
        let name_of = |key: &str, keep: u64| match a.0.get(key) {
            Some(face) => face.get("name").and_then(Value::as_u64).unwrap_or(0),
            None => keep,
        };
        feature = rejoined(ctx, tool, (moving_body, moving), (other, fixed), radius)?;
        feature.names = [name_of("face", kept[0]), name_of("other_face", kept[1])];
    }
    for (end, key) in ["moving_end", "fixed_end"].into_iter().enumerate() {
        if let Some(v) = a.opt_number(key)? {
            feature.ends[end] = v as f32;
        }
    }
    match &mut feature.kind {
        JointKind::Mate { flip, offset } => {
            if let Some(v) = a.opt_number("offset")? {
                *offset = v as f32;
            }
            if let Some(v) = a.opt_bool("flip")? {
                *flip = v;
            }
        }
        JointKind::Angle { degrees } => {
            if let Some(v) = a.opt_number("degrees")? {
                *degrees = v as f32;
            }
        }
        JointKind::Hinge { offset, drive, .. } => {
            if let Some(v) = a.opt_number("offset")? {
                *offset = v as f32;
            }
            drive_args(&a, drive)?;
        }
        JointKind::Slider { drive, .. } => drive_args(&a, drive)?,
        JointKind::Distance { offset } => {
            if let Some(v) = a.opt_number("offset")? {
                *offset = v as f32;
            }
        }
        JointKind::Tangent { radius } => {
            if let Some(v) = a.opt_number("radius")? {
                *radius = v as f32;
            }
        }
        JointKind::Align { turn, slide, .. } => {
            drive_args_named(&a, turn, "turn_drive", "turn_limits")?;
            drive_args_named(&a, slide, "slide_drive", "slide_limits")?;
        }
        JointKind::Ground
        | JointKind::Fixed { .. }
        | JointKind::Parallel
        | JointKind::Perpendicular
        | JointKind::Ball
        | JointKind::Universal
        | JointKind::Slot
        | JointKind::Path
        | JointKind::Width => {}
        JointKind::Cam { radius } => {
            if let Some(v) = a.opt_number("radius")? {
                *radius = v as f32;
            }
        }
    }
    let data = serde_json::to_value(&feature).map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document
        .update_feature_data(joint, data)
        .map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document.clear_feature_dirty(joint);
    solved(ctx, Value::Null)
}

fn travel(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let joint = FeatureId(a.id("joint")?);
    let found = joints(ctx.document).into_iter().find(|j| j.id == joint);
    let found = found.ok_or_else(|| CommandError::bad("joint", "is not a joint"))?;
    let at = |b: BodyId| -> crate::Rigid { ctx.document.body_placement(b).into() };
    let travel = found
        .feature
        .travel(&at(found.body), &at(found.feature.other_body))
        .ok_or_else(|| CommandError::bad("joint", "is not a hinge or a slider"))?;
    Ok(json!(travel))
}

fn ground(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let body = a.body(ctx.document, "body")?;
    let grounded = a.opt_bool("grounded")?.unwrap_or(true);
    let id = set_grounded(ctx, body, grounded);
    solved(ctx, id.map_or(Value::Null, |id| json!(id.0.to_string())))
}

fn freedom(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let only = a.opt_id("body")?.map(BodyId);
    Ok(Value::Array(
        crate::freedom(ctx.document)
            .into_iter()
            .filter(|(body, _)| only.is_none_or(|o| o == *body))
            .map(|(body, motions)| {
                json!({
                    "body": body.0.to_string(),
                    "free": motions.len(),
                    "motions": motions.iter().map(|m| match m {
                        crate::Motion::Turn { axis, through, at_limit } => json!({
                            "turn": {"axis": axis, "through": through},
                            "at_limit": at_limit,
                        }),
                        crate::Motion::Slide { direction, at_limit } => {
                            json!({"slide": direction, "at_limit": at_limit})
                        }
                    }).collect::<Vec<_>>(),
                })
            })
            .collect(),
    ))
}

fn solve(_args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    crate::apply_solve(ctx)
        .map(Value::String)
        .map_err(CommandError::failed)
}

fn unmet(_args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    Ok(Value::Array(
        crate::unmet(ctx.document)
            .into_iter()
            .map(|(id, name)| json!({"joint": id.0.to_string(), "name": name}))
            .collect(),
    ))
}

fn select_joints(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let body = a.body(ctx.document, "body")?;
    let held = joints_holding(ctx.document, body);
    ctx.request(core_document::HostRequest::SelectFeatures(
        held.iter().map(|(id, _)| *id).collect(),
    ));
    Ok(Value::Array(
        held.into_iter()
            .map(|(id, name)| json!({"joint": id.0.to_string(), "name": name}))
            .collect(),
    ))
}

fn redundant(_args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    Ok(Value::Array(
        crate::redundant(ctx.document)
            .into_iter()
            .map(|(id, name)| json!({"joint": id.0.to_string(), "name": name}))
            .collect(),
    ))
}

const DRIVE: &str = "A hinge's angle (degrees from where it was made) or a slider's \
    position (mm) to hold it at; false lets it move again";

const GEARING: &str = "gears (hinges turning opposite ways), belt (the same way), \
    rack (a hinge and a slider, by the pinion's pitch radius) or screw (by the lead); \
    the first that suits the two joints when left out";

const RATIO: &str = "Turns of the driven hinge per turn of the driver for gears and a \
    belt, the pitch radius in mm for a rack, the lead in mm a turn for a screw";

const LIMITS: &str = "{low, high}: the range a hinge's angle or a slider's position stays \
    in while not driven; false takes the limits away";

/// Read `drive` and `limits` into a hinge's or a slider's drive.
fn drive_args(a: &Args, drive: &mut Drive) -> Result<(), CommandError> {
    drive_args_named(a, drive, "drive", "limits")
}

/// Read the arguments named `to` and `range` into a drive.
fn drive_args_named(
    a: &Args,
    drive: &mut Drive,
    to_name: &str,
    range: &str,
) -> Result<(), CommandError> {
    match a.0.get(to_name) {
        None => {}
        Some(Value::Bool(false)) | Some(Value::Null) => drive.to = None,
        Some(v) => {
            let to = v
                .as_f64()
                .ok_or_else(|| CommandError::bad(to_name, "must be a number or false"))?;
            drive.to = Some(to as f32);
        }
    }
    match a.0.get(range) {
        None => {}
        Some(Value::Bool(false)) | Some(Value::Null) => drive.limits = None,
        Some(v) => {
            let bad = || CommandError::bad(range, "must be {low, high} or false");
            let pair = v.as_array().filter(|l| l.len() == 2).ok_or_else(bad)?;
            let low = pair[0].as_f64().ok_or_else(bad)? as f32;
            let high = pair[1].as_f64().ok_or_else(bad)? as f32;
            if low > high {
                return Err(CommandError::bad(range, "low must not be above high"));
            }
            drive.limits = Some([low, high]);
        }
    }
    Ok(())
}

/// An alignment's drives: its turn and its slide, each held or limited.
fn align_drives(spec: CommandSpec) -> CommandSpec {
    spec.optional(
        "turn_drive",
        ParamKind::Any,
        "An alignment's turn (degrees from where it was made) to hold it at; false lets it turn",
    )
    .optional(
        "turn_limits",
        ParamKind::Any,
        "{low, high}: the range an alignment's turn stays in; false takes it away",
    )
    .optional(
        "slide_drive",
        ParamKind::Any,
        "How far along the axis (mm) to hold an alignment; false lets it slide",
    )
    .optional(
        "slide_limits",
        ParamKind::Any,
        "{low, high}: the range an alignment's slide stays in, mm; false takes it away",
    )
}

/// The notes, related commands and examples of this file's commands, the
/// examples' setup as `{{box}}` and `{{facing}}`.
pub(super) fn prose() -> &'static CommandProse {
    static PROSE: OnceLock<CommandProse> = OnceLock::new();
    CommandProse::once(
        &PROSE,
        "joints.md",
        include_str!("joints.md"),
        &[("box", BOX), ("facing", FACING)],
    )
}

fn make_joint(id: &str, a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let moving_body = a.body(ctx.document, "body")?;
    let other = BodyId(a.id("other")?);
    if other != crate::WORLD && ctx.document.body(other).is_none() {
        return Err(CommandError::bad(
            "other",
            core_document::command::NOT_A_BODY,
        ));
    }
    if other == moving_body {
        return Err(CommandError::bad("other", "must be another body"));
    }
    let tool = JointTool::of_command(id).unwrap_or(JointTool::Mate);
    let at = |body: BodyId| -> crate::Rigid { ctx.document.body_placement(body).into() };
    let anchor = |name: &str, body: BodyId| {
        if tool == JointTool::Fixed && a.0.get(name).is_none() {
            // The body's own origin: a fixed joint holds the body, not a face.
            return Ok(Anchor::Plane {
                point: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
            });
        }
        let world = anchor_of(a.0.get(name), name, tool.takes())?;
        // Stored in the body's own frame, as a picked face is.
        Ok::<_, CommandError>(world.moved(&ctx.document.body_placement(body).inverse()))
    };
    let moving = anchor("face", moving_body)?;
    let fixed = anchor("other_face", other)?;
    if tool.takes() == Takes::FlatAndRound
        && matches!(moving, Anchor::Plane { .. }) == matches!(fixed, Anchor::Plane { .. })
    {
        return Err(CommandError::bad(
            "other_face",
            "must be round where `face` is flat, or flat where it is round",
        ));
    }
    let radius = match a.opt_number("radius")? {
        Some(r) => r as f32,
        None => ["face", "other_face"]
            .iter()
            .find_map(|name| a.0.get(*name)?.get("radius")?.as_f64())
            .unwrap_or(0.0) as f32,
    };
    if tool == JointTool::Tangent && radius <= 0.0 {
        return Err(CommandError::bad(
            "radius",
            "must be given where the round face has none",
        ));
    }
    let mut kind = tool.joint(&moving, &at(moving_body), &fixed, &at(other), radius);
    match &mut kind {
        JointKind::Mate { flip, offset } => {
            *flip = a.opt_bool("flip")?.unwrap_or(false);
            *offset = a.opt_number("offset")?.unwrap_or(0.0) as f32;
        }
        JointKind::Angle { degrees } => {
            if let Some(d) = a.opt_number("degrees")? {
                *degrees = d as f32;
            }
        }
        JointKind::Hinge { offset, drive, .. } => {
            if let Some(v) = a.opt_number("offset")? {
                *offset = v as f32;
            }
            drive_args(a, drive)?;
        }
        JointKind::Slider { drive, .. } => drive_args(a, drive)?,
        JointKind::Align { turn, slide, .. } => {
            drive_args_named(a, turn, "turn_drive", "turn_limits")?;
            drive_args_named(a, slide, "slide_drive", "slide_limits")?;
        }
        JointKind::Distance { offset } => {
            if let Some(v) = a.opt_number("offset")? {
                *offset = v as f32;
            }
        }
        _ => {}
    }
    let label = kind.label();
    let name = match a.opt_string("name")? {
        Some(name) => name.to_string(),
        None => next_name(ctx.document, label),
    };
    let name_of = |key: &str| {
        a.0.get(key)
            .and_then(|f| f.get("name"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let joint = JointFeature {
        second: None,
        shape: Vec::new(),
        ends: [0.0; 2],
        names: [name_of("face"), name_of("other_face")],
        kind,
        moving,
        other_body: other,
        fixed,
    };
    let mut joint = joint;
    if tool == JointTool::Width {
        let second = anchor("face2", moving_body)?;
        let other_second = anchor("other_face2", other)?;
        joint.second = Some([second, other_second]);
    }
    crate::shapes::take_shape(ctx.document, &mut joint);
    if matches!(joint.kind, JointKind::Path | JointKind::Cam { .. }) && joint.shape.is_empty() {
        return Err(CommandError::bad(
            "other_face",
            "finds no edge or face of the other body there",
        ));
    }
    let grounded = ground_first(ctx, other);
    let feature = ctx
        .document
        .add_feature_in_body(joint, name, Some(moving_body))
        .map_err(|e| CommandError::failed(e.to_string()))?;
    // A joint has no solid to rebuild.
    ctx.document.clear_feature_dirty(feature);
    let made: Vec<FeatureId> = std::iter::once(feature).chain(grounded).collect();
    solved_with(ctx, &made, json!(feature.0.to_string()))
}

/// Solve with `made` just added. A joint that cannot hold is not left
/// behind: when the assembly holds without what was made, it is taken
/// back out and the call fails saying why. A conflict that was there
/// before is no fault of the new joint, which stays; what the conflict
/// does not hold up is placed.
fn solved_with(
    ctx: &mut WorkbenchRuntimeContext,
    made: &[FeatureId],
    value: Value,
) -> CommandResult {
    match crate::solve(ctx.document) {
        Ok(moves) => {
            crate::place_bodies(ctx.document, &moves);
            Ok(value)
        }
        Err(crate::SolveError::Conflict {
            body,
            joints,
            moves,
        }) => {
            let mut without = ctx.document.clone();
            for id in made {
                let _ = without.remove_feature(*id);
            }
            let name = ctx
                .document
                .body(body)
                .map(|b| b.name.clone())
                .unwrap_or_default();
            if crate::solve(&without).is_ok() {
                for id in made {
                    let _ = ctx.document.remove_feature(*id);
                }
                return Err(CommandError::failed(format!(
                    "{name} cannot hold this joint with the others ({}): it is not made",
                    joints.join(", ")
                )));
            }
            crate::place_bodies(ctx.document, &moves);
            ctx.log_warn(format!(
                "{name} cannot hold all its joints at once: {}",
                joints.join(", ")
            ));
            Ok(value)
        }
    }
}

/// A coupling of two joints where they stand, from a command's
/// arguments; `keep` supplies what the arguments leave out.
fn build_coupling(
    a: &Args,
    ctx: &WorkbenchRuntimeContext,
    keep: Option<&Coupling>,
) -> Result<Coupling, CommandError> {
    let all = joints(ctx.document);
    let joint = |name: &str, kept: Option<FeatureId>| -> Result<crate::Joint, CommandError> {
        let id = match a.opt_id(name)? {
            Some(id) => FeatureId(id),
            None => kept.ok_or_else(|| CommandError::bad(name, "is needed"))?,
        };
        all.iter()
            .find(|j| j.id == id)
            .cloned()
            .filter(|j| {
                matches!(
                    j.feature.kind,
                    JointKind::Hinge { .. } | JointKind::Slider { .. }
                )
            })
            .ok_or_else(|| CommandError::bad(name, "must be a hinge or a slider"))
    };
    let driver = joint("driver", keep.map(|c| c.driver))?;
    let driven = joint("driven", keep.map(|c| c.driven))?;
    if driver.id == driven.id {
        return Err(CommandError::bad("driven", "must be another joint"));
    }
    let gearing = match a.opt_string("gearing")? {
        Some(word) => Gearing::of_word(word)
            .ok_or_else(|| CommandError::bad("gearing", "must be gears, belt, rack or screw"))?,
        None => keep
            .map(|c| c.gearing)
            .filter(|g| g.fits(&driver.feature.kind, &driven.feature.kind))
            .or_else(|| Gearing::suiting(&driver.feature.kind, &driven.feature.kind))
            .ok_or_else(|| CommandError::bad("driven", "cannot be tied to the driver"))?,
    };
    if !gearing.fits(&driver.feature.kind, &driven.feature.kind) {
        return Err(CommandError::bad(
            "gearing",
            "does not suit these joints: gears and a belt tie two hinges, a rack and a \
             screw a hinge and a slider",
        ));
    }
    let ratio = match a.opt_number("ratio")? {
        Some(r) => r as f32,
        // A kept ratio reads the same way only in a kind of the same sort.
        None => keep
            .filter(|c| c.gearing.ratio_label().1 == gearing.ratio_label().1)
            .map_or(gearing.default_ratio(), |c| c.ratio),
    };
    if !(ratio.is_finite() && ratio > 0.0) {
        return Err(CommandError::bad("ratio", "must be above zero"));
    }
    let reverse = match a.opt_bool("reverse")? {
        Some(r) => r,
        None => keep.is_some_and(|c| c.reverse),
    };
    let same_joints = keep.is_some_and(|c| c.driver == driver.id && c.driven == driven.id);
    match keep {
        // The same two joints keep where they were tied, so a new ratio
        // moves nothing at that place.
        Some(kept) if same_joints => Ok(Coupling {
            gearing,
            ratio,
            reverse,
            ..kept.clone()
        }),
        _ => Coupling::new(
            gearing,
            &driver,
            &driven,
            ratio,
            reverse,
            &placements(ctx.document),
        )
        .ok_or_else(|| CommandError::failed("the joints' bodies are not there")),
    }
}

fn couple(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = &Args(args);
    let coupling = build_coupling(a, ctx, None)?;
    let body = joints(ctx.document)
        .into_iter()
        .find(|j| j.id == coupling.driven)
        .map(|j| j.body)
        .ok_or_else(|| CommandError::bad("driven", "is not a joint"))?;
    let name = match a.opt_string("name")? {
        Some(name) => name.to_string(),
        None => next_name(ctx.document, coupling.gearing.label()),
    };
    let id = ctx
        .document
        .add_feature_in_body(coupling, name, Some(body))
        .map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document.clear_feature_dirty(id);
    solved_with(ctx, &[id], json!(id.0.to_string()))
}

fn set_coupling(a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let id = FeatureId(a.id("joint")?);
    let kept = ctx
        .document
        .get_feature_data(id)
        .and_then(|d| serde_json::from_value::<Coupling>(d.clone()).ok())
        .ok_or_else(|| CommandError::bad("joint", "is not a coupling"))?;
    let coupling = build_coupling(a, ctx, Some(&kept))?;
    let data = serde_json::to_value(&coupling).map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document
        .update_feature_data(id, data)
        .map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document.clear_feature_dirty(id);
    solved(ctx, Value::Null)
}

/// A face as a joint anchors to it: a flat face's `point` and `normal`, or
/// a round face's `axis`.
fn anchor_of(value: Option<&Value>, name: &str, takes: Takes) -> Result<Anchor, CommandError> {
    let face = value
        .and_then(Value::as_object)
        .ok_or_else(|| CommandError::bad(name, "must be a face, as pc.doc.faces lists it"))?;
    let flat = || -> Result<Anchor, CommandError> {
        let point = vector(face.get("point").unwrap_or(&Value::Null), name)?;
        let normal = face
            .get("normal")
            .ok_or_else(|| CommandError::bad(name, "must be a flat face, with a normal"))?;
        let normal = vector(normal, name)?;
        Ok(Anchor::Plane {
            point: point.to_array(),
            normal: normal.normalize_or_zero().to_array(),
        })
    };
    let round = || -> Result<Anchor, CommandError> {
        let axis = face
            .get("axis")
            .and_then(Value::as_object)
            .ok_or_else(|| CommandError::bad(name, "must be a round face, with an axis"))?;
        let point = vector(axis.get("point").unwrap_or(&Value::Null), name)?;
        let direction = vector(axis.get("direction").unwrap_or(&Value::Null), name)?;
        Ok(Anchor::Axis {
            point: point.to_array(),
            direction: direction.normalize_or_zero().to_array(),
        })
    };
    let point = || -> Result<Anchor, CommandError> {
        let at = face
            .get("centre")
            .or_else(|| face.get("point"))
            .unwrap_or(&Value::Null);
        Ok(Anchor::Point {
            point: vector(at, name)?.to_array(),
        })
    };
    match takes {
        Takes::Flat => flat(),
        Takes::Round => round(),
        Takes::Point => point(),
        Takes::Any | Takes::FlatAndRound | Takes::Directed if face.contains_key("normal") => flat(),
        Takes::Any | Takes::FlatAndRound | Takes::Directed => round(),
        Takes::Anything if face.contains_key("normal") => flat(),
        Takes::Anything if face.contains_key("axis") => round(),
        Takes::Anything => point(),
        // The pin is a point, the slot a line.
        Takes::PointAndLine if name == "face" => point(),
        Takes::PointAndLine => round(),
        Takes::PointAndEdge | Takes::PointAndFace => point(),
    }
}
