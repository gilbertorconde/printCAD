//! Assembly's commands: joints between bodies and moving bodies, by
//! numbers, for scripts and other callers that are not a click.
//!
//! A joint takes faces as `pc.doc.faces` lists them, where the bodies sit
//! now: a flat face as `{point, normal}`, a round one as its `axis`. Each
//! is stored in its body's own frame, as a picked face is, and every
//! joint solves as it is made or changed.

use core_document::{
    Args, BodyId, BodyPlacement, CommandArgs, CommandError, CommandResult, CommandSpec,
    ComponentId, FeatureId, ParamKind, WorkbenchContext, WorkbenchRuntimeContext,
};
use glam::{Quat, Vec3};
use serde_json::{Value, json};

use crate::coupling::{COUPLING_KIND, Coupling, Gearing};
use crate::joint::{Anchor, Drive, JOINT_KIND, JointFeature, JointKind, JointTool, Rigid, Takes};
use crate::solve::joints;

const DRIVE: &str = "A hinge's angle (degrees from where it was made) or a slider's \
    position (mm) to hold it at; false lets it move again";
const GEARING: &str = "gears (hinges turning opposite ways), belt (the same way), \
    rack (a hinge and a slider, by the pinion's pitch radius) or screw (by the lead); \
    the first that suits the two joints when left out";
const RATIO: &str = "Turns of the driven hinge per turn of the driver for gears and a \
    belt, the pitch radius in mm for a rack, the lead in mm a turn for a screw";
const LIMITS: &str = "{low, high}: the range a hinge's angle or a slider's position stays \
    in while not driven; false takes the limits away";

