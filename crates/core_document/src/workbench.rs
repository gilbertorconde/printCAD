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

mod tools;
pub use tools::BenchTools;

mod panels;
pub use panels::BenchPanels;

mod build;
pub use build::BenchBuild;

mod parameters;
pub use parameters::BenchParameters;

mod output;
pub use output::BenchOutput;

mod commands;
pub use commands::BenchCommands;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct WorkbenchId(String);

/// An id read back from a file, a log or a message is called what it is
/// now (`renamed::workbench`).
impl<'de> Deserialize<'de> for WorkbenchId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        Ok(Self(crate::renamed::workbench(&id).to_owned()))
    }
}

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

/// A picture laid over the viewport, under the bench's lines: its pixels
/// and where its corners land on screen.
#[derive(Debug, Clone)]
pub struct ScreenSpaceImage {
    /// Names the pixels: one key, one picture, so the host uploads it once.
    pub key: u64,
    /// Width and height in pixels.
    pub size: [u32; 2],
    /// Row by row from the top, RGBA.
    pub rgba: std::sync::Arc<Vec<u8>>,
    /// The picture's top left, top right, bottom right and bottom left, in
    /// viewport pixels.
    pub corners: [[f32; 2]; 4],
    /// 0..=1.
    pub opacity: f32,
}

