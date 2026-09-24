use std::collections::{HashMap, HashSet};

use core_document::{
    Body, BodyId, Document, DocumentService, FeatureId, FeatureInfo, FeatureNode, FeatureTree,
    MenuScope, WorkbenchId,
};
use egui::{Response, Ui, Vec2};
use ui_kit::sans;
use ui_kit::tokens::*;
use uuid::Uuid;

use super::keymap::Keymap;

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
    /// A body row's eye was clicked: the body, and whether it is to show.
    pub body_visibility_change: Option<(BodyId, bool)>,
    pub feature_command: Option<(FeatureId, TreeFeatureCommand)>,
    /// The row the user asked to delete (menu or the Delete key).
    pub delete_item: Option<TreeItemId>,
    /// A bench's own menu entry was picked: the bench, its id, the scope.
    pub bench_command: Option<(WorkbenchId, String, MenuScope)>,
    /// "Repair shape" was picked: the broken bodies at or below the row.
    pub repair: Option<Vec<BodyId>>,
    /// "Convert to solid" was picked: the mesh bodies at or below the row.
    pub convert: Option<Vec<BodyId>>,
    /// A row's "!" was clicked: its label and the full message, to show
    /// where it can be read at length and copied.
    pub details: Option<(String, String)>,
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
    /// The kernel's checker calls this body's shape broken, or one below
    /// this row: the row draws in the danger colour.
    defect: bool,
    /// Bodies at or below this row whose shape is broken and not yet sent
    /// for repair: what the row's "Repair" entry acts on.
    repairable: Vec<BodyId>,
    /// Mesh bodies at or below this row not yet sent for conversion: what
    /// the row's "Convert to solid" entry acts on.
    convertible: Vec<BodyId>,
    /// The row is a mesh body: its icon takes the mesh colour.
    mesh: bool,
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
    /// The body this row stands for, when it stands for one. An imported
    /// part merged into its instance keeps the part's body here, which is
    /// what lets a face picked in the viewport find this row.
    body: Option<BodyId>,
    /// The design set's icon for this item.
    icon: &'static str,
    /// Bodies and linked parts read their icon in accent.
    accent_icon: bool,
}

impl DocumentTree {
    pub fn build(document: &Document, registry: &DocumentService) -> Self {
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
                let tree_node = build_feature_node(
                    feature_tree,
                    node,
                    &mut visited,
                    &tip_seq_by_body,
                    registry,
                );
                push_root(body, tree_node, &mut roots_by_body);
            }
        }

        // Then, include any remaining nodes that weren't reachable from roots
        // (defensive: should be rare in a well-formed DAG).
        for (&id, node) in feature_tree.all_nodes() {
            if !visited.contains(&id) {
                let body = node.body;
                let tree_node = build_feature_node(
                    feature_tree,
                    node,
                    &mut visited,
                    &tip_seq_by_body,
                    registry,
                );
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

        mark_shape_health(&mut body_nodes, document);
        mark_meshes(&mut body_nodes, document);

        Self {
            document_label: document.name().to_string(),
            nodes: body_nodes,
        }
    }

    /// The spelled-out description of a tree item, for the details line.
    /// The row that stands for `body`, with every row above it, root first.
    ///
    /// A pick in the viewport names a body; the tree is the only thing that
    /// knows which row shows it, since an imported part merged into its
    /// instance answers to the instance's id.
    pub fn path_to_body(&self, body: BodyId) -> Option<Vec<TreeItemId>> {
        fn walk(nodes: &[TreeNode], body: BodyId, trail: &mut Vec<TreeItemId>) -> bool {
            for node in nodes {
                trail.push(node.id);
                if node.body == Some(body) || walk(&node.children, body, trail) {
                    return true;
                }
                trail.pop();
            }
            false
        }
        let mut trail = Vec::new();
        walk(&self.nodes, body, &mut trail).then_some(trail)
    }

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
    registry: &DocumentService,
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
                registry,
            ));
        }
    }

    children.sort_by_key(|n| (n.seq, n.id));

    let tip = node.body.and_then(|b| tip_seq_by_body.get(&b));
    let is_tip = tip.map(|(id, _)| *id == node.id).unwrap_or(false);
    let after_tip = tip.map(|(_, seq)| node.seq > *seq).unwrap_or(false);

    let info = feature_info(registry, node);
    TreeNode {
        id: TreeItemId::Feature(node.id),
        label: node.name.clone(),
        detail: Some(info.family_label.clone()),
        tooltip: Some(feature_tooltip(node, &info.family_label, after_tip)),
        dirty: node.dirty,
        visible: node.visible,
        suppressed: node.suppressed,
        error: node.error.clone(),
        defect: false,
        repairable: Vec::new(),
        convertible: Vec::new(),
        mesh: false,
        is_tip,
        after_tip,
        feature_menu: Some(node.id),
        seq: node.seq,
        children,
        imported_object_id: None,
        body: None,
        icon: info.icon,
        accent_icon: false,
    }
}

