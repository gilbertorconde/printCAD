//! What a WebAssembly workbench and printCAD say to each other.
//!
//! The component interface (`wit/workbench.wit`) carries these types as
//! JSON text; this crate is their one definition, used by the host
//! (`wb_wasm`) and by the guest SDK. Ids (features, bodies) travel as
//! UUID strings, lengths in millimetres, angles in degrees unless a field
//! says otherwise, and every position is in world space.
//!
//! # Versions
//!
//! The crate's major and minor version are the contract's: 0.1.x speaks
//! `printcad:workbench@0.1`. A patch release only adds, so a package
//! written against an earlier 0.1.x keeps building and running:
//!
//! - Enums the host sends and a package matches on ([`Event`],
//!   [`PanelEvent`], [`MenuScope`], [`PanelSlot`], [`Button`]) are
//!   `#[non_exhaustive]`, as are `kernel_api`'s `FaceSurface` and
//!   `ProfileSegment`: a match on them needs a wildcard arm, so a new
//!   variant does not break it, and binds a variant's fields with `..`,
//!   so a new field does not either. (The `exhaustive` feature lifts this for
//!   printCAD itself, which makes every variant it sends; a package leaves
//!   it off.)
//! - Structs a package fills in derive `Default`; fill them with
//!   `..Default::default()` and a new field does not break the literal.
//! - Struct-like variants a package builds ([`Widget`], [`Request`],
//!   [`Cell`], [`DiagramShape`], [`MarkKind`], [`Plan`], and `kernel_api`'s
//!   `SolidOp` and the enums inside it) never gain a field within a minor
//!   version: a new option comes as a new variant, or as a new struct that
//!   derives `Default`.
//! - A field added to a struct carries `#[serde(default)]`, so what an
//!   earlier package sends still reads.
//!
//! A new minor version (0.2) is a new contract, which a host of 0.1 does
//! not load.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use kernel_api::{self, SolidOp, TriMesh};

/// The version of this contract, as a package's manifest names it in
/// `api`. A host loads packages of its own major version.
pub const API_VERSION: &str = "0.1";

/// The full version of the contract this build speaks, patch included:
/// what a manifest's `min_contract` is held against.
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Whether this build's contract is at least `min`, a version such as
/// `0.1.4` (a `printcad:workbench@` prefix is read past). Only the
/// numbers it names count, so `0.1` asks for any 0.1.x.
pub fn contract_satisfies(min: &str) -> bool {
    let numbers = |v: &str| -> Option<Vec<u64>> {
        v.trim()
            .trim_start_matches("printcad:workbench@")
            .split('.')
            .map(|part| part.trim().parse().ok())
            .collect()
    };
    match (numbers(min), numbers(CONTRACT_VERSION)) {
        (Some(min), Some(ours)) => {
            let ours = ours.iter().copied().chain(std::iter::repeat(0));
            for (want, have) in min.iter().zip(ours) {
                if have != *want {
                    return have > *want;
                }
            }
            true
        }
        _ => false,
    }
}

/// The component interface: the WIT world `workbench` of package
/// `printcad:workbench`, whose values cross as JSON of this crate's types.
pub const WIT: &str = include_str!("../wit/workbench.wit");

/// Whether a package built against `api` runs on this host.
pub fn api_compatible(api: &str) -> bool {
    let major = |v: &str| {
        let mut parts = v.split('.');
        let first = parts.next().unwrap_or("");
        // Before 1.0 a minor version breaks as a major one would.
        if first == "0" {
            format!("0.{}", parts.next().unwrap_or(""))
        } else {
            first.to_string()
        }
    };
    let api = api.trim().trim_start_matches("printcad:workbench@");
    major(api) == major(API_VERSION)
}

// ---------------------------------------------------------------- package

/// `bench.toml`: what a package is and what it may reach.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Reverse-domain and unique, such as `acme.cam`. Its commands and
    /// icons are named under it.
    pub id: String,
    pub name: String,
    /// The package's own version.
    pub version: String,
    /// The contract it was built against: `printcad:workbench@0.1`.
    pub api: String,
    /// The earliest release of the contract it needs, patch included
    /// (`0.1.4`), when it uses something a patch release added: a host
    /// of an earlier one refuses it rather than draw part of it. Left
    /// out, any host of `api` loads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_contract: Option<String>,
    #[serde(default)]
    pub description: String,
    /// The feature kinds it owns; each starts with `id`.
    #[serde(default)]
    pub feature_kinds: Vec<String>,
    #[serde(default)]
    pub capabilities: Capabilities,
    /// Memory an instance may grow to, in MiB (at most 4096).
    #[serde(default = "default_memory_mb")]
    pub memory_mb: u32,
}

fn default_memory_mb() -> u32 {
    1024
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            version: String::new(),
            api: format!("printcad:workbench@{API_VERSION}"),
            min_contract: None,
            description: String::new(),
            feature_kinds: Vec::new(),
            capabilities: Capabilities::default(),
            memory_mb: default_memory_mb(),
        }
    }
}

/// What a package asks to reach beyond itself. The user grants each.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Ask the user where to save a file it made, or a folder for several.
    #[serde(default)]
    pub save_dialog: bool,
    /// Ask the user for a file to read: the package is handed its bytes.
    #[serde(default)]
    pub open_dialog: bool,
    /// Run the native helpers it ships, from a job.
    #[serde(default)]
    pub helper: bool,
    /// Open network connections.
    #[serde(default)]
    pub network: bool,
}

impl Capabilities {
    /// The capabilities both `self` and `other` hold.
    pub fn and(&self, other: &Capabilities) -> Capabilities {
        Capabilities {
            save_dialog: self.save_dialog && other.save_dialog,
            open_dialog: self.open_dialog && other.open_dialog,
            helper: self.helper && other.helper,
            network: self.network && other.network,
        }
    }

    /// Named, for the Preferences page and the log.
    pub fn names(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.save_dialog {
            out.push("save_dialog");
        }
        if self.open_dialog {
            out.push("open_dialog");
        }
        if self.helper {
            out.push("helper");
        }
        if self.network {
            out.push("network");
        }
        out
    }
}

// ----------------------------------------------------------- registration

/// What a bench declares once, when it is loaded (`describe`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Registration {
    pub label: String,
    #[serde(default)]
    pub description: String,
    /// An icon of the package (`icons/<name>.svg`) or of the design set.
    #[serde(default)]
    pub icon: String,
    /// An edit-session bench: entering it remembers the bench to return to.
    #[serde(default)]
    pub modal: bool,
    #[serde(default)]
    pub tools: Vec<Tool>,
    /// Keyboard actions beside the tools; a key sends `Event::Action`.
    #[serde(default)]
    pub actions: Vec<Action>,
    /// Commands scripts and agents may call; each id starts with the
    /// package id.
    #[serde(default)]
    pub commands: Vec<Command>,
    /// Numeric fields of feature data the property panel shows in the
    /// document's length unit.
    #[serde(default)]
    pub length_keys: Vec<String>,
    /// Feature kinds of the package's own whose bodies are bought rather
    /// than made (catalog hardware): the parts list marks them bought
    /// until the user says otherwise, and exports and the slicer leave
    /// them out.
    #[serde(default)]
    pub bought_kinds: Vec<String>,
    /// The tools and commands, by id, the radial menu offers while the
    /// bench is active, clockwise from the top, up to eight. The user can
    /// choose others in Preferences; empty leaves the bench's most used
    /// tools.
    #[serde(default)]
    pub radial: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolBehavior {
    /// One of a group is active at a time; the default.
    #[default]
    Radio,
    /// On or off on its own.
    Check,
    /// Fires once.
    Action,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub behavior: ToolBehavior,
    #[serde(default)]
    pub group: Option<String>,
    /// Toolbar row: 1 or 2 are the bench's own.
    #[serde(default = "one")]
    pub row: u8,
    #[serde(default)]
    pub category: Option<String>,
    /// Default keys, such as `"G"` or `"Shift+G"`.
    #[serde(default)]
    pub shortcuts: Vec<String>,
}

fn one() -> u8 {
    1
}

impl Default for Tool {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            icon: String::new(),
            behavior: ToolBehavior::default(),
            group: None,
            row: one(),
            category: None,
            shortcuts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub shortcuts: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    Number,
    Integer,
    Bool,
    String,
    Id,
    List,
    #[default]
    Any,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Param {
    pub name: String,
    #[serde(default)]
    pub kind: ParamKind,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub doc: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Command {
    pub id: String,
    pub summary: String,
    #[serde(default)]
    pub params: Vec<Param>,
    #[serde(default)]
    pub returns: String,
    /// It changes nothing, so an agent may run it without asking.
    #[serde(default)]
    pub read_only: bool,
    /// What a caller must know beyond the arguments: what it refuses, does
    /// silently, or is easily mistaken for.
    #[serde(default)]
    pub notes: Vec<String>,
    /// Working uses, each a Lua script run from an empty document.
    #[serde(default)]
    pub examples: Vec<Example>,
    /// Other commands that do the related thing, by id.
    #[serde(default)]
    pub see_also: Vec<String>,
}

/// A working use of a command.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Example {
    /// What it shows, one line.
    pub title: String,
    /// A Lua script, run from an empty document, ending in `assert`s on
    /// what it made.
    pub script: String,
}

// --------------------------------------------------------------- document

/// A feature of the document as a bench sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub body: Option<String>,
    /// The data the feature builds from: what the user set with every
    /// formula's value in.
    pub data: Value,
    #[serde(default)]
    pub visible: bool,
    /// The package and version that made it, `acme.cam 0.3.0`.
    #[serde(default)]
    pub made_by: Option<String>,
    /// The features it reads.
    #[serde(default)]
    pub deps: Vec<String>,
    /// Its place in its body's history.
    #[serde(default)]
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Body {
    pub id: String,
    pub name: String,
    pub hidden: bool,
    /// Where the body sits: a row-major rigid 4x4 matrix.
    pub placement: [f64; 16],
    /// Its solid's bounds, when it has one.
    #[serde(default)]
    pub bounds: Option<([f32; 3], [f32; 3])>,
}

/// A number of a feature that a formula may set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    /// What its formula is kept under.
    pub key: String,
    /// What formulas call it; `None` when they cannot refer to it.
    #[serde(default)]
    pub name: Option<String>,
    pub label: String,
    #[serde(default)]
    pub dim: Dim,
    /// A JSON pointer into the feature's data.
    pub pointer: String,
    /// What the data holds for one millimetre or degree.
    #[serde(default = "unit_scale")]
    pub scale: f64,
    #[serde(default)]
    pub integer: bool,
}

