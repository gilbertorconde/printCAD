//! Joints and couplings: making one, changing its settings, grounding, solving,
//! and what holds, what does not and the freedom left.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, FeatureId,
    ParamKind, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};

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
            explained(
                tool,
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
            ),
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
        &format!(
            "{BOX}{FACING}{}",
            r#"
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
            "#
        ),
    );
    t.add(align_drives(set), set_joint);
    t.add(
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
            &format!(
                "{BOX}{FACING}{}",
                r#"
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
                "#
            ),
        ),
        redundant,
    );
    t.add(
        CommandSpec::new("asm.select_joints", "Select every joint that holds a body")
            .param("body", ParamKind::Id, "The body")
            .returns("a list of {joint, name}, in history order")
            .read_only()
            .note(
                "The joints the body moves by and those that hold another body to it, a ground \
             on it too, suppressed ones as well. The tree selects them together, the first \
             the active one; with none, the document is selected.",
            )
            .see_also("asm.redundant")
            .see_also("asm.unmet")
            .example(
                "A lid mated to a grounded base, and a block mated onto the lid",
                &format!(
                    "{BOX}{FACING}{}",
                    r#"
            local base = box(0, 20, 20, 5)
            local lid = box(40, 10, 10, 3)
            local block = box(80, 4, 4, 3)
            assert(#pc.doc.rebuild() == 0)
            local g = pc.asm.ground{body = base}
            local m = pc.asm.mate{body = lid, face = facing(lid, -1),
              other = base, other_face = facing(base, 1)}
            pc.asm.mate{body = block, face = facing(block, -1),
              other = lid, other_face = facing(lid, 1)}
            local held = pc.asm.select_joints{body = base}
            assert(#held == 2 and held[1].joint == g and held[2].joint == m,
              "the ground and the mate onto it")
            assert(#pc.asm.select_joints{body = lid} == 2, "the lid's mate and the one onto it")
                    "#
                ),
            ),
        select_joints,
    );
    t.add(
        CommandSpec::new(
            "asm.unmet",
            "The joints and couplings that do not hold where the bodies sit",
        )
        .returns("a list of {joint, name}, in history order")
        .read_only()
        .note(
            "What a solve left apart: joints asking more of their bodies than they can give at \
             once, or a coupling its joints cannot follow. An assembly that holds lists \
             nothing.",
        )
        .see_also("asm.redundant")
        .see_also("asm.select_joints")
        .example(
            "A mate that holds",
            &format!(
                "{BOX}{FACING}{}",
                r#"
            local base = box(0, 20, 20, 5)
            local lid = box(40, 10, 10, 3)
            assert(#pc.doc.rebuild() == 0)
            pc.asm.mate{body = lid, face = facing(lid, -1),
              other = base, other_face = facing(base, 1)}
            assert(#pc.asm.unmet{} == 0, "the mate holds")
                "#
            ),
        ),
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
            &format!(
                "{BOX}{FACING}{}",
                r#"
            local base = box(0, 20, 20, 5)
            local lid = box(40, 10, 10, 3)
            assert(#pc.doc.rebuild() == 0)
            local g = pc.asm.ground{body = base}
            assert(pc.asm.ground{body = base} == g, "grounding twice keeps one ground")
            pc.asm.mate{body = lid, face = facing(lid, -1),
              other = base, other_face = facing(base, 1)}
            assert(pc.asm.placement{body = base}.translation[3] == 0, "the ground stays")
            assert(pc.asm.ground{body = base, grounded = false} == nil)
                "#
            ),
        ),
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
            &format!(
                "{BOX}{FACING}{}",
                r#"
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
                "#
            ),
        ),
        freedom,
    );
    t.add(
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
                &format!(
                    "{BOX}{FACING}{}",
                    r#"
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
                    "#
                ),
            ),
        solve,
    );
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

/// A joint command's notes, related commands and example.
fn explained(tool: JointTool, spec: CommandSpec) -> CommandSpec {
    match tool {
        JointTool::Mate => spec
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
                &format!(
                    "{BOX}{FACING}{}",
                    r#"
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
                    "#
                ),
            ),
        JointTool::Align => spec
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
        JointTool::Angle => spec
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
                &format!(
                    "{BOX}{FACING}{}",
                    r#"
            local base = box(0, 20, 20, 5)
            local plate = box(40, 10, 10, 2)
            assert(#pc.doc.rebuild() == 0)
            pc.asm.angle{body = plate, face = facing(plate, 1),
              other = base, other_face = facing(base, 1), degrees = 30}
            local w = pc.asm.placement{body = plate}.rotation[4]
            assert(math.abs(math.deg(2 * math.acos(w)) - 30) < 1e-3, "turned 30 degrees")
            assert(pc.asm.freedom{body = plate}[1].free == 5, "only the angle is held")
                    "#
                ),
            ),
        JointTool::Hinge => spec
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
        JointTool::Slider => spec
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
                &format!(
                    "{BOX}{}",
                    r#"
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
                    "#
                ),
            ),
        JointTool::Fixed => spec
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
                &format!(
                    "{BOX}{}",
                    r#"
            local base = box(0, 20, 20, 5)
            local tag = box(30, 5, 5, 5)
            assert(#pc.doc.rebuild() == 0)
            pc.asm.fix{body = tag, other = base}
            assert(pc.asm.freedom{body = tag}[1].free == 0)
            pc.asm.move{body = base, by = {0, 0, 10}}
            pc.asm.solve{}
            local at = pc.asm.placement{body = tag}.translation
            assert(math.abs(at[3] - 10) < 1e-4, "the tag follows the base")
                    "#
                ),
            ),
        JointTool::Parallel => spec
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
                &format!(
                    "{BOX}{}",
                    r#"
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
                    "#
                ),
            ),
        JointTool::Perpendicular => spec
            .note(
                "It turns the body until the faces or axes are square to each other and \
                 holds only that, five motions left.",
            )
            .see_also("asm.parallel")
            .see_also("asm.angle")
            .example(
                "A fin stood square to a base",
                &format!(
                    "{BOX}{}",
                    r#"
            local base = box(0, 20, 20, 5)
            local fin = box(40, 10, 10, 2)
            assert(#pc.doc.rebuild() == 0)
            pc.asm.move{body = fin, turn = 60, axis = {0, 1, 0}}
            pc.asm.perpendicular{body = fin, face = pc.doc.faces{body = fin}[6],
              other = base, other_face = pc.doc.faces{body = base}[6]}
            local n = pc.doc.faces{body = fin}[6].normal
            assert(math.abs(n[3]) < 1e-4, "the fin's face stands square to the base's top")
            assert(pc.asm.freedom{body = fin}[1].free == 5)
                    "#
                ),
            ),
        JointTool::Distance => spec
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
                &format!(
                    "{BOX}{FACING}{}",
                    r#"
            local base = box(0, 20, 20, 5)
            local plate = box(40, 10, 10, 2)
            assert(#pc.doc.rebuild() == 0)
            pc.asm.distance{body = plate, face = facing(plate, -1),
              other = base, other_face = facing(base, 1), offset = 10}
            local at = pc.asm.placement{body = plate}.translation
            assert(math.abs(at[3] - 15) < 1e-4, "10 mm above the base's top")
                    "#
                ),
            ),
        JointTool::Tangent => spec
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
        JointTool::Ball => spec
            .note(
                "It takes points: a ball's `{centre}`, else `{point}`; a flat face given \
                 whole is taken at its listed point.",
            )
            .note("The two points meet and the body may turn every way, three motions left.")
            .see_also("asm.universal")
            .see_also("asm.distance")
            .example(
                "An arm's corner on a base's corner",
                &format!(
                    "{BOX}{}",
                    r#"
            local base = box(0, 40, 40, 5)
            local arm = box(60, 30, 4, 4)
            assert(#pc.doc.rebuild() == 0)
            pc.asm.ball{body = arm, face = {point = {60, 0, 4}},
              other = base, other_face = {point = {40, 40, 5}}}
            local at = pc.asm.placement{body = arm}.translation
            assert(math.abs(at[1] + 20) < 1e-4 and math.abs(at[2] - 40) < 1e-4
              and math.abs(at[3] - 1) < 1e-4, "the arm's corner on the base's corner")
            assert(pc.asm.freedom{body = arm}[1].free == 3, "it turns every way")
                    "#
                ),
            ),
        JointTool::Universal => spec
            .note(
                "Each body gives a pin, `{axis = {point, direction}}`. The two axes' points \
                 meet and the body keeps two turns, one about each pin.",
            )
            .see_also("asm.ball")
            .see_also("asm.hinge")
            .example(
                "Two pins crossed at one point",
                &format!(
                    "{BOX}{}",
                    r#"
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
                    "#
                ),
            ),
        JointTool::Slot => spec
            .note(
                "`face` is the pin, a point (`{centre}` or `{point}`) on the moving body; \
                 `other_face` the slot, a line `{axis = {point, direction}}` on the other.",
            )
            .note("The pin stays on the line: the body keeps three turns and the slide along it.")
            .see_also("asm.path")
            .see_also("asm.slider")
            .example(
                "A pin kept on a line",
                &format!(
                    "{BOX}{}",
                    r#"
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
                    "#
                ),
            ),
        JointTool::Path => spec
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
        JointTool::Cam => spec
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
        JointTool::Width => spec
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
    }
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
