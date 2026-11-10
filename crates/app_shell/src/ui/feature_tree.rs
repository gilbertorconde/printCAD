use std::collections::{HashMap, HashSet};

use core_document::{
    Body, BodyId, ComponentId, Document, DocumentService, FeatureId, FeatureInfo, FeatureNode,
    MenuScope, NoteId, WorkbenchId,
};
use egui::{Response, Ui, Vec2};
use ui_kit::sans;
use ui_kit::tokens::*;
use uuid::Uuid;

use super::keymap::Keymap;

mod menus;
mod model;

use menus::*;
pub(crate) use model::feature_info;

/// Identifier for selectable items in the tree panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TreeItemId {
    DocumentRoot,
    Body(BodyId),
    Feature(FeatureId),
    ImportedObject(Uuid),
    Component(ComponentId),
    /// The group of a body's notes.
    Notes(BodyId),
    Note(NoteId),
    /// The group of an array's copies.
    Array(core_document::ArrayId),
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
    MoveUp,
    MoveDown,
    SetTip,
    ClearTip,
    /// Dragged and dropped on `target`'s row: goes just before or after it
    /// in the history.
    MoveNextTo {
        target: FeatureId,
        before: bool,
    },
}

/// A feature row being dragged.
#[derive(Debug, Clone, Copy)]
struct DraggedFeature(FeatureId);

#[derive(Debug, Default)]
pub struct TreeUiResult {
    pub selection: Option<TreeItemId>,
    /// A feature's row was Ctrl-clicked: it joins the selection, or leaves.
    pub toggle_feature: Option<FeatureId>,
    pub activation: Option<TreeItemId>,
    /// The row under the pointer this frame. It drives the details line, so
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
    /// Commands a row's menu asked for.
    pub commands: Vec<super::UiCommand>,
    /// A row's "!" was clicked: its label and the full message, to show
    /// where it can be read at length and copied.
    pub details: Option<(String, String)>,
    /// The document row's menu asked for a new variable set.
    pub new_variable_set: bool,
    /// The document row's menu asked for the configurations table.
    pub new_configurations: bool,
    /// A feature's menu asked to install the package it needs, from this
    /// repository.
    pub install_package: Option<String>,
    /// A menu entry that changes the window rather than the document.
    pub local: Option<super::MenuLocal>,
    /// Where each body's feature rows drew this frame, for the tip bar's
    /// drag to find the row under the pointer.
    history_rows: Vec<(BodyId, FeatureId, egui::Rect)>,
    /// Where each body's end of history drew: the tip dropped there builds
    /// all of it.
    history_ends: Vec<(BodyId, egui::Rect)>,
    /// The body whose tip bar is being dragged.
    tip_drag: Option<BodyId>,
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
    /// item is selected, never inline, where it only crowds the names.
    detail: Option<String>,
    tooltip: Option<String>,
    dirty: bool,
    visible: bool,
    suppressed: bool,
    error: Option<String>,
    /// A problem that does not stop the feature building, which its bench
    /// warns of: drawn as a warning badge with this as its tooltip.
    warning: Option<String>,
    /// The kernel's checker calls this body's shape broken, or one below
    /// this row: the row draws in the danger colour.
    defect: bool,
    /// Bodies at or below this row whose shape is broken and not yet sent
    /// for repair: what the row's "Repair" entry acts on.
    repairable: Vec<BodyId>,
    /// Mesh bodies at or below this row not yet sent for conversion: what
    /// the row's "Convert to solid" entry acts on.
    convertible: Vec<BodyId>,
    /// Converted solids at or below this row still in facets, not yet sent
    /// to be refined: what the row's "Refine shape" entry acts on.
    refinable: Vec<BodyId>,
    /// The row is a mesh body: its icon takes the mesh colour.
    mesh: bool,
    /// A feature from a package that is not loaded: the package's id and
    /// the repository its menu installs it from.
    needs_package: Option<(String, String)>,
    /// The row is its body's tip feature, or a feature past the tip (left
    /// out of the build).
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
    /// A feature of another body listed here because it acts on this one
    /// (a joint holding that body to this): a link, with no eye of its own.
    linked: bool,
    /// A body linked from another printCAD file.
    linked_file: bool,
    /// A frozen body: its features are not rebuilt.
    frozen: bool,
    /// A scan body, with what its row's tip says of it.
    scan: Option<String>,
    /// A body with surface textures.
    textured: bool,
    /// A converted solid whose curved areas are still the mesh's facets.
    faceted: bool,
    /// How the feature's work stands, as its bench says.
    status: Option<core_document::FeatureStatus>,
    /// What is wrong with this row's own item, drawn as a chip at its end
    /// and counted in the tree's header.
    problem: Option<Problem>,
    /// The body whose history this feature row stands in (not a linked
    /// row's): where the tip bar may be dragged to.
    history_body: Option<BodyId>,
}

/// A row's problem: the chip's few words, its tone and the full reason.
#[derive(Debug, Clone, PartialEq)]
struct Problem {
    word: &'static str,
    tone: ui_kit::widgets::Note,
    why: String,
}

/// What the tree draws with this frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct TreeDrawOptions<'a> {
    pub selected: Option<TreeItemId>,
    /// Features Ctrl-clicked into the selection beside `selected`.
    pub also: &'a [FeatureId],
    /// A body picked in the viewport that the tree should jump to: its row
    /// is selected, every branch above it opens, and the list scrolls to it.
    pub reveal_body: Option<BodyId>,
    /// The feature whose edit session is open: badged `EDITING`, and every
    /// other row dims.
    pub editing: Option<FeatureId>,
    /// The feature that made the face picked in the view: its row is lit
    /// as related, not selected.
    pub lit: Option<FeatureId>,
    /// Open every branch above the lit row and scroll to it.
    pub reveal_lit: bool,
    /// A row to select and bring into view: every branch above it opens.
    pub reveal_item: Option<TreeItemId>,
    /// Features tied to the one in hand (its users, and a selected one's
    /// inputs): their rows take the related tint.
    pub related: Option<&'a HashSet<FeatureId>>,
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
    /// The document's file cannot be written in place: its row says so.
    read_only: bool,
    /// Linked parts whose file cannot be written in place.
    read_only_links: &'a [BodyId],
}

impl<'a> TreeDrawOptions<'a> {
    pub fn new(selected: Option<TreeItemId>, editing: Option<FeatureId>, filter: &'a str) -> Self {
        Self {
            selected,
            also: &[],
            reveal_body: None,
            editing,
            lit: None,
            reveal_lit: false,
            reveal_item: None,
            related: None,
            filter,
            scroll_to: None,
            bench_menus: None,
            keymap: None,
            read_only: false,
            read_only_links: &[],
        }
    }

