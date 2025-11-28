use std::collections::HashSet;

use core_document::{Body, BodyId, Document, FeatureId, FeatureNode, FeatureTree};
use egui::{Color32, Response, RichText, Ui};

/// Identifier for selectable items in the tree panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TreeItemId {
    DocumentRoot,
    Body(BodyId),
    Feature(FeatureId),
}

impl From<FeatureId> for TreeItemId {
    fn from(value: FeatureId) -> Self {
        TreeItemId::Feature(value)
    }
}

#[derive(Debug, Default)]
pub struct TreeUiResult {
    pub selection: Option<TreeItemId>,
    pub activation: Option<TreeItemId>,
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
    created_at_ms: i64,
    children: Vec<TreeNode>,
}

impl DocumentTree {
    pub fn build(document: &Document) -> Self {
        let feature_tree = document.feature_tree();
        let mut visited = HashSet::new();

        let mut body_nodes: Vec<TreeNode> = document.bodies().iter().map(build_body_node).collect();
        body_nodes.sort_by_key(|n| n.created_at_ms);

        let mut feature_nodes = Vec::new();
        for &root_id in feature_tree.roots() {
            if let Some(node) = feature_tree.get_node(root_id) {
                feature_nodes.push(build_feature_node(feature_tree, node, &mut visited));
            }
        }

        for (&id, node) in feature_tree.all_nodes() {
            if !visited.contains(&id) {
                feature_nodes.push(build_feature_node(feature_tree, node, &mut visited));
            }
        }
        feature_nodes.sort_by_key(|n| n.created_at_ms);

        body_nodes.extend(feature_nodes);

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
) -> TreeNode {
    visited.insert(node.id);

    let mut children = Vec::new();
    for child_id in feature_tree.dependents(node.id) {
        if visited.contains(&child_id) {
            continue;
        }
        if let Some(child) = feature_tree.get_node(child_id) {
            children.push(build_feature_node(feature_tree, child, visited));
        }
    }

    children.sort_by_key(|n| n.created_at_ms);

    TreeNode {
        id: TreeItemId::Feature(node.id),
        label: node.name.clone(),
        badge: Some(format_workbench_tag(node.workbench_id.as_str())),
        tooltip: Some(feature_tooltip(node)),
        dirty: node.dirty,
        visible: node.visible,
        suppressed: node.suppressed,
        created_at_ms: node.created_at,
        children,
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
        created_at_ms: body.created_at,
        children: Vec::new(),
    }
}

fn format_workbench_tag(raw: &str) -> String {
    raw.trim_start_matches("wb.")
        .replace('-', " ")
        .replace('_', " ")
}

pub fn draw_tree(ui: &mut Ui, model: &DocumentTree, selected: Option<TreeItemId>) -> TreeUiResult {
    let mut result = TreeUiResult::default();

    let doc_response = ui.selectable_label(
        selected == Some(TreeItemId::DocumentRoot),
        format!("Document: {}", model.document_label()),
    );
    handle_response(doc_response, TreeItemId::DocumentRoot, &mut result);

    ui.separator();

    for node in model.nodes() {
        draw_node(ui, node, 0, selected, &mut result);
    }

    result
}

fn draw_node(
    ui: &mut Ui,
    node: &TreeNode,
    depth: usize,
    selected: Option<TreeItemId>,
    result: &mut TreeUiResult,
) {
    ui.horizontal(|ui| {
        ui.add_space((depth as f32) * 14.0);
        let label = compose_label(node);
        let is_selected = selected == Some(node.id);
        let response = if let Some(tooltip) = &node.tooltip {
            ui.selectable_label(is_selected, label)
                .on_hover_text(tooltip)
        } else {
            ui.selectable_label(is_selected, label)
        };
        handle_response(response, node.id, result);
    });

    for child in &node.children {
        draw_node(ui, child, depth + 1, selected, result);
    }
}

fn handle_response(response: Response, id: TreeItemId, result: &mut TreeUiResult) {
    if response.clicked() {
        result.selection = Some(id);
    }
    if response.double_clicked() {
        result.activation = Some(id);
    }
}

fn compose_label(node: &TreeNode) -> RichText {
    let mut pieces = Vec::new();
    if let Some(tag) = &node.badge {
        pieces.push(format!("[{}]", tag));
    }
    pieces.push(node.label.clone());
    if node.dirty {
        pieces.push("•dirty".into());
    }
    let text = pieces.join(" ");

    let mut rich = RichText::new(text);
    if node.suppressed || !node.visible {
        rich = rich.color(Color32::from_gray(150)).italics();
    }
    rich
}

fn feature_tooltip(node: &FeatureNode) -> String {
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
    parts.join("\n")
}
