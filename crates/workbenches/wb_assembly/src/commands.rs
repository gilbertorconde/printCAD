//! Assembly's commands: joints between bodies and moving bodies, by
//! numbers, for scripts and other callers that are not a click.
//!
//! A joint takes faces as `pc.doc.faces` lists them, where the bodies sit
//! now: a flat face as `{point, normal}`, a round one as its `axis`. Each
//! is stored in its body's own frame, as a picked face is, and every
//! joint solves as it is made or changed.

use core_document::{
    Args, BodyId, BodyPlacement, CommandArgs, CommandError, CommandResult, CommandSpec, FeatureId,
    ParamKind, WorkbenchContext, WorkbenchRuntimeContext,
};
use glam::{Quat, Vec3};
use serde_json::{Value, json};

use crate::joint::{Anchor, Drive, JOINT_KIND, JointFeature, JointKind, JointTool, Takes};
use crate::solve::joints;

const DRIVE: &str = "A hinge's angle (degrees from where it was made) or a slider's \
    position (mm) to hold it at; false lets it move again";
const LIMITS: &str = "{low, high}: the range a hinge's angle or a slider's position stays \
    in while not driven; false takes the limits away";

/// Read `drive` and `limits` into a hinge's or a slider's drive.
fn drive_args(a: &Args, drive: &mut Drive) -> Result<(), CommandError> {
    match a.0.get("drive") {
        None => {}
        Some(Value::Bool(false)) | Some(Value::Null) => drive.to = None,
        Some(v) => {
            let to = v
                .as_f64()
                .ok_or_else(|| CommandError::bad("drive", "must be a number or false"))?;
            drive.to = Some(to as f32);
        }
    }
    match a.0.get("limits") {
        None => {}
        Some(Value::Bool(false)) | Some(Value::Null) => drive.limits = None,
        Some(v) => {
            let bad = || CommandError::bad("limits", "must be {low, high} or false");
            let pair = v.as_array().filter(|l| l.len() == 2).ok_or_else(bad)?;
            let low = pair[0].as_f64().ok_or_else(bad)? as f32;
            let high = pair[1].as_f64().ok_or_else(bad)? as f32;
            if low > high {
                return Err(CommandError::bad("limits", "low must not be above high"));
            }
            drive.limits = Some([low, high]);
        }
    }
    Ok(())
}

/// Register every command this module runs.
pub fn register(context: &mut WorkbenchContext) {
    let joint = |id: &str, summary: &str, face: &str| {
        CommandSpec::new(id, summary)
            .param("body", ParamKind::Id, "The body that moves")
            .param("face", ParamKind::Any, face)
            .param("other", ParamKind::Id, "The body it is held against")
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
            _ => spec,
        };
        context.register_command(spec.returns("the joint's id"));
    }
    context.register_command(
        CommandSpec::new("asm.set", "Change a joint's gap, side, angle or radius")
            .param("joint", ParamKind::Id, "")
            .optional(
                "offset",
                ParamKind::Number,
                "A mate's gap, a hinge's height or a distance, mm",
            )
            .optional("flip", ParamKind::Bool, "A mate's side")
            .optional("degrees", ParamKind::Number, "An angle joint's angle")
            .optional("radius", ParamKind::Number, "A tangent's radius, mm")
            .optional("drive", ParamKind::Any, DRIVE)
            .optional("limits", ParamKind::Any, LIMITS),
    );
    context.register_command(
        CommandSpec::new(
            "asm.interference",
            "Where solid bodies share material: each pair that clashes, how much \
             and where",
        )
        .optional(
            "bodies",
            ParamKind::List,
            "Only these bodies; every visible one when left out",
        )
        .returns(
            "{checked, skipped, clashes}, each clash {a, b, volume (mm³), centre}; \
             skipped counts visible bodies with no solid",
        )
        .read_only(),
    );
    context.register_command(
        CommandSpec::new(
            "asm.parts",
            "Every part: bodies of the same shape counted together",
        )
        .returns(
            "a list of {name, quantity, bodies, size = {x, y, z} in mm or nil, mesh}, \
             in name order",
        )
        .read_only(),
    );
    context.register_command(
        CommandSpec::new(
            "asm.travel",
            "Where a hinge or a slider has got to: the hinge's angle in degrees, \
             the slider's position in mm",
        )
        .param("joint", ParamKind::Id, "")
        .returns("a number")
        .read_only(),
    );
    context.register_command(
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
    );
    context.register_command(
        CommandSpec::new(
            "asm.freedom",
            "What each jointed body may still do: the motions its joints leave open",
        )
        .optional("body", ParamKind::Id, "Only this body")
        .returns(
            "a list of {body, free, motions}, each motion {turn = {axis, through}} \
             or {slide = direction}",
        )
        .read_only(),
    );
    context.register_command(
        CommandSpec::new("asm.solve", "Place every body its joints hold")
            .returns("what moved, in words"),
    );
    context.register_command(
        CommandSpec::new("asm.placement", "Where a body sits")
            .param("body", ParamKind::Id, "")
            .returns("{translation, rotation}, rotation a quaternion {x, y, z, w}")
            .read_only(),
    );
    context.register_command(
        CommandSpec::new("asm.place", "Put a body at a placement")
            .param("body", ParamKind::Id, "")
            .optional("translation", ParamKind::List, "{x, y, z} in mm")
            .optional("rotation", ParamKind::List, "A quaternion {x, y, z, w}"),
    );
    context.register_command(
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
    );
}

