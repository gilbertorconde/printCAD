//! The workbench plugin surface: trait, descriptors, tools, and commands.
//!
//! This lives in `core_document` (not the `workbenches` crate) so that
//! third-party workbenches only need to depend on this crate; see
//! `docs/WORKBENCH_GUIDE.md`.

use serde::{Deserialize, Serialize};

use crate::feature::{BodyId, FeatureNode};
use crate::rebuild::RebuildJob;
use crate::runtime::{InputResult, WorkbenchInputEvent, WorkbenchRuntimeContext};
use crate::{Document, FeatureId};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkbenchId(String);

impl WorkbenchId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for WorkbenchId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// A screen-space overlay line segment for constant-thickness visualization.
///
/// Screen-space overlays are rendered as 2D lines in screen coordinates, maintaining
/// constant thickness regardless of zoom or camera rotation. Ideal for grid lines,
/// guides, and reference geometry.
#[derive(Debug, Clone)]
pub struct ScreenSpaceOverlay {
    /// Starting point in screen coordinates (x, y) in pixels, relative to viewport origin.
    pub start: [f32; 2],
    /// Ending point in screen coordinates (x, y) in pixels, relative to viewport origin.
    pub end: [f32; 2],
    /// RGB color [r, g, b] in range 0.0-1.0.
    pub color: [f32; 3],
    /// Line thickness in pixels (constant screen-space).
    pub thickness: f32,
    /// Opacity 0..=1.
    pub alpha: f32,
    /// `(dash, gap)` in pixels; the painter dashes after projection so the
    /// pattern stays zoom-independent. `None` draws solid.
    pub dash: Option<(f32, f32)>,
}

impl ScreenSpaceOverlay {
    /// Create a new solid, opaque screen-space overlay line.
    pub fn new(start: [f32; 2], end: [f32; 2], color: [f32; 3], thickness: f32) -> Self {
        Self {
            start,
            end,
            color,
            thickness,
            alpha: 1.0,
            dash: None,
        }
    }

    pub fn with_alpha(mut self, alpha: f32) -> Self {
        self.alpha = alpha;
        self
    }

    pub fn dashed(mut self, dash: f32, gap: f32) -> Self {
        self.dash = Some((dash, gap));
        self
    }
}

/// A screen-space point marker or glyph drawn in the viewport.
#[derive(Debug, Clone)]
pub struct ScreenSpaceMark {
    /// Center in screen coordinates (x, y) in pixels, relative to the
    /// viewport origin.
    pub pos: [f32; 2],
    /// RGB color [r, g, b] in range 0.0-1.0.
    pub color: [f32; 3],
    /// Opacity 0..=1.
    pub alpha: f32,
    pub kind: MarkKind,
}

/// What a [`ScreenSpaceMark`] draws.
#[derive(Debug, Clone)]
pub enum MarkKind {
    /// A filled circle of `radius` pixels.
    Dot { radius: f32 },
    /// An icon from the design system's set, `size` pixels square.
    Icon { name: &'static str, size: f32 },
    /// Two crossing 1px lines, `size` pixels long.
    Crosshair { size: f32 },
}

impl ScreenSpaceMark {
    pub fn dot(pos: [f32; 2], radius: f32, color: [f32; 3]) -> Self {
        Self {
            pos,
            color,
            alpha: 1.0,
            kind: MarkKind::Dot { radius },
        }
    }

    pub fn icon(pos: [f32; 2], name: &'static str, size: f32, color: [f32; 3]) -> Self {
        Self {
            pos,
            color,
            alpha: 1.0,
            kind: MarkKind::Icon { name, size },
        }
    }

    pub fn crosshair(pos: [f32; 2], size: f32, color: [f32; 3]) -> Self {
        Self {
            pos,
            color,
            alpha: 1.0,
            kind: MarkKind::Crosshair { size },
        }
    }