    /// Draw these features selected too, beside `selected`.
    pub fn with_also(mut self, also: &'a [FeatureId]) -> Self {
        self.also = also;
        self
    }

    /// The features a feature row's menu acts on: every selected feature
    /// when the row is one of them, else the row's own.
    fn acting_on(&self, feature: FeatureId) -> Vec<FeatureId> {
        let selected = match self.selected {
            Some(TreeItemId::Feature(id)) => Some(id),
            _ => None,
        };
        let all: Vec<FeatureId> = selected
            .into_iter()
            .chain(self.also.iter().copied())
            .collect();
        if all.contains(&feature) {
            all
        } else {
            vec![feature]
        }
    }

    /// Mark the document's row when its file is `read_only`, and the
    /// rows of the linked parts in `read_only_links`.
    pub fn with_file_marks(mut self, read_only: bool, read_only_links: &'a [BodyId]) -> Self {
        self.read_only = read_only;
        self.read_only_links = read_only_links;
        self
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

    /// Select `item`'s row and bring it into view.
    pub fn jumping_to(mut self, item: Option<TreeItemId>) -> Self {
        self.reveal_item = item;
        self
    }

    /// Tint the rows of `related` features as tied to the one in hand.
    pub fn relating(mut self, related: &'a HashSet<FeatureId>) -> Self {
        self.related = Some(related);
        self
    }

    /// Light the row of the feature that made the face picked in the view,
    /// and when `reveal`, open the branches above it and scroll to it.
    pub fn lighting(mut self, feature: Option<FeatureId>, reveal: bool) -> Self {
        self.lit = feature;
        self.reveal_lit = reveal;
        self
    }
}

/// The features tied to those in hand: every feature that uses a hovered
/// or selected one (a sketch's pads and pockets), and every input of a
/// selected one (a pad's sketch). The ones in hand are left out.
pub fn related_features(
    document: &Document,
    hovered: Option<FeatureId>,
    selected: &[FeatureId],
) -> HashSet<FeatureId> {
    let tree = document.feature_tree();
    let mut related = HashSet::new();
    for &feature in hovered.iter().chain(selected) {
        related.extend(tree.dependents(feature));
    }
    for &feature in selected {
        related.extend(tree.dependencies(feature));
    }
    for feature in hovered.iter().chain(selected) {
        related.remove(feature);
    }
    related
}

/// The feature row under the pointer when the tree was last drawn: what a
/// hover lights the users of, a frame later.
pub fn last_hovered_feature(ui: &Ui) -> Option<FeatureId> {
    ui.data(|d| d.get_temp::<Option<FeatureId>>(hovered_id()))
        .flatten()
}

fn hovered_id() -> egui::Id {
    egui::Id::new("tree_hovered_feature")
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
    // selection, the same answer a click on it would give.
    let mut options = options;
    let mut scroll_to = None;
    let trail = match (options.reveal_item, options.reveal_body) {
        (Some(item), _) => model.path_to(item),
        (None, Some(body)) => model.path_to_body(body),
        (None, None) => None,
    };
    if let Some(trail) = trail
        && let Some((row, ancestors)) = trail.split_last()
    {
        for ancestor in ancestors {
            set_open_state(ui, *ancestor, true);
        }
        set_open_state(ui, TreeItemId::DocumentRoot, true);
        options.selected = Some(*row);
        result.selection = Some(*row);
        scroll_to = Some(*row);
    } else if options.reveal_lit
        && let Some(feature) = options.lit
        && let Some(trail) = model.path_to(TreeItemId::Feature(feature))
        && let Some((row, ancestors)) = trail.split_last()
    {
        for ancestor in ancestors {
            set_open_state(ui, *ancestor, true);
        }
        set_open_state(ui, TreeItemId::DocumentRoot, true);
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
        badges: if options.read_only {
            vec![read_only_badge(crate::app::file_safety::READ_ONLY_TIP)]
        } else {
            Vec::new()
        },
        problem: None,
        eye: None,
        tooltip: options
            .read_only
            .then_some(crate::app::file_safety::READ_ONLY_TIP),
    };
    let open = draw_row(ui, &root, &options, &mut result, None);
    if open || !options.filter.is_empty() {
        for node in model.nodes() {
            if matches_filter(node, options.filter) {
                draw_node(ui, node, 1, &options, &mut result);
            }
        }
    }
    let hovered = match result.hovered {
        Some(TreeItemId::Feature(id)) => Some(id),
        _ => None,
    };
    ui.data_mut(|d| d.insert_temp(hovered_id(), hovered));
    if let Some(body) = result.tip_drag
        && let Some(y) = ui.ctx().pointer_interact_pos().map(|p| p.y)
    {
        match tip_drop(&result.history_rows, &result.history_ends, body, y) {
            Some(TipDrop::At(feature)) => {
                result.commands.push(super::UiCommand::MoveTipTo(feature));
            }
            Some(TipDrop::End) => result.commands.push(super::UiCommand::MoveTipToEnd(body)),
            None => {}
        }
    }
    result
}

/// Where the tip bar dropped: at a feature, which the body's build stops
/// at (with everything before it in build order), or at the end of the
/// history, which builds all of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TipDrop {
    At(FeatureId),
    End,
}

/// Where the tip bar of `body` dragged to height `y` lands: the end of
/// its history at or below the end's place, else the feature row under
/// it, else the nearest above it, else the first.
fn tip_drop(
    rows: &[(BodyId, FeatureId, egui::Rect)],
    ends: &[(BodyId, egui::Rect)],
    body: BodyId,
    y: f32,
) -> Option<TipDrop> {
    if ends.iter().any(|(b, end)| *b == body && end.top() <= y) {
        return Some(TipDrop::End);
    }
    let ours = || rows.iter().filter(|(b, _, _)| *b == body);
    ours()
        .filter(|(_, _, rect)| rect.top() <= y)
        .max_by(|a, b| a.2.top().total_cmp(&b.2.top()))
        .or_else(|| ours().min_by(|a, b| a.2.top().total_cmp(&b.2.top())))
        .map(|(_, feature, _)| TipDrop::At(*feature))
}

/// Whether a feature at or below `node` is its body's tip.
fn holds_tip(node: &TreeNode) -> bool {
    node.is_tip || node.children.iter().any(holds_tip)
}

