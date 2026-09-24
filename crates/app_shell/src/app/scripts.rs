//! Scripts and the console: the Lua engine run against every command the
//! application and the workbenches offer.
//!
//! The application's own commands are the document's queries and edits
//! (`doc.*`) and every command a key can run (`file.save`, `view.front`,
//! ...). A workbench's command runs in the workbench through
//! `Workbench::run_command`, with the same context its tools get, so the
//! host never needs to know what the command does. Every call's arguments
//! are checked against its spec before it runs.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, FeatureId, ParamKind,
};
use serde_json::{Value, json};
use uuid::Uuid;
use winit::event_loop::ActiveEventLoop;

use crate::PrintCadApp;
use crate::app::workbench_host::HookSite;
use crate::console::{self, LineKind};
use crate::ui::TreeItemId;
use crate::ui::keymap::{self, HostOutcome, HostState};

/// The application's own commands beyond the keyboard's.
fn doc_commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("doc.info", "The document's name, file and unit")
            .returns("{name, file, unit, modified}"),
        CommandSpec::new("doc.bodies", "List the bodies")
            .returns("a list of {id, name, visible, features}"),
        CommandSpec::new("doc.features", "List the features in build order")
            .optional("body", ParamKind::Id, "Only this body's")
            .returns("a list of {id, name, kind, body, visible, suppressed, error}"),
        CommandSpec::new("doc.feature", "A feature with its fields")
            .param("id", ParamKind::Id, "")
            .returns("{id, name, kind, body, visible, fields}"),
        CommandSpec::new("doc.selection", "What is selected")
            .returns("{item, body, feature}, each an id or nil"),
        CommandSpec::new(
            "doc.select",
            "Select a body or a feature, as a click on its row",
        )
        .param("id", ParamKind::Id, ""),
        CommandSpec::new("doc.new_body", "Make an empty body")
            .optional("name", ParamKind::String, "")
            .returns("the body's id"),
        CommandSpec::new("doc.rename", "Rename a body or a feature")
            .param("id", ParamKind::Id, "")
            .param("name", ParamKind::String, ""),
        CommandSpec::new("doc.set_visible", "Show or hide a body or a feature")
            .param("id", ParamKind::Id, "")
            .param("visible", ParamKind::Bool, ""),
        CommandSpec::new("doc.delete", "Delete a body or a feature").param("id", ParamKind::Id, ""),
    ]
}

/// The commands a key can run that make sense without a key: all but
/// those that open a window of the interface's own.
fn key_commands() -> impl Iterator<Item = (CommandSpec, keymap::HostAction)> {
    use keymap::HostAction::*;
    keymap::host_actions()
        .filter(|(_, _, action)| {
            !matches!(
                action,
                Palette | Preferences | Console | Delete | ToggleVisibility | PivotAtCursor
            )
        })
        .map(|(id, label, action)| (CommandSpec::new(id, label), action))
}

/// The commands a script can call, by the application and then the
/// workbenches in registration order.
fn all_commands(app: &PrintCadApp) -> Vec<CommandSpec> {
    let mut out = doc_commands();
    out.extend(key_commands().map(|(spec, _)| spec));
    out.extend(app.registry.commands().into_iter().map(|(_, c)| c.clone()));
    out
}

/// What scripts run against: the application, for the length of one run.
struct AppHost<'a> {
    app: &'a mut PrintCadApp,
    event_loop: &'a ActiveEventLoop,
}

impl scripting::Host for AppHost<'_> {
    fn commands(&self) -> Vec<CommandSpec> {
        all_commands(self.app)
    }

    fn call(&mut self, id: &str, args: CommandArgs) -> CommandResult {
        if let Some(spec) = doc_commands().into_iter().find(|c| c.id == id) {
            spec.check(&args)?;
            return self.app.run_doc_command(id, &args, self.event_loop);
        }
        if let Some((spec, action)) = key_commands().find(|(c, _)| c.id == id) {
            spec.check(&args)?;
            return self.app.run_key_command(action, self.event_loop);
        }
        let Some((bench, spec)) = self.app.registry.command(id) else {
            return Err(CommandError::Unknown(id.to_string()));
        };
        spec.check(&args)?;
        let params = self.app.interaction_ctx_params();
        let (result, outcome) = self
            .app
            .with_workbench_ctx(&bench, params, |wb, ctx| wb.run_command(id, &args, ctx))
            .ok_or_else(|| CommandError::Unknown(id.to_string()))?;
        self.app.apply_hook_outcome(outcome, HookSite::Interaction);
        result
    }
}