/// Run command `id`.
pub fn run(id: &str, args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    match id {
        id if JointTool::of_command(id).is_some() => make_joint(id, &a, ctx),
        "asm.set" => {
            let joint = FeatureId(a.id("joint")?);
            let not_a_joint = || CommandError::bad("joint", "is not a joint");
            let node = ctx
                .document
                .get_feature_meta(joint)
                .filter(|n| n.workbench_id.as_str() == JOINT_KIND)
                .ok_or_else(not_a_joint)?;
            let mut feature: JointFeature =
                serde_json::from_value(node.data.clone()).map_err(|_| not_a_joint())?;
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
                JointKind::Align
                | JointKind::Ground
                | JointKind::Fixed { .. }
                | JointKind::Parallel
                | JointKind::Perpendicular => {}
            }
            let data =
                serde_json::to_value(&feature).map_err(|e| CommandError::failed(e.to_string()))?;
            ctx.document
                .update_feature_data(joint, data)
                .map_err(|e| CommandError::failed(e.to_string()))?;
            ctx.document.clear_feature_dirty(joint);
            solved(ctx, Value::Null)
        }
        "asm.interference" => {
            let kernel = ctx
                .kernel
                .ok_or_else(|| CommandError::failed("no kernel to check with"))?;
            let among = match a.0.get("bodies") {
                None | Some(Value::Null) => None,
                Some(list) => Some(
                    list.as_array()
                        .ok_or_else(|| CommandError::bad("bodies", "must be a list of ids"))?
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .and_then(|t| uuid::Uuid::parse_str(t).ok())
                                .map(BodyId)
                                .ok_or_else(|| CommandError::bad("bodies", "must be a list of ids"))
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                ),
            };
            let found = crate::interference(ctx.document, kernel, among.as_deref())
                .map_err(CommandError::failed)?;
            let clashes: Vec<Value> = found
                .clashes
                .iter()
                .map(|c| {
                    json!({
                        "a": c.a.0.to_string(),
                        "b": c.b.0.to_string(),
                        "volume": c.volume_mm3,
                        "centre": c.centre,
                    })
                })
                .collect();
            Ok(json!({
                "checked": found.checked,
                "skipped": found.skipped,
                "clashes": clashes,
            }))
        }
        "asm.parts" => Ok(Value::Array(
            crate::parts_list(ctx.document)
                .into_iter()
                .map(|part| {
                    json!({
                        "name": part.name,
                        "quantity": part.bodies.len(),
                        "bodies": part.bodies.iter().map(|b| b.0.to_string()).collect::<Vec<_>>(),
                        "size": part.size_mm,
                        "mesh": part.mesh,
                    })
                })
                .collect(),
        )),
        "asm.travel" => {
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
        "asm.ground" => {
            let body = BodyId(a.id("body")?);
            if !ctx.document.bodies().iter().any(|b| b.id == body) {
                return Err(CommandError::bad("body", "is not a body"));
            }
            let grounded = a.opt_bool("grounded")?.unwrap_or(true);
            let id = set_grounded(ctx, body, grounded);
            solved(ctx, id.map_or(Value::Null, |id| json!(id.0.to_string())))
        }
        "asm.freedom" => {
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
                                crate::Motion::Turn { axis, through } => {
                                    json!({"turn": {"axis": axis, "through": through}})
                                }
                                crate::Motion::Slide { direction } => json!({"slide": direction}),
                            }).collect::<Vec<_>>(),
                        })
                    })
                    .collect(),
            ))
        }
        "asm.solve" => crate::apply_solve(ctx)
            .map(Value::String)
            .map_err(CommandError::failed),
        "asm.placement" => {
            let placement = ctx.document.body_placement(body(&a, ctx)?);
            Ok(json!({
                "translation": placement.translation,
                "rotation": placement.rotation,
            }))
        }
        "asm.place" => {
            let body = body(&a, ctx)?;
            let now = ctx.document.body_placement(body);
            let translation = match a.0.get("translation") {
                Some(v) if !v.is_null() => vector(v, "translation")?,
                _ => Vec3::from(now.translation),
            };
            let rotation = match a.0.get("rotation") {
                Some(v) if !v.is_null() => quaternion(v)?,
                _ => now.quat(),
            };
            ctx.document
                .set_body_placement(body, BodyPlacement::new(rotation, translation));
            Ok(Value::Null)
        }
        "asm.move" => {
            let body = body(&a, ctx)?;
            let by = match a.0.get("by") {
                Some(v) if !v.is_null() => vector(v, "by")?,
                _ => Vec3::ZERO,
            };
            let axis = match a.0.get("axis") {
                Some(v) if !v.is_null() => vector(v, "axis")?,
                _ => Vec3::Z,
            };
            if axis.length() < 1e-9 {
                return Err(CommandError::bad("axis", "must not be zero"));
            }
            let about = match a.0.get("about") {
                Some(v) if !v.is_null() => vector(v, "about")?,
                _ => Vec3::ZERO,
            };
            let turn = Quat::from_axis_angle(
                axis.normalize(),
                (a.opt_number("turn")?.unwrap_or(0.0) as f32).to_radians(),
            );
            // Turn about `about`, then step: p -> turn (p - about) + about + by.
            let step = BodyPlacement::new(turn, about - turn * about + by);
            let now = ctx.document.body_placement(body);
            ctx.document.set_body_placement(body, step.after(&now));
            Ok(Value::Null)
        }
        _ => Err(CommandError::Unknown(id.to_string())),
    }
}

