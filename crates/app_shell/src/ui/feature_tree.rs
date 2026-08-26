use std::collections::{HashMap, HashSet};

use core_document::{Body, BodyId, Document, FeatureId, FeatureNode, FeatureTree};
use egui::{Color32, Response, RichText, Ui};
use uuid::Uuid;

/// Identifier for selectable items in the tree panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TreeItemId {
    DocumentRoot,
    Body(BodyId),
    Feature(FeatureId),
    ImportedObject(Uuid),
}

impl From<FeatureId> for TreeItemId {
    fn from(value: FeatureId) -> Self {
        TreeItemId::Feature(value)
    }
}

/// Context-menu action on a feature row, applied by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeFeatureCommand {
    Suppress(bool),
    SetVisible(bool),
    Delete,
    MoveUp,
    MoveDown,
    SetTip,
    ClearTip,
}

#[derive(Debug, Default)]
pub struct TreeUiResult {
    pub selection: Option<TreeItemId>,
    pub activation: Option<TreeItemId>,
    pub imported_visibility_change: Option<(Uuid, bool)>,
    pub feature_command: Option<(FeatureId, TreeFeatureCommand)>,
}

/// View model describing the current document tree.
#[derive(Debug)]
pub struct DocumentTree {
    document_label: String,
    nodes: Vec<TreeNode>,
}

#[derive(Debug)]
struct TreeNode {
    id: TreeItemId,
    label: String,
    badge: Option<String>,
    tooltip: Option<String>,
    dirty: bool,
    visible: bool,
    suppressed: bool,
    error: Option<String>,
    /// Marks the body-tip feature / features past the tip (excluded from
    /// the build).
    is_tip: bool,
    after_tip: bool,
    /// Feature nodes get a history context menu.
    feature_menu: Option<FeatureId>,
    /// Insertion order within the document; THE history ordering key.
    seq: u64,
    children: Vec<TreeNode>,
    imported_object_id: Option<Uuid>,
}

impl DocumentTree {
    pub fn build(document: &Document) -> Self {
        let feature_tree = document.feature_tree();
        let mut visited = HashSet::new();
        let mut roots_by_body: HashMap<Option<BodyId>, Vec<TreeNode>> = HashMap::new();

        // Helper to group feature roots under their owning body (or None for document-level).
        let push_root = |body: Option<BodyId>,
                         node: TreeNode,
                         map: &mut HashMap<Option<BodyId>, Vec<TreeNode>>| {
            map.entry(body).or_default().push(node);
        };

        // Tip metadata per body: the tip feature's seq bounds the build.
        let tip_seq_by_body: HashMap<BodyId, (FeatureId, u64)> = document
            .bodies()
            .iter()
            .filter_map(|body| {
                let tip = body.tip?;
                let seq = feature_tree.get_node(tip)?.seq;
                Some((body.id, (tip, seq)))
            })
            .collect();

        // First, build subtrees for all root features.
        for &root_id in feature_tree.roots() {
            if let Some(node) = feature_tree.get_node(root_id) {
                let body = node.body;
                let tree_node =
                    build_feature_node(feature_tree, node, &mut visited, &tip_seq_by_body);
                push_root(body, tree_node, &mut roots_by_body);
            }
        }

        // Then, include any remaining nodes that weren't reachable from roots
        // (defensive: should be rare in a well-formed DAG).
        for (&id, node) in feature_tree.all_nodes() {
            if !visited.contains(&id) {
                let body = node.body;
                let tree_node =
                    build_feature_node(feature_tree, node, &mut visited, &tip_seq_by_body);
                push_root(body, tree_node, &mut roots_by_body);
            }
        }

        // Sort feature roots within each body group by insertion order (the
        // history ordering key; creation times have millisecond ties).
        for nodes in roots_by_body.values_mut() {
            nodes.sort_by_key(|n| (n.seq, n.id));
        }

        // Build body nodes and attach their feature subtrees. Bodies represented
        // in imported-object hierarchy are shown there instead to avoid duplicates.
        let mut body_nodes: Vec<TreeNode> = document
            .bodies()
            .iter()
            .filter(|body| document.imported_object_for_body(body.id).is_none())
            .map(|body| {
                let mut node = build_body_node(body);
                if let Some(children) = roots_by_body.remove(&Some(body.id)) {
                    node.children = children;
                }
                node
            })
            .collect();

        for &root in document.imported_object_roots() {
            if let Some(node) = build_imported_node(document, root) {
                body_nodes.push(node);
            }
        }

        // Any remaining roots without a body (or with unknown body IDs) are appended at the end.
        if let Some(mut doc_level) = roots_by_body.remove(&None) {
            body_nodes.append(&mut doc_level);
        }
        for (_key, mut nodes) in roots_by_body {
            body_nodes.append(&mut nodes);
        }

        Self {
            document_label: document.name().to_string(),
            nodes: body_nodes,
        }
    }