fn unit_scale() -> f64 {
    1.0
}

impl Default for Parameter {
    fn default() -> Self {
        Self {
            key: String::new(),
            name: None,
            label: String::new(),
            dim: Dim::default(),
            pointer: String::new(),
            scale: unit_scale(),
            integer: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dim {
    #[default]
    Length,
    Angle,
    Number,
}

/// How a feature shows in the tree, the property panel and the hover card.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FeatureInfo {
    pub icon: String,
    pub kind_label: String,
    #[serde(default)]
    pub family_label: String,
    /// It adds to its body's solid.
    #[serde(default)]
    pub builds_solid: bool,
    /// How its work stands, shown as a badge beside its name in the tree.
    /// It is asked for again when the feature changes or the bench calls
    /// `host::redraw`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<FeatureStatus>,
}

/// How a feature's work stands: a setup run, an operation checked.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FeatureStatus {
    pub state: FeatureState,
    /// While `Running`, the job doing the work: the host shows how far
    /// it is, rather than `fraction`.
    #[serde(default)]
    pub job: Option<u64>,
    /// While `Running`, how far, 0 to 1.
    #[serde(default)]
    pub fraction: Option<f32>,
    /// More words, shown when the pointer rests on the badge.
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureState {
    #[default]
    NotRun,
    Running,
    Done,
    Warnings,
    Failed,
    OutOfDate,
}

// ---------------------------------------------------------------- rebuild

/// The bodies to rebuild, each with its features of this bench in history
/// order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RebuildRequest {
    pub bodies: Vec<BodyHistory>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BodyHistory {
    pub body: String,
    pub features: Vec<Node>,
}

/// One body's plan.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Rebuild {
    pub body: String,
    pub plan: Plan,
    /// Threads drawn on round faces of the solid the plan builds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub threads: Vec<CosmeticThread>,
}

/// A thread drawn on a round face, never modelled: the face keeps its
/// size (a bore its tap drill, a shank its major diameter) and a helix of
/// the thread's pitch is drawn just off it, inside a bore and outside a
/// shank, in every bench and never exported. A thread with a
/// `designation` is also named where the app lists threads: a callout at
/// the face's end, the parts list, and a STEP export's callouts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CosmeticThread {
    /// The feature it belongs to: drawn while that feature shows.
    pub feature: String,
    /// A point on the round face, in the body's own frame, as the plan's
    /// ops are.
    pub point: [f64; 3],
    /// The face's name ([`FaceRef::name`], `geom.face_loops`), found
    /// before `point` while the solid still has a face of that name.
    #[serde(default)]
    pub name: Option<u64>,
    /// mm a turn.
    pub pitch: f64,
    /// How far it runs from the face's end nearest `point`; 0 runs the
    /// whole face.
    #[serde(default)]
    pub length: f64,
    #[serde(default)]
    pub left_handed: bool,
    /// What the thread is called ("M6", "1/4-20 UNC"); `None` names it
    /// nowhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub designation: Option<String>,
}

/// What a [`Rebuild`] asks of the kernel.
///
/// A package builds these, so a variant keeps its fields for as long as
/// the contract's minor version holds: a new option comes as a new
/// variant, or as a new struct that derives `Default`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Plan {
    /// Build these ops; `op_features[i]` is the feature op `i` is for.
    Ops {
        ops: Vec<SolidOp>,
        op_features: Vec<String>,
    },
    /// Nothing to build: the body's solid goes.
    #[default]
    Empty,
    /// The plan could not be made; the message goes on `feature`.
    Error {
        #[serde(default)]
        feature: Option<String>,
        message: String,
    },
}

// ------------------------------------------------------------------ input

/// Something the user did while the bench is active, with what was under
/// the cursor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Input {
    pub event: Event,
    /// The active tool, with a `:variant` suffix when one was picked.
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub pointer: Pointer,
}

/// Where the cursor is and what it is over.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Pointer {
    /// Viewport pixels.
    #[serde(default)]
    pub position: Option<[f32; 2]>,
    /// The ray under the cursor: origin and unit direction.
    #[serde(default)]
    pub ray: Option<([f32; 3], [f32; 3])>,
    /// The point of a body under the cursor.
    #[serde(default)]
    pub world: Option<[f32; 3]>,
    #[serde(default)]
    pub body: Option<String>,
    /// The selected body and the face and edges picked on it.
    #[serde(default)]
    pub selected_body: Option<String>,
    #[serde(default)]
    pub face: Option<FaceRef>,
    /// Every face picked, in the order picked; `face` is the last.
    #[serde(default)]
    pub faces: Vec<FaceRef>,
    #[serde(default)]
    pub edges: Vec<EdgeRef>,
    /// The feature selected in the tree.
    #[serde(default)]
    pub active_feature: Option<String>,
    #[serde(default)]
    pub ctrl: bool,
    /// The face under the cursor, whatever is selected: what a click
    /// would pick there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hover_face: Option<FaceRef>,
    /// The edge under the cursor, when one is within reach.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hover_edge: Option<EdgeRef>,
    /// The vertex under the cursor, when one is within a few pixels: an
    /// end of the edge under it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertex: Option<VertexRef>,
}

/// A vertex of a body's solid, where it sits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VertexRef {
    pub body: String,
    pub point: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FaceRef {
    pub point: [f32; 3],
    pub normal: [f32; 3],
    /// The face's exact surface, when known.
    #[serde(default)]
    pub surface: Option<kernel_api::FaceSurface>,
    /// The kernel's name for the face, which a rebuild keeps: what
    /// `geom.face_loops` and a [`CosmeticThread`] find it by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeRef {
    pub body: String,
    pub point: [f32; 3],
    pub direction: [f32; 3],
    pub length: f32,
    /// A circle's or an arc's centre, axis and radius, when the edge is
    /// one.
    #[serde(default)]
    pub circle: Option<EdgeCircle>,
    /// The names of the two faces it runs between, which name the edge
    /// across rebuilds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faces: Option<[u64; 2]>,
}

/// A circular edge's centre and axis (world space) and radius, mm.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EdgeCircle {
    pub center: [f32; 3],
    pub normal: [f32; 3],
    pub radius: f32,
}

#[cfg_attr(not(feature = "exhaustive"), non_exhaustive)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    Left,
    Middle,
    Right,
    Other,
}

#[cfg_attr(not(feature = "exhaustive"), non_exhaustive)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Press {
        button: Button,
    },
    Release {
        button: Button,
    },
    Move,
    /// A key by name: `Escape`, `Enter`, `A`, `Digit1`.
    Key {
        key: String,
        down: bool,
    },
    /// The active tool changed to `Input::tool`.
    ToolActivated,
    /// A registered action's key.
    Action {
        id: String,
    },
    Activated,
    Deactivated,
    /// Formulas moved the values of these features.
    ValuesMoved {
        features: Vec<String>,
    },
    /// A job this bench started finished.
    JobFinished {
        job: u64,
        result: Result<String, String>,
    },
    /// The user asked to edit a feature of the bench's: a double click on
    /// its tree row. Selecting a feature alone never opens it; its menu
    /// may offer to (`menu_items`).
    EditFeature {
        feature: String,
    },
    /// A feature of the bench's dragged in the tree and dropped just
    /// before (`before`) or after `target`. Used, the bench decides: it
    /// moves it itself (`calls::MOVE_FEATURE`, or its own data) or leaves
    /// it where it was; passed over, the host moves it in its body's
    /// history. Either way it is one undo step.
    FeatureMoved {
        feature: String,
        target: String,
        before: bool,
    },
    /// The files the user picked for `Request::OpenFile` of this `id`;
    /// none when the dialog was cancelled.
    FileOpened {
        id: String,
        files: Vec<OpenedFile>,
    },
}

/// A file the user picked to open: its name, without the folder, and its
/// bytes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OpenedFile {
    pub name: String,
    pub contents: Vec<u8>,
}

/// A kind of file a dialog offers: its label and extensions, without the
/// dot.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileKind {
    pub label: String,
    pub extensions: Vec<String>,
}

/// A file to write into the folder `Request::SaveFiles` asks for: a plain
/// file name, no folder in it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileToWrite {
    pub name: String,
    pub contents: Vec<u8>,
}

// ------------------------------------------------------------------ frame