/// How a feature row presents: what the bench that claimed its kind says,
/// or the plain fallback for a kind no bench claims.
pub(crate) fn feature_info(registry: &DocumentService, node: &FeatureNode) -> FeatureInfo {
    registry
        .feature_info(node)
        .unwrap_or_else(|| FeatureInfo::fallback(node))
}

fn build_body_node(body: &Body) -> TreeNode {
    TreeNode {
        id: TreeItemId::Body(body.id),
        label: body.name.clone(),
        detail: Some("Body".to_string()),
        tooltip: None,
        dirty: false,
        visible: !body.hidden,
        suppressed: false,
        error: None,
        defect: false,
        repairable: Vec::new(),
        convertible: Vec::new(),
        mesh: false,
        is_tip: false,
        after_tip: false,
        feature_menu: None,
        seq: 0,
        children: Vec::new(),
        imported_object_id: None,
        body: Some(body.id),
        icon: "tree-body",
        accent_icon: true,
    }
}

/// Mark the rows whose body the kernel's checker calls broken, and every
/// row above one: imported assemblies start closed, and a defect three
/// levels down would otherwise be out of sight. Returns how many defective
/// bodies lie at or below `nodes`.
fn mark_shape_health(nodes: &mut [TreeNode], document: &Document) -> usize {
    let mut total = 0;
    for node in nodes {
        let below = mark_shape_health(&mut node.children, document);
        let own = node.body.and_then(|body| {
            let health = document.imported_geometry(body)?.health.as_ref()?;
            health.is_broken().then_some((body, health))
        });
        let mut repairable: Vec<BodyId> = node
            .children
            .iter()
            .flat_map(|c| c.repairable.iter().copied())
            .collect();
        match own {
            Some((body, health)) => {
                let requested = document
                    .bodies()
                    .iter()
                    .any(|b| b.id == body && b.repair_requested);
                let next = if health.repaired {
                    ""
                } else if requested {
                    "\nRepairing…"
                } else {
                    "\nRight-click › Repair shape to run the kernel's repair"
                };
                node.error = Some(format!("{}{next}", health.describe()));
                node.defect = true;
                if !requested {
                    repairable.push(body);
                }
                total += below + 1;
            }
            None if below > 0 => {
                if node.error.is_none() {
                    node.error = Some(format!(
                        "{below} part(s) inside have shape defects the kernel's checker \
                         calls broken"
                    ));
                }
                node.defect = true;
                total += below;
            }
            None => {}
        }
        repairable.sort();
        repairable.dedup();
        node.repairable = repairable;
    }
    total
}

/// Say which rows are meshes, and gather the ones a row's "Convert to solid"
/// acts on: its own body and every mesh body below it not yet sent.
fn mark_meshes(nodes: &mut [TreeNode], document: &Document) {
    for node in nodes {
        mark_meshes(&mut node.children, document);
        let mut convertible: Vec<BodyId> = node
            .children
            .iter()
            .flat_map(|c| c.convertible.iter().copied())
            .collect();
        if let Some(body) = node.body
            && document.is_mesh_body(body)
        {
            let requested = document
                .bodies()
                .iter()
                .any(|b| b.id == body && b.solid_requested);
            node.icon = "workbench-mesh";
            node.mesh = true;
            node.detail = Some(if requested {
                "Mesh, converting to a solid".to_string()
            } else {
                "Mesh: draws and measures, takes features once converted to a solid".to_string()
            });
            if !requested {
                convertible.push(body);
            }
        }
        convertible.sort();
        convertible.dedup();
        node.convertible = convertible;
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
        defect: false,
        repairable: Vec::new(),
        convertible: Vec::new(),
        mesh: false,
        is_tip: false,
        after_tip: false,
        feature_menu: None,
        seq: 0,
        children,
        imported_object_id: Some(imported.id),
        body: imported.body_id,
        icon: match imported.kind {
            kernel_api::ImportedNodeKind::Assembly => "tree-group",
            kernel_api::ImportedNodeKind::Part => "tree-body",
            kernel_api::ImportedNodeKind::Instance => "tree-feature",
        },
        accent_icon: imported.body_id.is_some(),
    })
}

