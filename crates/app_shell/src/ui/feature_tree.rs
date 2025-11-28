use std::collections::{HashMap, HashSet};

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
        let mut roots_by_body: HashMap<Option<BodyId>, Vec<TreeNode>> = HashMap::new();

        // Helper to group feature roots under their owning body (or None for document-level).
        let mut push_root =
            |body: Option<BodyId>,
             node: TreeNode,
             map: &mut HashMap<Option<BodyId>, Vec<TreeNode>>| {
                map.entry(body).or_default().push(node);
            };

        // First, build subtrees for all root features.
        for &root_id in feature_tree.roots() {
            if let Some(node) = feature_tree.get_node(root_id) {
                let body = node.body;
                let tree_node = build_feature_node(feature_tree, node, &mut visited);
                push_root(body, tree_node, &mut roots_by_body);
            }
        }

        // Then, include any remaining nodes that weren't reachable from roots
        // (defensive: should be rare in a well-formed DAG).
        for (&id, node) in feature_tree.all_nodes() {
            if !visited.contains(&id) {
                let body = node.body;
                let tree_node = build_feature_node(feature_tree, node, &mut visited);
                push_root(body, tree_node, &mut roots_by_body);
            }
        }

        // Sort feature roots within each body group by creation time.
        for nodes in roots_by_body.values_mut() {
            nodes.sort_by_key(|n| n.created_at_ms);
        }

        // Build body nodes and attach their feature subtrees.
        let mut body_nodes: Vec<TreeNode> = document
            .bodies()
            .iter()
            .map(|body| {
                let mut node = build_body_node(body);
                if let Some(children) = roots_by_body.remove(&Some(body.id)) {
                    node.children = children;
                }
                node
            })
            .collect();

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

    // Document root behaves like a top-level collapsible item.
    let is_doc_selected = selected == Some(TreeItemId::DocumentRoot);
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
    } else {
        ui.horizontal(|ui| {
            ui.add_space(indent);
            let label = compose_label(node);
            let is_selected = selected == Some(node.id);

            let collapsing = egui::CollapsingHeader::new(label)
                .id_salt(format!("tree_node_{:?}", node.id))
                .show(ui, |ui| {
                    for child in &node.children {
                        draw_node(ui, child, depth + 1, selected, result);
                    }
                });

            handle_response(collapsing.header_response, node.id, result);
        });
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