    pub fn document_label(&self) -> &str {
        &self.document_label
    }

    fn nodes(&self) -> &[TreeNode] {
        &self.nodes
    }
}

fn build_feature_node(
    feature_tree: &FeatureTree,
    node: &FeatureNode,
    visited: &mut HashSet<FeatureId>,
    tip_seq_by_body: &HashMap<BodyId, (FeatureId, u64)>,
) -> TreeNode {
    visited.insert(node.id);

    let mut children = Vec::new();
    for child_id in feature_tree.dependents(node.id) {
        if visited.contains(&child_id) {
            continue;
        }
        if let Some(child) = feature_tree.get_node(child_id) {
            children.push(build_feature_node(
                feature_tree,
                child,
                visited,
                tip_seq_by_body,
            ));
        }
    }

    children.sort_by_key(|n| (n.seq, n.id));

    let tip = node.body.and_then(|b| tip_seq_by_body.get(&b));
    let is_tip = tip.map(|(id, _)| *id == node.id).unwrap_or(false);
    let after_tip = tip.map(|(_, seq)| node.seq > *seq).unwrap_or(false);

    TreeNode {
        id: TreeItemId::Feature(node.id),
        label: node.name.clone(),
        badge: Some(format_workbench_tag(node.workbench_id.as_str())),
        tooltip: Some(feature_tooltip(node, after_tip)),
        dirty: node.dirty,
        visible: node.visible,
        suppressed: node.suppressed,
        error: node.error.clone(),
        is_tip,
        after_tip,
        feature_menu: Some(node.id),
        seq: node.seq,
        children,
        imported_object_id: None,
    }
}

fn build_body_node(body: &Body) -> TreeNode {
    TreeNode {
        id: TreeItemId::Body(body.id),
        label: body.name.clone(),
        badge: None,
        tooltip: None,
        dirty: false,
        visible: true,
        suppressed: false,
        error: None,
        is_tip: false,
        after_tip: false,
        feature_menu: None,
        seq: 0,
        children: Vec::new(),
        imported_object_id: None,
    }
}

fn build_imported_node(document: &Document, id: Uuid) -> Option<TreeNode> {
    let imported = document.imported_object(id)?;
    let mut children = Vec::new();
    for child_id in &imported.children {
        if let Some(child) = build_imported_node(document, *child_id) {
            children.push(child);
        }
    }
    let label = if imported.name.is_empty() {
        "Imported".to_string()
    } else {
        imported.name.clone()
    };
    let kind_badge = match imported.kind {
        kernel_api::ImportedNodeKind::Assembly => Some("asm".to_string()),
        kernel_api::ImportedNodeKind::Part => Some("part".to_string()),
        kernel_api::ImportedNodeKind::Instance => Some("inst".to_string()),
    };
    Some(TreeNode {
        id: TreeItemId::ImportedObject(imported.id),
        label,
        badge: kind_badge,
        tooltip: imported
            .body_id
            .map(|body| format!("Linked body: {}", body.0)),
        dirty: false,
        visible: imported.visible,
        suppressed: false,
        error: None,
        is_tip: false,
        after_tip: false,
        feature_menu: None,
        seq: 0,
        children,
        imported_object_id: Some(imported.id),
    })
}

