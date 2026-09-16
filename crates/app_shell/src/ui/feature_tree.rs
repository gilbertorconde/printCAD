use std::collections::{HashMap, HashSet};

use core_document::{
    Body, BodyId, Document, FeatureId, FeatureNode, FeatureTree, WorkbenchFeature,
};
use egui::{Response, Ui, Vec2};
use ui_kit::sans;
use ui_kit::tokens::*;
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
    /// The row under the pointer this frame — drives the details line, so
    /// a glance tells what something is without committing a click.
    pub hovered: Option<TreeItemId>,
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
    /// What this item is, spelled out ("Instance of assembly Frame",
    /// "Sketch feature"). Shown in the details line under the tree when the
    /// item is selected — never inline, where it only crowds the names.
    detail: Option<String>,
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
    /// The design set's icon for this item.
    icon: &'static str,
    /// Bodies and linked parts read their icon in accent.
    accent_icon: bool,
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

    /// The spelled-out description of a tree item, for the details line.
    pub fn detail_for(&self, id: TreeItemId) -> Option<String> {
        fn find(nodes: &[TreeNode], id: TreeItemId) -> Option<&TreeNode> {
            for node in nodes {
                if node.id == id {
                    return Some(node);
                }
                if let Some(found) = find(&node.children, id) {
                    return Some(found);
                }
            }
            None
        }
        if id == TreeItemId::DocumentRoot {
            return Some("Document".to_string());
        }
        find(&self.nodes, id).and_then(|n| n.detail.clone())
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
        detail: Some(describe_workbench(node.workbench_id.as_str())),
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
        icon: feature_icon(node),
        accent_icon: false,
    }
}

/// The icon a feature row draws: the part feature's own, the datum's
/// shape, or the sketch glyph.
fn feature_icon(node: &FeatureNode) -> &'static str {
    match node.workbench_id.as_str() {
        "wb.sketch" => "tree-sketch",
        "wb.part" => wb_part::PartFeature::from_json(&node.data)
            .map(|f| f.icon())
            .unwrap_or("tree-feature"),
        "core.datum" => core_document::DatumFeature::from_json(&node.data)
            .map(|d| match d.shape {
                core_document::DatumShape::Plane { .. } => "datum-plane",
                core_document::DatumShape::Line { .. } => "datum-line",
                core_document::DatumShape::Point => "datum-point",
            })
            .unwrap_or("datum-plane"),
        _ => "tree-feature",
    }
}

fn build_body_node(body: &Body) -> TreeNode {
    TreeNode {
        id: TreeItemId::Body(body.id),
        label: body.name.clone(),
        detail: Some("Body".to_string()),
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
        icon: "tree-body",
        accent_icon: true,
    }
}

fn kind_word(kind: kernel_api::ImportedNodeKind) -> &'static str {
    match kind {
        kernel_api::ImportedNodeKind::Assembly => "assembly",
        kernel_api::ImportedNodeKind::Part => "part",
        kernel_api::ImportedNodeKind::Instance => "instance",
    }
}

