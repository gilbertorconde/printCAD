//! The state the host keeps and the UI shows: what is selected, the view's
//! pick filter and menus, the tabs, the property panel's page, the camera
//! facts panel hooks see.

use core_document::{BodyId, ComponentId, FeatureId, NoteId};
use uuid::Uuid;

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

/// What a click in the view picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PickFilter {
    /// An edge near the cursor, else the face under it; a double click
    /// the whole body.
    #[default]
    Any,
    Faces,
    Edges,
    /// A click takes the whole body.
    Bodies,
}

impl PickFilter {
    pub const ALL: [PickFilter; 4] = [
        PickFilter::Any,
        PickFilter::Faces,
        PickFilter::Edges,
        PickFilter::Bodies,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PickFilter::Any => "Faces and edges",
            PickFilter::Faces => "Faces only",
            PickFilter::Edges => "Edges only",
            PickFilter::Bodies => "Whole bodies",
        }
    }

    /// Whether an edge near the cursor hovers and picks.
    pub fn edges(self) -> bool {
        matches!(self, PickFilter::Any | PickFilter::Edges)
    }

    /// Whether the face under the cursor hovers and picks.
    pub fn faces(self) -> bool {
        matches!(self, PickFilter::Any | PickFilter::Faces)
    }
}

/// What the measure tool's pill shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasureBar {
    /// The unit chosen for the readout; `None` follows the document's.
    pub unit: Option<core_document::Unit>,
    pub document_unit: core_document::Unit,
    /// How many picks the chain holds.
    pub picks: usize,
}

/// A right click on a body, and where it landed, in points.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewportMenu {
    pub body: core_document::BodyId,
    pub at: [f32; 2],
    /// The face it was opened on, when there was one: its index in the
    /// body's mesh.
    pub face: Option<u32>,
    /// The point it was opened on, where the body sits in the document.
    pub point: Option<[f32; 3]>,
    /// The last tool started, by name, which the menu offers to repeat.
    pub repeat: Option<String>,
    /// Every face under the cursor, front to back, for Select other.
    pub others: Vec<OtherPick>,
}

/// A menu a bench opened at the pointer: its entries, the tag its pick
/// carries back, and where it stands, in points.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchMenu {
    pub bench: core_document::WorkbenchId,
    pub tag: String,
    pub items: Vec<core_document::MenuItem>,
    pub at: [f32; 2],
}

/// A face under the cursor, which Select other offers: its body, the
/// triangle the cursor's ray crossed on it, and how the menu names it.
#[derive(Debug, Clone, PartialEq)]
pub struct OtherPick {
    pub body: core_document::BodyId,
    pub triangle: u32,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum PropertyTab {
    View,
    #[default]
    Data,
}

/// A point or a line the Physical group can take a body's inertia about,
/// placed in the world.
#[derive(Debug, Clone, PartialEq)]
pub enum InertiaReference {
    CentreOfMass,
    Point {
        name: String,
        at: [f64; 3],
    },
    Axis {
        name: String,
        point: [f64; 3],
        direction: [f64; 3],
    },
}

impl InertiaReference {
    pub fn name(&self) -> &str {
        match self {
            InertiaReference::CentreOfMass => "Centre of mass",
            InertiaReference::Point { name, .. } | InertiaReference::Axis { name, .. } => name,
        }
    }
}

/// The document server serving a tab, as the status bar shows it.
#[derive(Debug, Clone, Default)]
pub struct ServerBadge {
    /// What the server calls itself, for the tooltip.
    pub name: String,
    /// The app reads and writes files itself, with no server process.
    pub standalone: bool,
    pub connected: bool,
    /// Other clients editing the same document.
    pub peers: u32,
}

/// One tab as the host describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabInfo {
    pub tab: Uuid,
    pub name: String,
    pub dirty: bool,
    pub active: bool,
    /// Nothing has happened in the tab: it shows the start page and New
    /// or Open take it over instead of opening another.
    pub blank: bool,
    /// Its file cannot be written in place: a lock before its name.
    pub read_only: bool,
    /// It was opened from or saved to a file, which Revert reads again.
    pub has_file: bool,
}

/// The open menu, as the UI draws it.
#[derive(Debug, Clone)]
pub struct RadialView {
    /// Its middle, in the view's physical pixels.
    pub centre: [f32; 2],
    /// Each slice's icon and name.
    pub slices: Vec<(&'static str, String)>,
    pub pointed: Option<usize>,
}

/// Camera and viewport facts the app hands the UI each frame.
#[derive(Debug, Clone)]
pub struct HostCtxParams {
    pub camera_position: [f32; 3],
    pub camera_target: [f32; 3],
    /// `(x, y, width, height)` in physical pixels.
    pub viewport: (u32, u32, u32, u32),
    pub pixels_per_point: f32,
    pub view_proj: Option<[[f32; 4]; 4]>,
    pub selected_body_id: Option<uuid::Uuid>,
    pub selected_face: Option<core_document::FaceRef>,
    pub selected_faces: Vec<core_document::FaceRef>,
    pub selected_edges: Vec<core_document::EdgeRef>,
    pub selected_parts: Vec<core_document::FeaturePart>,
    /// Every feature of the tree's selection when there are several, the
    /// selected row first.
    pub selected_features: Vec<FeatureId>,
    /// The feature kinds benches declare bought.
    pub bought_kinds: Vec<core_document::WorkbenchId>,
    /// The tab's newest closed undo step.
    pub undo_entry: Option<core_document::JournalEntryId>,
}

impl Default for HostCtxParams {
    fn default() -> Self {
        Self {
            camera_position: [0.0, 0.0, 5.0],
            camera_target: [0.0; 3],
            viewport: (0, 0, 1, 1),
            pixels_per_point: 1.0,
            view_proj: None,
            selected_body_id: None,
            selected_face: None,
            selected_faces: Vec::new(),
            selected_edges: Vec::new(),
            selected_parts: Vec::new(),
            selected_features: Vec::new(),
            bought_kinds: Vec::new(),
            undo_entry: None,
        }
    }
}

/// What a menu entry changes in the window rather than the document.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuLocal {
    /// Select the item and show this page of the property panel.
    Properties(TreeItemId, PropertyTab),
    /// Select the item and put its name up for editing.
    Rename(TreeItemId),
    /// Put this text in the script console's input.
    Console(String),
    /// Open one of the application's tasks in the right panel.
    Task(OpenTask),
}

/// A task a menu opens in the right panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OpenTask {
    Placement(BodyId),
    /// The body's look, with the face the menu was opened on, if any.
    Appearance(BodyId, Option<u32>),
    /// A feature's place in its body's history.
    History(FeatureId),
    /// The patterns pressed into the body's faces, with the face the menu
    /// was opened on, if any.
    Texture(BodyId, Option<u32>),
    /// Every part laid flat on the bed, for export or the slicer.
    PrintLayout,
    /// A new note on the body, at a point where it sits in the document
    /// (the one the menu was opened on), or where the body offers one.
    AddNote(BodyId, Option<[f32; 3]>),
    /// A note's text and where its label stands.
    EditNote(NoteId),
    /// The array that owns or repeats the body, or a new one of it.
    Array(BodyId),
    /// The array that repeats the component, or a new one of it.
    ComponentArray(core_document::ComponentId),
    /// Marking a scan's surface by hand.
    Mark(BodyId),
}