    pub fn with_alpha(mut self, alpha: f32) -> Self {
        self.alpha = alpha;
        self
    }
}

/// A screen-space text label rendered in the viewport (dimension values,
/// constraint glyphs, on-view parameter readouts).
#[derive(Debug, Clone)]
pub struct ScreenSpaceLabel {
    /// Label center in screen coordinates (x, y) in pixels, relative to the
    /// viewport origin.
    pub pos: [f32; 2],
    pub text: String,
    /// RGB color [r, g, b] in range 0.0-1.0.
    pub color: [f32; 3],
    /// Font size in pixels.
    pub size: f32,
    /// Draw a rounded background pill behind the text (dimension values).
    pub background: bool,
    /// Set in the monospace face: numbers, units and identifiers.
    pub mono: bool,
}

impl ScreenSpaceLabel {
    pub fn new(pos: [f32; 2], text: impl Into<String>, color: [f32; 3], size: f32) -> Self {
        Self {
            pos,
            text: text.into(),
            color,
            size,
            background: false,
            mono: false,
        }
    }

    pub fn pill(mut self) -> Self {
        self.background = true;
        self
    }

    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }
}

/// The tool hint shown in the viewport's top-left corner while a tool is
/// active: what it is and what the next click does.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolHint {
    pub icon: &'static str,
    pub name: String,
    pub prompt: String,
    /// `(key, meaning)` chips, e.g. `("Esc", "cancel")`.
    pub keys: Vec<(String, &'static str)>,
}

/// One row of the on-view parameter widget beside the cursor.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OvpRow {
    /// Short mono label: `Ø`, `L`, `∠`.
    pub label: &'static str,
    pub value: String,
    pub unit: &'static str,
    /// Keyboard input goes to this row.
    pub focused: bool,
    /// The user typed a value; it will not follow the cursor.
    pub locked: bool,
}

/// The on-view parameter widget: typed dimensions while drawing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OvpWidget {
    /// Top-left anchor in viewport pixels.
    pub anchor: [f32; 2],
    pub rows: Vec<OvpRow>,
    pub hint: &'static str,
}

/// Everything a workbench wants drawn as widgets over the viewport.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewportHud {
    pub tool: Option<ToolHint>,
    /// Top-right badge: `(rgb, text)`, e.g. the solver's degrees of freedom.
    pub badge: Option<([f32; 3], String)>,
    /// Bottom-left legend of `(rgb, label)` swatches.
    pub legend: Vec<([f32; 3], &'static str)>,
    /// Bottom-right mono readouts.
    pub footer: Vec<String>,
    pub ovp: Option<OvpWidget>,
}

/// A workbench's contribution to the status bar.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatusItems {
    /// The state dot and its title at the far left, e.g. the solver state.
    pub state: Option<([f32; 3], String)>,
    /// "Selected: …" summary.
    pub selection: Option<String>,
    /// Cursor coordinates in the workbench's own frame; replaces the world
    /// readout while set.
    pub coords: Option<String>,
    /// Mode label at the far right, e.g. "Sketch edit mode".
    pub mode: Option<String>,
}

/// What the task panel is currently editing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskInfo {
    pub title: String,
    pub icon: &'static str,
    /// Show OK and Cancel; otherwise a single Close.
    pub confirmable: bool,
}

/// Host → workbench: the buttons or keys pressed on the task panel this
/// frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TaskRequest {
    pub accept: bool,
    pub cancel: bool,
}

/// Workbench → host: what the task panel did this frame.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TaskOutcome {
    /// Nothing changed; the task stays open.
    #[default]
    Open,
    /// The task closed keeping its edits; `label` names the undo entry.
    Accepted { label: String },
    /// The task closed and its edits were reverted.
    Cancelled,
}

/// What a workbench is, for the switcher, the menus, the Preferences rail
/// and the registry's lookups.
#[derive(Debug, Clone)]
pub struct WorkbenchDescriptor {
    pub id: WorkbenchId,
    pub label: String,
    pub description: String,
    /// The design set's icon for the bench.
    pub icon: &'static str,
    /// The `FeatureNode::workbench_id` values this bench presents, renders,
    /// picks, edits and deletes. Its own id is one of them whenever it
    /// stores features. Registration fails when two benches claim one.
    pub feature_kinds: Vec<WorkbenchId>,
    /// An edit-session bench: entering it from another bench remembers
    /// that bench as the one to return to when the session ends. Never
    /// the bench a new document lands in.
    pub modal: bool,
}

impl WorkbenchDescriptor {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: WorkbenchId::new(id),
            label: label.into(),
            description: description.into(),
            icon: "workbench-print",
            feature_kinds: Vec::new(),
            modal: false,
        }
    }

    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = icon;
        self
    }

    pub fn feature_kinds<I, S>(mut self, kinds: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.feature_kinds = kinds.into_iter().map(WorkbenchId::new).collect();
        self
    }

    pub fn modal(mut self) -> Self {
        self.modal = true;
        self
    }
}