impl PrintCadApp {
    /// Run one line typed in the console and show it with what it printed
    /// and came to.
    pub(crate) fn run_console_line(&mut self, line: &str, event_loop: &ActiveEventLoop) {
        console::push(LineKind::Input, line);
        let mut engine = self.scripts.take().unwrap_or_default();
        let out = engine.eval_line(
            line,
            &mut AppHost {
                app: self,
                event_loop,
            },
        );
        self.scripts = Some(engine);
        for printed in out.printed {
            console::push(LineKind::Printed, printed);
        }
        if let Some(value) = out.value {
            console::push(LineKind::Value, value);
        }
        if let Some(error) = out.error {
            console::push(LineKind::Error, error);
        }
        self.session.journal.label_next("Console");
        self.redraw_needed = true;
    }

    fn run_key_command(
        &mut self,
        action: keymap::HostAction,
        event_loop: &ActiveEventLoop,
    ) -> CommandResult {
        let state = HostState {
            active_tab: Some(self.session.tab),
            section_on: self.session.section.is_some(),
            document: &self.session.document,
            tree_selection: self.session.tree_selection,
            editing: false,
        };
        match keymap::host_outcome(action, &state) {
            HostOutcome::Command(command) => {
                self.apply_ui_commands(vec![command], event_loop);
                Ok(Value::Null)
            }
            _ => Err(CommandError::failed("this command is not available here")),
        }
    }

    fn run_doc_command(
        &mut self,
        id: &str,
        args: &CommandArgs,
        event_loop: &ActiveEventLoop,
    ) -> CommandResult {
        let a = Args(args);
        let document = &self.session.document;
        match id {
            "doc.info" => Ok(json!({
                "name": document.name(),
                "file": self.session.current_file.as_ref().map(|p| p.display().to_string()),
                "unit": document.display_unit().short_label(),
                "modified": document.metadata().dirty(),
            })),
            "doc.bodies" => Ok(Value::Array(
                document
                    .bodies()
                    .iter()
                    .map(|b| {
                        json!({
                            "id": b.id.0.to_string(),
                            "name": b.name,
                            "visible": !b.hidden,
                            "features": features_in_order(document, Some(b.id))
                                .iter()
                                .map(|n| n.id.0.to_string())
                                .collect::<Vec<_>>(),
                        })
                    })
                    .collect(),
            )),
            "doc.features" => {
                let body = a.opt_id("body")?.map(BodyId);
                Ok(Value::Array(
                    features_in_order(document, None)
                        .into_iter()
                        .filter(|n| body.is_none() || n.body == body)
                        .map(|n| {
                            json!({
                                "id": n.id.0.to_string(),
                                "name": n.name,
                                "kind": self.kind_of(n),
                                "body": n.body.map(|b| b.0.to_string()),
                                "visible": n.visible,
                                "suppressed": n.suppressed,
                                "error": n.error,
                            })
                        })
                        .collect(),
                ))
            }
            "doc.feature" => {
                let node = document
                    .get_feature_meta(FeatureId(a.id("id")?))
                    .ok_or_else(|| CommandError::bad("id", "is not a feature of this document"))?;
                Ok(json!({
                    "id": node.id.0.to_string(),
                    "name": node.name,
                    "kind": self.kind_of(node),
                    "body": node.body.map(|b| b.0.to_string()),
                    "visible": node.visible,
                    "fields": node.data,
                }))
            }
            "doc.selection" => Ok(json!({
                "item": self.session.tree_selection.and_then(item_id).map(|u| u.to_string()),
                "body": self.session.active_body_id.map(|b| b.0.to_string()),
                "feature": self.session.active_document_object.map(|f| f.0.to_string()),
            })),
            "doc.select" => {
                let item = self.tree_item(a.id("id")?)?;
                self.apply_tree_selection(item);
                Ok(Value::Null)
            }
            "doc.new_body" => {
                let body = self.session.document.create_body(None);
                if let Some(name) = a.opt_string("name")? {
                    self.session.document.rename_body(body, name);
                }
                Ok(json!(body.0.to_string()))
            }
            "doc.rename" => {
                let name = a.string("name")?.to_string();
                match self.tree_item(a.id("id")?)? {
                    TreeItemId::Body(body) => self.session.document.rename_body(body, name),
                    TreeItemId::Feature(feature) => {
                        self.session.document.rename_feature(feature, name)
                    }
                    _ => return Err(CommandError::bad("id", "cannot be renamed")),
                }
                Ok(Value::Null)
            }
            "doc.set_visible" => {
                let visible = a.opt_bool("visible")?.unwrap_or(true);
                let command = match self.tree_item(a.id("id")?)? {
                    TreeItemId::Body(body) => {
                        crate::ui::UiCommand::SetBodyVisible { body, visible }
                    }
                    TreeItemId::Feature(feature) => crate::ui::UiCommand::TreeFeature {
                        feature,
                        command: crate::ui::TreeFeatureCommand::SetVisible(visible),
                    },
                    TreeItemId::ImportedObject(node) => {
                        crate::ui::UiCommand::SetImportedVisibility { node, visible }
                    }
                    TreeItemId::DocumentRoot => {
                        return Err(CommandError::bad("id", "cannot be hidden"));
                    }
                };
                self.apply_ui_commands(vec![command], event_loop);
                Ok(Value::Null)
            }
            "doc.delete" => {
                let item = self.tree_item(a.id("id")?)?;
                self.apply_ui_commands(
                    vec![crate::ui::UiCommand::DeleteTreeItem(item)],
                    event_loop,
                );
                Ok(Value::Null)
            }
            _ => Err(CommandError::Unknown(id.to_string())),
        }
    }