fn format_workbench_tag(raw: &str) -> String {
    raw.trim_start_matches("wb.").replace(['-', '_'], " ")
}

pub fn draw_tree(ui: &mut Ui, model: &DocumentTree, selected: Option<TreeItemId>) -> TreeUiResult {
    let mut result = TreeUiResult::default();

    // Document root behaves like a top-level collapsible item.
    let header_text = format!("Document: {}", model.document_label());
    let collapsing = egui::CollapsingHeader::new(header_text)
        .id_salt("document_root")
        .show(ui, |ui| {
            for node in model.nodes() {
                draw_node(ui, node, 0, selected, &mut result);
            }
        });
    handle_response(
        collapsing.header_response,
        TreeItemId::DocumentRoot,
        &mut result,
    );

    result
}

fn draw_node(
    ui: &mut Ui,
    node: &TreeNode,
    depth: usize,
    selected: Option<TreeItemId>,
    result: &mut TreeUiResult,
) {
    let indent = (depth as f32) * 14.0;

    // Nodes with children are rendered as collapsible tree branches; leaves as simple rows.
    if node.children.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(indent);
            maybe_draw_imported_visibility_toggle(ui, node, result);
            let label = compose_label(node);
            let is_selected = selected == Some(node.id);
            let response = if let Some(tooltip) = &node.tooltip {
                ui.selectable_label(is_selected, label)
                    .on_hover_text(tooltip)
            } else {
                ui.selectable_label(is_selected, label)
            };
            let response = attach_feature_menu(response, node, result);
            handle_response(response, node.id, result);
        });
    } else {
        ui.horizontal(|ui| {
            ui.add_space(indent);
            maybe_draw_imported_visibility_toggle(ui, node, result);
            let label = compose_label(node);
            let collapsing = egui::CollapsingHeader::new(label)
                .id_salt(format!("tree_node_{:?}", node.id))
                .show(ui, |ui| {
                    for child in &node.children {
                        draw_node(ui, child, depth + 1, selected, result);
                    }
                });

            let response = attach_feature_menu(collapsing.header_response, node, result);
            handle_response(response, node.id, result);
        });
    }
}

/// History context menu on feature rows (right-click).
fn attach_feature_menu(response: Response, node: &TreeNode, result: &mut TreeUiResult) -> Response {
    let Some(feature_id) = node.feature_menu else {
        return response;
    };
    let mut command = None;
    response.context_menu(|ui| {
        let suppress_label = if node.suppressed {
            "Unsuppress"
        } else {
            "Suppress"
        };
        if ui.button(suppress_label).clicked() {
            command = Some(TreeFeatureCommand::Suppress(!node.suppressed));
            ui.close();
        }
        let visible_label = if node.visible { "Hide" } else { "Show" };
        if ui.button(visible_label).clicked() {
            command = Some(TreeFeatureCommand::SetVisible(!node.visible));
            ui.close();
        }
        ui.separator();
        if ui
            .button("Move up")
            .on_hover_text("Swap with the previous feature in the build history")
            .clicked()
        {
            command = Some(TreeFeatureCommand::MoveUp);
            ui.close();
        }
        if ui
            .button("Move down")
            .on_hover_text("Swap with the next feature in the build history")
            .clicked()
        {
            command = Some(TreeFeatureCommand::MoveDown);
            ui.close();
        }
        if node.is_tip {
            if ui
                .button("Clear tip")
                .on_hover_text("Expose the full history again")
                .clicked()
            {
                command = Some(TreeFeatureCommand::ClearTip);
                ui.close();
            }
        } else if ui
            .button("Set as tip")
            .on_hover_text("Preview the history up to this feature; later features are excluded")
            .clicked()
        {
            command = Some(TreeFeatureCommand::SetTip);
            ui.close();
        }
        ui.separator();
        if ui.button("Delete").clicked() {
            command = Some(TreeFeatureCommand::Delete);
            ui.close();
        }
    });
    if let Some(command) = command {
        result.feature_command = Some((feature_id, command));
    }
    response
}

