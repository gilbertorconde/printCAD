//! The bench's menu entries: a body's and a feature row's, what each
//! runs, and Edit › Copy, Cut and Paste.

use crate::{
    DUPLICATE, DesignWorkbench, MAP_SKETCH, MOVE_TO_BODY, SECTION, SEPARATE, SPLIT, borrow,
    clipboard, commands, failing, making::ToolMade, section, separate, split,
};
use core_document::{
    BodyId, Document, FeatureId, HostRequest, MenuItem, MenuScope, WorkbenchId,
    WorkbenchRuntimeContext,
};

/// Duplicate `feature` into `body` (its own when `None`), record it as the
/// command and select the copy.
fn duplicate_recorded(ctx: &mut WorkbenchRuntimeContext, feature: FeatureId, body: Option<BodyId>) {
    match commands::duplicate(ctx, feature, body) {
        Ok(made) => {
            let mut args = serde_json::json!({"feature": feature.0.to_string()});
            if let Some(body) = body {
                args["body"] = serde_json::json!(body.0.to_string());
            }
            let ids: Vec<String> = made.iter().map(|f| f.0.to_string()).collect();
            ctx.record(DUPLICATE, commands::object(args), serde_json::json!(ids));
            ctx.active_document_object = made.last().copied();
            ctx.request(HostRequest::JournalLabel("Duplicate".into()));
        }
        Err(message) => ctx.log_warn(format!("Cannot duplicate the feature: {message}")),
    }
}

/// What Design offers on a body's menu, for a body with a solid: Split,
/// and Separate pieces when the solid is drawn in several pieces.
fn body_menu_items(document: &Document, body: BodyId) -> Vec<MenuItem> {
    if document.is_mesh_body(body) || document.imported_brep_blob(body).is_none() {
        return Vec::new();
    }
    let mut items = vec![
        MenuItem::new(SPLIT, "Split…")
            .icon("split")
            .hint("Cut the body into pieces along a plane or a sketch, each a body of its own")
            .separator_before(),
    ];
    if separate::drawn_pieces(document, body) >= 2 {
        items.push(
            MenuItem::new(SEPARATE, "Separate pieces")
                .icon("separate-pieces")
                .hint("One body per piece of the solid, each following this one"),
        );
    }
    items.extend(["z", "y", "x"].into_iter().map(|axis| {
        MenuItem::new(
            format!("{SECTION}:{axis}"),
            format!("Section across {} to sketch", axis.to_uppercase()),
        )
        .icon("section-sketch")
        .hint("The outline where a plane through the middle cuts the solid, as a new sketch")
    }));
    items
}

impl DesignWorkbench {
    /// The body menu's Split: the body cut in two across its middle, and
    /// the first piece's Split open in the task, where the cut is chosen.
    fn split_from_menu(&mut self, ctx: &mut WorkbenchRuntimeContext, body: BodyId) {
        let cut = split::Cut::across_middle(ctx.document, body);
        match split::split_body(ctx, body, cut, None) {
            Ok(pieces) => {
                let Some(&(first, feature)) = pieces.first() else {
                    return;
                };
                self.tab.pending_task_from_tool = Some(ToolMade {
                    feature,
                    tool: SPLIT.to_string(),
                    body: first,
                    pieces: pieces.iter().map(|(b, _)| *b).collect(),
                    repaired: None,
                });
                ctx.active_document_object = Some(feature);
                ctx.request(HostRequest::SwitchWorkbench(WorkbenchId::from(crate::KIND)));
            }
            Err(message) => ctx.log_warn(format!("Split: {message}")),
        }
    }

    /// Edit › Copy (or Cut) of the feature selected in the tree.
    fn copy(&mut self, ctx: &mut WorkbenchRuntimeContext, cut: bool) -> bool {
        let Some(id) = ctx.active_document_object else {
            return false;
        };
        let Some(copied) = clipboard::Clipboard::copy(ctx.document, id) else {
            return false;
        };
        let count = copied.ids().len();
        if cut {
            clipboard::cut(ctx.document, &copied);
            ctx.active_document_object = None;
            ctx.request(HostRequest::JournalLabel("Cut".into()));
            ctx.log_info(format!("Cut {count} feature(s)"));
        } else {
            ctx.log_info(format!("Copied {count} feature(s)"));
        }
        self.tab.clipboard = Some(copied);
        true
    }