/// A line segment drawn in viewport pixels, the same thickness at any zoom
/// or camera rotation.
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
    /// How far into the dash pattern, in pixels, this segment starts: a
    /// polyline's segments carry the pattern on from one to the next.
    pub dash_offset: f32,
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
            dash_offset: 0.0,
        }
    }

    /// The segments of one projected polyline, `None` where a point does
    /// not project. A dash pattern runs on along the whole path, each
    /// segment starting where the one before left off: a curve drawn as
    /// segments shorter than one dash still reads dashed.
    pub fn polyline(
        points: impl IntoIterator<Item = Option<[f32; 2]>>,
        color: [f32; 3],
        thickness: f32,
        dash: Option<(f32, f32)>,
    ) -> Vec<Self> {
        let period = dash.map_or(0.0, |(on, off)| on + off);
        let mut travelled = 0.0_f32;
        let mut prev: Option<[f32; 2]> = None;
        let mut out = Vec::new();
        for point in points {
            if let (Some(a), Some(b)) = (prev, point) {
                let mut segment = Self::new(a, b, color, thickness);
                segment.dash = dash;
                if period > 0.0 {
                    segment.dash_offset = travelled % period;
                    travelled += (b[0] - a[0]).hypot(b[1] - a[1]);
                }
                out.push(segment);
            }
            prev = point;
        }
        out
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

/// A filled polygon drawn in viewport pixels, beneath the overlay lines: a
/// triangle fan from its first point, so it is convex or a sector seen from
/// its centre (a handle's cone, a swept angle).
#[derive(Debug, Clone)]
pub struct ScreenSpacePolygon {
    /// Corners in screen coordinates (x, y) in pixels, relative to the
    /// viewport origin; the triangles are `(0, i, i + 1)`.
    pub points: Vec<[f32; 2]>,
    /// RGB color [r, g, b] in range 0.0-1.0.
    pub color: [f32; 3],
    /// Opacity 0..=1.
    pub alpha: f32,
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
    /// A filled arrowhead, its tip at the mark's position, pointing along
    /// `way` (of unit length on screen), `length` pixels from tip to base
    /// and `width` pixels across the base.
    Arrowhead {
        way: [f32; 2],
        length: f32,
        width: f32,
    },
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

/// The clicks of an element in progress, pinned on the view: each a dot
/// with its number in a pill beside it, `color` both, the same size on
/// screen at any zoom (`pixels_per_point` is the UI's scale). Each pin is
/// a viewport position and the number it shows.
pub fn numbered_pins(
    pins: impl IntoIterator<Item = ([f32; 2], usize)>,
    color: [f32; 3],
    pixels_per_point: f32,
) -> (Vec<ScreenSpaceMark>, Vec<ScreenSpaceLabel>) {
    /// The dot's radius, the number's offset from it and its size, in
    /// logical pixels.
    const DOT: f32 = 4.0;
    const OFFSET: [f32; 2] = [10.0, -10.0];
    const TEXT: f32 = 11.0;
    let s = pixels_per_point;
    pins.into_iter()
        .map(|([x, y], number)| {
            (
                ScreenSpaceMark::dot([x, y], DOT * s, color),
                ScreenSpaceLabel::new(
                    [x + OFFSET[0] * s, y + OFFSET[1] * s],
                    number.to_string(),
                    color,
                    TEXT * s,
                )
                .pill()
                .mono(),
            )
        })
        .unzip()
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
    /// The values a tool draws with, typed through the bench's keys: a
    /// readout card (one with a `hint`) the host draws as it draws
    /// [`Workbench::floating_card`]'s, beside its `screen` point, and only
    /// while that offers no card, since one card shows at a time.
    pub card: Option<panel_api::FloatingCard>,
}

/// An input of a feature that can be swapped for another, as the property
/// panel offers it.
#[derive(Debug, Clone, PartialEq)]
pub struct FeatureReference {
    /// Handed back to [`Workbench::set_reference`].
    pub key: String,
    pub label: String,
    /// What it reads now, in words.
    pub current: String,
    /// The feature it reads now, when it reads one.
    pub selected: Option<FeatureId>,
    /// The features it may read instead, with their names.
    pub choices: Vec<(FeatureId, String)>,
    /// A flat face picked in the view can stand in for a feature.
    pub takes_face: bool,
    /// The feature data's fields this input stands for, left out of the
    /// plain rows beside it.
    pub fields: Vec<String>,
}

/// What an input is pointed at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReferenceChoice {
    Feature(FeatureId),
    /// The face selected in the view.
    SelectedFace,
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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TaskInfo {
    pub title: String,
    /// Under the title in the panel's header: what it edits or where it
    /// sits ("Body › on XY plane"); empty for none.
    pub subtitle: String,
    /// The state the header shows beside the title, as a small pill.
    pub status: Option<TaskStatus>,
    pub icon: &'static str,
    /// Show OK and Cancel; otherwise a single Close.
    pub confirmable: bool,
    /// Each edit made in the task is an undo step of its own (a sketch's
    /// editing session, where Ctrl+Z takes back the last line drawn);
    /// otherwise the whole task is one step, as Cancel treats it.
    pub stepwise: bool,
}

/// A task's state as its panel's header shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStatus {
    /// A few words: "1 degree of freedom left".
    pub text: String,
    /// The longer reason, on hover.
    pub detail: String,
    pub tone: StatusTone,
}

/// How a [`TaskStatus`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusTone {
    #[default]
    Info,
    Good,
    Warning,
    Bad,
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
    /// Feature kinds, among `feature_kinds`, whose bodies are bought
    /// rather than made (catalog hardware): the parts list counts such a
    /// body bought unless the user says it is made, and exports and the
    /// slicer leave it out as they do any bought part.
    pub bought_kinds: Vec<WorkbenchId>,
    /// The tools and commands, by id, the radial menu offers while the
    /// bench is active (a right press held still), clockwise from the top,
    /// up to [`RADIAL_SLOTS`]. The user's own choice in Preferences takes
    /// its place; empty leaves the bench's most used tools.
    pub radial: Vec<String>,
}

/// How many slices the radial menu holds.
pub const RADIAL_SLOTS: usize = 8;

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
            bought_kinds: Vec::new(),
            radial: Vec::new(),
        }
    }

    pub fn radial<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.radial = ids.into_iter().map(Into::into).take(RADIAL_SLOTS).collect();
        self
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

    pub fn bought_kinds<I, S>(mut self, kinds: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.bought_kinds = kinds.into_iter().map(WorkbenchId::new).collect();
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
    /// The colour it draws in while not selected or hovered.
    pub tint: PassiveTint,
    /// What it encloses, shaded behind its lines, one entry per region:
    /// the host meshes each profile (once per `revision`) and draws it
    /// see-through in the same colour, a region picked or under the
    /// cursor in the selection or hover paint. Never picked by the GPU.
    pub regions: Vec<PassiveRegion>,
}