/// A feature's 3D presence while it is not under edit: what the scene
/// draws for it, with a revision the mesh cache keys on.
#[derive(Debug, Clone)]
pub struct PassiveGeometry {
    pub mesh: kernel_api::TriMesh,
    /// Changes whenever `mesh` would.
    pub revision: u64,
}

/// The cursor and the view it sits in, for picking.
#[derive(Debug, Clone, Copy)]
pub struct ViewportPick {
    pub view_proj: [[f32; 4]; 4],
    /// `(x, y, width, height)` in physical pixels.
    pub viewport: (u32, u32, u32, u32),
    /// Viewport-local, in physical pixels.
    pub cursor: (f32, f32),
}

/// Where a contextual menu is being built, and for what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuScope {
    /// The viewport's right-click menu on a body.
    ViewportBody(BodyId),
    /// A feature row's menu in the tree.
    TreeFeature(FeatureId),
    /// A body row's menu in the tree.
    TreeBody(BodyId),
    /// The start page's New cards: each item is a way to begin a document.
    StartPage,
    /// The Edit menu's clipboard entries, run on the active bench:
    /// `edit.cut`, `edit.copy`, `edit.paste`.
    EditMenu,
}

/// One entry a bench contributes to a contextual menu. Picking it calls
/// the bench's `on_command` with `id` and the scope it was offered in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub id: String,
    pub label: String,
    pub icon: Option<&'static str>,
    /// A line under the label where the menu has room for one (a start
    /// card), the tooltip where it has not.
    pub hint: Option<String>,
    pub enabled: bool,
    pub separator_before: bool,
}

impl MenuItem {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            hint: None,
            enabled: true,
            separator_before: false,
        }
    }

    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn separator_before(mut self) -> Self {
        self.separator_before = true;
        self
    }
}

/// What the generic property panel needs to know about a bench's feature
/// payloads to show them well: which numeric fields are lengths (so they
/// take the display unit) and which strings name other features or bodies
/// (so they show as names).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PropertyHints {
    pub length_keys: Vec<&'static str>,
    pub reference_keys: Vec<&'static str>,
}

impl PropertyHints {
    pub fn is_length(&self, key: &str) -> bool {
        self.length_keys.contains(&key)
    }

    pub fn is_reference(&self, key: &str) -> bool {
        self.reference_keys.contains(&key)
    }

    /// Both lists, without duplicates.
    pub fn merge(&mut self, other: PropertyHints) {
        for key in other.length_keys {
            if !self.length_keys.contains(&key) {
                self.length_keys.push(key);
            }
        }
        for key in other.reference_keys {
            if !self.reference_keys.contains(&key) {
                self.reference_keys.push(key);
            }
        }
    }
}

/// How a feature shows up outside its bench: the tree row, the property
/// panel, the hover card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureInfo {
    /// The design set's icon for the row.
    pub icon: &'static str,
    /// What this particular feature is: "Pad", "Datum plane", "Sketch".
    pub kind_label: String,
    /// The family the tree's Kind row names: "Part design feature".
    pub family_label: String,
    /// The feature contributes to its body's solid; the hover card names
    /// a body after the last such feature.
    pub builds_solid: bool,
}

impl FeatureInfo {
    /// What a feature of a kind no bench claims looks like.
    pub fn fallback(node: &FeatureNode) -> Self {
        let family = format!(
            "{} feature",
            node.workbench_id
                .as_str()
                .trim_start_matches("wb.")
                .replace(['-', '_'], " ")
        );
        Self {
            icon: "tree-feature",
            kind_label: family.clone(),
            family_label: family,
            builds_solid: false,
        }
    }
}

/// Trait implemented by all workbench plugins.
///
/// Workbenches declare their tools via `configure`, and can optionally
/// implement runtime hooks for input handling, per-frame updates, and custom UI.
pub trait Workbench: Send {
    /// Returns metadata describing this workbench.
    fn descriptor(&self) -> WorkbenchDescriptor;