    /// Edit › Paste: the copied features again, after the tip of the body
    /// selected (or the selected feature's body).
    fn paste(&mut self, ctx: &mut WorkbenchRuntimeContext) -> bool {
        let Some(copied) = self.tab.clipboard.as_ref().filter(|c| !c.is_empty()) else {
            return false;
        };
        let body = ctx
            .active_document_object
            .and_then(|f| ctx.document.get_feature_meta(f))
            .and_then(|n| n.body)
            .or(ctx.selected_body_id.map(BodyId))
            .filter(|b| ctx.document.body(*b).is_some());
        let Some(body) = body else {
            ctx.log_warn("Select a body to paste into");
            return true;
        };
        let made = copied.paste(ctx.document, body);
        ctx.active_document_object = made.last().copied();
        ctx.request(HostRequest::JournalLabel("Paste".into()));
        ctx.log_info(format!("Pasted {} feature(s)", made.len()));
        true
    }

    pub(crate) fn menu_entries(&self, scope: &MenuScope, document: &Document) -> Vec<MenuItem> {
        if let MenuScope::TreeBody(body) | MenuScope::ViewportBody(body) = scope {
            return body_menu_items(document, *body);
        }
        let MenuScope::TreeFeature(id) = scope else {
            return Vec::new();
        };
        let Some(node) = document.get_feature_meta(*id) else {
            return Vec::new();
        };
        let Some(from) = node.body else {
            return Vec::new();
        };
        if node.workbench_id.as_str() == wb_sketch::KIND {
            return borrow::flat_faces_of_body(document, from)
                .into_iter()
                .enumerate()
                .map(|(i, (r, name))| {
                    let item = MenuItem::new(
                        format!("{MAP_SKETCH}:{}:{}", r.borrow.0, r.index),
                        format!("Map onto {name}"),
                    )
                    .hint("The sketch follows the borrowed face");
                    if i == 0 {
                        item.separator_before()
                    } else {
                        item
                    }
                })
                .collect();
        }
        if !matches!(node.workbench_id.as_str(), crate::KIND | "core.datum") {
            return Vec::new();
        }
        let sharp = failing::sharp_count(document, *id);
        let leave_sharp = (sharp > 0).then(|| {
            MenuItem::new(failing::LEAVE_SHARP, failing::sharp_label(sharp))
                .hint("Takes the edges that fail out of the feature's edges")
                .separator_before()
        });
        let duplicate = MenuItem::new(DUPLICATE, "Duplicate")
            .hint("A copy after the tip, with its own sketch")
            .separator_before();
        leave_sharp
            .into_iter()
            .chain(std::iter::once(duplicate))
            .chain(
                document
                    .bodies()
                    .iter()
                    .filter(|b| b.id != from && !document.body_solid_is_imported(b.id))
                    .enumerate()
                    .map(|(i, b)| {
                        let item = MenuItem::new(
                            format!("{MOVE_TO_BODY}:{}", b.id.0),
                            format!("Move to {}", b.name),
                        )
                        .hint("With the sketch and datums only it uses");
                        if i == 0 {
                            item.separator_before()
                        } else {
                            item
                        }
                    }),
            )
            .collect()
    }