fn make_joint(id: &str, a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let moving_body = body(a, ctx)?;
    let other = BodyId(a.id("other")?);
    if !ctx.document.bodies().iter().any(|b| b.id == other) {
        return Err(CommandError::bad("other", "is not a body of this document"));
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
        None => {
            let number = joints(ctx.document)
                .iter()
                .filter(|j| j.feature.kind.label() == label)
                .count()
                + 1;
            format!("{label} {number}")
        }
    };
    let joint = JointFeature {
        kind,
        moving,
        other_body: other,
        fixed,
    };
    let feature = ctx
        .document
        .add_feature_in_body(joint, name, Some(moving_body))
        .map_err(|e| CommandError::failed(e.to_string()))?;
    // A joint has no solid to rebuild.
    ctx.document.clear_feature_dirty(feature);
    solved(ctx, json!(feature.0.to_string()))
}

/// A joint's task accepted, as a recording says it: a new joint as the
/// command that makes it, its faces where the bodies sat before it moved
/// them (where a replay finds them); an edited one as `asm.set` with what
/// changed.
#[cfg(feature = "egui")]
pub(crate) fn record_joint(
    ctx: &mut WorkbenchRuntimeContext,
    id: FeatureId,
    before: Option<&Value>,
    placements: &[(BodyId, BodyPlacement)],
) {
    let Some(node) = ctx.document.get_feature_meta(id).cloned() else {
        return;
    };
    let Ok(joint) = serde_json::from_value::<JointFeature>(node.data.clone()) else {
        return;
    };
    let settings = |kind: &JointKind| match *kind {
        JointKind::Mate { flip, offset } => json!({"offset": offset, "flip": flip}),
        JointKind::Angle { degrees } => json!({"degrees": degrees}),
        JointKind::Hinge { offset, drive, .. } => json!({
            "offset": offset,
            "drive": drive.to.map_or(json!(false), |v| json!(v)),
            "limits": drive.limits.map_or(json!(false), |l| json!(l)),
        }),
        JointKind::Slider { drive, .. } => json!({
            "drive": drive.to.map_or(json!(false), |v| json!(v)),
            "limits": drive.limits.map_or(json!(false), |l| json!(l)),
        }),
        JointKind::Distance { offset } => json!({"offset": offset}),
        JointKind::Tangent { radius } => json!({"radius": radius}),
        JointKind::Align
        | JointKind::Ground
        | JointKind::Fixed { .. }
        | JointKind::Parallel
        | JointKind::Perpendicular => json!({}),
    };
    match before {
        None => {
            let Some(body) = node.body else {
                return;
            };
            let at = |b: BodyId| {
                placements
                    .iter()
                    .find(|(p, _)| *p == b)
                    .map(|(_, placement)| *placement)
                    .unwrap_or_default()
            };
            let face = |anchor: &Anchor, b: BodyId| match anchor.moved(&at(b)) {
                Anchor::Plane { point, normal } => json!({"point": point, "normal": normal}),
                Anchor::Axis { point, direction } => {
                    json!({"axis": {"point": point, "direction": direction}})
                }
            };
            let command = match JointTool::of_kind(&joint.kind) {
                Some(tool) => tool.command(),
                None => {
                    ctx.record(
                        "asm.ground",
                        object(json!({"body": body.0.to_string()})),
                        json!(id.0.to_string()),
                    );
                    return;
                }
            };
            let mut args = object(json!({
                "body": body.0.to_string(),
                "face": face(&joint.moving, body),
                "other": joint.other_body.0.to_string(),
                "other_face": face(&joint.fixed, joint.other_body),
                "name": node.name,
            }));
            args.extend(object(settings(&joint.kind)));
            ctx.record(command, args, json!(id.0.to_string()));
        }
        Some(before) => {
            let Ok(old) = serde_json::from_value::<JointFeature>(before.clone()) else {
                return;
            };
            let (was, now) = (object(settings(&old.kind)), object(settings(&joint.kind)));
            let mut args = object(json!({"joint": id.0.to_string()}));
            for (name, value) in now {
                if was.get(&name) != Some(&value) {
                    args.insert(name, value);
                }
            }
            if args.len() > 1 {
                ctx.record("asm.set", args, Value::Null);
            }
        }
    }
}

