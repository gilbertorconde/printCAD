//! Assembly's commands: joints between bodies and moving bodies, by
//! numbers, for scripts and other callers that are not a click.
//!
//! A joint takes faces as `pc.doc.faces` lists them, where the bodies sit
//! now: a flat face as `{point, normal}`, a round one as its `axis`. Each
//! is stored in its body's own frame, as a picked face is, and every
//! joint solves as it is made or changed.

use core_document::{
    Args, BodyId, BodyPlacement, CommandArgs, CommandError, CommandResult, CommandTable, FeatureId,
    WorkbenchContext, WorkbenchRuntimeContext,
};
use glam::Vec3;
use std::sync::OnceLock;

pub(crate) use core_document::command::object;
use serde_json::{Value, json};

use crate::coupling::Coupling;
use crate::joint::{Anchor, JointFeature, JointKind, JointTool, Rigid};

/// The examples' setup: a block `w` by `h` padded `len` high with its
/// corner at `x` on the XY plane, its body answered.
const BOX: &str = r#"
            local function box(x, w, h, len)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
            end"#;

/// The examples' setup: a body's flat face facing up (`z` 1) or down (-1).
const FACING: &str = r#"
            local function facing(body, z)
              for _, f in ipairs(pc.doc.faces{body = body}) do
                if f.normal and f.normal[3] * z > 0.99 then return f end
              end
            end"#;

/// How far apart copies of `body` sit by default: its width along X and
/// a tenth more, in the world.
pub(crate) fn copy_step(document: &core_document::Document, body: BodyId) -> Vec3 {
    let width = document
        .imported_geometry(body)
        .and_then(|g| g.bounds())
        .map_or(20.0, |(lo, hi)| hi[0] - lo[0]);
    Vec3::new((width * 1.1).max(1.0), 0.0, 0.0)
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

mod checks;
mod components;
mod copies;
mod joints;
mod motion;
mod parts;
mod placing;
#[cfg(test)]
mod tests;

/// The commands in the order they are registered, which is the order the
/// scripting reference lists them, whichever family each belongs to. The
/// joint tools' commands, made from `JointTool::ALL` and not listed, come
/// first. A new command goes in here too: `the_order_names_every_command`
/// fails until it does.
const ORDER: &[&str] = &[
    "asm.couple",
    "asm.set",
    "asm.copy",
    "asm.mirror",
    "asm.replace",
    "asm.group",
    "asm.component",
    "asm.component_set",
    "asm.component_add",
    "asm.component_remove",
    "asm.motion",
    "asm.motion_frames",
    "asm.trace",
    "asm.exploded_view",
    "asm.explode_at",
    "asm.save_state",
    "asm.restore_state",
    "asm.redundant",
    "asm.motion_clashes",
    "asm.turn",
    "asm.flip",
    "asm.turn90",
    "asm.select_joints",
    "asm.unmet",
    "asm.interference",
    "asm.mass",
    "asm.parts",
    "asm.part",
    "asm.print_material",
    "asm.parts_table",
    "asm.travel",
    "asm.ground",
    "asm.freedom",
    "asm.solve",
    "asm.placement",
    "asm.place",
    "asm.move",
];

/// Every command this module runs, in `ORDER`.
fn table() -> &'static CommandTable {
    static TABLE: OnceLock<CommandTable> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = CommandTable::default();
        joints::table(&mut t);
        copies::table(&mut t);
        components::table(&mut t);
        motion::table(&mut t);
        placing::table(&mut t);
        checks::table(&mut t);
        parts::table(&mut t);
        t.sort_by_order(ORDER);
        t
    })
}

/// Register every command this module runs.
pub fn register(context: &mut WorkbenchContext) {
    table().register(context);
}

pub fn run(id: &str, args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    table().run(id, args, ctx)
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

/// `label 1`, `label 2`, ...: the next name no feature has.
pub(crate) fn next_name(document: &core_document::Document, label: &str) -> String {
    document.new_object_name(label, core_document::Numbering::Counted)
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

/// Solve, and answer `value` when every joint holds.
fn solved(ctx: &mut WorkbenchRuntimeContext, value: Value) -> CommandResult {
    crate::apply_solve(ctx).map_err(CommandError::failed)?;
    Ok(value)
}

/// Every joint that holds `body`, suppressed or not, in history order:
/// those it owns and those that hold another body to it.
pub(crate) fn joints_holding(
    document: &core_document::Document,
    body: BodyId,
) -> Vec<(FeatureId, String)> {
    let mut found: Vec<(u64, FeatureId, String)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, node)| node.workbench_id.as_str() == crate::JOINT_KIND)
        .filter(|(_, node)| {
            node.body == Some(body)
                || <JointFeature as core_document::WorkbenchFeature>::from_json(&node.data)
                    .is_ok_and(|j| j.kind != JointKind::Ground && j.other_body == body)
        })
        .map(|(id, node)| (node.seq, *id, node.name.clone()))
        .collect();
    found.sort_by_key(|(seq, id, _)| (*seq, *id));
    found.into_iter().map(|(_, id, name)| (id, name)).collect()
}

fn vector(value: &Value, name: &str) -> Result<Vec3, CommandError> {
    core_document::command::vec3_of(Some(value), name).map(single)
}

fn single(v: [f64; 3]) -> Vec3 {
    glam::DVec3::from(v).as_vec3()
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