    pub(crate) fn menu_command(
        &mut self,
        id: &str,
        scope: &MenuScope,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> bool {
        match (scope, id) {
            (MenuScope::EditMenu, "edit.copy") => return self.copy(ctx, false),
            (MenuScope::EditMenu, "edit.cut") => return self.copy(ctx, true),
            (MenuScope::EditMenu, "edit.paste") => return self.paste(ctx),
            (MenuScope::TreeBody(body) | MenuScope::ViewportBody(body), SEPARATE) => {
                let args = commands::object(serde_json::json!({"body": body.0.to_string()}));
                match separate::separate_body(ctx, *body) {
                    Ok(pieces) => {
                        let made = pieces.iter().map(|b| b.0.to_string()).collect::<Vec<_>>();
                        ctx.record(SEPARATE, args, serde_json::Value::from(made));
                        ctx.request(HostRequest::JournalLabel("Separate pieces".into()));
                    }
                    Err(message) => ctx.log_warn(format!("Separate pieces: {message}")),
                }
                return true;
            }
            (MenuScope::TreeBody(body) | MenuScope::ViewportBody(body), SPLIT) => {
                self.split_from_menu(ctx, *body);
                return true;
            }
            (MenuScope::TreeBody(body) | MenuScope::ViewportBody(body), id)
                if id.starts_with(SECTION) =>
            {
                let axis = match id.strip_prefix(SECTION).and_then(|r| r.strip_prefix(':')) {
                    Some("x") => 0,
                    Some("y") => 1,
                    _ => 2,
                };
                let cuts = section::Cuts::middle(ctx.document, *body, axis);
                match section::section_body(ctx, *body, cuts, None, None) {
                    Ok(made) => {
                        let args = commands::object(serde_json::json!({
                            "body": body.0.to_string(),
                            "plane": cuts.plane,
                            "offset": cuts.offset,
                        }));
                        let ids = made.iter().map(|f| f.0.to_string()).collect::<Vec<_>>();
                        ctx.record(SECTION, args, serde_json::Value::from(ids));
                        ctx.request(HostRequest::JournalLabel("Section to sketch".into()));
                        if let Some(first) = made.first() {
                            ctx.request(HostRequest::SelectFeatures(vec![*first]));
                        }
                    }
                    Err(message) => ctx.log_warn(format!("Section to sketch: {message}")),
                }
                return true;
            }
            (MenuScope::TreeFeature(feature), DUPLICATE) => {
                duplicate_recorded(ctx, *feature, None);
                return true;
            }
            (MenuScope::TreeFeature(feature), failing::LEAVE_SHARP) => {
                match failing::leave_sharp(ctx, *feature) {
                    Ok(count) => ctx.request(core_document::HostRequest::JournalLabel(
                        failing::sharp_label(count),
                    )),
                    Err(why) => ctx.log_warn(why),
                }
                return true;
            }
            (MenuScope::TreeFeature(sketch), id) if id.starts_with(MAP_SKETCH) => {
                let face = id
                    .strip_prefix(MAP_SKETCH)
                    .and_then(|rest| rest.strip_prefix(':'))
                    .and_then(|rest| rest.split_once(':'))
                    .and_then(|(borrow, index)| {
                        Some(crate::feature::BorrowedRef {
                            borrow: FeatureId(uuid::Uuid::parse_str(borrow).ok()?),
                            index: index.parse().ok()?,
                        })
                    });
                let Some(face) = face else {
                    return false;
                };
                match borrow::map_sketch(ctx.document, *sketch, face) {
                    Ok(()) => {
                        ctx.record(
                            MAP_SKETCH,
                            commands::object(serde_json::json!({
                                "sketch": sketch.0.to_string(),
                                "borrow": face.borrow.0.to_string(),
                                "index": face.index,
                            })),
                            serde_json::Value::Null,
                        );
                        ctx.request(HostRequest::JournalLabel("Map sketch".into()));
                    }
                    Err(message) => ctx.log_warn(format!("Cannot map the sketch: {message}")),
                }
                return true;
            }
            _ => {}
        }
        let (MenuScope::TreeFeature(feature), Some(body)) = (scope, id.strip_prefix(MOVE_TO_BODY))
        else {
            return false;
        };
        let Some(body) = body
            .strip_prefix(':')
            .and_then(|b| uuid::Uuid::parse_str(b).ok())
            .map(BodyId)
        else {
            return false;
        };
        match commands::move_feature(ctx, *feature, body) {
            Ok(moved) => {
                ctx.record(
                    MOVE_TO_BODY,
                    commands::object(serde_json::json!({
                        "feature": feature.0.to_string(),
                        "body": body.0.to_string(),
                    })),
                    serde_json::json!(moved.iter().map(|f| f.0.to_string()).collect::<Vec<_>>()),
                );
                ctx.request(HostRequest::JournalLabel("Move to body".into()));
            }
            Err(message) => ctx.log_warn(format!("Cannot move the feature: {message}")),
        }
        true
    }
}