/// What the tree draws with this frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct TreeDrawOptions<'a> {
    pub selected: Option<TreeItemId>,
    /// A body picked in the viewport that the tree should jump to: its row
    /// is selected, every branch above it opens, and the list scrolls to it.
    pub reveal_body: Option<BodyId>,
    /// The feature whose edit session is open: badged `EDITING`, and every
    /// other row dims.
    pub editing: Option<FeatureId>,
    /// Case-insensitive substring over labels; a branch stays visible when
    /// any descendant matches.
    pub filter: &'a str,
    /// Resolved from `reveal_body` at the top of a draw; the row that should
    /// be brought into view.
    scroll_to: Option<TreeItemId>,
    /// The document and registry the row menus ask for bench entries.
    bench_menus: Option<(&'a Document, &'a DocumentService)>,
    /// The keys the row menus name beside their entries.
    keymap: Option<&'a Keymap>,
}

impl<'a> TreeDrawOptions<'a> {
    pub fn new(selected: Option<TreeItemId>, editing: Option<FeatureId>, filter: &'a str) -> Self {
        Self {
            selected,
            reveal_body: None,
            editing,
            filter,
            scroll_to: None,
            bench_menus: None,
            keymap: None,
        }
    }

    /// Name each menu entry's key beside it.
    pub fn with_keys(mut self, keymap: &'a Keymap) -> Self {
        self.keymap = Some(keymap);
        self
    }

    /// The key of the command `id`, as a menu names it.
    fn key(&self, id: &str) -> Option<String> {
        self.keymap.and_then(|k| k.text(id))
    }

    /// Let the benches add their own entries to the row menus.
    pub fn with_bench_menus(
        mut self,
        document: &'a Document,
        registry: &'a DocumentService,
    ) -> Self {
        self.bench_menus = Some((document, registry));
        self
    }

    /// Jump to the row for a body picked in the viewport.
    pub fn revealing(mut self, body: Option<BodyId>) -> Self {
        self.reveal_body = body;
        self
    }
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

    // A row hidden inside a closed branch cannot be scrolled to, so the
    // branches above it open first and the row names itself as the
    // selection — the same answer a click on it would give.
    let mut options = options;
    let mut scroll_to = None;
    if let Some(body) = options.reveal_body
        && let Some(trail) = model.path_to_body(body)
        && let Some((row, ancestors)) = trail.split_last()
    {
        for ancestor in ancestors {
            set_open_state(ui, *ancestor, true);
        }
        set_open_state(ui, TreeItemId::DocumentRoot, true);
        options.selected = Some(*row);
        result.selection = Some(*row);
        scroll_to = Some(*row);
    }
    let options = TreeDrawOptions {
        scroll_to,
        ..options
    };

    let root = RowSpec {
        id: TreeItemId::DocumentRoot,
        depth: 0,
        icon: "tree-document",
        icon_tint: TEXT1,
        label: model.document_label(),
        has_children: !model.nodes().is_empty(),
        muted: false,
        strikethrough: false,
        alert: false,
        badges: Vec::new(),
        eye: None,
        tooltip: None,
    };
    let open = draw_row(ui, &root, &options, &mut result, None);
    if open || !options.filter.is_empty() {
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
    /// A click opens the tooltip's text in a window of its own.
    opens_details: bool,
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
    /// Drawn in the danger colour: the row's shape, or one below it, is
    /// broken.
    alert: bool,
    badges: Vec<Badge>,
    /// Visibility toggle at the row's end: `Some(visible)`.
    eye: Option<bool>,
    tooltip: Option<&'a str>,
}

const ROW_FONT: f32 = 12.5;

/// Whether a branch shows its children. Until the user says otherwise, a
/// body and its features are open — that is the work in progress — and an
/// imported assembly is closed: a real-world STEP file is hundreds of parts,
/// and unfolding all of them would bury the rest of the tree.
fn open_state(ui: &Ui, id: TreeItemId) -> bool {
    ui.data(|d| d.get_temp::<bool>(egui::Id::new(("tree_open", id))))
        .unwrap_or(!matches!(id, TreeItemId::ImportedObject(_)))
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
    if options.scroll_to == Some(spec.id) {
        ui.scroll_to_rect(rect, Some(egui::Align::Center));
    }
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
                TreeItemId::Body(id) => {
                    result.body_visibility_change = Some((id, !visible));
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
            let sense = if badge.opens_details {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            };
            let response = ui
                .interact(
                    badge_rect,
                    ui.id().with(("tree_badge", spec.id, badge.text)),
                    sense,
                )
                .on_hover_text(if badge.opens_details {
                    format!("{tip}\n\nClick to open this in a window you can copy from")
                } else {
                    tip.clone()
                });
            if badge.opens_details && response.clicked() {
                result.details = Some((spec.label.to_string(), tip.clone()));
            }
        }
        right -= w + 6.0;
    }