/// Everything a bench shows. The host keeps it and asks again only after
/// an event reached the bench, the document changed, the selection
/// changed, or the bench asked (`redraw`); world-space drawing is projected
/// by the host every frame, so the view moves freely without asking.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    #[serde(default)]
    pub hud: Hud,
    #[serde(default)]
    pub status: Status,
    /// An open task: the right panel shows `panel` under it.
    #[serde(default)]
    pub task: Option<Task>,
    #[serde(default)]
    pub panel: Vec<Widget>,
    /// The feature whose edit session is open.
    #[serde(default)]
    pub editing: Option<String>,
    /// While `editing`, the view stays square to its plane.
    #[serde(default)]
    pub locks_view: bool,
    /// Bare digits go to the bench (a length being typed).
    #[serde(default)]
    pub numeric_input: bool,
    /// Check and action tools that draw pressed.
    #[serde(default)]
    pub toggled: Vec<String>,
    /// Tools that draw disabled.
    #[serde(default)]
    pub disabled: Vec<String>,
    #[serde(default)]
    pub lines: Vec<Polyline>,
    #[serde(default)]
    pub marks: Vec<Mark>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub meshes: Vec<Mesh>,
    /// Meshes the package keeps (`host::mesh_create`), drawn by handle:
    /// a large mesh crosses once and is then updated in part.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kept: Vec<KeptMesh>,
    /// Flat regions the host fills, and hatches where asked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<Region>,
    /// A strip under the view, its widgets in a row, while the bench is
    /// active: a transport bar's timeline, buttons and clock. Its events
    /// come with `PanelSlot::Transport`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transport: Vec<Widget>,
    /// Something in it moves: while set and the bench is active, the
    /// host asks for the frame again every frame, at the display's rate,
    /// rather than waiting for an event.
    #[serde(default)]
    pub animating: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Hud {
    /// The tool hint at the top left.
    #[serde(default)]
    pub tool: Option<ToolHint>,
    /// The top-right badge.
    #[serde(default)]
    pub badge: Option<(Rgb, String)>,
    #[serde(default)]
    pub legend: Vec<(Rgb, String)>,
    /// Mono readouts at the bottom right.
    #[serde(default)]
    pub footer: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolHint {
    pub icon: String,
    pub name: String,
    pub prompt: String,
    /// `(key, meaning)` chips.
    #[serde(default)]
    pub keys: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    #[serde(default)]
    pub state: Option<(Rgb, String)>,
    #[serde(default)]
    pub selection: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub title: String,
    #[serde(default)]
    pub icon: String,
    /// OK and Cancel; otherwise a single Close.
    #[serde(default)]
    pub confirmable: bool,
}

/// Linear RGB in 0..=1.
pub type Rgb = [f32; 3];

/// A line through world points, drawn a constant `width` pixels wide over
/// the scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Polyline {
    pub points: Vec<[f32; 3]>,
    pub color: Rgb,
    #[serde(default = "one_px")]
    pub width: f32,
    #[serde(default)]
    pub dashed: bool,
    #[serde(default)]
    pub closed: bool,
}

fn one_px() -> f32 {
    1.5
}

impl Default for Polyline {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            color: [1.0; 3],
            width: one_px(),
            dashed: false,
            closed: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    pub at: [f32; 3],
    pub color: Rgb,
    pub kind: MarkKind,
}

/// What a [`Mark`] draws.
///
/// A package builds these, so a variant keeps its fields for as long as
/// the contract's minor version holds: a new option comes as a new
/// variant, or as a new struct that derives `Default`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MarkKind {
    Dot { radius: f32 },
    Cross { size: f32 },
    Icon { name: String, size: f32 },
}

impl Default for MarkKind {
    fn default() -> Self {
        MarkKind::Dot { radius: 4.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub at: [f32; 3],
    pub text: String,
    pub color: Rgb,
    #[serde(default = "label_size")]
    pub size: f32,
    /// A rounded pill behind it.
    #[serde(default)]
    pub pill: bool,
    #[serde(default)]
    pub mono: bool,
}

fn label_size() -> f32 {
    12.0
}

impl Default for Label {
    fn default() -> Self {
        Self {
            at: [0.0; 3],
            text: String::new(),
            color: [1.0; 3],
            size: label_size(),
            pill: false,
            mono: false,
        }
    }
}

/// A mesh drawn in the scene, never picked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub color: Rgb,
    #[serde(default)]
    pub wireframe: bool,
    #[serde(default = "opaque")]
    pub opacity: f32,
    /// Over everything, whatever is in front of it.
    #[serde(default)]
    pub on_top: bool,
    /// A colour per position, multiplying `color` (leave that white to
    /// show them as they are). Ignored unless it holds one per position.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<Rgb>,
}

fn opaque() -> f32 {
    1.0
}

impl Default for Mesh {
    fn default() -> Self {
        Self {
            positions: Vec::new(),
            indices: Vec::new(),
            color: [1.0; 3],
            wireframe: false,
            opacity: opaque(),
            on_top: false,
            colors: Vec::new(),
        }
    }
}

/// A kept mesh as a frame draws it: its handle and how.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeptMesh {
    pub handle: u64,
    /// Multiplies the mesh's own colours, where it has them.
    pub color: Rgb,
    #[serde(default)]
    pub wireframe: bool,
    #[serde(default = "opaque")]
    pub opacity: f32,
    #[serde(default)]
    pub on_top: bool,
}

impl Default for KeptMesh {
    fn default() -> Self {
        Self {
            handle: 0,
            color: [1.0; 3],
            wireframe: false,
            opacity: opaque(),
            on_top: false,
        }
    }
}

/// A flat region in world space: an outline and the holes in it, all on
/// one plane, filled and, with `hatch`, hatched in lines that far apart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub outline: Vec<[f32; 3]>,
    #[serde(default)]
    pub holes: Vec<Vec<[f32; 3]>>,
    pub color: Rgb,
    /// The fill's; 0 leaves only the hatch.
    #[serde(default = "region_opacity")]
    pub opacity: f32,
    /// mm between hatch lines; none for no hatch.
    #[serde(default)]
    pub hatch: Option<f32>,
    /// The hatch lines' angle in the region's plane, degrees.
    #[serde(default = "hatch_angle")]
    pub hatch_angle: f32,
    /// Over everything, whatever is in front of it.
    #[serde(default)]
    pub on_top: bool,
}

fn region_opacity() -> f32 {
    0.35
}

fn hatch_angle() -> f32 {
    45.0
}

impl Default for Region {
    fn default() -> Self {
        Self {
            outline: Vec::new(),
            holes: Vec::new(),
            color: [1.0; 3],
            opacity: region_opacity(),
            hatch: None,
            hatch_angle: hatch_angle(),
            on_top: false,
        }
    }
}

// ------------------------------------------------------------------ panel

/// One element of a declared panel. The host draws it with the app's
/// widgets and sends back a [`PanelEvent`] when the user changes it.
///
/// A package builds these, so a variant keeps its fields for as long as
/// the contract's minor version holds: a new option comes as a new
/// variant, or as a new struct that derives `Default`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Widget {
    Heading {
        text: String,
    },
    Text {
        text: String,
        #[serde(default)]
        mono: bool,
    },
    Note {
        #[serde(default)]
        kind: NoteKind,
        #[serde(default)]
        title: Option<String>,
        text: String,
    },
    /// A number. Bound to a parameter of a feature (`bind`), it takes
    /// formulas: the host keeps the formula, and the bench is told the
    /// value.
    Number {
        id: String,
        label: String,
        value: f64,
        #[serde(default)]
        dim: Dim,
        #[serde(default)]
        bind: Option<Bind>,
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
        #[serde(default = "two")]
        decimals: usize,
        #[serde(default)]
        error: Option<String>,
    },
    Choice {
        id: String,
        label: String,
        options: Vec<String>,
        selected: usize,
    },
    Toggle {
        id: String,
        label: String,
        on: bool,
    },
    TextField {
        id: String,
        label: String,
        value: String,
    },
    Button {
        id: String,
        label: String,
        #[serde(default)]
        style: ButtonStyle,
        #[serde(default = "yes")]
        enabled: bool,
    },
    /// A row asking for something to be clicked in the viewport. A click
    /// on it arms it (`PanelEvent::Pick`); the bench then reads the picks
    /// from the events that follow.
    Pick {
        id: String,
        label: String,
        /// What has been picked, in words.
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        armed: bool,
    },
    /// Selectable rows.
    List {
        id: String,
        items: Vec<ListItem>,
        #[serde(default)]
        selected: Option<usize>,
    },
    /// Columns of text, one row selectable. A column marked in
    /// `editable` takes typing in its cells (`PanelEvent::Cell`): a tool
    /// table, a list of settings by row.
    Table {
        id: String,
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
        #[serde(default)]
        selected: Option<usize>,
        /// One per column; a column left out is read-only.
        #[serde(default)]
        editable: Vec<bool>,
    },
    Group {
        title: String,
        #[serde(default = "yes")]
        open: bool,
        children: Vec<Widget>,
    },
    Progress {
        label: String,
        /// `None` while the amount is unknown.
        #[serde(default)]
        fraction: Option<f32>,
        /// A job of the bench's: the host shows how far it is.
        #[serde(default)]
        job: Option<u64>,
    },
    Separator,
    /// A drawing: a schematic of what the panel edits, with its measures
    /// marked beside the parts they size. It is drawn in a space of its
    /// own, `[0, width] × [0, height]` with y up, which the host fits to
    /// the panel's width; strokes and text keep their pixel size. A press
    /// on it, and every move of a drag begun on it, comes back as
    /// `PanelEvent::DiagramPress` in those coordinates: a timeline seeks
    /// where it is pressed.
    Diagram {
        id: String,
        width: f32,
        height: f32,
        #[serde(default)]
        shapes: Vec<DiagramShape>,
        #[serde(default)]
        dimensions: Vec<Dimension>,
        #[serde(default)]
        callouts: Vec<Callout>,
    },
    /// A task panel's title with an icon, on a raised band.
    Header {
        icon: String,
        title: String,
    },
    /// A label and a value beside it, read only.
    Value {
        label: String,
        value: String,
        #[serde(default)]
        mono: bool,
    },
    /// Widgets side by side, going on to the next line when the panel is
    /// narrow. A field with an empty label takes no label column here.
    Row {
        children: Vec<Widget>,
    },
    /// A widget with words shown while the pointer rests on it.
    Hinted {
        hint: String,
        widget: Box<Widget>,
    },
    /// A widget with words and a figure under them (a [`Widget::Diagram`])
    /// shown while the pointer rests on it.
    HintedFigure {
        hint: String,
        figure: Box<Widget>,
        widget: Box<Widget>,
    },
    /// A number dragged along a range, for a spread or a place in time
    /// rather than a measure; it comes back as `PanelEvent::Number`.
    Slider {
        id: String,
        label: String,
        value: f64,
        min: f64,
        max: f64,
        /// The values it stops at, every `step` from `min`.
        #[serde(default)]
        step: Option<f64>,
        #[serde(default = "two")]
        decimals: usize,
        /// The value written beside it.
        #[serde(default = "yes")]
        show_value: bool,
    },
    /// Rows of cells under column titles, some cells editable; it scrolls
    /// sideways when wider than the panel.
    Sheet {
        id: String,
        columns: Vec<String>,
        rows: Vec<Vec<Cell>>,
    },
    /// A folding section with a count and a summary while closed.
    Section(PanelSection),
    /// One item: an icon, a name, a value at the end, tags and badges.
    Item(Item),
    /// Items, selectable, narrowed by a [`Widget::Filter`].
    Items(ItemList),
    /// A filter field for item lists, with a kind selector.
    Filter(FilterField),
    /// A state in a few words, as a toned pill.
    Status(StatusLine),
    /// Widgets one under another.
    Stack(Stack),
    /// Widgets side by side.
    Flow(Flow),
    /// A widget taking the rest of a [`Widget::Flow`]'s width.
    Fill(Fill),
    /// Fields sharing one label column of their own.
    Fields(Fields),
    /// A field with its label beside it at its own width, no column.
    Inline(Inline),
    /// What is picked in the view, as a chip that clears.
    PickChip(PickChip),
    /// A line of text the user types, with a placeholder while empty.
    Entry(Entry),
    /// What the task panel's header shows under and beside the title.
    TaskHeader(TaskHeader),
    /// A number field with its drag speed, width and state, and as a
    /// stepper.
    Quantity(Quantity),
    /// A labelled check box.
    Check(Check),
    /// A choice of a few, with its width and state.
    Select(Select),
    /// Fields in a two-column grid, labels as wide as the longest.
    Grid(FieldGrid),
    /// A note the user can click.
    Alert(Alert),
    /// A raised card holding widgets, under an icon and a title.
    Card(Card),
    /// A line of words in a size and a tone.
    Caption(Caption),
    /// Room between widgets.
    Gap(Gap),
    /// Lines plotted along a strip.
    Plot(Plot),
    /// Colours to click, the chosen ones ringed.
    Swatches(Swatches),
    /// A colour, edited in the system's picker.
    Color(ColorField),
    /// A number dragged along a range, written with a suffix of its own.
    Range(Range),
    /// A frameless button as wide as the panel: one of a list of places.
    RowButton(RowButton),
    /// Words that light up while chosen, as one of a few side by side.
    Selectable(Selectable),
    /// Pictures to choose among, each over its name.
    Gallery(Gallery),
    /// A titled, bordered group of rows, as the Preferences pages draw
    /// theirs.
    Settings(SettingsGroup),
    /// Widgets in rows of a few columns, each column as wide as its
    /// widest: a table of readings.
    Cells(Cells),
    /// A small icon button on its own, its words on hover.
    Action(PanelAction),
}

