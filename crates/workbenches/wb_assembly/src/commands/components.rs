//! Rigid groups and components: making them, their settings, and the bodies
//! in them.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, ComponentId, FeatureId,
    ParamKind, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};

use super::{body_list, next_name, solved};

/// Whether `id` is one of these commands.
pub(super) fn owns(id: &str) -> bool {
    matches!(
        id,
        "asm.group"
            | "asm.component"
            | "asm.component_set"
            | "asm.component_add"
            | "asm.component_remove"
    )
}

/// These commands' specs.
pub(super) fn configure(specs: &mut Vec<CommandSpec>) {
    specs.push(
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
            &format!(
                "{CUBE}{}",
                r#"
            local a, b = box(0), box(10)
            assert(#pc.doc.rebuild() == 0)
            pc.asm.group{bodies = {a, b}}
            assert(pc.asm.freedom{body = b}[1].free == 0, "b holds to a")
            pc.asm.move{body = a, by = {0, 10, 0}}
            assert(pc.asm.placement{body = b}.translation[2] == 0, "a move places one body")
            pc.asm.solve{}
            local at = pc.asm.placement{body = b}.translation
            assert(math.abs(at[2] - 10) < 1e-4, "the solve brings b along")
                "#
            ),
        ),
    );
    specs.push(
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
            &format!(
                "{CUBE}{}",
                r#"
            local a, b, c = box(0), box(10), box(20)
            assert(#pc.doc.rebuild() == 0)
            pc.asm.component{bodies = {a, b}, name = "Pair"}
            pc.asm.move{body = a, by = {0, 7, 0}}
            assert(pc.asm.placement{body = b}.translation[2] == 7, "b moves with a")
            assert(pc.asm.placement{body = c}.translation[2] == 0, "c is not in it")
                "#
            ),
        ),
    );
    specs.push(
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
            &format!(
                "{CUBE}{}",
                r#"
            local a, b = box(0), box(10)
            assert(#pc.doc.rebuild() == 0)
            local k = pc.asm.component{bodies = {a, b}}
            pc.asm.component_set{component = k, name = "Loose pair", flexible = true}
            pc.asm.move{body = a, by = {0, 7, 0}}
            assert(pc.asm.placement{body = b}.translation[2] == 0, "flexible: a moves alone")
            pc.asm.component_set{component = k, flexible = false}
            pc.asm.move{body = a, by = {0, 1, 0}}
            assert(pc.asm.placement{body = b}.translation[2] == 1, "rigid: they move as one")
                "#
            ),
        )
        .example(
            "A component put in another and taken out to the top level",
            &format!(
                "{CUBE}{}",
                r#"
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
                "#
            ),
        ),
    );
    specs.push(
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
            &format!(
                "{CUBE}{}",
                r#"
            local a, b, c = box(0), box(10), box(20)
            assert(#pc.doc.rebuild() == 0)
            local k = pc.asm.component{bodies = {a, b}}
            pc.asm.component_add{bodies = {c}, component = k}
            pc.asm.move{body = a, by = {0, 0, 3}}
            assert(pc.asm.placement{body = c}.translation[3] == 3, "c moves with them")
            pc.asm.component_add{bodies = {c}}
            pc.asm.move{body = a, by = {0, 0, 3}}
            assert(pc.asm.placement{body = c}.translation[3] == 3, "taken out, it stays")
                "#
            ),
        ),
    );
    specs.push(
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
            &format!(
                "{CUBE}{}",
                r#"
            local a, b = box(0), box(10)
            assert(#pc.doc.rebuild() == 0)
            local k = pc.asm.component{bodies = {a, b}}
            pc.asm.component_remove{component = k}
            pc.asm.move{body = a, by = {0, 0, 3}}
            assert(pc.asm.placement{body = b}.translation[3] == 0, "apart, a moves alone")
            local renamed = pcall(pc.asm.component_set, {component = k, name = "Gone"})
            assert(not renamed, "the component is gone")
                "#
            ),
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
        "asm.group" => {
            let bodies = body_list(&a)?.unwrap_or_default();
            if bodies.len() < 2 {
                return Err(CommandError::bad("bodies", "must name two bodies or more"));
            }
            if let Some(b) = bodies.iter().find(|b| ctx.document.body(**b).is_none()) {
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
        _ => Err(CommandError::Unknown(id.to_string())),
    }
}

/// The examples' setup: a 5 mm cube with its corner at `x`.
const CUBE: &str = r#"
            local function box(x)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = 0, width = 5, height = 5}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
            end"#;

/// The `bodies` named, every one a body of the document.
fn existing_bodies(a: &Args, ctx: &WorkbenchRuntimeContext) -> Result<Vec<BodyId>, CommandError> {
    let bodies = body_list(a)?.unwrap_or_default();
    if let Some(b) = bodies.iter().find(|b| ctx.document.body(**b).is_none()) {
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