/// A JSON object as named arguments.
pub(crate) fn object(value: Value) -> CommandArgs {
    match value {
        Value::Object(map) => map,
        _ => CommandArgs::new(),
    }
}

/// Solve, and answer `value` when every joint holds.
fn solved(ctx: &mut WorkbenchRuntimeContext, value: Value) -> CommandResult {
    crate::apply_solve(ctx).map_err(CommandError::failed)?;
    Ok(value)
}

fn body(a: &Args, ctx: &WorkbenchRuntimeContext) -> Result<BodyId, CommandError> {
    let body = BodyId(a.id("body")?);
    if ctx.document.bodies().iter().any(|b| b.id == body) {
        Ok(body)
    } else {
        Err(CommandError::bad("body", "is not a body of this document"))
    }
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
    match takes {
        Takes::Flat => flat(),
        Takes::Round => round(),
        Takes::Any | Takes::FlatAndRound if face.contains_key("normal") => flat(),
        Takes::Any | Takes::FlatAndRound => round(),
    }
}

fn vector(value: &Value, name: &str) -> Result<Vec3, CommandError> {
    let bad = || CommandError::bad(name, "must be {x, y, z}");
    let v = match value {
        Value::Array(v) if v.len() == 3 => [v[0].as_f64(), v[1].as_f64(), v[2].as_f64()],
        Value::Object(m) => ["x", "y", "z"].map(|k| m.get(k).and_then(Value::as_f64)),
        _ => return Err(bad()),
    };
    Ok(Vec3::new(
        v[0].ok_or_else(bad)? as f32,
        v[1].ok_or_else(bad)? as f32,
        v[2].ok_or_else(bad)? as f32,
    ))
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

/// Ground `body`, or let it move again: a ground joint on it, or none.
/// Answers the ground joint made.
pub(crate) fn set_grounded(
    ctx: &mut WorkbenchRuntimeContext,
    body: BodyId,
    grounded: bool,
) -> Option<FeatureId> {
    let existing: Vec<FeatureId> = crate::joints(ctx.document)
        .into_iter()
        .filter(|j| j.body == body && j.feature.kind == JointKind::Ground)
        .map(|j| j.id)
        .collect();
    if !grounded {
        for id in existing {
            let _ = ctx.document.remove_feature(id);
        }
        return None;
    }
    if let Some(id) = existing.first() {
        return Some(*id);
    }
    let anchor = Anchor::Plane {
        point: [0.0; 3],
        normal: [0.0, 0.0, 1.0],
    };
    ctx.document
        .add_feature_in_body(
            JointFeature {
                kind: JointKind::Ground,
                moving: anchor,
                other_body: body,
                fixed: anchor,
            },
            "Ground".into(),
            Some(body),
        )
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::Document;

    fn call(doc: &mut Document, id: &str, args: Value) -> CommandResult {
        let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        run(id, args.as_object().unwrap(), &mut ctx)
    }

    #[test]
    fn a_mate_puts_one_body_s_face_on_the_other_s() {
        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        call(
            &mut doc,
            "asm.move",
            json!({"body": a.0.to_string(), "by": [0, 0, 50]}),
        )
        .unwrap();
        // a's bottom (at z = 50 where it sits) onto b's top at z = 10.
        let joint = call(
            &mut doc,
            "asm.mate",
            json!({
                "body": a.0.to_string(),
                "face": {"point": [0, 0, 50], "normal": [0, 0, -1]},
                "other": b.0.to_string(),
                "other_face": {"point": [0, 0, 10], "normal": [0, 0, 1]},
                "offset": 2,
            }),
        )
        .unwrap();
        let z = doc.body_placement(a).translation[2];
        assert!((z - 12.0).abs() < 1e-3, "{z}");
        call(&mut doc, "asm.set", json!({"joint": joint, "offset": 5})).unwrap();
        assert!((doc.body_placement(a).translation[2] - 15.0).abs() < 1e-3);
        let err = call(
            &mut doc,
            "asm.align",
            json!({
                "body": a.0.to_string(),
                "face": {"point": [0, 0, 0], "normal": [0, 0, 1]},
                "other": b.0.to_string(),
                "other_face": {"point": [0, 0, 0], "normal": [0, 0, 1]},
            }),
        );
        assert!(err.is_err(), "an alignment takes round faces");
    }

    #[test]
    fn a_tangent_takes_its_radius_from_the_round_face_and_either_order() {
        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        let roller = json!({"axis": {"point": [0, 0, 0], "direction": [1, 0, 0]}, "radius": 4});
        let top = json!({"point": [0, 0, 10], "normal": [0, 0, 1]});
        let joint = call(
            &mut doc,
            "asm.tangent",
            json!({"body": a.0.to_string(), "face": roller, "other": b.0.to_string(), "other_face": top}),
        )
        .unwrap();
        assert!((doc.body_placement(a).translation[2] - 14.0).abs() < 1e-3);
        call(&mut doc, "asm.set", json!({"joint": joint, "radius": 6})).unwrap();
        assert!((doc.body_placement(a).translation[2] - 16.0).abs() < 1e-3);
        let both_flat = call(
            &mut doc,
            "asm.tangent",
            json!({"body": a.0.to_string(), "face": top, "other": b.0.to_string(), "other_face": top}),
        );
        assert!(both_flat.is_err(), "one face of each");
    }

    #[test]
    fn a_hinge_is_driven_to_an_angle_and_kept_within_its_limits() {
        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        let pin = json!({"axis": {"point": [0, 0, 0], "direction": [0, 0, 1]}});
        let joint = call(
            &mut doc,
            "asm.hinge",
            json!({"body": a.0.to_string(), "face": pin, "other": b.0.to_string(), "other_face": pin}),
        )
        .unwrap();
        let travel = |doc: &mut Document| {
            call(doc, "asm.travel", json!({"joint": joint}))
                .unwrap()
                .as_f64()
                .unwrap()
        };
        assert!(travel(&mut doc).abs() < 1e-6, "made where it sits");
        call(&mut doc, "asm.set", json!({"joint": joint, "drive": 30})).unwrap();
        assert!((travel(&mut doc) - 30.0).abs() < 1e-3);
        let x = doc.body_placement(a).direction([1.0, 0.0, 0.0]);
        assert!((x[1].atan2(x[0]).to_degrees() - 30.0).abs() < 1e-2, "{x:?}");
        // Let go and limited below where it is: it turns back to the limit.
        call(
            &mut doc,
            "asm.set",
            json!({"joint": joint, "drive": false, "limits": [-10, 10]}),
        )
        .unwrap();
        assert!((travel(&mut doc) - 10.0).abs() < 1e-2);
        // Within its limits it is still free to turn.
        assert_eq!(crate::freedom(&doc)[0].1.len(), 1);
        let bad = call(
            &mut doc,
            "asm.set",
            json!({"joint": joint, "limits": [5, -5]}),
        );
        assert!(bad.is_err());
    }

    #[test]
    fn a_slider_is_driven_along_its_axis() {
        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        let rail = json!({"axis": {"point": [0, 0, 0], "direction": [1, 0, 0]}});
        let joint = call(
            &mut doc,
            "asm.slider",
            json!({"body": a.0.to_string(), "face": rail, "other": b.0.to_string(),
                   "other_face": rail, "drive": 12.5}),
        )
        .unwrap();
        assert!((doc.body_placement(a).translation[0] - 12.5).abs() < 1e-3);
        assert!(crate::freedom(&doc)[0].1.is_empty(), "driven: nothing left");
        let travel = call(&mut doc, "asm.travel", json!({"joint": joint})).unwrap();
        assert!((travel.as_f64().unwrap() - 12.5).abs() < 1e-3);
    }

    #[test]
    fn a_fixed_joint_needs_no_faces_and_carries_the_body_along() {
        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        call(
            &mut doc,
            "asm.move",
            json!({"body": a.0.to_string(), "by": [5, 6, 7]}),
        )
        .unwrap();
        call(
            &mut doc,
            "asm.fix",
            json!({"body": a.0.to_string(), "other": b.0.to_string()}),
        )
        .unwrap();
        call(
            &mut doc,
            "asm.move",
            json!({"body": b.0.to_string(), "by": [100, 0, 0]}),
        )
        .unwrap();
        call(&mut doc, "asm.solve", json!({})).unwrap();
        let at = doc.body_placement(a).translation;
        assert!(
            (at[0] - 105.0).abs() < 1e-3 && (at[1] - 6.0).abs() < 1e-3,
            "{at:?}"
        );
    }

    #[test]
    fn a_move_turns_about_a_point_and_steps() {
        let mut doc = Document::new("t");
        let a = doc.create_body(None);
        call(
            &mut doc,
            "asm.move",
            json!({"body": a.0.to_string(), "turn": 90, "about": [10, 0, 0], "by": [0, 0, 5]}),
        )
        .unwrap();
        let placement = doc.body_placement(a);
        // The origin turned a quarter about (10, 0, 0) lands on (10, -10, 0).
        let p = placement.point([0.0, 0.0, 0.0]);
        assert!(
            (p[0] - 10.0).abs() < 1e-4 && (p[1] + 10.0).abs() < 1e-4 && (p[2] - 5.0).abs() < 1e-4,
            "{p:?}"
        );
        let listed = call(&mut doc, "asm.placement", json!({"body": a.0.to_string()})).unwrap();
        assert_eq!(listed["translation"][2], json!(5.0));
    }
}
