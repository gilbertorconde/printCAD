//! What a WebAssembly workbench and printCAD say to each other.
//!
//! The component interface (`wit/workbench.wit`) carries these types as
//! JSON text; this crate is their one definition, used by the host
//! (`wb_wasm`) and by the guest SDK. Ids (features, bodies) travel as
//! UUID strings, lengths in millimetres, angles in degrees unless a field
//! says otherwise, and every position is in world space.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use kernel_api::{self, SolidOp, TriMesh};

/// The version of this contract, as a package's manifest names it in
/// `api`. A host loads packages of its own major version.
pub const API_VERSION: &str = "0.1";

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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dim {
    #[default]
    Length,
    Angle,
    Number,
}

/// How a feature shows in the tree, the property panel and the hover card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rebuild {
    pub body: String,
    pub plan: Plan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Plan {
    /// Build these ops; `op_features[i]` is the feature op `i` is for.
    Ops {
        ops: Vec<SolidOp>,
        op_features: Vec<String>,
    },
    /// Nothing to build: the body's solid goes.
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeRef {
    pub body: String,
    pub point: [f32; 3],
    pub direction: [f32; 3],
    pub length: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    Left,
    Middle,
    Right,
    Other,
}

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    pub at: [f32; 3],
    pub color: Rgb,
    pub kind: MarkKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MarkKind {
    Dot { radius: f32 },
    Cross { size: f32 },
    Icon { name: String, size: f32 },
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

// ------------------------------------------------------------------ panel

/// One element of a declared panel. The host draws it with the app's
/// widgets and sends back a [`PanelEvent`] when the user changes it.
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
    /// Columns of text, one row selectable.
    Table {
        id: String,
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
        #[serde(default)]
        selected: Option<usize>,
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
}

/// A number field's parameter: feature `feature`'s parameter `key`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bind {
    pub feature: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListItem {
    pub label: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

/// Which declared panel an event came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelSlot {
    Task,
    Settings,
}

/// What the user did to a panel widget, by the widget's id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PanelEvent {
    Number { id: String, value: f64 },
    Choice { id: String, index: usize },
    Toggle { id: String, on: bool },
    Text { id: String, value: String },
    Button { id: String },
    Pick { id: String },
    Select { id: String, index: usize },
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
            | PanelEvent::Select { id, .. } => id,
        }
    }
}

// ------------------------------------------------------------------- menus

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

// --------------------------------------------------------------- requests

/// Something a bench asks the host to do once its call returns.
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
    /// `{body, matrix}`: a row-major rigid 4x4.
    pub const SET_PLACEMENT: &str = "doc.set_placement";
    /// `{entry, input}` → the job's number. The result arrives as
    /// `Event::JobFinished`.
    pub const JOB_START: &str = "job.start";
    /// `{job}`
    pub const JOB_CANCEL: &str = "job.cancel";
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
    fn a_frame_left_empty_reads_from_nothing() {
        let frame: Frame = serde_json::from_str("{}").unwrap();
        assert_eq!(frame, Frame::default());
    }
}