    /// How a feature of one of this bench's `feature_kinds` presents. The
    /// registry calls it on the owning bench whichever bench is active.
    fn feature_info(&self, node: &FeatureNode) -> FeatureInfo {
        FeatureInfo::fallback(node)
    }

    /// While this bench has an edit session open (`editing_feature` is
    /// `Some`) the view stays square to its plane: orbit off, pan, zoom
    /// and roll on.
    fn locks_view_to_plane(&self) -> bool {
        false
    }

    /// Whether the bench is taking a typed number from the keyboard right
    /// now (a length while drawing). Bare digits, `.`, `,` and `-` stay
    /// with it rather than running their shortcuts.
    fn takes_numeric_input(&self) -> bool {
        false
    }

    /// What the scene draws for an owned feature that is visible and not
    /// under edit. Called on the owner whichever bench is active; the host
    /// colours it and skips the feature under edit.
    fn passive_geometry(
        &self,
        _document: &Document,
        _id: FeatureId,
        _node: &FeatureNode,
    ) -> Option<PassiveGeometry> {
        None
    }

    /// How far, in pixels, the cursor is from an owned feature, when it is
    /// close enough to count as over it. `None` when it is not.
    fn pick_feature(
        &self,
        _document: &Document,
        _id: FeatureId,
        _node: &FeatureNode,
        _pick: &ViewportPick,
    ) -> Option<f32> {
        None
    }

    /// What this bench offers in a contextual menu, computed while the
    /// menu is open. Every bench is asked, whichever is active.
    fn menu_items(&self, _scope: &MenuScope, _document: &Document) -> Vec<MenuItem> {
        Vec::new()
    }

    /// Run one of this bench's `menu_items` ids in the scope it was
    /// offered in. `false` when the id is not one of this bench's.
    fn on_command(
        &mut self,
        _id: &str,
        _scope: &MenuScope,
        _ctx: &mut WorkbenchRuntimeContext,
    ) -> bool {
        false
    }

    /// Run one of the commands this bench registered with
    /// `WorkbenchContext::register_command`. The host has checked `args`
    /// against the command's spec. A command that makes something answers
    /// its id; it never opens a task or waits for a click.
    fn run_command(
        &mut self,
        id: &str,
        _args: &crate::CommandArgs,
        _ctx: &mut WorkbenchRuntimeContext,
    ) -> crate::CommandResult {
        Err(crate::CommandError::Unknown(id.to_string()))
    }

    /// What the generic property panel should know about this bench's
    /// feature payloads.
    fn property_hints(&self) -> PropertyHints {
        PropertyHints::default()
    }

    /// The bodies whose derived solid this bench must rebuild now, each
    /// with its plan. Called on every bench each frame. The bench settles
    /// the dirty flags of every feature a plan consumed before returning,
    /// or the same job comes back every frame.
    fn rebuild_jobs(&self, _document: &mut Document) -> Vec<RebuildJob> {
        Vec::new()
    }

    /// The body's history changed shape (a feature left it, its tip
    /// moved): rebuild it from the start, or drop its derived solid when
    /// no history is left.
    fn invalidate_body(&self, _document: &mut Document, _body: BodyId) {}

    /// Every derived solid this bench produces is stale.
    fn invalidate_all(&self, _document: &mut Document) {}

    /// Called once at registration to declare tools.
    fn configure(&self, context: &mut WorkbenchContext);

    /// Called when this workbench becomes active.
    fn on_activate(&mut self, _ctx: &mut WorkbenchRuntimeContext) {}

    /// Called when this workbench is deactivated (another WB becomes active).
    fn on_deactivate(&mut self, _ctx: &mut WorkbenchRuntimeContext) {}

    /// Called every frame while this workbench is active.
    fn on_frame(&mut self, _dt: f32, _ctx: &mut WorkbenchRuntimeContext) {}

    /// Called when an input event occurs while this workbench is active.
    /// Return `InputResult::consumed()` to prevent further event propagation.
    fn on_input(
        &mut self,
        _event: &WorkbenchInputEvent,
        _active_tool: Option<&str>,
        _ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        InputResult::ignored()
    }

    /// Draw custom UI in the left panel (below the tool list).
    /// Called every frame while this workbench is active.
    #[cfg(feature = "egui")]
    fn ui_left_panel(&mut self, _ui: &mut egui::Ui, _ctx: &mut WorkbenchRuntimeContext) {}