fn build_imported_node(document: &Document, id: Uuid) -> Option<TreeNode> {
    let imported = document.imported_object(id)?;

    // An instance whose only child is the product it instances is one thing
    // to the user, not two: show a single row named for the instance, with
    // the product's children hoisted under it. Selection and visibility keep
    // the instance's identity — hiding an instance hides that placement.
    if imported.kind == kernel_api::ImportedNodeKind::Instance
        && imported.children.len() == 1
        && let Some(target) = document.imported_object(imported.children[0])
        && target.kind != kernel_api::ImportedNodeKind::Instance
    {
        let mut merged = build_imported_node(document, target.id)?;
        merged.id = TreeItemId::ImportedObject(imported.id);
        merged.imported_object_id = Some(imported.id);
        merged.visible = imported.visible && target.visible;
        let instance_name = imported.name.trim();
        if !instance_name.is_empty() && instance_name != "Instance" {
            merged.label = imported.name.clone();
        }
        merged.detail = Some(format!(
            "Instance of {} {}",
            kind_word(target.kind),
            target.name
        ));
        return Some(merged);
    }

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
    let detail = match imported.kind {
        kernel_api::ImportedNodeKind::Assembly => "Assembly".to_string(),
        kernel_api::ImportedNodeKind::Part if imported.body_id.is_some() => {
            "Part, linked to a body".to_string()
        }
        kernel_api::ImportedNodeKind::Part => "Part".to_string(),
        kernel_api::ImportedNodeKind::Instance => "Instance".to_string(),
    };
    Some(TreeNode {
        id: TreeItemId::ImportedObject(imported.id),
        label,
        detail: Some(detail),
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
        icon: match imported.kind {
            kernel_api::ImportedNodeKind::Assembly => "tree-group",
            kernel_api::ImportedNodeKind::Part => "tree-body",
            kernel_api::ImportedNodeKind::Instance => "tree-feature",
        },
        accent_icon: imported.body_id.is_some(),
    })
}

pub(crate) fn describe_workbench(raw: &str) -> String {
    match raw {
        "wb.sketch" => "Sketch".to_string(),
        "wb.part" => "Part design feature".to_string(),
        "core.datum" => "Datum".to_string(),
        other => format!(
            "{} feature",
            other.trim_start_matches("wb.").replace(['-', '_'], " ")
        ),
    }
}

/// What the tree draws with this frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct TreeDrawOptions<'a> {
    pub selected: Option<TreeItemId>,
    /// The feature whose edit session is open: badged `EDITING`, and every
    /// other row dims.
    pub editing: Option<FeatureId>,
    /// Case-insensitive substring over labels; a branch stays visible when
    /// any descendant matches.
    pub filter: &'a str,
}

fn matches_filter(node: &TreeNode, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    let needle = filter.to_lowercase();
    node.label.to_lowercase().contains(&needle)
        || node.children.iter().any(|c| matches_filter(c, filter))
}

pub fn draw_tree(ui: &mut Ui, model: &DocumentTree, options: TreeDrawOptions<'_>) -> TreeUiResult {
    let mut result = TreeUiResult::default();
    ui.spacing_mut().item_spacing.y = 0.0;

    let root = RowSpec {
        id: TreeItemId::DocumentRoot,
        depth: 0,
        icon: "tree-document",
        icon_tint: TEXT1,
        label: model.document_label(),
        has_children: !model.nodes().is_empty(),
        muted: false,
        strikethrough: false,
        badges: Vec::new(),
        eye: None,
        tooltip: None,
    };
    let open = draw_row(ui, &root, &options, &mut result, None);
    if open {
        for node in model.nodes() {
            if matches_filter(node, options.filter) {
                draw_node(ui, node, 1, &options, &mut result);
            }
        }
    }
    result
}

/// A badge at the row's end.
struct Badge {
    text: &'static str,
    color: egui::Color32,
    tooltip: Option<String>,
}

/// Everything one row draws.
struct RowSpec<'a> {
    id: TreeItemId,
    depth: usize,
    icon: &'static str,
    icon_tint: egui::Color32,
    label: &'a str,
    has_children: bool,
    /// Hidden, suppressed or past the tip: drawn in the muted text color.
    muted: bool,
    strikethrough: bool,
    badges: Vec<Badge>,
    /// Visibility toggle at the row's end: `Some(visible)`.
    eye: Option<bool>,
    tooltip: Option<&'a str>,
}

const ROW_FONT: f32 = 12.5;

fn open_state(ui: &Ui, id: TreeItemId) -> bool {
    ui.data(|d| d.get_temp::<bool>(egui::Id::new(("tree_open", id))))
        .unwrap_or(true)
}

fn set_open_state(ui: &Ui, id: TreeItemId, open: bool) {
    ui.data_mut(|d| d.insert_temp(egui::Id::new(("tree_open", id)), open));
}

