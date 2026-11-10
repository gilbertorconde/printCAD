//! Motion studies, traced points, exploded views, kept states and the clashes
//! of a motion.

use core_document::{
    Args, BodyId, BodyPlacement, CommandArgs, CommandError, CommandResult, CommandSpec, FeatureId,
    ParamKind, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};

use crate::joint::JointKind;
use crate::solve::joints;

use super::{BOX, next_name, single, solved};

/// Whether `id` is one of these commands.
pub(super) fn owns(id: &str) -> bool {
    matches!(
        id,
        "asm.motion"
            | "asm.motion_frames"
            | "asm.trace"
            | "asm.exploded_view"
            | "asm.explode_at"
            | "asm.save_state"
            | "asm.restore_state"
            | "asm.motion_clashes"
    )
}

/// These commands' specs.
pub(super) fn configure(specs: &mut Vec<CommandSpec>) {
    specs.push(
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
    specs.push(
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
    specs.push(
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
    specs.push(
        CommandSpec::new(
            "asm.exploded_view",
            "Keep an exploded view: steps, each turning, spreading and shifting some bodies, \
             played in order",
        )
        .param(
            "steps",
            ParamKind::List,
            "{{bodies = {ids}, shift = {x, y, z}, turn = {...}, radial = {...}}, ...}, in the \
             order they play",
        )
        .optional(
            "view",
            ParamKind::Id,
            "A view to change, rather than a new one",
        )
        .optional("name", ParamKind::String, "A new view's name in the tree")
        .returns("the view's id")
        .note(
            "Each step moves its bodies on from the steps before it: first turned by `turn`, \
             then pushed out by `radial`, then moved by `shift`, mm in world space. A step \
             needs both `bodies` and `shift` ({0, 0, 0} for none); `turn` and `radial` may \
             be left out.",
        )
        .note(
            "`turn = {point = {x, y, z}, axis = {x, y, z}, degrees = a}` turns the bodies \
             about the line through `point` along `axis`. `radial = {centre = {x, y, z}, \
             distance = d}` pushes each body `d` mm straight away from `centre`, measured \
             from the middle of its box; with `axis = {x, y, z}` it pushes square away from \
             the line through `centre` along it.",
        )
        .note(
            "Making it moves nothing: `asm.explode_at` answers where it puts the bodies. \
             `view` changes a view already made, keeping its id.",
        )
        .see_also("asm.explode_at")
        .example(
            "A lid lifted, then the base moved aside",
            &format!(
                "{BOX}{}",
                r#"
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
                "#
            ),
        )
        .example(
            "Two blocks spread apart from the point between them",
            r#"
            local function box(x)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = 0, width = 10, height = 10}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = 4}}.body
            end
            local left, right = box(0), box(30)
            assert(#pc.doc.rebuild() == 0)
            local view = pc.asm.exploded_view{steps = {
              {bodies = {left, right}, shift = {0, 0, 0},
               radial = {centre = {20, 5, 2}, distance = 10}},
            }}
            for _, p in ipairs(pc.asm.explode_at{view = view, at = 1}) do
              if p.body == left then assert(math.abs(p.translation[1] + 10) < 1e-3) end
              if p.body == right then assert(math.abs(p.translation[1] - 10) < 1e-3) end
            end
            "#,
        ),
    );
    specs.push(
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
    specs.push(
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
    specs.push(
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
    specs.push(
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
}

/// Run command `id`, one of these.
pub(super) fn run(
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    match id {
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
            let body = a.body(ctx.document, "body")?;
            let point = single(a.vec3("point")?);
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
                view.placed_at(ctx.document, &start, a.number("at")? as f32)
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
        _ => Err(CommandError::Unknown(id.to_string())),
    }
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