    /// What the task panel is editing, if anything. `Some` opens the panel.
    fn task(&self, _ctx: &WorkbenchRuntimeContext) -> Option<TaskInfo> {
        None
    }

    /// Draw the task panel body and react to the host's accept/cancel.
    #[cfg(feature = "egui")]
    fn ui_task_panel(
        &mut self,
        _ui: &mut egui::Ui,
        _ctx: &mut WorkbenchRuntimeContext,
        _request: TaskRequest,
    ) -> TaskOutcome {
        TaskOutcome::Open
    }

    /// Widgets to draw over the viewport this frame.
    /// The keys in effect for this workbench's tools and actions, by id,
    /// after the user's changes; an id with no key is absent. Called once
    /// the workbenches are registered and whenever the keys change, so a
    /// workbench that names its keys in hints can name the right ones.
    fn shortcuts_changed(
        &mut self,
        _keys: &std::collections::HashMap<String, Vec<crate::shortcut::Chord>>,
    ) {
    }

    fn viewport_hud(&self, _ctx: &WorkbenchRuntimeContext) -> Option<ViewportHud> {
        None
    }

    /// The workbench's status-bar items this frame.
    fn status_items(&self, _ctx: &WorkbenchRuntimeContext) -> Option<StatusItems> {
        None
    }

    /// The feature whose edit session is open, for the tree to badge.
    fn editing_feature(&self) -> Option<FeatureId> {
        None
    }

    /// Whether an Action tool that acts as a toggle is currently on, so its
    /// button can render pressed.
    fn tool_toggled(&self, _tool_id: &str) -> bool {
        false
    }

    /// Check if a tool is enabled given the current runtime context.
    /// Called by the UI to determine if a tool button should be enabled/disabled.
    /// Default implementation returns true for all tools.
    fn is_tool_enabled(&self, _tool_id: &str, _ctx: &WorkbenchRuntimeContext) -> bool {
        true
    }

    /// Draw custom settings UI in the Settings window.
    /// Called when the Settings window is open and this workbench's tab is selected.
    /// `filter` is the dialog's lowercase search text; rows that do not
    /// match it stay hidden (`ui_kit::widgets::pref_group` applies it).
    #[cfg(feature = "egui")]
    fn ui_settings(&mut self, _ui: &mut egui::Ui, _filter: &str) -> bool {
        false // Return true if settings changed
    }

    /// Finish/close the current editing session (e.g., finish sketch).
    /// Called when the user requests to finish editing (e.g., via UI button).
    fn finish_editing(&mut self, _ctx: &mut WorkbenchRuntimeContext) {}

    /// This bench's own settings, for the host to keep in the user's
    /// settings file without reading them. `None` when it has none.
    fn settings_json(&self) -> Option<serde_json::Value> {
        None
    }

    /// Settings `settings_json` produced earlier, back from the file.
    fn apply_settings_json(&mut self, _value: &serde_json::Value) {}

    /// Hand the host this bench's editing state for the document on
    /// screen, leaving the bench as if no document were open. The host
    /// keeps it with the tab and gives it back through `resume_session`
    /// when that tab returns. A bench with no such state returns `None`.
    fn suspend_session(&mut self) -> Option<Box<dyn std::any::Any + Send>> {
        None
    }

    /// Take back a state `suspend_session` produced, or start from
    /// nothing when `state` is `None` (a new tab).
    fn resume_session(&mut self, _state: Option<Box<dyn std::any::Any + Send>>) {}

    /// Remove an owned feature and settle what depended on it: features
    /// it hid come back, its body rebuilds. `false` when nothing was
    /// removed.
    fn delete_feature(&mut self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) -> bool {
        ctx.document.remove_feature(id).is_ok()
    }