/// The widgets a widget holds, in the order they are drawn: one match for
/// [`Widget::children`] and [`Widget::children_mut`].
macro_rules! widget_children {
    ($widget:expr, $deref:ident, $iter:ident $(, $mut:ident)?) => {{
        let mut out = Vec::new();
        match $widget {
            Widget::Group { children, .. }
            | Widget::Row { children }
            | Widget::Section(PanelSection { children, .. })
            | Widget::Stack(Stack { children, .. })
            | Widget::Flow(Flow { children, .. })
            | Widget::Fields(Fields { children, .. })
            | Widget::Grid(FieldGrid { children, .. })
            | Widget::Card(Card { children, .. })
            | Widget::Cells(Cells { children, .. })
            | Widget::Settings(SettingsGroup { children, .. }) => out.extend(children),
            Widget::Hinted { widget, .. } => out.push(&$($mut)? **widget),
            Widget::HintedFigure { figure, widget, .. } => {
                out.push(&$($mut)? **figure);
                out.push(&$($mut)? **widget);
            }
            Widget::Fill(Fill { child }) | Widget::Inline(Inline { child }) => {
                out.push(&$($mut)? **child)
            }
            Widget::Item(item) => out.extend(item.field.$deref()),
            Widget::Items(list) => {
                for item in list.items.$iter() {
                    out.extend(item.field.$deref());
                }
            }
            Widget::Heading { .. }
            | Widget::Text { .. }
            | Widget::Note { .. }
            | Widget::Number { .. }
            | Widget::Choice { .. }
            | Widget::Toggle { .. }
            | Widget::TextField { .. }
            | Widget::Button { .. }
            | Widget::Pick { .. }
            | Widget::List { .. }
            | Widget::Table { .. }
            | Widget::Progress { .. }
            | Widget::Separator
            | Widget::Diagram { .. }
            | Widget::Header { .. }
            | Widget::Value { .. }
            | Widget::Slider { .. }
            | Widget::Sheet { .. }
            | Widget::Filter(_)
            | Widget::Status(_)
            | Widget::PickChip(_)
            | Widget::Entry(_)
            | Widget::TaskHeader(_)
            | Widget::Quantity(_)
            | Widget::Check(_)
            | Widget::Select(_)
            | Widget::Alert(_)
            | Widget::Caption(_)
            | Widget::Gap(_)
            | Widget::Plot(_)
            | Widget::Swatches(_)
            | Widget::Color(_)
            | Widget::Range(_)
            | Widget::RowButton(_)
            | Widget::Selectable(_)
            | Widget::Gallery(_)
            | Widget::Action(_) => {}
        }
        out
    }};
}

impl Widget {
    /// The widgets this one holds, in the order they are drawn: a
    /// container's children, a hinted widget (and its figure), an item's
    /// field. Empty for a widget that holds none.
    pub fn children(&self) -> Vec<&Widget> {
        widget_children!(self, as_deref, iter)
    }

    /// [`Widget::children`], to change in place.
    pub fn children_mut(&mut self) -> Vec<&mut Widget> {
        widget_children!(self, as_deref_mut, iter_mut, mut)
    }

    /// This widget and every widget it holds, at any depth, each before
    /// its children.
    pub fn walk<'a>(&'a self, visit: &mut impl FnMut(&'a Widget)) {
        visit(self);
        for child in self.children() {
            child.walk(visit);
        }
    }

    /// [`Widget::walk`], to change in place.
    pub fn walk_mut(&mut self, visit: &mut impl FnMut(&mut Widget)) {
        visit(self);
        for child in self.children_mut() {
            child.walk_mut(visit);
        }
    }

    /// The id its events carry, for a widget that has one.
    pub fn id(&self) -> Option<&str> {
        let id = match self {
            Widget::Number { id, .. }
            | Widget::Choice { id, .. }
            | Widget::Toggle { id, .. }
            | Widget::TextField { id, .. }
            | Widget::Button { id, .. }
            | Widget::Pick { id, .. }
            | Widget::List { id, .. }
            | Widget::Table { id, .. }
            | Widget::Diagram { id, .. }
            | Widget::Slider { id, .. }
            | Widget::Sheet { id, .. }
            | Widget::Section(PanelSection { id, .. })
            | Widget::Item(Item { id, .. })
            | Widget::Items(ItemList { id, .. })
            | Widget::Filter(FilterField { id, .. })
            | Widget::PickChip(PickChip { id, .. })
            | Widget::Entry(Entry { id, .. })
            | Widget::Quantity(Quantity { id, .. })
            | Widget::Check(Check { id, .. })
            | Widget::Select(Select { id, .. })
            | Widget::Swatches(Swatches { id, .. })
            | Widget::Color(ColorField { id, .. })
            | Widget::Range(Range { id, .. })
            | Widget::RowButton(RowButton { id, .. })
            | Widget::Selectable(Selectable { id, .. })
            | Widget::Gallery(Gallery { id, .. })
            | Widget::Action(PanelAction { id, .. }) => id,
            Widget::Alert(Alert { id, .. }) => id.as_ref()?,
            _ => return None,
        };
        (!id.is_empty()).then_some(id.as_str())
    }

    /// The label a field writes in its label column, for a field that has
    /// one: a check box's or a button's own words are not one.
    pub fn field_label(&self) -> Option<&str> {
        let label = match self {
            Widget::Number { label, .. }
            | Widget::Choice { label, .. }
            | Widget::TextField { label, .. }
            | Widget::Pick { label, .. }
            | Widget::Value { label, .. }
            | Widget::Slider { label, .. }
            | Widget::PickChip(PickChip { label, .. })
            | Widget::Entry(Entry { label, .. })
            | Widget::Quantity(Quantity { label, .. })
            | Widget::Check(Check { label, .. })
            | Widget::Select(Select { label, .. })
            | Widget::Range(Range { label, .. })
            | Widget::Color(ColorField { label, .. })
            | Widget::Flow(Flow { label, .. }) => label,
            _ => return None,
        };
        (!label.is_empty()).then_some(label.as_str())
    }
}

/// `widgets` keeping only those `keep` holds to, at any depth: a widget
/// left out goes with what it holds. A widget held alone (a hint's, a
/// fill's) stays whatever `keep` says of it.
pub fn retain_widgets(widgets: &mut Vec<Widget>, keep: &mut impl FnMut(&Widget) -> bool) {
    widgets.retain(|w| keep(w));
    for widget in widgets {
        retain_within(widget, keep);
    }
}

fn retain_within(widget: &mut Widget, keep: &mut impl FnMut(&Widget) -> bool) {
    match widget {
        Widget::Group { children, .. }
        | Widget::Row { children }
        | Widget::Section(PanelSection { children, .. })
        | Widget::Stack(Stack { children, .. })
        | Widget::Flow(Flow { children, .. })
        | Widget::Fields(Fields { children, .. })
        | Widget::Grid(FieldGrid { children, .. })
        | Widget::Card(Card { children, .. })
        | Widget::Cells(Cells { children, .. })
        | Widget::Settings(SettingsGroup { children, .. }) => retain_widgets(children, keep),
        other => {
            for child in other.children_mut() {
                retain_within(child, keep);
            }
        }
    }
}

/// Every widget of `widgets`, at any depth, each before its children.
pub fn walk_widgets(widgets: &[Widget]) -> Vec<&Widget> {
    let mut all = Vec::new();
    for widget in widgets {
        widget.walk(&mut |w| all.push(w));
    }
    all
}

// ----------------------------------------------------------- panel blocks
//
// The blocks say what a part of a panel is; the host draws each with the
// controls its own panels use. They came in contract 0.1.4: a package
// using them names `min_contract = "0.1.4"` in its manifest.

/// How a mark, a tag or a status reads: what it means, not its colour.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    #[default]
    Neutral,
    Good,
    Warning,
    Bad,
    Info,
    /// The accent: what is in hand.
    Accent,
    /// Dim: what is out of play or says little.
    Muted,
}

