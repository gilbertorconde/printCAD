//! Rigid groups and components: making them, their settings, and the bodies
//! in them.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, ComponentId,
    FeatureId, ParamKind, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};

use super::{body_list, next_name, solved};

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable) {
    t.add(
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
        .returns("the group's id"),
        group,
    );
    t.add(
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
        .returns("the component's id"),
        component,
    );
    t.add(
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
        ),
        component_set,
    );
    t.add(
        CommandSpec::new(
            "asm.component_add",
            "Put bodies in a component, or take them out",
        )
        .param("bodies", ParamKind::List, "The bodies")
        .optional(
            "component",
            ParamKind::Id,
            "The component; left out, the bodies go to the top",
        ),
        component_add,
    );
    t.add(
        CommandSpec::new(
            "asm.component_remove",
            "Take a component apart: its bodies and components go one level up",
        )
        .param("component", ParamKind::Id, "The component"),
        component_remove,
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "components.md", include_str!("components.md"), &[])
}

fn group(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
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

fn component(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
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

fn component_set(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let id = ComponentId(a.id("component")?);
    let mut component = ctx
        .document
        .component(id)
        .cloned()
        .ok_or_else(|| CommandError::bad("component", "is not a component of this document"))?;
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

fn component_add(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let bodies = existing_bodies(&a, ctx)?;
    let component = a.opt_id("component")?.map(ComponentId);
    for body in bodies {
        ctx.document
            .set_body_component(body, component)
            .map_err(|e| CommandError::bad("component", e.to_string()))?;
    }
    solved(ctx, Value::Null)
}

fn component_remove(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let id = ComponentId(a.id("component")?);
    ctx.document
        .remove_component(id)
        .map_err(|e| CommandError::bad("component", e.to_string()))?;
    solved(ctx, Value::Null)
}

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