/// What a feature lays out on its plane, in its body's own frame: chains
/// of curves joined end to end, open or closed, each walked from its first
/// segment, and points that stand alone.
#[derive(Debug, Clone, Default)]
pub struct FeaturePaths {
    pub plane: kernel_api::ProfilePlane,
    pub chains: Vec<kernel_api::ProfileWire>,
    /// In the plane's own coordinates.
    pub points: Vec<[f64; 2]>,
}

/// Closed loops a feature's lines bound, for the host to shade.
#[derive(Debug, Clone)]
pub struct PassiveRegion {
    /// In the feature's body frame, as the mesh is.
    pub profile: kernel_api::Profile,
    pub opacity: f32,
    /// The part of the feature this shades, when it can be picked on its
    /// own ([`Workbench::pick_part`]).
    pub part: Option<FeaturePart>,
}

/// A piece of a feature picked in the view on its own: one of the regions
/// a sketch's curves close, or one of its curves. The owning bench names
/// it; the host only keeps and compares the names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FeaturePart {
    /// A closed region, by the bench's name for it.
    Region(String),
    /// A curve, by the bench's id for it.
    Edge(String),
}

/// A part of a feature under the cursor and where on it the cursor's ray
/// meets it, in the feature's body frame (the document's, once the
/// registry places it), so the host can tell what stands in front.
#[derive(Debug, Clone, PartialEq)]
pub struct PartPick {
    pub part: FeaturePart,
    pub point: [f32; 3],
}

/// Which colour of the sketch palette a feature's 3D presence draws in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PassiveTint {
    /// The feature's own geometry.
    #[default]
    Plain,
    /// Geometry that stands for something outside its body, drawn in the
    /// palette's external colour.
    External,
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
    /// A component row's menu in the tree.
    TreeComponent(crate::ComponentId),
    /// The start page's New cards: each item is a way to begin a document.
    StartPage,
    /// The Edit menu's clipboard entries, run on the active bench:
    /// `edit.cut`, `edit.copy`, `edit.paste`.
    EditMenu,
    /// A menu the bench opened itself at the pointer
    /// ([`HostRequest::OpenMenu`](crate::HostRequest::OpenMenu)), by the
    /// tag it gave it: only that bench hears its pick.
    Popup(String),
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
    /// Drawn ticked: the choice that stands among the entries.
    pub checked: bool,
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
            checked: false,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
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
    /// The family the tree's Kind row names: "Design feature".
    pub family_label: String,
    /// The feature contributes to its body's solid; the hover card names
    /// a body after the last such feature.
    pub builds_solid: bool,
}

/// How a feature's work stands, as its bench says: a badge beside its
/// name in the tree.
#[derive(Debug, Clone, PartialEq)]
pub struct FeatureStatus {
    pub state: FeatureState,
    /// While running, how far, 0 to 1, when known.
    pub fraction: Option<f32>,
    /// More words, shown when the pointer rests on the badge.
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureState {
    NotRun,
    Running,
    Done,
    Warnings,
    Failed,
    OutOfDate,
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

    /// What a feature of a kind no loaded bench claims looks like: the
    /// fallback, naming the package that made it when one did.
    pub fn unowned(node: &FeatureNode) -> Self {
        let mut info = Self::fallback(node);
        if let Some(by) = &node.made_by {
            info.family_label = format!("Needs {by}");
        }
        info
    }

    /// Why a feature no loaded bench claims cannot be edited, when a
    /// package made it.
    pub fn missing_package(node: &FeatureNode) -> Option<String> {
        node.made_by.as_ref().map(|by| {
            let get = match &node.package_source {
                Some(repo) => {
                    format!(" Install it from github.com/{repo} (the feature's menu offers to).")
                }
                None => String::new(),
            };
            format!(
                "Made by the workbench {by}, which is not installed. It keeps its data, \
                 and its body keeps the shape it was saved with.{get}"
            )
        })
    }

    /// The package a feature no loaded bench claims needs: its id, and
    /// where it is published when that is known.
    pub fn needed_package(node: &FeatureNode) -> Option<(String, Option<String>)> {
        let id = node
            .made_by
            .as_ref()?
            .split_whitespace()
            .next()?
            .to_string();
        Some((id, node.package_source.clone()))
    }
}

/// One part of a print: the bodies of one shape (the first the one whose
/// shape is printed), its name, and how many of it to print.
#[derive(Debug, Clone, PartialEq)]
pub struct PrintPart {
    pub name: String,
    pub bodies: Vec<BodyId>,
    pub count: u32,
}

/// `node`'s dependency when it has exactly one.
pub fn sole_dependency(document: &Document, node: &FeatureNode) -> Option<FeatureId> {
    match document.feature_tree().dependencies(node.id).as_slice() {
        [only] => Some(*only),
        _ => None,
    }
}

/// A bench's per-tab state, which the trait's default `suspend_session`
/// and `resume_session` move with the tab: any `Default` value.
pub trait TabSlot: Send {
    /// The state, boxed, leaving its `Default` in its place.
    fn take_out(&mut self) -> Box<dyn std::any::Any + Send>;
    /// A state `take_out` produced put in place, or the `Default` when
    /// there is none or it is of another type.
    fn put_back(&mut self, state: Option<Box<dyn std::any::Any + Send>>);
}

impl<T: Default + Send + 'static> TabSlot for T {
    fn take_out(&mut self) -> Box<dyn std::any::Any + Send> {
        Box::new(std::mem::take(self))
    }