/// A small icon button: in a section's title, on its own
/// ([`Widget::Action`]: a cross at a row's end that takes it out), or an
/// entry of an item's menu (`label` then names it). A click comes back as
/// `PanelEvent::Button` (a section's or one on its own) or
/// `PanelEvent::ContextMenu` (a menu's) with `id`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PanelAction {
    pub id: String,
    #[serde(default)]
    pub icon: Option<String>,
    /// The words on hover, or the menu entry.
    #[serde(default)]
    pub label: String,
    /// A menu entry's words on hover.
    #[serde(default)]
    pub hint: String,
    /// A menu entry with a line above it, setting it apart.
    #[serde(default)]
    pub separated: bool,
}

/// A [`Widget::Section`]: a fold the user opens and closes, its title in
/// small capitals, the count of what it holds, a one-line summary while
/// closed, icon actions at its right end, and its children, whose fields
/// share one label column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelSection {
    /// Keeps the fold's state; the title when empty.
    pub id: String,
    pub title: String,
    pub count: Option<usize>,
    pub summary: Option<String>,
    /// Open the first time it is shown.
    pub open: bool,
    pub actions: Vec<PanelAction>,
    pub children: Vec<Widget>,
}

impl Default for PanelSection {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            count: None,
            summary: None,
            open: true,
            actions: Vec::new(),
            children: Vec::new(),
        }
    }
}

/// A small rounded name in an item's row: an element a constraint ties.
/// A click comes back as `PanelEvent::Tag`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tag {
    pub text: String,
    /// `Neutral` draws it plain.
    pub tone: Tone,
    /// What it names is selected.
    pub selected: bool,
    /// What it names is in hand or tied to it: outlined.
    pub lit: bool,
}

/// A glyph badge at an item's end: a constraint on an element. A click
/// comes back as `PanelEvent::Badge`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Badge {
    pub icon: String,
    pub tone: Tone,
    /// Words on hover.
    pub hint: String,
    /// What it stands for is in hand: drawn on a lit ground.
    pub lit: bool,
    /// Its colour as sRGB, in place of the tone's.
    pub color: Option<[u8; 3]>,
}

/// One row of a list: what an [`ItemList`] holds, or alone as a
/// [`Widget::Item`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    /// Names the row in events when it stands alone; in a list the row's
    /// place names it.
    pub id: String,
    pub icon: Option<String>,
    /// The icon's tone; `Neutral` is the row's plain one.
    pub tone: Tone,
    /// Out of play: its name dim.
    pub muted: bool,
    /// A problem: its name in the danger colour.
    pub problem: bool,
    pub label: String,
    /// Dim words after the name.
    pub secondary: Option<String>,
    /// At the row's end.
    pub value: Option<String>,
    /// The value in the plain face rather than the mono one.
    pub value_plain: bool,
    pub tags: Vec<Tag>,
    pub badges: Vec<Badge>,
    pub selected: bool,
    /// What a [`FilterField`]'s kind selector matches.
    pub kind: Option<String>,
    /// Words on hover.
    pub hint: Option<String>,
    /// The entries of its right-click menu.
    pub menu: Vec<PanelAction>,
    /// Tied to what is in hand (hovered or selected elsewhere): a faint
    /// tint and a thin bar at its left edge.
    pub related: bool,
    /// What is in hand itself, hovered through something else (a tag,
    /// the view): lit as under the pointer.
    pub lit: bool,
    /// The icon's colour as sRGB, in place of the tone's: the colour the
    /// view draws it in.
    pub color: Option<[u8; 3]>,
    /// The value's tone; `Neutral` is the row's plain one.
    pub value_tone: Tone,
    /// Words on hover over the value.
    pub value_hint: Option<String>,
    /// A word after the value in its tone, its `detail` on hover.
    pub flag: Option<StatusLine>,
    /// Scrolled into view.
    pub reveal: bool,
    /// Its name as a text field, focused: told as `PanelEvent::ItemText`
    /// at each change and once more as it is left.
    pub editing: bool,
    /// A control at the row's end in place of the value (a
    /// [`Widget::Quantity`] the row's value is typed into); its events are
    /// its own.
    pub field: Option<Box<Widget>>,
}

/// A [`Widget::Items`]: rows a click selects (`PanelEvent::Select`), or
/// with `multi` a click with Ctrl adds to (`PanelEvent::Selection`), a
/// double click opens (`PanelEvent::Activate`); the pointer over one is
/// told as `PanelEvent::Hover`, so the bench lights it in the view.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemList {
    pub id: String,
    pub items: Vec<Item>,
    pub multi: bool,
    /// The [`FilterField`] that narrows it, by its id.
    pub filter: Option<String>,
    /// Shown when nothing is listed, or nothing passes the filter.
    pub empty: Option<String>,
}

/// A [`Widget::Filter`]: a search field narrowing the item lists that
/// name it, by their label, secondary words and tags, and a kind selector
/// when `kinds` names any (the first, "All" say, passes every item; the
/// others pass items of that `kind`). The host filters as the user types;
/// with `by_bench` it leaves the lists whole and tells the bench
/// (`PanelEvent::Filter`) instead.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterField {
    pub id: String,
    /// In the field while it is empty.
    pub hint: String,
    pub kinds: Vec<String>,
    /// The kind chosen when first shown.
    pub kind: usize,
    pub by_bench: bool,
}

/// A [`Widget::Status`], and a task header's state ([`TaskHeader`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusLine {
    pub text: String,
    pub tone: Tone,
    /// The longer reason, on hover.
    pub detail: String,
}

/// A [`Widget::TaskHeader`]: in a task's panel, the header above it
/// shows `subtitle` under the title and `status` as a pill beside it,
/// as the app's own tasks do; anywhere else they draw in the panel.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskHeader {
    /// What the task edits or where it sits.
    pub subtitle: String,
    pub status: Option<StatusLine>,
}

/// A [`Widget::Stack`]: its children the panel's spacing apart, or
/// close together when `tight`, as a task's lines of text and fields
/// sit.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stack {
    pub children: Vec<Widget>,
    pub tight: bool,
    /// The room between its children in points, in place of the panel's.
    pub spacing: Option<f32>,
}

/// A [`Widget::Flow`]: side by side, going on to the next line when the
/// panel is narrow unless `no_wrap`. A [`Widget::Fill`] among them takes
/// what the others leave.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Flow {
    pub children: Vec<Widget>,
    pub no_wrap: bool,
    /// Written in its section's label column before the children, as a
    /// field's label is.
    pub label: String,
}

/// A [`Widget::Fill`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fill {
    pub child: Box<Widget>,
}

impl Default for Fill {
    fn default() -> Self {
        Self {
            child: Box::new(Widget::Separator),
        }
    }
}

/// A [`Widget::Fields`]: its fields' labels in one column as wide as the
/// longest, within the theme's limits, apart from the section's; or
/// `label_width` wide.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fields {
    pub children: Vec<Widget>,
    /// The column's width in points, whatever the labels.
    pub label_width: Option<f32>,
}

/// A [`Widget::Inline`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inline {
    pub child: Box<Widget>,
}

impl Default for Inline {
    fn default() -> Self {
        Self {
            child: Box::new(Widget::Separator),
        }
    }
}

/// A [`Widget::PickChip`]: a click arms it (`PanelEvent::Pick`), its
/// cross clears it (`PanelEvent::Clear`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PickChip {
    pub id: String,
    pub label: String,
    /// What has been picked, in words.
    pub value: Option<String>,
    pub armed: bool,
    /// In the chip while nothing is picked.
    pub placeholder: String,
}

/// A [`Widget::Entry`]: told once typing ends (`PanelEvent::Text`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub id: String,
    pub label: String,
    pub value: String,
    pub placeholder: String,
    /// Lines may break: Enter starts a new one.
    pub multiline: bool,
    /// Its width in points; the panel's field width when left out.
    pub width: Option<f32>,
    /// In the mono face.
    pub mono: bool,
    /// Told at each change rather than once typing ends.
    pub live: bool,
    /// Offers the document's names (variables, features and their
    /// properties) as a formula is typed, as the formula fields do.
    pub formulas: bool,
    /// In the panel's larger text size: the one text a task edits.
    pub large: bool,
}

/// A [`Widget::Quantity`]: a number as a [`Widget::Number`] says it, with
/// how it drags and how wide it is. Bound to a parameter (`bind`), it
/// takes formulas as a bound `Number` does. Changes come back as
/// `PanelEvent::Number`; a stepper's buttons as the value one up or down.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Quantity {
    pub id: String,
    pub label: String,
    pub value: f64,
    pub dim: Dim,
    pub bind: Option<Bind>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub decimals: usize,
    pub error: Option<String>,
    /// How much the value moves per point dragged; left out, as a bound
    /// `Number` drags.
    pub speed: Option<f64>,
    /// Its width in points.
    pub width: Option<f32>,
    pub enabled: bool,
    /// Takes the keyboard focus this frame, as a field just made does.
    pub focus: bool,
    /// Written after the value in place of the `dim`'s unit: `N·m`.
    pub unit: Option<String>,
    /// The value between a down and an up button, in place of a field.
    pub stepper: Option<Stepper>,
    /// The system's bare number drag in place of the panel's field: its
    /// `unit` written after the value as given, its decimals as the value
    /// needs (none when `decimals` is 0).
    pub bare: bool,
}

impl Default for Quantity {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            value: 0.0,
            dim: Dim::default(),
            bind: None,
            min: None,
            max: None,
            decimals: 2,
            error: None,
            speed: None,
            width: None,
            enabled: true,
            focus: false,
            unit: None,
            stepper: None,
            bare: false,
        }
    }
}

/// A [`Quantity`] drawn as a stepper: `text` (the value when empty)
/// between a down and an up button, each with its state and its words on
/// hover.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stepper {
    pub text: String,
    pub down: bool,
    pub up: bool,
    pub down_hint: String,
    pub up_hint: String,
}

/// A [`Widget::Check`]: `text` beside its box, in its section's label
/// column under `label` when that is not empty. Told as
/// `PanelEvent::Toggle`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Check {
    pub id: String,
    pub label: String,
    pub text: String,
    pub on: bool,
    pub enabled: bool,
    /// Words on hover.
    pub hint: String,
}

