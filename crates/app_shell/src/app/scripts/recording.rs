//! UI edits expressed as recorded command calls.

use super::document::suppress_call;
use crate::ui::TreeItemId;
use core_document::{BodyId, CommandArgs};
use serde_json::{Value, json};
use uuid::Uuid;

/// The command a body menu's edit runs, and its arguments.
pub(crate) fn body_edit_call(body: BodyId, edit: &crate::ui::BodyEdit) -> (&'static str, Value) {
    use crate::ui::BodyEdit;
    let id = body.0.to_string();
    match edit {
        BodyEdit::Frozen(frozen) => ("doc.set_body", json!({"body": id, "frozen": frozen})),
        BodyEdit::Scan(scan) => ("doc.set_body", json!({"body": id, "scan": scan})),
        BodyEdit::Selectable(selectable) => (
            "doc.set_body",
            json!({"body": id, "selectable": selectable}),
        ),
        BodyEdit::LinkedCopy => ("doc.linked_copy", json!({"body": id})),
        BodyEdit::Recompute => ("doc.recompute", json!({"body": id})),
    }
}

/// What a host UI command does, as the command a recording says it with:
/// renaming, showing or hiding and deleting tree rows. The benches record
/// their own.
pub(crate) fn recorded_of(command: &crate::ui::UiCommand) -> Option<core_document::Recorded> {
    use crate::ui::{TreeFeatureCommand, UiCommand};
    let call = |id: &str, args: Value| core_document::Recorded {
        id: id.to_string(),
        args: match args {
            Value::Object(map) => map,
            _ => CommandArgs::new(),
        },
        result: Value::Null,
    };
    match command {
        UiCommand::SetDocumentAgentRules(rules) => {
            Some(call("doc.set_agent_rules", json!({"rules": rules})))
        }
        UiCommand::Config(crate::ui::ConfigEdit::NewTable) => None,
        UiCommand::Config(edit) => {
            let (id, args) = edit.command();
            Some(call(id, args))
        }
        UiCommand::SetVariable {
            set,
            name,
            formula,
            comment,
        } => {
            let mut args = json!({"set": set.0.to_string(), "name": name, "formula": formula});
            if let Some(comment) = comment {
                args["comment"] = json!(comment);
            }
            Some(call("var.set", args))
        }
        UiCommand::RemoveVariable { set, name } => Some(call(
            "var.remove",
            json!({"set": set.0.to_string(), "name": name}),
        )),
        UiCommand::RenameVariable { set, name, to } => Some(call(
            "var.rename",
            json!({"set": set.0.to_string(), "name": name, "to": to}),
        )),
        UiCommand::SetParameter {
            feature,
            parameter,
            edit,
        } => {
            let which = parameter
                .name
                .clone()
                .unwrap_or_else(|| parameter.key.clone());
            let id = feature.0.to_string();
            Some(match edit {
                ui_kit::widgets::FormulaEdit::Formula(text) => call(
                    "doc.set_formula",
                    json!({"id": id, "parameter": which, "formula": text}),
                ),
                ui_kit::widgets::FormulaEdit::Value(v) => call(
                    "doc.set_value",
                    json!({"id": id, "parameter": which, "value": v}),
                ),
            })
        }
        UiCommand::RenameTreeItem { item, name } => {
            let id = item_id(*item)?;
            Some(call(
                "doc.rename",
                json!({"id": id.to_string(), "name": name}),
            ))
        }
        UiCommand::BodyEdit { body, edit } => {
            let (id, args) = body_edit_call(*body, edit);
            Some(call(id, args))
        }
        UiCommand::SetBodyDisplay { body, display } => Some(call(
            "doc.set_body",
            match display {
                Some(d) => json!({
                    "body": body.0.to_string(),
                    "color": d.color,
                    "opacity": d.opacity,
                }),
                None => json!({"body": body.0.to_string(), "color": Value::Null}),
            },
        )),
        UiCommand::SetBodyVisible { body, visible } => Some(call(
            "doc.set_visible",
            json!({"id": body.0.to_string(), "visible": visible}),
        )),
        UiCommand::SetNoteVisible { note, visible } => Some(call(
            "doc.set_visible",
            json!({"id": note.0.to_string(), "visible": visible}),
        )),
        UiCommand::SetImportedVisibility { node, visible } => Some(call(
            "doc.set_visible",
            json!({"id": node.to_string(), "visible": visible}),
        )),
        UiCommand::TreeFeature {
            feature,
            command: TreeFeatureCommand::SetVisible(visible),
        } => Some(call(
            "doc.set_visible",
            json!({"id": feature.0.to_string(), "visible": visible}),
        )),
        UiCommand::DeleteTreeItem(item) => {
            let id = item_id(*item)?;
            Some(call("doc.delete", json!({"id": id.to_string()})))
        }
        UiCommand::RepairShapes(bodies)
        | UiCommand::ConvertToSolid(bodies)
        | UiCommand::RefineShapes(bodies) => {
            let id = match command {
                UiCommand::RepairShapes(_) => "doc.repair",
                UiCommand::RefineShapes(_) => "doc.refine",
                _ => "doc.convert_to_solid",
            };
            let bodies: Vec<String> = bodies.iter().map(|b| b.0.to_string()).collect();
            Some(call(id, json!({"bodies": bodies})))
        }
        UiCommand::TreeFeature { feature, command } => {
            let id = feature.0.to_string();
            match command {
                TreeFeatureCommand::Suppress(on) => Some(suppress_call(*feature, *on)),
                TreeFeatureCommand::Delete => Some(call("doc.delete", json!({"id": id}))),
                TreeFeatureCommand::MoveUp => Some(call("doc.move", json!({"id": id, "up": true}))),
                TreeFeatureCommand::MoveDown => {
                    Some(call("doc.move", json!({"id": id, "up": false})))
                }
                TreeFeatureCommand::SetTip => Some(call("doc.set_tip", json!({"id": id}))),
                TreeFeatureCommand::ClearTip => {
                    Some(call("doc.set_tip", json!({"id": id, "clear": true})))
                }
                // Recorded where it lands, which only the document knows.
                TreeFeatureCommand::SetVisible(_) | TreeFeatureCommand::MoveNextTo { .. } => None,
            }
        }
        _ => None,
    }
}

pub(super) fn item_id(item: TreeItemId) -> Option<Uuid> {
    match item {
        TreeItemId::Body(b) => Some(b.0),
        TreeItemId::Feature(f) => Some(f.0),
        TreeItemId::ImportedObject(n) => Some(n),
        TreeItemId::Component(c) => Some(c.0),
        TreeItemId::Note(n) => Some(n.0),
        TreeItemId::Array(a) => Some(a.0),
        TreeItemId::DocumentRoot | TreeItemId::Notes(_) => None,
    }
}