    fn put_back(&mut self, state: Option<Box<dyn std::any::Any + Send>>) {
        *self = state
            .and_then(|s| s.downcast::<T>().ok())
            .map(|s| *s)
            .unwrap_or_default();
    }
}

/// Trait implemented by all workbench plugins.
///
/// Workbenches declare their tools via `configure`, and can optionally
/// implement runtime hooks for input handling, per-frame updates, and the
/// declared panels and cards the host draws. What a bench draws itself
/// with egui (its task panel, Preferences page, left panel and strip under
/// the view) is a drawing-side trait of a host that draws, which reaches
/// the bench as `dyn Any` through the registry's drawing side
/// ([`crate::DocumentService::set_drawing_side`]).
pub trait Workbench:
    std::any::Any
    + Send
    + BenchCommands
    + BenchOutput
    + BenchParameters
    + BenchBuild
    + BenchPanels
    + BenchTools
{
    /// Returns metadata describing this workbench.
    fn descriptor(&self) -> WorkbenchDescriptor;

    /// How a feature of one of this bench's `feature_kinds` presents. The
    /// registry calls it on the owning bench whichever bench is active.
    fn feature_info(&self, node: &FeatureNode) -> FeatureInfo {
        FeatureInfo::fallback(node)
    }

    /// The feature an owned feature consumes, whose row it sits under in
    /// the tree (a pad under its sketch, a sketch under its datum). `None`
    /// puts it at its body's top level in build order: a feature that
    /// works on the whole body, or reads several features without
    /// consuming one. Unless a bench says otherwise, a feature consumes
    /// its one dependency and nothing when it has several.
    fn consumed_feature(&self, document: &Document, node: &FeatureNode) -> Option<FeatureId> {
        sole_dependency(document, node)
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

    /// The curves and points an owned feature lays out, for what is
    /// repeated along or at them (an array of copies along a path or at
    /// points). `None` when it lays out none.
    fn feature_paths(
        &self,
        _document: &Document,
        _id: FeatureId,
        _node: &FeatureNode,
    ) -> Option<FeaturePaths> {
        None
    }

    /// The bounded solid an owned feature offers a mesh boolean as its
    /// operand (a fitted cylinder over its drawn extent), in the
    /// document's frame. `None` when it offers none.
    fn feature_solid(
        &self,
        _document: &Document,
        _id: FeatureId,
    ) -> Option<kernel_api::MeshOperand> {
        None
    }

    /// The plane an owned feature offers to sketch on (a scan's section,
    /// a fitted plane), in the document's frame: Create sketch puts a new
    /// sketch there while the feature is selected. `None` when it offers
    /// none.
    fn feature_plane(
        &self,
        _document: &Document,
        _id: FeatureId,
        _node: &FeatureNode,
    ) -> Option<kernel_api::ProfilePlane> {
        None
    }

    /// A problem an owned feature has that does not stop it building (a
    /// sketch whose projected edges are gone), which the tree warns of on
    /// its row. Asked whenever the tree is drawn, so it answers from what
    /// it worked out last for the same inputs.
    fn feature_warning(
        &self,
        _document: &Document,
        _kernel: Option<&dyn kernel_api::KernelQueries>,
        _id: FeatureId,
    ) -> Option<String> {
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

    /// The part of an owned feature under the cursor: a curve within
    /// `tolerance_px` of it, else the region it lies in. `None` when the
    /// feature has no parts to pick there. Asked only while no task or
    /// edit session is open.
    fn pick_part(
        &self,
        _document: &Document,
        _id: FeatureId,
        _node: &FeatureNode,
        _pick: &ViewportPick,
        _tolerance_px: f32,
    ) -> Option<PartPick> {
        None
    }

    /// The lines one part of an owned feature draws (a curve), in its
    /// body's frame, for the view to draw it picked. `None` for a part
    /// drawn some other way (a region, through `passive_geometry`).
    fn part_lines(
        &self,
        _document: &Document,
        _id: FeatureId,
        _part: &FeaturePart,
    ) -> Option<kernel_api::TriMesh> {
        None
    }

    /// Called once at registration to declare tools, actions, commands and
    /// file imports.
    fn configure(&self, context: &mut WorkbenchContext);

    /// The bench has work running away from the window (a job) whose end
    /// it shows: the host keeps frames coming while it does.
    fn busy(&self) -> bool {
        false
    }

    /// Called when this workbench becomes active.
    fn on_activate(&mut self, _ctx: &mut WorkbenchRuntimeContext) {}

    /// Called when this workbench is deactivated (another WB becomes active).
    fn on_deactivate(&mut self, _ctx: &mut WorkbenchRuntimeContext) {}

    /// Called every frame while this workbench is active.
    fn on_frame(&mut self, _dt: f32, _ctx: &mut WorkbenchRuntimeContext) {}

    /// The files the user picked for this bench's
    /// [`HostRequest::OpenFile`](crate::HostRequest::OpenFile) of `id`,
    /// each its name and bytes; none when the dialog was cancelled.
    fn files_opened(
        &mut self,
        _ctx: &mut WorkbenchRuntimeContext,
        _id: &str,
        _files: Vec<(String, Vec<u8>)>,
    ) {
    }

    /// How the work of a feature of a kind this bench claims stands, when
    /// the bench says.
    fn feature_status(&self, _node: &FeatureNode) -> Option<FeatureStatus> {
        None
    }

    /// A feature of a kind this bench claims, dragged in the tree and
    /// dropped just before (`before`) or after `target`: true when the
    /// bench took it (moved it its own way, or refused), false to leave the
    /// host to move it in its body's history.
    fn feature_dropped(
        &mut self,
        _ctx: &mut WorkbenchRuntimeContext,
        _feature: FeatureId,
        _target: FeatureId,
        _before: bool,
    ) -> bool {
        false
    }

    /// Called every frame while this workbench is not active and
    /// [`Self::busy`]: the work it runs away from the window ends whatever
    /// bench is on screen, and this is where it hears of it.
    fn on_background_frame(&mut self, _ctx: &mut WorkbenchRuntimeContext) {}

    /// The keys in effect for this workbench's tools and actions, by id,
    /// after the user's changes; an id with no key is absent. Called once
    /// the workbenches are registered and whenever the keys change, so a
    /// workbench that names its keys in hints can name the right ones.
    fn shortcuts_changed(
        &mut self,
        _keys: &std::collections::HashMap<String, Vec<crate::shortcut::Chord>>,
    ) {
    }

    /// The feature whose edit session is open, for the tree to badge.
    fn editing_feature(&self) -> Option<FeatureId> {
        None
    }

    /// The user asked to edit `id`, a feature of a kind this bench claims
    /// (a double click on its tree row): open its task. Selecting a feature
    /// alone never opens one; the host has made `id` the active document
    /// object before this call.
    fn edit_feature(&mut self, _ctx: &mut WorkbenchRuntimeContext, _id: FeatureId) {}

    /// Finish/close the current editing session (e.g., finish sketch).
    /// Called when the user requests to finish editing (e.g., via UI button).
    fn finish_editing(&mut self, _ctx: &mut WorkbenchRuntimeContext) {}

    /// The bench's state for the document on screen: everything that is
    /// not a setting or a cache keyed by what it was worked out from,
    /// held in one field (the open task, a gesture in progress, a job
    /// running, a tool armed, a toggle of the view, a value as typed).
    /// The default `suspend_session` and `resume_session` move it with
    /// the tab. `None` for a bench with no such state.
    fn tab_state(&mut self) -> Option<&mut dyn TabSlot> {
        None
    }

    /// Hand the host this bench's editing state for the document on
    /// screen, leaving the bench as if no document were open. The host
    /// keeps it with the tab and gives it back through `resume_session`
    /// when that tab returns. By default the `tab_state` field, left at
    /// its `Default`.
    fn suspend_session(&mut self) -> Option<Box<dyn std::any::Any + Send>> {
        self.tab_state().map(TabSlot::take_out)
    }

    /// Take back a state `suspend_session` produced, or start from
    /// nothing when `state` is `None` (a new tab). By default the
    /// `tab_state` field takes it, or its `Default`.
    fn resume_session(&mut self, state: Option<Box<dyn std::any::Any + Send>>) {
        if let Some(slot) = self.tab_state() {
            slot.put_back(state);
        }
    }

    /// Features of other bodies that act on `body` (a joint holding
    /// another body to it): the tree lists each under `body` too, as a
    /// link to the feature.
    fn linked_features(&self, _document: &Document, _body: BodyId) -> Vec<FeatureId> {
        Vec::new()
    }

    /// Remove an owned feature and settle what depended on it: features
    /// it hid come back, its body rebuilds. `false` when nothing was
    /// removed.
    fn delete_feature(&mut self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) -> bool {
        ctx.document.remove_feature(id).is_ok()
    }
}

/// Everything a bench contributes to the viewport and chrome in one frame
/// ([`Workbench::viewport`]). Screen-space items are in viewport pixels,
/// relative to the viewport's origin, and keep their size at any zoom or
/// camera rotation; they draw bottom to top as pictures, polygons, lines,
/// marks, labels.
#[derive(Default)]
pub struct ViewportOutput {
    /// Meshes drawn in the scene, in world space, for guides and marks
    /// (datums, what two bodies share). They scale and turn with the view.
    pub meshes: Vec<OverlayMesh>,
    /// Lines of constant thickness.
    pub overlays: Vec<ScreenSpaceOverlay>,
    /// Filled polygons beneath the lines (the Move task's handles).
    pub polygons: Vec<ScreenSpacePolygon>,
    /// Pictures under the lines (the sketcher's reference images while a
    /// sketch is edited).
    pub images: Vec<ScreenSpaceImage>,
    /// Point markers and icon glyphs over the lines, beneath the labels.
    pub marks: Vec<ScreenSpaceMark>,
    /// Text of constant size (dimension values, constraint glyphs,
    /// on-view parameter readouts).
    pub labels: Vec<ScreenSpaceLabel>,
    /// Widgets drawn over the viewport.
    pub hud: Option<ViewportHud>,
    /// Scalar colours on document meshes, in imported geometry vertex
    /// order.
    pub scalar_maps: Vec<ScalarMap>,
    /// The bench's status-bar items.
    pub status: Option<StatusItems>,
    /// A plane the scene is cut at, in world space: the renderer's
    /// `[a, b, c, d]`, keeping what has `a·x + b·y + c·z + d >= 0` and
    /// hiding the rest from drawing and picking alike. While it is `Some`
    /// it stands in for the view toolbar's clipping plane.
    pub clip_plane: Option<[f32; 4]>,
    /// A grid on a plane of the bench's own, drawn by the renderer in
    /// place of the scene's grid: three decades of lines from
    /// `PlaneGrid::base`, each fading by its spacing on screen as
    /// `crate::grid` reckons.
    pub view_grid: Option<crate::PlaneGrid>,
    /// Bodies drawn faded: what the bench asks the user to see past (a
    /// joint tool, the bodies not being picked). They stay pickable.
    pub faded: Vec<BodyId>,
}

/// A body's measured values, in its imported geometry's vertex order.
#[derive(Debug, Clone, PartialEq)]
pub struct ScalarMap {
    pub body: BodyId,
    /// One per vertex; NaN marks an unmeasured point.
    pub values: std::sync::Arc<[f32]>,
    /// Bumped whenever values change, including after a mesh rebuild.
    pub revision: u64,
    /// Finite, increasing values at the ramp's ends.
    pub range: [f32; 2],
    /// Zero for continuous colour, otherwise the number of flat steps.
    pub bands: u32,
    /// Evenly spaced RGB stops from low to high.
    pub ramp: std::sync::Arc<[[f32; 3]]>,
    pub under: [f32; 3],
    pub over: [f32; 3],
    pub none: [f32; 3],
    pub legend: MapLegend,
}

impl ScalarMap {
    /// Where a value `t` of the way along the range (0 to 1) lands on the
    /// ramp, by `metrology::ramp::span`'s rule: as it is without bands,
    /// else at its band's middle, the range's top value in the last band.
    /// `None` outside 0 to 1, where a cap colour goes. The view's shader
    /// bands by the same rule.
    pub fn position(&self, t: f32) -> Option<f32> {
        metrology::ramp::span(f64::from(t), 0.0, 1.0, self.bands).map(|at| at as f32)
    }

    /// The ramp's colour at `at` (0 to 1), between its two nearest stops.
    pub fn ramp_colour(&self, at: f32) -> [f32; 3] {
        let at = at.clamp(0.0, 1.0) * self.ramp.len().saturating_sub(1) as f32;
        let low = at.floor() as usize;
        let a = self.ramp.get(low).copied().unwrap_or(self.none);
        let b = self.ramp.get(low + 1).copied().unwrap_or(a);
        std::array::from_fn(|c| a[c] + (b[c] - a[c]) * (at - low as f32))
    }

    /// The colour the map paints `value`: below and above the range in
    /// its caps, unmeasured in its grey, else where it lands on the ramp.
    pub fn colour(&self, value: f32) -> [f32; 3] {
        if value.is_nan() {
            return self.none;
        }
        let [lo, hi] = self.range;
        let t = (value - lo) / (hi - lo);
        if t < 0.0 {
            return self.under;
        }
        if t > 1.0 {
            return self.over;
        }
        self.position(t)
            .map_or(self.none, |at| self.ramp_colour(at))
    }
}

/// A bench's part of a printed report.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BenchReport {
    pub title: String,
    pub summary: Vec<String>,
    pub tables: Vec<ReportTable>,
    /// The bodies its pictures show.
    pub bodies: Vec<BodyId>,
}

/// A table of a report: its columns' headings and its rows, as text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReportTable {
    pub heading: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// A scalar map's readout, drawn beside its colour scale.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MapLegend {
    pub title: String,
    pub unit: String,
    pub stats: Vec<(String, String)>,
    /// Counts of evenly spaced bins over the map's range, low to high.
    pub histogram: Vec<u32>,
    pub tolerance: Option<(f32, f32)>,
}