impl Default for Check {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            text: String::new(),
            on: false,
            enabled: true,
            hint: String::new(),
        }
    }
}

/// A [`Widget::Select`]: one of `options`, told as `PanelEvent::Choice`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Select {
    pub id: String,
    pub label: String,
    pub options: Vec<String>,
    pub selected: usize,
    /// Its width in points; the panel's field width when left out.
    pub width: Option<f32>,
    pub enabled: bool,
    /// Shown in the closed field in place of the option chosen: a count,
    /// or a prompt for a list that only adds (`selected` past the end
    /// then marks none).
    pub text: Option<String>,
    /// The options offered but not to be chosen now, by their place.
    pub disabled: Vec<usize>,
    /// Words on hover over each option, by its place; empty for none.
    pub hints: Vec<String>,
    /// The options with a line above them, setting them apart.
    pub separated: Vec<usize>,
    /// The options side by side in the row, the chosen one lit, in place
    /// of a dropdown.
    pub segmented: bool,
}

impl Default for Select {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            options: Vec::new(),
            selected: 0,
            width: None,
            enabled: true,
            text: None,
            disabled: Vec::new(),
            hints: Vec::new(),
            separated: Vec::new(),
            segmented: false,
        }
    }
}

/// A [`Widget::Grid`]: two columns, a field's label in the first and its
/// control in the second, the column as wide as the grid's longest label.
/// A widget with no label takes one cell, so two of them make a row.
/// Rows sit close, or a little apart when `roomy`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FieldGrid {
    /// Keeps it apart from another grid of the panel.
    pub id: String,
    pub children: Vec<Widget>,
    pub roomy: bool,
    /// The control column a little further from the labels.
    pub apart: bool,
}

/// A [`Widget::Alert`]: a note as [`Widget::Note`] draws it; with an
/// `id`, a click on it comes back as `PanelEvent::Button`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Alert {
    pub id: Option<String>,
    pub kind: NoteKind,
    pub title: Option<String>,
    pub text: String,
}

/// A [`Widget::Card`]: a raised card, its icon and title on the first
/// line, then its children.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Card {
    pub icon: Option<String>,
    pub title: String,
    /// The icon's size in points.
    pub icon_size: f32,
    pub children: Vec<Widget>,
    /// The room inside its edge in points, in place of the card's own.
    pub padding: Option<f32>,
}

impl Default for Card {
    fn default() -> Self {
        Self {
            icon: None,
            title: String::new(),
            icon_size: 18.0,
            children: Vec::new(),
            padding: None,
        }
    }
}

/// A [`Widget::Caption`]: words in the panel's text size, or the small
/// one, in a tone (`Neutral` is the panel's secondary text, `Muted`
/// dimmer still), or in the primary text colour.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Caption {
    pub text: String,
    pub tone: Tone,
    pub small: bool,
    pub mono: bool,
    /// A small heading over what follows, in small capitals.
    pub overline: bool,
    /// In the semibold face and the panel's primary text colour, over
    /// the tone: the name of what the widgets after it edit.
    pub strong: bool,
    /// In the medium face: a heading over the rows under it.
    pub medium: bool,
    /// In the primary text colour, in place of the tone's: a value read.
    pub primary: bool,
}

/// A [`Widget::Plot`]: lines along a strip `height` points tall, each
/// series scaled to its own largest value, all on one x range from zero.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Plot {
    pub height: f32,
    pub series: Vec<PlotSeries>,
}

/// A line of a [`Plot`]: its points as `[x, y]`, in its tone.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlotSeries {
    pub points: Vec<[f64; 2]>,
    pub tone: Tone,
}

/// A [`Widget::Cells`]: `children` in rows of `columns` cells, left to
/// right, each column as wide as its widest cell; rows sit close and
/// columns a little apart. Column titles as strong captions, then a
/// caption per reading, make a table that reads as the panel's text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cells {
    /// Keeps it apart from another of the panel.
    pub id: String,
    /// Cells a row; one when left at zero.
    pub columns: usize,
    pub children: Vec<Widget>,
}

/// How much room a [`Widget::Gap`] leaves.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapSize {
    #[default]
    Small,
    Medium,
    Large,
    /// The room before a part of a panel under its own heading, between
    /// `medium` and `large`.
    Section,
}

/// A [`Widget::Gap`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Gap {
    pub size: GapSize,
}

/// A colour of a [`Swatches`] block, as sRGB.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Swatch {
    pub color: [u8; 3],
    /// It is the colour in use: ringed in the accent.
    pub chosen: bool,
}

/// A [`Widget::Swatches`]: small squares of colour, side by side and
/// going on to the next line; a click is `PanelEvent::Choice` with the
/// swatch's place.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Swatches {
    pub id: String,
    pub swatches: Vec<Swatch>,
    /// Keys to the colours the view paints rather than colours to pick:
    /// smaller, unringed, and no click; one beside each caption in a flow
    /// makes a legend.
    pub keys: bool,
}

/// A [`Widget::Color`]: a button showing `color` that opens the system's
/// colour picker; each change is `PanelEvent::Color`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorField {
    pub id: String,
    pub label: String,
    /// Red, green and blue, each from 0 to 1 (sRGB).
    pub color: [f32; 3],
}

/// A [`Widget::Range`]: a slider over `min` to `max`, its value written
/// beside it with `decimals` and `suffix` (` %`); told as
/// `PanelEvent::Number`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Range {
    pub id: String,
    pub label: String,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    /// The values it stops at, every `step` from `min`.
    pub step: Option<f64>,
    pub decimals: usize,
    pub suffix: String,
    pub enabled: bool,
}

impl Default for Range {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            value: 0.0,
            min: 0.0,
            max: 1.0,
            step: None,
            decimals: 2,
            suffix: String::new(),
            enabled: true,
        }
    }
}

/// A [`Widget::RowButton`]: `text` at the left of a frameless button the
/// panel's width, told as `PanelEvent::Button`. The `current` one (where
/// what the panel moves stands) is in the accent, strong, and takes no
/// click.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RowButton {
    pub id: String,
    pub text: String,
    pub current: bool,
}

/// A [`Widget::Selectable`]: words that light up while `selected`, told
/// as `PanelEvent::Button` when clicked; a few in a [`Widget::Flow`] pick
/// one of them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Selectable {
    pub id: String,
    pub text: String,
    pub selected: bool,
}

/// A picture a panel shows, as rows of sRGB pixels with their alpha.
/// The host keeps it by `key`, so a picture shown again under the same
/// key is not sent to the screen again: a key names one picture.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Picture {
    pub key: String,
    pub width: u32,
    pub height: u32,
    /// `width × height` pixels of four bytes each, red, green, blue and
    /// alpha, row by row from the top.
    pub rgba: Vec<u8>,
}

/// A choice of a [`Gallery`]: its picture over its name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tile {
    pub label: String,
    pub picture: Picture,
}

/// A [`Widget::Gallery`]: small square pictures side by side, going on
/// to the next line, each over its name; the `selected` one ringed in
/// the accent. A click is `PanelEvent::Choice` with the tile's place.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Gallery {
    pub id: String,
    pub tiles: Vec<Tile>,
    pub selected: Option<usize>,
}

/// A [`Widget::Settings`]: under its overline `title` (none when empty),
/// a bordered group with one row per child, the child's label at the
/// left and its control at the right, a line between rows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SettingsGroup {
    pub title: String,
    pub children: Vec<Widget>,
}

macro_rules! block_into_widget {
    ($($block:ident => $variant:ident),* $(,)?) => {
        $(impl From<$block> for Widget {
            fn from(block: $block) -> Self {
                Widget::$variant(block)
            }
        })*
    };
}

block_into_widget! {
    PanelSection => Section,
    Item => Item,
    ItemList => Items,
    FilterField => Filter,
    StatusLine => Status,
    Stack => Stack,
    Flow => Flow,
    Fill => Fill,
    Fields => Fields,
    Inline => Inline,
    PickChip => PickChip,
    Entry => Entry,
    TaskHeader => TaskHeader,
    Quantity => Quantity,
    Check => Check,
    Select => Select,
    FieldGrid => Grid,
    Alert => Alert,
    Card => Card,
    Caption => Caption,
    Gap => Gap,
    Plot => Plot,
    Swatches => Swatches,
    ColorField => Color,
    Range => Range,
    RowButton => RowButton,
    Selectable => Selectable,
    Gallery => Gallery,
    SettingsGroup => Settings,
    Cells => Cells,
    PanelAction => Action,
}

/// A cell of a [`Widget::Sheet`].
///
/// A package builds these, so a variant keeps its fields for as long as
/// the contract's minor version holds: a new option comes as a new
/// variant, or as a new struct that derives `Default`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Cell {
    /// Text; `strong` for a row heading the rows under it.
    Text {
        text: String,
        #[serde(default)]
        mono: bool,
        #[serde(default)]
        strong: bool,
    },
    /// Text a click on selects its row (`PanelEvent::Select`).
    Link { text: String },
    /// Text the user edits, told once typing ends (`PanelEvent::CellText`).
    Edit { text: String },
    /// A box ticked or not (`PanelEvent::CellCheck`).
    Check {
        on: bool,
        #[serde(default)]
        hint: Option<String>,
    },
}

/// A line of a [`Widget::Diagram`]: what it stands for, not its colour,
/// so the host draws it in the design's own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagramStroke {
    /// The outline of the part.
    #[default]
    Outline,
    /// An edge behind the surface, dashed.
    Hidden,
    /// An axis or a centre line, long-dashed and faint.
    Axis,
    /// A faint line: a hatch, a reference.
    Thin,
    /// Drawn in the accent colour.
    Accent,
}

