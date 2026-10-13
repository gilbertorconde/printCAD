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
//!   variant does not break it. (The `exhaustive` feature lifts this for
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
    /// Ask the user where to save a file it made.
    #[serde(default)]
    pub save_dialog: bool,
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
    /// the panel's width; strokes and text keep their pixel size.
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
            | PanelEvent::CellCheck { id, .. } => id,
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