    let label_rect =
        egui::Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(right, rect.bottom()));
    let color = if spec.alert {
        DANGER
    } else if spec.muted {
        TEXT3
    } else {
        TEXT1
    };
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
        Some(node) => attach_feature_menu(response, node, options, result),
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
            opens_details: false,
        });
    }
    if editing_here {
        badges.push(Badge {
            text: "EDITING",
            color: ACCENT,
            tooltip: None,
            opens_details: false,
        });
    }
    if node.dirty {
        badges.push(Badge {
            text: "…",
            color: TEXT3,
            tooltip: Some("Pending recompute".to_string()),
            opens_details: false,
        });
    }
    if let Some(error) = &node.error {
        badges.push(Badge {
            text: "!",
            color: DANGER,
            tooltip: Some(error.clone()),
            opens_details: true,
        });
    }
    let dimmed_by_edit = options.editing.is_some() && !editing_here;
    let eye = match node.id {
        TreeItemId::ImportedObject(_) | TreeItemId::Feature(_) | TreeItemId::Body(_) => {
            Some(node.visible)
        }
        _ => None,
    };
    let icon_tint = if editing_here {
        ACCENT
    } else if node.mesh {
        MESH
    } else if node.accent_icon {
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
        alert: node.defect,
        badges,
        eye,
        tooltip: node.tooltip.as_deref(),
    };
    let open = draw_row(ui, &spec, options, result, Some(node));
    // A closed branch stays closed to the eye, but not to a filter: what
    // matches is shown wherever it sits.
    if open || !options.filter.is_empty() {
        for child in &node.children {
            if matches_filter(child, options.filter) {
                draw_node(ui, child, depth + 1, options, result);
            }
        }
    }
}

/// Right-click menu: history actions on feature rows, Delete on anything
/// that can go.
fn attach_feature_menu(
    response: Response,
    node: &TreeNode,
    options: &TreeDrawOptions<'_>,
    result: &mut TreeUiResult,
) -> Response {
    let Some(feature_id) = node.feature_menu else {
        return attach_body_menu(response, node, options, result);
    };
    let mut command = None;
    let mut delete = false;
    let mut bench_command = None;
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
        if menu_entry(ui, visible_label, options.key("view.toggle_visibility")).clicked() {
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
        if menu_entry(ui, "Delete", options.key("edit.delete")).clicked() {
            delete = true;
            ui.close();
        }
        bench_command = bench_menu_entries(ui, options, MenuScope::TreeFeature(feature_id));
    });
    if delete {
        result.delete_item = Some(node.id);
    }
    if let Some(command) = command {
        result.feature_command = Some((feature_id, command));
    }
    if bench_command.is_some() {
        result.bench_command = bench_command;
    }
    response
}

/// A menu entry with its key, if it has one, beside it.
fn menu_entry(ui: &mut Ui, label: &str, key: Option<String>) -> Response {
    let mut button = egui::Button::new(label);
    if let Some(key) = key {
        button = button.shortcut_text(key);
    }
    ui.add(button)
}