/// A shape of a [`Widget::Diagram`], in its coordinates.
///
/// A package builds these, so a variant keeps its fields for as long as
/// the contract's minor version holds: a new option comes as a new
/// variant, or as a new struct that derives `Default`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DiagramShape {
    /// A run of straight lines; `fill` shades it, and reads only a convex
    /// outline right.
    Path {
        points: Vec<[f32; 2]>,
        #[serde(default)]
        closed: bool,
        #[serde(default)]
        stroke: DiagramStroke,
        #[serde(default)]
        fill: bool,
    },
    Circle {
        center: [f32; 2],
        radius: f32,
        #[serde(default)]
        stroke: DiagramStroke,
        #[serde(default)]
        fill: bool,
    },
    /// Text at a point, centred on it.
    Text {
        at: [f32; 2],
        text: String,
        #[serde(default)]
        mono: bool,
    },
}

/// A measure of a [`Widget::Diagram`]: the distance from `from` to `to`,
/// drawn as a dimension line `offset` away from them (to the left of the
/// way from `from` to `to`; negative for the right), its ends led out to
/// the points, with `text` on it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Dimension {
    pub from: [f32; 2],
    pub to: [f32; 2],
    #[serde(default)]
    pub offset: f32,
    pub text: String,
    /// Drawn in the accent colour: the one being edited.
    #[serde(default)]
    pub emphasis: bool,
}

/// A note of a [`Widget::Diagram`] pointing at a feature of it: a dot at
/// `anchor`, a leader to `at`, and `text` there.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Callout {
    pub anchor: [f32; 2],
    pub at: [f32; 2],
    pub text: String,
    #[serde(default)]
    pub emphasis: bool,
}

fn two() -> usize {
    2
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonStyle {
    Primary,
    #[default]
    Secondary,
    Destructive,
    /// A small secondary button, for an action on the row it sits in.
    Small,
    /// The plain button of the system's controls, unframed in style.
    Plain,
    /// A plain button in the small text size.
    PlainSmall,
    /// Outlined in the accent colour: takes what is picked in the view.
    Outline,
}

/// A number field's parameter: feature `feature`'s parameter `key`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bind {
    pub feature: String,
    pub key: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ListItem {
    pub label: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

/// Which declared panel an event came from.
#[cfg_attr(not(feature = "exhaustive"), non_exhaustive)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelSlot {
    Task,
    Settings,
    /// The strip under the view (`Frame::transport`).
    Transport,
}

/// What the user did to a panel widget, by the widget's id.
#[cfg_attr(not(feature = "exhaustive"), non_exhaustive)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PanelEvent {
    Number {
        id: String,
        value: f64,
    },
    Choice {
        id: String,
        index: usize,
    },
    Toggle {
        id: String,
        on: bool,
    },
    Text {
        id: String,
        value: String,
    },
    Button {
        id: String,
    },
    Pick {
        id: String,
    },
    Select {
        id: String,
        index: usize,
    },
    /// A cell of a table's editable column, typed in: its text as left.
    Cell {
        id: String,
        row: usize,
        column: usize,
        value: String,
    },
    /// A sheet's editable cell, once typing ended.
    CellText {
        id: String,
        row: usize,
        column: usize,
        value: String,
    },
    /// A sheet's box ticked or cleared.
    CellCheck {
        id: String,
        row: usize,
        column: usize,
        on: bool,
    },
    /// A diagram pressed at `x`, `y` in its own coordinates; `dragging`
    /// for each move of the pointer held down after the press, and
    /// `released` once, where the drag ended.
    DiagramPress {
        id: String,
        x: f32,
        y: f32,
        #[serde(default)]
        dragging: bool,
        #[serde(default)]
        released: bool,
    },
    /// The pointer came onto an item (`index` in its list, 0 for an
    /// item alone), or left the list (`None`).
    Hover {
        id: String,
        index: Option<usize>,
    },
    /// An item double clicked.
    Activate {
        id: String,
        index: usize,
    },
    /// An entry of an item's menu chosen: the entry's id in `action`.
    ContextMenu {
        id: String,
        index: usize,
        action: String,
    },
    /// A tag of an item clicked: `tag` is its place among the item's.
    Tag {
        id: String,
        index: usize,
        tag: usize,
    },
    /// A multiple list's selection, whole, as the user left it.
    Selection {
        id: String,
        indices: Vec<usize>,
    },
    /// A filter the bench applies, typed in or its kind chosen.
    Filter {
        id: String,
        text: String,
        kind: usize,
    },
    /// A picked chip cleared.
    Clear {
        id: String,
    },
    /// A badge of an item clicked: `badge` is its place among the item's.
    Badge {
        id: String,
        index: usize,
        badge: usize,
    },
    /// What the pointer is on in a list changed: the item (`None` off
    /// the list), and the tag or badge of it under the pointer.
    HoverPart {
        id: String,
        index: Option<usize>,
        tag: Option<usize>,
        badge: Option<usize>,
    },
    /// An item's name typed in while it is `editing`: at each change,
    /// and with `done` as the field is left.
    ItemText {
        id: String,
        index: usize,
        value: String,
        done: bool,
    },
    /// A colour field's colour changed: red, green and blue from 0 to 1.
    Color {
        id: String,
        color: [f32; 3],
    },
    /// A `live` [`Entry`]'s field left, its text as it stands: by Escape
    /// when `escaped`, which drops what was typed.
    TextLeft {
        id: String,
        value: String,
        escaped: bool,
    },
}

impl PanelEvent {
    pub fn id(&self) -> &str {
        match self {
            PanelEvent::Number { id, .. }
            | PanelEvent::Choice { id, .. }
            | PanelEvent::Toggle { id, .. }
            | PanelEvent::Text { id, .. }
            | PanelEvent::Button { id }
            | PanelEvent::Pick { id }
            | PanelEvent::Select { id, .. }
            | PanelEvent::Cell { id, .. }
            | PanelEvent::CellText { id, .. }
            | PanelEvent::CellCheck { id, .. }
            | PanelEvent::DiagramPress { id, .. }
            | PanelEvent::Hover { id, .. }
            | PanelEvent::Activate { id, .. }
            | PanelEvent::ContextMenu { id, .. }
            | PanelEvent::Tag { id, .. }
            | PanelEvent::Selection { id, .. }
            | PanelEvent::Filter { id, .. }
            | PanelEvent::Clear { id }
            | PanelEvent::Badge { id, .. }
            | PanelEvent::HoverPart { id, .. }
            | PanelEvent::ItemText { id, .. }
            | PanelEvent::Color { id, .. }
            | PanelEvent::TextLeft { id, .. } => id,
        }
    }
}

// ------------------------------------------------------------------- menus

#[cfg_attr(not(feature = "exhaustive"), non_exhaustive)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum MenuScope {
    ViewportBody(String),
    TreeFeature(String),
    TreeBody(String),
    StartPage,
    EditMenu,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuItem {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub hint: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub separator_before: bool,
}

impl Default for MenuItem {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            icon: None,
            hint: None,
            enabled: yes(),
            separator_before: false,
        }
    }
}

// --------------------------------------------------------------- requests

/// Something a bench asks the host to do once its call returns.
///
/// A package builds these, so a variant keeps its fields for as long as
/// the contract's minor version holds: a new option comes as a new
/// variant, or as a new struct that derives `Default`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    ActivateTool {
        tool: String,
    },
    SelectBody {
        body: String,
    },
    /// Name the undo step this gesture closes.
    JournalLabel {
        label: String,
    },
    SwitchWorkbench {
        workbench: String,
    },
    OrientCamera {
        origin: [f32; 3],
        normal: [f32; 3],
        up: [f32; 3],
    },
    FinishEditing,
    /// Ask the user where to save `contents` (the `save_dialog`
    /// capability).
    SaveFile {
        name: String,
        kind: String,
        extension: String,
        contents: Vec<u8>,
    },
    /// Ask the user for a folder and write every file into it (the
    /// `save_dialog` capability): one program per operation.
    SaveFiles {
        files: Vec<FileToWrite>,
    },
    /// Ask the user for a file to read, or several with `multiple` (the
    /// `open_dialog` capability), of the `kinds` offered; the answer comes
    /// as `Event::FileOpened` with this `id`.
    OpenFile {
        id: String,
        kinds: Vec<FileKind>,
        #[serde(default)]
        multiple: bool,
    },
    /// Put `text` on the system clipboard (contract 0.1.4): a table the
    /// bench made, to paste into a spreadsheet.
    CopyText {
        text: String,
    },
}

// ------------------------------------------------------------------ calls

/// The document commands a bench calls through `host.call`. Each is an
/// ordinary recorded edit: one undo step per gesture, sent to peers.
pub mod calls {
    /// `{kind, name, body?, data, deps?}` → the new feature's id. `kind`
    /// must be one the package owns.
    pub const ADD_FEATURE: &str = "doc.add_feature";
    /// `{id, data}`: replace an owned feature's data.
    pub const SET_FEATURE_DATA: &str = "doc.set_feature_data";
    /// `{id}`: remove an owned feature.
    pub const REMOVE_FEATURE: &str = "doc.remove_feature";
    /// `{id, after?}`: move an owned feature in its body's history to just
    /// after `after`, a feature of the same body, or first without it. A
    /// feature passes no feature it uses, nor one using it.
    pub const MOVE_FEATURE: &str = "doc.move_feature";
    /// `{id, name}`
    pub const RENAME_FEATURE: &str = "doc.rename_feature";
    /// `{id, visible}`
    pub const SET_FEATURE_VISIBLE: &str = "doc.set_feature_visible";
    /// `{name?}` → the new body's id.
    pub const CREATE_BODY: &str = "doc.create_body";
    /// `{id}`: remove a body with the package's own features on it, never
    /// one holding another package's feature or taking its shape from an
    /// import, a link or a copy. An empty body's removal undoes, so the
    /// body a cancelled part made can go without clearing the history.
    pub const REMOVE_BODY: &str = "doc.remove_body";
    /// `{body, matrix}`: a row-major rigid 4x4.
    pub const SET_PLACEMENT: &str = "doc.set_placement";
    /// `{entry, input}` → the job's number. The result arrives as
    /// `Event::JobFinished`.
    pub const JOB_START: &str = "job.start";
    /// `{job}`
    pub const JOB_CANCEL: &str = "job.cancel";
    /// `{body}` → the round holes in the body's solid, where it sits:
    /// `[{entry, direction, diameter, depth, through, drill_point_deg}]`.
    /// Read only; answered where the call may change the document (an
    /// event, a panel change, a command), refused elsewhere.
    pub const GEOM_HOLES: &str = "geom.holes";
    /// `{body, point}` → the edges round the face of the body's solid at
    /// `point` (world), each `{point, direction}` halfway along it, where
    /// the body sits. Read only, answered as `geom.holes` is.
    pub const GEOM_FACE_EDGES: &str = "geom.face_edges";
    /// `{body, plane: {origin, normal, x_axis?}, face?}` →
    /// [`Section`](crate::Section): where the plane (world) crosses the body's
    /// solid, or with `face` (a point, world) only the face there. Read
    /// only, answered as `geom.holes` is.
    pub const GEOM_SECTION: &str = "geom.section";
    /// `{body, point?, name?}` → [`FaceLoops`](crate::FaceLoops): the
    /// boundary loops of the face named `name` (the one of several
    /// nearest `point`), or of the face at `point` (world). Read only,
    /// answered as `geom.holes` is.
    pub const GEOM_FACE_LOOPS: &str = "geom.face_loops";
}