/// A mesh a bench draws in the scene: never picked.
#[derive(Debug, Clone)]
pub struct OverlayMesh {
    pub mesh: std::sync::Arc<kernel_api::TriMesh>,
    /// The mesh's revision: equal for two overlays only while their meshes
    /// are the same, so the host and the GPU keep an unchanged mesh without
    /// reading it. A maker caching its mesh takes a [`fresh_revision`] each
    /// time it makes one; [`mesh_revision`] is for a mesh small enough to
    /// read every frame.
    pub revision: u64,
    /// RGB in 0..=1.
    pub color: [f32; 3],
    /// Drawn as its triangles' lines, pulled toward the camera so it shows
    /// on the surfaces it lies on.
    pub wireframe: bool,
    /// Under 1.0 it blends over what is behind it.
    pub opacity: f32,
    /// Over everything, whatever is in front of it.
    pub on_top: bool,
}

impl OverlayMesh {
    /// A wireframe guide of a few lines, its revision read from the mesh.
    pub fn wireframe(mesh: kernel_api::TriMesh, color: [f32; 3]) -> Self {
        Self {
            revision: mesh_revision(&mesh),
            mesh: std::sync::Arc::new(mesh),
            color,
            wireframe: true,
            opacity: 1.0,
            on_top: false,
        }
    }