/// How far apart copies of `body` sit by default: its width along X and
/// a tenth more, in the world.
pub(crate) fn copy_step(document: &core_document::Document, body: BodyId) -> Vec3 {
    let width = document
        .imported_geometry(body)
        .and_then(|g| g.bounds_mm.or_else(|| g.mesh.bounds()))
        .map_or(20.0, |(lo, hi)| hi[0] - lo[0]);
    Vec3::new((width * 1.1).max(1.0), 0.0, 0.0)
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

/// The joint `tool` makes between these two anchors (each in its own
/// body's frame) where the bodies stand.
pub(crate) fn rejoined(
    ctx: &WorkbenchRuntimeContext,
    tool: JointTool,
    (moving_body, moving): (BodyId, Anchor),
    (other, fixed): (BodyId, Anchor),
    radius: f32,
) -> Result<JointFeature, CommandError> {
    if !tool.fits(&moving, &fixed) {
        return Err(CommandError::bad("kind", tool.refusal()));
    }
    if tool == JointTool::Tangent && radius <= 0.0 {
        return Err(CommandError::bad(
            "radius",
            "must be given where the round face has none",
        ));
    }
    let at = |b: BodyId| -> Rigid { ctx.document.body_placement(b).into() };
    Ok(JointFeature {
        second: None,
        shape: Vec::new(),
        ends: [0.0; 2],
        names: [0; 2],
        kind: tool.joint(&moving, &at(moving_body), &fixed, &at(other), radius),
        moving,
        other_body: other,
        fixed,
    })
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

/// The motion `study` names.
fn motion_study(
    a: &Args,
    document: &core_document::Document,
) -> Result<crate::MotionStudy, CommandError> {
    let not_a_motion = || CommandError::bad("study", "is not a motion");
    let node = document
        .get_feature_meta(FeatureId(a.id("study")?))
        .filter(|n| n.workbench_id.as_str() == crate::MOTION_KIND)
        .ok_or_else(not_a_motion)?;
    <crate::MotionStudy as core_document::WorkbenchFeature>::from_json(&node.data)
        .map_err(|_| not_a_motion())
}

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

/// Register every command this module runs.
pub fn register(context: &mut WorkbenchContext) {
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
        context.register_command(explained(
            tool.command(),
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
        ));
    }
    context.register_command(
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
        .returns("the coupling's id")
        .note(
            "Both joints must be hinges or sliders. `gearing` left out is the first that \
             suits them: gears for two hinges, a rack and pinion for a hinge and a slider.",
        )
        .note(
            "`ratio` is 1 when left out for gears and a belt, a 10 mm pitch radius for a \
             rack and a 2 mm lead for a screw; it must be above zero.",
        )
        .note(
            "The tie starts where both joints stand as it is made. Moving either moves the \
             other: `asm.turn` on the driven hinge turns the driver too.",
        )
        .see_also("asm.hinge")
        .see_also("asm.slider")
        .see_also("asm.set")
        .example(
            "Two gears, the second half as fast the other way",
            r#"
            local function disc(x, r)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.circle{sketch = s, x = x, y = 30, radius = r}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = 3}}.body
            end
            local f = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = f, x = -50, y = 0, width = 100, height = 10}
            local frame = pc.doc.feature{id = pc.design.pad{sketch = f, length = 2}}.body
            local small, big = disc(0, 10), disc(30, 20)
            assert(#pc.doc.rebuild() == 0)
            local function at(x)
              return {axis = {point = {x, 30, 0}, direction = {0, 0, 1}}}
            end
            local h1 = pc.asm.hinge{body = small, face = at(0), other = frame, other_face = at(0)}
            local h2 = pc.asm.hinge{body = big, face = at(30), other = frame, other_face = at(30)}
            pc.asm.couple{driver = h1, driven = h2, ratio = 0.5}
            pc.asm.set{joint = h1, drive = 40}
            assert(math.abs(pc.asm.travel{joint = h2} + 20) < 1e-3,
              "gears: half as far, the other way")
            "#,
        ),
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
    )
    .note(
        "Only what is given changes; a setting the joint's kind does not have, such as \
             `degrees` on a mate, is passed over without an error.",
    )
    .note(
        "`kind` makes the joint again as that kind from where the bodies stand, keeping \
             its faces and its name; the new kind's settings start afresh (a mate at offset \
             0) unless given in the same call. A kind that does not take the faces is refused.",
    )
    .note(
        "`face`, `other` and `other_face` pick again, in world space as `pc.doc.faces` \
             lists them. `moving_end` and `fixed_end` move each end of the joint along its \
             own normal or axis, mm.",
    )
    .note(
        "Given a coupling, it changes `gearing`, `ratio`, `reverse`, `driver` and \
             `driven`; a gearing that does not suit the joints is refused.",
    )
    .see_also("asm.couple")
    .see_also("asm.turn")
    .example(
        "A mate's gap changed, then the mate made a distance",
        r#"
        local function box(x, w, h, len)
          local s = pc.sketch.new{plane = "XY"}
          pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
          return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
        end
        local function facing(body, z)
          for _, f in ipairs(pc.doc.faces{body = body}) do
            if f.normal and f.normal[3] * z > 0.99 then return f end
          end
        end
        local base = box(0, 20, 20, 5)
        local lid = box(40, 10, 10, 3)
        assert(#pc.doc.rebuild() == 0)
        local j = pc.asm.mate{body = lid, face = facing(lid, -1),
          other = base, other_face = facing(base, 1)}
        local function height() return pc.asm.placement{body = lid}.translation[3] end
        pc.asm.set{joint = j, offset = 2}
        assert(math.abs(height() - 7) < 1e-4, "a 2 mm gap")
        pc.asm.set{joint = j, kind = "distance", offset = 4}
        assert(pc.doc.feature{id = j}.kind == "Distance")
        assert(math.abs(height() - 9) < 1e-4, "4 mm apart")
        "#,
    );
    context.register_command(align_drives(set));
    context.register_command(
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
        .returns("the copies' ids")
        .note(
            "A copy has no features of its own: it takes the source's shape and follows its \
             every rebuild, placed by its own placement.",
        )
        .note(
            "Left out, `step` puts each copy beside the one before along X, the body's width \
             and a tenth apart (22 mm for a 20 mm body). `count` is held between 1 and 500.",
        )
        .note(
            "With `around`, a whole turn (360, the default) is shared with the source, so 3 \
             copies stand at 90, 180 and 270 degrees; a part turn puts the last copy at \
             `angle`. `around` needs a `direction`.",
        )
        .see_also("asm.mirror")
        .see_also("asm.parts")
        .example(
            "Two copies in a row and a ring of three",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local pad = pc.design.pad{sketch = s, length = 5}
            local body = pc.doc.feature{id = pad}.body
            assert(#pc.doc.rebuild() == 0)
            local copies = pc.asm.copy{body = body, count = 2}
            assert(#copies == 2)
            local second = pc.asm.placement{body = copies[2]}.translation
            assert(math.abs(second[1] - 44) < 1e-4, "22 mm apart along X")
            pc.doc.set_value{id = pad, parameter = "length", value = 8}
            assert(#pc.doc.rebuild() == 0)
            local volume = pc.doc.measure{body = copies[1]}.volume
            assert(math.abs(volume - 20 * 10 * 8) < 1e-3, "the copy follows")
            local z = {point = {0, 0, 0}, direction = {0, 0, 1}}
            local ring = pc.asm.copy{body = body, count = 3, around = z}
            local w = pc.asm.placement{body = ring[1]}.rotation[4]
            assert(math.abs(math.deg(2 * math.acos(w)) - 90) < 1e-3,
              "a whole turn shared by four")
            "#,
        ),
    );
    context.register_command(
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
        .returns("the mirrored copy's id")
        .note(
            "The plane is in world space. The copy keeps the identity placement and draws \
             the source mirrored, following its changes.",
        )
        .note(
            "The mirrored mesh shows at once; its solid, which measuring and STEP export \
             read, is made by `pc.doc.rebuild()`.",
        )
        .note("A mirror of a mirrored copy is refused, as is a zero normal.")
        .see_also("asm.copy")
        .example(
            "A block mirrored across X = 0",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 10, y = 0, width = 20, height = 10}
            local body = pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
            assert(#pc.doc.rebuild() == 0)
            local m = pc.asm.mirror{body = body, point = {0, 0, 0}, normal = {1, 0, 0}}
            local faces = pc.doc.faces{body = m}
            assert(#faces == 6)
            for _, f in ipairs(faces) do
              local x = f.point[1]
              assert(x <= -10 + 1e-4 and x >= -30 - 1e-4, "on the other side of X = 0")
            end
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = m}.volume - 1000) < 1e-6, "its solid, mirrored")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.replace",
            "Put another body in a body's place, with its joints found again on the new body's faces",
        )
        .param("body", ParamKind::Id, "The body to replace; it is hidden")
        .param("with", ParamKind::Id, "The body that takes its place")
        .returns(
            "{kept, unmatched}: the joints whose ends were found on the new body, and those \
             that were not",
        )
        .note(
            "The replaced body is hidden, not deleted. Its joints move to the new body, found \
             again on its faces, and the new body is placed where they put it.",
        )
        .note("`kept` and `unmatched` list joint names. A body replacing itself is refused.")
        .see_also("asm.copy")
        .example(
            "A lid swapped for a thicker one",
            r#"
            local function box(x, w, h, len)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
            end
            local function facing(body, z)
              for _, f in ipairs(pc.doc.faces{body = body}) do
                if f.normal and f.normal[3] * z > 0.99 then return f end
              end
            end
            local base = box(0, 20, 20, 5)
            local lid = box(40, 10, 10, 3)
            local thicker = box(80, 12, 12, 4)
            assert(#pc.doc.rebuild() == 0)
            local j = pc.asm.mate{body = lid, face = facing(lid, -1),
              other = base, other_face = facing(base, 1)}
            local report = pc.asm.replace{body = lid, with = thicker}
            assert(#report.kept == 1 and #report.unmatched == 0)
            assert(pc.doc.feature{id = j}.body == thicker, "the mate is on the new body")
            local at = pc.asm.placement{body = thicker}.translation
            assert(math.abs(at[3] - 5) < 1e-4, "on the base's top")
            for _, b in ipairs(pc.doc.bodies()) do
              if b.id == lid then assert(not b.visible, "the old lid is hidden") end
            end
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.group",
            "Lock bodies together where they sit, in one rigid group",
        )
        .param(
            "bodies",
            ParamKind::List,
            "Two bodies or more; the first the one the rest hold to",
        )
        .optional(
            "group",
            ParamKind::Id,
            "A group to change to these bodies, rather than a new one",
        )
        .optional("name", ParamKind::String, "A new group's name in the tree")
        .returns("the group's id")
        .note(
            "The bodies hold to the first as they sit now, which leaves the others no \
             motion. Fewer than two bodies are refused.",
        )
        .note(
            "`asm.move` and `asm.place` move one member only; the rest follow at the next \
             solve. The bodies of a rigid `asm.component` move together at once.",
        )
        .see_also("asm.component")
        .see_also("asm.fix")
        .example(
            "Two blocks locked together",
            r#"
            local function box(x)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = 0, width = 5, height = 5}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
            end
            local a, b = box(0), box(10)
            assert(#pc.doc.rebuild() == 0)
            pc.asm.group{bodies = {a, b}}
            assert(pc.asm.freedom{body = b}[1].free == 0, "b holds to a")
            pc.asm.move{body = a, by = {0, 10, 0}}
            assert(pc.asm.placement{body = b}.translation[2] == 0, "a move places one body")
            pc.asm.solve{}
            local at = pc.asm.placement{body = b}.translation
            assert(math.abs(at[2] - 10) < 1e-4, "the solve brings b along")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.component",
            "Put bodies in a new component: one row in the tree that moves as one, or, \
             flexible, keeps the joints inside it live; components nest",
        )
        .param(
            "bodies",
            ParamKind::List,
            "The bodies it holds, taken out of any other",
        )
        .optional("name", ParamKind::String, "Its name in the tree")
        .optional(
            "parent",
            ParamKind::Id,
            "The component it sits in; the top if left out",
        )
        .optional(
            "flexible",
            ParamKind::Bool,
            "The joints inside it move (false: rigid)",
        )
        .returns("the component's id")
        .note(
            "Rigid, the default: `asm.move` or `asm.place` of any body in it moves every body \
             in it and in the components nested in it. Flexible: each body moves alone.",
        )
        .note(
            "A body is taken out of any component it was in. Components are not features: \
             `pc.doc.features` does not list them.",
        )
        .see_also("asm.component_set")
        .see_also("asm.component_add")
        .see_also("asm.group")
        .example(
            "Two blocks that move as one",
            r#"
            local function box(x)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = 0, width = 5, height = 5}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
            end
            local a, b, c = box(0), box(10), box(20)
            assert(#pc.doc.rebuild() == 0)
            pc.asm.component{bodies = {a, b}, name = "Pair"}
            pc.asm.move{body = a, by = {0, 7, 0}}
            assert(pc.asm.placement{body = b}.translation[2] == 7, "b moves with a")
            assert(pc.asm.placement{body = c}.translation[2] == 0, "c is not in it")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.component_set",
            "Rename a component, move it, or make it rigid or flexible",
        )
        .param("component", ParamKind::Id, "The component")
        .optional("name", ParamKind::String, "A new name")
        .optional("flexible", ParamKind::Bool, "The joints inside it move")
        .optional(
            "parent",
            ParamKind::Any,
            "The component it goes in, or \"top\" (or JSON null) for the top level",
        )
        .note(
            "Only what is given changes. Made rigid again, the bodies move together from \
             where they sit then.",
        )
        .note(
            "Lua has no null in a table, so `parent = \"top\"` takes a component out of \
             the one it is in, to the top level.",
        )
        .note("A component cannot be put inside itself; an unknown component is refused.")
        .see_also("asm.component")
        .see_also("asm.component_add")
        .example(
            "A component renamed, made flexible and rigid again",
            r#"
            local function box(x)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = 0, width = 5, height = 5}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
            end
            local a, b = box(0), box(10)
            assert(#pc.doc.rebuild() == 0)
            local k = pc.asm.component{bodies = {a, b}}
            pc.asm.component_set{component = k, name = "Loose pair", flexible = true}
            pc.asm.move{body = a, by = {0, 7, 0}}
            assert(pc.asm.placement{body = b}.translation[2] == 0, "flexible: a moves alone")
            pc.asm.component_set{component = k, flexible = false}
            pc.asm.move{body = a, by = {0, 1, 0}}
            assert(pc.asm.placement{body = b}.translation[2] == 1, "rigid: they move as one")
            "#,
        )
        .example(
            "A component put in another and taken out to the top level",
            r#"
            local function box(x)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = 0, width = 5, height = 5}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
            end
            local a, b = box(0), box(10)
            assert(#pc.doc.rebuild() == 0)
            local outer = pc.asm.component{bodies = {a}, name = "Frame"}
            local inner = pc.asm.component{bodies = {b}, name = "Arm"}
            pc.asm.component_set{component = inner, parent = outer}
            pc.asm.move{body = a, by = {0, 4, 0}}
            assert(pc.asm.placement{body = b}.translation[2] == 4, "inside the frame, it follows")
            pc.asm.component_set{component = inner, parent = "top"}
            pc.asm.move{body = a, by = {0, 4, 0}}
            assert(pc.asm.placement{body = b}.translation[2] == 4, "at the top, it stays")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.component_add",
            "Put bodies in a component, or take them out",
        )
        .param("bodies", ParamKind::List, "The bodies")
        .optional(
            "component",
            ParamKind::Id,
            "The component; left out, the bodies go to the top",
        )
        .note(
            "A body is in one component at a time: adding it takes it out of the one it was \
             in. Nothing moves.",
        )
        .see_also("asm.component")
        .see_also("asm.component_remove")
        .example(
            "A body put in a component and taken out again",
            r#"
            local function box(x)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = 0, width = 5, height = 5}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
            end
            local a, b, c = box(0), box(10), box(20)
            assert(#pc.doc.rebuild() == 0)
            local k = pc.asm.component{bodies = {a, b}}
            pc.asm.component_add{bodies = {c}, component = k}
            pc.asm.move{body = a, by = {0, 0, 3}}
            assert(pc.asm.placement{body = c}.translation[3] == 3, "c moves with them")
            pc.asm.component_add{bodies = {c}}
            pc.asm.move{body = a, by = {0, 0, 3}}
            assert(pc.asm.placement{body = c}.translation[3] == 3, "taken out, it stays")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.component_remove",
            "Take a component apart: its bodies and components go one level up",
        )
        .param("component", ParamKind::Id, "The component")
        .note(
            "Only the component goes: its bodies and nested components move one level up, \
             and nothing moves in the model.",
        )
        .see_also("asm.component")
        .example(
            "A component taken apart",
            r#"
            local function box(x)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = 0, width = 5, height = 5}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
            end
            local a, b = box(0), box(10)
            assert(#pc.doc.rebuild() == 0)
            local k = pc.asm.component{bodies = {a, b}}
            pc.asm.component_remove{component = k}
            pc.asm.move{body = a, by = {0, 0, 3}}
            assert(pc.asm.placement{body = b}.translation[3] == 0, "apart, a moves alone")
            local renamed = pcall(pc.asm.component_set, {component = k, name = "Gone"})
            assert(not renamed, "the component is gone")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.motion",
            "Keep a motion over time: hinges and sliders each driven by a formula of t, seconds",
        )
        .param(
            "drives",
            ParamKind::List,
            "{{joint = id, formula = \"90 * t\"}, ...}: a hinge's angle in degrees, a slider's \
             position in mm",
        )
        .optional(
            "start",
            ParamKind::Number,
            "When it starts, s (0 when left out)",
        )
        .optional(
            "end",
            ParamKind::Number,
            "When it ends, s (2 when left out)",
        )
        .optional(
            "step",
            ParamKind::Number,
            "The time between frames, s (0.05 when left out)",
        )
        .optional(
            "study",
            ParamKind::Id,
            "A motion to change, rather than a new one",
        )
        .optional("name", ParamKind::String, "A new motion's name in the tree")
        .returns("the motion's id")
        .note(
            "Each formula gives its drive's value at time `t`, seconds: a hinge's angle in \
             degrees, a slider's position in mm. A plain number takes the drive's unit; a \
             formula that does not read, or gives a length for a hinge (\"1 in * t\") or an \
             angle for a slider, is refused, and so is a joint that is not a hinge or a \
             slider.",
        )
        .note(
            "`end` is a Lua keyword: write `[\"end\"] = 1`. `start`, `end` and `step` are 0, \
             2 and 0.05 s when left out.",
        )
        .note(
            "Making it moves nothing: `asm.motion_frames` and `asm.trace` play it on a copy. \
             `study` changes a motion already made, keeping its id.",
        )
        .see_also("asm.motion_frames")
        .see_also("asm.trace")
        .see_also("asm.motion_clashes")
        .example(
            "A hinge turned 90 degrees a second, then changed",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
            local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
            local a = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = a, x = -2, y = -2, width = 30, height = 4}
            local arm = pc.doc.feature{id = pc.design.pad{sketch = a, length = 3}}.body
            assert(#pc.doc.rebuild() == 0)
            local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
            local h = pc.asm.hinge{body = arm, face = z, other = post, other_face = z}
            local m = pc.asm.motion{drives = {{joint = h, formula = "90 * t"}}, ["end"] = 1}
            assert(#pc.asm.motion_frames{study = m} == 21, "0 to 1 s every 0.05 s")
            local slower = {{joint = h, formula = "45 * t"}}
            pc.asm.motion{study = m, drives = slower, ["end"] = 1, step = 0.5}
            assert(#pc.asm.motion_frames{study = m} == 3, "the same motion, changed")
            assert(pc.asm.travel{joint = h} == 0, "making it moves nothing")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.motion_frames",
            "Every body's placement at each frame of a motion; nothing is moved",
        )
        .param("study", ParamKind::Id, "The motion")
        .returns("a list of {t, bodies = {{body, translation, rotation}, ...}}")
        .read_only()
        .note(
            "Frames run from `start` to `end`, both included, every `step`: 0 to 1 s every \
             0.25 s is 5 frames. Every body is in every frame.",
        )
        .note(
            "Each frame solves the joints with every drive held at its formula's value, on \
             a copy: the bodies stay where they are. An id that is not a motion is refused \
             (\"is not a motion\").",
        )
        .see_also("asm.motion")
        .see_also("asm.trace")
        .example(
            "Where an arm is at the end of its motion",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
            local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
            local a = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = a, x = -2, y = -2, width = 30, height = 4}
            local arm = pc.doc.feature{id = pc.design.pad{sketch = a, length = 3}}.body
            assert(#pc.doc.rebuild() == 0)
            local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
            local h = pc.asm.hinge{body = arm, face = z, other = post, other_face = z}
            local drives = {{joint = h, formula = "90 * t"}}
            local m = pc.asm.motion{drives = drives, ["end"] = 1, step = 0.25}
            local frames = pc.asm.motion_frames{study = m}
            local last = frames[#frames]
            assert(#frames == 5 and last.t == 1)
            for _, b in ipairs(last.bodies) do
              if b.body == arm then
                local turned = math.deg(2 * math.acos(b.rotation[4]))
                assert(math.abs(turned - 90) < 1e-3, "a quarter turn at 1 s")
              end
            end
            assert(pc.asm.placement{body = arm}.rotation[4] == 1, "the arm has not moved")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.trace",
            "Follow a point of a body through a motion: where it is and how fast at each frame",
        )
        .param("study", ParamKind::Id, "The motion")
        .param("body", ParamKind::Id, "The body")
        .param(
            "point",
            ParamKind::List,
            "{x, y, z} in the body's own frame",
        )
        .returns("a list of {t, point, speed (mm/s)}")
        .read_only()
        .note(
            "`point` is given in the body's own frame; each frame's `point` is where it is in \
             the world then. Nothing moves. An id that is not a motion is refused (\"is not \
             a motion\").",
        )
        .see_also("asm.motion_frames")
        .see_also("asm.motion")
        .example(
            "The tip of a turning arm",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
            local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
            local a = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = a, x = -2, y = -2, width = 30, height = 4}
            local arm = pc.doc.feature{id = pc.design.pad{sketch = a, length = 3}}.body
            assert(#pc.doc.rebuild() == 0)
            local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
            local h = pc.asm.hinge{body = arm, face = z, other = post, other_face = z}
            local m = pc.asm.motion{drives = {{joint = h, formula = "90 * t"}}, ["end"] = 1}
            local path = pc.asm.trace{study = m, body = arm, point = {28, 0, 0}}
            local last = path[#path]
            assert(math.abs(last.point[2] - 28) < 1e-3, "the arm's tip ends on +Y")
            assert(math.abs(last.speed - 28 * math.pi / 2) < 0.1,
              "a quarter turn a second at 28 mm")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.exploded_view",
            "Keep an exploded view: steps, each moving some bodies by a shift, played in order",
        )
        .param(
            "steps",
            ParamKind::List,
            "{{bodies = {ids}, shift = {x, y, z}}, ...}, in the order they play",
        )
        .optional(
            "view",
            ParamKind::Id,
            "A view to change, rather than a new one",
        )
        .optional("name", ParamKind::String, "A new view's name in the tree")
        .returns("the view's id")
        .note(
            "Each step moves its bodies by `shift`, mm in world space, on from the steps \
             before it; a step needs both `bodies` and `shift`.",
        )
        .note(
            "Making it moves nothing: `asm.explode_at` answers where it puts the bodies. \
             `view` changes a view already made, keeping its id.",
        )
        .see_also("asm.explode_at")
        .example(
            "A lid lifted, then the base moved aside",
            r#"
            local function box(x, w, h, len)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
            end
            local base = box(0, 20, 20, 5)
            local lid = box(40, 10, 10, 3)
            assert(#pc.doc.rebuild() == 0)
            local view = pc.asm.exploded_view{steps = {
              {bodies = {lid}, shift = {0, 0, 20}},
              {bodies = {base}, shift = {-15, 0, 0}},
            }}
            for _, p in ipairs(pc.asm.explode_at{view = view, at = 2}) do
              if p.body == lid then assert(p.translation[3] == 20) end
              if p.body == base then assert(p.translation[1] == -15) end
            end
            assert(pc.asm.placement{body = lid}.translation[3] == 0, "nothing has moved")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.explode_at",
            "Where an exploded view puts every body, part way through its steps",
        )
        .param("view", ParamKind::Id, "The exploded view")
        .param(
            "at",
            ParamKind::Number,
            "How many steps in: 1.5 is half way through the second",
        )
        .returns("a list of {body, translation, rotation}; nothing is moved")
        .read_only()
        .note(
            "Past the last step it stays at the end. Every body of the document is listed, \
             moved by the view or not.",
        )
        .see_also("asm.exploded_view")
        .example(
            "Half way through the second step",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
            local lid = pc.doc.feature{id = pc.design.pad{sketch = s, length = 3}}.body
            assert(#pc.doc.rebuild() == 0)
            local view = pc.asm.exploded_view{steps = {
              {bodies = {lid}, shift = {0, 0, 20}},
              {bodies = {lid}, shift = {10, 0, 0}},
            }}
            local at = pc.asm.explode_at{view = view, at = 1.5}[1].translation
            assert(at[1] == 5 and at[3] == 20, "the first step done, half the second")
            local past = pc.asm.explode_at{view = view, at = 9}[1].translation
            assert(past[1] == 10, "past the last step is the last step")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.save_state",
            "Save where every body sits, which are hidden and where drives hold, under a name",
        )
        .optional("name", ParamKind::String, "A new state's name in the tree")
        .optional(
            "state",
            ParamKind::Id,
            "A saved state to keep the assembly in instead",
        )
        .returns("the state's id")
        .note(
            "It keeps every body's placement, which bodies are hidden and the value each \
             drive holds. `state` saves the assembly as it is now over a state already \
             made, keeping its id.",
        )
        .see_also("asm.restore_state")
        .example(
            "A state saved again after a change",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
            local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
            local a = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = a, x = -2, y = -2, width = 30, height = 4}
            local arm = pc.doc.feature{id = pc.design.pad{sketch = a, length = 3}}.body
            assert(#pc.doc.rebuild() == 0)
            local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
            local h = pc.asm.hinge{body = arm, face = z, other = post, other_face = z,
              drive = 0}
            local closed = pc.asm.save_state{name = "Closed"}
            pc.asm.set{joint = h, drive = 90}
            pc.asm.save_state{state = closed}
            pc.asm.set{joint = h, drive = 10}
            pc.asm.restore_state{state = closed}
            assert(math.abs(pc.asm.travel{joint = h} - 90) < 1e-3, "saved again at 90")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.restore_state",
            "Put the assembly back as a saved state has it",
        )
        .param("state", ParamKind::Id, "The saved state")
        .note(
            "It puts back the placements, the hidden bodies (showing the rest) and the \
             drives' values, then solves. Anything but a saved state is refused.",
        )
        .see_also("asm.save_state")
        .example(
            "An opened arm put back, shown again",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
            local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
            local a = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = a, x = -2, y = -2, width = 30, height = 4}
            local arm = pc.doc.feature{id = pc.design.pad{sketch = a, length = 3}}.body
            assert(#pc.doc.rebuild() == 0)
            local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
            local h = pc.asm.hinge{body = arm, face = z, other = post, other_face = z,
              drive = 0}
            local closed = pc.asm.save_state{name = "Closed"}
            pc.asm.set{joint = h, drive = 90}
            pc.doc.set_visible{id = arm, visible = false}
            pc.asm.restore_state{state = closed}
            assert(math.abs(pc.asm.travel{joint = h}) < 1e-3, "the drive is back at 0")
            for _, b in ipairs(pc.doc.bodies()) do
              if b.id == arm then assert(b.visible, "and the arm is shown again") end
            end
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.redundant",
            "The joints that hold nothing a body's other joints do not",
        )
        .returns("a list of {joint, name}")
        .read_only()
        .note(
            "A joint is listed when its body's other joints already hold all it holds, such \
             as a parallel beside a mate of the same faces. Nothing is removed.",
        )
        .see_also("asm.freedom")
        .example(
            "A parallel that a mate makes needless",
            r#"
            local function box(x, w, h, len)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
            end
            local function facing(body, z)
              for _, f in ipairs(pc.doc.faces{body = body}) do
                if f.normal and f.normal[3] * z > 0.99 then return f end
              end
            end
            local base = box(0, 20, 20, 5)
            local lid = box(40, 10, 10, 3)
            assert(#pc.doc.rebuild() == 0)
            pc.asm.mate{body = lid, face = facing(lid, -1),
              other = base, other_face = facing(base, 1)}
            local p = pc.asm.parallel{body = lid, face = facing(lid, 1),
              other = base, other_face = facing(base, 1)}
            local extra = pc.asm.redundant{}
            assert(#extra == 1 and extra[1].joint == p and extra[1].name == "Parallel 1",
              "the mate already keeps them parallel")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.motion_clashes",
            "Step a hinge's or a slider's drive through a range and find where bodies collide",
        )
        .param("joint", ParamKind::Id, "The hinge or slider")
        .param(
            "low",
            ParamKind::Number,
            "Where the steps start: degrees or mm",
        )
        .param("high", ParamKind::Number, "Where they end")
        .optional(
            "steps",
            ParamKind::Number,
            "How many steps (24 when left out)",
        )
        .returns(
            "a list of {at, a, b, volume (mm³)}: each step and pair sharing more material \
             than where the joint stands",
        )
        .read_only()
        .note(
            "`steps` positions are checked, `low` and `high` among them: 3 from 0 to 180 are \
             0, 90 and 180. Each is solved on a copy: nothing moves.",
        )
        .note(
            "A pair is listed only where it shares more material than it does where the joint \
             stands now, so a contact already there does not count. A joint that is not a \
             hinge or a slider is refused.",
        )
        .see_also("asm.interference")
        .see_also("asm.motion")
        .example(
            "An arm swung into a post",
            r#"
            local function box(x, y, w, h, len)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = y, width = w, height = h}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
            end
            local base = box(-5, -5, 10, 10, 2)
            local arm = box(-2, -2, 30, 4, 3)
            local post = box(0, 15, 4, 4, 10)
            assert(#pc.doc.rebuild() == 0)
            local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
            local h = pc.asm.hinge{body = arm, face = z, other = base, other_face = z}
            local clashes = pc.asm.motion_clashes{joint = h, low = 0, high = 180, steps = 3}
            assert(#clashes == 1 and clashes[1].at == 90,
              "the arm hits the post a quarter turn round")
            assert(clashes[1].volume > 0)
            assert(pc.asm.travel{joint = h} == 0, "the arm is left where it was")
            "#,
        ),
    );
    context.register_command(
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
            r#"
            local function box(x, w, h, len)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
            end
            local function facing(body, z)
              for _, f in ipairs(pc.doc.faces{body = body}) do
                if f.normal and f.normal[3] * z > 0.99 then return f end
              end
            end
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
            "#,
        ),
    );
    context.register_command(
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
        .optional(
            "clearance",
            ParamKind::Number,
            "Look instead for pairs nearer than this many mm",
        )
        .returns(
            "{checked, skipped, clashes, unchecked}, each clash {a, b, volume (mm³), \
             centre}; skipped counts visible bodies with no solid, unchecked lists the \
             pairs the kernel failed on as {a, b, error}. With a clearance, {checked, \
             skipped, near, unchecked}, each near pair {a, b, distance (mm), on_a, on_b}, \
             nearest first",
        )
        .read_only()
        .note(
            "Faces that only touch, a body resting on another, are no clash. `volume` is the \
             shared material in mm³ and `centre` its middle, in world space.",
        )
        .note(
            "`clearance` answers the nearby pairs instead of the clashes, with the nearest \
             point on each body; it takes longer.",
        )
        .see_also("asm.motion_clashes")
        .see_also("asm.mass")
        .example(
            "A lid sunk 1 mm into its base",
            r#"
            local function box(x, w, h, len)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
            end
            local base = box(0, 20, 20, 5)
            local lid = box(5, 10, 10, 3)
            assert(#pc.doc.rebuild() == 0)
            pc.asm.move{body = lid, by = {0, 0, 5}}
            assert(#pc.asm.interference{}.clashes == 0, "resting on the top is no clash")
            pc.asm.move{body = lid, by = {0, 0, -1}}
            local found = pc.asm.interference{}
            assert(found.checked == 2 and #found.clashes == 1)
            local clash = found.clashes[1]
            assert(math.abs(clash.volume - 10 * 10 * 1) < 1e-3, "1 mm of the lid sunk in")
            assert(math.abs(clash.centre[3] - 4.5) < 1e-3)
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.mass",
            "The mass and centre of mass of the solid bodies at one density",
        )
        .optional(
            "bodies",
            ParamKind::List,
            "Only these bodies; every visible one when left out",
        )
        .optional(
            "density",
            ParamKind::Number,
            "g/cm³ for bodies without a material (1 when left out)",
        )
        .returns(
            "{mass (g), volume (mm³), centre = {x, y, z} or nil, bodies = {{body, mass, \
             volume, centre}, ...}, skipped}",
        )
        .read_only()
        .note(
            "`centre` is the centre of mass in world space, mm. An id in `bodies` that is not \
             a body is passed over, so a list of feature ids answers a mass of 0.",
        )
        .see_also("asm.parts")
        .see_also("asm.interference")
        .example(
            "A block's mass at 1.24 g/cm³",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 20}
            local block = pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
            assert(#pc.doc.rebuild() == 0)
            local m = pc.asm.mass{density = 1.24}
            assert(math.abs(m.volume - 2000) < 1e-3)
            assert(math.abs(m.mass - 2.48) < 1e-6, "2 cm³ at 1.24 g/cm³")
            assert(m.centre[1] == 10 and m.centre[3] == 2.5)
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.parts",
            "Every part: bodies of the same shape counted together",
        )
        .optional(
            "by_component",
            ParamKind::Bool,
            "Each component's parts under it: every entry gains a depth, and components \
             come as {component, name, depth}",
        )
        .returns(
            "a list of {name, quantity, bodies, size = {x, y, z} in mm or nil, mesh, number \
             or nil, bought, values = {column = text}, print, volume (mm³) or nil, mass (g) \
             or nil, density}, numbered parts first by number, then by name",
        )
        .read_only()
        .note(
            "Bodies count as one part when they share one shape: linked copies from \
             `asm.copy` do; bodies modelled apart do not, however alike.",
        )
        .note("`size` is the part's bounding box, mm.")
        .note(
            "`volume` and `mass` are of one piece: the volume the kernel measures of its \
             solid (a mesh body's from its triangles), the mass at its body's material's \
             density, else at the list's printing material (`asm.print_material`). `print` \
             is how many to print: the count set with `asm.part`, else one per body, none \
             of a bought part.",
        )
        .see_also("asm.part")
        .see_also("asm.parts_table")
        .see_also("asm.copy")
        .example(
            "A plate and its copies counted as one part",
            r#"
            local function box(x)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = 0, width = 10, height = 10}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = 2}}.body
            end
            local plate = box(0)
            local other = box(20)
            assert(#pc.doc.rebuild() == 0)
            pc.asm.copy{body = plate, count = 2}
            local parts = pc.asm.parts{}
            assert(#parts == 2, "alike bodies modelled apart are two parts")
            local counts = {}
            for _, p in ipairs(parts) do counts[p.quantity] = p end
            assert(counts[3] and counts[3].size[3] == 2,
              "the plate and its two copies: one part, three of it")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.part",
            "Set what the parts list keeps for a part: its number, whether it is bought, \
             its values in the added columns",
        )
        .param("body", ParamKind::Id, "Any body of the part")
        .optional("number", ParamKind::Number, "Its item number")
        .optional(
            "bought",
            ParamKind::Bool,
            "Bought rather than made: left out of exports and the slicer",
        )
        .optional(
            "values",
            ParamKind::Any,
            "{column = text}: its values, a column not yet in the list added to it",
        )
        .optional(
            "print",
            ParamKind::Number,
            "How many of it to print; below 0 goes back to one per body",
        )
        .note(
            "What is set is kept for the whole part, every body of its shape, whichever \
             body is named.",
        )
        .note(
            "A `number` of 0 or less takes the number away. A value that is not text is kept \
             as text: 3 becomes \"3\".",
        )
        .see_also("asm.parts")
        .see_also("asm.parts_table")
        .example(
            "A bought screw numbered on one of its copies",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
            local screw = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
            assert(#pc.doc.rebuild() == 0)
            local copies = pc.asm.copy{body = screw, count = 3}
            pc.asm.part{body = copies[2], number = 4, bought = true,
              values = {Supplier = "ACME"}}
            local p = pc.asm.parts{}[1]
            assert(p.quantity == 4 and p.number == 4 and p.bought,
              "set on one, kept for the part")
            assert(p.values.Supplier == "ACME")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.print_material",
            "Set the material the parts list weighs printed parts in",
        )
        .param(
            "name",
            ParamKind::String,
            "PLA, PETG, ABS, ASA, TPU, Nylon or PC, or a name of your own with its density",
        )
        .optional(
            "density",
            ParamKind::Number,
            "g/cm³; an offered material's own when left out",
        )
        .returns("{name, density}")
        .note(
            "It is kept with the document. A body with a material of its own \
             (`doc.set_body`) is weighed at that material's density instead.",
        )
        .note("A name the list does not offer needs its density.")
        .see_also("asm.parts")
        .see_also("asm.part")
        .example(
            "A plate's filament in PETG, two to print",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local plate = pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
            assert(#pc.doc.rebuild() == 0)
            local m = pc.asm.print_material{name = "petg"}
            assert(m.name == "PETG" and math.abs(m.density - 1.27) < 1e-6)
            pc.asm.part{body = plate, print = 2}
            local p = pc.asm.parts{}[1]
            assert(p.print == 2 and math.abs(p.volume - 1000) < 1e-3)
            assert(math.abs(p.mass - 1.27) < 1e-4, "1 cm³ at 1.27 g/cm³")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.parts_table",
            "Replace what the parts list keeps, whole",
        )
        .param(
            "table",
            ParamKind::Any,
            "{columns = {...}, entries = {[body id] = {number, bought, values}}}",
        )
        .note(
            "It replaces the whole list: a number, bought mark or value the table leaves out \
             is gone.",
        )
        .note(
            "An empty Lua table goes as a list and `entries` refuses it: leave `entries` out \
             for none.",
        )
        .see_also("asm.part")
        .see_also("asm.parts")
        .example(
            "The parts list written whole",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
            local screw = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
            assert(#pc.doc.rebuild() == 0)
            pc.asm.part{body = screw, number = 9, values = {Note = "old"}}
            pc.asm.parts_table{table = {
              columns = {"Supplier"},
              entries = {[screw] = {number = 2, bought = true, values = {Supplier = "ACME"}}},
            }}
            local p = pc.asm.parts{}[1]
            assert(p.number == 2 and p.bought and p.values.Supplier == "ACME")
            assert(p.values.Note == nil, "what the table left out is gone")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            "asm.travel",
            "Where a hinge or a slider has got to: the hinge's angle in degrees, \
             the slider's position in mm",
        )
        .param("joint", ParamKind::Id, "")
        .returns("a number")
        .read_only()
        .note(
            "It is read from where the bodies sit now, counted from where the joint was made, \
             so a body moved by hand reads its new travel before any solve.",
        )
        .note("Any other joint is refused, an alignment too.")
        .see_also("asm.hinge")
        .see_also("asm.slider")
        .see_also("asm.turn")
        .example(
            "A hinge's angle after its arm is moved",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
            local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
            local a = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = a, x = -2, y = -2, width = 30, height = 4}
            local arm = pc.doc.feature{id = pc.design.pad{sketch = a, length = 3}}.body
            assert(#pc.doc.rebuild() == 0)
            local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
            local h = pc.asm.hinge{body = arm, face = z, other = post, other_face = z}
            assert(pc.asm.travel{joint = h} == 0, "0 where it was made")
            pc.asm.move{body = arm, turn = 45}
            assert(math.abs(pc.asm.travel{joint = h} - 45) < 1e-3,
              "read from where the arm sits")
            "#,
        ),
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
        .returns("the ground joint's id, or nil when it was taken away")
        .note(
            "Grounding a body already grounded answers its ground joint again. The \
             document's first joint grounds the body its `other` names, so a ground is often \
             there already.",
        )
        .see_also("asm.fix")
        .see_also("asm.freedom")
        .example(
            "A base kept where it is",
            r#"
            local function box(x, w, h, len)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
            end
            local function facing(body, z)
              for _, f in ipairs(pc.doc.faces{body = body}) do
                if f.normal and f.normal[3] * z > 0.99 then return f end
              end
            end
            local base = box(0, 20, 20, 5)
            local lid = box(40, 10, 10, 3)
            assert(#pc.doc.rebuild() == 0)
            local g = pc.asm.ground{body = base}
            assert(pc.asm.ground{body = base} == g, "grounding twice keeps one ground")
            pc.asm.mate{body = lid, face = facing(lid, -1),
              other = base, other_face = facing(base, 1)}
            assert(pc.asm.placement{body = base}.translation[3] == 0, "the ground stays")
            assert(pc.asm.ground{body = base, grounded = false} == nil)
            "#,
        ),
    );
    context.register_command(
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
        .read_only()
        .note(
            "Grounded bodies, and bodies no joint moves, are not listed. A held drive takes \
             its motion away; a limit does not: a hinge, slider or alignment resting on a \
             limit keeps the motion, `at_limit`. A point on a path keeps its run along it at \
             a corner too, and at an open path's end `at_limit`.",
        )
        .note("`through` is a point on a turn's axis, in world space.")
        .see_also("asm.redundant")
        .see_also("asm.solve")
        .example(
            "What a mate leaves a lid",
            r#"
            local function box(x, w, h, len)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
            end
            local function facing(body, z)
              for _, f in ipairs(pc.doc.faces{body = body}) do
                if f.normal and f.normal[3] * z > 0.99 then return f end
              end
            end
            local base = box(0, 20, 20, 5)
            local lid = box(40, 10, 10, 3)
            assert(#pc.doc.rebuild() == 0)
            pc.asm.mate{body = lid, face = facing(lid, -1),
              other = base, other_face = facing(base, 1)}
            local all = pc.asm.freedom{}
            assert(#all == 1 and all[1].body == lid, "the grounded base is not listed")
            local turns, slides = 0, 0
            for _, m in ipairs(all[1].motions) do
              if m.turn then turns = turns + 1; assert(m.turn.axis[3] == 1) end
              if m.slide then slides = slides + 1; assert(m.slide[3] == 0) end
            end
            assert(turns == 1 and slides == 2,
              "on a face: a turn about its normal and two slides along it")
            "#,
        ),
    );
    context.register_command(
        CommandSpec::new("asm.solve", "Place every body its joints hold")
            .returns("what moved, in words")
            .note(
                "The joint commands, `asm.set` and `asm.ground` solve as they run; \
                 `asm.place` and `asm.move` do not, so solve after them.",
            )
            .note(
                "It answers \"Moved 1 body; every joint holds\", or \"Every joint holds\" \
                 when nothing had to move.",
            )
            .note(
                "A joint moves the body it belongs to; one that belongs to a grounded body \
                 moves the body at its other end instead. A joint between two bodies that \
                 cannot move (both grounded) and does not hold is refused, named \
                 (\"cannot hold all its joints at once\").",
            )
            .see_also("asm.place")
            .see_also("asm.move")
            .example(
                "A lid put back on its base",
                r#"
                local function box(x, w, h, len)
                  local s = pc.sketch.new{plane = "XY"}
                  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
                  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
                end
                local function facing(body, z)
                  for _, f in ipairs(pc.doc.faces{body = body}) do
                    if f.normal and f.normal[3] * z > 0.99 then return f end
                  end
                end
                local base = box(0, 20, 20, 5)
                local lid = box(40, 10, 10, 3)
                assert(#pc.doc.rebuild() == 0)
                pc.asm.mate{body = lid, face = facing(lid, -1),
                  other = base, other_face = facing(base, 1)}
                local function height() return pc.asm.placement{body = lid}.translation[3] end
                pc.asm.place{body = lid, translation = {0, 0, 30}}
                assert(height() == 30, "a placement does not solve")
                assert(pc.asm.solve{} == "Moved 1 body; every joint holds")
                assert(math.abs(height() - 5) < 1e-4)
                assert(pc.asm.solve{} == "Every joint holds")
                "#,
            ),
    );
    context.register_command(
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
    );
    context.register_command(
        CommandSpec::new("asm.place", "Put a body at a placement")
            .param("body", ParamKind::Id, "")
            .optional("translation", ParamKind::List, "{x, y, z} in mm")
            .optional("rotation", ParamKind::List, "A quaternion {x, y, z, w}")
            .note(
                "It sets the placement outright; what is left out keeps its value. The \
                 quaternion is normalised, and a zero one is refused.",
            )
            .note(
                "It does not solve: joints catch up at the next `asm.solve`. The other \
                 bodies of a rigid component move with it.",
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
    );
}

/// A joint command's notes, related commands and example.
fn explained(id: &str, spec: CommandSpec) -> CommandSpec {
    match id {
        "asm.mate" => spec
            .note(
                "`offset` is the gap between the faces in mm, 0 when left out; `flip = true` \
                 turns the body so both faces point the same way.",
            )
            .note(
                "It holds the faces together and nothing else: the body keeps a turn about \
                 the normal and two slides along the face, so it is not centred and stays \
                 where it was across the face.",
            )
            .note("Both faces must be flat: a round face is refused as having no normal.")
            .see_also("asm.distance")
            .see_also("asm.flip")
            .example(
                "A lid set on a base",
                r#"
                local function box(x, w, h, len)
                  local s = pc.sketch.new{plane = "XY"}
                  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
                  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
                end
                local function facing(body, z)
                  for _, f in ipairs(pc.doc.faces{body = body}) do
                    if f.normal and f.normal[3] * z > 0.99 then return f end
                  end
                end
                local base = box(0, 20, 20, 5)
                local lid = box(40, 10, 10, 3)
                assert(#pc.doc.rebuild() == 0)
                pc.asm.mate{body = lid, face = facing(lid, -1),
                  other = base, other_face = facing(base, 1)}
                local at = pc.asm.placement{body = lid}.translation
                assert(at[1] == 0 and at[2] == 0 and math.abs(at[3] - 5) < 1e-4,
                  "lifted onto the top, not moved across")
                assert(pc.asm.freedom{body = lid}[1].free == 3,
                  "it may still slide and turn on the face")
                "#,
            ),
        "asm.align" => spec
            .note(
                "The two axes go on one line; the body keeps a turn about it and a slide along it.",
            )
            .note(
                "`turn_drive` (degrees from where it was made) and `slide_drive` (mm) hold \
                 those motions; `turn_limits` and `slide_limits` keep them in a range; false \
                 takes each away.",
            )
            .note(
                "`asm.travel` refuses an alignment: it reads a hinge or a slider, which have \
                 one motion.",
            )
            .see_also("asm.hinge")
            .see_also("asm.slider")
            .example(
                "A wheel on a shaft, held 8 mm up",
                r#"
                local function pin(x, r, len)
                  local s = pc.sketch.new{plane = "XY"}
                  pc.sketch.circle{sketch = s, x = x, y = 0, radius = r}
                  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
                end
                local function round(body)
                  for _, f in ipairs(pc.doc.faces{body = body}) do
                    if f.axis then return f end
                  end
                end
                local shaft = pin(0, 5, 20)
                local wheel = pin(40, 15, 4)
                assert(#pc.doc.rebuild() == 0)
                pc.asm.align{body = wheel, face = round(wheel),
                  other = shaft, other_face = round(shaft), slide_drive = 8}
                local at = pc.asm.placement{body = wheel}.translation
                assert(math.abs(at[1] + 40) < 1e-3 and math.abs(at[3] - 8) < 1e-3,
                  "on the shaft's axis, 8 mm up")
                assert(pc.asm.freedom{body = wheel}[1].free == 1,
                  "the slide is held, the turn is free")
                "#,
            ),
        "asm.angle" => spec
            .note(
                "`degrees` is between the outward normals, or the axes; left out, the angle \
                 they make now is kept.",
            )
            .note(
                "It holds only the angle: the body may still slide every way and turn about \
                 the other axes, five motions left.",
            )
            .see_also("asm.parallel")
            .see_also("asm.perpendicular")
            .example(
                "A plate held at 30 degrees to a base",
                r#"
                local function box(x, w, h, len)
                  local s = pc.sketch.new{plane = "XY"}
                  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
                  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
                end
                local function facing(body, z)
                  for _, f in ipairs(pc.doc.faces{body = body}) do
                    if f.normal and f.normal[3] * z > 0.99 then return f end
                  end
                end
                local base = box(0, 20, 20, 5)
                local plate = box(40, 10, 10, 2)
                assert(#pc.doc.rebuild() == 0)
                pc.asm.angle{body = plate, face = facing(plate, 1),
                  other = base, other_face = facing(base, 1), degrees = 30}
                local w = pc.asm.placement{body = plate}.rotation[4]
                assert(math.abs(math.deg(2 * math.acos(w)) - 30) < 1e-3, "turned 30 degrees")
                assert(pc.asm.freedom{body = plate}[1].free == 5, "only the angle is held")
                "#,
            ),
        "asm.hinge" => spec
            .note(
                "The axes go on one line and the body keeps one motion, the turn about it. \
                 `offset` is how far along the axis the first sits from the second, mm.",
            )
            .note(
                "`drive` holds the angle in degrees from where the hinge was made, positive \
                 turning right-handed about the axis's direction; false lets it turn again.",
            )
            .note(
                "`limits = {low, high}` keeps the angle in that range while it is not driven; \
                 a body outside it is brought to the nearer end.",
            )
            .see_also("asm.travel")
            .see_also("asm.turn")
            .see_also("asm.couple")
            .see_also("asm.motion")
            .example(
                "An arm on a post, turned a quarter turn",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
                local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
                local a = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = a, x = -2, y = 20, width = 30, height = 4}
                local arm = pc.doc.feature{id = pc.design.pad{sketch = a, length = 3}}.body
                assert(#pc.doc.rebuild() == 0)
                local axis = {axis = {point = {0, 22, 0}, direction = {0, 0, 1}}}
                local post_axis = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
                local h = pc.asm.hinge{body = arm, face = axis,
                  other = post, other_face = post_axis}
                assert(pc.asm.freedom{body = arm}[1].free == 1, "it turns about the post")
                pc.asm.set{joint = h, drive = 90}
                assert(math.abs(pc.asm.travel{joint = h} - 90) < 1e-3)
                local q = pc.asm.placement{body = arm}.rotation
                assert(math.abs(q[3] - math.sin(math.rad(45))) < 1e-4, "a quarter turn about +Z")
                "#,
            ),
        "asm.slider" => spec
            .note(
                "The axes go on one line and the body keeps one motion, the slide along it; \
                 it does not turn.",
            )
            .note(
                "`drive` holds the position in mm from where the slider was made, positive \
                 along the axis's direction; `limits` keeps it within {low, high}.",
            )
            .see_also("asm.travel")
            .see_also("asm.couple")
            .see_also("asm.align")
            .example(
                "A carriage on a rail, 30 mm along",
                r#"
                local function box(x, w, h, len)
                  local s = pc.sketch.new{plane = "XY"}
                  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
                  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
                end
                local rail = box(0, 100, 10, 5)
                local carriage = box(0, 20, 10, 5)
                assert(#pc.doc.rebuild() == 0)
                local below = {axis = {point = {0, 5, 0}, direction = {1, 0, 0}}}
                local along = {axis = {point = {0, 5, 5}, direction = {1, 0, 0}}}
                local s = pc.asm.slider{body = carriage, face = below,
                  other = rail, other_face = along, limits = {0, 80}}
                pc.asm.set{joint = s, drive = 30}
                local at = pc.asm.placement{body = carriage}.translation
                assert(math.abs(at[1] - 30) < 1e-3 and math.abs(at[3] - 5) < 1e-3,
                  "on the rail, 30 mm along")
                assert(math.abs(pc.asm.travel{joint = s} - 30) < 1e-3)
                "#,
            ),
        "asm.fix" => spec
            .note(
                "With no faces it holds the body to the other where both sit now: nothing \
                 moves as it is made, and no motion is left.",
            )
            .note(
                "The fixed body follows the other at the next solve: `asm.move` and \
                 `asm.place` of the other do not carry it until then.",
            )
            .see_also("asm.group")
            .see_also("asm.ground")
            .example(
                "A tag that follows its base",
                r#"
                local function box(x, w, h, len)
                  local s = pc.sketch.new{plane = "XY"}
                  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
                  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
                end
                local base = box(0, 20, 20, 5)
                local tag = box(30, 5, 5, 5)
                assert(#pc.doc.rebuild() == 0)
                pc.asm.fix{body = tag, other = base}
                assert(pc.asm.freedom{body = tag}[1].free == 0)
                pc.asm.move{body = base, by = {0, 0, 10}}
                pc.asm.solve{}
                local at = pc.asm.placement{body = tag}.translation
                assert(math.abs(at[3] - 10) < 1e-4, "the tag follows the base")
                "#,
            ),
        "asm.parallel" => spec
            .note(
                "It turns the body until the faces or axes are parallel and holds only that: \
                 the body may still slide every way and turn about the normal, four motions \
                 left.",
            )
            .see_also("asm.angle")
            .see_also("asm.perpendicular")
            .see_also("asm.mate")
            .example(
                "A tilted plate turned back square",
                r#"
                local function box(x, w, h, len)
                  local s = pc.sketch.new{plane = "XY"}
                  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
                  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
                end
                local base = box(0, 20, 20, 5)
                local plate = box(40, 10, 10, 2)
                assert(#pc.doc.rebuild() == 0)
                pc.asm.move{body = plate, turn = 20, axis = {0, 1, 0}}
                local top = pc.doc.faces{body = plate}[6]
                local base_top = pc.doc.faces{body = base}[6]
                pc.asm.parallel{body = plate, face = top, other = base, other_face = base_top}
                local w = pc.asm.placement{body = plate}.rotation[4]
                assert(math.abs(w - 1) < 1e-6, "turned back square")
                assert(pc.asm.freedom{body = plate}[1].free == 4,
                  "it still slides every way and turns about Z")
                "#,
            ),
        "asm.perpendicular" => spec
            .note(
                "It turns the body until the faces or axes are square to each other and \
                 holds only that, five motions left.",
            )
            .see_also("asm.parallel")
            .see_also("asm.angle")
            .example(
                "A fin stood square to a base",
                r#"
                local function box(x, w, h, len)
                  local s = pc.sketch.new{plane = "XY"}
                  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
                  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
                end
                local base = box(0, 20, 20, 5)
                local fin = box(40, 10, 10, 2)
                assert(#pc.doc.rebuild() == 0)
                pc.asm.move{body = fin, turn = 60, axis = {0, 1, 0}}
                pc.asm.perpendicular{body = fin, face = pc.doc.faces{body = fin}[6],
                  other = base, other_face = pc.doc.faces{body = base}[6]}
                local n = pc.doc.faces{body = fin}[6].normal
                assert(math.abs(n[3]) < 1e-4, "the fin's face stands square to the base's top")
                assert(pc.asm.freedom{body = fin}[1].free == 5)
                "#,
            ),
        "asm.distance" => spec
            .note(
                "Between flat faces, `offset` is measured along the second face's normal, \
                 mm; left out, the distance they are apart now is kept.",
            )
            .note(
                "It holds only that distance: unlike a mate with an offset, the body may \
                 still tilt and slide, five motions left.",
            )
            .note(
                "Faces, axes `{axis}` and points `{centre}` or `{point}` mix: a face's \
                 distance from a point, an axis's from an axis.",
            )
            .see_also("asm.mate")
            .example(
                "A plate held 10 mm above a base",
                r#"
                local function box(x, w, h, len)
                  local s = pc.sketch.new{plane = "XY"}
                  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
                  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
                end
                local function facing(body, z)
                  for _, f in ipairs(pc.doc.faces{body = body}) do
                    if f.normal and f.normal[3] * z > 0.99 then return f end
                  end
                end
                local base = box(0, 20, 20, 5)
                local plate = box(40, 10, 10, 2)
                assert(#pc.doc.rebuild() == 0)
                pc.asm.distance{body = plate, face = facing(plate, -1),
                  other = base, other_face = facing(base, 1), offset = 10}
                local at = pc.asm.placement{body = plate}.translation
                assert(math.abs(at[3] - 15) < 1e-4, "10 mm above the base's top")
                "#,
            ),
        "asm.tangent" => spec
            .note(
                "One face is flat and the other round, either way round; two of one kind \
                 are refused.",
            )
            .note(
                "The round face's radius comes from the face, as `pc.doc.faces` lists it \
                 with `radius`, or from the `radius` argument; with neither the joint is \
                 refused.",
            )
            .note("It leaves the body four motions.")
            .see_also("asm.mate")
            .see_also("asm.cam")
            .example(
                "A roller against a block's side",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 40, height = 40}
                local base = pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
                local r = pc.sketch.new{plane = "XY"}
                pc.sketch.circle{sketch = r, x = 80, y = 0, radius = 3}
                local roller = pc.doc.feature{id = pc.design.pad{sketch = r, length = 10}}.body
                assert(#pc.doc.rebuild() == 0)
                local side, round
                for _, f in ipairs(pc.doc.faces{body = base}) do
                  if f.normal and f.normal[1] > 0.99 then side = f end
                end
                for _, f in ipairs(pc.doc.faces{body = roller}) do
                  if f.axis then round = f end
                end
                pc.asm.tangent{body = roller, face = round, other = base, other_face = side}
                local at = pc.asm.placement{body = roller}.translation
                assert(math.abs(80 + at[1] - 43) < 1e-3, "its axis 3 mm out from x = 40")
                "#,
            ),
        "asm.ball" => spec
            .note(
                "It takes points: a ball's `{centre}`, else `{point}`; a flat face given \
                 whole is taken at its listed point.",
            )
            .note("The two points meet and the body may turn every way, three motions left.")
            .see_also("asm.universal")
            .see_also("asm.distance")
            .example(
                "An arm's corner on a base's corner",
                r#"
                local function box(x, w, h, len)
                  local s = pc.sketch.new{plane = "XY"}
                  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
                  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
                end
                local base = box(0, 40, 40, 5)
                local arm = box(60, 30, 4, 4)
                assert(#pc.doc.rebuild() == 0)
                pc.asm.ball{body = arm, face = {point = {60, 0, 4}},
                  other = base, other_face = {point = {40, 40, 5}}}
                local at = pc.asm.placement{body = arm}.translation
                assert(math.abs(at[1] + 20) < 1e-4 and math.abs(at[2] - 40) < 1e-4
                  and math.abs(at[3] - 1) < 1e-4, "the arm's corner on the base's corner")
                assert(pc.asm.freedom{body = arm}[1].free == 3, "it turns every way")
                "#,
            ),
        "asm.universal" => spec
            .note(
                "Each body gives a pin, `{axis = {point, direction}}`. The two axes' points \
                 meet and the body keeps two turns, one about each pin.",
            )
            .see_also("asm.ball")
            .see_also("asm.hinge")
            .example(
                "Two pins crossed at one point",
                r#"
                local function box(x, w, h, len)
                  local s = pc.sketch.new{plane = "XY"}
                  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
                  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
                end
                local base = box(0, 40, 40, 5)
                local arm = box(60, 30, 4, 4)
                assert(#pc.doc.rebuild() == 0)
                local along_x = {axis = {point = {60, 2, 2}, direction = {1, 0, 0}}}
                local up_z = {axis = {point = {20, 20, 10}, direction = {0, 0, 1}}}
                pc.asm.universal{body = arm, face = along_x, other = base, other_face = up_z}
                local free = pc.asm.freedom{body = arm}[1]
                assert(free.free == 2 and free.motions[1].turn and free.motions[2].turn)
                local at = pc.asm.placement{body = arm}.translation
                assert(math.abs(at[1] + 40) < 1e-3 and math.abs(at[3] - 8) < 1e-3,
                  "the two pins cross at (20, 20, 10)")
                "#,
            ),
        "asm.slot" => spec
            .note(
                "`face` is the pin, a point (`{centre}` or `{point}`) on the moving body; \
                 `other_face` the slot, a line `{axis = {point, direction}}` on the other.",
            )
            .note("The pin stays on the line: the body keeps three turns and the slide along it.")
            .see_also("asm.path")
            .see_also("asm.slider")
            .example(
                "A pin kept on a line",
                r#"
                local function box(x, w, h, len)
                  local s = pc.sketch.new{plane = "XY"}
                  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
                  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
                end
                local base = box(0, 40, 40, 5)
                local arm = box(60, 30, 4, 4)
                assert(#pc.doc.rebuild() == 0)
                local edge = {axis = {point = {0, 40, 5}, direction = {1, 0, 0}}}
                local pin = {point = {60, 0, 0}}
                pc.asm.slot{body = arm, face = pin, other = base, other_face = edge}
                local at = pc.asm.placement{body = arm}.translation
                assert(math.abs(at[2] - 40) < 1e-4 and math.abs(at[3] - 5) < 1e-4,
                  "the pin on the base's back top edge")
                local free = pc.asm.freedom{body = arm}[1]
                assert(free.free == 4 and free.motions[4].slide[1] == 1,
                  "three turns and a slide along X")
                "#,
            ),
        "asm.path" => spec
            .note(
                "`other_face` is only a `{point}` near the edge: the other body's edge \
                 nearest it is taken, however far, and kept with the joint.",
            )
            .note(
                "The moving point goes onto that edge where the edge is nearest to it, not \
                 to the point given.",
            )
            .see_also("asm.slot")
            .see_also("asm.cam")
            .example(
                "A corner run along a disc's rim",
                r#"
                local d = pc.sketch.new{plane = "XY"}
                pc.sketch.circle{sketch = d, x = 0, y = 0, radius = 20}
                local disc = pc.doc.feature{id = pc.design.pad{sketch = d, length = 5}}.body
                local r = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = r, x = 60, y = 0, width = 4, height = 4}
                local rider = pc.doc.feature{id = pc.design.pad{sketch = r, length = 4}}.body
                assert(#pc.doc.rebuild() == 0)
                pc.asm.path{body = rider, face = {point = {60, 0, 0}},
                  other = disc, other_face = {point = {0, 20, 5}}}
                local at = pc.asm.placement{body = rider}.translation
                local x, y, z = 60 + at[1], at[2], at[3]
                assert(math.abs(math.sqrt(x * x + y * y) - 20) < 0.01 and math.abs(z - 5) < 1e-3,
                  "the rider's corner on the disc's top rim")
                "#,
            ),
        "asm.cam" => spec
            .note(
                "`other_face` is a `{point}` near the cam's face: the other body's face \
                 nearest it is taken.",
            )
            .note(
                "`radius` is the roller's, mm: the follower's point keeps that far off the \
                 face; 0 when left out, a point follower.",
            )
            .see_also("asm.path")
            .see_also("asm.tangent")
            .example(
                "A follower 2 mm off a round cam",
                r#"
                local d = pc.sketch.new{plane = "XY"}
                pc.sketch.circle{sketch = d, x = 0, y = 0, radius = 20}
                local cam = pc.doc.feature{id = pc.design.pad{sketch = d, length = 5}}.body
                local r = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = r, x = 60, y = 0, width = 4, height = 4}
                local follower = pc.doc.feature{id = pc.design.pad{sketch = r, length = 4}}.body
                assert(#pc.doc.rebuild() == 0)
                pc.asm.cam{body = follower, face = {point = {60, 0, 0}},
                  other = cam, other_face = {point = {20, 0, 2}}, radius = 2}
                local p = pc.asm.placement{body = follower}
                local q, t = p.rotation, p.translation
                -- Where the follower's point (60, 0, 0) is now: turned by q, then moved by t.
                local x = 60 * (1 - 2 * (q[2] ^ 2 + q[3] ^ 2)) + t[1]
                local y = 60 * 2 * (q[1] * q[2] + q[3] * q[4]) + t[2]
                assert(math.abs(math.sqrt(x * x + y * y) - 22) < 0.1,
                  "2 mm off the cam's 20 mm face")
                "#,
            ),
        "asm.width" => spec
            .note(
                "It takes four flat faces: `face` and `face2`, the tab's two sides, and \
                 `other_face` and `other_face2`, the slot's two walls.",
            )
            .note(
                "The tab is centred between the walls and keeps three motions: two slides \
                 along the walls and a turn about their normal.",
            )
            .see_also("asm.mate")
            .see_also("asm.distance")
            .example(
                "A tab centred between two walls",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 5, height = 20}
                pc.sketch.rect{sketch = s, x = 15, y = 0, width = 5, height = 20}
                local walls = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
                local t = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = t, x = 40, y = 0, width = 6, height = 10}
                local tab = pc.doc.feature{id = pc.design.pad{sketch = t, length = 10}}.body
                assert(#pc.doc.rebuild() == 0)
                local function side(body, sign, x)
                  for _, f in ipairs(pc.doc.faces{body = body}) do
                    local n = f.normal
                    if n and n[1] * sign > 0.99 and math.abs(f.point[1] - x) < 1e-3 then
                      return f
                    end
                  end
                end
                pc.asm.width{body = tab, face = side(tab, -1, 40), face2 = side(tab, 1, 46),
                  other = walls, other_face = side(walls, 1, 5), other_face2 = side(walls, -1, 15)}
                local at = pc.asm.placement{body = tab}.translation
                assert(math.abs(at[1] + 33) < 1e-3, "the 6 mm tab centred in the 10 mm gap")
                "#,
            ),
        _ => spec,
    }
}

pub fn run(id: &str, args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    match id {
        id if JointTool::of_command(id).is_some() => make_joint(id, &a, ctx),
        "asm.couple" => couple(&a, ctx),
        "asm.set"
            if ctx
                .document
                .get_feature_meta(FeatureId(a.id("joint")?))
                .is_some_and(|n| n.workbench_id.as_str() == COUPLING_KIND) =>
        {
            set_coupling(&a, ctx)
        }
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
                if !(other == crate::WORLD || ctx.document.bodies().iter().any(|b| b.id == other))
                    || other == moving_body
                {
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
            let data =
                serde_json::to_value(&feature).map_err(|e| CommandError::failed(e.to_string()))?;
            ctx.document
                .update_feature_data(joint, data)
                .map_err(|e| CommandError::failed(e.to_string()))?;
            ctx.document.clear_feature_dirty(joint);
            solved(ctx, Value::Null)
        }
        "asm.mass" => {
            let kernel = ctx
                .kernel
                .ok_or_else(|| CommandError::failed("no kernel to measure with"))?;
            let among = body_list(&a)?;
            let density = a
                .opt_number("density")?
                .unwrap_or(f64::from(DEFAULT_DENSITY));
            let report = crate::mass::plan(ctx.document, among.as_deref())
                .run(
                    kernel,
                    &std::sync::atomic::AtomicUsize::new(0),
                    &std::sync::atomic::AtomicBool::new(false),
                )
                .map_err(CommandError::failed)?;
            let bodies: Vec<Value> = report
                .bodies
                .iter()
                .map(|b| {
                    json!({
                        "body": b.body.0.to_string(),
                        "mass": b.mass_g(density),
                        "volume": b.volume_mm3,
                        "centre": b.centre,
                    })
                })
                .collect();
            Ok(json!({
                "mass": report.mass_g(density),
                "volume": report.volume_mm3(),
                "centre": report.centre(density),
                "bodies": bodies,
                "skipped": report.skipped,
            }))
        }
        "asm.interference" => {
            let kernel = ctx
                .kernel
                .ok_or_else(|| CommandError::failed("no kernel to check with"))?;
            let among = body_list(&a)?;
            if let Some(gap) = a.opt_number("clearance")? {
                let found =
                    crate::interference::plan_clearance(ctx.document, among.as_deref(), gap)
                        .run(
                            kernel,
                            &std::sync::atomic::AtomicUsize::new(0),
                            &std::sync::atomic::AtomicBool::new(false),
                        )
                        .map_err(CommandError::failed)?;
                let near: Vec<Value> = found
                    .near
                    .iter()
                    .map(|n| {
                        json!({
                            "a": n.a.0.to_string(),
                            "b": n.b.0.to_string(),
                            "distance": n.distance_mm,
                            "on_a": n.on_a,
                            "on_b": n.on_b,
                        })
                    })
                    .collect();
                return Ok(json!({
                    "checked": found.checked,
                    "skipped": found.skipped,
                    "near": near,
                    "unchecked": unchecked_json(&found.unchecked),
                }));
            }
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
                "unchecked": unchecked_json(&found.unchecked),
            }))
        }
        "asm.parts" => {
            let mut parts = crate::parts_list(ctx.document, &ctx.bought_kinds);
            let mut volumes = crate::parts::Volumes::default();
            volumes.measure_now(ctx.document, &parts, ctx.kernel);
            volumes.fill(ctx.document, &mut parts);
            let entry = |part: &crate::Part, bodies: &[BodyId]| {
                json!({
                    "print": part.print,
                    "volume": part.volume_mm3,
                    "mass": part.mass_g(),
                    "density": part.density,
                    "name": part.name,
                    "quantity": bodies.len(),
                    "bodies": bodies.iter().map(|b| b.0.to_string()).collect::<Vec<_>>(),
                    "size": part.size_mm,
                    "mesh": part.mesh,
                    "number": part.number,
                    "bought": part.bought,
                    "values": part.values,
                })
            };
            if a.opt_bool("by_component")? != Some(true) {
                return Ok(Value::Array(
                    parts.iter().map(|p| entry(p, &p.bodies)).collect(),
                ));
            }
            Ok(Value::Array(
                crate::parts::parts_by_component(ctx.document, &parts)
                    .into_iter()
                    .map(|row| match row {
                        crate::parts::LevelRow::Component { depth, id, name } => {
                            json!({"component": id.0.to_string(), "name": name, "depth": depth})
                        }
                        crate::parts::LevelRow::Part {
                            depth,
                            part,
                            bodies,
                        } => {
                            let mut value = entry(&parts[part], &bodies);
                            value["depth"] = json!(depth);
                            value
                        }
                    })
                    .collect(),
            ))
        }
        "asm.print_material" => {
            let name = a.string("name")?.to_string();
            let density = a.opt_number("density")?;
            if density.is_some_and(|d| d <= 0.0) {
                return Err(CommandError::bad("density", "must be above 0"));
            }
            let known = crate::parts::PRINT_MATERIALS
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case(&name));
            if !known && density.is_none() {
                return Err(CommandError::bad(
                    "density",
                    format!("{name} is not a material offered: give its density"),
                ));
            }
            let mut table = crate::parts::table_of(ctx.document)
                .map(|(_, t)| t)
                .unwrap_or_default();
            table.material = crate::parts::PrintMaterial::named(&name, density.map(|d| d as f32));
            crate::parts::store_table(ctx.document, &table)
                .map_err(|e| CommandError::failed(e.to_string()))?;
            Ok(json!({"name": table.material.name, "density": table.material.density}))
        }
        "asm.parts_table" => {
            let table: crate::parts::PartsTable =
                serde_json::from_value(a.0.get("table").cloned().unwrap_or(Value::Null))
                    .map_err(|e| CommandError::bad("table", e.to_string()))?;
            crate::parts::store_table(ctx.document, &table)
                .map_err(|e| CommandError::failed(e.to_string()))?;
            Ok(Value::Null)
        }
        "asm.part" => {
            let body = body(&a, ctx)?;
            let part = crate::parts_list(ctx.document, &ctx.bought_kinds)
                .into_iter()
                .find(|p| p.bodies.contains(&body));
            let bodies = part
                .as_ref()
                .map_or_else(|| vec![body], |p| p.bodies.clone());
            let mut table = crate::parts::table_of(ctx.document)
                .map(|(_, t)| t)
                .unwrap_or_default();
            if let Some(bought) = a.opt_bool("bought")? {
                match &part {
                    Some(part) => table.set_bought(part, bought),
                    None => table.entry_mut(&bodies).bought = bought,
                }
            }
            let entry = table.entry_mut(&bodies);
            if let Some(n) = a.opt_number("number")? {
                entry.number = n.max(0.0) as u32;
            }
            if let Some(n) = a.opt_number("print")? {
                entry.print = (n >= 0.0).then_some(n.round() as u32);
            }
            if let Some(values) = a.0.get("values").and_then(Value::as_object) {
                for (column, value) in values {
                    let text = value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_string);
                    entry.values.insert(column.clone(), text);
                }
                for column in values.keys() {
                    if !table.columns.contains(column) {
                        table.columns.push(column.clone());
                    }
                }
            }
            crate::parts::store_table(ctx.document, &table)
                .map_err(|e| CommandError::failed(e.to_string()))?;
            Ok(Value::Null)
        }
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
            crate::components::move_with_unit(
                ctx.document,
                body,
                BodyPlacement::new(rotation, translation),
            );
            Ok(Value::Null)
        }
        "asm.copy" => {
            let source = body(&a, ctx)?;
            let count = a.opt_number("count")?.unwrap_or(1.0).clamp(1.0, 500.0) as usize;
            let step = match a.0.get("step") {
                Some(v) if !v.is_null() => Some(vector(v, "step")?),
                _ => None,
            };
            let made = match a.0.get("around").filter(|v| !v.is_null()) {
                Some(around) => {
                    let point = vector(around.get("point").unwrap_or(&json!([0, 0, 0])), "around")?;
                    let direction =
                        vector(around.get("direction").unwrap_or(&Value::Null), "around")?;
                    if direction.length() < 1e-9 {
                        return Err(CommandError::bad("around", "needs a direction"));
                    }
                    let angle = around.get("angle").and_then(Value::as_f64).unwrap_or(360.0) as f32;
                    insert_copies_around(ctx, source, count, (point, direction.normalize(), angle))
                }
                None => insert_copies(ctx, source, count, step),
            }
            .ok_or_else(|| CommandError::bad("body", "is not a body of this document"))?;
            Ok(Value::from(
                made.iter().map(|b| b.0.to_string()).collect::<Vec<_>>(),
            ))
        }
        "asm.replace" => {
            let old = body(&a, ctx)?;
            let new = BodyId(a.id("with")?);
            let report =
                crate::replace::replace(ctx.document, old, new).map_err(CommandError::failed)?;
            solved(
                ctx,
                json!({"kept": report.kept, "unmatched": report.unmatched}),
            )
        }
        "asm.mirror" => {
            let source = body(&a, ctx)?;
            let point = vector(a.0.get("point").unwrap_or(&Value::Null), "point")?;
            let normal = vector(a.0.get("normal").unwrap_or(&Value::Null), "normal")?;
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
                .ok_or_else(|| {
                    CommandError::bad("body", "cannot be mirrored: a mirror of a mirror")
                })?;
            Ok(json!(copy.0.to_string()))
        }
        "asm.group" => {
            let bodies = body_list(&a)?.unwrap_or_default();
            if bodies.len() < 2 {
                return Err(CommandError::bad("bodies", "must name two bodies or more"));
            }
            if let Some(b) = bodies
                .iter()
                .find(|b| !ctx.document.bodies().iter().any(|x| x.id == **b))
            {
                return Err(CommandError::bad(
                    "bodies",
                    format!("{} is not a body of this document", b.0),
                ));
            }
            let group = crate::RigidGroup::of(ctx.document, &bodies);
            let id = match a.opt_id("group")? {
                Some(id) => {
                    let id = FeatureId(id);
                    ctx.document
                        .update_feature_data(id, core_document::WorkbenchFeature::to_json(&group))
                        .map_err(|e| CommandError::failed(e.to_string()))?;
                    id
                }
                None => {
                    let name = match a.opt_string("name")? {
                        Some(n) => n.to_string(),
                        None => next_name(ctx.document, "Group"),
                    };
                    ctx.document
                        .add_feature_in_body(group, name, Some(bodies[0]))
                        .map_err(|e| CommandError::failed(e.to_string()))?
                }
            };
            ctx.document.clear_feature_dirty(id);
            solved(ctx, json!(id.0.to_string()))
        }
        "asm.component" => {
            let bodies = existing_bodies(&a, ctx)?;
            let parent = a.opt_id("parent")?.map(ComponentId);
            let name = match a.opt_string("name")? {
                Some(n) => n.to_string(),
                None => next_component_name(ctx.document),
            };
            let id = ctx
                .document
                .create_component(name, parent)
                .map_err(|e| CommandError::bad("parent", e.to_string()))?;
            if a.opt_bool("flexible")? == Some(true)
                && let Some(mut component) = ctx.document.component(id).cloned()
            {
                component.flexible = true;
                ctx.document
                    .update_component(component)
                    .map_err(|e| CommandError::failed(e.to_string()))?;
            }
            for body in bodies {
                ctx.document
                    .set_body_component(body, Some(id))
                    .map_err(|e| CommandError::failed(e.to_string()))?;
            }
            solved(ctx, json!(id.0.to_string()))
        }
        "asm.component_set" => {
            let id = ComponentId(a.id("component")?);
            let mut component = ctx.document.component(id).cloned().ok_or_else(|| {
                CommandError::bad("component", "is not a component of this document")
            })?;
            if let Some(name) = a.opt_string("name")? {
                component.name = name.to_string();
            }
            if let Some(flexible) = a.opt_bool("flexible")? {
                component.flexible = flexible;
            }
            match a.0.get("parent") {
                None => {}
                Some(Value::Null) => component.parent = None,
                Some(Value::String(p)) if p == "top" => component.parent = None,
                Some(Value::String(p)) => {
                    let parent = uuid::Uuid::parse_str(p)
                        .map_err(|_| CommandError::bad("parent", "is not an id"))?;
                    component.parent = Some(ComponentId(parent));
                }
                Some(_) => return Err(CommandError::bad("parent", "is an id or \"top\"")),
            }
            ctx.document
                .update_component(component)
                .map_err(|e| CommandError::bad("parent", e.to_string()))?;
            solved(ctx, Value::Null)
        }
        "asm.component_add" => {
            let bodies = existing_bodies(&a, ctx)?;
            let component = a.opt_id("component")?.map(ComponentId);
            for body in bodies {
                ctx.document
                    .set_body_component(body, component)
                    .map_err(|e| CommandError::bad("component", e.to_string()))?;
            }
            solved(ctx, Value::Null)
        }
        "asm.component_remove" => {
            let id = ComponentId(a.id("component")?);
            ctx.document
                .remove_component(id)
                .map_err(|e| CommandError::bad("component", e.to_string()))?;
            solved(ctx, Value::Null)
        }
        "asm.motion" => {
            let defaults = crate::MotionStudy::default();
            let study = crate::MotionStudy {
                start: a.opt_number("start")?.map_or(defaults.start, |v| v as f32),
                end: a.opt_number("end")?.map_or(defaults.end, |v| v as f32),
                step: a.opt_number("step")?.map_or(defaults.step, |v| v as f32),
                drives: serde_json::from_value(a.0.get("drives").cloned().unwrap_or(Value::Null))
                    .map_err(|e| CommandError::bad("drives", e.to_string()))?,
            };
            for drive in &study.drives {
                let want = crate::motion::drive_dim(ctx.document, drive.joint)
                    .map_err(|e| CommandError::bad("drives", e))?;
                crate::motion::value_at(&drive.formula, 0.0, want)
                    .map_err(|e| CommandError::bad("drives", e))?;
            }
            let data = core_document::WorkbenchFeature::to_json(&study);
            let id = match a.opt_id("study")? {
                Some(id) => {
                    let id = FeatureId(id);
                    ctx.document
                        .update_feature_data(id, data)
                        .map_err(|e| CommandError::failed(e.to_string()))?;
                    id
                }
                None => {
                    let name = match a.opt_string("name")? {
                        Some(n) => n.to_string(),
                        None => next_name(ctx.document, "Motion"),
                    };
                    ctx.document
                        .add_feature_in_body(study, name, None)
                        .map_err(|e| CommandError::failed(e.to_string()))?
                }
            };
            ctx.document.clear_feature_dirty(id);
            Ok(json!(id.0.to_string()))
        }
        "asm.motion_frames" => {
            let study = motion_study(&a, ctx.document)?;
            let frames = study.frames(ctx.document).map_err(CommandError::failed)?;
            Ok(Value::Array(
                frames
                    .into_iter()
                    .map(|(t, bodies)| {
                        let bodies: Vec<Value> = bodies
                            .into_iter()
                            .map(|(body, p)| {
                                json!({
                                    "body": body.0.to_string(),
                                    "translation": p.translation,
                                    "rotation": p.rotation,
                                })
                            })
                            .collect();
                        json!({"t": t, "bodies": bodies})
                    })
                    .collect(),
            ))
        }
        "asm.trace" => {
            let study = motion_study(&a, ctx.document)?;
            let body = body(&a, ctx)?;
            let point = vector(a.0.get("point").unwrap_or(&Value::Null), "point")?;
            let frames = study.frames(ctx.document).map_err(CommandError::failed)?;
            Ok(Value::Array(
                crate::motion::trace(&frames, body, point.to_array())
                    .into_iter()
                    .map(|(t, p, v)| json!({"t": t, "point": p, "speed": v}))
                    .collect(),
            ))
        }
        "asm.exploded_view" => {
            let steps: Vec<crate::ExplodeStep> =
                serde_json::from_value(a.0.get("steps").cloned().unwrap_or(Value::Null))
                    .map_err(|e| CommandError::bad("steps", e.to_string()))?;
            let view = crate::ExplodedView { steps };
            let data = core_document::WorkbenchFeature::to_json(&view);
            let id = match a.opt_id("view")? {
                Some(id) => {
                    let id = FeatureId(id);
                    ctx.document
                        .update_feature_data(id, data)
                        .map_err(|e| CommandError::failed(e.to_string()))?;
                    id
                }
                None => {
                    let name = match a.opt_string("name")? {
                        Some(n) => n.to_string(),
                        None => next_name(ctx.document, "Exploded view"),
                    };
                    ctx.document
                        .add_feature_in_body(view, name, None)
                        .map_err(|e| CommandError::failed(e.to_string()))?
                }
            };
            ctx.document.clear_feature_dirty(id);
            Ok(json!(id.0.to_string()))
        }
        "asm.explode_at" => {
            let id = FeatureId(a.id("view")?);
            let view = crate::exploded::view_of(ctx.document, id)
                .ok_or_else(|| CommandError::bad("view", "is not an exploded view"))?;
            let start: Vec<(BodyId, BodyPlacement)> = ctx
                .document
                .bodies()
                .iter()
                .map(|b| (b.id, b.placement))
                .collect();
            Ok(Value::Array(
                view.placed_at(&start, a.number("at")? as f32)
                    .into_iter()
                    .map(|(body, p)| {
                        json!({
                            "body": body.0.to_string(),
                            "translation": p.translation,
                            "rotation": p.rotation,
                        })
                    })
                    .collect(),
            ))
        }
        "asm.save_state" => {
            if let Some(state) = a.opt_id("state")? {
                let state = FeatureId(state);
                let now = crate::states::capture(ctx.document);
                ctx.document
                    .update_feature_data(state, core_document::WorkbenchFeature::to_json(&now))
                    .map_err(|e| CommandError::failed(e.to_string()))?;
                ctx.document.clear_feature_dirty(state);
                return Ok(json!(state.0.to_string()));
            }
            let name = match a.opt_string("name")? {
                Some(n) => n.to_string(),
                None => next_name(ctx.document, "State"),
            };
            let id = crate::states::save(ctx.document, name)
                .map_err(|e| CommandError::failed(e.to_string()))?;
            Ok(json!(id.0.to_string()))
        }
        "asm.restore_state" => {
            let id = FeatureId(a.id("state")?);
            let state = ctx
                .document
                .get_feature_data(id)
                .and_then(|d| {
                    <crate::AssemblyState as core_document::WorkbenchFeature>::from_json(d).ok()
                })
                .ok_or_else(|| CommandError::bad("state", "is not a saved assembly state"))?;
            crate::states::restore(ctx.document, &state);
            solved(ctx, Value::Null)
        }
        "asm.redundant" => Ok(Value::Array(
            crate::redundant(ctx.document)
                .into_iter()
                .map(|(id, name)| json!({"joint": id.0.to_string(), "name": name}))
                .collect(),
        )),
        "asm.motion_clashes" => {
            let kernel = ctx
                .kernel
                .ok_or_else(|| CommandError::failed("no kernel to check with"))?;
            let joint = FeatureId(a.id("joint")?);
            if !joints(ctx.document).iter().any(|j| {
                j.id == joint
                    && matches!(
                        j.feature.kind,
                        JointKind::Hinge { .. } | JointKind::Slider { .. }
                    )
            }) {
                return Err(CommandError::bad("joint", "is not a hinge or a slider"));
            }
            let steps = a.opt_number("steps")?.unwrap_or(24.0).clamp(2.0, 1000.0) as usize;
            let (low, high) = (a.number("low")? as f32, a.number("high")? as f32);
            let Some(check) = crate::sweep_check::plan(ctx.document, joint, low, high, steps)
            else {
                return Ok(json!([]));
            };
            let found = check
                .run(
                    kernel,
                    &std::sync::atomic::AtomicUsize::new(0),
                    &std::sync::atomic::AtomicBool::new(false),
                )
                .map_err(CommandError::failed)?;
            Ok(Value::Array(
                found
                    .iter()
                    .map(|c| {
                        json!({
                            "at": c.at,
                            "a": c.a.0.to_string(),
                            "b": c.b.0.to_string(),
                            "volume": c.volume_mm3,
                        })
                    })
                    .collect(),
            ))
        }
        "asm.turn" | "asm.flip" => {
            let joint = FeatureId(a.id("joint")?);
            let found = joints(ctx.document)
                .into_iter()
                .find(|j| j.id == joint && j.feature.kind != JointKind::Ground)
                .ok_or_else(|| CommandError::bad("joint", "is not a joint between two bodies"))?;
            let degrees = if id == "asm.turn" {
                a.number("degrees")?
            } else {
                0.0
            };
            if id == "asm.turn" && matches!(found.feature.kind, JointKind::Hinge { .. }) {
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
            turn_joint(ctx, &found, degrees, id == "asm.flip")?;
            solved(ctx, Value::Null)
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
            crate::components::move_with_unit(ctx.document, body, step.after(&now));
            Ok(Value::Null)
        }
        _ => Err(CommandError::Unknown(id.to_string())),
    }
}

fn make_joint(id: &str, a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let moving_body = body(a, ctx)?;
    let other = BodyId(a.id("other")?);
    if other != crate::WORLD && !ctx.document.bodies().iter().any(|b| b.id == other) {
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
        None => {
            let number = joints(ctx.document)
                .iter()
                .filter(|j| j.feature.kind.label() == label)
                .count()
                + 1;
            format!("{label} {number}")
        }
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
                .bodies()
                .iter()
                .find(|b| b.id == body)
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
        JointKind::Align { turn, slide, .. } => json!({
            "turn_drive": turn.to.map_or(json!(false), |v| json!(v)),
            "turn_limits": turn.limits.map_or(json!(false), |l| json!(l)),
            "slide_drive": slide.to.map_or(json!(false), |v| json!(v)),
            "slide_limits": slide.limits.map_or(json!(false), |l| json!(l)),
        }),
        JointKind::Ground
        | JointKind::Fixed { .. }
        | JointKind::Parallel
        | JointKind::Perpendicular
        | JointKind::Ball
        | JointKind::Universal
        | JointKind::Slot
        | JointKind::Path
        | JointKind::Width => json!({}),
        JointKind::Cam { radius } => json!({"radius": radius}),
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
            let face = |anchor: &Anchor, b: BodyId, name: u64| {
                let mut face = match anchor.moved(&at(b)) {
                    Anchor::Plane { point, normal } => json!({"point": point, "normal": normal}),
                    Anchor::Axis { point, direction } => {
                        json!({"axis": {"point": point, "direction": direction}})
                    }
                    Anchor::Point { point } => json!({"point": point}),
                };
                if name != 0 {
                    face["name"] = json!(name);
                }
                face
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
                "face": face(&joint.moving, body, joint.names[0]),
                "other": joint.other_body.0.to_string(),
                "other_face": face(&joint.fixed, joint.other_body, joint.names[1]),
                "name": node.name,
            }));
            args.extend(object(settings(&joint.kind)));
            if let Some([tab, wall]) = joint.second {
                args.insert("face2".into(), face(&tab, body, 0));
                args.insert("other_face2".into(), face(&wall, joint.other_body, 0));
            }
            ctx.record(command, args, json!(id.0.to_string()));
            if joint.ends != [0.0, 0.0] {
                ctx.record(
                    "asm.set",
                    object(json!({
                        "joint": id.0.to_string(),
                        "moving_end": joint.ends[0],
                        "fixed_end": joint.ends[1],
                    })),
                    Value::Null,
                );
            }
        }
        Some(before) => {
            let Ok(old) = serde_json::from_value::<JointFeature>(before.clone()) else {
                return;
            };
            let with_ends = |j: &JointFeature| {
                let mut all = object(settings(&j.kind));
                all.insert("moving_end".into(), json!(j.ends[0]));
                all.insert("fixed_end".into(), json!(j.ends[1]));
                all
            };
            let (was, now) = (with_ends(&old), with_ends(&joint));
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

/// Every body's placement, in double precision.
fn placements(document: &core_document::Document) -> std::collections::HashMap<BodyId, Rigid> {
    crate::solve::rigid_placements(document)
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

fn couple(a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
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

/// `label` numbered past every coupling already named so.
pub(crate) fn next_name(document: &core_document::Document, label: &str) -> String {
    let taken = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == COUPLING_KIND && n.name.starts_with(label))
        .count();
    format!("{label} {}", taken + 1)
}

/// A coupling's settings, as `asm.couple` and `asm.set` take them.
fn coupling_settings(c: &Coupling) -> CommandArgs {
    object(json!({
        "driver": c.driver.0.to_string(),
        "driven": c.driven.0.to_string(),
        "gearing": c.gearing.word(),
        "ratio": c.ratio,
        "reverse": c.reverse,
    }))
}

/// A coupling's task accepted, as a recording says it: a new one as the
/// command that makes it, an edit as the settings it changed.
pub(crate) fn record_coupling(
    ctx: &mut WorkbenchRuntimeContext,
    id: FeatureId,
    before: Option<&Value>,
) {
    let Some(node) = ctx.document.get_feature_meta(id).cloned() else {
        return;
    };
    let Ok(coupling) = serde_json::from_value::<Coupling>(node.data.clone()) else {
        return;
    };
    let now = coupling_settings(&coupling);
    match before.and_then(|b| serde_json::from_value::<Coupling>(b.clone()).ok()) {
        None => {
            let mut args = now;
            args.insert("name".into(), json!(node.name));
            ctx.record("asm.couple", args, json!(id.0.to_string()));
        }
        Some(old) => {
            let was = coupling_settings(&old);
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

/// The `bodies` a command is limited to, when it names any.
fn body_list(a: &Args) -> Result<Option<Vec<BodyId>>, CommandError> {
    let bad = || CommandError::bad("bodies", "must be a list of ids");
    match a.0.get("bodies") {
        None | Some(Value::Null) => Ok(None),
        Some(list) => list
            .as_array()
            .ok_or_else(bad)?
            .iter()
            .map(|v| {
                v.as_str()
                    .and_then(|t| uuid::Uuid::parse_str(t).ok())
                    .map(BodyId)
                    .ok_or_else(bad)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
    }
}

/// A JSON object as named arguments.
pub(crate) fn object(value: Value) -> CommandArgs {
    match value {
        Value::Object(map) => map,
        _ => CommandArgs::new(),
    }
}

/// The `bodies` named, every one a body of the document.
fn existing_bodies(a: &Args, ctx: &WorkbenchRuntimeContext) -> Result<Vec<BodyId>, CommandError> {
    let bodies = body_list(a)?.unwrap_or_default();
    if let Some(b) = bodies
        .iter()
        .find(|b| !ctx.document.bodies().iter().any(|x| x.id == **b))
    {
        return Err(CommandError::bad(
            "bodies",
            format!("{} is not a body of this document", b.0),
        ));
    }
    Ok(bodies)
}

/// "Component 1", or the next number not taken.
pub(crate) fn next_component_name(document: &core_document::Document) -> String {
    (1..)
        .map(|n| format!("Component {n}"))
        .find(|name| !document.components().iter().any(|c| &c.name == name))
        .unwrap_or_default()
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

/// The density the mass tool starts at, g/cm³.
pub(crate) const DEFAULT_DENSITY: f32 = 1.0;

/// The first joint of an assembly grounds the body it holds against (the
/// world needs none), when nothing is grounded yet: the assembly then
/// stands on it, and what its joints leave free reads true from the start.
pub(crate) fn ground_first(ctx: &mut WorkbenchRuntimeContext, other: BodyId) -> Option<FeatureId> {
    let all = crate::joints(ctx.document);
    if all.is_empty() && other != crate::WORLD {
        return set_grounded(ctx, other, true);
    }
    None
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
                second: None,
                shape: Vec::new(),
                ends: [0.0; 2],
                names: [0; 2],
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

/// The pairs an interference check could not compare, as `{a, b, error}`.
fn unchecked_json(unchecked: &[crate::interference::Unchecked]) -> Vec<Value> {
    unchecked
        .iter()
        .map(|u| json!({"a": u.a.0.to_string(), "b": u.b.0.to_string(), "error": u.why}))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::Document;

    /// Run a command as a script does: its arguments checked against its
    /// registered spec first.
    fn call(doc: &mut Document, id: &str, args: Value) -> CommandResult {
        let args = args.as_object().unwrap();
        let mut registered = core_document::WorkbenchContext::default();
        register(&mut registered);
        let spec = registered
            .commands()
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("`{id}` is registered"));
        spec.check(args)?;
        let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        run(id, args, &mut ctx)
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

    /// Turning a joint's body keeps it there: a driven hinge keeps its
    /// angle with the body turned further, a slider turned over runs the
    /// other way round, a mate turned over faces the same way.
    #[test]
    fn a_joint_s_body_turns_and_turns_over_and_stays() {
        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        let pin = json!({"axis": {"point": [0, 0, 0], "direction": [0, 0, 1]}});
        let hinge = call(
            &mut doc,
            "asm.hinge",
            json!({"body": a.0.to_string(), "face": pin, "other": b.0.to_string(), "other_face": pin,
                   "drive": 30}),
        )
        .unwrap();
        call(&mut doc, "asm.turn", json!({"joint": hinge, "degrees": 20})).unwrap();
        let x = doc.body_placement(a).direction([1.0, 0.0, 0.0]);
        assert!((x[1].atan2(x[0]).to_degrees() - 50.0).abs() < 1e-2, "{x:?}");
        // The drive moved with it: the hinge reads where the body is.
        let travel = call(&mut doc, "asm.travel", json!({"joint": hinge})).unwrap();
        assert!((travel.as_f64().unwrap() - 50.0).abs() < 1e-2, "{travel}");
        call(&mut doc, "asm.solve", json!({})).unwrap();
        let x = doc.body_placement(a).direction([1.0, 0.0, 0.0]);
        assert!(
            (x[1].atan2(x[0]).to_degrees() - 50.0).abs() < 1e-2,
            "stays: {x:?}"
        );

        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        let top = json!({"point": [0, 0, 0], "normal": [0, 0, 1]});
        let mate = call(
            &mut doc,
            "asm.mate",
            json!({"body": a.0.to_string(), "face": top, "other": b.0.to_string(), "other_face": top}),
        )
        .unwrap();
        let up = doc.body_placement(a).direction([0.0, 0.0, 1.0]);
        assert!(up[2] < -0.99, "faces the other: {up:?}");
        call(&mut doc, "asm.flip", json!({"joint": mate})).unwrap();
        call(&mut doc, "asm.solve", json!({})).unwrap();
        let up = doc.body_placement(a).direction([0.0, 0.0, 1.0]);
        assert!(up[2] > 0.99, "the same way now: {up:?}");
        let id = FeatureId(uuid::Uuid::parse_str(mate.as_str().unwrap()).unwrap());
        let data: JointFeature =
            serde_json::from_value(doc.get_feature_data(id).unwrap().clone()).unwrap();
        assert!(matches!(data.kind, JointKind::Mate { flip: true, .. }));
    }

    /// Turning either of two geared hinges turns both: the driver by what
    /// it is turned, or by what takes the driven one along.
    #[test]
    fn a_turn_of_either_geared_hinge_turns_both() {
        let mut doc = Document::new("t");
        let [base, g1, g2] = [
            doc.create_body(None),
            doc.create_body(None),
            doc.create_body(None),
        ];
        let pin = |x: f32| json!({"axis": {"point": [x, 0, 0], "direction": [0, 0, 1]}});
        let hinge = |doc: &mut Document, body: BodyId, x: f32| {
            call(
                doc,
                "asm.hinge",
                json!({"body": body.0.to_string(), "face": pin(x), "other": base.0.to_string(),
                       "other_face": pin(x)}),
            )
            .unwrap()
        };
        let h1 = hinge(&mut doc, g1, -20.0);
        let h2 = hinge(&mut doc, g2, 10.0);
        call(
            &mut doc,
            "asm.couple",
            json!({"driver": h1, "driven": h2, "gearing": "gears", "ratio": 2}),
        )
        .unwrap();
        let travel = |doc: &mut Document, joint: &Value| {
            call(doc, "asm.travel", json!({"joint": joint}))
                .unwrap()
                .as_f64()
                .unwrap()
        };
        call(&mut doc, "asm.turn", json!({"joint": h1, "degrees": 30})).unwrap();
        assert!((travel(&mut doc, &h1) - 30.0).abs() < 1e-2);
        assert!((travel(&mut doc, &h2) + 60.0).abs() < 1e-2);
        call(&mut doc, "asm.turn", json!({"joint": h2, "degrees": 30})).unwrap();
        assert!((travel(&mut doc, &h1) - 15.0).abs() < 1e-2);
        assert!((travel(&mut doc, &h2) + 30.0).abs() < 1e-2);
        // The drives held for the turns are taken off again.
        for joint in crate::joints(&doc) {
            if let JointKind::Hinge { drive, .. } = joint.feature.kind {
                assert_eq!(drive.to, None);
            }
        }
    }

    /// A slider turns its body onto the axis, whichever way the body's
    /// own axis pointed when it was made.
    #[test]
    fn a_slider_turns_its_body_onto_the_axis() {
        for along in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]] {
            let mut doc = Document::new("t");
            let (a, b) = (doc.create_body(None), doc.create_body(None));
            let peg = json!({"axis": {"point": [0, 0, 20], "direction": along}});
            let hole = json!({"axis": {"point": [0, 0, 0], "direction": [0, 0, 1]}});
            call(
                &mut doc,
                "asm.slider",
                json!({"body": a.0.to_string(), "face": peg, "other": b.0.to_string(),
                       "other_face": hole}),
            )
            .unwrap();
            let d = doc.body_placement(a).direction(along);
            let n = glam::Vec3::from_array(d).normalize();
            assert!(n.z.abs() > 0.999, "{along:?} now {d:?}");
        }
    }

    /// Faces that start exactly parallel are turned square.
    #[test]
    fn perpendicular_faces_start_parallel_and_end_square() {
        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        let up = json!({"point": [0, 0, 0], "normal": [0, 0, 1]});
        call(
            &mut doc,
            "asm.perpendicular",
            json!({"body": a.0.to_string(), "face": up, "other": b.0.to_string(), "other_face": up}),
        )
        .unwrap();
        let n = doc.body_placement(a).direction([0.0, 0.0, 1.0]);
        assert!(n[2].abs() < 1e-3, "square to Z: {n:?}");
    }

    /// A part set bought is bought for every body of its shape, the
    /// bench leaves them out of what is printed, and the list says so.
    #[test]
    fn a_part_is_marked_bought_and_given_a_column() {
        use core_document::Workbench;
        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(Some("Screw".into())), doc.create_body(None));
        doc.set_imported_brep_data(a, b"screw".to_vec(), Vec::new());
        doc.set_imported_brep_data(b, b"screw".to_vec(), Vec::new());
        call(
            &mut doc,
            "asm.part",
            json!({"body": b.0.to_string(), "bought": true, "number": 4,
                   "values": {"Supplier": "Fasteners Ltd"}}),
        )
        .unwrap();
        let listed = call(&mut doc, "asm.parts", json!({})).unwrap();
        assert_eq!(listed[0]["bought"], json!(true));
        assert_eq!(listed[0]["number"], json!(4));
        assert_eq!(listed[0]["values"]["Supplier"], json!("Fasteners Ltd"));
        let mut not_made = crate::AssemblyWorkbench::default().not_printed(&doc, &[]);
        not_made.sort();
        let mut both = vec![a, b];
        both.sort();
        assert_eq!(not_made, both);
    }

    /// Copies turned about an axis spread over a whole turn with the
    /// original: three about Z make four in all, a quarter turn apart.
    #[test]
    fn copies_spread_round_an_axis() {
        let mut doc = Document::new("t");
        let arm = doc.create_body(Some("Arm".into()));
        doc.set_body_placement(
            arm,
            BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(10.0, 0.0, 0.0)),
        );
        let made = call(
            &mut doc,
            "asm.copy",
            json!({"body": arm.0.to_string(), "count": 3,
                   "around": {"point": [0, 0, 0], "direction": [0, 0, 1]}}),
        )
        .unwrap();
        let at: Vec<[f32; 3]> = made
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                let id = BodyId(uuid::Uuid::parse_str(v.as_str().unwrap()).unwrap());
                doc.body_placement(id).translation
            })
            .collect();
        let want = [[0.0, 10.0], [-10.0, 0.0], [0.0, -10.0]];
        for (got, want) in at.iter().zip(want) {
            assert!(
                (got[0] - want[0]).abs() < 1e-3 && (got[1] - want[1]).abs() < 1e-3,
                "{at:?}"
            );
        }
    }

    /// A body replaced by another: the mate on the old body's underside
    /// goes to the new body's, which sits on the base; the old is hidden.
    #[test]
    fn a_replacement_takes_the_joints_on_its_own_faces() {
        use std::sync::Arc;
        let mut doc = Document::new("t");
        let [base, old, new] = [
            doc.create_body(Some("Base".into())),
            doc.create_body(Some("Old".into())),
            doc.create_body(Some("New".into())),
        ];
        let with_bottom = |z: f32| core_document::ImportedGeometry {
            mesh: Arc::new(kernel_api::TriMesh {
                positions: vec![[0.0, 0.0, z], [10.0, 0.0, z], [0.0, 10.0, z]],
                normals: vec![[0.0, 0.0, -1.0]; 3],
                indices: vec![0, 2, 1],
                faces: vec![0],
                face_surfaces: vec![kernel_api::FaceSurface::Plane {
                    origin: [0.0, 0.0, z],
                    normal: [0.0, 0.0, -1.0],
                }],
                ..kernel_api::TriMesh::default()
            }),
            source_asset: None,
            revision: 0,
            bounds_mm: None,
            brep_blob_path: None,
            mesh_path: None,
            face_colors_path: None,
            health: None,
        };
        doc.set_imported_geometry(old, with_bottom(0.0));
        doc.set_imported_geometry(new, with_bottom(2.0));
        doc.set_body_placement(
            old,
            BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(0.0, 0.0, 30.0)),
        );
        call(
            &mut doc,
            "asm.distance",
            json!({"body": old.0.to_string(), "face": {"point": [0, 0, 30], "normal": [0, 0, -1]},
                   "other": base.0.to_string(), "other_face": {"point": [0, 0, 10], "normal": [0, 0, 1]},
                   "offset": 0}),
        )
        .unwrap();
        assert!((doc.body_placement(old).translation[2] - 10.0).abs() < 1e-3);
        let report = call(
            &mut doc,
            "asm.replace",
            json!({"body": old.0.to_string(), "with": new.0.to_string()}),
        )
        .unwrap();
        assert_eq!(report["unmatched"], json!([]), "{report}");
        let joint = crate::joints(&doc)
            .into_iter()
            .find(|j| j.feature.kind != JointKind::Ground)
            .unwrap();
        assert_eq!(joint.body, new);
        // Its underside, 2 up in its own frame, on the base's top at 10.
        assert!((doc.body_placement(new).translation[2] - 8.0).abs() < 1e-3);
        assert!(doc.bodies().iter().any(|b| b.id == old && b.hidden));
    }

    /// A motion drives a hinge by a formula of time, frame by frame, and
    /// moves nothing in the document.
    #[test]
    fn a_motion_drives_its_joints_over_time() {
        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        let pin = json!({"axis": {"point": [0, 0, 0], "direction": [0, 0, 1]}});
        let hinge = call(
            &mut doc,
            "asm.hinge",
            json!({"body": a.0.to_string(), "face": pin, "other": b.0.to_string(), "other_face": pin,
                   "drive": 0}),
        )
        .unwrap();
        let study = call(
            &mut doc,
            "asm.motion",
            json!({"drives": [{"joint": hinge, "formula": "90 * t"}], "start": 0, "end": 1, "step": 0.5}),
        )
        .unwrap();
        let frames = call(&mut doc, "asm.motion_frames", json!({"study": study})).unwrap();
        let frames = frames.as_array().unwrap();
        assert_eq!(frames.len(), 3);
        let turned = |frame: &Value| {
            let row = frame["bodies"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["body"] == json!(a.0.to_string()))
                .unwrap()
                .clone();
            let q: [f32; 4] = serde_json::from_value(row["rotation"].clone()).unwrap();
            let x = glam::Quat::from_array(q) * glam::Vec3::X;
            x.y.atan2(x.x).to_degrees()
        };
        assert!((turned(&frames[1]) - 45.0).abs() < 1e-2);
        assert!((turned(&frames[2]) - 90.0).abs() < 1e-2);
        let x = doc.body_placement(a).direction([1.0, 0.0, 0.0]);
        assert!(x[1].abs() < 1e-4, "the document is not moved");
        let bad = call(
            &mut doc,
            "asm.motion",
            json!({"drives": [{"joint": hinge, "formula": "90 * "}]}),
        );
        assert!(bad.is_err());
    }

    /// An exploded view kept by a script plays its steps in order.
    #[test]
    fn an_exploded_view_is_kept_and_played() {
        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        let view = call(
            &mut doc,
            "asm.exploded_view",
            json!({"steps": [
                {"bodies": [a.0.to_string()], "shift": [0, 0, 10]},
                {"bodies": [a.0.to_string(), b.0.to_string()], "shift": [4, 0, 0]},
            ]}),
        )
        .unwrap();
        let at = call(&mut doc, "asm.explode_at", json!({"view": view, "at": 1.5})).unwrap();
        let of = |body: BodyId| {
            at.as_array()
                .unwrap()
                .iter()
                .find(|r| r["body"] == json!(body.0.to_string()))
                .unwrap()["translation"]
                .clone()
        };
        assert_eq!(of(a), json!([2.0, 0.0, 10.0]));
        assert_eq!(of(b), json!([2.0, 0.0, 0.0]));
        assert_eq!(doc.body_placement(a).translation, [0.0; 3], "nothing moved");
    }

    /// A saved state brings back a driven hinge's angle, where the bodies
    /// sat and which were hidden.
    #[test]
    fn a_saved_state_is_returned_to() {
        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        let pin = json!({"axis": {"point": [0, 0, 0], "direction": [0, 0, 1]}});
        let hinge = call(
            &mut doc,
            "asm.hinge",
            json!({"body": a.0.to_string(), "face": pin, "other": b.0.to_string(), "other_face": pin,
                   "drive": 0}),
        )
        .unwrap();
        let folded = call(&mut doc, "asm.save_state", json!({"name": "Folded"})).unwrap();
        call(&mut doc, "asm.set", json!({"joint": hinge, "drive": 90})).unwrap();
        doc.set_body_visible(b, false);
        let turned = |doc: &Document| {
            let x = doc.body_placement(a).direction([1.0, 0.0, 0.0]);
            x[1].atan2(x[0]).to_degrees()
        };
        assert!((turned(&doc) - 90.0).abs() < 1e-2);
        call(&mut doc, "asm.restore_state", json!({"state": folded})).unwrap();
        assert!(turned(&doc).abs() < 1e-2, "{}", turned(&doc));
        assert!(doc.bodies().iter().all(|x| !x.hidden));
    }

    /// A parallel joint added to a mate holds nothing new: it is reported
    /// redundant; the mate alone is not.
    #[test]
    fn a_joint_holding_nothing_new_is_redundant() {
        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        let top = json!({"point": [0, 0, 0], "normal": [0, 0, 1]});
        let under = json!({"point": [0, 0, 0], "normal": [0, 0, -1]});
        call(
            &mut doc,
            "asm.mate",
            json!({"body": a.0.to_string(), "face": under, "other": b.0.to_string(), "other_face": top}),
        )
        .unwrap();
        assert_eq!(
            call(&mut doc, "asm.redundant", json!({})).unwrap(),
            json!([])
        );
        call(
            &mut doc,
            "asm.parallel",
            json!({"body": a.0.to_string(), "face": under, "other": b.0.to_string(), "other_face": top,
                   "name": "Extra"}),
        )
        .unwrap();
        let found = call(&mut doc, "asm.redundant", json!({})).unwrap();
        let names: Vec<&str> = found
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"Extra"), "{names:?}");
    }

    /// A tab centred in a slot: its middle on the slot's middle.
    #[test]
    fn a_tab_is_centred_in_its_slot() {
        let mut doc = Document::new("t");
        let (tab, slot) = (doc.create_body(None), doc.create_body(None));
        let face = |x: f32, n: f32| json!({"point": [x, 0, 0], "normal": [n, 0, 0]});
        call(
            &mut doc,
            "asm.width",
            json!({"body": tab.0.to_string(), "face": face(0.0, -1.0), "face2": face(4.0, 1.0),
                   "other": slot.0.to_string(), "other_face": face(10.0, 1.0),
                   "other_face2": face(20.0, -1.0)}),
        )
        .unwrap();
        let middle = doc.body_placement(tab).point([2.0, 0.0, 0.0]);
        assert!((middle[0] - 15.0).abs() < 1e-3, "{middle:?}");
    }

    /// A point on a path runs along the edge it was put on; a follower on
    /// a cam sits a roller's radius off the cam's face.
    #[test]
    fn a_path_and_a_cam_hold_to_the_other_body_s_edge_and_face() {
        use std::sync::Arc;
        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        // An L-shaped edge along X then Y at height 10, and a floor face at
        // z = 10 facing up.
        let mesh = kernel_api::TriMesh {
            positions: vec![
                [0.0, 0.0, 10.0],
                [20.0, 0.0, 10.0],
                [20.0, 20.0, 10.0],
                [0.0, 20.0, 10.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            indices: vec![0, 1, 2, 0, 2, 3],
            faces: vec![0, 0],
            edges: vec![0, 1, 1, 2],
            edge_ids: vec![4, 4],
            ..kernel_api::TriMesh::default()
        };
        doc.set_imported_geometry(
            b,
            core_document::ImportedGeometry {
                mesh: Arc::new(mesh),
                source_asset: None,
                revision: 0,
                bounds_mm: None,
                brep_blob_path: None,
                mesh_path: None,
                face_colors_path: None,
                health: None,
            },
        );
        doc.set_body_placement(
            a,
            BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(18.0, 6.0, 15.0)),
        );
        call(
            &mut doc,
            "asm.path",
            json!({"body": a.0.to_string(), "face": {"point": [18, 6, 15]}, "other": b.0.to_string(),
                   "other_face": {"point": [20, 5, 10]}}),
        )
        .unwrap();
        let at = doc.body_placement(a).point([0.0; 3]);
        let joint = crate::joints(&doc)
            .into_iter()
            .find(|j| j.body == a)
            .unwrap();
        assert!(
            (at[0] - 20.0).abs() < 1e-3 && (at[2] - 10.0).abs() < 1e-3,
            "{at:?}"
        );
        assert_eq!(joint.feature.shape.len(), 3, "the whole L");

        let mut doc2 = doc.clone();
        let doc = &mut doc2;
        for id in crate::joints(doc).iter().map(|j| j.id).collect::<Vec<_>>() {
            doc.remove_feature(id).unwrap();
        }
        call(
            doc,
            "asm.cam",
            json!({"body": a.0.to_string(), "face": {"point": [5, 5, 30]}, "other": b.0.to_string(),
                   "other_face": {"point": [5, 5, 10]}, "radius": 2}),
        )
        .unwrap();
        // The follower was picked at (5, 5, 30) where the body stood.
        let cam = crate::joints(doc)
            .into_iter()
            .find(|j| j.body == a)
            .unwrap();
        let Anchor::Point { point } = cam.feature.moving else {
            panic!()
        };
        let follower = doc.body_placement(a).point(point);
        assert!((follower[2] - 12.0).abs() < 1e-3, "{follower:?}");
    }

    /// A pin in a slot rides on the slot's line with four motions left; a
    /// universal joint crosses two pins at a point with two.
    #[test]
    fn a_slot_and_a_universal_joint_leave_what_they_should() {
        let motions = |doc: &Document, body: BodyId| {
            crate::freedom(doc)
                .into_iter()
                .find(|(b, _)| *b == body)
                .map(|(_, m)| m.len())
                .unwrap()
        };
        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        call(
            &mut doc,
            "asm.slot",
            json!({"body": a.0.to_string(), "face": {"point": [0, 0, 5]}, "other": b.0.to_string(),
                   "other_face": {"axis": {"point": [10, 0, 0], "direction": [1, 0, 0]}}}),
        )
        .unwrap();
        let pin = doc.body_placement(a).point([0.0, 0.0, 5.0]);
        assert!(pin[1].abs() < 1e-3 && pin[2].abs() < 1e-3, "{pin:?}");
        assert_eq!(motions(&doc, a), 4);

        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        doc.set_body_placement(
            a,
            BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(3.0, 2.0, 1.0)),
        );
        call(
            &mut doc,
            "asm.universal",
            json!({"body": a.0.to_string(), "face": {"axis": {"point": [3, 2, 1], "direction": [1, 0, 0]}},
                   "other": b.0.to_string(), "other_face": {"axis": {"point": [0, 0, 0], "direction": [0, 1, 0]}}}),
        )
        .unwrap();
        let centre = doc.body_placement(a).point([0.0; 3]);
        assert!(glam::Vec3::from_array(centre).length() < 1e-3, "{centre:?}");
        assert_eq!(motions(&doc, a), 2);
    }

    /// An end moved along its normal moves the body with it: a mate with
    /// the other end raised 3 sits 3 higher.
    #[test]
    fn an_end_offset_moves_the_body() {
        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        let top = json!({"point": [0, 0, 0], "normal": [0, 0, 1]});
        let mate = call(
            &mut doc,
            "asm.mate",
            json!({"body": a.0.to_string(), "face": {"point": [0, 0, 0], "normal": [0, 0, -1]},
                   "other": b.0.to_string(), "other_face": top}),
        )
        .unwrap();
        call(&mut doc, "asm.set", json!({"joint": mate, "fixed_end": 3})).unwrap();
        assert!((doc.body_placement(a).translation[2] - 3.0).abs() < 1e-3);
        call(&mut doc, "asm.set", json!({"joint": mate, "moving_end": 1})).unwrap();
        // The moving end, its underside, pushed 1 down its own normal.
        assert!((doc.body_placement(a).translation[2] - 4.0).abs() < 1e-3);
    }

    /// A ball joint puts two points together and leaves three turns free;
    /// a distance holds between points or parallel axes as it does between
    /// faces, and parallel takes axes.
    #[test]
    fn points_and_axes_are_held_as_faces_are() {
        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        let ball = json!({"centre": [0, 0, 20], "point": [0, 0, 25]});
        let socket = json!({"point": [5, 5, 5]});
        call(
            &mut doc,
            "asm.ball",
            json!({"body": a.0.to_string(), "face": ball, "other": b.0.to_string(), "other_face": socket}),
        )
        .unwrap();
        let centre = doc.body_placement(a).point([0.0, 0.0, 20.0]);
        assert!(
            (centre[0] - 5.0).abs() < 1e-3
                && (centre[1] - 5.0).abs() < 1e-3
                && (centre[2] - 5.0).abs() < 1e-3,
            "{centre:?}"
        );
        let free = crate::freedom(&doc);
        let motions = &free.iter().find(|(body, _)| *body == a).unwrap().1;
        assert_eq!(motions.len(), 3, "three turns: {motions:?}");

        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        let axis = |x: f32| json!({"axis": {"point": [x, 0, 0], "direction": [0, 0, 1]}});
        call(
            &mut doc,
            "asm.parallel",
            json!({"body": a.0.to_string(), "face": axis(0.0), "other": b.0.to_string(), "other_face": axis(0.0)}),
        )
        .unwrap();
        call(
            &mut doc,
            "asm.distance",
            json!({"body": a.0.to_string(), "face": axis(0.0), "other": b.0.to_string(),
                   "other_face": axis(0.0), "offset": 30}),
        )
        .unwrap();
        let at = doc.body_placement(a).point([0.0, 0.0, 0.0]);
        assert!(
            ((at[0] * at[0] + at[1] * at[1]).sqrt() - 30.0).abs() < 1e-3,
            "{at:?}"
        );

        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        call(
            &mut doc,
            "asm.distance",
            json!({"body": a.0.to_string(), "face": {"point": [0, 0, 0]}, "other": b.0.to_string(),
                   "other_face": {"point": [0, 0, 0]}, "offset": 12}),
        )
        .unwrap();
        let at = doc.body_placement(a).point([0.0, 0.0, 0.0]);
        assert!(
            (glam::Vec3::from_array(at).length() - 12.0).abs() < 1e-3,
            "{at:?}"
        );
    }

    /// A group moves as one: the body held to its first member follows it
    /// when a mate moves that one.
    #[test]
    fn a_rigid_group_moves_as_one() {
        let mut doc = Document::new("t");
        let [base, lid, knob] = [
            doc.create_body(None),
            doc.create_body(None),
            doc.create_body(None),
        ];
        doc.set_body_placement(
            lid,
            BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(0.0, 0.0, 30.0)),
        );
        doc.set_body_placement(
            knob,
            BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(5.0, 0.0, 40.0)),
        );
        let group = call(
            &mut doc,
            "asm.group",
            json!({"bodies": [lid.0.to_string(), knob.0.to_string()]}),
        )
        .unwrap();
        assert!(group.is_string());
        // The lid's underside, 30 up, onto the base's top at 10.
        call(
            &mut doc,
            "asm.distance",
            json!({"body": lid.0.to_string(), "face": {"point": [0, 0, 30], "normal": [0, 0, -1]},
                   "other": base.0.to_string(), "other_face": {"point": [0, 0, 10], "normal": [0, 0, 1]},
                   "offset": 0}),
        )
        .unwrap();
        let lid_z = doc.body_placement(lid).translation[2];
        let knob_z = doc.body_placement(knob).translation[2];
        assert!((lid_z - 10.0).abs() < 1e-3, "{lid_z}");
        assert!(
            (knob_z - 20.0).abs() < 1e-3,
            "the knob keeps its 10 above: {knob_z}"
        );
        assert!(
            call(
                &mut doc,
                "asm.group",
                json!({"bodies": [lid.0.to_string()]})
            )
            .is_err()
        );
    }

    /// A rigid component moves as one whichever of its bodies a joint
    /// holds, and the joints inside it rest until it is made flexible.
    #[test]
    fn a_rigid_component_moves_as_one_and_a_flexible_one_solves_inside() {
        let mut doc = Document::new("t");
        let [base, lid, knob] = [
            doc.create_body(None),
            doc.create_body(None),
            doc.create_body(None),
        ];
        doc.set_body_placement(
            lid,
            BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(0.0, 0.0, 30.0)),
        );
        doc.set_body_placement(
            knob,
            BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(5.0, 0.0, 40.0)),
        );
        let component = doc.create_component("Lid".into(), None).unwrap();
        doc.set_body_component(lid, Some(component)).unwrap();
        doc.set_body_component(knob, Some(component)).unwrap();
        // The knob's underside, 40 up, onto the base's top at 10.
        call(
            &mut doc,
            "asm.distance",
            json!({"body": knob.0.to_string(), "face": {"point": [5, 0, 40], "normal": [0, 0, -1]},
                   "other": base.0.to_string(), "other_face": {"point": [0, 0, 10], "normal": [0, 0, 1]},
                   "offset": 0}),
        )
        .unwrap();
        let z = |doc: &Document, b| doc.body_placement(b).translation[2];
        assert!((z(&doc, knob) - 10.0).abs() < 1e-3, "{}", z(&doc, knob));
        assert!(
            (z(&doc, lid) - 0.0).abs() < 1e-3,
            "the lid follows: {}",
            z(&doc, lid)
        );
        // A joint inside the rigid component rests.
        call(
            &mut doc,
            "asm.distance",
            json!({"body": lid.0.to_string(), "face": {"point": [0, 0, 0], "normal": [0, 0, 1]},
                   "other": knob.0.to_string(), "other_face": {"point": [5, 0, 10], "normal": [0, 0, -1]},
                   "offset": 4}),
        )
        .unwrap();
        assert!((z(&doc, lid) - 0.0).abs() < 1e-3, "{}", z(&doc, lid));
        // Flexible, it holds: the lid's top 4 under the knob.
        let mut flexible = doc.component(component).unwrap().clone();
        flexible.flexible = true;
        doc.update_component(flexible).unwrap();
        let moves = crate::solve(&doc).unwrap();
        crate::place_bodies(&mut doc, &moves);
        assert!((z(&doc, knob) - 10.0).abs() < 1e-3, "{}", z(&doc, knob));
        assert!((z(&doc, lid) - 6.0).abs() < 1e-3, "{}", z(&doc, lid));
    }

    /// Components from commands: made, nested, flexible, emptied, taken
    /// apart.
    #[test]
    fn components_are_made_nested_and_taken_apart() {
        let mut doc = Document::new("t");
        let [a, b, c] = [
            doc.create_body(None),
            doc.create_body(None),
            doc.create_body(None),
        ];
        let id = |v: Value| ComponentId(uuid::Uuid::parse_str(v.as_str().unwrap()).unwrap());
        let outer = id(call(
            &mut doc,
            "asm.component",
            json!({"bodies": [a.0.to_string()], "parent": null}),
        )
        .unwrap());
        assert_eq!(doc.component(outer).unwrap().name, "Component 1");
        let inner = id(call(
            &mut doc,
            "asm.component",
            json!({"bodies": [b.0.to_string(), c.0.to_string()], "parent": outer.0.to_string(),
                   "flexible": true, "name": "Hinge"}),
        )
        .unwrap());
        assert_eq!(doc.component_bodies(outer), vec![a, b, c]);
        assert!(doc.component(inner).unwrap().flexible);
        // Inside itself is refused.
        assert!(
            call(
                &mut doc,
                "asm.component_set",
                json!({"component": outer.0.to_string(), "parent": inner.0.to_string()}),
            )
            .is_err()
        );
        call(
            &mut doc,
            "asm.component_set",
            json!({"component": inner.0.to_string(), "parent": null}),
        )
        .unwrap();
        assert_eq!(doc.component_bodies(outer), vec![a]);
        // "top" stands for null where a script cannot write one.
        call(
            &mut doc,
            "asm.component_set",
            json!({"component": inner.0.to_string(), "parent": outer.0.to_string()}),
        )
        .unwrap();
        assert_eq!(doc.component_bodies(outer), vec![a, b, c]);
        call(
            &mut doc,
            "asm.component_set",
            json!({"component": inner.0.to_string(), "parent": "top"}),
        )
        .unwrap();
        assert_eq!(doc.component_bodies(outer), vec![a]);
        call(
            &mut doc,
            "asm.component_add",
            json!({"bodies": [c.0.to_string()]}),
        )
        .unwrap();
        assert_eq!(doc.component_of(c), None);
        call(
            &mut doc,
            "asm.component_remove",
            json!({"component": inner.0.to_string()}),
        )
        .unwrap();
        assert_eq!(doc.component_of(b), None);
        assert_eq!(doc.components().len(), 1);
    }

    /// A joint that cannot hold with the others is not made, and one
    /// conflict holds up only the bodies joined to it: a joint between two
    /// other bodies still solves.
    #[test]
    fn a_joint_that_cannot_hold_is_not_made_and_blocks_nothing_else() {
        let mut doc = Document::new("t");
        let [a, b, c, d] = [
            doc.create_body(None),
            doc.create_body(None),
            doc.create_body(None),
            doc.create_body(None),
        ];
        let up = |z: f32| json!({"point": [0, 0, z], "normal": [0, 0, 1]});
        let down = |z: f32| json!({"point": [0, 0, z], "normal": [0, 0, -1]});
        // B's bottom 5 above A's top.
        call(
            &mut doc,
            "asm.distance",
            json!({"body": b.0.to_string(), "face": down(0.0), "other": a.0.to_string(),
                   "other_face": up(0.0), "offset": 5}),
        )
        .unwrap();
        let joints_before = crate::joints(&doc).len();
        // The same faces 9 apart as well: it cannot hold.
        let refused = call(
            &mut doc,
            "asm.distance",
            json!({"body": b.0.to_string(), "face": down(0.0), "other": a.0.to_string(),
                   "other_face": up(0.0), "offset": 9}),
        );
        assert!(refused.is_err(), "{refused:?}");
        assert_eq!(crate::joints(&doc).len(), joints_before, "not left behind");
        // An unrelated pair solves.
        call(
            &mut doc,
            "asm.distance",
            json!({"body": d.0.to_string(), "face": down(0.0), "other": c.0.to_string(),
                   "other_face": up(0.0), "offset": 3}),
        )
        .unwrap();
        let z = doc.body_placement(d).translation[2] - doc.body_placement(c).translation[2];
        assert!((z - 3.0).abs() < 1e-3, "{z}");
    }

    /// With a conflict already in the assembly, the rest is still placed.
    #[test]
    fn a_conflict_holds_up_only_what_it_is_joined_to() {
        let mut doc = Document::new("t");
        let [a, b, c, d] = [
            doc.create_body(None),
            doc.create_body(None),
            doc.create_body(None),
            doc.create_body(None),
        ];
        let up = |z: f32| json!({"point": [0, 0, z], "normal": [0, 0, 1]});
        let down = |z: f32| json!({"point": [0, 0, z], "normal": [0, 0, -1]});
        call(
            &mut doc,
            "asm.distance",
            json!({"body": b.0.to_string(), "face": down(0.0), "other": a.0.to_string(),
                   "other_face": up(0.0), "offset": 5}),
        )
        .unwrap();
        // A conflicting joint written straight into the document, as an
        // old file might hold it.
        let mut conflicting = crate::joints(&doc)[1].feature.clone();
        conflicting.kind = JointKind::Distance { offset: 9.0 };
        doc.add_feature_in_body(conflicting, "Clash".into(), Some(b))
            .unwrap();
        doc.set_body_placement(
            d,
            BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(0.0, 0.0, 40.0)),
        );
        let made = call(
            &mut doc,
            "asm.distance",
            json!({"body": d.0.to_string(), "face": down(40.0), "other": c.0.to_string(),
                   "other_face": up(0.0), "offset": 3}),
        );
        assert!(made.is_ok(), "{made:?}");
        let z = doc.body_placement(d).translation[2] - doc.body_placement(c).translation[2];
        assert!((z - 3.0).abs() < 1e-3, "D's bottom 3 above C's top: {z}");
    }

    /// A joint to the world holds the body to the origin's planes, and
    /// grounds nothing.
    #[test]
    fn a_body_mates_to_the_origin() {
        let mut doc = Document::new("t");
        let a = doc.create_body(None);
        doc.set_body_placement(
            a,
            BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(3.0, 4.0, 25.0)),
        );
        let bottom = json!({"point": [3, 4, 25], "normal": [0, 0, -1]});
        let xy = json!({"point": [0, 0, 0], "normal": [0, 0, 1]});
        call(
            &mut doc,
            "asm.mate",
            json!({"body": a.0.to_string(), "face": bottom, "other": crate::WORLD.0.to_string(),
                   "other_face": xy}),
        )
        .unwrap();
        let t = doc.body_placement(a).translation;
        assert!(t[2].abs() < 1e-3, "on the XY plane: {t:?}");
        assert!(
            crate::joints(&doc)
                .iter()
                .all(|j| j.feature.kind != JointKind::Ground),
            "the origin needs no ground"
        );
    }

    /// A joint made another kind keeps its faces and name; faces of the
    /// wrong sort for the kind asked are refused, and new faces go in.
    #[test]
    fn a_joint_changes_kind_and_faces() {
        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        let top = json!({"point": [0, 0, 0], "normal": [0, 0, 1]});
        let mate = call(
            &mut doc,
            "asm.mate",
            json!({"body": a.0.to_string(), "face": top, "other": b.0.to_string(), "other_face": top}),
        )
        .unwrap();
        let id = FeatureId(uuid::Uuid::parse_str(mate.as_str().unwrap()).unwrap());
        let kind = |doc: &Document| {
            serde_json::from_value::<JointFeature>(doc.get_feature_data(id).unwrap().clone())
                .unwrap()
                .kind
        };
        call(
            &mut doc,
            "asm.set",
            json!({"joint": mate, "kind": "parallel"}),
        )
        .unwrap();
        assert_eq!(kind(&doc), JointKind::Parallel);
        assert!(call(&mut doc, "asm.set", json!({"joint": mate, "kind": "hinge"})).is_err());
        let pin = json!({"axis": {"point": [0, 0, 0], "direction": [0, 0, 1]}});
        call(
            &mut doc,
            "asm.set",
            json!({"joint": mate, "kind": "hinge", "face": pin, "other_face": pin}),
        )
        .unwrap();
        assert!(matches!(kind(&doc), JointKind::Hinge { .. }));
        assert_eq!(doc.get_feature_meta(id).unwrap().name, "Mate 1");
    }

    /// An alignment's turn and slide are each driven, and an alignment
    /// stored as a plain word still loads, both free.
    #[test]
    fn an_alignment_drives_its_turn_and_its_slide() {
        let mut doc = Document::new("t");
        let (a, b) = (doc.create_body(None), doc.create_body(None));
        let pin = json!({"axis": {"point": [0, 0, 0], "direction": [0, 0, 1]}});
        let joint = call(
            &mut doc,
            "asm.align",
            json!({"body": a.0.to_string(), "face": pin, "other": b.0.to_string(), "other_face": pin,
                   "slide_drive": 5}),
        )
        .unwrap();
        assert!((doc.body_placement(a).translation[2] - 5.0).abs() < 1e-3);
        call(
            &mut doc,
            "asm.set",
            json!({"joint": joint, "turn_drive": 30}),
        )
        .unwrap();
        let x = doc.body_placement(a).direction([1.0, 0.0, 0.0]);
        assert!((x[1].atan2(x[0]).to_degrees() - 30.0).abs() < 1e-2, "{x:?}");
        assert!((doc.body_placement(a).translation[2] - 5.0).abs() < 1e-3);
        let free = crate::freedom(&doc);
        let motions = &free.iter().find(|(body, _)| *body == a).unwrap().1;
        assert!(motions.is_empty(), "both held: {motions:?}");

        let old = json!({
            "kind": "Align",
            "moving": {"Axis": {"point": [0.0, 0.0, 0.0], "direction": [0.0, 0.0, 1.0]}},
            "other_body": b.0.to_string(),
            "fixed": {"Axis": {"point": [0.0, 0.0, 0.0], "direction": [0.0, 0.0, 1.0]}},
        });
        let read: JointFeature = serde_json::from_value(old).expect("an old alignment loads");
        assert_eq!(read.kind, JointKind::align());
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
        // At its limit it may still turn, back the other way.
        let motions = &crate::freedom(&doc)[0].1;
        assert_eq!(motions.len(), 1);
        assert!(
            matches!(motions[0], crate::Motion::Turn { at_limit: true, .. }),
            "{motions:?}"
        );
        assert!(motions[0].describe().ends_with("(one way, at its limit)"));
        call(
            &mut doc,
            "asm.set",
            json!({"joint": joint, "limits": [-45, 45]}),
        )
        .unwrap();
        assert!(
            matches!(
                crate::freedom(&doc)[0].1[0],
                crate::Motion::Turn {
                    at_limit: false,
                    ..
                }
            ),
            "well within its limits"
        );
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