    /// What kind of feature `node` is, as its workbench names it.
    fn kind_of(&self, node: &core_document::FeatureNode) -> String {
        self.registry
            .feature_info(node)
            .map(|info| info.kind_label)
            .unwrap_or_else(|| node.workbench_id.as_str().to_string())
    }

    /// The tree row an id names: a body, a feature or an imported part.
    fn tree_item(&self, id: Uuid) -> Result<TreeItemId, CommandError> {
        let document = &self.session.document;
        if document.bodies().iter().any(|b| b.id.0 == id) {
            Ok(TreeItemId::Body(BodyId(id)))
        } else if document.get_feature_meta(FeatureId(id)).is_some() {
            Ok(TreeItemId::Feature(FeatureId(id)))
        } else if document.imported_object(id).is_some() {
            Ok(TreeItemId::ImportedObject(id))
        } else {
            Err(CommandError::bad("id", "is not in this document"))
        }
    }
}

/// The features of `body` (or all of them) in build order.
fn features_in_order(
    document: &core_document::Document,
    body: Option<BodyId>,
) -> Vec<&core_document::FeatureNode> {
    let mut nodes: Vec<_> = document
        .feature_tree()
        .all_nodes()
        .map(|(_, n)| n)
        .filter(|n| body.is_none() || n.body == body)
        .collect();
    nodes.sort_by_key(|n| n.seq);
    nodes
}

fn item_id(item: TreeItemId) -> Option<Uuid> {
    match item {
        TreeItemId::Body(b) => Some(b.0),
        TreeItemId::Feature(f) => Some(f.0),
        TreeItemId::ImportedObject(n) => Some(n),
        TreeItemId::DocumentRoot => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_application_s_command_ids_are_unique() {
        let mut ids: Vec<String> = doc_commands().into_iter().map(|c| c.id).collect();
        ids.extend(key_commands().map(|(c, _)| c.id));
        let count = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }
}