fn paint_icon(ui: &Ui, name: &str, rect: egui::Rect, tint: egui::Color32) {
    if let Some(tex) = ui_kit::icon::texture(ui.ctx(), name) {
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().image(tex.id(), rect, uv, tint);
    }
}

/// Draw one row; returns whether its children are shown. `node` carries
/// the feature context menu when the row is a feature.
fn draw_row(
    ui: &mut Ui,
    spec: &RowSpec<'_>,
    options: &TreeDrawOptions<'_>,
    result: &mut TreeUiResult,
    node: Option<&TreeNode>,
) -> bool {
    let selected = options.selected == Some(spec.id);
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), TREE_ROW),
        egui::Sense::click(),
    );
    if selected {
        ui.painter().rect_filled(rect, 0.0, ACCENT_DIM);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.left_top(), Vec2::new(2.0, rect.height())),
            0.0,
            ACCENT,
        );
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, BG2);
    }

    let mut x = rect.left() + 8.0 + spec.depth as f32 * 16.0;
    let cy = rect.center().y;

    // The chevron toggles the branch without selecting the row.
    let mut open = open_state(ui, spec.id);
    if spec.has_children {
        let chevron_rect = egui::Rect::from_center_size(egui::pos2(x + 6.0, cy), Vec2::splat(12.0));
        let chevron = ui.interact(
            chevron_rect.expand(3.0),
            ui.id().with(("tree_chevron", spec.id)),
            egui::Sense::click(),
        );
        if chevron.clicked() {
            open = !open;
            set_open_state(ui, spec.id, open);
        }
        let name = if open {
            "chevron-down"
        } else {
            "chevron-right"
        };
        paint_icon(ui, name, chevron_rect, TEXT3);
    }
    x += 18.0;

    let icon_rect = egui::Rect::from_center_size(egui::pos2(x + 8.0, cy), Vec2::splat(16.0));
    let tint = if spec.muted { TEXT3 } else { spec.icon_tint };
    paint_icon(ui, spec.icon, icon_rect, tint);
    x += 22.0;

    // Badges and the eye claim the right end first; the label gets what is
    // left.
    let mut right = rect.right() - 8.0;
    if let Some(visible) = spec.eye {
        let eye_rect = egui::Rect::from_center_size(egui::pos2(right - 7.0, cy), Vec2::splat(14.0));
        let eye = ui.interact(
            eye_rect.expand(3.0),
            ui.id().with(("tree_eye", spec.id)),
            egui::Sense::click(),
        );
        let shown = !visible || eye.hovered() || response.hovered() || selected;
        if shown {
            let name = if visible { "eye" } else { "eye-off" };
            paint_icon(ui, name, eye_rect, TEXT3);
        }
        let eye = eye.on_hover_text(if visible { "Hide" } else { "Show" });
        if eye.clicked() {
            match spec.id {
                TreeItemId::ImportedObject(id) => {
                    result.imported_visibility_change = Some((id, !visible));
                }
                TreeItemId::Feature(id) => {
                    result.feature_command = Some((id, TreeFeatureCommand::SetVisible(!visible)));
                }
                _ => {}
            }
        }
        right -= 20.0;
    }
    for badge in spec.badges.iter().rev() {
        let galley = ui.painter().layout_no_wrap(
            badge.text.to_string(),
            ui_kit::sans_semibold(10.0),
            badge.color,
        );
        let w = galley.size().x + 8.0;
        let badge_rect =
            egui::Rect::from_min_max(egui::pos2(right - w, cy - 7.0), egui::pos2(right, cy + 7.0));
        ui.painter().rect_stroke(
            badge_rect,
            RADIUS_SM,
            egui::Stroke::new(1.0, with_alpha(badge.color, 0.4)),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(
            egui::pos2(badge_rect.left() + 4.0, cy - galley.size().y / 2.0),
            galley,
            badge.color,
        );
        if let Some(tip) = &badge.tooltip {
            ui.interact(
                badge_rect,
                ui.id().with(("tree_badge", spec.id, badge.text)),
                egui::Sense::hover(),
            )
            .on_hover_text(tip);
        }
        right -= w + 6.0;
    }

    let label_rect =
        egui::Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(right, rect.bottom()));
    let color = if spec.muted { TEXT3 } else { TEXT1 };
    let galley = ui
        .painter()
        .layout_no_wrap(spec.label.to_string(), sans(ROW_FONT), color);
    let text_width = galley.size().x.min(label_rect.width());
    ui.painter().with_clip_rect(label_rect).galley(
        egui::pos2(label_rect.left(), cy - galley.size().y / 2.0),
        galley,
        color,
    );
    if spec.strikethrough {
        ui.painter().line_segment(
            [egui::pos2(x, cy), egui::pos2(x + text_width, cy)],
            egui::Stroke::new(1.0, color),
        );
    }

    let response = match spec.tooltip {
        Some(tip) => response.on_hover_text(tip),
        None => response,
    };
    let response = match node {
        Some(node) => attach_feature_menu(response, node, result),
        None => response,
    };
    handle_response(response, spec.id, result);
    open
}