/// The benches' entries for `scope`, after a separator when there are
/// any. The picked one, if any.
fn bench_menu_entries(
    ui: &mut Ui,
    options: &TreeDrawOptions<'_>,
    scope: MenuScope,
) -> Option<(WorkbenchId, String, MenuScope)> {
    let (document, registry) = options.bench_menus?;
    let items = registry.menu_items(&scope, document);
    if items.is_empty() {
        return None;
    }
    ui.separator();
    let mut picked = None;
    for (bench, item) in items {
        if item.separator_before {
            ui.separator();
        }
        let mut button = egui::Button::new(&item.label);
        if let Some(key) = options.key(&item.id) {
            button = button.shortcut_text(key);
        }
        let button = ui.add_enabled(item.enabled, button);
        let button = match &item.hint {
            Some(hint) => button.on_hover_text(hint),
            None => button,
        };
        if button.clicked() {
            picked = Some((bench, item.id, scope.clone()));
            ui.close();
        }
    }
    picked
}

/// Bodies and imported parts: select the body, or Delete, which takes the
/// body's features and geometry with it.
fn attach_body_menu(
    response: Response,
    node: &TreeNode,
    options: &TreeDrawOptions<'_>,
    result: &mut TreeUiResult,
) -> Response {
    if !matches!(node.id, TreeItemId::Body(_) | TreeItemId::ImportedObject(_)) {
        return response;
    }
    let mut select = false;
    let mut delete = false;
    let mut repair = false;
    let mut convert = false;
    let mut bench_command = None;
    response.context_menu(|ui| {
        if !node.convertible.is_empty() {
            let label = match node.convertible.len() {
                1 => "Convert to solid".to_string(),
                n => format!("Convert {n} meshes to solids"),
            };
            if ui
                .button(label)
                .on_hover_text(
                    "Build a B-rep solid from the mesh's triangles, flat regions merged \
                     into faces, so it can be measured, checked and cloned into a body \
                     for features. This clears the undo history.",
                )
                .clicked()
            {
                convert = true;
                ui.close();
            }
            ui.separator();
        }
        if !node.repairable.is_empty() {
            let label = match node.repairable.len() {
                1 => "Repair shape".to_string(),
                n => format!("Repair {n} shapes"),
            };
            if ui
                .button(label)
                .on_hover_text(
                    "Run the kernel's repair on the shapes its checker calls broken. \
                     This clears the undo history.",
                )
                .clicked()
            {
                repair = true;
                ui.close();
            }
            ui.separator();
        }
        if node.body.is_some() {
            if ui
                .button("Select body")
                .on_hover_text("Select the whole body in the viewport")
                .clicked()
            {
                select = true;
                ui.close();
            }
            ui.separator();
        }
        if menu_entry(ui, "Delete", options.key("edit.delete"))
            .on_hover_text("Remove this body, its features and its geometry")
            .clicked()
        {
            delete = true;
            ui.close();
        }
        if let Some(body) = node.body {
            bench_command = bench_menu_entries(ui, options, MenuScope::TreeBody(body));
        }
    });
    if select {
        result.selection = Some(node.id);
    }
    if delete {
        result.delete_item = Some(node.id);
    }
    if repair {
        result.repair = Some(node.repairable.clone());
    }
    if convert {
        result.convert = Some(node.convertible.clone());
    }
    if bench_command.is_some() {
        result.bench_command = bench_command;
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

fn feature_tooltip(node: &FeatureNode, family: &str, after_tip: bool) -> String {
    let mut parts = Vec::new();
    parts.push(family.to_string());
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

        let tree = DocumentTree::build(&doc, &DocumentService::default());
        let mut ids = Vec::new();
        collect_ids(tree.nodes(), &mut ids);
        assert!(ids.contains(&TreeItemId::ImportedObject(root)));
        assert!(ids.contains(&TreeItemId::ImportedObject(leaf)));
        assert!(!ids.contains(&TreeItemId::Body(body_id)));
    }

    /// A part whose shape the checker calls broken is red and offers its
    /// repair, and so is the closed assembly above it; once the repair is
    /// asked for, neither offers it again.
    #[test]
    fn a_broken_shape_marks_its_row_and_every_row_above_it() {
        let mut doc = Document::new("tree");
        let broken = doc.create_body(Some("Broken".into()));
        let sound = doc.create_body(Some("Sound".into()));
        let asset = doc.add_asset_with_data(
            core_document::AssetReference::new(
                "assets/x.step".to_string(),
                core_document::AssetType::Step,
                serde_json::json!({}),
            ),
            b"ISO-10303-21;".to_vec(),
        );
        for (body, broken_count) in [(broken, 2), (sound, 0)] {
            doc.set_imported_geometry(
                body,
                core_document::ImportedGeometry {
                    mesh: std::sync::Arc::new(kernel_api::TriMesh::default()),
                    source_asset: Some(asset),
                    revision: 0,
                    bounds_mm: None,
                    brep_blob_path: None,
                    face_colors_path: None,
                    health: Some(kernel_api::ShapeHealth {
                        broken: broken_count,
                        ..Default::default()
                    }),
                },
            );
        }
        let root = Uuid::new_v4();
        let (broken_leaf, sound_leaf) = (Uuid::new_v4(), Uuid::new_v4());
        let part = |id: Uuid, body: BodyId| core_document::ImportedObjectNode {
            id,
            parent_id: Some(root),
            children: Vec::new(),
            kind: kernel_api::ImportedNodeKind::Part,
            name: "Part".into(),
            visible: true,
            body_id: Some(body),
            local_transform: None,
        };
        let mut graph = std::collections::HashMap::new();
        graph.insert(
            root,
            core_document::ImportedObjectNode {
                id: root,
                parent_id: None,
                children: vec![broken_leaf, sound_leaf],
                kind: kernel_api::ImportedNodeKind::Assembly,
                name: "Asm".into(),
                visible: true,
                body_id: None,
                local_transform: None,
            },
        );
        graph.insert(broken_leaf, part(broken_leaf, broken));
        graph.insert(sound_leaf, part(sound_leaf, sound));
        doc.set_imported_object_graph(vec![root], graph);

        let find = |tree: &DocumentTree, id: Uuid| -> (bool, Vec<BodyId>) {
            fn walk(nodes: &[TreeNode], id: Uuid) -> Option<(bool, Vec<BodyId>)> {
                nodes.iter().find_map(|n| {
                    if n.id == TreeItemId::ImportedObject(id) {
                        Some((n.defect, n.repairable.clone()))
                    } else {
                        walk(&n.children, id)
                    }
                })
            }
            walk(tree.nodes(), id).expect("row exists")
        };
        let tree = DocumentTree::build(&doc, &DocumentService::default());
        assert_eq!(find(&tree, broken_leaf), (true, vec![broken]));
        assert_eq!(find(&tree, sound_leaf), (false, vec![]));
        assert_eq!(
            find(&tree, root),
            (true, vec![broken]),
            "the closed assembly shows what is inside it"
        );

        assert!(doc.request_body_repair(broken));
        let tree = DocumentTree::build(&doc, &DocumentService::default());
        assert_eq!(
            find(&tree, broken_leaf),
            (true, vec![]),
            "still red until the repair lands, and not offered twice"
        );
        assert_eq!(find(&tree, root), (true, vec![]));
    }

    /// A body from a mesh file says it is a mesh and offers its conversion,
    /// once; a body from a STEP file offers none.
    #[test]
    fn a_mesh_body_row_offers_its_conversion_once() {
        let mut doc = Document::new("tree");
        let mesh = doc.create_body(Some("Part".into()));
        let asset = doc.add_asset_with_data(
            core_document::AssetReference::new(
                "assets/part.stl".to_string(),
                core_document::AssetType::Stl,
                serde_json::json!({}),
            ),
            b"solid".to_vec(),
        );
        doc.set_imported_geometry(
            mesh,
            core_document::ImportedGeometry {
                mesh: std::sync::Arc::new(kernel_api::TriMesh::default()),
                source_asset: Some(asset),
                revision: 0,
                bounds_mm: None,
                brep_blob_path: None,
                face_colors_path: None,
                health: None,
            },
        );
        let row = |doc: &Document| {
            let tree = DocumentTree::build(doc, &DocumentService::default());
            let node = tree
                .nodes()
                .iter()
                .find(|n| n.id == TreeItemId::Body(mesh))
                .expect("the body's row");
            (
                node.detail.clone().unwrap_or_default(),
                node.convertible.clone(),
            )
        };
        let (detail, convertible) = row(&doc);
        assert!(detail.starts_with("Mesh"), "{detail}");
        let tree = DocumentTree::build(&doc, &DocumentService::default());
        let icon = tree
            .nodes()
            .iter()
            .find(|n| n.id == TreeItemId::Body(mesh))
            .map(|n| n.icon);
        assert_eq!(icon, Some("workbench-mesh"), "a mesh row has its own icon");
        assert!(
            tree.nodes()
                .iter()
                .any(|n| n.id == TreeItemId::Body(mesh) && n.mesh),
            "and its own colour"
        );
        assert_eq!(convertible, vec![mesh]);

        assert!(doc.request_mesh_solid(mesh));
        let (detail, convertible) = row(&doc);
        assert!(detail.contains("converting"), "{detail}");
        assert!(convertible.is_empty(), "not offered twice");
    }

    #[test]
    fn a_hidden_body_row_reads_as_hidden() {
        let mut doc = Document::new("tree");
        let body = doc.create_body(Some("Body".into()));
        let visible = |doc: &Document| {
            DocumentTree::build(doc, &DocumentService::default())
                .nodes()
                .iter()
                .find(|n| n.id == TreeItemId::Body(body))
                .map(|n| n.visible)
        };
        assert_eq!(visible(&doc), Some(true));
        doc.set_body_visible(body, false);
        assert_eq!(
            visible(&doc),
            Some(false),
            "its eye is shut and its row muted"
        );
    }

    #[test]
    fn a_plain_body_is_found_by_its_own_row() {
        let mut doc = Document::new("tree");
        let body = doc.create_body(Some("Body".into()));
        let tree = DocumentTree::build(&doc, &DocumentService::default());
        assert_eq!(
            tree.path_to_body(body),
            Some(vec![TreeItemId::Body(body)]),
            "a body with no import above it is its own row"
        );
    }

    #[test]
    fn an_imported_part_is_found_under_the_assembly_that_holds_it() {
        let mut doc = Document::new("tree");
        let body = doc.create_body(Some("Imported Body".into()));
        let root = Uuid::new_v4();
        let leaf = Uuid::new_v4();
        let mut graph = std::collections::HashMap::new();
        graph.insert(
            root,
            node(
                root,
                None,
                vec![leaf],
                kernel_api::ImportedNodeKind::Assembly,
                "Asm",
            ),
        );
        let mut part = node(
            leaf,
            Some(root),
            Vec::new(),
            kernel_api::ImportedNodeKind::Part,
            "Part",
        );
        part.body_id = Some(body);
        graph.insert(leaf, part);
        doc.set_imported_object_graph(vec![root], graph);

        let tree = DocumentTree::build(&doc, &DocumentService::default());
        assert_eq!(
            tree.path_to_body(body),
            Some(vec![
                TreeItemId::ImportedObject(root),
                TreeItemId::ImportedObject(leaf),
            ]),
            "the assembly above the part is on the way to it"
        );
    }

    #[test]
    fn a_part_merged_into_its_instance_answers_to_the_instance() {
        let mut doc = Document::new("tree");
        let body = doc.create_body(Some("Imported Body".into()));
        let asm = Uuid::new_v4();
        let instance = Uuid::new_v4();
        let part = Uuid::new_v4();
        let mut graph = std::collections::HashMap::new();
        graph.insert(
            asm,
            node(
                asm,
                None,
                vec![instance],
                kernel_api::ImportedNodeKind::Assembly,
                "Asm",
            ),
        );
        graph.insert(
            instance,
            node(
                instance,
                Some(asm),
                vec![part],
                kernel_api::ImportedNodeKind::Instance,
                "Screw:1",
            ),
        );
        let mut leaf = node(
            part,
            Some(instance),
            Vec::new(),
            kernel_api::ImportedNodeKind::Part,
            "Screw",
        );
        leaf.body_id = Some(body);
        graph.insert(part, leaf);
        doc.set_imported_object_graph(vec![asm], graph);

        // The instance and its only part draw as one row, which carries the
        // instance's id — so that is the row a pick has to land on.
        let tree = DocumentTree::build(&doc, &DocumentService::default());
        assert_eq!(
            tree.path_to_body(body),
            Some(vec![
                TreeItemId::ImportedObject(asm),
                TreeItemId::ImportedObject(instance),
            ])
        );
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

        let tree = DocumentTree::build(&doc, &DocumentService::default());
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

        let tree = DocumentTree::build(&doc, &DocumentService::default());
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
        let tree = DocumentTree::build(&doc, &DocumentService::default());
        assert_eq!(tree.nodes()[0].label, "Asm");
        assert_eq!(tree.nodes()[0].icon, "tree-group");
    }
}
