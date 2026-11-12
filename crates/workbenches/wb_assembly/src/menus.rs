//! The bench's entries on body and feature menus, and what each runs.

use crate::{AssemblyWorkbench, STATE_KIND, commands, states};
use core_document::{BodyId, HostRequest, WorkbenchFeature, WorkbenchRuntimeContext};

/// The body menus' entry selecting every joint that holds `body`, when
/// one does.
fn select_joints_item(
    document: &core_document::Document,
    body: BodyId,
) -> Option<core_document::MenuItem> {
    let count = commands::joints_holding(document, body).len();
    (count > 0).then(|| {
        core_document::MenuItem::new(
            "asm.menu.select_joints",
            if count == 1 {
                "Select its joint".to_string()
            } else {
                format!("Select its {count} joints")
            },
        )
        .icon("joint-mate")
        .hint("Select every joint that holds this body, in the tree")
    })
}

impl AssemblyWorkbench {
    pub(crate) fn menu_entries(
        &self,
        scope: &core_document::MenuScope,
        document: &core_document::Document,
    ) -> Vec<core_document::MenuItem> {
        use core_document::MenuItem;
        let id = match scope {
            core_document::MenuScope::TreeFeature(id) => id,
            core_document::MenuScope::ViewportBody(body) => {
                return select_joints_item(document, *body)
                    .map(MenuItem::separator_before)
                    .into_iter()
                    .collect();
            }
            core_document::MenuScope::TreeBody(body) => {
                let own = document.component_of(*body);
                let mut items: Vec<MenuItem> = select_joints_item(document, *body)
                    .map(MenuItem::separator_before)
                    .into_iter()
                    .collect();
                items.push(
                    MenuItem::new("asm.menu.component_new", "New component")
                        .icon("tree-group")
                        .hint("Put this body in a component of its own, inside the one it is in")
                        .separator_before(),
                );
                for component in document.components() {
                    if Some(component.id) != own {
                        items.push(
                            MenuItem::new(
                                format!("asm.menu.component_to.{}", component.id.0),
                                format!("Move to {}", component.name),
                            )
                            .hint("Put this body in that component"),
                        );
                    }
                }
                if let Some(own) = own.and_then(|c| document.component(c)) {
                    items.push(
                        MenuItem::new(
                            "asm.menu.component_out",
                            format!("Take out of {}", own.name),
                        )
                        .hint("Put this body one level up"),
                    );
                }
                return items;
            }
            core_document::MenuScope::TreeComponent(component) => {
                let Some(component) = document.component(*component) else {
                    return Vec::new();
                };
                return vec![
                    if component.flexible {
                        MenuItem::new("asm.menu.component_rigid", "Make rigid")
                            .hint("Move it as one; the joints inside it rest")
                    } else {
                        MenuItem::new("asm.menu.component_flexible", "Make flexible")
                            .hint("Keep the joints inside it live in the assembly")
                    },
                    MenuItem::new("asm.menu.component_sub", "New component inside")
                        .icon("tree-group")
                        .hint("An empty component in this one, to move bodies into"),
                ];
            }
            _ => return Vec::new(),
        };
        let is_state = document
            .get_feature_meta(*id)
            .is_some_and(|n| n.workbench_id.as_str() == STATE_KIND);
        if !is_state {
            return Vec::new();
        }
        vec![
            core_document::MenuItem::new("asm.restore_state", "Restore this state")
                .icon("save")
                .hint("Put every body back where it was, drives and visibility too")
                .separator_before(),
            core_document::MenuItem::new("asm.update_state", "Save the assembly into it")
                .hint("Keep the assembly as it stands now under this state's name"),
        ]
    }

    pub(crate) fn menu_command(
        &mut self,
        id: &str,
        scope: &core_document::MenuScope,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> bool {
        let component_call = match scope {
            core_document::MenuScope::TreeBody(body) => {
                let bodies = serde_json::json!([body.0.to_string()]);
                let own = ctx.document.component_of(*body);
                match id {
                    "asm.menu.component_new" => Some((
                        "asm.component",
                        serde_json::json!({"bodies": bodies, "parent": own.map(|c| c.0.to_string())}),
                        "New component",
                    )),
                    "asm.menu.component_out" => Some((
                        "asm.component_add",
                        serde_json::json!({
                            "bodies": bodies,
                            "component": own
                                .and_then(|c| ctx.document.component(c))
                                .and_then(|c| c.parent)
                                .map(|c| c.0.to_string()),
                        }),
                        "Take out of component",
                    )),
                    _ => id.strip_prefix("asm.menu.component_to.").map(|to| {
                        (
                            "asm.component_add",
                            serde_json::json!({"bodies": bodies, "component": to}),
                            "Move to component",
                        )
                    }),
                }
            }
            core_document::MenuScope::TreeComponent(component) => {
                let c = component.0.to_string();
                match id {
                    "asm.menu.component_rigid" | "asm.menu.component_flexible" => Some((
                        "asm.component_set",
                        serde_json::json!({
                            "component": c,
                            "flexible": id == "asm.menu.component_flexible",
                        }),
                        if id == "asm.menu.component_rigid" {
                            "Make component rigid"
                        } else {
                            "Make component flexible"
                        },
                    )),
                    "asm.menu.component_sub" => Some((
                        "asm.component",
                        serde_json::json!({"bodies": [], "parent": c}),
                        "New component",
                    )),
                    _ => None,
                }
            }
            _ => None,
        };
        if let (
            "asm.menu.select_joints",
            core_document::MenuScope::TreeBody(body) | core_document::MenuScope::ViewportBody(body),
        ) = (id, scope)
        {
            let args = commands::object(serde_json::json!({"body": body.0.to_string()}));
            if let Err(err) = commands::run("asm.select_joints", &args, ctx) {
                ctx.log_warn(format!("Select joints: {err}"));
            }
            return true;
        }
        if let Some((command, args, label)) = component_call {
            let args = commands::object(args);
            match commands::run(command, &args, ctx) {
                Ok(result) => {
                    ctx.record(command, args, result);
                    ctx.request(HostRequest::JournalLabel(label.into()));
                }
                Err(err) => ctx.log_warn(format!("{label}: {err}")),
            }
            return true;
        }
        let core_document::MenuScope::TreeFeature(state) = scope else {
            return false;
        };
        match id {
            "asm.restore_state" => {
                self.restore_state(ctx, *state);
                true
            }
            "asm.update_state" => {
                let now = states::capture(ctx.document);
                if ctx
                    .document
                    .update_feature_data(*state, now.to_json())
                    .is_ok()
                {
                    ctx.document.clear_feature_dirty(*state);
                    ctx.record(
                        "asm.save_state",
                        commands::object(serde_json::json!({"state": state.0.to_string()})),
                        serde_json::json!(state.0.to_string()),
                    );
                    ctx.request(HostRequest::JournalLabel("Save assembly state".into()));
                }
                true
            }
            _ => false,
        }
    }
}