    /// Get additional render meshes for overlay/helper visualization.
    /// Called every frame to allow workbenches to contribute visual aids (grid lines, guides, etc.).
    /// Returns a vector of (mesh, color, is_wireframe) tuples where:
    /// - mesh: The triangular mesh to render
    /// - color: RGB color [r, g, b] in range 0.0-1.0
    /// - is_wireframe: If true, render as wireframe with depth bias (appears on top of solid geometry)
    ///
    /// These meshes are rendered in 3D world space and will scale with zoom and rotate with the camera.
    /// For constant-thickness lines that don't change with zoom/rotation, use `get_screen_space_overlays` instead.
    /// Default implementation returns empty vector.
    fn get_overlay_meshes(
        &self,
        _ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<(kernel_api::TriMesh, [f32; 3], bool)> {
        Vec::new()
    }

    /// Get screen-space overlays for constant-thickness visualization.
    /// Called every frame to allow workbenches to contribute visual aids that maintain
    /// constant screen-space thickness regardless of zoom or camera rotation.
    ///
    /// Screen-space overlays are rendered as 2D lines in screen coordinates, making them
    /// ideal for grid lines, guides, and other reference geometry that should remain visible
    /// and maintain consistent appearance regardless of camera position.
    ///
    /// Returns a vector of screen-space line segments where:
    /// - start: Starting point in screen coordinates (x, y) in pixels, relative to viewport origin
    /// - end: Ending point in screen coordinates (x, y) in pixels, relative to viewport origin
    /// - color: RGB color [r, g, b] in range 0.0-1.0
    /// - thickness: Line thickness in pixels (constant screen-space)
    ///
    /// Default implementation returns empty vector.
    fn get_screen_space_overlays(
        &self,
        _ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<ScreenSpaceOverlay> {
        Vec::new()
    }

    /// Get screen-space text labels for constant-size viewport annotations
    /// (dimension values, constraint glyphs, on-view parameter readouts).
    /// Same coordinate convention as [`Self::get_screen_space_overlays`].
    fn get_screen_space_labels(
        &self,
        _ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<ScreenSpaceLabel> {
        Vec::new()
    }

    /// Point markers and icon glyphs drawn in the viewport, on top of the
    /// overlay lines and beneath the labels. Same coordinate convention as
    /// [`Self::get_screen_space_overlays`].
    fn get_screen_space_marks(
        &self,
        _ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<ScreenSpaceMark> {
        Vec::new()
    }
}

/// Registry used by workbenches to declare the tools they expose.
#[derive(Debug, Default)]
pub struct WorkbenchContext {
    tools: Vec<ToolDescriptor>,
    actions: Vec<crate::shortcut::ActionDescriptor>,
    commands: Vec<crate::CommandSpec>,
}

impl WorkbenchContext {
    pub fn register_tool(&mut self, tool: ToolDescriptor) {
        self.tools.push(tool);
    }

    pub fn tools(&self) -> &[ToolDescriptor] {
        &self.tools
    }

    /// Offer a keyboard action that is not a tool. Its shortcut, while the
    /// workbench is active, sends `WorkbenchInputEvent::Action`.
    pub fn register_action(&mut self, action: crate::shortcut::ActionDescriptor) {
        self.actions.push(action);
    }

    pub fn actions(&self) -> &[crate::shortcut::ActionDescriptor] {
        &self.actions
    }

    /// Offer a command to scripts and other callers; it runs in
    /// `Workbench::run_command`.
    pub fn register_command(&mut self, command: crate::CommandSpec) {
        self.commands.push(command);
    }

    pub fn commands(&self) -> &[crate::CommandSpec] {
        &self.commands
    }
}

/// Describes how a tool button should behave in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolBehavior {
    /// Radio button behavior: only one tool in the same group can be active at a time.
    /// Clicking an active tool deactivates it. Tools in different groups are independent.
    /// This is the default.
    #[default]
    Radio,
    /// Check button behavior: independent toggle. Each tool can be on or off independently.
    /// Multiple check tools can be active simultaneously.
    Check,
    /// Action button behavior: fire-and-forget. Clicking triggers the action
    /// but doesn't keep the tool "active". The tool is cleared after handling.
    Action,
}

/// Describes an interactive tool contributed by a workbench.
#[derive(Debug, Clone)]
pub struct ToolDescriptor {
    pub id: String,
    pub label: String,
    /// Optional category for grouping/organization (e.g., "drawing", "modeling", "utility").
    /// This is informational and doesn't affect behavior.
    pub category: Option<String>,
    /// How the tool button should behave in the UI.
    pub behavior: ToolBehavior,
    /// Optional group name for Radio tools. Tools in the same group are mutually exclusive.
    /// Only one tool per group can be active at a time. If None, each tool is its own group.
    /// Ignored for Check and Action tools.
    pub group: Option<String>,
    /// Name of the tool's icon in the design system's set.
    pub icon: Option<&'static str>,
    /// Present in the design but not built: the button renders disabled and
    /// this note is its tooltip.
    pub planned: Option<&'static str>,
    /// Alternatives offered from a dropdown on the button. A picked variant
    /// activates `"{id}:{variant.id}"`; the button remembers the last pick.
    pub variants: Vec<ToolVariant>,
    /// Toolbar row: 0 shares the row with the standard tools, 1 and 2 are
    /// the workbench's own rows.
    pub row: u8,
    /// Push the button to the far right of its row.
    pub align_end: bool,
    /// Default keys that activate the tool while its workbench is active,
    /// as a click on its button would. The user can rebind them.
    pub shortcuts: Vec<crate::shortcut::Chord>,
}

/// One entry of a tool's variant dropdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolVariant {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
    /// Present in the design but not built.
    pub planned: Option<&'static str>,
}

impl ToolVariant {
    pub const fn new(id: &'static str, label: &'static str, icon: &'static str) -> Self {
        Self {
            id,
            label,
            icon,
            planned: None,
        }
    }

    pub const fn planned(mut self, note: &'static str) -> Self {
        self.planned = Some(note);
        self
    }
}

/// The base tool id of an activated id, without a `:variant` suffix.
pub fn base_tool_id(id: &str) -> &str {
    id.split_once(':').map_or(id, |(base, _)| base)
}

/// The variant suffix of an activated id, if any.
pub fn tool_variant(id: &str) -> Option<&str> {
    id.split_once(':').map(|(_, v)| v)
}

impl ToolDescriptor {
    /// Create a new tool descriptor with radio button behavior (default).
    /// Tools in the same group are mutually exclusive.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        category: Option<impl Into<String>>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            category: category.map(|c| c.into()),
            behavior: ToolBehavior::Radio,
            group: None, // Each tool is its own group by default
            icon: None,
            planned: None,
            variants: Vec::new(),
            row: 1,
            align_end: false,
            shortcuts: Vec::new(),
        }
    }

