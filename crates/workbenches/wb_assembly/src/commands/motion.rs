//! Motion studies, traced points, exploded views, kept states and the clashes
//! of a motion.

use core_document::{
    Args, BodyId, BodyPlacement, CommandArgs, CommandError, CommandResult, CommandSpec,
    CommandTable, FeatureId, ParamKind, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};

use crate::joint::JointKind;
use crate::solve::joints;

use super::{BOX, next_name, single, solved};

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable) {
    t.add(
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
        .returns("the motion's id"),
        motion,
    );
    t.add(
        CommandSpec::new(
            "asm.motion_frames",
            "Every body's placement at each frame of a motion; nothing is moved",
        )
        .param("study", ParamKind::Id, "The motion")
        .returns("a list of {t, bodies = {{body, translation, rotation}, ...}}")
        .read_only(),
        motion_frames,
    );
    t.add(
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
        .read_only(),
        trace,
    );
    t.add(
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
        .returns("the view's id"),
        exploded_view,
    );
    t.add(
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
        .read_only(),
        explode_at,
    );
    t.add(
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
        .returns("the state's id"),
        save_state,
    );
    t.add(
        CommandSpec::new(
            "asm.restore_state",
            "Put the assembly back as a saved state has it",
        )
        .param("state", ParamKind::Id, "The saved state"),
        restore_state,
    );
    t.add(
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
        .read_only(),
        motion_clashes,
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands, the
/// examples' setup as `{{box}}`.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(
        &PROSE,
        "motion.md",
        include_str!("motion.md"),
        &[("box", BOX)],
    )
}

fn motion(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
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

fn motion_frames(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
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

fn trace(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
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

fn exploded_view(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
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

fn explode_at(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
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

fn save_state(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
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
    let id =
        crate::states::save(ctx.document, name).map_err(|e| CommandError::failed(e.to_string()))?;
    Ok(json!(id.0.to_string()))
}

fn restore_state(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let id = FeatureId(a.id("state")?);
    let state = ctx
        .document
        .get_feature_data(id)
        .and_then(|d| <crate::AssemblyState as core_document::WorkbenchFeature>::from_json(d).ok())
        .ok_or_else(|| CommandError::bad("state", "is not a saved assembly state"))?;
    crate::states::restore(ctx.document, &state);
    solved(ctx, Value::Null)
}

fn motion_clashes(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
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
    let Some(check) = crate::sweep_check::plan(ctx.document, joint, low, high, steps) else {
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