fn draw_node(
    ui: &mut Ui,
    node: &TreeNode,
    depth: usize,
    options: &TreeDrawOptions<'_>,
    result: &mut TreeUiResult,
) {
    let editing_here =
        matches!((node.id, options.editing), (TreeItemId::Feature(a), Some(b)) if a == b);
    let mut badges = Vec::new();
    if node.is_tip {
        badges.push(Badge {
            text: "TIP",
            color: SUCCESS,
            tooltip: Some("The body's shape stops at this feature".to_string()),
        });
    }
    if editing_here {
        badges.push(Badge {
            text: "EDITING",
            color: ACCENT,
            tooltip: None,
        });
    }
    if node.dirty {
        badges.push(Badge {
            text: "…",
            color: TEXT3,
            tooltip: Some("Pending recompute".to_string()),
        });
    }
    if let Some(error) = &node.error {
        badges.push(Badge {
            text: "!",
            color: DANGER,
            tooltip: Some(error.clone()),
        });
    }
    let dimmed_by_edit = options.editing.is_some() && !editing_here;
    let eye = match node.id {
        TreeItemId::ImportedObject(_) | TreeItemId::Feature(_) => Some(node.visible),
        _ => None,
    };
    let icon_tint = if editing_here || node.accent_icon {
        ACCENT
    } else if node.error.is_some() {
        DANGER
    } else if node.is_tip {
        SUCCESS
    } else {
        TEXT2
    };
    let spec = RowSpec {
        id: node.id,
        depth,
        icon: node.icon,
        icon_tint,
        label: &node.label,
        has_children: !node.children.is_empty(),
        muted: node.suppressed || !node.visible || node.after_tip || dimmed_by_edit,
        strikethrough: node.suppressed,
        badges,
        eye,
        tooltip: node.tooltip.as_deref(),
    };
    let open = draw_row(ui, &spec, options, result, Some(node));
    if open {
        for child in &node.children {
            if matches_filter(child, options.filter) {
                draw_node(ui, child, depth + 1, options, result);
            }
        }
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
    if response.hovered() {
        result.hovered = Some(id);
    }
    if response.clicked() {
        result.selection = Some(id);
    }
    if response.double_clicked() {
        result.activation = Some(id);
    }
}

fn feature_tooltip(node: &FeatureNode, after_tip: bool) -> String {
    let mut parts = Vec::new();
    parts.push(describe_workbench(node.workbench_id.as_str()));
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

    fn node(
        id: Uuid,
        parent: Option<Uuid>,
        children: Vec<Uuid>,
        kind: kernel_api::ImportedNodeKind,
        name: &str,
    ) -> core_document::ImportedObjectNode {
        core_document::ImportedObjectNode {
            id,
            parent_id: parent,
            children,
            kind,
            name: name.into(),
            visible: true,
            body_id: None,
            local_transform: None,
        }
    }

    /// An instance whose only child is the assembly it instances shows as
    /// ONE row — named for the instance, carrying the assembly's children,
    /// keeping the instance's identity — and the row says what it is.
    #[test]
    fn an_instance_and_its_product_collapse_into_one_row() {
        use kernel_api::ImportedNodeKind as K;
        let mut doc = Document::new("tree");
        let (root, inst, asm, leaf_a, leaf_b) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let mut graph = std::collections::HashMap::new();
        graph.insert(root, node(root, None, vec![inst], K::Assembly, "Top"));
        graph.insert(
            inst,
            node(inst, Some(root), vec![asm], K::Instance, "Frame:1"),
        );
        graph.insert(
            asm,
            node(asm, Some(inst), vec![leaf_a, leaf_b], K::Assembly, "Frame"),
        );
        graph.insert(
            leaf_a,
            node(leaf_a, Some(asm), vec![], K::Instance, "Vertical1:1"),
        );
        graph.insert(
            leaf_b,
            node(leaf_b, Some(asm), vec![], K::Instance, "Vertical2:1"),
        );
        doc.set_imported_object_graph(vec![root], graph);

        let tree = DocumentTree::build(&doc);
        let top = &tree.nodes()[0];
        assert_eq!(top.children.len(), 1, "Top holds one merged row");
        let row = &top.children[0];
        assert_eq!(
            row.id,
            TreeItemId::ImportedObject(inst),
            "row keeps the instance id"
        );
        assert_eq!(row.label, "Frame:1", "named for the instance");
        assert_eq!(row.children.len(), 2, "the assembly's children are hoisted");
        assert_eq!(
            row.detail.as_deref(),
            Some("Instance of assembly Frame"),
            "the kind is spelled out for the details line"
        );
        let mut ids = Vec::new();
        collect_ids(tree.nodes(), &mut ids);
        assert!(
            !ids.contains(&TreeItemId::ImportedObject(asm)),
            "the assembly node no longer appears as its own row"
        );
        assert_eq!(
            tree.detail_for(TreeItemId::ImportedObject(inst)).as_deref(),
            Some("Instance of assembly Frame")
        );
    }

    /// A generically named instance takes its product's name.
    #[test]
    fn a_generic_instance_borrows_its_products_name() {
        use kernel_api::ImportedNodeKind as K;
        let mut doc = Document::new("tree");
        let (root, inst, part) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let mut graph = std::collections::HashMap::new();
        graph.insert(root, node(root, None, vec![inst], K::Assembly, "Top"));
        graph.insert(
            inst,
            node(inst, Some(root), vec![part], K::Instance, "Instance"),
        );
        graph.insert(
            part,
            node(part, Some(inst), vec![], K::Part, "Anet v1-body"),
        );
        doc.set_imported_object_graph(vec![root], graph);

        let tree = DocumentTree::build(&doc);
        let row = &tree.nodes()[0].children[0];
        assert_eq!(row.label, "Anet v1-body");
        assert_eq!(row.detail.as_deref(), Some("Instance of part Anet v1-body"));
    }

    /// Bracket tags are gone from labels: a label is just the name.
    #[test]
    fn labels_carry_no_kind_tags() {
        let mut doc = Document::new("tree");
        let root = Uuid::new_v4();
        let mut graph = std::collections::HashMap::new();
        graph.insert(
            root,
            node(
                root,
                None,
                vec![],
                kernel_api::ImportedNodeKind::Assembly,
                "Asm",
            ),
        );
        doc.set_imported_object_graph(vec![root], graph);
        let tree = DocumentTree::build(&doc);
        assert_eq!(tree.nodes()[0].label, "Asm");
        assert_eq!(tree.nodes()[0].icon, "tree-group");
    }
}