    /// Create a new tool descriptor with radio button behavior in a specific group.
    /// Tools in the same group are mutually exclusive.
    pub fn new_radio_group(
        id: impl Into<String>,
        label: impl Into<String>,
        category: Option<impl Into<String>>,
        group: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            category: category.map(|c| c.into()),
            behavior: ToolBehavior::Radio,
            group: Some(group.into()),
            icon: None,
            planned: None,
            variants: Vec::new(),
            row: 1,
            align_end: false,
            shortcuts: Vec::new(),
        }
    }

    /// Create a new tool descriptor with check button behavior.
    /// Check tools are independent - multiple can be active simultaneously.
    pub fn new_check(
        id: impl Into<String>,
        label: impl Into<String>,
        category: Option<impl Into<String>>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            category: category.map(|c| c.into()),
            behavior: ToolBehavior::Check,
            group: None, // Groups don't apply to Check tools
            icon: None,
            planned: None,
            variants: Vec::new(),
            row: 1,
            align_end: false,
            shortcuts: Vec::new(),
        }
    }

    /// Create a new tool descriptor with action button behavior.
    pub fn new_action(
        id: impl Into<String>,
        label: impl Into<String>,
        category: Option<impl Into<String>>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            category: category.map(|c| c.into()),
            behavior: ToolBehavior::Action,
            group: None, // Groups don't apply to Action tools
            icon: None,
            planned: None,
            variants: Vec::new(),
            row: 1,
            align_end: false,
            shortcuts: Vec::new(),
        }
    }

    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn planned(mut self, note: &'static str) -> Self {
        self.planned = Some(note);
        self
    }

    pub fn variants(mut self, variants: Vec<ToolVariant>) -> Self {
        self.variants = variants;
        self
    }

    pub fn row(mut self, row: u8) -> Self {
        self.row = row;
        self
    }

    /// Add a default key, such as `"L"` or `"Ctrl+Shift+R"`.
    ///
    /// # Panics
    ///
    /// When `chord` does not parse: a default key is written in the code,
    /// and a typo there is a bug to catch at registration.
    pub fn shortcut(mut self, chord: &str) -> Self {
        self.shortcuts.push(crate::shortcut::parse_default(chord));
        self
    }

    pub fn align_end(mut self) -> Self {
        self.align_end = true;
        self
    }
}