/// The tip bar: a thin line under the body's tip row (under the whole
/// history, at the body's level, when it has no tip) that drags along the
/// history.
fn tip_bar(ui: &mut Ui, body: BodyId, depth: usize, result: &mut TreeUiResult) -> egui::Rect {
    // The id stays with the body, not the bar's place: the bar moves as
    // the tip follows the drag, and the drag must go on.
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), TIP_BAR),
        egui::Sense::hover(),
    );
    let response = ui.interact(
        rect,
        egui::Id::new(("tree_tip_bar", body)),
        egui::Sense::drag(),
    );
    let active = response.dragged() || response.hovered();
    let left = rect.left() + 8.0 + depth as f32 * 16.0;
    let color = if active {
        SUCCESS
    } else {
        with_alpha(SUCCESS, 0.6)
    };
    let width = if active { 2.0 } else { 1.5 };
    ui.painter().hline(
        left..=rect.right() - 8.0,
        rect.center().y,
        egui::Stroke::new(width, color),
    );
    ui.painter()
        .circle_filled(egui::pos2(left, rect.center().y), 3.0, color);
    let response = response
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text(
            "The body's tip: drag it along the history to see the body as it stands there",
        );
    if response.dragged() {
        result.tip_drag = Some(body);
    }
    rect
}

/// The end of a body's history while its tip stands earlier: a place the
/// tip bar drops on to build all of it, drawn faintly while the bar is
/// dragged.
fn history_end(ui: &mut Ui, body: BodyId, depth: usize, result: &TreeUiResult) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), TIP_BAR),
        egui::Sense::hover(),
    );
    if result.tip_drag == Some(body) {
        let left = rect.left() + 8.0 + depth as f32 * 16.0;
        let color = with_alpha(SUCCESS, 0.35);
        ui.painter().hline(
            left..=rect.right() - 8.0,
            rect.center().y,
            egui::Stroke::new(1.0, color),
        );
        ui.painter().circle_stroke(
            egui::pos2(left, rect.center().y),
            3.0,
            egui::Stroke::new(1.0, color),
        );
    }
    rect
}

/// How tall the tip bar is; its line sits in the middle.
const TIP_BAR: f32 = 7.0;

/// The mark of a file that cannot be written in place.
/// The badge a feature's status draws as, its note and how far it is in
/// the tooltip.
fn status_badge(status: &core_document::FeatureStatus) -> Badge {
    use core_document::FeatureState as S;
    let (text, color, words) = match status.state {
        S::NotRun => ("NOT RUN", TEXT3, "Not run yet"),
        S::Running => ("RUNNING", ACCENT, "Running"),
        S::Done => ("DONE", SUCCESS, "Done"),
        S::Warnings => ("WARNINGS", WARNING, "Done, with warnings"),
        S::Failed => ("FAILED", DANGER, "Failed"),
        S::OutOfDate => ("OUT OF DATE", WARNING, "Out of date: what it reads changed"),
    };
    let mut tooltip = match status.fraction {
        Some(f) if status.state == S::Running => format!("{words}, {:.0}%", f * 100.0),
        _ => words.to_string(),
    };
    if let Some(note) = &status.note {
        tooltip = format!("{tooltip}\n{note}");
    }
    Badge {
        text,
        color,
        tooltip: Some(tooltip),
        opens_details: false,
    }
}

fn read_only_badge(tip: &str) -> Badge {
    Badge {
        text: "READ-ONLY",
        color: WARNING,
        tooltip: Some(tip.to_string()),
        opens_details: false,
    }
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
    /// The row's own problem, a chip before the badges.
    problem: Option<&'a Problem>,
    /// Visibility toggle at the row's end: `Some(visible)`.
    eye: Option<bool>,
    tooltip: Option<&'a str>,
}

const ROW_FONT: f32 = 12.5;

/// Whether a branch shows its children. Until the user says otherwise, a
/// body and its features are open (that is the work in progress) and an
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