/// What `geom.section` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    /// The plane as cut, world space: the curves' 2D coordinates run
    /// along its `x_axis` and `y_axis`.
    pub plane: kernel_api::ProfilePlane,
    pub loops: Vec<SectionLoop>,
}

/// Curves of a section end to end: closed round a solid's cut, open where
/// a sheet's edge stops it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionLoop {
    pub closed: bool,
    pub curves: Vec<SectionCurve>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionCurve {
    /// The exact curve, in the plane's 2D coordinates.
    pub curve: kernel_api::ProjectedEdge,
    /// The loop runs against the way `curve` does.
    #[serde(default)]
    pub reversed: bool,
    /// Points along it in the loop's order, world space.
    pub points: Vec<[f64; 3]>,
    /// The name of the face it crosses, when the solid names its faces.
    #[serde(default)]
    pub face: Option<u64>,
}

/// What `geom.face_loops` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaceLoops {
    /// The face's name, when the solid names its faces.
    #[serde(default)]
    pub name: Option<u64>,
    /// The outer loop first, then the holes, each edge in turn.
    pub loops: Vec<Vec<LoopEdge>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoopEdge {
    /// Points along it in the loop's order, world space.
    pub points: Vec<[f64; 3]>,
    pub length: f64,
    #[serde(default)]
    pub circle: Option<EdgeCircle>,
    /// The edge a round face meets itself along, in its loop twice.
    #[serde(default)]
    pub seam: bool,
    /// The names of the two faces it runs between (as
    /// [`EdgeRef::faces`]).
    #[serde(default)]
    pub faces: Option<[u64; 2]>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_loads_packages_of_its_own_contract_version() {
        assert!(api_compatible("printcad:workbench@0.1"));
        assert!(api_compatible("0.1.4"));
        assert!(!api_compatible("printcad:workbench@0.2"));
        assert!(!api_compatible("1.0"));
        // A manifest's `min_contract`, held against this build's patch.
        assert!(contract_satisfies(CONTRACT_VERSION));
        assert!(contract_satisfies("0.1"));
        assert!(contract_satisfies("printcad:workbench@0.1.0"));
        assert!(!contract_satisfies("0.1.999"));
        assert!(!contract_satisfies("0.2.0"));
        assert!(!contract_satisfies("next"));
    }

    #[test]
    fn a_block_reads_with_its_fields_left_out_and_writes_back_the_same() {
        let section: Widget =
            serde_json::from_str(r#"{"type":"section","title":"Elements","count":9}"#).unwrap();
        let Widget::Section(PanelSection {
            open,
            count,
            children,
            ..
        }) = &section
        else {
            panic!("{section:?}");
        };
        assert!(*open && *count == Some(9) && children.is_empty());
        let item: Widget = Item {
            label: "Line 1".into(),
            value: Some("89.92 mm".into()),
            ..Default::default()
        }
        .into();
        for widget in [section, item] {
            let json = serde_json::to_string(&widget).unwrap();
            assert_eq!(serde_json::from_str::<Widget>(&json).unwrap(), widget);
        }
    }

    /// `major.minor` of a version such as `0.1.4`.
    fn contract(version: &str) -> String {
        version.split('.').take(2).collect::<Vec<_>>().join(".")
    }

    #[test]
    fn the_crate_the_wit_package_and_the_contract_share_a_version() {
        let declared = WIT
            .lines()
            .find_map(|line| line.trim().strip_prefix("package printcad:workbench@"))
            .and_then(|rest| rest.strip_suffix(';'))
            .expect("the WIT file names its package");
        assert_eq!(contract(declared), API_VERSION);
        assert_eq!(contract(env!("CARGO_PKG_VERSION")), API_VERSION);
    }

    #[test]
    fn tagged_values_read_as_a_guest_writes_them() {
        let widget: Widget = serde_json::from_str(
            r#"{"type":"number","id":"teeth","label":"Teeth","value":20,"bind":{"feature":"f","key":"/teeth"}}"#,
        )
        .unwrap();
        match widget {
            Widget::Number {
                decimals,
                dim,
                bind,
                ..
            } => {
                assert_eq!(decimals, 2);
                assert_eq!(dim, Dim::Length);
                assert_eq!(bind.unwrap().key, "/teeth");
            }
            other => panic!("{other:?}"),
        }
        let plan: Plan = serde_json::from_str(r#"{"type":"error","message":"no"}"#).unwrap();
        assert_eq!(
            plan,
            Plan::Error {
                feature: None,
                message: "no".into()
            }
        );
        let scope = serde_json::to_string(&MenuScope::TreeBody("b".into())).unwrap();
        assert_eq!(scope, r#"{"type":"tree_body","id":"b"}"#);
        assert_eq!(
            serde_json::from_str::<MenuScope>(&scope).unwrap(),
            MenuScope::TreeBody("b".into())
        );
        let event = serde_json::to_string(&Event::JobFinished {
            job: 3,
            result: Ok("x".into()),
        })
        .unwrap();
        assert_eq!(
            event,
            r#"{"type":"job_finished","job":3,"result":{"Ok":"x"}}"#
        );
    }

    #[test]
    fn layout_widgets_read_as_a_guest_writes_them() {
        let widget: Widget = serde_json::from_str(
            r#"{"type":"row","children":[
                {"type":"value","label":"Moves","value":"Part"},
                {"type":"hinted","hint":"Why","widget":
                    {"type":"button","id":"x","label":"X","style":"small"}}
            ]}"#,
        )
        .unwrap();
        assert_eq!(
            widget,
            Widget::Row {
                children: vec![
                    Widget::Value {
                        label: "Moves".into(),
                        value: "Part".into(),
                        mono: false,
                    },
                    Widget::Hinted {
                        hint: "Why".into(),
                        widget: Box::new(Widget::Button {
                            id: "x".into(),
                            label: "X".into(),
                            style: ButtonStyle::Small,
                            enabled: true,
                        }),
                    },
                ],
            }
        );
    }

    #[test]
    fn a_sheet_and_its_cell_events_read_as_written() {
        let widget: Widget = serde_json::from_str(
            r#"{"type":"sheet","id":"parts","columns":["Part","Bought"],"rows":[[
                {"type":"link","text":"Bolt"},{"type":"check","on":true}
            ]]}"#,
        )
        .unwrap();
        let Widget::Sheet { rows, .. } = widget else {
            panic!("{widget:?}");
        };
        assert_eq!(
            rows[0][1],
            Cell::Check {
                on: true,
                hint: None
            }
        );
        let event = PanelEvent::CellText {
            id: "parts".into(),
            row: 2,
            column: 5,
            value: "ACME".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(
            json,
            r#"{"type":"cell_text","id":"parts","row":2,"column":5,"value":"ACME"}"#
        );
        assert_eq!(event.id(), "parts");
    }

    #[test]
    fn a_struct_left_to_its_defaults_reads_as_its_fields_left_out() {
        fn read<T: serde::de::DeserializeOwned>(json: &str) -> T {
            serde_json::from_str(json).unwrap()
        }
        assert_eq!(read::<Tool>(r#"{"id":"","label":""}"#), Tool::default());
        assert_eq!(
            read::<Parameter>(r#"{"key":"","label":"","pointer":""}"#),
            Parameter::default()
        );
        assert_eq!(
            read::<MenuItem>(r#"{"id":"","label":""}"#),
            MenuItem::default()
        );
        assert_eq!(
            read::<Label>(r#"{"at":[0,0,0],"text":"","color":[1,1,1]}"#),
            Label::default()
        );
        assert_eq!(
            read::<Mesh>(r#"{"positions":[],"indices":[],"color":[1,1,1]}"#),
            Mesh::default()
        );
        assert_eq!(
            read::<Polyline>(r#"{"points":[],"color":[1,1,1]}"#),
            Polyline::default()
        );
        assert_eq!(
            read::<Region>(r#"{"outline":[],"color":[1,1,1]}"#),
            Region::default()
        );
        assert_eq!(
            read::<KeptMesh>(r#"{"handle":0,"color":[1,1,1]}"#),
            KeptMesh::default()
        );
        assert_eq!(
            read::<Manifest>(r#"{"id":"","name":"","version":"","api":"printcad:workbench@0.1"}"#),
            Manifest::default()
        );
    }

    /// The SDK passes over what it reads this way: a kind of event a newer
    /// app sends within the same contract.
    #[test]
    fn an_event_of_a_kind_not_known_yet_reads_as_an_unknown_variant() {
        let input = r#"{"event":{"type":"gesture","scale":2},"pointer":{}}"#;
        let error = serde_json::from_str::<Input>(input).unwrap_err();
        assert!(error.to_string().starts_with("unknown variant"), "{error}");
        let error = serde_json::from_str::<PanelEvent>(r#"{"type":"drag","id":"x"}"#).unwrap_err();
        assert!(error.to_string().starts_with("unknown variant"), "{error}");
    }

    #[test]
    fn a_frame_left_empty_reads_from_nothing() {
        let frame: Frame = serde_json::from_str("{}").unwrap();
        assert_eq!(frame, Frame::default());
    }
}