    /// A see-through solid drawn over the whole scene.
    pub fn on_top(
        mesh: std::sync::Arc<kernel_api::TriMesh>,
        revision: u64,
        color: [f32; 3],
        opacity: f32,
    ) -> Self {
        Self {
            mesh,
            revision,
            color,
            wireframe: false,
            opacity,
            on_top: true,
        }
    }
}

/// A revision no other call in this run answers, for a mesh just made.
pub fn fresh_revision() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // Spread over the whole range (a bijection), so it stays apart from
    // the hashes other makers use as revisions.
    let mut z = n.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A revision read from what a mesh draws: its positions, colours,
/// triangles and lines. It reads every vertex, so it suits small meshes.
pub fn mesh_revision(mesh: &kernel_api::TriMesh) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for p in mesh.positions.iter().chain(&mesh.colors) {
        p.map(f32::to_bits).hash(&mut hasher);
    }
    mesh.colors.len().hash(&mut hasher);
    mesh.indices.hash(&mut hasher);
    mesh.edges.hash(&mut hasher);
    hasher.finish()
}

/// Registry used by workbenches to declare the tools they expose.
#[derive(Debug, Default)]
pub struct WorkbenchContext {
    tools: Vec<ToolDescriptor>,
    actions: Vec<crate::shortcut::ActionDescriptor>,
    commands: Vec<crate::CommandSpec>,
    imports: Vec<FileImport>,
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