/// A feature row's part in dragging features along the history: it starts
/// a drag, and while another feature is dragged over it, shows where that
/// goes (above its middle, before it; below, after it) and takes the drop.
fn drag_and_drop(
    ui: &Ui,
    response: &egui::Response,
    rect: egui::Rect,
    id: FeatureId,
    result: &mut TreeUiResult,
) {
    if response.drag_started() {
        response.dnd_set_drag_payload(DraggedFeature(id));
    }
    if response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    let Some(dragged) = response.dnd_hover_payload::<DraggedFeature>() else {
        return;
    };
    if dragged.0 == id {
        return;
    }
    let before = ui
        .ctx()
        .pointer_hover_pos()
        .is_some_and(|p| p.y < rect.center().y);
    let y = if before { rect.top() } else { rect.bottom() };
    ui.painter().hline(
        rect.left()..=rect.right(),
        y,
        egui::Stroke::new(2.0, ACCENT),
    );
    if let Some(dragged) = response.dnd_release_payload::<DraggedFeature>() {
        result.feature_command = Some((
            dragged.0,
            TreeFeatureCommand::MoveNextTo { target: id, before },
        ));
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
    let selected = options.selected == Some(spec.id)
        || matches!(spec.id, TreeItemId::Feature(id) if options.also.contains(&id));
    // A feature's row drags to another place in the history.
    let feature = match spec.id {
        TreeItemId::Feature(id) => Some(id),
        _ => None,
    };
    let sense = if feature.is_some() {
        egui::Sense::click_and_drag()
    } else {
        egui::Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), TREE_ROW), sense);
    if let Some(id) = feature {
        drag_and_drop(ui, &response, rect, id, result);
        if let Some(body) = node.and_then(|n| n.history_body) {
            result.history_rows.push((body, id, rect));
        }
    }
    if options.scroll_to == Some(spec.id) {
        ui.scroll_to_rect(rect, Some(egui::Align::Center));
    }
    let lit = feature.is_some() && feature == options.lit;
    let related = feature.is_some_and(|id| options.related.is_some_and(|r| r.contains(&id)));
    if selected {
        ui.painter().rect_filled(rect, 0.0, ACCENT_DIM);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.left_top(), Vec2::new(2.0, rect.height())),
            0.0,
            ACCENT,
        );
    } else if lit {
        ui.painter().rect_filled(rect, 0.0, RELATED_FAINT);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.left_top(), Vec2::new(1.5, rect.height())),
            0.0,
            RELATED,
        );
    } else {
        let fill = if response.hovered() {
            BG2
        } else if related {
            RELATED_FAINT
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 0.0, fill);
        if related {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.left_top(), Vec2::new(1.5, rect.height())),
                0.0,
                RELATED,
            );
        }
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
                TreeItemId::Note(note) => {
                    result.commands.push(super::UiCommand::SetNoteVisible {
                        note,
                        visible: !visible,
                    });
                }
                TreeItemId::Notes(_) => {
                    if let Some(node) = node {
                        result.commands.extend(node.children.iter().filter_map(
                            |row| match row.id {
                                TreeItemId::Note(note) => Some(super::UiCommand::SetNoteVisible {
                                    note,
                                    visible: !visible,
                                }),
                                _ => None,
                            },
                        ));
                    }
                }
                TreeItemId::Component(_) | TreeItemId::Array(_) => {
                    let mut bodies = Vec::new();
                    if let Some(node) = node {
                        bodies_below(node, &mut bodies);
                    }
                    result.commands.extend(bodies.into_iter().map(|body| {
                        super::UiCommand::SetBodyVisible {
                            body,
                            visible: !visible,
                        }
                    }));
                }
                _ => {}
            }
        }
        right -= 20.0;
    }
    if let Some(problem) = spec.problem {
        let w = ui_kit::widgets::chip_width(ui, problem.word);
        let chip_rect = egui::Rect::from_center_size(
            egui::pos2(right - w / 2.0, cy),
            Vec2::new(w, ui_kit::tokens::CHIP),
        );
        let chip = ui
            .scope_builder(egui::UiBuilder::new().max_rect(chip_rect), |ui| {
                ui_kit::widgets::chip(
                    ui,
                    problem.word,
                    ui_kit::widgets::ChipLook {
                        tone: Some(problem.tone),
                        ..Default::default()
                    },
                )
            })
            .inner
            .on_hover_text(format!(
                "{}\n\nClick to open this in a window you can copy from",
                problem.why
            ));
        if chip.clicked() {
            result.details = Some((spec.label.to_string(), problem.why.clone()));
        }
        right -= w + 6.0;
    }
    for badge in spec.badges.iter().rev() {
        let galley = ui.painter().layout_no_wrap(
            badge.text.to_string(),
            ui_kit::sans_semibold(FONT_XXS),
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
        None if spec.id == TreeItemId::DocumentRoot => {
            response.context_menu(|ui| {
                ui_kit::widgets::fitted_menu(ui, |ui| {
                    if ui
                        .button("New variable set")
                        .on_hover_text("Named values any number in the model can follow")
                        .clicked()
                    {
                        result.new_variable_set = true;
                        ui.close();
                    }
                    if ui
                        .button("New configurations table")
                        .on_hover_text(
                            "Versions of the model, each giving variables values of its own",
                        )
                        .clicked()
                    {
                        result.new_configurations = true;
                        ui.close();
                    }
                });
            });
            response
        }
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
    if node.textured {
        badges.push(Badge {
            text: "TEXTURED",
            color: TEXT3,
            tooltip: Some(
                "A pattern is pressed into its faces: drawn here, and in the files for the slicer"
                    .to_string(),
            ),
            opens_details: false,
        });
    }
    if node.faceted {
        let own = node.body.is_some_and(|b| node.refinable.contains(&b));
        badges.push(Badge {
            text: "FACETED",
            color: WARNING,
            tooltip: Some(if own {
                "Converted from a mesh as it is: its curved areas are flat facets, and \
                 features on or along them (pads and pockets on a face, fillets) may fail. \
                 Right-click › Refine shape finds their round faces first."
                    .to_string()
            } else {
                "Converted from a mesh as it is: its curved areas are flat facets, and \
                 features on or along them may fail. Refine shape is offered only before \
                 the body has features."
                    .to_string()
            }),
            opens_details: false,
        });
    }
    if node.linked_file
        && node
            .body
            .is_some_and(|body| options.read_only_links.contains(&body))
    {
        badges.push(read_only_badge(crate::app::file_safety::READ_ONLY_LINK_TIP));
    }
    if let Some(status) = &node.status {
        badges.push(status_badge(status));
    }
    if let Some(tip) = &node.scan {
        badges.push(Badge {
            text: "SCAN",
            color: ACCENT,
            tooltip: Some(tip.clone()),
            opens_details: false,
        });
    }
    if node.frozen {
        badges.push(Badge {
            text: "FROZEN",
            color: TEXT3,
            tooltip: Some("Kept as it stands: its features are not rebuilt".to_string()),
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
    if node.problem.is_some() {
        // The chip names it.
    } else if let Some(error) = &node.error {
        badges.push(Badge {
            text: "!",
            color: DANGER,
            tooltip: Some(error.clone()),
            opens_details: true,
        });
    } else if let Some(warning) = &node.warning {
        badges.push(Badge {
            text: "!",
            color: WARNING,
            tooltip: Some(warning.clone()),
            opens_details: true,
        });
    }
    let dimmed_by_edit = options.editing.is_some() && !editing_here;
    let eye = match node.id {
        _ if node.linked => None,
        TreeItemId::ImportedObject(_)
        | TreeItemId::Feature(_)
        | TreeItemId::Body(_)
        | TreeItemId::Component(_)
        | TreeItemId::Notes(_)
        | TreeItemId::Array(_)
        | TreeItemId::Note(_) => Some(node.visible),
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
    } else if node.warning.is_some() {
        WARNING
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
        problem: node.problem.as_ref(),
        eye,
        tooltip: node.tooltip.as_deref(),
    };
    let open = draw_row(ui, &spec, options, result, Some(node));
    // The tip bar shows in the plain tree, not a filtered one.
    let bars = options.filter.is_empty();
    if bars
        && node.is_tip
        && let Some(body) = node.history_body
    {
        tip_bar(ui, body, depth, result);
    }
    // A closed branch stays closed to the eye, but not to a filter: what
    // matches is shown wherever it sits.
    if open || !options.filter.is_empty() {
        for child in &node.children {
            if matches_filter(child, options.filter) {
                draw_node(ui, child, depth + 1, options, result);
            }
        }
        // Under the last of a body's history, at the body's level: the bar
        // of a body with no tip, which builds all of it, or the place the
        // bar drops on to build all of it again.
        if bars
            && !node.mesh
            && matches!(node.id, TreeItemId::Body(_) | TreeItemId::ImportedObject(_))
            && let Some(body) = node.body
            && node.children.iter().any(|c| c.history_body == Some(body))
        {
            let end = if holds_tip(node) {
                history_end(ui, body, depth + 1, result)
            } else {
                tip_bar(ui, body, depth + 1, result)
            };
            result.history_ends.push((body, end));
        }
    }
}

fn handle_response(response: Response, id: TreeItemId, result: &mut TreeUiResult) {
    if response.hovered() {
        result.hovered = Some(id);
    }
    if response.clicked() {
        match id {
            TreeItemId::Feature(feature) if response.ctx.input(|i| i.modifiers.command) => {
                result.toggle_feature = Some(feature);
            }
            _ => result.selection = Some(id),
        }
    }
    if response.double_clicked() {
        // A note's double click opens it for editing, as a feature's does.
        if let TreeItemId::Note(note) = id {
            result.local = Some(super::MenuLocal::Task(super::OpenTask::EditNote(note)));
        } else {
            result.activation = Some(id);
        }
    }
}

/// A feature row's tooltip: what the feature is ("Hole · M6 thread") when
/// its bench says more than the family, the family, then its state.
fn feature_tooltip(node: &FeatureNode, info: &FeatureInfo, after_tip: bool) -> String {
    let mut parts = Vec::new();
    if info.kind_label != info.family_label {
        parts.push(info.kind_label.clone());
    }
    parts.push(info.family_label.clone());
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

/// A scan row's tip: what it is and how dense, from its mesh.
fn scan_tip(document: &Document, body: BodyId) -> String {
    let mut tip = "A scan: the measured part inspection and mesh editing work on".to_string();
    if let Some(mesh) = document.imported_geometry(body).map(|g| &g.mesh) {
        let facts = crate::app::scans::mesh_facts(mesh);
        tip.push_str(&format!(
            "\n{} triangles, {} points, {:.3} mm between points (median)",
            facts.triangles, facts.points, facts.spacing
        ));
    }
    if let Some(unit) = document.body_scan(body).and_then(|s| s.read_in) {
        let name = crate::app::scans::unit_name(unit);
        tip.push_str(&format!("\nRead in {name}, scaled to millimetres"));
    }
    tip
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

    /// The tree of a document with no workbench registered.
    fn tree(doc: &Document) -> DocumentTree {
        DocumentTree::build(doc, &DocumentService::default())
    }

    /// The top-level row of `body`.
    fn body_row(tree: &DocumentTree, body: BodyId) -> &TreeNode {
        tree.nodes()
            .iter()
            .find(|n| n.id == TreeItemId::Body(body))
            .expect("the body's row")
    }

    /// An empty mesh read from `asset`, with the checker's `health`.
    fn imported(
        asset: Uuid,
        health: Option<kernel_api::ShapeHealth>,
    ) -> core_document::ImportedGeometry {
        core_document::ImportedGeometry {
            source_asset: Some(asset),
            health,
            ..crate::test_support::geometry(kernel_api::TriMesh::default())
        }
    }

    /// An imported object graph from its nodes, the parentless ones its
    /// roots.
    fn set_graph(doc: &mut Document, nodes: Vec<core_document::ImportedObjectNode>) {
        let roots = nodes
            .iter()
            .filter(|n| n.parent_id.is_none())
            .map(|n| n.id)
            .collect();
        doc.set_imported_object_graph(roots, nodes.into_iter().map(|n| (n.id, n)).collect());
    }

    /// A part leaf of `parent` holding `body`.
    fn part_of(
        id: Uuid,
        parent: Uuid,
        name: &str,
        body: BodyId,
    ) -> core_document::ImportedObjectNode {
        let mut part = node(
            id,
            Some(parent),
            Vec::new(),
            kernel_api::ImportedNodeKind::Part,
            name,
        );
        part.body_id = Some(body);
        part
    }

    #[test]
    fn a_feature_menu_acts_on_the_whole_selection_only_from_inside_it() {
        let [a, b, c] = [(); 3].map(|_| FeatureId(Uuid::new_v4()));
        let also = [b];
        let options = TreeDrawOptions::new(Some(TreeItemId::Feature(a)), None, "").with_also(&also);
        assert_eq!(options.acting_on(a), [a, b]);
        assert_eq!(options.acting_on(b), [a, b]);
        assert_eq!(options.acting_on(c), [c], "a row outside it acts alone");
        let alone = TreeDrawOptions::new(Some(TreeItemId::Feature(a)), None, "");
        assert_eq!(alone.acting_on(a), [a]);
    }

    /// An assembly over a part draws the import's rows, not a body row of
    /// its own, labelled by name alone; the body is found under the
    /// assembly that holds it.
    #[test]
    fn an_imported_part_sits_under_its_assembly_without_a_body_row() {
        let mut doc = Document::new("tree");
        let body = doc.create_body(Some("Imported Body".into()));
        let (root, leaf, loose) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        set_graph(
            &mut doc,
            vec![
                node(
                    root,
                    None,
                    vec![leaf, loose],
                    kernel_api::ImportedNodeKind::Assembly,
                    "Asm",
                ),
                part_of(leaf, root, "Part", body),
                node(
                    loose,
                    Some(root),
                    Vec::new(),
                    kernel_api::ImportedNodeKind::Part,
                    "Loose",
                ),
            ],
        );
        let tree = tree(&doc);
        // The details line says which part holds a body.
        let detail = |id| {
            tree.nodes()[0]
                .children
                .iter()
                .find(|n| n.id == TreeItemId::ImportedObject(id))
                .and_then(|n| n.detail.clone())
        };
        assert_eq!(detail(leaf).as_deref(), Some("Part, linked to a body"));
        assert_eq!(detail(loose).as_deref(), Some("Part"));
        let mut ids = Vec::new();
        collect_ids(tree.nodes(), &mut ids);
        assert!(ids.contains(&TreeItemId::ImportedObject(root)));
        assert!(ids.contains(&TreeItemId::ImportedObject(leaf)));
        assert!(!ids.contains(&TreeItemId::Body(body)));
        assert_eq!(
            tree.nodes()[0].label,
            "Asm",
            "no bracket tag, just the name"
        );
        assert_eq!(tree.nodes()[0].icon, "tree-group");
        assert_eq!(
            tree.path_to_body(body),
            Some(vec![
                TreeItemId::ImportedObject(root),
                TreeItemId::ImportedObject(leaf),
            ]),
            "the assembly above the part is on the way to it"
        );
    }

    /// A feature a bench links to another body shows under that body too,
    /// with the body it is kept with named, and no eye of its own.
    #[test]
    fn a_linked_feature_shows_under_the_body_it_acts_on() {
        use core_document::{Workbench, WorkbenchContext, WorkbenchDescriptor};
        struct Bench;
        impl Workbench for Bench {
            fn descriptor(&self) -> WorkbenchDescriptor {
                WorkbenchDescriptor::new("test.links", "Links", "")
            }
            fn configure(&self, _context: &mut WorkbenchContext) {}
            fn linked_features(&self, document: &Document, body: BodyId) -> Vec<FeatureId> {
                document
                    .feature_tree()
                    .all_nodes()
                    .filter(|(_, n)| n.body.is_some_and(|b| b != body))
                    .map(|(id, _)| *id)
                    .collect()
            }
        }
        let mut doc = Document::new("t");
        let base = doc.create_body(Some("Base".into()));
        let part = doc.create_body(Some("Part".into()));
        let joint = doc.add_feature_of_kind(
            core_document::WorkbenchId::new("test.joint"),
            "Mate 1".into(),
            Some(part),
            Vec::new(),
            serde_json::json!({}),
            core_document::FeatureOrigin::default(),
        );
        let mut registry = DocumentService::default();
        registry.register_workbench(Box::new(Bench)).unwrap();
        let tree = DocumentTree::build(&doc, &registry);
        let row = |body: BodyId| {
            tree.nodes()
                .iter()
                .find(|n| n.id == TreeItemId::Body(body))
                .expect("a body row")
        };
        let own = &row(part).children;
        assert_eq!(own.len(), 1);
        assert!(!own[0].linked);
        let linked = &row(base).children;
        assert_eq!(linked.len(), 1);
        assert_eq!(linked[0].id, TreeItemId::Feature(joint));
        assert!(linked[0].linked);
        assert_eq!(linked[0].label, "Mate 1 (Part)");
    }

    /// A feature row's tooltip says what the feature is, as its bench
    /// names it ("Hole · M12 thread"), above the family the Kind row shows.
    #[test]
    fn a_feature_row_tooltip_names_the_feature_as_its_bench_does() {
        use core_document::{Workbench, WorkbenchContext, WorkbenchDescriptor};
        struct Bench;
        impl Workbench for Bench {
            fn descriptor(&self) -> WorkbenchDescriptor {
                WorkbenchDescriptor::new("test.holes", "Holes", "").feature_kinds(["test.hole"])
            }
            fn configure(&self, _context: &mut WorkbenchContext) {}
            fn feature_info(&self, _node: &FeatureNode) -> FeatureInfo {
                FeatureInfo {
                    icon: "tree-feature",
                    kind_label: "Hole · M12 thread".into(),
                    family_label: "Drilling".into(),
                    builds_solid: true,
                }
            }
        }
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Body".into()));
        let hole = doc.add_feature_of_kind(
            core_document::WorkbenchId::new("test.hole"),
            "Hole".into(),
            Some(body),
            Vec::new(),
            serde_json::json!({}),
            core_document::FeatureOrigin::default(),
        );
        let mut registry = DocumentService::default();
        registry.register_workbench(Box::new(Bench)).unwrap();
        let tree = DocumentTree::build(&doc, &registry);
        let body_row = tree
            .nodes()
            .iter()
            .find(|n| n.id == TreeItemId::Body(body))
            .expect("a body row");
        let row = body_row
            .children
            .iter()
            .find(|n| n.id == TreeItemId::Feature(hole))
            .expect("the hole's row");
        let tooltip = row.tooltip.as_deref().expect("a tooltip");
        let mut lines = tooltip.lines();
        assert_eq!(lines.next(), Some("Hole · M12 thread"), "{tooltip}");
        assert_eq!(lines.next(), Some("Drilling"), "{tooltip}");
    }

    /// A failing feature, one left unbuilt after it and one reading it each
    /// name their problem, and the header's jump walks them in the tree's
    /// order, round again from the last.
    #[test]
    fn each_problem_row_names_its_problem_and_the_jump_walks_them() {
        use core_document::{Workbench, WorkbenchContext, WorkbenchDescriptor};
        struct Bench;
        impl Workbench for Bench {
            fn descriptor(&self) -> WorkbenchDescriptor {
                WorkbenchDescriptor::new("test.steps", "Steps", "").feature_kinds(["test.step"])
            }
            fn configure(&self, _context: &mut WorkbenchContext) {}
        }
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Body".into()));
        let add = |doc: &mut Document, name: &str, deps: Vec<FeatureId>| {
            doc.add_feature_of_kind(
                core_document::WorkbenchId::new("test.step"),
                name.into(),
                Some(body),
                deps,
                serde_json::json!({}),
                core_document::FeatureOrigin::default(),
            )
        };
        let sound = add(&mut doc, "Sound", Vec::new());
        let failed = add(&mut doc, "Failed", Vec::new());
        let reader = add(&mut doc, "Reader", vec![failed]);
        let later = add(&mut doc, "Later", Vec::new());
        doc.set_feature_error(failed, Some("the cut misses the solid".into()));
        doc.set_feature_error(later, Some("not built: `Failed` before it fails".into()));
        let mut registry = DocumentService::default();
        registry.register_workbench(Box::new(Bench)).unwrap();
        let tree = DocumentTree::build(&doc, &registry);
        let word = |id: FeatureId| {
            fn find(nodes: &[TreeNode], id: TreeItemId) -> Option<&TreeNode> {
                nodes
                    .iter()
                    .find_map(|n| (n.id == id).then_some(n).or_else(|| find(&n.children, id)))
            }
            find(tree.nodes(), TreeItemId::Feature(id))
                .expect("a row")
                .problem
                .as_ref()
                .map(|p| p.word)
        };
        assert_eq!(word(sound), None);
        assert_eq!(word(failed), Some("Failed"));
        assert_eq!(word(reader), Some("Input broken"));
        assert_eq!(word(later), Some("Not built"));

        let order = tree.problems();
        assert_eq!(order.len(), 3);
        assert_eq!(tree.next_problem(None), Some(order[0]));
        assert_eq!(tree.next_problem(Some(order[0])), Some(order[1]));
        assert_eq!(tree.next_problem(Some(order[2])), Some(order[0]));
        assert_eq!(
            tree.next_problem(Some(TreeItemId::Feature(sound))),
            Some(order[0])
        );
    }

    /// A hovered sketch ties the features built on it; a selected pad
    /// ties its sketch and what is built on the pad, never itself.
    #[test]
    fn a_sketch_ties_its_users_and_a_selected_feature_its_inputs() {
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Body".into()));
        let add = |doc: &mut Document, name: &str, deps: Vec<FeatureId>| {
            doc.add_feature_of_kind(
                core_document::WorkbenchId::new("test.step"),
                name.into(),
                Some(body),
                deps,
                serde_json::json!({}),
                core_document::FeatureOrigin::default(),
            )
        };
        let sketch = add(&mut doc, "Sketch", Vec::new());
        let pad = add(&mut doc, "Pad", vec![sketch]);
        let pocket = add(&mut doc, "Pocket", vec![sketch]);
        let on_pad = add(&mut doc, "Sketch001", vec![pad]);
        let set = |items: &[FeatureId]| items.iter().copied().collect::<HashSet<_>>();

        assert_eq!(
            related_features(&doc, Some(sketch), &[]),
            set(&[pad, pocket])
        );
        assert_eq!(related_features(&doc, None, &[pad]), set(&[sketch, on_pad]));
        assert_eq!(
            related_features(&doc, Some(on_pad), &[pad]),
            set(&[sketch]),
            "what is in hand is never lit as related"
        );
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
            let health = kernel_api::ShapeHealth {
                broken: broken_count,
                ..Default::default()
            };
            doc.set_imported_geometry(body, imported(asset, Some(health)));
        }
        let root = Uuid::new_v4();
        let (broken_leaf, sound_leaf) = (Uuid::new_v4(), Uuid::new_v4());
        set_graph(
            &mut doc,
            vec![
                node(
                    root,
                    None,
                    vec![broken_leaf, sound_leaf],
                    kernel_api::ImportedNodeKind::Assembly,
                    "Asm",
                ),
                part_of(broken_leaf, root, "Part", broken),
                part_of(sound_leaf, root, "Part", sound),
            ],
        );

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
        let tree = self::tree(&doc);
        assert_eq!(
            tree.problems(),
            vec![TreeItemId::ImportedObject(broken_leaf)],
            "the part has the problem; the assembly above only shows it"
        );
        assert_eq!(find(&tree, broken_leaf), (true, vec![broken]));
        assert_eq!(find(&tree, sound_leaf), (false, vec![]));
        assert_eq!(
            find(&tree, root),
            (true, vec![broken]),
            "the closed assembly shows what is inside it"
        );

        assert!(doc.request_body_repair(broken));
        let tree = self::tree(&doc);
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
        doc.set_imported_geometry(mesh, imported(asset, None));
        let row = |doc: &Document| {
            let tree = self::tree(doc);
            let node = body_row(&tree, mesh);
            (
                node.detail.clone().unwrap_or_default(),
                node.convertible.clone(),
            )
        };
        let (detail, convertible) = row(&doc);
        assert!(detail.starts_with("Mesh"), "{detail}");
        let tree = tree(&doc);
        let node = body_row(&tree, mesh);
        assert_eq!(node.icon, "workbench-mesh", "a mesh row has its own icon");
        assert!(node.mesh, "and its own colour");
        assert_eq!(convertible, vec![mesh]);

        assert!(doc.request_mesh_solid(mesh));
        let (detail, convertible) = row(&doc);
        assert!(detail.contains("converting"), "{detail}");
        assert!(convertible.is_empty(), "not offered twice");

        // The solid lands with its curved areas as facets: the row says so
        // and offers the refine, once.
        doc.set_imported_brep_data(mesh, b"ogeom".to_vec(), Vec::new());
        let health = kernel_api::ShapeHealth {
            faceted: true,
            ..Default::default()
        };
        doc.set_imported_geometry(mesh, imported(asset, Some(health)));
        let faceted = |doc: &Document| {
            let tree = self::tree(doc);
            let node = body_row(&tree, mesh);
            (node.faceted, node.refinable.clone())
        };
        assert_eq!(faceted(&doc), (true, vec![mesh]));
        assert!(doc.request_body_refine(mesh));
        assert_eq!(
            faceted(&doc),
            (true, vec![]),
            "still faceted until the refine lands, and not offered twice"
        );
    }

    /// The tip bar lands on its own body's row under the pointer, the
    /// nearest above it between rows, and the first above them all; at or
    /// below the end of its body's history it builds all of it, whatever
    /// row sits there last.
    #[test]
    fn the_tip_bar_lands_on_its_bodys_row_under_the_pointer() {
        let (a, b) = (BodyId(Uuid::new_v4()), BodyId(Uuid::new_v4()));
        let f: Vec<FeatureId> = (0..3).map(|_| FeatureId(Uuid::new_v4())).collect();
        let row =
            |top: f32| egui::Rect::from_min_size(egui::pos2(0.0, top), egui::vec2(200.0, TREE_ROW));
        let rows = vec![
            (a, f[0], row(100.0)),
            (b, f[1], row(124.0)),
            (a, f[2], row(148.0)),
        ];
        let ends = vec![(a, row(172.0))];
        for (y, want) in [
            (110.0, TipDrop::At(f[0])),
            (130.0, TipDrop::At(f[0])),
            (160.0, TipDrop::At(f[2])),
            (175.0, TipDrop::End),
            (400.0, TipDrop::End),
            (10.0, TipDrop::At(f[0])),
        ] {
            assert_eq!(tip_drop(&rows, &ends, a, y), Some(want), "at {y}");
        }
        assert_eq!(
            tip_drop(&rows, &ends, b, 400.0),
            Some(TipDrop::At(f[1])),
            "another body's end is not this one's"
        );
    }

    /// A feature sits under the feature its bench says it consumes; one
    /// that works on the whole body sits at the body's top level in build
    /// order, whatever it depends on.
    #[test]
    fn only_a_consumed_feature_holds_a_row_under_it() {
        use core_document::{Workbench, WorkbenchContext, WorkbenchDescriptor};
        struct Bench;
        impl Workbench for Bench {
            fn descriptor(&self) -> WorkbenchDescriptor {
                WorkbenchDescriptor::new("test.steps", "Steps", "").feature_kinds(["test.step"])
            }
            fn configure(&self, _context: &mut WorkbenchContext) {}
            fn consumed_feature(
                &self,
                _document: &Document,
                node: &FeatureNode,
            ) -> Option<FeatureId> {
                serde_json::from_value(node.data.get("consumes")?.clone()).ok()
            }
        }
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Body".into()));
        let mut add = |name: &str, deps: Vec<FeatureId>, consumes: Option<FeatureId>| {
            doc.add_feature_of_kind(
                core_document::WorkbenchId::new("test.step"),
                name.into(),
                Some(body),
                deps,
                serde_json::json!({ "consumes": consumes }),
                core_document::FeatureOrigin::default(),
            )
        };
        let sketch = add("Sketch", vec![], None);
        let pad = add("Pad", vec![sketch], Some(sketch));
        let other = add("Sketch_1", vec![], None);
        let hole = add("Hole", vec![other], Some(other));
        let mirror = add("Mirror body", vec![pad], None);
        let fillet = add("Fillet", vec![pad, hole], None);
        let mut registry = DocumentService::default();
        registry.register_workbench(Box::new(Bench)).unwrap();
        let tree = DocumentTree::build(&doc, &registry);
        let ids = |rows: &[TreeNode]| rows.iter().map(|n| n.id).collect::<Vec<_>>();
        let top = &body_row(&tree, body).children;
        assert_eq!(
            ids(top),
            [sketch, other, mirror, fillet].map(TreeItemId::Feature),
            "the whole-body features last, in build order"
        );
        assert_eq!(ids(&top[0].children), [TreeItemId::Feature(pad)]);
        assert_eq!(ids(&top[1].children), [TreeItemId::Feature(hole)]);
    }

    #[test]
    fn a_hidden_body_row_reads_as_hidden() {
        let mut doc = Document::new("tree");
        let body = doc.create_body(Some("Body".into()));
        let visible = |doc: &Document| body_row(&tree(doc), body).visible;
        assert!(visible(&doc));
        assert_eq!(
            tree(&doc).path_to_body(body),
            Some(vec![TreeItemId::Body(body)]),
            "a body with no import above it is its own row"
        );
        doc.set_body_visible(body, false);
        assert!(!visible(&doc), "its eye is shut and its row muted");
    }

    #[test]
    fn a_bodys_notes_sit_in_its_annotations_group() {
        let mut doc = Document::new("tree");
        let body = doc.create_body(Some("Body".into()));
        let groups = |tree: &DocumentTree| {
            body_row(tree, body)
                .children
                .iter()
                .filter(|c| c.id == TreeItemId::Notes(body))
                .map(|c| {
                    let notes: Vec<(TreeItemId, String, bool)> = c
                        .children
                        .iter()
                        .map(|n| (n.id, n.label.clone(), n.visible))
                        .collect();
                    notes
                })
                .collect::<Vec<_>>()
        };
        assert!(groups(&tree(&doc)).is_empty(), "no group without notes");
        let note = core_document::Note::pinned("=1 mm + 1 mm".into(), [0.0; 3], [0.0; 3], None);
        let id = doc.add_note(body, note).unwrap();
        let mut hidden = core_document::Note::pinned("Hidden".into(), [0.0; 3], [0.0; 3], None);
        hidden.hidden = true;
        let other = doc.add_note(body, hidden).unwrap();
        assert_eq!(
            groups(&tree(&doc)),
            [vec![
                (TreeItemId::Note(id), "#1  2 mm".to_string(), true),
                (TreeItemId::Note(other), "#2  Hidden".to_string(), false),
            ]],
            "one group, each note labelled as it reads, its eye its own"
        );
    }

    #[test]
    fn an_arrays_copies_sit_under_its_row_and_a_copy_is_still_found() {
        let mut doc = Document::new("tree");
        let source = doc.create_body(Some("Peg".into()));
        let other = doc.create_body(Some("Other".into()));
        let row = core_document::ArrayPattern::Linear {
            direction: [1.0, 0.0, 0.0],
            spacing: 10.0,
        };
        let array = doc.create_copy_array(source, row.clone(), 3).unwrap();
        let copies: Vec<TreeItemId> = doc.copy_array(array).unwrap().instances[..]
            .iter()
            .map(|i| TreeItemId::Body(i.body))
            .collect();
        let tree = tree(&doc);
        let ids: Vec<TreeItemId> = tree.nodes().iter().map(|n| n.id).collect();
        assert_eq!(
            ids,
            [
                TreeItemId::Body(source),
                TreeItemId::Body(other),
                TreeItemId::Array(array)
            ]
        );
        let group = &tree.nodes()[2];
        let inside: Vec<TreeItemId> = group.children.iter().map(|n| n.id).collect();
        assert_eq!(inside, copies);
        assert_eq!(
            tree.path_to_body(doc.copy_array(array).unwrap().instances[1].body),
            Some(vec![TreeItemId::Array(array), copies[1]])
        );

        // A component's array gathers the components holding its copies.
        let unit = doc.create_component("Pair".into(), None).unwrap();
        doc.set_body_component(other, Some(unit)).unwrap();
        let pairs = doc.create_component_array(unit, row, 2).unwrap();
        let group = doc.copy_array(pairs).unwrap().instances[0].group.unwrap();
        let tree = self::tree(&doc);
        let held = tree
            .nodes()
            .iter()
            .find(|n| n.id == TreeItemId::Array(pairs))
            .map(|n| n.children.iter().map(|c| c.id).collect::<Vec<_>>());
        assert_eq!(held, Some(vec![TreeItemId::Component(group)]));
    }

    #[test]
    fn bodies_sit_under_their_components_where_the_first_one_would() {
        let mut doc = Document::new("tree");
        let [a, b, c, d] = ["A", "B", "C", "D"].map(|n| doc.create_body(Some(n.into())));
        let outer = doc.create_component("Outer".into(), None).unwrap();
        let inner = doc.create_component("Inner".into(), Some(outer)).unwrap();
        let empty = doc.create_component("Empty".into(), None).unwrap();
        doc.set_body_component(b, Some(inner)).unwrap();
        doc.set_body_component(d, Some(outer)).unwrap();
        let tree = tree(&doc);
        let ids: Vec<TreeItemId> = tree.nodes().iter().map(|n| n.id).collect();
        assert_eq!(
            ids,
            [
                TreeItemId::Body(a),
                TreeItemId::Component(outer),
                TreeItemId::Body(c),
                TreeItemId::Component(empty),
            ]
        );
        assert_eq!(
            tree.path_to_body(b),
            Some(vec![
                TreeItemId::Component(outer),
                TreeItemId::Component(inner),
                TreeItemId::Body(b),
            ])
        );
        let outer_row = &tree.nodes()[1];
        let inside: Vec<TreeItemId> = outer_row.children.iter().map(|n| n.id).collect();
        assert_eq!(inside, [TreeItemId::Component(inner), TreeItemId::Body(d)]);
    }

    /// An instance wrapping a single part is one row with the instance's
    /// id, which is where a pick of the part lands; a generically named
    /// one takes its part's name.
    #[test]
    fn an_instance_and_its_one_part_are_one_row() {
        use kernel_api::ImportedNodeKind as K;
        let mut doc = Document::new("tree");
        let (screw_body, frame_body) = (
            doc.create_body(Some("Screw".into())),
            doc.create_body(Some("Frame".into())),
        );
        let [asm, screw, screw_part, generic, generic_part] = [(); 5].map(|_| Uuid::new_v4());
        set_graph(
            &mut doc,
            vec![
                node(asm, None, vec![screw, generic], K::Assembly, "Asm"),
                node(screw, Some(asm), vec![screw_part], K::Instance, "Screw:1"),
                part_of(screw_part, screw, "Screw", screw_body),
                node(
                    generic,
                    Some(asm),
                    vec![generic_part],
                    K::Instance,
                    "Instance",
                ),
                part_of(generic_part, generic, "Anet v1-body", frame_body),
            ],
        );
        let tree = tree(&doc);
        assert_eq!(
            tree.path_to_body(screw_body),
            Some(vec![
                TreeItemId::ImportedObject(asm),
                TreeItemId::ImportedObject(screw),
            ])
        );
        let row = tree.nodes()[0]
            .children
            .iter()
            .find(|n| n.id == TreeItemId::ImportedObject(generic))
            .expect("the generic instance's row");
        assert_eq!(row.label, "Anet v1-body");
        assert_eq!(row.detail.as_deref(), Some("Instance of part Anet v1-body"));
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
            annotation: None,
            layers: Vec::new(),
        }
    }

    /// An instance whose only child is the assembly it instances shows as
    /// ONE row (named for the instance, carrying the assembly's children,
    /// keeping the instance's identity), and the row says what it is.
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
        set_graph(
            &mut doc,
            vec![
                node(root, None, vec![inst], K::Assembly, "Top"),
                node(inst, Some(root), vec![asm], K::Instance, "Frame:1"),
                node(asm, Some(inst), vec![leaf_a, leaf_b], K::Assembly, "Frame"),
                node(leaf_a, Some(asm), vec![], K::Instance, "Vertical1:1"),
                node(leaf_b, Some(asm), vec![], K::Instance, "Vertical2:1"),
            ],
        );
        let tree = tree(&doc);
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
}