fn handle_response(response: Response, id: TreeItemId, result: &mut TreeUiResult) {
    if response.clicked() {
        result.selection = Some(id);
    }
    if response.double_clicked() {
        result.activation = Some(id);
    }
}

fn maybe_draw_imported_visibility_toggle(ui: &mut Ui, node: &TreeNode, result: &mut TreeUiResult) {
    let Some(imported_id) = node.imported_object_id else {
        return;
    };
    let mut visible = node.visible;
    let resp = ui.checkbox(&mut visible, "");
    if resp.changed() {
        result.imported_visibility_change = Some((imported_id, visible));
    }
    resp.on_hover_text(if visible {
        "Hide imported node"
    } else {
        "Show imported node"
    });
}

fn compose_label(node: &TreeNode) -> RichText {
    let mut pieces = Vec::new();
    if node.error.is_some() {
        pieces.push("⚠".to_string());
    }
    if let Some(tag) = &node.badge {
        pieces.push(format!("[{}]", tag));
    }
    pieces.push(node.label.clone());
    if node.is_tip {
        pieces.push("◄ tip".into());
    }
    if node.dirty {
        pieces.push("•dirty".into());
    }
    let text = pieces.join(" ");

    let mut rich = RichText::new(text);
    if node.error.is_some() {
        rich = rich.color(Color32::from_rgb(240, 90, 90));
    } else if node.suppressed || !node.visible || node.after_tip {
        rich = rich.color(Color32::from_gray(150)).italics();
    }
    if node.suppressed {
        rich = rich.strikethrough();
    }
    rich
}

fn feature_tooltip(node: &FeatureNode, after_tip: bool) -> String {
    let mut parts = Vec::new();
    parts.push(format!(
        "Workbench: {}",
        format_workbench_tag(node.workbench_id.as_str())
    ));
    parts.push(format!("Visible: {}", node.visible));
    parts.push(format!("Suppressed: {}", node.suppressed));
    if node.dirty {
        parts.push("Pending recompute".into());
    }
    if after_tip {
        parts.push("After the tip: excluded from the build".into());
    }
    if let Some(error) = &node.error {
        parts.push(format!("Error: {error}"));
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_ids(nodes: &[TreeNode], out: &mut Vec<TreeItemId>) {
        for node in nodes {
            out.push(node.id);
            collect_ids(&node.children, out);
        }
    }

    #[test]
    fn imported_nodes_render_in_tree_without_duplicate_body_row() {
        let mut doc = Document::new("tree");
        let body_id = doc.create_body(Some("Imported Body".into()));
        let root = Uuid::new_v4();
        let leaf = Uuid::new_v4();
        let mut graph = std::collections::HashMap::new();
        graph.insert(
            root,
            core_document::ImportedObjectNode {
                id: root,
                parent_id: None,
                children: vec![leaf],
                kind: kernel_api::ImportedNodeKind::Assembly,
                name: "Asm".into(),
                visible: true,
                body_id: None,
                local_transform: None,
            },
        );
        graph.insert(
            leaf,
            core_document::ImportedObjectNode {
                id: leaf,
                parent_id: Some(root),
                children: Vec::new(),
                kind: kernel_api::ImportedNodeKind::Part,
                name: "Part".into(),
                visible: true,
                body_id: Some(body_id),
                local_transform: None,
            },
        );
        doc.set_imported_object_graph(vec![root], graph);

        let tree = DocumentTree::build(&doc);
        let mut ids = Vec::new();
        collect_ids(tree.nodes(), &mut ids);
        assert!(ids.contains(&TreeItemId::ImportedObject(root)));
        assert!(ids.contains(&TreeItemId::ImportedObject(leaf)));
        assert!(!ids.contains(&TreeItemId::Body(body_id)));
    }
}