    /// Offer to import files with `import.extensions` through File ›
    /// Import: the host runs `import.command`, one of this bench's
    /// commands, with the picked file as its `path`.
    pub fn register_import(&mut self, import: FileImport) {
        self.imports.push(import);
    }

    pub fn imports(&self) -> &[FileImport] {
        &self.imports
    }
}

/// A kind of file a bench imports: what the import dialog calls it, its
/// extensions, and the command that takes one as `path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileImport {
    pub label: String,
    /// Lower case, without the dot.
    pub extensions: Vec<String>,
    pub command: String,
}

impl FileImport {
    pub fn new<I, S>(label: impl Into<String>, extensions: I, command: impl Into<String>) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            label: label.into(),
            extensions: extensions
                .into_iter()
                .map(|e| e.into().to_ascii_lowercase())
                .collect(),
            command: command.into(),
        }
    }

    /// Whether `path` has one of this import's extensions.
    pub fn takes(&self, path: &std::path::Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| self.extensions.iter().any(|x| x.eq_ignore_ascii_case(e)))
    }
}

/// Describes how a tool button should behave in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolBehavior {
    /// Radio button behavior: only one tool in the same group can be active at a time.
    /// Clicking an active tool deactivates it. Tools in different groups are independent.
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
/// The category of a bench's checks: tools that look at the model and
/// change nothing (interference, continuity, curvature). Inspect, in the
/// menu bar, lists every bench's tools of it beside the host's own.
pub const INSPECT_CATEGORY: &str = "analysis";

#[derive(Debug, Clone)]
pub struct ToolDescriptor {
    pub id: String,
    pub label: String,
    /// Optional category (e.g., "drawing", "modeling", "utility"). The toolbar
    /// makes one group of each run of a category along a row, and menus
    /// separate where it changes.
    pub category: Option<String>,
    /// How the tool button should behave in the UI.
    pub behavior: ToolBehavior,
    /// Optional group name for Radio tools. Tools in the same group are mutually exclusive.
    /// Only one tool per group can be active at a time. If None, activating the tool
    /// deactivates every other active tool. Ignored for Check and Action tools.
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
    /// Create a new tool descriptor with radio button behavior and no group:
    /// activating it deactivates every other tool.
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
            group: None,
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
            group: None,
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
            group: None,
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
