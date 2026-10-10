// Without the egui feature the crate is the headless subset the tests and
// the kernel bench use; the panel-only state and helpers go unused there.
#![cfg_attr(not(feature = "egui"), allow(dead_code))]

mod commands;
pub mod conic;
mod constrain;
mod curves;
mod dxf;
mod external;
mod external_ref;
mod feature;
pub mod generator;
mod geom2d;
mod glyphs;
mod images;
pub mod internal;
mod measure;
mod overlay;
mod ovp;
#[cfg(feature = "egui")]
mod panel;
mod params;
pub mod profile;
pub mod render;
mod repair;
mod selection;
pub mod sketch;
pub mod snap;
mod solver;
pub use solver::report as solver_report;
pub mod spline;
pub mod spline_edit;
mod step;
pub mod style;
pub mod text;
mod tools;
mod walls;

use std::collections::{HashMap, HashSet};
use std::time::Duration;
use web_time::Instant;

use core_document::{
    BasePlane, BodyId, FeatureId, FeatureInfo, HostRequest, InputResult, KeyCode, MenuItem,
    MenuScope, ScreenSpaceLabel, ScreenSpaceMark, SketchPalette, StatusItems, TaskInfo,
    ToolDescriptor, ToolHint, ToolVariant, ViewportHud, Workbench, WorkbenchContext,
    WorkbenchDescriptor, WorkbenchFeature, WorkbenchInputEvent, WorkbenchRuntimeContext,
    base_tool_id, tool_variant,
};
pub use feature::{AttachedSupport, DatumSupport, FaceSupport, LentFace, SketchFeature};
use overlay::SketchProjector;
use ovp::DimCapture;
use sketch::{Constraint, GeometryElement, Sketch, SketchPlane, Vec2D};
use solver::{Diagnosis, SolveOutcome};
pub use tools::ToolParams;
use tools::ToolState;
use uuid::Uuid;

/// Snap / hit-test tolerance in screen pixels, converted to sketch units
/// per frame from the current zoom.
const SNAP_TOLERANCE_PX: f32 = 8.0;

/// Elements-panel filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ElementFilter {
    #[default]
    All,
    Normal,
    Construction,
}

impl ElementFilter {
    pub(crate) fn accepts(self, sketch: &Sketch, id: Uuid) -> bool {
        match self {
            ElementFilter::All => true,
            ElementFilter::Normal => !sketch.is_construction(id),
            ElementFilter::Construction => sketch.is_construction(id),
        }
    }
}

/// How the constraint list names the constraint at `idx`: its own name, or
/// its place and kind.
pub(crate) fn constraint_row_label(idx: usize, c: &Constraint) -> String {
    c.name.clone().unwrap_or_else(|| {
        format!(
            "Constraint{} · {}",
            idx + 1,
            sketch::constraint_label(&c.kind)
        )
    })
}

/// Constraint-list filter: which constraints the panel lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConstraintFilter {
    #[default]
    All,
    /// Relations with no value: coincident, parallel, tangent and the like.
    Geometric,
    /// Those with a value, driving or not.
    Dimensional,
    /// Those the user named.
    Named,
    /// Dimensions that measure without driving.
    Reference,
    /// The selected constraints.
    Selected,
    /// Those on the selected geometry, a curve's own points included, and
    /// on the curves meeting at a selected corner.
    Related,
    /// Those on the parked layer.
    Parked,
}

impl ConstraintFilter {
    pub const ALL: [(ConstraintFilter, &'static str); 8] = [
        (ConstraintFilter::All, "All"),
        (ConstraintFilter::Geometric, "Geometric"),
        (ConstraintFilter::Dimensional, "Dimensional"),
        (ConstraintFilter::Named, "Named"),
        (ConstraintFilter::Reference, "Reference"),
        (ConstraintFilter::Selected, "Selected"),
        (ConstraintFilter::Related, "Related to selection"),
        (ConstraintFilter::Parked, "Parked"),
    ];

    /// Whether `c` is listed, with `selected` the selected geometry and
    /// `selected_constraints` the selected constraints.
    pub(crate) fn accepts(
        self,
        sketch: &Sketch,
        c: &Constraint,
        selected: &HashSet<Uuid>,
        selected_constraints: &HashSet<Uuid>,
    ) -> bool {
        match self {
            ConstraintFilter::All => true,
            ConstraintFilter::Geometric => !c.kind.is_dimensional(),
            ConstraintFilter::Dimensional => c.kind.is_dimensional(),
            ConstraintFilter::Named => c.name.as_deref().is_some_and(|n| !n.trim().is_empty()),
            ConstraintFilter::Reference => c.kind.is_dimensional() && !c.driving,
            ConstraintFilter::Selected => selected_constraints.contains(&c.id),
            ConstraintFilter::Related => {
                let reach = related_reach(sketch, selected);
                sketch::constraint_refs(&c.kind)
                    .iter()
                    .any(|id| reach.contains(id))
            }
            ConstraintFilter::Parked => c.parked,
        }
    }
}

/// What the selection reaches, for the Related filter: the selected
/// geometry and a selected curve's own points; and a selected point is a
/// corner, reaching every curve that ends or centres there. Points held
/// together by a Coincident constraint are one point, so a corner joined
/// by a constraint rather than a shared point reaches both sides. A curve
/// selected reaches its own constraints, not its neighbours'.
fn related_reach(sketch: &Sketch, selected: &HashSet<Uuid>) -> HashSet<Uuid> {
    let mut corners: HashSet<Uuid> = HashSet::new();
    let mut ends: HashSet<Uuid> = HashSet::new();
    for id in selected {
        match sketch.get_geometry(*id) {
            Some(GeometryElement::Point(_)) => {
                corners.insert(*id);
            }
            Some(element) => ends.extend(Sketch::curve_point_ids(element)),
            None => {}
        }
    }
    let corners = coincident_with(sketch, corners);
    let ends = coincident_with(sketch, ends);
    let mut reach: HashSet<Uuid> = selected.clone();
    reach.extend(corners.iter().chain(&ends).copied());
    for element in &sketch.geometry {
        if Sketch::curve_point_ids(element)
            .iter()
            .any(|p| corners.contains(p))
        {
            reach.insert(element.id());
        }
    }
    reach
}

/// `points` and every point a chain of Coincident constraints holds to one
/// of them.
fn coincident_with(sketch: &Sketch, mut points: HashSet<Uuid>) -> HashSet<Uuid> {
    loop {
        let before = points.len();
        for c in &sketch.constraints {
            if let sketch::ConstraintKind::Coincident { point1, point2 } = c.kind {
                if points.contains(&point1) {
                    points.insert(point2);
                } else if points.contains(&point2) {
                    points.insert(point1);
                }
            }
        }
        if points.len() == before {
            return points;
        }
    }
}

/// What a new sketch stands on besides its plane, and what makes it.
#[derive(Default)]
struct NewSketchOn {
    support: Option<crate::feature::DatumSupport>,
    face: Option<crate::feature::FaceSupport>,
    attached: Option<crate::feature::AttachedSupport>,
    made_by: Option<generator::Generator>,
}

/// A "create sketch" request waiting for the user to pick a plane.
struct PendingCreation {
    body: Option<BodyId>,
    /// The solid face that was selected when the request was made, in the
    /// body's frame, offered as the first choice in the picker: a sketch
    /// placed on it follows it.
    face: Option<core_document::FaceRef>,
    /// Where `face` comes from: whether a sketch on it follows it.
    face_origin: core_document::FaceOrigin,
    /// The generator the sketch is made by, if any.
    generator: Option<generator::Generator>,
}

/// In-progress box selection (select mode, started by pressing on empty
/// space). Corners are in sketch coordinates.
struct BoxSelect {
    anchor: Vec2D,
    current: Vec2D,
}

/// In-progress drag of sketch geometry in select mode.
struct DragState {
    /// Every point the drag moves, with its position at press time.
    points: Vec<(Uuid, Vec2D)>,
    /// Cursor position (sketch coords) at press time.
    grab: Vec2D,
    /// The element the press landed on.
    hit: Uuid,
    /// It was already selected when the press landed: the drag carries the
    /// whole selection, and a release without movement drops it again.
    was_selected: bool,
    moved: bool,
    /// The elements the drag carries, and how far it has carried them:
    /// what a recording of it says.
    elements: Vec<Uuid>,
    delta: Vec2D,
}

/// A shape the active tool is drawing, as a recording will say it: the
/// `sketch.draw` call its clicks add up to.
struct DrawRecord {
    sketch: FeatureId,
    tool: String,
    /// Each click (a point, or a point with typed values), and the polyline's
    /// segment switches ("line", "arc", "perpendicular_arc", "reverse_arc")
    /// and "finish".
    events: Vec<serde_json::Value>,
    /// The other arguments: tolerance, tool settings, construction.
    settings: serde_json::Map<String, serde_json::Value>,
    /// Something was made or changed.
    changed: bool,
    elements_before: HashSet<Uuid>,
    constraints_before: HashSet<Uuid>,
}

/// In-progress drag of a dimension label (select mode).
struct LabelDrag {
    constraint: Uuid,
    /// The label has moved: a press that stays within the pick tolerance
    /// is a click that selects, and leaves the label where it was.
    moved: bool,
    /// Cursor position (sketch coords) at press time.
    grab: Vec2D,
    /// Offset the glyph was drawn with at press time (default when unset).
    base: Vec2D,
    /// Where the drag has the label now; written to the sketch once, on
    /// release, and drawn from here until then.
    current: Option<Vec2D>,
}

/// An in-viewport dimension edit (opened by double-clicking a dimensional
/// glyph, or as a tool adds a dimension; drawn from the task panel).
#[derive(Debug, Clone)]
pub struct DimEdit {
    pub constraint: Uuid,
    /// Label position at open time, viewport px.
    pub screen_pos: [f32; 2],
    pub text: String,
    pub driving: bool,
    /// The dimension was just added by a tool: it stands at what it
    /// measured, whatever the editor ends with.
    pub new: bool,
    /// The value goes into a variable the dimension then reads.
    pub save_as: Option<SaveAs>,
    /// The text is taken whole by the first key typed; set as the editor
    /// opens, cleared once the edit has selected it.
    pub select_all: bool,
}

impl DimEdit {
    /// An editor on `constraint` showing `text`, its text selected.
    pub fn new(constraint: Uuid, screen_pos: [f32; 2], text: String, driving: bool) -> Self {
        Self {
            constraint,
            screen_pos,
            text,
            driving,
            new: false,
            save_as: None,
            select_all: true,
        }
    }
}

/// Where a dimension's value goes when it is saved as a variable.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveAs {
    /// The variable's name.
    pub name: String,
    /// The set it joins; `None` makes a set for it.
    pub set: Option<FeatureId>,
}

/// A constraint glyph resolved under a click.
struct GlyphHit {
    constraint: Uuid,
    dimensional: bool,
    pos: [f32; 2],
}

const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(400);
/// The angle a drawn segment keeps to with Shift held.
const ANGLE_STEP_DEG: f32 = 15.0;
/// How far the shaded regions stand off the sketch plane, in mm.
const REGION_LIFT_MM: f64 = 0.01;
/// How much of the geometry colour the shaded regions take.
const REGION_OPACITY: f32 = 0.12;

/// The switches on the sketcher's panel and Preferences page.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SketchOptions {
    /// A grid on the sketch plane.
    pub grid_on: bool,
    /// The grid step follows the zoom instead of `grid_size`.
    pub grid_auto: bool,
    /// The grid step in sketch units when it does not follow the zoom.
    pub grid_size: f32,
    /// Drawing tools land on grid points.
    pub grid_snap: bool,
    /// Constraint glyphs stay off the viewport.
    pub constraints_hidden: bool,
    /// Axis-snapped lines get horizontal/vertical constraints as drawn.
    pub auto_constraints: bool,
    /// An auto constraint the solver reports redundant is dropped again.
    pub avoid_redundant_auto: bool,
    /// A constraint a constraint tool adds takes away the older ones it
    /// made redundant.
    pub auto_remove_redundant: bool,
    /// The solver runs after every edit; off, it runs on request.
    pub auto_update: bool,
    /// Construction geometry draws over normal geometry while editing;
    /// off, normal geometry draws over construction.
    pub construction_on_top: bool,
    /// The wall thickness check marks a wall thinner than this, in mm.
    pub min_wall: f32,
    /// A dimension a tool adds opens its value editor at once.
    pub ask_dimension_value: bool,
    /// What dimensions' labels show.
    #[serde(default)]
    pub dimension_labels: glyphs::DimensionLabels,
    /// Splines draw their curvature combs.
    pub spline_comb: bool,
    /// Splines mark their knots.
    pub spline_knots: bool,
    /// Editing a sketch cuts the view at its plane unless the sketch was
    /// set otherwise; off, only a sketch switched on is cut.
    pub section_on_open: bool,
    /// The regions the profile closes are shaded while editing.
    pub shade_regions: bool,
}

impl Default for SketchOptions {
    fn default() -> Self {
        Self {
            grid_on: false,
            grid_auto: true,
            grid_size: 10.0,
            grid_snap: false,
            constraints_hidden: false,
            auto_constraints: true,
            avoid_redundant_auto: true,
            auto_remove_redundant: false,
            auto_update: true,
            construction_on_top: false,
            min_wall: walls::DEFAULT_MINIMUM_MM,
            ask_dimension_value: true,
            dimension_labels: glyphs::DimensionLabels::Value,
            spline_comb: false,
            spline_knots: false,
            section_on_open: false,
            shade_regions: true,
        }
    }
}

/// Which sketch-list action the panel's picker serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SketchPickerMode {
    /// Copy one sketch's geometry into the one being edited.
    CarbonCopy,
    /// Make a new sketch of the edited one and the ticked others.
    Merge,
    /// Bring another sketch or a datum in as external geometry.
    ExternalFrom,
}

/// The panel's list of the document's other sketches, open for carbon copy
/// or merge.
#[derive(Debug, Clone)]
pub(crate) struct SketchPicker {
    pub mode: SketchPickerMode,
    /// The sketches ticked for a merge.
    pub checked: HashSet<FeatureId>,
}

/// A left press of the line tool, and what the pointer did since.
#[derive(Debug, Clone, Copy)]
struct LinePress {
    /// Where it landed, in viewport pixels.
    at: (f32, f32),
    /// The pointer has moved far enough to be a drag.
    dragged: bool,
    /// The drag draws an arc tangent to what ends where the line starts.
    arcing: bool,
}

/// How far, in pixels, a press moves before it is a drag.
const LINE_DRAG_PX: f32 = 5.0;

/// Sketch workbench: 2D drawing with constraints.
#[derive(Default)]
pub struct SketchWorkbench {
    /// Reference pictures decoded so far, by asset.
    image_cache: images::Cache,
    /// A left press of the line tool, held to tell a click from a drag:
    /// dragged from the end it draws from, it draws an arc.
    line_press: Option<LinePress>,
    /// The line tool's next click is its arc (`step::line_arc_click`).
    arc_click: bool,
    /// The shaded regions of the sketch being edited, by the profile they
    /// were meshed from.
    regions: std::cell::RefCell<Option<(u64, kernel_api::TriMesh)>>,
    /// The panel's list of other sketches, when carbon copy or merge is
    /// picking.
    /// The first key of each of the workbench's tools and actions, as the
    /// user set them, for the hints that name a key.
    action_keys: HashMap<String, String>,
    sketch_picker: Option<SketchPicker>,
    /// The external geometry tool takes picked faces and adds where they
    /// cross the sketch plane (its intersection variant), not picked edges.
    external_intersection: bool,
    /// The picked edges the external geometry tool has taken, so a pick is
    /// taken once.
    external_seen: HashSet<(Uuid, [u32; 3])>,
    /// The sketch whose external geometry was last brought up to its
    /// solids, once per editing session.
    external_refreshed: Option<FeatureId>,
    /// The panel's and the Preferences page's switches.
    pub options: SketchOptions,
    /// Geometry cut or copied, as a sketch of its own, until pasted.
    clipboard: Option<Sketch>,
    /// Currently active sketch feature ID (if any).
    active_sketch_id: Option<FeatureId>,
    /// The sketch as the editing session found it, for Cancel.
    session_start: Option<serde_json::Value>,
    /// Waiting for a plane choice before creating a sketch.
    pending_creation: Option<PendingCreation>,
    /// Point being dragged (select mode).
    dragging: Option<DragState>,
    /// Box selection in progress (select mode).
    box_select: Option<BoxSelect>,
    /// While the trim tool's button is held: where the pointer was last,
    /// the start of the path whose crossings trim next.
    trim_stroke: Option<Vec2D>,
    /// Viewport position of a right press the camera is free to pan with.
    right_press: Option<(f32, f32)>,
    /// In-progress drawing-tool state.
    tool_state: ToolState,
    /// Selected geometry ids (select mode; click toggles).
    selected: selection::Selection,
    /// Geometry under the cursor (select mode).
    hovered: Option<Uuid>,
    /// Cursor position in sketch coordinates (for previews), updated on
    /// mouse move while the cursor projects onto the sketch plane.
    cursor: Option<Vec2D>,
    /// Panel-editable tool parameters (polygon sides, slot width, fillet
    /// radius).
    tool_params: ToolParams,
    /// What the text tool lays out at its next click.
    text_draft: text::TextSpec,
    /// Most recent sketch tool seen in `on_input`; used by the task panel to
    /// show the matching tool settings.
    last_tool: Option<String>,
    /// The tool id as it was activated, variant and all: a change of it is
    /// a change of tool.
    activated_tool: Option<String>,
    /// The document's edit count after the sketcher last wrote the sketch:
    /// when it has moved on without a write of ours (an undo, a script), the
    /// cached solve verdict and the selection are checked again.
    own_seq: std::sync::atomic::AtomicU64,
    /// While on, every newly created element (from any drawing tool) is
    /// flagged as construction geometry. Toggled by the
    /// `sketch.construction` action when nothing is selected.
    construction_mode: bool,
    /// Last solver outcome, surfaced in the panel.
    last_solve: Option<SolveOutcome>,
    /// Cached constraint diagnosis (recomputed lazily after every solve).
    last_diagnosis: Option<Diagnosis>,
    /// Constraint whose value field should grab keyboard focus (set right
    /// after a dimensional constraint is added from the panel).
    pending_focus: Option<Uuid>,
    /// Elements-panel filter.
    element_filter: ElementFilter,
    /// The current `hovered` came from the elements panel (cleared when the
    /// panel stops hovering, so viewport hover can take over again).
    hover_from_panel: bool,
    /// Selected constraint ids (glyph click; ctrl additive).
    selected_constraints: HashSet<Uuid>,
    /// Dimension label being dragged (select mode).
    label_drag: Option<LabelDrag>,
    /// Pending in-viewport dimension edit.
    dim_edit: Option<DimEdit>,
    /// Last glyph press, for double-click detection.
    last_glyph_click: Option<(Uuid, Instant)>,
    /// On-view parameter capture for the active drawing tool.
    dim_capture: DimCapture,
    /// Object snapping is off (the toolbar toggle).
    snap_off: bool,
    /// The selection sorted by kind, refreshed each frame for tool
    /// enablement.
    selection_shape: constrain::SelectionShape,
    /// The selection names a curve that takes internal geometry, refreshed
    /// each frame for tool enablement.
    internal_target: bool,
    /// The copy tool is armed: transforms leave the originals in place.
    copy_mode: bool,
    /// Constraint whose name is being edited inline in the task panel.
    renaming_constraint: Option<Uuid>,
    /// Substring filter over the constraint list.
    constraint_filter: String,
    /// Which kind of constraint the list shows.
    constraint_kind_filter: ConstraintFilter,
    /// The glyphs drawn are the parked constraints', not the others'.
    parked_layer: bool,
    /// The edited sketch cuts the scene at its plane, refreshed each frame
    /// for the toolbar.
    section_view: bool,
    /// The shape being drawn, until it is recorded.
    draw_record: Option<DrawRecord>,
    /// The last wall thickness check, drawn until the sketch changes, the
    /// session ends, Escape or another tool.
    wall_check: Option<walls::WallCheck>,
}

/// A flag each constraint carries, as the toolbar flips it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConstraintFlag {
    Driving,
    Active,
    Parked,
}

/// The solver's verdict on the edited sketch, as the panel, HUD and status
/// bar show it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SolverVerdict {
    pub kind: SolverKind,
    pub title: String,
    pub body: String,
    pub dof: i32,
    /// Constraints whose geometry a click on the message selects.
    pub offenders: Vec<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SolverKind {
    Empty,
    Unanalyzed,
    Conflicting,
    Redundant,
    Fully,
    Under,
}

impl SolverKind {
    pub(crate) fn color(self, pal: &SketchPalette) -> [f32; 3] {
        match self {
            SolverKind::Fully => pal.fully_constrained,
            SolverKind::Conflicting => pal.trim,
            SolverKind::Redundant => pal.constraint,
            SolverKind::Empty | SolverKind::Unanalyzed | SolverKind::Under => pal.reference,
        }
    }
}

/// Tool ids, labels and icons of the geometry row, in toolbar order.
const GEOMETRY_TOOLS: &[(&str, &str, &str)] = &[
    ("sketch.select", "Select", "select"),
    ("sketch.point", "Point", "point"),
    ("sketch.line", "Line", "line"),
    ("sketch.arc", "Arc", "arc"),
    ("sketch.circle", "Circle", "circle"),
    ("sketch.ellipse", "Ellipse", "ellipse"),
    ("sketch.bspline", "B-spline", "bspline"),
    ("sketch.rect", "Rectangle", "rectangle"),
    ("sketch.polygon", "Regular polygon", "regular-polygon"),
    ("sketch.slot", "Slot", "slot"),
    ("sketch.text", "Text", "sketch-text"),
    ("sketch.fillet", "Fillet", "sketch-fillet"),
    ("sketch.trim", "Trim", "trim"),
    ("sketch.extend", "Extend", "extend"),
    ("sketch.split", "Split", "split"),
    ("sketch.copy", "Copy", "copy-geometry"),
    ("sketch.translate", "Move", "move-geometry"),
    ("sketch.rotate", "Rotate", "rotate-geometry"),
    ("sketch.scale", "Scale", "scale-geometry"),
    ("sketch.offset", "Offset", "offset-geometry"),
    ("sketch.mirror", "Symmetry", "symmetry-geometry"),
];

/// The most curves one pointer move of a trim stroke trims.
const MAX_STROKE_TRIMS: usize = 64;

/// The polyline's switch between a straight and a tangent-arc segment.
const POLYLINE_ARC_ACTION: &str = "sketch.polyline_arc";

/// Default keys of the tools; the user can rebind them in Preferences.
/// A plain letter draws or edits, Shift and a letter constrains.
const TOOL_KEYS: &[(&str, &str)] = &[
    ("sketch.point", "O"),
    ("sketch.line", "L"),
    ("sketch.polyline", "P"),
    ("sketch.arc", "A"),
    ("sketch.circle", "C"),
    ("sketch.ellipse", "E"),
    ("sketch.bspline", "B"),
    ("sketch.rect", "R"),
    ("sketch.polygon", "G"),
    ("sketch.slot", "S"),
    ("sketch.trim", "T"),
    ("sketch.external", "X"),
    ("sketch.construction", "N"),
    ("sketch.internal_geometry", "I"),
    ("sketch.constrain.coincident", "Shift+C"),
    ("sketch.constrain.point_on_object", "Shift+O"),
    ("sketch.constrain.vertical", "Shift+V"),
    ("sketch.constrain.horizontal", "Shift+H"),
    ("sketch.constrain.parallel", "Shift+P"),
    ("sketch.constrain.perpendicular", "Shift+N"),
    ("sketch.constrain.tangent", "Shift+T"),
    ("sketch.constrain.equal", "Shift+E"),
    ("sketch.constrain.symmetric", "Shift+S"),
    ("sketch.constrain.block", "Shift+B"),
    ("sketch.constrain.dimension", "Shift+D"),
    ("sketch.constrain.lock", "Shift+K"),
    ("sketch.constrain.distance_x", "Shift+L"),
    ("sketch.constrain.distance_y", "Shift+I"),
    ("sketch.constrain.radius", "Shift+R"),
    ("sketch.constrain.angle", "Shift+A"),
];

/// The default key of a tool, if it has one.
fn tool_key(id: &str) -> Option<&'static str> {
    TOOL_KEYS.iter().find(|(t, _)| *t == id).map(|(_, k)| *k)
}

/// `tool` with its default key, if it has one.
fn keyed(tool: ToolDescriptor) -> ToolDescriptor {
    match tool_key(&tool.id) {
        Some(key) => tool.shortcut(key),
        None => tool,
    }
}

/// The tools that act on the selection, whose recording names it.
const SELECTION_TOOLS: &[&str] = &[
    "sketch.offset",
    "sketch.translate",
    "sketch.rotate",
    "sketch.scale",
    "sketch.mirror",
];

/// Ids as a JSON list of strings.
fn ids_json(ids: &[Uuid]) -> serde_json::Value {
    serde_json::Value::Array(
        ids.iter()
            .map(|id| serde_json::json!(id.to_string()))
            .collect(),
    )
}

/// The points a drag of `id` moves: the point itself, or every point the
/// curve is pinned to. Moving them all translates the element, and
/// anything sharing those points comes with it.
pub(crate) fn drag_point_ids(sketch: &Sketch, id: Uuid) -> Vec<Uuid> {
    match sketch.get_geometry(id) {
        Some(sketch::GeometryElement::Point(p)) => vec![p.id],
        Some(other) => Sketch::curve_point_ids(other),
        None => Vec::new(),
    }
}

/// The icon of a canonical tool id, for the viewport hint.
fn tool_icon(tool: &str) -> &'static str {
    match tool {
        "sketch.external" => "external-geometry",
        "sketch.arc3" => "arc-3pt",
        "sketch.circle3" => "circle-3pt",
        "sketch.ellipse3" => "ellipse-3pt",
        "sketch.polyline" => "polyline",
        "sketch.ellipse_arc" => "arc-of-ellipse",
        "sketch.parabola" => "arc-of-parabola",
        "sketch.hyperbola" => "arc-of-hyperbola",
        "sketch.rect_center" | "sketch.rect_center3" => "rectangle-centered",
        "sketch.rect3" => "rectangle",
        "sketch.rect_frame" => "offset-geometry",
        "sketch.arc_slot" => "arc-slot",
        "sketch.chamfer" => "sketch-chamfer",
        _ => GEOMETRY_TOOLS
            .iter()
            .find(|(id, _, _)| *id == tool)
            .map(|(_, _, icon)| *icon)
            .unwrap_or("sketch"),
    }
}

/// The name and first prompt of a tool with no shape in progress.
fn idle_hint(tool: &str) -> (&'static str, &'static str) {
    match tool {
        "sketch.point" => ("Point", "Click to place a point"),
        "sketch.line" => ("Line", "Click the start point"),
        "sketch.polyline" => ("Polyline", "Click the start point"),
        "sketch.arc" => ("Arc", "Click the center"),
        "sketch.arc3" => ("Arc", "Click the first endpoint"),
        "sketch.circle" => ("Circle", "Click the center"),
        "sketch.circle3" => ("Circle", "Click a first rim point"),
        "sketch.ellipse" => ("Ellipse", "Click the center"),
        "sketch.ellipse3" => ("Ellipse", "Click one end of the major axis"),
        "sketch.ellipse_arc" => ("Arc of ellipse", "Click the center"),
        "sketch.parabola" => ("Arc of parabola", "Click the vertex"),
        "sketch.hyperbola" => ("Arc of hyperbola", "Click the center"),
        "sketch.bspline" => ("B-spline", "Click its first point"),
        "sketch.rect" => ("Rectangle", "Click the first corner"),
        "sketch.rect_center" | "sketch.rect_center3" => ("Rectangle", "Click the center"),
        "sketch.rect3" => ("Rectangle", "Click the first corner"),
        "sketch.rect_frame" => ("Frame", "Click the first corner"),
        "sketch.polygon" => ("Polygon", "Click the center"),
        "sketch.slot" => ("Slot", "Click the centerline start"),
        "sketch.arc_slot" => ("Arc slot", "Click the arc center"),
        "sketch.text" => ("Text", "Click where the text starts"),
        "sketch.fillet" => ("Fillet", "Click a corner, or two curves"),
        "sketch.chamfer" => ("Chamfer", "Click a corner, or two curves"),
        "sketch.trim" => ("Trim", "Click the span to remove, or drag across spans"),
        "sketch.external" => (
            "External geometry",
            "Click edges of a solid to bring them in, or a face for every edge around it; \
             Ctrl picks more",
        ),
        "sketch.extend" => ("Extend", "Click the end to extend"),
        "sketch.split" => ("Split", "Click where to split"),
        "sketch.offset" => ("Offset", "Click the curve to offset"),
        "sketch.translate" => ("Move", "Click the base point"),
        "sketch.rotate" => ("Rotate", "Click the pivot"),
        "sketch.scale" => ("Scale", "Click the base point"),
        "sketch.mirror" => ("Symmetry", "Click a mirror line or its first point"),
        _ => ("Select", "Click to select · drag for a box"),
    }
}

impl SketchWorkbench {
    /// The dimensions of the edited sketch that a formula sets.
    fn bound_dimensions(&self, ctx: &WorkbenchRuntimeContext) -> HashSet<Uuid> {
        self.active_sketch_id
            .and_then(|id| ctx.document.get_feature_meta(id))
            .map(|node| {
                node.formulas
                    .keys()
                    .filter_map(|k| Uuid::parse_str(k).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The active sketch, with its plane where the scene has it: a sketch
    /// keeps its plane in its body's frame, and editing works where the
    /// body sits.
    fn get_active_sketch(&self, ctx: &WorkbenchRuntimeContext) -> Option<SketchFeature> {
        let id = self.active_sketch_id?;
        let mut feature = stored_sketch(ctx.document, id)?;
        let placement = sketch_placement(ctx.document, id);
        feature.plane = placed_plane(&feature.plane, &placement);
        feature.sketch.plane = placed_plane(&feature.sketch.plane, &placement);
        if let Some(LabelDrag {
            constraint,
            current: Some(offset),
            ..
        }) = &self.label_drag
            && let Some(c) = feature
                .sketch
                .constraints
                .iter_mut()
                .find(|c| c.id == *constraint)
        {
            c.label_offset = Some(*offset);
        }
        Some(feature)
    }

    /// Persist a modified sketch feature back into the document and mark it
    /// dirty for recompute. Its plane goes back into the body's frame; a
    /// plane the edit did not move keeps its stored value exactly, so
    /// repeated edits never drift it.
    fn store_sketch(&self, ctx: &mut WorkbenchRuntimeContext, feature: SketchFeature) -> bool {
        let stored = self.store_sketch_data(ctx, feature);
        if stored && let Some(id) = self.active_sketch_id {
            ctx.document.mark_feature_dirty(id);
        }
        stored
    }

    /// [`Self::store_sketch`] for a change nothing is built from (where a
    /// dimension's label sits): the solids that use the sketch stay as
    /// they are.
    fn store_sketch_data(
        &self,
        ctx: &mut WorkbenchRuntimeContext,
        mut feature: SketchFeature,
    ) -> bool {
        let Some(id) = self.active_sketch_id else {
            return false;
        };
        let placement = sketch_placement(ctx.document, id);
        if let Some(stored) = stored_sketch(ctx.document, id) {
            feature.plane = local_plane(&feature.plane, &stored.plane, &placement);
            feature.sketch.plane =
                local_plane(&feature.sketch.plane, &stored.sketch.plane, &placement);
        }
        if let Err(e) = ctx.document.update_feature_data(id, feature.to_json()) {
            ctx.log_error(format!("Failed to update sketch: {e}"));
            return false;
        }
        self.own_seq.store(
            ctx.document.mutation_seq(),
            std::sync::atomic::Ordering::Relaxed,
        );
        true
    }

    /// Run the constraint solver on `feature`, record the outcome, and log
    /// failures. Returns the (possibly adjusted) feature.
    fn solve(&mut self, ctx: &mut WorkbenchRuntimeContext, feature: &mut SketchFeature) {
        // An edit may have taken selected items away: they leave the
        // selection with it, so Delete and the tools act on what is there.
        self.selected
            .retain(|id| feature.sketch.get_geometry(*id).is_some());
        self.selected_constraints
            .retain(|id| feature.sketch.constraints.iter().any(|c| c.id == *id));
        if !self.options.auto_update {
            // The panel's Solve now button runs it.
            self.last_diagnosis = None;
            return;
        }
        self.solve_now(ctx, feature);
    }

    /// Run the solver whatever the auto-update switch says.
    fn solve_now(&mut self, ctx: &mut WorkbenchRuntimeContext, feature: &mut SketchFeature) {
        let outcome = solver::solve(&mut feature.sketch);
        self.last_solve = Some(outcome);
        self.last_diagnosis = None; // recomputed lazily by the panel
        if let SolveOutcome::NotConverged { residual } = outcome {
            ctx.log_warn(format!(
                "Constraints did not converge (residual {residual:.2e}); check for contradictions"
            ));
        }
    }

    /// Press on `id`: it joins the selection when it is new, and a drag is
    /// armed: of the whole selection when the press landed inside it, of
    /// this element alone otherwise. Selection inside a sketch accumulates,
    /// so no modifier is needed; clicking empty space clears it.
    fn begin_drag(&mut self, sketch: &Sketch, id: Uuid, cursor: Vec2D) {
        let was_selected = self.selected.contains(&id);
        if !was_selected {
            self.selected.insert(id);
        }
        let moving: Vec<Uuid> = if was_selected {
            self.selected.iter().copied().collect()
        } else {
            vec![id]
        };
        let points = step::drag_targets(sketch, &moving);
        self.dragging = Some(DragState {
            points,
            grab: cursor,
            hit: id,
            was_selected,
            moved: false,
            elements: moving,
            delta: Vec2D::new(0.0, 0.0),
        });
    }

    fn clear_interaction_state(&mut self) {
        self.tool_state = ToolState::Idle;
        self.selected.clear();
        self.hovered = None;
        self.hover_from_panel = false;
        self.cursor = None;
        self.dragging = None;
        self.box_select = None;
        self.trim_stroke = None;
        self.right_press = None;
        self.last_diagnosis = None;
        self.pending_focus = None;
        self.selected_constraints.clear();
        self.label_drag = None;
        self.dim_edit = None;
        self.last_glyph_click = None;
        self.dim_capture = DimCapture::default();
    }

    fn sync_active_sketch_from_ctx(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if let Some(feature_id) = ctx.active_document_object
            && self.is_sketch_feature(ctx, feature_id)
            && self.active_sketch_id != Some(feature_id)
        {
            self.active_sketch_id = Some(feature_id);
            self.session_start = ctx.document.get_feature_data(feature_id).cloned();
            self.clear_interaction_state();

            // A generated sketch is centred on where its generator stands.
            if let Some(sketch_feature) = self.get_active_sketch(ctx) {
                let plane = sketch_feature.plane;
                let centre = sketch_feature.generator.is_some().then_some(plane.origin);
                ctx.request(HostRequest::OrientCamera(
                    core_document::CameraOrientRequest {
                        plane_origin: plane.origin,
                        plane_normal: plane.normal,
                        plane_up: plane.y_axis,
                        centre,
                    },
                ));
            }
        }
    }

    fn is_sketch_feature(&self, ctx: &WorkbenchRuntimeContext, feature_id: FeatureId) -> bool {
        ctx.document
            .get_feature_meta(feature_id)
            .map(|meta| meta.workbench_id.as_str() == "wb.sketch")
            .unwrap_or(false)
    }

    /// `base` when no feature has that name, else `base_n` one past the
    /// highest `n` in use: a generated sketch's name.
    pub(crate) fn next_generated_name(document: &core_document::Document, base: &str) -> String {
        let mut highest = None::<u32>;
        for (_, node) in document.feature_tree().all_nodes() {
            let n = if node.name == base {
                Some(0)
            } else {
                node.name
                    .strip_prefix(base)
                    .and_then(|rest| rest.strip_prefix('_'))
                    .and_then(|n| n.parse::<u32>().ok())
            };
            highest = highest.max(n);
        }
        match highest {
            None => base.to_string(),
            Some(n) => format!("{base}_{}", n + 1),
        }
    }

    pub(crate) fn next_sketch_name(document: &core_document::Document) -> String {
        let mut max_index = None::<u32>;
        for (_, node) in document.feature_tree().all_nodes() {
            if node.workbench_id.as_str() == "wb.sketch"
                && let Some(idx) = parse_sketch_index(&node.name)
            {
                max_index = Some(max_index.map_or(idx, |m| m.max(idx)));
            }
        }
        match max_index {
            None => "sketch".to_string(),
            Some(m) => format!("sketch_{}", m.saturating_add(1)),
        }
    }

    /// Project a viewport-local cursor position onto the sketch plane and
    /// express it in sketch coordinates.
    fn cursor_to_sketch(
        ctx: &WorkbenchRuntimeContext,
        plane: &SketchPlane,
        viewport_pos: (f32, f32),
    ) -> Option<Vec2D> {
        let world = ctx.viewport_to_plane(viewport_pos, plane.origin, plane.normal)?;
        let origin = glam::Vec3::from_array(plane.origin);
        let rel = glam::Vec3::from_array(world) - origin;
        Some(Vec2D::new(
            rel.dot(glam::Vec3::from_array(plane.x_axis)),
            rel.dot(glam::Vec3::from_array(plane.y_axis)),
        ))
    }

    /// Where a click of `tool` at `cursor` goes before its snap, and the
    /// snap's reach (off for drawing tools when snapping is switched off;
    /// modify tools keep their pick tolerance): a point within reach as it
    /// is, anything else rounded to the grid first. The click and the cue before it
    /// both come through here, so they cannot disagree.
    fn landing(
        &self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        tool: &str,
        cursor: Vec2D,
    ) -> (Vec2D, f32) {
        let plane = feature.plane;
        let tol = if self.snap_off && is_draw_tool(tool) {
            0.0
        } else {
            Self::snap_tolerance(ctx, &plane)
        };
        if is_transform_tool(tool) {
            // A base, target or centre lands on a point within reach as
            // the tool takes it, anywhere else on the grid.
            let on_point = matches!(
                crate::snap::snap_to_point(&feature.sketch, cursor, tol, &[]),
                crate::snap::SnapTarget::Existing(_)
            );
            return if on_point {
                (cursor, tol)
            } else {
                (self.grid_snapped(ctx, &plane, cursor), tol)
            };
        }
        if !is_draw_tool(tool) {
            return (cursor, tol);
        }
        // With Shift held, a segment leaves its start in steps of 15
        // degrees, as far as the cursor reaches along that direction.
        if ctx.shift_down
            && let Some(from) = tools::snap_context(&self.tool_state, &feature.sketch).from
        {
            return (snap::angle_step(from, cursor, ANGLE_STEP_DEG), tol);
        }
        // A snap onto a point wins over the grid; one onto a curve, an axis
        // or an alignment leaves a direction free for the grid to round.
        use crate::snap::SnapKind;
        let on_point = matches!(
            tools::snap_at(&self.tool_state, &feature.sketch, cursor, tol).kind,
            Some(
                SnapKind::Endpoint
                    | SnapKind::Center
                    | SnapKind::Origin
                    | SnapKind::Intersection
                    | SnapKind::Midpoint
            )
        );
        if on_point {
            (cursor, tol)
        } else {
            (self.grid_snapped(ctx, &plane, cursor), tol)
        }
    }

    /// Pixel tolerance converted into sketch units at the current zoom.
    fn snap_tolerance(ctx: &WorkbenchRuntimeContext, plane: &SketchPlane) -> f32 {
        let proj = SketchProjector::new(ctx, *plane);
        SNAP_TOLERANCE_PX * proj.units_per_px()
    }

    /// Another workbench (or the host) asked for a sketch on a specific
    /// body: take the request and open the plane picker, the face it
    /// offers in the body's own frame.
    fn take_attach_request(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if let Some(request) = ctx.attach_request.take() {
            let face = request
                .face
                .map(|f| f.moved(&ctx.document.body_placement(BodyId(request.body)).inverse()));
            self.begin_sketch_creation(Some(BodyId(request.body)), face, request.face_origin);
            if let Some(pending) = &mut self.pending_creation {
                pending.generator = request.generator.and_then(generator::Generator::named);
            }
        }
    }

    /// Open the plane picker. The sketch is created once a plane is chosen
    /// in the task panel.
    fn begin_sketch_creation(
        &mut self,
        body: Option<BodyId>,
        face: Option<core_document::FaceRef>,
        face_origin: core_document::FaceOrigin,
    ) {
        self.pending_creation = Some(PendingCreation {
            body,
            face,
            face_origin,
            generator: None,
        });
    }

    /// Close the plane picker with a sketch on the origin plane `plane`,
    /// chosen by its button or clicked in the view.
    fn create_on_base_plane(&mut self, ctx: &mut WorkbenchRuntimeContext, plane: BasePlane) {
        let Some(pending) = self.pending_creation.take() else {
            return;
        };
        self.create_sketch_on_plane(
            ctx,
            pending.body,
            SketchPlane::of_base(plane),
            NewSketchOn {
                made_by: pending.generator,
                ..Default::default()
            },
        );
    }

    fn create_sketch_on_plane(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        body: Option<BodyId>,
        plane: SketchPlane,
        on: NewSketchOn,
    ) {
        let NewSketchOn {
            support,
            face,
            attached,
            made_by,
        } = on;
        let sketch_name = match &made_by {
            Some(made_by) => Self::next_generated_name(ctx.document, made_by.base_name()),
            None => Self::next_sketch_name(ctx.document),
        };
        let mut sketch = Sketch::new(sketch_name.clone());
        sketch.plane = plane;
        let mut sketch_feature = SketchFeature::new(sketch, plane);
        sketch_feature.support = support.clone();
        sketch_feature.face = face;
        sketch_feature.attached = attached;
        let kind = made_by.as_ref().map(generator::Generator::kind);
        if let Some(made_by) = made_by {
            sketch_feature.generator = Some(made_by);
            if let Err(why) = generator::regenerate(&mut sketch_feature) {
                ctx.log_error(format!("Failed to make {sketch_name}: {why}"));
                return;
            }
        }

        match ctx
            .document
            .add_feature_in_body(sketch_feature, sketch_name.clone(), body)
        {
            Ok(feature_id) => {
                let mut args = match &support {
                    Some(support) => {
                        let mut args = serde_json::json!({
                            "name": sketch_name,
                            "on": support.datum.0.to_string(),
                        });
                        if let Some(which) = &support.plane {
                            args["plane"] = serde_json::json!(which);
                        }
                        args
                    }
                    None => serde_json::json!({
                        "name": sketch_name,
                        "normal": plane.normal,
                        "origin": plane.origin,
                        "x_axis": plane.x_axis,
                    }),
                };
                if let Some(body) = body {
                    args["body"] = serde_json::json!(body.0.to_string());
                }
                if let Some(kind) = kind {
                    args["generator"] = serde_json::json!(kind);
                }
                if let Some(attached) = attached {
                    args["attachment"] = serde_json::json!(attached.attachment);
                    args["attachment_offset"] = serde_json::json!(attached.offset);
                }
                ctx.record(
                    "sketch.new",
                    commands::args(args),
                    serde_json::json!(feature_id.0.to_string()),
                );
                self.active_sketch_id = Some(feature_id);
                self.session_start = ctx.document.get_feature_data(feature_id).cloned();
                self.clear_interaction_state();
                ctx.active_document_object = Some(feature_id);
                // The view turns to the plane where the body has it, onto
                // the face the sketch was placed on when it was.
                // A generated sketch is centred on its origin, where the
                // generator stands.
                let placement = sketch_placement(ctx.document, feature_id);
                let centre = match kind {
                    Some(_) => Some(placement.point(plane.origin)),
                    None => face.as_ref().map(|f| placement.point(f.point)),
                };
                let plane = placed_plane(&plane, &placement);
                ctx.request(HostRequest::OrientCamera(
                    core_document::CameraOrientRequest {
                        plane_origin: plane.origin,
                        plane_normal: plane.normal,
                        plane_up: plane.y_axis,
                        centre,
                    },
                ));
                ctx.log_info(format!("Created new sketch: {sketch_name}"));
            }
            Err(e) => {
                ctx.log_error(format!("Failed to create sketch: {e}"));
            }
        }
    }

    /// Run the tool at `cursor` (typed dimensions override it). `constrain`
    /// turns the typed values into constraints as well; a plain click keeps
    /// the geometry free.
    fn apply_tool_click(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: &str,
        cursor: Vec2D,
        constrain: bool,
    ) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let (cursor, tol) = self.landing(ctx, &feature, tool, cursor);
        if tool == "sketch.text" {
            let spec = self.text_draft.clone();
            return self.sketch_edit(
                ctx,
                "sketch.text",
                text_args(&spec, Some(cursor)),
                move |s| match text::add(s, cursor, &spec) {
                    Ok(_) => tools::ToolEffect::changed(format!("Text \"{}\"", spec.text)),
                    Err(why) => tools::ToolEffect::log(why),
                },
            );
        }
        // The copy tool is the move tool with at least one copy.
        let params = ToolParams {
            copies: if self.copy_mode {
                self.tool_params.copies.max(1)
            } else {
                self.tool_params.copies
            },
            auto_constraints: self.options.auto_constraints,
            ..self.tool_params
        };
        self.dim_capture.sync(&self.tool_state);
        let typed = self.dim_capture.typed();
        let settings = step::StepSettings {
            tol,
            params,
            construction: self.construction_mode,
            avoid_redundant: self.options.avoid_redundant_auto,
        };
        self.note_draw_click(ctx, &feature, tool, cursor, &typed, constrain, &settings);
        let arc = tool == "sketch.line" && std::mem::take(&mut self.arc_click);
        let outcome = if arc {
            // Recorded as the arc it is.
            if let Some(event) = self.draw_record.as_mut().and_then(|r| r.events.last_mut()) {
                *event = serde_json::json!({"x": cursor.x, "y": cursor.y, "arc": true});
            }
            step::line_arc_click(
                &mut self.tool_state,
                &mut self.dim_capture,
                &mut feature.sketch,
                cursor,
                &settings,
                &self.selected,
            )
        } else {
            step::click(
                &mut self.tool_state,
                &mut self.dim_capture,
                tool,
                &mut feature.sketch,
                cursor,
                &typed,
                constrain,
                &settings,
                &self.selected,
            )
        };
        if outcome.skipped > 0 {
            ctx.log_info(format!(
                "Skipped {} redundant auto constraint(s)",
                outcome.skipped
            ));
        }
        let (effect, added) = (
            tools::ToolEffect {
                changed: outcome.changed,
                log: outcome.log,
            },
            outcome.added,
        );
        if (effect.changed || added > 0)
            && let Some(record) = self.draw_record.as_mut()
        {
            record.changed = true;
        }
        if effect.changed {
            self.dim_capture.clear_buffers();
        }
        if effect.changed || added > 0 {
            self.solve(ctx, &mut feature);
            if let Some(log) = effect.log {
                ctx.log_info(log);
            }
            self.store_sketch(ctx, feature);
        } else if let Some(why) = effect.log {
            // A click that did nothing says why.
            ctx.log_warn(why);
        }
        InputResult::consumed()
    }

    /// Note a click for the recording: the start of a new `sketch.draw`
    /// call when the tool starts a shape, else one more of its points.
    #[allow(clippy::too_many_arguments)]
    fn note_draw_click(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        feature: &SketchFeature,
        tool: &str,
        cursor: Vec2D,
        typed: &[(ovp::FieldKind, f32)],
        constrain: bool,
        settings: &step::StepSettings,
    ) {
        let Some(sketch_id) = self.active_sketch_id else {
            return;
        };
        // A trim stroke's clicks are one call, as a shape's are.
        let fresh = match &self.draw_record {
            Some(r) => {
                r.tool != tool
                    || r.sketch != sketch_id
                    || (self.tool_state.is_idle() && self.trim_stroke.is_none())
            }
            None => true,
        };
        if fresh {
            self.flush_draw_record(ctx);
            let mut named = serde_json::Map::new();
            named.insert("tolerance".into(), serde_json::json!(settings.tol));
            let params = step::params_to_json(&settings.params);
            if !params.is_empty() {
                named.insert("params".into(), serde_json::Value::Object(params));
            }
            if settings.construction {
                named.insert("construction".into(), serde_json::json!(true));
            }
            if !settings.avoid_redundant {
                named.insert("avoid_redundant".into(), serde_json::json!(false));
            }
            if SELECTION_TOOLS.contains(&tool) && !self.selected.is_empty() {
                let mut selected: Vec<Uuid> = self.selected.iter().copied().collect();
                selected.sort();
                named.insert("selection".into(), ids_json(&selected));
            }
            self.draw_record = Some(DrawRecord {
                sketch: sketch_id,
                tool: tool.to_string(),
                events: Vec::new(),
                settings: named,
                changed: false,
                elements_before: feature.sketch.geometry.iter().map(|g| g.id()).collect(),
                constraints_before: feature.sketch.constraints.iter().map(|c| c.id).collect(),
            });
        }
        let event = if typed.is_empty() {
            serde_json::json!([cursor.x, cursor.y])
        } else {
            let values: serde_json::Map<String, serde_json::Value> = typed
                .iter()
                .map(|(k, v)| (step::field_name(*k).to_string(), serde_json::json!(v)))
                .collect();
            serde_json::json!({"x": cursor.x, "y": cursor.y, "typed": values, "constrain": constrain})
        };
        if let Some(record) = self.draw_record.as_mut() {
            record.events.push(event);
        }
    }

    /// Record the shape drawn so far as one `sketch.draw` call, with what
    /// it made, when it made anything.
    fn flush_draw_record(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(record) = self.draw_record.take() else {
            return;
        };
        if !record.changed {
            return;
        }
        let Some(feature) = stored_sketch(ctx.document, record.sketch) else {
            return;
        };
        let elements: Vec<Uuid> = feature
            .sketch
            .geometry
            .iter()
            .map(|g| g.id())
            .filter(|id| !record.elements_before.contains(id))
            .collect();
        let constraints: Vec<Uuid> = feature
            .sketch
            .constraints
            .iter()
            .map(|c| c.id)
            .filter(|id| !record.constraints_before.contains(id))
            .collect();
        let mut args = record.settings;
        args.insert(
            "sketch".into(),
            serde_json::json!(record.sketch.0.to_string()),
        );
        args.insert(
            "tool".into(),
            serde_json::json!(record.tool.trim_start_matches("sketch.")),
        );
        args.insert("points".into(), serde_json::Value::Array(record.events));
        ctx.record(
            "sketch.draw",
            args,
            serde_json::json!({
                "elements": ids_json(&elements),
                "constraints": ids_json(&constraints),
            }),
        );
    }

    fn handle_left_click(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: Option<&str>,
        viewport_pos: (f32, f32),
    ) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let plane = feature.plane;
        let Some(cursor) = Self::cursor_to_sketch(ctx, &plane, viewport_pos) else {
            return InputResult::ignored();
        };
        let tol = Self::snap_tolerance(ctx, &plane);
        self.cursor = Some(cursor);
        // A new press ends whatever the last one armed.
        self.dragging = None;
        self.box_select = None;
        self.line_press = None;

        // The line tool: a press on the end it draws from, dragged, is an
        // arc tangent to what ends there; a press anywhere else clicks, and
        // dragged from a first point draws the line to where it is let go.
        if tool == Some("sketch.line") {
            let on_end = match self.tool_state {
                ToolState::LineFrom {
                    from: snap::SnapTarget::Existing(from),
                    ..
                } => feature
                    .sketch
                    .point_position(from)
                    .is_some_and(|at| (at - cursor).to_glam().length() <= tol),
                _ => false,
            };
            let press = LinePress {
                at: viewport_pos,
                dragged: false,
                arcing: false,
            };
            if on_end {
                self.line_press = Some(press);
                return InputResult::consumed();
            }
            let result = self.apply_tool_click(ctx, "sketch.line", cursor, false);
            if matches!(self.tool_state, ToolState::LineFrom { chain: false, .. }) {
                self.line_press = Some(press);
            }
            return result;
        }

        match tool {
            // A press trims what it lands on and starts a stroke: dragging
            // on trims whatever the pointer crosses.
            Some("sketch.trim") => {
                self.trim_stroke = None;
                self.flush_draw_record(ctx);
                let result = if tools::trim_preview(&feature.sketch, cursor, tol).is_some() {
                    self.apply_tool_click(ctx, "sketch.trim", cursor, false)
                } else {
                    InputResult::consumed()
                };
                self.trim_stroke = Some(cursor);
                result
            }
            Some(t) if t != "sketch.select" => self.apply_tool_click(ctx, t, cursor, false),
            _ => {
                // Select mode. Constraint glyphs sit on top of geometry, so
                // they win the hit-test. Pressing on geometry selects it and
                // arms a drag: moving carries the element (and everything
                // sharing its points) to the cursor, releasing in place
                // leaves it selected. Empty space starts a box selection
                // (resolved on release).
                if let Some(hit) = self.glyph_hit(ctx, &feature, viewport_pos) {
                    return self.handle_glyph_press(ctx, &feature, hit, cursor);
                }
                match snap::hit_test(&feature.sketch, cursor, tol) {
                    Some(id) => {
                        self.begin_drag(&feature.sketch, id, cursor);
                        InputResult::consumed()
                    }
                    None => {
                        // Empty space: begin a box selection. The release
                        // decides between a real box (which adds what it
                        // covers) and a plain click (which clears).
                        self.box_select = Some(BoxSelect {
                            anchor: cursor,
                            current: cursor,
                        });
                        InputResult::consumed()
                    }
                }
            }
        }
    }

    /// The constraint glyph under a viewport click, if any.
    fn glyph_hit(
        &self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        viewport_pos: (f32, f32),
    ) -> Option<GlyphHit> {
        // Hidden glyphs take no clicks.
        if self.options.constraints_hidden {
            return None;
        }
        let proj = SketchProjector::new(ctx, feature.plane);
        let glyphs = self.glyphs(ctx, &feature.sketch, &proj);
        glyphs::hit_test(&glyphs, [viewport_pos.0, viewport_pos.1]).map(|g| GlyphHit {
            constraint: g.constraint,
            dimensional: g.dimensional,
            pos: g.pos,
        })
    }

    /// Click on a constraint glyph: select it (ctrl additive), start a
    /// label drag on dimension labels, open the value editor on
    /// double-click.
    fn handle_glyph_press(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        feature: &SketchFeature,
        hit: GlyphHit,
        cursor: Vec2D,
    ) -> InputResult {
        let now = Instant::now();
        let double = self.last_glyph_click.take().is_some_and(|(id, t)| {
            id == hit.constraint && now.duration_since(t) < DOUBLE_CLICK_WINDOW
        });
        self.last_glyph_click = Some((hit.constraint, now));
        let constraint = feature
            .sketch
            .constraints
            .iter()
            .find(|c| c.id == hit.constraint);

        if double && hit.dimensional {
            if let Some(c) = constraint {
                // A dimension a formula sets opens on its formula.
                let formula = self.active_sketch_id.and_then(|sketch| {
                    ctx.document
                        .feature_formula(sketch, &c.id.to_string())
                        .map(str::to_string)
                });
                self.dim_edit = Some(DimEdit::new(
                    c.id,
                    hit.pos,
                    formula.unwrap_or_else(|| {
                        sketch::dimension_value(&c.kind)
                            .map(glyphs::fmt_num)
                            .unwrap_or_default()
                    }),
                    c.driving,
                ));
            }
            self.label_drag = None;
            return InputResult::consumed();
        }

        // Selection accumulates: a second click on the same glyph takes it
        // back out, and empty space clears everything.
        if !self.selected_constraints.remove(&hit.constraint) {
            self.selected_constraints.insert(hit.constraint);
        }
        if hit.dimensional {
            let proj = SketchProjector::new(ctx, feature.plane);
            let base = glyphs::default_label_offset(proj.units_per_px());
            let original = constraint.and_then(|c| c.label_offset);
            self.label_drag = Some(LabelDrag {
                constraint: hit.constraint,
                moved: false,
                grab: cursor,
                base: original.unwrap_or(base),
                current: None,
            });
        }
        InputResult::consumed()
    }

    fn handle_mouse_move(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: Option<&str>,
        viewport_pos: (f32, f32),
    ) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let plane = feature.plane;
        self.cursor = Self::cursor_to_sketch(ctx, &plane, viewport_pos);

        // A line tool press that moves far enough is a drag; from an end
        // something runs into, the drag draws a tangent arc.
        if tool == Some("sketch.line")
            && let Some(press) = self.line_press.as_mut()
            && !press.dragged
            && (viewport_pos.0 - press.at.0).hypot(viewport_pos.1 - press.at.1) > LINE_DRAG_PX
        {
            press.dragged = true;
            press.arcing = tools::line_arc_begin(&mut self.tool_state, &feature.sketch);
        }

        // Dimension label drag: purely cosmetic, no solver run needed.
        let tol = Self::snap_tolerance(ctx, &plane);
        if let Some(ld) = self.label_drag.as_mut() {
            if let Some(cursor) = self.cursor
                && (ld.moved || (cursor - ld.grab).to_glam().length() > tol)
            {
                ld.moved = true;
                ld.current = Some(ld.base + (cursor - ld.grab));
            }
            return InputResult::consumed();
        }
        // Constraint-aware drag: carry every point of the grabbed geometry
        // by the cursor delta and let the solver settle the rest: whatever
        // shares those points comes along, and the dimensions that are not
        // driving adjust. Consumed so the camera doesn't orbit underneath.
        if let Some(drag) = self.dragging.as_mut() {
            let Some(cursor) = self.cursor else {
                return InputResult::consumed();
            };
            let delta = cursor - drag.grab;
            // A dead zone the size of the snap tolerance: a click that
            // trembles selects instead of nudging the geometry.
            if !drag.moved && delta.to_glam().length() <= Self::snap_tolerance(ctx, &plane) {
                return InputResult::consumed();
            }
            drag.moved = true;
            drag.delta = delta;
            step::drag(&mut feature.sketch, &drag.points, delta);
            // The dragged points stay under the cursor and the rest gives
            // way; constraints hold during a drag whatever auto update says.
            let held: Vec<Uuid> = drag.points.iter().map(|(id, _)| *id).collect();
            self.last_solve = Some(solver::solve_holding(&mut feature.sketch, &held));
            self.last_diagnosis = None;
            self.store_sketch(ctx, feature);
            return InputResult::consumed();
        }
        // A trim stroke: every curve the pointer crossed since the last
        // move is trimmed where it was crossed, first crossed first.
        if let Some(mut from) = self.trim_stroke
            && tool == Some("sketch.trim")
        {
            let Some(to) = self.cursor else {
                return InputResult::consumed();
            };
            // Each trim can leave new pieces in the way; the bound only
            // guards against a curve that never stops being crossed.
            for _ in 0..MAX_STROKE_TRIMS {
                let Some(sketch) = self.get_active_sketch(ctx) else {
                    break;
                };
                let Some(at) = tools::next_stroke_crossing(&sketch.sketch, from, to) else {
                    break;
                };
                self.apply_tool_click(ctx, "sketch.trim", at, false);
                from = at;
            }
            self.trim_stroke = Some(to);
            return InputResult::consumed();
        }
        // Box selection in progress: track the moving corner. Consumed so
        // the camera doesn't move underneath the box.
        if let Some(bs) = self.box_select.as_mut() {
            if let Some(cursor) = self.cursor {
                bs.current = cursor;
            }
            return InputResult::consumed();
        }
        if matches!(tool, None | Some("sketch.select")) {
            let tol = Self::snap_tolerance(ctx, &plane);
            self.hovered = self
                .cursor
                .and_then(|c| snap::hit_test(&feature.sketch, c, tol));
        } else {
            self.hovered = None;
        }
        // Never consume moves: the camera still needs them for orbiting.
        InputResult::redraw_only()
    }

    fn handle_left_release(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        // A line tool drag lands: the arc, or the line, ends where it is let go.
        if let Some(press) = self.line_press.take()
            && press.dragged
            && !self.tool_state.is_idle()
            && let Some(cursor) = self.cursor
        {
            self.arc_click = press.arcing;
            return self.apply_tool_click(ctx, "sketch.line", cursor, false);
        }
        if self.trim_stroke.take().is_some() {
            return InputResult::consumed();
        }
        if let Some(ld) = &self.label_drag {
            // The label lands: one write for the whole drag.
            let landed = ld.current.is_some();
            let feature = self.get_active_sketch(ctx);
            self.label_drag = None;
            if landed && let Some(feature) = feature {
                self.store_sketch_data(ctx, feature);
            }
            return InputResult::consumed();
        }
        if let Some(bs) = self.box_select.take() {
            return self.finish_box_select(ctx, bs);
        }
        if let Some(drag) = self.dragging.take() {
            // A press+release without movement is a click. The press put a
            // new element in the selection already, so only one that was
            // there before comes back out.
            if !drag.moved && drag.was_selected {
                self.selected.remove(&drag.hit);
            }
            if drag.moved
                && let Some(sketch_id) = self.active_sketch_id
            {
                ctx.record(
                    "sketch.drag",
                    commands::args(serde_json::json!({
                        "sketch": sketch_id.0.to_string(),
                        "items": ids_json(&drag.elements),
                        "by": [drag.delta.x, drag.delta.y],
                    })),
                    serde_json::Value::Null,
                );
            }
            return InputResult::consumed();
        }
        InputResult::ignored()
    }

    /// Resolve a released box selection. A drag beyond the snap tolerance
    /// adds every element fully inside the rectangle to the selection
    /// (selection accumulates), a curve without the points that define it,
    /// so a boxed line is a line to the constraint tools; anything shorter
    /// is a click on empty space, which clears it.
    fn finish_box_select(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        bs: BoxSelect,
    ) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::consumed();
        };
        let tol = Self::snap_tolerance(ctx, &feature.plane);
        // A press and release in the same spot is a click on empty space:
        // the one gesture that clears the selection.
        if (bs.current - bs.anchor).to_glam().length() <= tol {
            self.selected.clear();
            self.selected_constraints.clear();
            return InputResult::consumed();
        }
        let min = Vec2D::new(bs.anchor.x.min(bs.current.x), bs.anchor.y.min(bs.current.y));
        let max = Vec2D::new(bs.anchor.x.max(bs.current.x), bs.anchor.y.max(bs.current.y));
        let boxed: Vec<&sketch::GeometryElement> = feature
            .sketch
            .geometry
            .iter()
            .filter(|g| element_fully_inside(&feature.sketch, g, min, max))
            .collect();
        let owned: HashSet<Uuid> = boxed
            .iter()
            .filter(|g| !matches!(g, sketch::GeometryElement::Point(_)))
            .flat_map(|g| Sketch::curve_point_ids(g))
            .collect();
        for geom in boxed {
            if !owned.contains(&geom.id()) {
                self.selected.insert(geom.id());
            }
        }
        InputResult::consumed()
    }

    /// Delete everything selected, elements and constraints alike, as one
    /// edit.
    fn delete_selected(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        if self.selected.is_empty() && self.selected_constraints.is_empty() {
            return InputResult::ignored();
        }
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let mut doomed: Vec<Uuid> = self
            .selected
            .drain()
            .chain(self.selected_constraints.drain())
            .collect();
        doomed.sort();
        let removed = commands::delete_items(&mut feature.sketch, &doomed);
        self.hovered = None;
        if removed == 0 {
            return InputResult::consumed();
        }
        if let Some(sketch_id) = self.active_sketch_id {
            ctx.record(
                "sketch.delete",
                commands::args(serde_json::json!({
                    "sketch": sketch_id.0.to_string(),
                    "items": ids_json(&doomed),
                })),
                serde_json::Value::Null,
            );
        }
        self.solve(ctx, &mut feature);
        ctx.log_info(format!("Deleted {removed} sketch item(s)"));
        self.store_sketch(ctx, feature);
        InputResult::consumed()
    }

    /// The `sketch.construction` action. With a selection: flip each
    /// selected element's construction flag individually (mixed selections
    /// end up mixed-inverted). With nothing selected: toggle construction
    /// *mode*, under which all newly drawn geometry is construction.
    fn toggle_construction_selected(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        if self.selected.is_empty() {
            self.construction_mode = !self.construction_mode;
            ctx.log_info(if self.construction_mode {
                "Construction mode ON"
            } else {
                "Construction mode OFF"
            });
            return InputResult::consumed();
        }
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        // Drawn curves are construction by their flag; projected ones are
        // guides until they count in the profile, so for them construction
        // is not counting.
        let sketch = &feature.sketch;
        let (projected, drawn): (Vec<Uuid>, Vec<Uuid>) = self
            .selected
            .iter()
            .copied()
            .filter(|id| sketch.get_geometry(*id).is_some())
            .partition(|id| sketch.external.contains_key(id));
        let is_construction = |id: &Uuid| match sketch.external.get(id) {
            Some(source) => !source.defining,
            None => sketch.is_construction(*id),
        };
        if drawn.is_empty() && projected.is_empty() {
            return InputResult::consumed();
        }
        // One way for the whole selection: construction unless all of it
        // is already, in which case back to normal.
        let on = !drawn.iter().chain(&projected).all(is_construction);
        for id in &drawn {
            feature.sketch.set_construction(*id, on);
        }
        commands::set_external_defining(&mut feature.sketch, &projected, !on);
        if let Some(sketch_id) = self.active_sketch_id {
            let mut drawn = drawn.clone();
            drawn.sort();
            if !drawn.is_empty() {
                ctx.record(
                    "sketch.construction",
                    commands::args(serde_json::json!({
                        "sketch": sketch_id.0.to_string(),
                        "items": ids_json(&drawn),
                        "on": on,
                    })),
                    serde_json::Value::Null,
                );
            }
            if !projected.is_empty() {
                ctx.record(
                    "sketch.external_defining",
                    commands::args(serde_json::json!({
                        "sketch": sketch_id.0.to_string(),
                        "items": ids_json(&projected),
                        "on": !on,
                    })),
                    serde_json::Value::Null,
                );
            }
        }
        let count = drawn.len() + projected.len();
        ctx.log_info(if on {
            format!("{count} element(s) made construction")
        } else {
            format!("{count} element(s) made normal geometry")
        });
        self.store_sketch(ctx, feature);
        InputResult::consumed()
    }

    /// The `sketch.internal_geometry` action: the selected curves' internal
    /// geometry shown when a piece of it is missing, else hidden, through
    /// the command's code.
    fn toggle_internal_geometry(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let (Some(mut feature), Some(sketch_id)) =
            (self.get_active_sketch(ctx), self.active_sketch_id)
        else {
            return InputResult::ignored();
        };
        let mut items: Vec<Uuid> = self.selected.iter().copied().collect();
        items.sort();
        if internal::curves_of(&feature.sketch, &items).is_empty() {
            ctx.log_warn("Select an ellipse, a parabola, a hyperbola or a B-spline");
            return InputResult::consumed();
        }
        let (shown, changed) = internal::toggle(&mut feature.sketch, &items, None);
        ctx.record(
            "sketch.internal_geometry",
            commands::args(serde_json::json!({
                "sketch": sketch_id.0.to_string(),
                "items": ids_json(&items),
                "show": shown,
            })),
            serde_json::json!({
                "shown": shown,
                "elements": changed.iter().map(Uuid::to_string).collect::<Vec<_>>(),
            }),
        );
        if !shown {
            self.selected.retain(|id| !changed.contains(id));
        }
        ctx.log_info(match (shown, changed.len()) {
            (true, n) => format!("Showed internal geometry: {n} element(s)"),
            (false, 0) => "Internal geometry kept: all of it is constrained".to_string(),
            (false, n) => format!("Hid internal geometry: {n} element(s)"),
        });
        if !changed.is_empty() {
            self.solve(ctx, &mut feature);
            self.store_sketch(ctx, feature);
        }
        InputResult::consumed()
    }

    /// The `sketch.section_view` action: the edited sketch's section view
    /// switched, through the command's code.
    fn toggle_section_view(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let (Some(mut feature), Some(sketch_id)) =
            (self.get_active_sketch(ctx), self.active_sketch_id)
        else {
            return InputResult::ignored();
        };
        let on = !feature.shows_section(self.options.section_on_open);
        feature.set_section(on);
        self.section_view = on;
        ctx.record(
            "sketch.section_view",
            commands::args(serde_json::json!({
                "sketch": sketch_id.0.to_string(),
                "on": on,
            })),
            serde_json::Value::Null,
        );
        ctx.log_info(if on {
            "Section view: the scene is cut at the sketch plane"
        } else {
            "Section view off"
        });
        self.store_sketch_data(ctx, feature);
        InputResult::consumed()
    }

    /// The `sketch.remove_axis_alignment` action: the selected lines'
    /// horizontal and vertical constraints turned into parallel and
    /// perpendicular ones among them, through the command's code.
    fn remove_axis_alignment(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let (Some(mut feature), Some(sketch_id)) =
            (self.get_active_sketch(ctx), self.active_sketch_id)
        else {
            return InputResult::ignored();
        };
        let mut items: Vec<Uuid> = self.selected.iter().copied().collect();
        items.sort();
        let set: HashSet<Uuid> = items.iter().copied().collect();
        let changed = tools::remove_axis_alignment(&mut feature.sketch, &set);
        if changed == 0 {
            ctx.log_warn("None of the selected lines is held horizontal or vertical");
            return InputResult::consumed();
        }
        ctx.record(
            "sketch.remove_axis_alignment",
            commands::args(serde_json::json!({
                "sketch": sketch_id.0.to_string(),
                "items": ids_json(&items),
            })),
            serde_json::json!(changed),
        );
        self.selected_constraints
            .retain(|id| feature.sketch.constraints.iter().any(|c| c.id == *id));
        ctx.log_info(format!(
            "Removed axis alignment: {changed} constraint(s) now relative"
        ));
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
        InputResult::consumed()
    }

    /// Right-click / Enter: ends a line chain, completes an in-progress
    /// B-spline; anything else stays with the camera (right-drag pans).
    fn handle_finish_gesture(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        match &self.tool_state {
            ToolState::LineFrom { chain: true, .. } | ToolState::PolylineFrom { .. } => {
                self.tool_state = ToolState::Idle;
                InputResult::consumed()
            }
            ToolState::BSplineDraw { .. } => {
                let Some(mut feature) = self.get_active_sketch(ctx) else {
                    self.tool_state = ToolState::Idle;
                    return InputResult::consumed();
                };
                let effect = tools::finish_click_sequence(
                    &mut self.tool_state,
                    &mut feature.sketch,
                    &self.tool_params,
                );
                if let Some(record) = self.draw_record.as_mut() {
                    record.events.push(serde_json::json!("finish"));
                    record.changed |= effect.changed;
                }
                if effect.changed {
                    self.solve(ctx, &mut feature);
                    if let Some(log) = effect.log {
                        ctx.log_info(log);
                    }
                    self.store_sketch(ctx, feature);
                }
                InputResult::consumed()
            }
            _ => InputResult::ignored(),
        }
    }

    /// A right click that stayed put (a pan moves the camera instead) with
    /// no gesture in flight hands the pointer back to the Select tool.
    fn handle_right_release(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        viewport_pos: (f32, f32),
    ) -> InputResult {
        /// How far the pointer may travel and still count as a click.
        const CLICK_SLOP_PX: f32 = 4.0;
        let Some(press) = self.right_press.take() else {
            return InputResult::ignored();
        };
        let travelled = (viewport_pos.0 - press.0).hypot(viewport_pos.1 - press.1);
        if travelled <= CLICK_SLOP_PX && self.tool_state.is_idle() {
            ctx.request(HostRequest::ActivateTool("sketch.select".to_string()));
        }
        // Never consumed: the camera still has a pan to finish.
        InputResult::ignored()
    }

    /// The constraint list's kind filter (the panel's dropdown; also set by
    /// integration tests).
    pub fn constraint_filter_mut(&mut self) -> &mut ConstraintFilter {
        &mut self.constraint_kind_filter
    }

    /// The constraints the panel's list shows, in sketch order, as its text
    /// and kind filters and the selection leave it.
    pub fn listed_constraints(&self, sketch: &Sketch) -> Vec<Uuid> {
        let text = self.constraint_filter.trim().to_lowercase();
        sketch
            .constraints
            .iter()
            .enumerate()
            .filter(|(idx, c)| {
                (text.is_empty() || constraint_row_label(*idx, c).to_lowercase().contains(&text))
                    && self.constraint_kind_filter.accepts(
                        sketch,
                        c,
                        &self.selected,
                        &self.selected_constraints,
                    )
            })
            .map(|(_, c)| c.id)
            .collect()
    }

    /// Panel-editable tool parameters (also used by integration tests to
    /// set copy counts, offset distances, …).
    pub fn tool_params_mut(&mut self) -> &mut ToolParams {
        &mut self.tool_params
    }

    fn handle_escape(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        if self.dim_edit.take().is_some() {
            return InputResult::consumed();
        }
        if self.label_drag.take().is_some() {
            // Nothing is written until the release: the label stays put.
            return InputResult::consumed();
        }
        if self.box_select.take().is_some() {
            // Cancel the box; the selection it would have replaced stays.
            return InputResult::consumed();
        }
        if let Some(drag) = self.dragging.take() {
            // Put every dragged point back where the press found it.
            if drag.moved
                && let Some(mut feature) = self.get_active_sketch(ctx)
            {
                for (id, original) in drag.points {
                    if let Some(sketch::GeometryElement::Point(p)) =
                        feature.sketch.get_geometry_mut(id)
                    {
                        p.position = original;
                    }
                }
                self.solve(ctx, &mut feature);
                self.store_sketch(ctx, feature);
            }
            return InputResult::consumed();
        }
        if self.pending_creation.is_some() {
            self.pending_creation = None;
            return InputResult::consumed();
        }
        if self.tool_state.is_idle() && self.wall_check.take().is_some() {
            return InputResult::consumed();
        }
        if !self.tool_state.is_idle() {
            self.tool_state = ToolState::Idle;
            ctx.log_info("Sketch: cancelled current tool operation");
        } else if !self.selected.is_empty() || !self.selected_constraints.is_empty() {
            self.selected.clear();
            self.selected_constraints.clear();
        }
        InputResult::consumed()
    }

    /// Enter with typed on-view parameters: commit the pending click at the
    /// derived position without a mouse click.
    fn commit_typed_enter(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: Option<&str>,
    ) -> InputResult {
        let Some(tool) = tool.filter(|t| *t != "sketch.select") else {
            return InputResult::ignored();
        };
        // Anchor the derived position on the last known cursor; the
        // override falls back to the +x direction when it is degenerate.
        let cursor = self.cursor.unwrap_or(Vec2D::new(0.0, 0.0));
        self.apply_tool_click(ctx, tool, cursor, true)
    }

    /// A keyboard action registered in `configure`, by its key.
    fn handle_action(&mut self, id: &str) -> InputResult {
        match id {
            POLYLINE_ARC_ACTION if tools::toggle_polyline_arc(&mut self.tool_state) => {
                if let (Some(record), ToolState::PolylineFrom { segment, .. }) =
                    (self.draw_record.as_mut(), &self.tool_state)
                {
                    record.events.push(serde_json::json!(segment.word()));
                }
                InputResult::consumed()
            }
            _ => InputResult::ignored(),
        }
    }

    /// Consolidated key handling: on-view parameter capture first (typing
    /// digits into the focused dimension field), then the global keys.
    fn handle_key_press(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: Option<&str>,
        key: KeyCode,
    ) -> InputResult {
        self.dim_capture.sync(&self.tool_state);
        if self.dim_capture.is_active() {
            match key {
                KeyCode::Enter if self.dim_capture.has_typed_input() => {
                    return self.commit_typed_enter(ctx, tool);
                }
                KeyCode::Escape if self.dim_capture.has_typed_input() => {
                    self.dim_capture.clear_buffers();
                    return InputResult::consumed();
                }
                _ => {
                    if self.dim_capture.handle_key(key) {
                        return InputResult::consumed();
                    }
                }
            }
        }
        match key {
            KeyCode::Escape => self.handle_escape(ctx),
            KeyCode::Enter => self.handle_finish_gesture(ctx),
            // A shape half drawn: the keys belong to it, not the selection.
            KeyCode::Delete | KeyCode::Backspace if !self.tool_state.is_idle() => {
                InputResult::consumed()
            }
            KeyCode::Delete | KeyCode::Backspace => self.delete_selected(ctx),
            _ => InputResult::ignored(),
        }
    }

    /// The pending in-viewport dimension edit, if one is open.
    pub fn pending_dim_edit(&self) -> Option<&DimEdit> {
        self.dim_edit.as_ref()
    }

    /// Mutable access to the pending dimension edit (text/driving fields).
    pub fn pending_dim_edit_mut(&mut self) -> Option<&mut DimEdit> {
        self.dim_edit.as_mut()
    }

    /// Close the pending dimension edit without applying it.
    pub fn cancel_dim_edit(&mut self) {
        self.dim_edit = None;
    }

    /// Commit the pending dimension edit: parse the value, update the
    /// constraint's value and driving flag, re-solve and persist.
    pub fn commit_dim_edit(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(edit) = self.dim_edit.take() else {
            return;
        };
        match &edit.save_as {
            Some(save) => self.save_dimension_as(ctx, edit.constraint, &edit.text, save),
            None => self.set_dimension(ctx, edit.constraint, &edit.text, edit.driving),
        }
    }

    /// Put what `text` comes to in a variable (`save`) and bind dimension
    /// `constraint` to it: a typed value becomes the variable's value with
    /// its unit, a formula the variable's formula.
    pub(crate) fn save_dimension_as(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        constraint: Uuid,
        text: &str,
        save: &SaveAs,
    ) {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return;
        };
        let Some(c) = feature
            .sketch
            .constraints
            .iter()
            .find(|c| c.id == constraint)
        else {
            return;
        };
        let dim = params::formula_dim(&c.kind);
        let text = text.trim();
        let value = match ctx.document.evaluate_formula(text, Some(dim)) {
            Ok(q) => q.value,
            Err(why) => {
                ctx.log_warn(format!("{text}: {why}"));
                return;
            }
        };
        let formula = if core_document::expr::is_constant(text) {
            core_document::expr::Quantity::new(value, dim).display(ctx.document.display_unit(), 6)
        } else {
            text.to_string()
        };
        let name = save.name.trim();
        let (set, set_name) = match save.set {
            Some(set) => match ctx.document.get_feature_meta(set) {
                Some(node) => (set, node.name.clone()),
                None => return,
            },
            None => {
                let set_name = ctx.document.unused_object_name("Variables");
                match ctx.document.add_variable_set(&set_name) {
                    Ok(set) => {
                        ctx.record(
                            "var.new",
                            commands::args(serde_json::json!({ "name": set_name })),
                            serde_json::json!(set.0.to_string()),
                        );
                        (set, set_name)
                    }
                    Err(why) => {
                        ctx.log_warn(why);
                        return;
                    }
                }
            }
        };
        if let Err(why) = ctx.document.set_variable(set, name, &formula, None) {
            ctx.log_warn(why);
            return;
        }
        ctx.record(
            "var.set",
            commands::args(serde_json::json!({
                "set": set_name,
                "name": name,
                "formula": formula,
            })),
            serde_json::Value::Null,
        );
        ctx.log_info(format!("{set_name}.{name} = {formula}"));
        let reference = format!(
            "{}.{}",
            core_document::expr::quote_name(&set_name),
            core_document::expr::quote_name(name)
        );
        self.bind_dimension(ctx, constraint, &reference, value);
    }

    /// Set a dimension from typed text: a value (with a unit if typed,
    /// `1 in`) that stands as it is, or a formula that reads other values
    /// and sets it from now on.
    pub(crate) fn set_dimension(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        constraint: Uuid,
        text: &str,
        driving: bool,
    ) {
        let Some(sketch_id) = self.active_sketch_id else {
            return;
        };
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return;
        };
        let Some(c) = feature
            .sketch
            .constraints
            .iter_mut()
            .find(|c| c.id == constraint)
        else {
            return;
        };
        let text = text.trim();
        let dim = params::formula_dim(&c.kind);
        let key = constraint.to_string();
        let evaluated = ctx.document.evaluate_formula(text, Some(dim));
        if !core_document::expr::is_constant(text) {
            if let Err(why) = core_document::expr::check_syntax(text) {
                ctx.log_warn(format!("{text}: {}", why.message));
                return;
            }
            match evaluated {
                Ok(q) => self.bind_dimension(ctx, constraint, text, q.value),
                Err(why) => {
                    ctx.log_warn(format!("{text}: {why}"));
                    let value = f64::from(sketch::dimension_value(&c.kind).unwrap_or_default());
                    self.bind_dimension(ctx, constraint, text, value);
                }
            }
            return;
        }
        let value = match evaluated {
            Ok(q) => q.value as f32,
            Err(why) => {
                ctx.log_warn(format!("{text}: {why}"));
                return;
            }
        };
        let _ = ctx.document.set_feature_formula(sketch_id, key, None);
        c.kind = sketch::with_dimension_value(&c.kind, value);
        c.driving = driving;
        ctx.record(
            "sketch.set_value",
            commands::args(serde_json::json!({
                "sketch": sketch_id.0.to_string(),
                "constraint": constraint.to_string(),
                "value": value,
                "driving": driving,
            })),
            serde_json::Value::Null,
        );
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
    }

    /// Set dimension `constraint` by `formula` from now on, standing at
    /// `value` (what the formula comes to) until the formulas are worked
    /// out again; a dimension a formula sets drives.
    fn bind_dimension(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        constraint: Uuid,
        formula: &str,
        value: f64,
    ) {
        let (Some(sketch_id), Some(mut feature)) =
            (self.active_sketch_id, self.get_active_sketch(ctx))
        else {
            return;
        };
        let Some(c) = feature
            .sketch
            .constraints
            .iter_mut()
            .find(|c| c.id == constraint)
        else {
            return;
        };
        let key = constraint.to_string();
        let _ = ctx
            .document
            .set_feature_formula(sketch_id, key.clone(), Some(formula.to_string()));
        ctx.record(
            "doc.set_formula",
            commands::args(serde_json::json!({
                "id": sketch_id.0.to_string(),
                "parameter": key,
                "formula": formula,
            })),
            serde_json::Value::Null,
        );
        c.kind = sketch::with_dimension_value(&c.kind, value as f32);
        c.driving = true;
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
    }

    /// Replace the constraint at `idx` with `constraint`, then re-solve and
    /// persist. Backs the panel's inline dimension editing: the constraint
    /// is edited in place, no extra state is kept.
    pub fn update_constraint(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        idx: usize,
        constraint: Constraint,
    ) {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return;
        };
        let Some(slot) = feature.sketch.constraints.get_mut(idx) else {
            return;
        };
        // A dimension's value or driving flag, as a recording says it; the
        // panel's other edits (a name, a label's place) are not modelling.
        let value = sketch::dimension_value(&constraint.kind);
        if (value != sketch::dimension_value(&slot.kind) || constraint.driving != slot.driving)
            && let (Some(value), Some(sketch_id)) = (value, self.active_sketch_id)
        {
            ctx.record(
                "sketch.set_value",
                commands::args(serde_json::json!({
                    "sketch": sketch_id.0.to_string(),
                    "constraint": constraint.id.to_string(),
                    "value": value,
                    "driving": constraint.driving,
                })),
                serde_json::Value::Null,
            );
        }
        // Its active and parked flags, as `sketch.set_constraint` sets them.
        if (constraint.active != slot.active || constraint.parked != slot.parked)
            && let Some(sketch_id) = self.active_sketch_id
        {
            let mut args = serde_json::json!({
                "sketch": sketch_id.0.to_string(),
                "items": ids_json(&[constraint.id]),
            });
            if constraint.active != slot.active {
                args["active"] = serde_json::json!(constraint.active);
            }
            if constraint.parked != slot.parked {
                args["parked"] = serde_json::json!(constraint.parked);
            }
            ctx.record(
                "sketch.set_constraint",
                commands::args(args),
                serde_json::Value::Null,
            );
        }
        // Formulas that read it by its old name read it by the new one.
        if let (Some(old), Some(new), Some(sketch_id)) = (
            slot.name.clone(),
            constraint.name.clone(),
            self.active_sketch_id,
        ) && old != new
            && !new.trim().is_empty()
            && let Some(sketch_name) = ctx
                .document
                .get_feature_meta(sketch_id)
                .map(|n| n.name.clone())
        {
            ctx.document.rewrite_formulas(|text| {
                core_document::expr::rename_property(text, &sketch_name, &old, &new)
            });
        }
        *slot = constraint;
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
    }
}

impl Workbench for SketchWorkbench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(
            "wb.sketch",
            "Sketch",
            "2D sketching environment with constraints and profiles.",
        )
        .icon("workbench-sketcher")
        .feature_kinds(["wb.sketch"])
        .modal()
    }

    fn feature_info(&self, node: &core_document::FeatureNode) -> FeatureInfo {
        if let Some(generator) = generator::generator_of(node) {
            return FeatureInfo {
                icon: generator.icon(),
                kind_label: format!("{} profile", generator.label()),
                family_label: "Sketch".to_string(),
                builds_solid: false,
            };
        }
        FeatureInfo {
            icon: "tree-sketch",
            kind_label: "Sketch".to_string(),
            family_label: "Sketch".to_string(),
            builds_solid: false,
        }
    }

    fn locks_view_to_plane(&self) -> bool {
        true
    }

    fn shows_origin_planes(&self) -> bool {
        self.pending_creation.is_some() && self.active_sketch_id.is_none()
    }

    fn takes_numeric_input(&self) -> bool {
        !ovp::fields_for(&self.tool_state).is_empty()
    }

    fn settings_json(&self) -> Option<serde_json::Value> {
        serde_json::to_value(self.options).ok()
    }

    fn apply_settings_json(&mut self, value: &serde_json::Value) {
        if let Ok(options) = serde_json::from_value(value.clone()) {
            self.options = options;
        }
    }

    /// The whole bench is its editing state: the open sketch, the tool in
    /// hand, the selection, the solver's last word.
    fn suspend_session(&mut self) -> Option<Box<dyn std::any::Any + Send>> {
        // The settings stay with the bench, not the tab.
        let options = self.options;
        let mut state = std::mem::take(self);
        self.options = options;
        state.options = options;
        Some(Box::new(state))
    }

    fn resume_session(&mut self, state: Option<Box<dyn std::any::Any + Send>>) {
        let options = self.options;
        *self = state
            .and_then(|s| s.downcast::<Self>().ok())
            .map(|s| *s)
            .unwrap_or_default();
        self.options = options;
    }

    fn menu_items(&self, scope: &MenuScope, _document: &core_document::Document) -> Vec<MenuItem> {
        match scope {
            MenuScope::StartPage => vec![
                MenuItem::new("sketch.start_blank", "Empty sketch")
                    .icon("sketch-new")
                    .hint("2D on XY plane"),
            ],
            _ => Vec::new(),
        }
    }

    fn run_command(
        &mut self,
        id: &str,
        args: &core_document::CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> core_document::CommandResult {
        if id == "sketch.wall_thickness" {
            return self.wall_thickness_command(args, ctx);
        }
        if id == "sketch.generator" {
            return generator::command(args, ctx);
        }
        commands::run(id, args, ctx)
    }

    /// `sketch.start_blank`: a sketch on the XY plane of the selected body,
    /// open for editing. The Edit menu's clipboard entries act on the
    /// selection of the sketch under edit.
    fn on_command(
        &mut self,
        id: &str,
        scope: &MenuScope,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> bool {
        match (scope, id) {
            (MenuScope::StartPage, "sketch.start_blank") => {
                let body = ctx.selected_body_id.map(BodyId);
                self.create_sketch_on_plane(
                    ctx,
                    body,
                    SketchPlane::default(),
                    NewSketchOn::default(),
                );
                true
            }
            (MenuScope::EditMenu, "edit.copy") => self.clipboard_copy(ctx, false),
            (MenuScope::EditMenu, "edit.cut") => self.clipboard_copy(ctx, true),
            (MenuScope::EditMenu, "edit.paste") => self.clipboard_paste(ctx),
            _ => false,
        }
    }

    fn parameters(&self, node: &core_document::FeatureNode) -> Vec<core_document::Parameter> {
        params::parameters(node)
    }

    fn settle(&self, _node: &core_document::FeatureNode, values: &mut serde_json::Value) {
        params::settle(values);
    }

    /// A sketch drawn on a datum sits on the datum's plane as the datum is
    /// now, and its geometry from other sketches and datums stands where
    /// their working data places it.
    fn derive(
        &self,
        node: &core_document::FeatureNode,
        values: &mut serde_json::Value,
        values_of: &dyn Fn(FeatureId) -> Option<serde_json::Value>,
        document: &core_document::Document,
    ) -> bool {
        let Ok(mut feature) = SketchFeature::from_json(values) else {
            return false;
        };
        let mut moved = false;
        if let Some(plane) = feature
            .support
            .as_ref()
            .and_then(|s| values_of(s.datum).and_then(|data| s.plane_from(&data)))
            && (feature.plane != plane || feature.sketch.plane != plane)
        {
            feature.plane = plane;
            feature.sketch.plane = plane;
            moved = true;
        }
        moved |= follow_references(&mut feature, node.id, values_of, document);
        if moved {
            *values = feature.to_json();
        }
        moved
    }

    fn derive_on_solid(
        &self,
        _node: &core_document::FeatureNode,
        values: &mut serde_json::Value,
        probed: &core_document::ProbedReferences,
    ) -> bool {
        let Ok(mut feature) = SketchFeature::from_json(values) else {
            return false;
        };
        // Attached by a mode: the plane the attachment makes of what the
        // solid answered.
        if let Some(attached) = feature.attached {
            if probed.probes != attached.probes() {
                return false;
            }
            let plane = attached.answered(&probed.answers).plane();
            if feature.plane == plane && feature.sketch.plane == plane {
                return false;
            }
            feature.plane = plane;
            feature.sketch.plane = plane;
            *values = feature.to_json();
            return true;
        }
        let (Some(face), Some(Ok(kernel_api::ProbeAnswer::Face { point, normal, .. }))) =
            (feature.face, probed.answers.first())
        else {
            return false;
        };
        let plane = face.plane_at(*point, *normal);
        if feature.plane == plane && feature.sketch.plane == plane {
            return false;
        }
        feature.plane = plane;
        feature.sketch.plane = plane;
        *values = feature.to_json();
        true
    }

    fn passive_geometry(
        &self,
        document: &core_document::Document,
        id: FeatureId,
        _node: &core_document::FeatureNode,
    ) -> Option<core_document::PassiveGeometry> {
        // As its formulas leave it, solved.
        let data = document.feature_values(id)?;
        let feature = SketchFeature::from_json(data).ok()?;
        // Its closed loops shaded, as while it is edited.
        let region = self
            .options
            .shade_regions
            .then(|| profile::extract_wires(&feature.sketch).ok())
            .flatten()
            .map(|wires| {
                let mut plane = profile::plane_of(&feature.plane);
                for (o, n) in plane.origin.iter_mut().zip(plane.normal) {
                    *o += n * REGION_LIFT_MM;
                }
                core_document::PassiveRegion {
                    profile: kernel_api::Profile { plane, wires },
                    opacity: REGION_OPACITY,
                }
            });
        Some(core_document::PassiveGeometry {
            mesh: render::sketch_to_lines(&feature.sketch, &feature.plane),
            revision: core_document::data_revision(data),
            tint: core_document::PassiveTint::Plain,
            region,
        })
    }

    /// The distance in pixels from the cursor to the sketch's nearest
    /// curve: the cursor unprojected onto the sketch plane, the distance
    /// measured in sketch units, then scaled by the pixels one unit spans
    /// at the sketch origin so the answer is zoom-independent.
    fn pick_feature(
        &self,
        document: &core_document::Document,
        id: FeatureId,
        _node: &core_document::FeatureNode,
        pick: &core_document::ViewportPick,
    ) -> Option<f32> {
        use core_document::runtime::{viewport_to_plane, world_to_viewport};
        let feature = SketchFeature::from_json(document.feature_values(id)?).ok()?;
        let plane = feature.plane;
        let world = viewport_to_plane(
            pick.view_proj,
            pick.viewport,
            pick.cursor,
            plane.origin,
            plane.normal,
        )?;
        let origin = glam::Vec3::from_array(plane.origin);
        let rel = glam::Vec3::from_array(world) - origin;
        let pos = Vec2D::new(
            rel.dot(glam::Vec3::from_array(plane.x_axis)),
            rel.dot(glam::Vec3::from_array(plane.y_axis)),
        );
        let o_px = world_to_viewport(pick.view_proj, pick.viewport, plane.origin)?;
        let x_px = world_to_viewport(
            pick.view_proj,
            pick.viewport,
            (origin + glam::Vec3::from_array(plane.x_axis)).to_array(),
        )?;
        let px_per_unit = ((x_px.0 - o_px.0).powi(2) + (x_px.1 - o_px.1).powi(2)).sqrt();
        if px_per_unit < 1e-6 {
            return None;
        }
        snap::nearest_curve_distance(&feature.sketch, pos).map(|units| units * px_per_unit)
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        commands::register(context);
        generator::register(context);
        context.register_action(
            core_document::ActionDescriptor::new(
                POLYLINE_ARC_ACTION,
                "Polyline: switch between line and arc",
            )
            .category("geometry.basic")
            .shortcut("M"),
        );
        // Row 0: sketch management, beside the standard tools.
        context.register_tool(
            ToolDescriptor::new_action("sketch.create", "Create sketch", Some("sketch.manage"))
                .icon("sketch-new")
                .row(0),
        );
        let manage = |id: &str, label: &str, icon: &'static str| {
            ToolDescriptor::new_action(id, label, Some("sketch.manage"))
                .icon(icon)
                .row(0)
        };
        context.register_tool(manage("sketch.edit", "Edit sketch", "sketch-edit"));
        context.register_tool(manage("sketch.attach", "Attach sketch", "sketch-map"));
        context.register_tool(
            manage("sketch.reorient", "Reorient sketch", "sketch-reorient").variants(vec![
                ToolVariant::new("flip", "Flip normal", "sketch-reorient"),
                ToolVariant::new("rotate", "Rotate 90°", "sketch-reorient"),
            ]),
        );
        context.register_tool(manage(
            "sketch.validate",
            "Validate sketch",
            "sketch-validate",
        ));
        context.register_tool(manage(
            "sketch.repair",
            "Repair sketch: join near ends, drop empty and doubled curves",
            "sketch-validate",
        ));
        context.register_tool(manage(
            "sketch.wall_thickness",
            "Check wall thickness",
            "thickness",
        ));
        context.register_tool(manage(
            "sketch.mirror_sketch",
            "Mirror sketch",
            "sketch-mirror",
        ));
        context.register_tool(
            ToolDescriptor::new_action("sketch.merge", "Merge sketches", Some("sketch.manage"))
                .icon("sketch-merge")
                .row(0),
        );
        context.register_tool(
            ToolDescriptor::new_action("sketch.finish", "Close sketch", Some("sketch.close"))
                .icon("sketch-leave")
                .row(0)
                .align_end(),
        );

        // Row 1: geometry. Tools with variants offer them from a dropdown.
        let variants = |id: &str| -> Vec<ToolVariant> {
            match id {
                "sketch.arc" => vec![
                    ToolVariant::new("center", "Center and endpoints", "arc"),
                    ToolVariant::new("3pt", "Three points", "arc-3pt"),
                ],
                "sketch.circle" => vec![
                    ToolVariant::new("center", "Center and rim", "circle"),
                    ToolVariant::new("3pt", "Three points", "circle-3pt"),
                ],
                "sketch.ellipse" => vec![
                    ToolVariant::new("center", "Center and axes", "ellipse"),
                    ToolVariant::new("3pt", "Three points", "ellipse-3pt"),
                    ToolVariant::new("arc", "Arc of ellipse", "arc-of-ellipse"),
                    ToolVariant::new("parabola", "Arc of parabola", "arc-of-parabola"),
                    ToolVariant::new("hyperbola", "Arc of hyperbola", "arc-of-hyperbola"),
                ],
                "sketch.bspline" => vec![
                    ToolVariant::new("open", "Open", "bspline"),
                    ToolVariant::new("periodic", "Periodic", "periodic-bspline"),
                    ToolVariant::new("through", "Through points", "bspline-knot"),
                    ToolVariant::new("through_periodic", "Closed through points", "bspline-knot"),
                ],
                "sketch.rect" => vec![
                    ToolVariant::new("corners", "Two corners", "rectangle"),
                    ToolVariant::new("center", "Center and corner", "rectangle-centered"),
                    ToolVariant::new("rounded", "Rounded", "rounded-rectangle"),
                    ToolVariant::new("3pt", "Three corners", "rectangle"),
                    ToolVariant::new("center3pt", "Center and two corners", "rectangle-centered"),
                    ToolVariant::new("frame", "Frame", "offset-geometry"),
                ],
                "sketch.polygon" => vec![
                    ToolVariant::new("3", "Triangle", "triangle"),
                    ToolVariant::new("4", "Square", "square"),
                    ToolVariant::new("5", "Pentagon", "pentagon"),
                    ToolVariant::new("6", "Hexagon", "hexagon"),
                    ToolVariant::new("7", "Heptagon", "heptagon"),
                    ToolVariant::new("8", "Octagon", "octagon"),
                ],
                "sketch.slot" => vec![
                    ToolVariant::new("straight", "Straight slot", "slot"),
                    ToolVariant::new("arc", "Arc slot", "arc-slot"),
                ],
                "sketch.fillet" => vec![
                    ToolVariant::new("fillet", "Fillet", "sketch-fillet"),
                    ToolVariant::new("chamfer", "Chamfer", "sketch-chamfer"),
                ],
                _ => Vec::new(),
            }
        };
        let category = |id: &str| -> &'static str {
            match id {
                "sketch.select" | "sketch.point" | "sketch.line" => "geometry.basic",
                "sketch.arc" | "sketch.circle" | "sketch.ellipse" | "sketch.bspline" => {
                    "geometry.curves"
                }
                "sketch.rect" | "sketch.polygon" | "sketch.slot" => "geometry.shapes",
                "sketch.fillet" | "sketch.trim" | "sketch.extend" | "sketch.split" => {
                    "geometry.modify"
                }
                _ => "geometry.transform",
            }
        };
        for (id, label, icon) in GEOMETRY_TOOLS {
            let mut tool = keyed(
                ToolDescriptor::new(*id, *label, Some(category(id)))
                    .icon(icon)
                    .variants(variants(id)),
            );
            tool.row = 1;
            if *id == "sketch.split" {
                // The row's planned entries sit after split.
                context.register_tool(tool);
                context.register_tool(keyed(
                    ToolDescriptor::new(
                        "sketch.external",
                        "External geometry",
                        Some("geometry.external"),
                    )
                    .icon("external-geometry")
                    .variants(vec![
                        ToolVariant::new("edges", "Projected edges", "external-geometry"),
                        ToolVariant::new(
                            "intersection",
                            "Intersection with faces",
                            "clipping-plane",
                        ),
                    ])
                    .row(1),
                ));
                context.register_tool(
                    ToolDescriptor::new_action(
                        "sketch.external_from",
                        "External from a sketch or datum",
                        Some("geometry.external"),
                    )
                    .icon("carbon-copy")
                    .row(1),
                );
                context.register_tool(
                    ToolDescriptor::new_action(
                        "sketch.external_defining",
                        "Toggle external geometry in profiles",
                        Some("geometry.external"),
                    )
                    .icon("external-geometry")
                    .row(1),
                );
                context.register_tool(
                    ToolDescriptor::new_action(
                        "sketch.carbon_copy",
                        "Carbon copy",
                        Some("geometry.external"),
                    )
                    .icon("carbon-copy")
                    .row(1),
                );
                context.register_tool(keyed(
                    ToolDescriptor::new_action(
                        "sketch.construction",
                        "Construction",
                        Some("geometry.construction"),
                    )
                    .icon("construction-mode")
                    .row(1),
                ));
                context.register_tool(keyed(
                    ToolDescriptor::new_action(
                        "sketch.internal_geometry",
                        "Show/hide internal geometry",
                        Some("geometry.construction"),
                    )
                    .icon("show-hide-internal-geometry")
                    .row(1),
                ));
                continue;
            }
            if *id == "sketch.point" {
                context.register_tool(tool);
                context.register_tool(keyed(
                    ToolDescriptor::new("sketch.polyline", "Polyline", Some("geometry.basic"))
                        .icon("polyline")
                        .row(1),
                ));
                continue;
            }
            context.register_tool(tool);
        }
        context.register_tool(
            ToolDescriptor::new_action(
                "sketch.array",
                "Rectangular array",
                Some("geometry.transform"),
            )
            .icon("rectangular-array")
            .row(1),
        );
        context.register_tool(
            ToolDescriptor::new_action("sketch.join", "Join curves", Some("geometry.modify"))
                .icon("bspline-degree")
                .row(1),
        );
        for (id, label, icon) in [
            ("sketch.bspline_convert", "Convert to B-spline", "bspline"),
            (
                "sketch.bspline_degree_up",
                "Raise spline degree",
                "bspline-degree",
            ),
            (
                "sketch.bspline_degree_down",
                "Lower spline degree",
                "bspline-degree",
            ),
        ] {
            context.register_tool(
                ToolDescriptor::new_action(id, label, Some("geometry.bspline"))
                    .icon(icon)
                    .row(1),
            );
        }
        context.register_tool(
            ToolDescriptor::new(
                "sketch.bspline_knot",
                "Insert knot",
                Some("geometry.bspline"),
            )
            .icon("bspline-knot")
            .row(1),
        );
        for (id, label, icon) in [
            ("sketch.bspline_comb", "Curvature comb", "bspline-comb"),
            ("sketch.bspline_knots", "Show knots", "bspline-polygon"),
        ] {
            context.register_tool(
                ToolDescriptor::new_action(id, label, Some("constraints.view"))
                    .icon(icon)
                    .row(2),
            );
        }
        for (id, label, icon) in [
            (
                "sketch.delete_all_geometry",
                "Delete all geometry",
                "delete-all-geometry",
            ),
            (
                "sketch.delete_all_constraints",
                "Delete all constraints",
                "delete-all-constraints",
            ),
        ] {
            context.register_tool(
                ToolDescriptor::new_action(id, label, Some("geometry.delete"))
                    .icon(icon)
                    .row(1),
            );
        }

        // Row 2: constraints, enabled by the shape of the selection.
        let constraint = |id: &str, label: &str, icon: &'static str, category: &str| {
            keyed(
                ToolDescriptor::new_action(format!("sketch.constrain.{id}"), label, Some(category))
                    .icon(icon)
                    .row(2),
            )
        };
        for (id, label, icon) in [
            ("coincident", "Coincident", "constraint-coincident"),
            (
                "point_on_object",
                "Point on object",
                "constraint-point-on-object",
            ),
            ("vertical", "Vertical", "constraint-vertical"),
            ("horizontal", "Horizontal", "constraint-horizontal"),
            ("parallel", "Parallel", "constraint-parallel"),
            ("perpendicular", "Perpendicular", "constraint-perpendicular"),
            ("tangent", "Tangent", "constraint-tangent"),
            ("equal", "Equal", "constraint-equal"),
            ("symmetric", "Symmetric", "constraint-symmetric"),
            ("block", "Block", "constraint-block"),
        ] {
            let mut tool = constraint(id, label, icon, "constraints.geometric");
            if id == "horizontal" {
                tool = tool.variants(vec![
                    ToolVariant::new("horizontal", "Horizontal", "constraint-horizontal"),
                    ToolVariant::new(
                        "horizontal_vertical",
                        "Horizontal or vertical, whichever is nearer",
                        "constraint-horizontal",
                    ),
                ]);
            }
            if id == "point_on_object" {
                tool = tool.variants(vec![
                    ToolVariant::new(
                        "point_on_object",
                        "Point on object",
                        "constraint-point-on-object",
                    ),
                    ToolVariant::new("midpoint", "Midpoint", "constraint-point-on-object"),
                ]);
            }
            context.register_tool(tool);
        }
        // One dimension tool: the distance, radius or angle the selection
        // takes.
        context.register_tool(constraint(
            "dimension",
            "Dimension",
            "dimensional-constraint",
            "constraints.dimensional",
        ));
        for (id, label, icon) in [
            ("lock", "Lock", "constraint-lock"),
            ("distance_x", "Horizontal distance", "constraint-distance-x"),
            ("distance_y", "Vertical distance", "constraint-distance-y"),
            ("distance", "Distance", "constraint-distance"),
            ("arc_length", "Arc length", "constraint-arc-length"),
            ("radius", "Radius", "constraint-radius"),
            ("diameter", "Diameter", "constraint-diameter"),
            ("angle", "Angle", "constraint-angle"),
            ("refraction", "Refraction", "constraint-refraction"),
        ] {
            let mut tool = constraint(id, label, icon, "constraints.dimensional");
            match id {
                "distance" => {
                    tool = tool.variants(vec![
                        ToolVariant::new("distance", "Distance", "constraint-distance"),
                        ToolVariant::new("gap", "Gap between curves", "constraint-distance"),
                    ]);
                }
                "radius" => {
                    tool = tool.variants(vec![
                        ToolVariant::new("radius", "Radius", "constraint-radius"),
                        ToolVariant::new(
                            "radius_diameter",
                            "Radius or diameter by kind",
                            "constraint-diameter",
                        ),
                        ToolVariant::new(
                            "ellipse_minor",
                            "An ellipse's minor radius",
                            "constraint-radius",
                        ),
                    ]);
                }
                "angle" => {
                    tool = tool.variants(vec![
                        ToolVariant::new("angle", "Between two lines", "constraint-angle"),
                        ToolVariant::new("angle_x", "To the X axis", "constraint-angle"),
                        ToolVariant::new("angle_y", "To the Y axis", "constraint-angle"),
                        ToolVariant::new(
                            "angle_at_point",
                            "Where two curves meet",
                            "constraint-angle",
                        ),
                        ToolVariant::new("arc_angle", "An arc's opening", "constraint-angle"),
                        ToolVariant::new(
                            "angle_three_points",
                            "By three points (arm, corner, arm)",
                            "constraint-angle",
                        ),
                    ]);
                }
                _ => {}
            }
            context.register_tool(tool);
        }
        context.register_tool(
            ToolDescriptor::new_action(
                "sketch.remove_axis_alignment",
                "Remove axis alignment",
                Some("constraints.geometric"),
            )
            .icon("toggle-axis-cross")
            .row(2),
        );
        for (id, label, icon) in [
            (
                "sketch.toggle_driving",
                "Toggle driving / reference",
                "toggle-driving",
            ),
            ("sketch.toggle_active", "Toggle active", "toggle-active"),
            (
                "sketch.park_constraints",
                "Park / unpark constraints",
                "pin",
            ),
        ] {
            context.register_tool(
                ToolDescriptor::new_action(id, label, Some("constraints.toggle"))
                    .icon(icon)
                    .row(2),
            );
        }
        for (id, label, icon) in [
            (
                "sketch.select_conflicting",
                "Select conflicting",
                "select-conflicting",
            ),
            (
                "sketch.select_redundant",
                "Select redundant",
                "select-redundant",
            ),
        ] {
            context.register_tool(
                ToolDescriptor::new_action(id, label, Some("constraints.select"))
                    .icon(icon)
                    .row(2),
            );
        }
        for (id, label, icon) in [
            (
                "sketch.select_malformed",
                "Select malformed",
                "select-malformed",
            ),
            (
                "sketch.select_unconstrained",
                "Select under-constrained",
                "select-unconstrained",
            ),
            (
                "sketch.select_dof",
                "Elements with DoF",
                "select-elements-with-dof",
            ),
            (
                "sketch.select_constraints",
                "Constraints of the selection",
                "dimensional-constraint",
            ),
            (
                "sketch.select_elements",
                "Elements of the selected constraints",
                "select",
            ),
        ] {
            context.register_tool(
                ToolDescriptor::new_action(id, label, Some("constraints.select"))
                    .icon(icon)
                    .row(2),
            );
        }
        context.register_tool(
            ToolDescriptor::new_action(
                "sketch.show_constraints",
                "Show/hide constraints",
                Some("constraints.view"),
            )
            .icon("show-hide-constraints")
            .row(2),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                "sketch.section_view",
                "Section view",
                Some("constraints.view"),
            )
            .icon("clipping-plane")
            .row(2),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                "sketch.parked_layer",
                "Show parked constraints",
                Some("constraints.view"),
            )
            .icon("eye")
            .row(2),
        );
        context.register_tool(
            ToolDescriptor::new_action("sketch.grid", "Grid", Some("constraints.view"))
                .icon("grid")
                .row(2),
        );
        context.register_tool(
            ToolDescriptor::new_action("sketch.snap", "Snap to objects", Some("constraints.view"))
                .icon("snap")
                .row(2),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                "sketch.rendering_order",
                "Rendering order",
                Some("constraints.view"),
            )
            .icon("rendering-order")
            .row(2),
        );
    }

    fn on_activate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        ctx.log_info("Sketch workbench activated");
    }

    fn on_deactivate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        ctx.log_info("Sketch workbench deactivated");
    }

    fn on_input(
        &mut self,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        self.sync_active_sketch_from_ctx(ctx);
        let base = active_tool.map(base_tool_id);

        if base == Some("sketch.finish") {
            if self.active_sketch_id.is_some() {
                // As the task panel's Close does: the host ends the session,
                // names its undo entry, and returns to the workbench it came
                // from.
                if let Some(feature) = self.get_active_sketch(ctx) {
                    ctx.request(HostRequest::JournalLabel(format!(
                        "Edit {}",
                        feature.sketch.name
                    )));
                }
                ctx.request(HostRequest::FinishEditing);
            } else {
                ctx.log_warn("No active sketch to finish");
            }
            return InputResult::consumed();
        }

        self.take_attach_request(ctx);

        if base == Some("sketch.create") {
            if self.pending_creation.is_none() && self.active_sketch_id.is_none() {
                let body = ctx.selected_body_id.map(BodyId);
                // A face of the selected body's own solid is followed.
                let (face, origin) = match body.and_then(|b| ctx.selected_face_in(b)) {
                    Some(face) => (Some(face), core_document::FaceOrigin::OwnSolid),
                    None => (ctx.selected_face, core_document::FaceOrigin::Elsewhere),
                };
                self.begin_sketch_creation(body, face, origin);
            }
            return InputResult::consumed();
        }

        if base == Some("sketch.edit") {
            // The sync above already adopted a sketch selected in the tree.
            if self.active_sketch_id.is_none() {
                ctx.log_warn("Select a sketch in the tree first");
            }
            return InputResult::consumed();
        }

        // A click on one of the origin's planes the picker shows chooses it.
        if let WorkbenchInputEvent::MouseRelease {
            button: core_document::MouseButton::Left,
            ..
        } = event
            && let Some(plane) = ctx.hovered_base_plane
            && self.shows_origin_planes()
        {
            self.create_on_base_plane(ctx, plane);
            return InputResult::consumed();
        }

        if self.active_sketch_id.is_none() {
            return InputResult::ignored();
        }

        // A generated sketch is made from its numbers, in its panel; the
        // viewport leaves its curves alone.
        if self.editing_generated(ctx) {
            return InputResult::ignored();
        }

        // Action tools that act on the selection or the whole sketch.
        if let (Some(tool), Some(base)) = (active_tool, base) {
            if let Some(rest) = base.strip_prefix("sketch.constrain.") {
                let which = tool_variant(tool).unwrap_or(rest);
                return self.apply_constraint_tool(ctx, which);
            }
            match base {
                "sketch.construction" => return self.toggle_construction_selected(ctx),
                "sketch.external_defining" => return self.toggle_external_defining(ctx),
                "sketch.internal_geometry" => return self.toggle_internal_geometry(ctx),
                "sketch.remove_axis_alignment" => return self.remove_axis_alignment(ctx),
                "sketch.snap" => {
                    self.snap_off = !self.snap_off;
                    return InputResult::consumed();
                }
                "sketch.show_constraints" => {
                    self.options.constraints_hidden = !self.options.constraints_hidden;
                    return InputResult::consumed();
                }
                "sketch.grid" => {
                    self.options.grid_on = !self.options.grid_on;
                    return InputResult::consumed();
                }
                "sketch.rendering_order" => {
                    self.options.construction_on_top = !self.options.construction_on_top;
                    ctx.log_info(if self.options.construction_on_top {
                        "Construction geometry draws on top"
                    } else {
                        "Normal geometry draws on top"
                    });
                    return InputResult::consumed();
                }
                "sketch.validate" => return self.validate(ctx),
                "sketch.repair" => return self.repair(ctx),
                "sketch.wall_thickness" => return self.check_walls(ctx),
                "sketch.select_malformed" => return self.select_malformed(ctx),
                "sketch.select_unconstrained" => return self.select_free(ctx, true),
                "sketch.select_dof" => return self.select_free(ctx, false),
                "sketch.select_constraints" => return self.select_constraints_of(ctx),
                "sketch.select_elements" => return self.select_elements_of(ctx),
                "sketch.reorient" => {
                    let rotate = tool_variant(tool) == Some("rotate");
                    return self.reorient(ctx, rotate);
                }
                "sketch.attach" => return self.attach_to_face(ctx),
                "sketch.array" => return self.array_selection(ctx),
                "sketch.join" => return self.join_selection(ctx),
                "sketch.bspline_convert" => {
                    return self.selection_edit(
                        ctx,
                        "sketch.to_bspline",
                        serde_json::json!({}),
                        spline_edit::to_bspline,
                    );
                }
                "sketch.bspline_degree_up" | "sketch.bspline_degree_down" => {
                    let by = if base == "sketch.bspline_degree_up" {
                        1
                    } else {
                        -1
                    };
                    return self.selection_edit(
                        ctx,
                        "sketch.spline_degree",
                        serde_json::json!({ "by": by }),
                        |s, sel| spline_edit::change_degree(s, sel, by),
                    );
                }
                "sketch.bspline_comb" => {
                    self.options.spline_comb = !self.options.spline_comb;
                    return InputResult::consumed();
                }
                "sketch.bspline_knots" => {
                    self.options.spline_knots = !self.options.spline_knots;
                    return InputResult::consumed();
                }
                "sketch.mirror_sketch" => return self.mirror_sketch(ctx),
                "sketch.external_from" => {
                    return self.open_sketch_picker(SketchPickerMode::ExternalFrom);
                }
                "sketch.carbon_copy" => {
                    return self.open_sketch_picker(SketchPickerMode::CarbonCopy);
                }
                "sketch.merge" => return self.open_sketch_picker(SketchPickerMode::Merge),
                "sketch.toggle_driving" => {
                    return self.toggle_constraint_flag(ctx, ConstraintFlag::Driving);
                }
                "sketch.toggle_active" => {
                    return self.toggle_constraint_flag(ctx, ConstraintFlag::Active);
                }
                "sketch.park_constraints" => {
                    let result = self.toggle_constraint_flag(ctx, ConstraintFlag::Parked);
                    // What moved to the other layer is out of sight.
                    self.selected_constraints.clear();
                    return result;
                }
                "sketch.section_view" => return self.toggle_section_view(ctx),
                "sketch.parked_layer" => {
                    self.parked_layer = !self.parked_layer;
                    ctx.log_info(if self.parked_layer {
                        "Showing the parked constraints"
                    } else {
                        "Showing the constraints in place"
                    });
                    return InputResult::consumed();
                }
                "sketch.select_conflicting" => return self.select_offenders(ctx, true),
                "sketch.select_redundant" => return self.select_offenders(ctx, false),
                "sketch.delete_all_geometry" => return self.delete_all(ctx, true),
                "sketch.delete_all_constraints" => return self.delete_all(ctx, false),
                _ => {}
            }
        }

        // Every remaining interaction needs an editing sketch. `None`
        // active tool behaves as select mode. Variants fold into the tool
        // they specialise.
        // A different tool starts fresh: what the last one had begun is
        // put away (a shape half drawn, typed values), and a variant's own
        // settings (a polygon's sides) apply once, as it is picked, so the
        // panel can change them afterwards.
        if active_tool.map(str::to_string) != self.activated_tool {
            self.activated_tool = active_tool.map(str::to_string);
            self.wall_check = None;
            if !self.tool_state.is_idle() {
                self.tool_state = ToolState::Idle;
            }
            self.dim_capture.clear_buffers();
            self.dim_capture.sync(&self.tool_state);
            self.flush_draw_record(ctx);
            if let Some(tool) = active_tool {
                self.apply_variant_settings(tool);
            }
        }
        let canonical = active_tool.and_then(canonical_tool);
        let tool = canonical.as_deref();
        self.copy_mode = base == Some("sketch.copy");
        // Remember the tool so the task panel can surface its settings
        // (polygon sides, slot width, fillet radius).
        if self.last_tool.as_deref() != tool {
            self.last_tool = tool.map(str::to_string);
            self.external_seen.clear();
        }
        // External geometry: a click is the host's, which picks the solid
        // edge under it; the frame hook projects what was picked.
        if base == Some("sketch.external") {
            self.last_tool = Some("sketch.external".to_string());
            let intersection = active_tool.and_then(tool_variant) == Some("intersection");
            if intersection != self.external_intersection {
                self.external_intersection = intersection;
                self.external_seen.clear();
            }
            return match event {
                WorkbenchInputEvent::KeyPress { key } => self.handle_key_press(ctx, tool, *key),
                _ => InputResult::ignored(),
            };
        }

        match event {
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                viewport_pos,
            } => self.handle_left_click(ctx, tool, *viewport_pos),
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Right,
                viewport_pos,
            } => {
                // A gesture in flight ends here. Otherwise the press is the
                // camera's (right-drag pans) and the release tells a click
                // from a pan.
                let result = self.handle_finish_gesture(ctx);
                if result.consumed {
                    return result;
                }
                // Anything else half drawn is dropped, as Escape drops it.
                if !self.tool_state.is_idle() {
                    self.tool_state = ToolState::Idle;
                    self.dim_capture.clear_buffers();
                    ctx.log_info("Sketch: cancelled current tool operation");
                    return InputResult::consumed();
                }
                self.right_press = Some(*viewport_pos);
                result
            }
            WorkbenchInputEvent::MouseRelease {
                button: core_document::MouseButton::Right,
                viewport_pos,
            } => self.handle_right_release(ctx, *viewport_pos),
            WorkbenchInputEvent::MouseRelease {
                button: core_document::MouseButton::Left,
                ..
            } => self.handle_left_release(ctx),
            WorkbenchInputEvent::MouseMove { viewport_pos } => {
                self.handle_mouse_move(ctx, tool, *viewport_pos)
            }
            WorkbenchInputEvent::KeyPress { key } => self.handle_key_press(ctx, tool, *key),
            WorkbenchInputEvent::Action { id } => self.handle_action(id),
            _ => InputResult::ignored(),
        }
    }

    /// The sketcher draws nothing under the tree: its UI lives in the
    /// task panel.
    #[cfg(feature = "egui")]
    fn ui_left_panel(&mut self, _ui: &mut egui::Ui, ctx: &mut WorkbenchRuntimeContext) {
        self.sync_active_sketch_from_ctx(ctx);
    }

    fn has_settings(&self) -> bool {
        true
    }

    /// The Sketcher preferences page: the solver, snap and view switches,
    /// the minimum wall, and the palette, read-only.
    #[cfg(feature = "egui")]
    fn ui_settings(&mut self, ui: &mut egui::Ui, filter: &str) {
        use ui_kit::widgets::{PrefRow, pref_group};
        let mut snap = !self.snap_off;
        let changed = pref_group(
            ui,
            "Solver & constraints",
            vec![
                PrefRow::toggle("Auto constraints", &mut self.options.auto_constraints)
                    .hint("Horizontal and vertical on axis-snapped lines while drawing"),
                PrefRow::toggle(
                    "Avoid redundant auto constraints",
                    &mut self.options.avoid_redundant_auto,
                )
                .hint("An auto constraint the solver calls redundant is dropped again"),
                PrefRow::toggle(
                    "Auto remove redundants",
                    &mut self.options.auto_remove_redundant,
                )
                .hint("A new constraint takes away the older ones it makes redundant"),
                PrefRow::toggle(
                    "Ask for a dimension's value",
                    &mut self.options.ask_dimension_value,
                )
                .hint("A new dimension opens its value editor, by its label"),
                PrefRow::toggle("Snap to objects", &mut snap)
                    .hint("Endpoints, midpoints and intersections attract the cursor"),
                PrefRow::toggle("Section view on open", &mut self.options.section_on_open)
                    .hint("Editing a sketch cuts the view at its plane; Section view turns it off for one sketch"),
                PrefRow::toggle("Shade closed regions", &mut self.options.shade_regions)
                    .hint("While editing, what the profile closes is shaded: what a pad takes"),
            ],
            filter,
        );
        if changed {
            self.snap_off = !snap;
        }
        pref_group(
            ui,
            "Printing",
            vec![
                PrefRow::qty(
                    "Minimum wall",
                    ui_kit::widgets::QtyField::new(&mut self.options.min_wall)
                        .unit("mm")
                        .decimals(2)
                        .speed(0.01)
                        .range(0.01..=100.0),
                )
                .hint("The wall thickness check marks walls thinner than this"),
            ],
            filter,
        );
        let pal = core_document::SketchPalette::default();
        pref_group(
            ui,
            "Colors",
            vec![
                PrefRow::swatch("Geometry", pal.geometry),
                PrefRow::swatch("Construction", pal.construction),
                PrefRow::swatch("External", pal.external),
                PrefRow::swatch("Fully constrained", pal.fully_constrained),
                PrefRow::swatch("Selected", pal.selected),
                PrefRow::swatch("Preselect", pal.preselect),
                PrefRow::swatch("Constraint", pal.constraint),
                PrefRow::swatch("Reference dimension", pal.reference),
            ],
            filter,
        );
    }

    fn task(&self, ctx: &WorkbenchRuntimeContext) -> Option<TaskInfo> {
        if self.pending_creation.is_some() {
            return Some(TaskInfo {
                title: "New sketch".to_string(),
                icon: "sketch-new",
                confirmable: false,
                stepwise: false,
            });
        }
        let id = self.active_sketch_id?;
        let name = ctx.document.get_feature_meta(id)?.name.clone();
        Some(TaskInfo {
            title: name,
            icon: "sketch-edit",
            confirmable: false,
            // Every line drawn or constraint added is a step of its own.
            stepwise: true,
        })
    }

    #[cfg(feature = "egui")]
    fn ui_task_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: core_document::TaskRequest,
    ) -> core_document::TaskOutcome {
        self.draw_task_panel(ui, ctx, request)
    }

    fn on_frame(&mut self, _dt: f32, ctx: &mut WorkbenchRuntimeContext) {
        // A shape is recorded once it is done: its tool back at rest, or
        // another tool picked.
        let shape_done = self.draw_record.as_ref().is_some_and(|r| {
            self.tool_state.is_idle() || self.last_tool.as_deref() != Some(r.tool.as_str())
        });
        if shape_done {
            self.flush_draw_record(ctx);
        }
        // The picker opens as the bench does, not at the next input.
        self.take_attach_request(ctx);
        self.sync_active_sketch_from_ctx(ctx);
        if self.active_sketch_id.is_some() && self.external_refreshed != self.active_sketch_id {
            self.external_refreshed = self.active_sketch_id;
            self.refresh_external(ctx);
        }
        if self.last_tool.as_deref() == Some("sketch.external") {
            if self.external_intersection {
                self.take_intersection_picks(ctx);
            } else {
                self.take_external_picks(ctx);
            }
        }
        self.drop_stale_wall_check(ctx);
        let seq = ctx.document.mutation_seq();
        let from_outside = self.own_seq.swap(seq, std::sync::atomic::Ordering::Relaxed) != seq;
        self.selection_shape = match self.get_active_sketch(ctx) {
            Some(feature) => {
                // What was selected may be gone, by an edit here that took
                // more with it (an element's constraints) or one from outside.
                self.selected.retain(|id| {
                    feature.sketch.get_geometry(*id).is_some()
                        || sketch::Reference::of(*id).is_some()
                });
                self.selected_constraints
                    .retain(|id| feature.sketch.constraints.iter().any(|c| c.id == *id));
                if from_outside {
                    // The verdict is of a sketch that is not there any more.
                    self.last_solve = None;
                    self.last_diagnosis = None;
                }
                self.section_view = feature.shows_section(self.options.section_on_open);
                let selected: Vec<Uuid> = self.selected.iter().copied().collect();
                self.internal_target = !internal::curves_of(&feature.sketch, &selected).is_empty();
                constrain::SelectionShape::picked_in(
                    &feature.sketch,
                    &self.selected,
                    self.selected.in_order(),
                )
            }
            None => constrain::SelectionShape::default(),
        };
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        if self.editing_generated(ctx) {
            return tool_id == "sketch.finish";
        }
        let editing = self.active_sketch_id.is_some();
        if let Some(rest) = tool_id.strip_prefix("sketch.constrain.") {
            let which = tool_variant(tool_id).unwrap_or(rest);
            if which == "dimension" {
                return editing && dimension_for(&self.selection_shape).is_some();
            }
            return editing && constrain::fits(which, &self.selection_shape);
        }
        match tool_id {
            "sketch.create" => ctx.selected_body_id.is_some(),
            "sketch.edit" => {
                !editing
                    && ctx
                        .active_document_object
                        .is_some_and(|id| self.is_sketch_feature(ctx, id))
            }
            "sketch.attach" => editing && ctx.selected_face.is_some(),
            "sketch.array" => editing && !self.selected.is_empty(),
            "sketch.bspline_convert" => {
                editing && self.selection_shape.all.len() > self.selection_shape.points.len()
            }
            "sketch.bspline_degree_up" | "sketch.bspline_degree_down" => {
                editing && !self.selection_shape.others.is_empty()
            }
            "sketch.internal_geometry" => editing && self.internal_target,
            "sketch.remove_axis_alignment" => editing && !self.selection_shape.lines.is_empty(),
            "sketch.join" => {
                editing && self.selection_shape.all.len() - self.selection_shape.points.len() >= 2
            }
            "sketch.toggle_driving" | "sketch.toggle_active" | "sketch.park_constraints" => {
                editing && !self.selected_constraints.is_empty()
            }
            "sketch.select_constraints" => editing && !self.selected.is_empty(),
            "sketch.select_elements" => editing && !self.selected_constraints.is_empty(),
            "sketch.select_conflicting" => {
                editing
                    && self
                        .last_diagnosis
                        .as_ref()
                        .is_some_and(|d| !d.conflicting.is_empty())
            }
            "sketch.select_redundant" => {
                editing
                    && self
                        .last_diagnosis
                        .as_ref()
                        .is_some_and(|d| !d.redundant.is_empty())
            }
            _ => editing,
        }
    }

    fn tool_toggled(&self, tool_id: &str) -> bool {
        match tool_id {
            "sketch.construction" => self.construction_mode,
            "sketch.snap" => !self.snap_off,
            "sketch.show_constraints" => self.options.constraints_hidden,
            "sketch.grid" => self.options.grid_on,
            "sketch.rendering_order" => self.options.construction_on_top,
            "sketch.bspline_comb" => self.options.spline_comb,
            "sketch.bspline_knots" => self.options.spline_knots,
            "sketch.parked_layer" => self.parked_layer,
            "sketch.section_view" => self.section_view,
            "sketch.carbon_copy" | "sketch.merge" => self.sketch_picker.as_ref().is_some_and(|p| {
                (p.mode == SketchPickerMode::Merge) == (tool_id == "sketch.merge")
            }),
            _ => false,
        }
    }

    fn editing_feature(&self) -> Option<FeatureId> {
        self.active_sketch_id
    }

    fn finish_editing(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.flush_draw_record(ctx);
        if self.active_sketch_id.is_some() {
            self.active_sketch_id = None;
            self.session_start = None;
            self.sketch_picker = None;
            self.clear_interaction_state();
            // Deselect the feature: with it still active the next input
            // event would immediately re-enter editing via
            // `sync_active_sketch_from_ctx`.
            ctx.active_document_object = None;
            ctx.log_info("Exited sketch editing mode");
        } else {
            ctx.log_warn("Not in sketch editing mode");
        }
    }

    fn shortcuts_changed(&mut self, keys: &HashMap<String, Vec<core_document::Chord>>) {
        self.action_keys = keys
            .iter()
            .filter_map(|(id, chords)| Some((id.clone(), chords.first()?.to_string())))
            .collect();
    }

    fn viewport_hud(&self, ctx: &WorkbenchRuntimeContext) -> Option<ViewportHud> {
        let feature = self.get_active_sketch(ctx)?;
        let proj = SketchProjector::new(ctx, feature.plane);
        let pal = ctx.sketch_palette;
        let tool = self.last_tool.as_deref().unwrap_or("sketch.select");
        let (name, prompt) = match self.tool_state.hint() {
            Some((name, prompt)) => (name, prompt),
            None if tool == "sketch.external" && self.external_intersection => (
                "Intersection",
                "Click faces of a solid to add where they cross the sketch plane",
            ),
            None => idle_hint(tool),
        };
        let mut keys: Vec<(String, &'static str)> = Vec::new();
        let mut key = |key: &str, meaning: &'static str| keys.push((key.to_string(), meaning));
        if self.dim_capture.is_active() {
            key("Tab", "next field");
            key("Enter", "lock value");
        } else if matches!(
            self.tool_state,
            ToolState::LineFrom { chain: true, .. } | ToolState::BSplineDraw { .. }
        ) {
            key("Enter", "finish");
        } else if let ToolState::PolylineFrom {
            segment, heading, ..
        } = self.tool_state
        {
            if heading.is_some()
                && let Some(switch) = self.action_keys.get(POLYLINE_ARC_ACTION)
            {
                key(
                    switch,
                    match segment.next() {
                        tools::PolySegment::Line => "lines",
                        tools::PolySegment::Tangent => "tangent arcs",
                        tools::PolySegment::Perpendicular => "square arcs",
                        tools::PolySegment::Reverse => "reversed arcs",
                    },
                );
            }
            key("Enter", "finish");
        }
        // Selecting is what the view does at rest: it takes no card.
        let resting = tool == "sketch.select" && self.tool_state.hint().is_none();
        if !resting {
            key("Esc", "cancel");
        }
        let verdict = self.solver_verdict(&feature.sketch);
        let zoom = 1.0 / proj.units_per_px().max(1e-6);
        let ovp = self.cursor.and_then(|cursor| {
            let px = proj.to_px(cursor)?;
            let rows =
                ovp::readout_rows(&self.dim_capture, &self.tool_state, &feature.sketch, cursor);
            // An ellipse's axes take no constraint: typed, they only size it.
            let hint = if matches!(
                self.tool_state,
                ToolState::EllipseCenter { .. } | ToolState::EllipseMajor { .. }
            ) {
                "Tab next · Enter places it"
            } else {
                "Tab next · Enter constrains · click keeps free"
            };
            (!rows.is_empty()).then(|| core_document::OvpWidget {
                anchor: [px[0] + 22.0, px[1] - 12.0],
                rows,
                hint,
            })
        });
        Some(ViewportHud {
            tool: (!resting).then(|| ToolHint {
                icon: tool_icon(tool),
                name: name.to_string(),
                prompt: prompt.to_string(),
                keys,
            }),
            badge: Some((verdict.kind.color(&pal), format!("{} DoF", verdict.dof))),
            legend: vec![
                (pal.geometry, "Normal"),
                (pal.construction, "Construction"),
                (pal.external, "External"),
                (pal.fully_constrained, "Fully constrained"),
                (pal.constraint, "Constraint"),
            ],
            footer: [
                style::placement_label(&feature),
                if self.snap_off {
                    "Snap: off".to_string()
                } else {
                    "Snap: objects".to_string()
                },
                format!("Zoom {zoom:.1}×"),
            ]
            .into_iter()
            .chain(
                self.parked_layer
                    .then(|| "Constraints: parked layer".to_string()),
            )
            .chain(self.shown_wall_check().map(walls::WallCheck::summary))
            .collect(),
            ovp,
        })
    }

    /// A sketch edited in section view cuts the scene at its plane, keeping
    /// the side away from the viewer, and a hair beyond the plane so a face
    /// the sketch lies on stays.
    fn clip_plane(&self, ctx: &WorkbenchRuntimeContext) -> Option<[f32; 4]> {
        let feature = self.get_active_sketch(ctx)?;
        if !feature.shows_section(self.options.section_on_open) {
            return None;
        }
        Some(section_equation(&feature.plane, ctx.camera_position))
    }

    fn status_items(&self, ctx: &WorkbenchRuntimeContext) -> Option<StatusItems> {
        let feature = self.get_active_sketch(ctx)?;
        let pal = ctx.sketch_palette;
        let verdict = self.solver_verdict(&feature.sketch);
        let mut names: Vec<String> = feature
            .sketch
            .geometry
            .iter()
            .filter(|g| self.selected.contains(&g.id()))
            .map(|g| style::element_name(&feature.sketch, g.id()))
            .collect();
        names.extend(
            feature
                .sketch
                .constraints
                .iter()
                .filter(|c| self.selected_constraints.contains(&c.id))
                .map(|c| {
                    c.name
                        .clone()
                        .unwrap_or_else(|| sketch::constraint_label(&c.kind))
                }),
        );
        let selection = match names.len() {
            0 => None,
            n if n <= 3 => Some(names.join(" · ")),
            n => Some(format!("{} · {} more", names[..2].join(" · "), n - 2)),
        };
        Some(StatusItems {
            state: Some((verdict.kind.color(&pal), verdict.title)),
            selection,
            coords: self
                .cursor
                .map(|c| format!("X {:.2} · Y {:.2} mm", c.x, c.y)),
            mode: Some("Sketch edit mode".to_string()),
        })
    }

    /// The edited sketch's reference pictures, laid on its plane under
    /// its lines.
    fn get_screen_space_images(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceImage> {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return Vec::new();
        };
        let proj = SketchProjector::new(ctx, feature.plane);
        feature
            .sketch
            .images
            .iter()
            .filter_map(|image| {
                let decoded = self.image_cache.get(image.asset, || {
                    ctx.document.asset_bytes(image.asset).map(<[u8]>::to_vec)
                })?;
                let aspect = decoded.height as f32 / decoded.width.max(1) as f32;
                let [a, b, c, d] = images::corners(image, aspect);
                let (hi, lo) = image.asset.as_u64_pair();
                Some(core_document::ScreenSpaceImage {
                    key: hi ^ lo,
                    size: [decoded.width, decoded.height],
                    rgba: std::sync::Arc::clone(&decoded.rgba),
                    corners: [
                        proj.to_px(a)?,
                        proj.to_px(b)?,
                        proj.to_px(c)?,
                        proj.to_px(d)?,
                    ],
                    opacity: image.opacity,
                })
            })
            .collect()
    }

    /// The regions the edited sketch's profile closes, shaded on its
    /// plane: what a feature swept from it takes, holes open.
    fn get_overlay_meshes(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::OverlayMesh> {
        if !self.options.shade_regions {
            return Vec::new();
        }
        let (Some(feature), Some(kernel)) = (self.get_active_sketch(ctx), ctx.kernel) else {
            return Vec::new();
        };
        let Ok(wires) = profile::extract_wires(&feature.sketch) else {
            return Vec::new();
        };
        let mut plane = profile::plane_of(&feature.plane);
        // A hair off the plane, toward its normal, so a face it is drawn
        // on does not fight it.
        for (o, n) in plane.origin.iter_mut().zip(plane.normal) {
            *o += n * REGION_LIFT_MM;
        }
        let profile = kernel_api::Profile { plane, wires };
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            serde_json::to_string(&profile)
                .unwrap_or_default()
                .hash(&mut hasher);
            hasher.finish()
        };
        let mut cached = self.regions.borrow_mut();
        if cached.as_ref().is_none_or(|(k, _)| *k != key) {
            let mesh = kernel.profile_mesh(&profile).unwrap_or_default();
            *cached = Some((key, mesh));
        }
        let Some((_, mesh)) = cached.as_ref() else {
            return Vec::new();
        };
        if mesh.indices.is_empty() {
            return Vec::new();
        }
        vec![core_document::OverlayMesh {
            mesh: mesh.clone(),
            color: ctx.sketch_palette.geometry,
            wireframe: false,
            opacity: REGION_OPACITY,
            on_top: false,
        }]
    }

    fn get_screen_space_overlays(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceOverlay> {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return Vec::new();
        };
        let proj = SketchProjector::new(ctx, feature.plane);
        let pal = ctx.sketch_palette;
        let mut out = self.grid_lines(ctx, &feature.plane, &proj, &pal);
        out.extend(self.build_overlays(ctx, &feature, &proj, &pal).lines);
        if let Some(check) = self.shown_wall_check() {
            out.extend(walls::overlays(check, &proj, &pal));
        }
        if self.options.spline_comb {
            for spline in feature.sketch.geometry.iter().filter_map(|g| match g {
                GeometryElement::BSpline(b) => Some(b.id),
                _ => None,
            }) {
                for (a, b) in spline_edit::comb(&feature.sketch, spline) {
                    if let (Some(a), Some(b)) = (proj.to_px(a), proj.to_px(b)) {
                        out.push(core_document::ScreenSpaceOverlay::new(
                            a,
                            b,
                            pal.preview,
                            1.0,
                        ));
                    }
                }
            }
        }
        if !self.options.constraints_hidden {
            let glyphs = self.glyphs(ctx, &feature.sketch, &proj);
            out.extend(glyphs::dimension_overlays(&glyphs));
        }
        out
    }

    fn get_screen_space_marks(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<ScreenSpaceMark> {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return Vec::new();
        };
        let proj = SketchProjector::new(ctx, feature.plane);
        let pal = ctx.sketch_palette;
        let mut out = self.build_overlays(ctx, &feature, &proj, &pal).marks;
        if let Some(check) = self.shown_wall_check() {
            out.extend(walls::marks(check, &proj, &pal));
        }
        if self.options.spline_knots {
            for spline in feature.sketch.geometry.iter().filter_map(|g| match g {
                GeometryElement::BSpline(b) => Some(b.id),
                _ => None,
            }) {
                for (at, multiplicity) in spline_edit::knot_points(&feature.sketch, spline) {
                    if let Some(px) = proj.to_px(at) {
                        let radius = 2.5 + multiplicity as f32;
                        out.push(ScreenSpaceMark::dot(px, radius, pal.constraint));
                    }
                }
            }
        }
        if !self.options.constraints_hidden {
            out.extend(
                self.glyphs(ctx, &feature.sketch, &proj)
                    .iter()
                    .filter_map(glyphs::Glyph::mark),
            );
        }
        out
    }

    fn get_screen_space_labels(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<ScreenSpaceLabel> {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return Vec::new();
        };
        let proj = SketchProjector::new(ctx, feature.plane);
        let pal = ctx.sketch_palette;
        let mut labels: Vec<ScreenSpaceLabel> = if self.options.constraints_hidden {
            Vec::new()
        } else {
            self.glyphs(ctx, &feature.sketch, &proj)
                .iter()
                .filter_map(glyphs::Glyph::label)
                .collect()
        };
        // What the cursor would snap to, by name.
        labels.extend(self.build_overlays(ctx, &feature, &proj, &pal).labels);
        if let Some(check) = self.shown_wall_check() {
            labels.extend(walls::labels(check, &proj, &pal));
        }
        labels
    }
}

impl SketchWorkbench {
    /// The constraint glyphs of the layer shown, in viewport pixels.
    fn glyphs(
        &self,
        ctx: &WorkbenchRuntimeContext,
        sketch: &Sketch,
        proj: &SketchProjector,
    ) -> Vec<glyphs::Glyph> {
        glyphs::build(
            sketch,
            proj,
            &self.selected_constraints,
            &self.bound_dimensions(ctx),
            &ctx.sketch_palette,
            self.parked_layer,
            self.options.dimension_labels,
        )
    }

    /// The geometry, previews and markers of one frame.
    fn build_overlays(
        &self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        proj: &SketchProjector,
        pal: &SketchPalette,
    ) -> overlay::Overlays {
        let snap_tol = SNAP_TOLERANCE_PX * proj.units_per_px();
        // Where a click would land, as the click itself works it out.
        let snap = match (self.last_tool.as_deref(), self.cursor) {
            (Some(tool), Some(cursor))
                if is_draw_tool(tool)
                    && self.box_select.is_none()
                    && self.dim_capture.typed().is_empty() =>
            {
                let (cursor, tol) = self.landing(ctx, feature, tool, cursor);
                Some(tools::snap_at(
                    &self.tool_state,
                    &feature.sketch,
                    cursor,
                    tol,
                ))
            }
            _ => None,
        };
        // Selected constraints show the geometry they hold, in their own
        // colour: it is not selected, so tools do not act on it.
        let selected = self.selected.clone();
        let mut referenced: HashSet<Uuid> = HashSet::new();
        for c in &feature.sketch.constraints {
            if self.selected_constraints.contains(&c.id) {
                referenced.extend(sketch::constraint_refs(&c.kind));
            }
        }
        // Typed values place the preview where they will place the click.
        let typed = self.dim_capture.typed();
        let preview_cursor = self.cursor.map(|cursor| {
            if typed.is_empty() {
                // A transform's ghost goes where its click would.
                match self.last_tool.as_deref() {
                    Some(tool) if is_transform_tool(tool) => {
                        self.landing(ctx, feature, tool, cursor).0
                    }
                    _ => cursor,
                }
            } else {
                ovp::override_cursor(&self.tool_state, &feature.sketch, cursor, &typed)
            }
        });
        overlay::build_overlays(
            proj,
            pal,
            &feature.sketch,
            &selected,
            &referenced,
            self.hovered,
            &self.tool_state,
            preview_cursor,
            &ToolParams {
                // The copy tool makes one copy at least.
                copies: if self.copy_mode {
                    self.tool_params.copies.max(1)
                } else {
                    self.tool_params.copies
                },
                ..self.tool_params
            },
            self.box_select.as_ref().map(|b| (b.anchor, b.current)),
            self.last_tool.as_deref(),
            snap_tol,
            snap,
            self.options.construction_on_top,
        )
    }

    /// The solver's verdict from the cached diagnosis, or a cheap estimate
    /// of the freedom left when none is cached yet.
    pub(crate) fn solver_verdict(&self, sketch: &Sketch) -> SolverVerdict {
        let not_converged = matches!(self.last_solve, Some(SolveOutcome::NotConverged { .. }));
        let (dof, analyzed, conflicting, redundant) = match &self.last_diagnosis {
            Some(d) => (
                d.dof,
                d.analyzed,
                d.conflicting.clone(),
                d.redundant.clone(),
            ),
            None => (solver::dof_estimate(sketch), true, Vec::new(), Vec::new()),
        };
        let verdict = |kind, title: &str, body: String, offenders: Vec<Uuid>| SolverVerdict {
            kind,
            title: title.to_string(),
            body,
            dof,
            offenders,
        };
        if sketch.geometry.is_empty() {
            verdict(
                SolverKind::Empty,
                "Empty sketch",
                "Draw geometry with the tools above.".into(),
                Vec::new(),
            )
        } else if !analyzed {
            verdict(
                SolverKind::Unanalyzed,
                "Not analyzed",
                "Too many constraints to check for conflicts.".into(),
                Vec::new(),
            )
        } else if not_converged || !conflicting.is_empty() {
            verdict(
                SolverKind::Conflicting,
                "Conflicting constraints",
                "Constraints contradict each other. Click to select their geometry.".into(),
                conflicting,
            )
        } else if !redundant.is_empty() {
            verdict(
                SolverKind::Redundant,
                "Redundant constraints",
                "Some constraints add nothing the others do not already enforce.".into(),
                redundant,
            )
        } else if dof == 0 && !sketch.constraints.is_empty() {
            verdict(
                SolverKind::Fully,
                "Fully constrained",
                "Sketch has 0 degrees of freedom.".into(),
                Vec::new(),
            )
        } else {
            verdict(
                SolverKind::Under,
                "Under-constrained",
                format!("{dof} degrees of freedom remain. Add dimensions or constraints."),
                Vec::new(),
            )
        }
    }

    /// The settings a tool variant carries (a hexagon's six sides, a
    /// periodic spline), put in the panel when it is picked.
    fn apply_variant_settings(&mut self, tool: &str) {
        match (base_tool_id(tool), tool_variant(tool)) {
            ("sketch.polygon", Some(sides)) => {
                if let Ok(n) = sides.parse::<u32>() {
                    self.tool_params.polygon_sides = n.clamp(3, 12);
                }
            }
            ("sketch.bspline", Some(variant)) => {
                self.tool_params.bspline_periodic =
                    matches!(variant, "periodic" | "through_periodic");
                self.tool_params.bspline_interpolate =
                    matches!(variant, "through" | "through_periodic");
            }
            _ => {}
        }
    }
}

/// The tool a variant specialises, as the tool state machine knows it;
/// `None` for a tool not the sketcher's.
fn canonical_tool(tool: &str) -> Option<String> {
    let base = base_tool_id(tool);
    if !base.starts_with("sketch.") {
        return None;
    }
    Some(match (base, tool_variant(tool)) {
        ("sketch.arc", Some("3pt")) => "sketch.arc3".to_string(),
        ("sketch.circle", Some("3pt")) => "sketch.circle3".to_string(),
        ("sketch.ellipse", Some("3pt")) => "sketch.ellipse3".to_string(),
        ("sketch.ellipse", Some("arc")) => "sketch.ellipse_arc".to_string(),
        ("sketch.ellipse", Some("parabola")) => "sketch.parabola".to_string(),
        ("sketch.ellipse", Some("hyperbola")) => "sketch.hyperbola".to_string(),
        ("sketch.rect", Some("center")) => "sketch.rect_center".to_string(),
        ("sketch.rect", Some("rounded")) => "sketch.rect_rounded".to_string(),
        ("sketch.rect", Some("3pt")) => "sketch.rect3".to_string(),
        ("sketch.rect", Some("center3pt")) => "sketch.rect_center3".to_string(),
        ("sketch.rect", Some("frame")) => "sketch.rect_frame".to_string(),
        ("sketch.slot", Some("arc")) => "sketch.arc_slot".to_string(),
        ("sketch.fillet", Some("chamfer")) => "sketch.chamfer".to_string(),
        ("sketch.polygon", Some(_)) => "sketch.polygon".to_string(),
        ("sketch.bspline", Some(_)) => "sketch.bspline".to_string(),
        ("sketch.copy", _) => "sketch.translate".to_string(),
        (base, _) => base.to_string(),
    })
}

impl SketchWorkbench {
    /// A constraint tool on the current selection: every kind it maps to
    /// is added, dimensional ones at their measured value.
    fn apply_constraint_tool(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        which: &str,
    ) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let shape = constrain::SelectionShape::picked_in(
            &feature.sketch,
            &self.selected,
            self.selected.in_order(),
        );
        // "dimension" is whichever dimensional tool the selection takes.
        let which = if which == "dimension" {
            match constrain::dimension_in(&shape, &feature.sketch) {
                Some(tool) => tool,
                None => {
                    ctx.log_warn("Select a line, circles, or two items to dimension");
                    self.selection_shape = shape;
                    return InputResult::consumed();
                }
            }
        } else {
            which
        };
        let mut items: Vec<Uuid> = self.selected.iter().copied().collect();
        items.sort();
        let mut feature = feature;
        match commands::constrain(&mut feature.sketch, which, &items, None) {
            Ok(made) => {
                let remove_redundant = self.options.auto_remove_redundant;
                let removed = if remove_redundant {
                    commands::remove_superseded(&mut feature.sketch, &made)
                } else {
                    0
                };
                if let Some(sketch_id) = self.active_sketch_id {
                    let mut args = serde_json::json!({
                        "sketch": sketch_id.0.to_string(),
                        "kind": which,
                        "items": ids_json(&items),
                    });
                    if remove_redundant {
                        args["remove_redundant"] = serde_json::json!(true);
                    }
                    ctx.record(
                        "sketch.constrain",
                        commands::args(args),
                        serde_json::json!(made),
                    );
                }
                if removed > 0 {
                    ctx.log_info(format!(
                        "Removed {removed} constraint(s) the new one made redundant"
                    ));
                    self.selected_constraints
                        .retain(|id| feature.sketch.constraints.iter().any(|c| c.id == *id));
                }
                let mut dimension = None;
                for id in &made {
                    if let Some(c) = feature
                        .sketch
                        .constraints
                        .iter()
                        .find(|c| c.id.to_string() == *id)
                    {
                        ctx.log_info(format!(
                            "Added constraint: {}",
                            sketch::constraint_label(&c.kind)
                        ));
                        if c.kind.is_dimensional() && dimension.is_none() {
                            dimension = Some(c.id);
                        }
                    }
                }
                self.solve(ctx, &mut feature);
                if let Some(id) = dimension {
                    if self.options.ask_dimension_value {
                        self.open_new_dimension(ctx, &feature, id);
                    } else {
                        self.pending_focus = Some(id);
                    }
                }
                self.store_sketch(ctx, feature);
            }
            Err(_) => ctx.log_warn(format!(
                "The {which} constraint does not fit the current selection"
            )),
        }
        self.selection_shape = shape;
        InputResult::consumed()
    }

    /// Open the value editor on dimension `id`, just added: by its label,
    /// showing what it measured.
    fn open_new_dimension(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        id: Uuid,
    ) {
        let Some(c) = feature.sketch.constraints.iter().find(|c| c.id == id) else {
            return;
        };
        let proj = SketchProjector::new(ctx, feature.plane);
        let pos = self
            .glyphs(ctx, &feature.sketch, &proj)
            .iter()
            .find(|g| g.constraint == id)
            .map(|g| g.pos)
            .unwrap_or_else(|| {
                let (_, _, w, h) = ctx.viewport;
                [w as f32 / 2.0, h as f32 / 2.0]
            });
        let text = sketch::dimension_value(&c.kind)
            .map(glyphs::fmt_num)
            .unwrap_or_default();
        let mut edit = DimEdit::new(id, pos, text, c.driving);
        edit.new = true;
        self.dim_edit = Some(edit);
    }

    /// Flip one flag of the selected constraints, each for itself, through
    /// `sketch.set_constraint`'s code.
    fn toggle_constraint_flag(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        which: ConstraintFlag,
    ) -> InputResult {
        let (Some(mut feature), Some(id)) = (self.get_active_sketch(ctx), self.active_sketch_id)
        else {
            return InputResult::ignored();
        };
        // The selected constraints, split by the flag each ends with.
        let mut to: [Vec<Uuid>; 2] = [Vec::new(), Vec::new()];
        for c in &feature.sketch.constraints {
            if self.selected_constraints.contains(&c.id) {
                let now = match which {
                    ConstraintFlag::Driving => c.driving,
                    ConstraintFlag::Active => c.active,
                    ConstraintFlag::Parked => c.parked,
                };
                to[usize::from(!now)].push(c.id);
            }
        }
        if to.iter().all(Vec::is_empty) {
            return InputResult::consumed();
        }
        for (flag, mut items) in [(false, to[0].clone()), (true, to[1].clone())] {
            if items.is_empty() {
                continue;
            }
            items.sort();
            let mut flags = commands::ConstraintFlags::default();
            let name = match which {
                ConstraintFlag::Driving => {
                    flags.driving = Some(flag);
                    "driving"
                }
                ConstraintFlag::Active => {
                    flags.active = Some(flag);
                    "active"
                }
                ConstraintFlag::Parked => {
                    flags.parked = Some(flag);
                    "parked"
                }
            };
            commands::set_constraints(&mut feature.sketch, &items, flags);
            let mut args = serde_json::json!({
                "sketch": id.0.to_string(),
                "items": ids_json(&items),
            });
            args[name] = serde_json::json!(flag);
            ctx.record(
                "sketch.set_constraint",
                commands::args(args),
                serde_json::Value::Null,
            );
        }
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
        InputResult::consumed()
    }

    /// The grid step on screen: the size chosen, or, following the zoom, a
    /// 1-2-5 step that puts lines about forty pixels apart.
    fn grid_step(&self, units_per_px: f32) -> f32 {
        if !self.options.grid_auto {
            return self.options.grid_size.max(1e-3);
        }
        let raw = 40.0 * units_per_px;
        let magnitude = 10f32.powf(raw.log10().floor());
        let mantissa = raw / magnitude;
        let nice = if mantissa <= 1.0 {
            1.0
        } else if mantissa <= 2.0 {
            2.0
        } else if mantissa <= 5.0 {
            5.0
        } else {
            10.0
        };
        nice * magnitude
    }

    /// Where a drawing click lands with the grid snapping on: the nearest
    /// grid point.
    fn grid_snapped(
        &self,
        ctx: &WorkbenchRuntimeContext,
        plane: &SketchPlane,
        cursor: Vec2D,
    ) -> Vec2D {
        if !(self.options.grid_on && self.options.grid_snap) {
            return cursor;
        }
        let step = self.grid_step(SketchProjector::new(ctx, *plane).units_per_px());
        Vec2D::new(
            (cursor.x / step).round() * step,
            (cursor.y / step).round() * step,
        )
    }

    /// The grid across the viewport, when it is on: lines at every step
    /// over the part of the plane the viewport shows.
    fn grid_lines(
        &self,
        ctx: &WorkbenchRuntimeContext,
        plane: &SketchPlane,
        proj: &SketchProjector,
        pal: &SketchPalette,
    ) -> Vec<core_document::ScreenSpaceOverlay> {
        if !self.options.grid_on {
            return Vec::new();
        }
        let units_per_px = proj.units_per_px();
        let step = self.grid_step(units_per_px);
        // A step under six pixels would be a wall, not a grid.
        if step / units_per_px < 6.0 {
            return Vec::new();
        }
        let (_, _, w, h) = ctx.viewport;
        let corners = [
            (0.0, 0.0),
            (w as f32, 0.0),
            (0.0, h as f32),
            (w as f32, h as f32),
        ];
        let mut lo = Vec2D::new(f32::MAX, f32::MAX);
        let mut hi = Vec2D::new(f32::MIN, f32::MIN);
        for corner in corners {
            let Some(at) = Self::cursor_to_sketch(ctx, plane, corner) else {
                return Vec::new();
            };
            lo = Vec2D::new(lo.x.min(at.x), lo.y.min(at.y));
            hi = Vec2D::new(hi.x.max(at.x), hi.y.max(at.y));
        }
        const MAX_LINES: i64 = 400;
        let mut out = Vec::new();
        let color = pal.inactive;
        let mut line = |a: Vec2D, b: Vec2D| {
            if let (Some(a), Some(b)) = (proj.to_px(a), proj.to_px(b)) {
                out.push(core_document::ScreenSpaceOverlay::new(a, b, color, 1.0).with_alpha(0.35));
            }
        };
        let (x0, x1) = ((lo.x / step).floor() as i64, (hi.x / step).ceil() as i64);
        let (y0, y1) = ((lo.y / step).floor() as i64, (hi.y / step).ceil() as i64);
        if x1 - x0 > MAX_LINES || y1 - y0 > MAX_LINES {
            return Vec::new();
        }
        for i in x0..=x1 {
            let x = i as f32 * step;
            line(Vec2D::new(x, lo.y), Vec2D::new(x, hi.y));
        }
        for j in y0..=y1 {
            let y = j as f32 * step;
            line(Vec2D::new(lo.x, y), Vec2D::new(hi.x, y));
        }
        out
    }

    /// Selected external geometry counts in profiles, or stops counting:
    /// all of it on when any was off.
    fn toggle_external_defining(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let (Some(mut feature), Some(id)) = (self.get_active_sketch(ctx), self.active_sketch_id)
        else {
            return InputResult::ignored();
        };
        let items: Vec<Uuid> = self
            .selected
            .iter()
            .copied()
            .filter(|e| feature.sketch.external.contains_key(e))
            .collect();
        if items.is_empty() {
            ctx.log_warn("Select external geometry to count in the profile");
            return InputResult::consumed();
        }
        let on = items.iter().any(|e| !feature.sketch.external[e].defining);
        commands::set_external_defining(&mut feature.sketch, &items, on);
        self.store_sketch(ctx, feature);
        ctx.record(
            "sketch.external_defining",
            commands::args(serde_json::json!({
                "sketch": id.0.to_string(),
                "items": items.iter().map(|i| i.to_string()).collect::<Vec<_>>(),
                "on": on,
            })),
            serde_json::Value::Null,
        );
        ctx.log_info(if on {
            "External geometry counts in the profile"
        } else {
            "External geometry only guides the sketch"
        });
        InputResult::consumed()
    }

    /// Repair the sketch being edited, ends within the snap reach joined,
    /// as `sketch.repair` does.
    fn repair(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let Some(id) = self.active_sketch_id else {
            return InputResult::ignored();
        };
        let tolerance = Self::snap_tolerance(ctx, &feature.plane) * 0.5;
        let done = repair::repair(&mut feature.sketch, tolerance);
        if !done.is_empty() {
            self.selected.clear();
            self.selected_constraints.clear();
            self.solve(ctx, &mut feature);
            self.store_sketch(ctx, feature);
            ctx.record(
                "sketch.repair",
                commands::args(serde_json::json!({
                    "sketch": id.0.to_string(),
                    "tolerance": tolerance,
                })),
                serde_json::json!(done.describe()),
            );
        }
        ctx.log_info(format!("Sketch repair: {}", done.describe()));
        InputResult::consumed()
    }

    /// Stray points, constraints that reference missing geometry, and
    /// whether the sketch closes into profiles: reported and selected.
    fn validate(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let sketch = &feature.sketch;
        let ids: HashSet<Uuid> = sketch.geometry.iter().map(GeometryElement::id).collect();
        let referenced: HashSet<Uuid> = sketch
            .geometry
            .iter()
            .flat_map(|g| match g {
                GeometryElement::Point(_) => Vec::new(),
                GeometryElement::Line(l) => vec![l.start, l.end],
                GeometryElement::Arc(a) => vec![a.center, a.start, a.end],
                GeometryElement::Circle(c) => vec![c.center],
                curve @ (GeometryElement::Ellipse(_)
                | GeometryElement::BSpline(_)
                | GeometryElement::Conic(_)) => Sketch::curve_point_ids(curve),
            })
            .collect();
        self.selected.clear();
        self.selected_constraints.clear();
        let mut problems = Vec::new();
        let stray: Vec<Uuid> = sketch
            .geometry
            .iter()
            .filter_map(|g| match g {
                GeometryElement::Point(p) if !referenced.contains(&p.id) => Some(p.id),
                _ => None,
            })
            .collect();
        if !stray.is_empty() {
            problems.push(format!("{} stray point(s)", stray.len()));
            self.selected.extend(stray);
        }
        let malformed: Vec<Uuid> = sketch
            .constraints
            .iter()
            .filter(|c| {
                sketch::constraint_refs(&c.kind)
                    .iter()
                    .any(|r| !ids.contains(r))
            })
            .map(|c| c.id)
            .collect();
        if !malformed.is_empty() {
            problems.push(format!(
                "{} constraint(s) referencing missing geometry",
                malformed.len()
            ));
            self.selected_constraints.extend(malformed);
        }
        match profile::extract_wires(sketch) {
            Ok(wires) => {
                problems.push(format!("{} closed profile(s)", wires.len()));
                let loose = profile::loose_ends(sketch);
                if !loose.is_empty() {
                    problems.push(format!(
                        "{} loose end(s) left out of the profile",
                        loose.len()
                    ));
                    self.selected.extend(loose);
                }
            }
            Err(profile::ProfileError::Empty) => problems.push("no closed profile".to_string()),
            Err(profile::ProfileError::OpenAt(id)) => {
                problems.push("an open profile".to_string());
                self.selected.insert(id);
            }
            Err(profile::ProfileError::BranchingAt(id)) => {
                problems.push("a branching profile".to_string());
                self.selected.insert(id);
            }
            Err(profile::ProfileError::MissingPoint(id)) => {
                problems.push("a curve missing its point".to_string());
                self.selected.insert(id);
            }
            Err(profile::ProfileError::Unsolved) => {
                problems.push("conflicting constraints, so no profile".to_string());
            }
        }
        ctx.log_info(format!("Sketch check: {}", problems.join(", ")));
        InputResult::consumed()
    }

    /// Measure how thin the edited sketch's profile gets: logged, drawn
    /// over the sketch and recorded as the command it is.
    fn check_walls(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(id) = self.active_sketch_id else {
            return InputResult::ignored();
        };
        // The host drops the action once it is handled, leaving no tool:
        // the one before is put away now, so the check outlives the next
        // event.
        if self.activated_tool.take().is_some() {
            self.tool_state = ToolState::Idle;
            self.dim_capture.clear_buffers();
            self.dim_capture.sync(&self.tool_state);
            self.flush_draw_record(ctx);
        }
        let minimum = f64::from(self.options.min_wall);
        match walls::check(ctx, id, minimum) {
            Ok(check) => {
                let summary = check.summary();
                if check.thinnest().is_some_and(|(wall, _)| wall < minimum) {
                    ctx.log_warn(summary);
                } else {
                    ctx.log_info(summary);
                }
                ctx.record(
                    "sketch.wall_thickness",
                    commands::args(serde_json::json!({
                        "sketch": id.0.to_string(),
                        "minimum": minimum,
                    })),
                    check.report(),
                );
                self.wall_check = Some(check);
            }
            Err(err) => {
                self.wall_check = None;
                ctx.log_warn(format!("Cannot check the wall thickness: {err}"));
            }
        }
        InputResult::consumed()
    }

    /// `sketch.wall_thickness`: the check's report, and on the sketch being
    /// edited its drawing too.
    fn wall_thickness_command(
        &mut self,
        args: &core_document::CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> core_document::CommandResult {
        let a = core_document::Args(args);
        let sketch = FeatureId(a.id("sketch")?);
        let minimum = match a.opt_number("minimum")? {
            Some(m) if m > 0.0 => m,
            Some(_) => {
                return Err(core_document::CommandError::bad(
                    "minimum",
                    "must be more than zero",
                ));
            }
            None => f64::from(self.options.min_wall),
        };
        if stored_sketch(ctx.document, sketch).is_none() {
            return Err(core_document::CommandError::bad(
                "sketch",
                "is not a sketch of this document",
            ));
        }
        let check =
            walls::check(ctx, sketch, minimum).map_err(core_document::CommandError::failed)?;
        let report = check.report();
        if self.active_sketch_id == Some(sketch) {
            self.wall_check = Some(check);
        }
        Ok(report)
    }

    /// The check on the sketch being edited.
    fn shown_wall_check(&self) -> Option<&walls::WallCheck> {
        self.wall_check
            .as_ref()
            .filter(|check| Some(check.sketch) == self.active_sketch_id)
    }

    /// Put the check away once its sketch is no longer edited, or once an
    /// edit has changed its profile.
    fn drop_stale_wall_check(&mut self, ctx: &WorkbenchRuntimeContext) {
        let Some(check) = &mut self.wall_check else {
            return;
        };
        if Some(check.sketch) != self.active_sketch_id {
            self.wall_check = None;
            return;
        }
        let seq = ctx.document.mutation_seq();
        if check.seen_seq == seq {
            return;
        }
        check.seen_seq = seq;
        let same = stored_sketch(ctx.document, check.sketch)
            .is_some_and(|f| profile::extract_wires(&f.sketch).is_ok_and(|w| w == check.wires));
        if !same {
            self.wall_check = None;
        }
    }

    /// Select the constraints on the selected elements (a curve's own
    /// points count as the curve), leaving the elements.
    fn select_constraints_of(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        self.selected_constraints = constraints_on(&feature.sketch, &self.selected);
        self.selected.clear();
        ctx.log_info(format!(
            "{} constraint(s) on the selection",
            self.selected_constraints.len()
        ));
        InputResult::consumed()
    }

    /// Select the elements the selected constraints hold, leaving the
    /// constraints.
    fn select_elements_of(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let elements = elements_of(&feature.sketch, &self.selected_constraints);
        self.selected_constraints.clear();
        self.selected.clear();
        self.selected.extend(elements);
        ctx.log_info(format!("{} element(s) selected", self.selected.len()));
        InputResult::consumed()
    }

    /// Select the constraints whose geometry is gone.
    fn select_malformed(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let ids: HashSet<Uuid> = feature
            .sketch
            .geometry
            .iter()
            .map(GeometryElement::id)
            .collect();
        self.selected.clear();
        self.selected_constraints.clear();
        for c in &feature.sketch.constraints {
            if sketch::constraint_refs(&c.kind)
                .iter()
                .any(|r| !ids.contains(r))
            {
                self.selected_constraints.insert(c.id);
            }
        }
        ctx.log_info(format!(
            "{} malformed constraint(s)",
            self.selected_constraints.len()
        ));
        InputResult::consumed()
    }

    /// Select what the constraints still let move: the free points, and,
    /// asked for the under-constrained geometry, the curves they belong to.
    fn select_free(&mut self, ctx: &mut WorkbenchRuntimeContext, curves_too: bool) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let free = solver::free_points(&feature.sketch);
        self.selected.clear();
        self.selected_constraints.clear();
        self.selected.extend(free.iter().copied());
        if curves_too {
            for g in &feature.sketch.geometry {
                let refs: Vec<Uuid> = match g {
                    GeometryElement::Point(_) => Vec::new(),
                    GeometryElement::Line(l) => vec![l.start, l.end],
                    GeometryElement::Arc(a) => vec![a.center, a.start, a.end],
                    GeometryElement::Circle(c) => vec![c.center],
                    curve @ (GeometryElement::Ellipse(_)
                    | GeometryElement::BSpline(_)
                    | GeometryElement::Conic(_)) => Sketch::curve_point_ids(curve),
                };
                if refs.iter().any(|r| free.contains(r)) {
                    self.selected.insert(g.id());
                }
            }
        }
        ctx.log_info(format!(
            "{} element(s) with a degree of freedom",
            self.selected.len()
        ));
        InputResult::consumed()
    }

    /// Turn the sketch plane over, or a quarter turn about its normal.
    fn reorient(&mut self, ctx: &mut WorkbenchRuntimeContext, rotate: bool) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let plane = feature.plane;
        let neg = |v: [f32; 3]| [-v[0], -v[1], -v[2]];
        let new_plane = if rotate {
            SketchPlane::from_frame(plane.origin, plane.normal, plane.y_axis)
        } else {
            SketchPlane::from_frame(plane.origin, neg(plane.normal), plane.x_axis)
        };
        self.set_plane(ctx, &mut feature, new_plane);
        ctx.log_info(if rotate {
            "Sketch plane rotated a quarter turn"
        } else {
            "Sketch plane flipped"
        });
        InputResult::consumed()
    }

    /// Move the sketch onto the face picked in the viewport.
    fn attach_to_face(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let body = self
            .active_sketch_id
            .and_then(|id| ctx.document.get_feature_meta(id))
            .and_then(|n| n.body);
        // Only a face of the sketch's own body is followed.
        let own = body.filter(|b| ctx.selected_body_id == Some(b.0));
        let face = match body {
            Some(body) => ctx.selected_face_in(body),
            None => ctx.selected_face,
        };
        let Some(face) = face.map(|f| f.on_its_plane()) else {
            ctx.log_warn("Click a face of a solid first");
            return InputResult::consumed();
        };
        let plane = SketchPlane::from_face(face.point, face.normal);
        // On a face of its own body the sketch follows the face; the face
        // replaces any datum it stood on.
        feature.support = None;
        feature.face = own.map(|_| crate::feature::FaceSupport::on(&face, plane));
        self.set_plane(ctx, &mut feature, plane);
        ctx.log_info("Sketch attached to the picked face");
        InputResult::consumed()
    }

    fn set_plane(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        feature: &mut SketchFeature,
        plane: SketchPlane,
    ) {
        feature.plane = plane;
        feature.sketch.plane = plane;
        // A sketch on a face keeps following it from where the face is now,
        // on the plane just set.
        if let Some(face) = feature.face.as_mut() {
            let now = self
                .active_sketch_id
                .and_then(|id| ctx.document.probed_references(id))
                .and_then(|probed| probed.answers.first())
                .and_then(|answer| match answer {
                    Ok(kernel_api::ProbeAnswer::Face { point, normal, .. }) => {
                        Some((point.map(|c| c as f32), normal.map(|c| c as f32)))
                    }
                    _ => None,
                });
            if let Some((point, normal)) = now {
                face.point = point;
                face.normal = normal;
            }
            face.placed = plane;
        }
        self.store_sketch(ctx, feature.clone());
        if let Some(id) = self.active_sketch_id
            && let Some(stored) = stored_sketch(ctx.document, id)
        {
            ctx.record(
                "sketch.set_plane",
                commands::args(serde_json::json!({
                    "sketch": id.0.to_string(),
                    "normal": stored.plane.normal,
                    "origin": stored.plane.origin,
                    "x_axis": stored.plane.x_axis,
                })),
                serde_json::Value::Null,
            );
        }
        ctx.request(HostRequest::OrientCamera(
            core_document::CameraOrientRequest {
                plane_origin: plane.origin,
                plane_normal: plane.normal,
                plane_up: plane.y_axis,
                centre: None,
            },
        ));
    }

    /// The selection repeated in a grid, as the panel's array settings say.
    fn array_selection(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let p = self.tool_params;
        let before = (
            feature
                .sketch
                .geometry
                .iter()
                .map(GeometryElement::id)
                .collect(),
            feature.sketch.constraints.iter().map(|c| c.id).collect(),
        );
        let effect = tools::array(
            &mut feature.sketch,
            &self.selected,
            p.array_rows,
            p.array_cols,
            p.array_dx,
            p.array_dy,
            p.copies_linked,
        );
        if effect.changed {
            if let Some(log) = effect.log {
                ctx.log_info(log);
            }
            if let Some(id) = self.active_sketch_id {
                let mut items: Vec<Uuid> = self.selected.iter().copied().collect();
                items.sort();
                ctx.record(
                    "sketch.array",
                    commands::args(serde_json::json!({
                        "sketch": id.0.to_string(),
                        "items": ids_json(&items),
                        "rows": p.array_rows,
                        "cols": p.array_cols,
                        "dx": p.array_dx,
                        "dy": p.array_dy,
                        "linked": p.copies_linked,
                    })),
                    commands::made_since(&feature.sketch, &before),
                );
            }
            self.solve(ctx, &mut feature);
            self.store_sketch(ctx, feature);
        } else {
            ctx.log_warn("Select geometry and give the array at least two rows or columns");
        }
        InputResult::consumed()
    }

    /// Merge the selected curves into one spline, recorded as `sketch.join`.
    fn join_selection(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let before = (
            feature
                .sketch
                .geometry
                .iter()
                .map(GeometryElement::id)
                .collect(),
            feature.sketch.constraints.iter().map(|c| c.id).collect(),
        );
        let effect = tools::join(&mut feature.sketch, &self.selected, tools::JOIN_TOLERANCE);
        if !effect.changed {
            if let Some(why) = effect.log {
                ctx.log_warn(why);
            }
            return InputResult::consumed();
        }
        if let Some(log) = effect.log {
            ctx.log_info(log);
        }
        if let Some(id) = self.active_sketch_id {
            let mut items: Vec<Uuid> = self.selected.iter().copied().collect();
            items.sort();
            ctx.record(
                "sketch.join",
                commands::args(serde_json::json!({
                    "sketch": id.0.to_string(),
                    "items": ids_json(&items),
                })),
                commands::made_since(&feature.sketch, &before),
            );
        }
        let alive: HashSet<Uuid> = feature
            .sketch
            .geometry
            .iter()
            .map(GeometryElement::id)
            .collect();
        self.selected.retain(|id| alive.contains(id));
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
        InputResult::consumed()
    }

    /// Apply `edit` to the edited sketch's selection and record it as
    /// `command`, its arguments `extra` with the sketch and the selection.
    fn selection_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        command: &str,
        extra: serde_json::Value,
        edit: impl FnOnce(&mut Sketch, &HashSet<Uuid>) -> tools::ToolEffect,
    ) -> InputResult {
        let mut items: Vec<Uuid> = self.selected.iter().copied().collect();
        items.sort();
        let mut args = extra;
        args["items"] = ids_json(&items);
        let selected: HashSet<Uuid> = items.into_iter().collect();
        self.sketch_edit(ctx, command, args, |s| edit(s, &selected))
    }

    /// Apply `edit` to the edited sketch and record it as `command` with
    /// `args` (the sketch's id added), solved and stored.
    pub(crate) fn sketch_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        command: &str,
        mut args: serde_json::Value,
        edit: impl FnOnce(&mut Sketch) -> tools::ToolEffect,
    ) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let before = (
            feature
                .sketch
                .geometry
                .iter()
                .map(GeometryElement::id)
                .collect(),
            feature.sketch.constraints.iter().map(|c| c.id).collect(),
        );
        let effect = edit(&mut feature.sketch);
        if !effect.changed {
            if let Some(why) = effect.log {
                ctx.log_warn(why);
            }
            return InputResult::consumed();
        }
        if let Some(log) = effect.log {
            ctx.log_info(log);
        }
        if let Some(id) = self.active_sketch_id {
            args["sketch"] = serde_json::json!(id.0.to_string());
            ctx.record(
                command,
                commands::args(args),
                commands::made_since(&feature.sketch, &before),
            );
        }
        let alive: HashSet<Uuid> = feature
            .sketch
            .geometry
            .iter()
            .map(GeometryElement::id)
            .collect();
        self.selected.retain(|id| alive.contains(id));
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
        InputResult::consumed()
    }

    /// Project every edge picked since the external geometry tool was
    /// armed into the edited sketch; a face picked brings every edge
    /// around it.
    fn take_external_picks(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let fresh: Vec<core_document::EdgeRef> = ctx
            .selected_edges
            .iter()
            .filter(|e| {
                self.external_seen
                    .insert((e.body, e.point.map(f32::to_bits)))
            })
            .copied()
            .collect();
        // As the drawing tools do: a guide in construction mode, counting
        // in the profile out of it.
        let defining = !self.construction_mode;
        let mut sources: Vec<sketch::ExternalSource> = fresh
            .iter()
            .map(|edge| {
                let local = edge.moved(&ctx.document.body_placement(BodyId(edge.body)).inverse());
                sketch::ExternalSource {
                    body: edge.body,
                    point: local.point,
                    direction: local.direction,
                    section: false,
                    defining,
                    reference: None,
                }
            })
            .collect();
        if ctx.selected_edges.is_empty() {
            sources.extend(self.face_outline_sources(ctx, defining));
        }
        if sources.is_empty() {
            return;
        }
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return;
        };
        let before = (
            feature
                .sketch
                .geometry
                .iter()
                .map(GeometryElement::id)
                .collect(),
            feature.sketch.constraints.iter().map(|c| c.id).collect(),
        );
        let added = match commands::add_external(ctx, &feature.plane, &mut feature.sketch, &sources)
        {
            Ok(added) => added,
            Err(why) => {
                ctx.log_warn(format!("Could not bring that edge in: {why}"));
                0
            }
        };
        if added > 0
            && let Some(id) = self.active_sketch_id
        {
            let edges: Vec<serde_json::Value> = sources
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "body": s.body.to_string(),
                        "point": s.point,
                        "direction": s.direction,
                    })
                })
                .collect();
            ctx.record(
                "sketch.external",
                commands::args(serde_json::json!({
                    "sketch": id.0.to_string(),
                    "edges": edges,
                    "counts": !self.construction_mode,
                })),
                commands::made_since(&feature.sketch, &before),
            );
        }
        if added > 0 {
            self.solve(ctx, &mut feature);
            self.store_sketch(ctx, feature);
            ctx.log_info(match added {
                1 => "Added 1 external element".to_string(),
                n => format!("Added {n} external elements"),
            });
        }
    }

    /// The edges around the face picked on a solid, as projection sources
    /// in its body's frame, once per face picked.
    fn face_outline_sources(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        defining: bool,
    ) -> Vec<sketch::ExternalSource> {
        let (Some(face), Some(body)) = (ctx.selected_face, ctx.selected_body_id) else {
            return Vec::new();
        };
        if !self
            .external_seen
            .insert((body, face.point.map(f32::to_bits)))
        {
            return Vec::new();
        }
        let (Some(kernel), Some(brep)) =
            (ctx.kernel, ctx.document.imported_brep_blob(BodyId(body)))
        else {
            return Vec::new();
        };
        let local = face.moved(&ctx.document.body_placement(BodyId(body)).inverse());
        match kernel.face_edges(brep, local.point.map(f64::from)) {
            Ok(edges) => edges
                .into_iter()
                .map(|(point, direction)| sketch::ExternalSource {
                    body,
                    point: point.map(|v| v as f32),
                    direction: direction.map(|v| v as f32),
                    section: false,
                    defining,
                    reference: None,
                })
                .collect(),
            Err(why) => {
                ctx.log_warn(format!("The face's edges could not be read: {why}"));
                Vec::new()
            }
        }
    }

    /// Add where every face picked since the intersection tool was armed
    /// crosses the edited sketch's plane.
    fn take_intersection_picks(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let (Some(face), Some(body)) = (ctx.selected_face, ctx.selected_body_id) else {
            return;
        };
        if !self
            .external_seen
            .insert((body, face.point.map(f32::to_bits)))
        {
            return;
        }
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return;
        };
        let local = face.moved(&ctx.document.body_placement(BodyId(body)).inverse());
        let source = sketch::ExternalSource {
            body,
            point: local.point,
            direction: local.normal,
            section: true,
            defining: !self.construction_mode,
            reference: None,
        };
        let before = commands::ids_of(&feature.sketch);
        let added =
            match commands::add_external(ctx, &feature.plane, &mut feature.sketch, &[source]) {
                Ok(added) => added,
                Err(why) => {
                    ctx.log_warn(format!("No intersection with that face: {why}"));
                    return;
                }
            };
        if let Some(id) = self.active_sketch_id {
            ctx.record(
                "sketch.intersection",
                commands::args(serde_json::json!({
                    "sketch": id.0.to_string(),
                    "faces": [{
                        "body": body.to_string(),
                        "point": source.point,
                        "normal": source.direction,
                    }],
                    "counts": !self.construction_mode,
                })),
                commands::made_since(&feature.sketch, &before),
            );
        }
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
        ctx.log_info(match added {
            1 => "Added 1 intersection element".to_string(),
            n => format!("Added {n} intersection elements"),
        });
    }

    /// Bring the edited sketch's external geometry up to the solids it came
    /// from: each edge projected again and each face cut again, moved in
    /// place where the curves are the same kinds. An edge or face that no
    /// longer exists is left as it was.
    fn refresh_external(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return;
        };
        let groups = external::groups(&feature.sketch);
        if groups.is_empty() {
            return;
        }
        let before = feature.sketch.clone();
        let mut lost = 0;
        for (source, group) in groups {
            // A solid's edge needs the kernel; another sketch or a datum
            // does not.
            if source.reference.is_none() && ctx.kernel.is_none() {
                continue;
            }
            match project_source(ctx, &feature.plane, &source) {
                Ok(projected) => {
                    external::refresh_group(&mut feature.sketch, source, &group, &projected);
                }
                Err(_) => lost += 1,
            }
        }
        if lost > 0 {
            ctx.log_warn(format!(
                "{lost} external element(s) kept where they were: their edges or faces could not be reached"
            ));
        }
        // Nothing moved: no edit to record.
        if serde_json::to_value(&before).ok() != serde_json::to_value(&feature.sketch).ok() {
            self.solve(ctx, &mut feature);
            self.store_sketch(ctx, feature);
        }
    }

    /// Open the panel's list of other sketches for carbon copy or merge; the
    /// same action again closes it.
    fn open_sketch_picker(&mut self, mode: SketchPickerMode) -> InputResult {
        self.sketch_picker = match &self.sketch_picker {
            Some(picker) if picker.mode == mode => None,
            _ => Some(SketchPicker {
                mode,
                checked: HashSet::new(),
            }),
        };
        InputResult::consumed()
    }

    /// Every sketch in the document but the one being edited, in history
    /// order: its id, name, and content.
    pub(crate) fn other_sketches(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> Vec<(FeatureId, String, SketchFeature)> {
        let mut out: Vec<(u64, FeatureId, String, SketchFeature)> = ctx
            .document
            .feature_tree()
            .all_nodes()
            .filter(|(id, node)| {
                node.workbench_id.as_str() == "wb.sketch" && Some(**id) != self.active_sketch_id
            })
            .filter_map(|(id, node)| {
                // Seen where its body sits, as the edited sketch is.
                let mut feature =
                    SketchFeature::from_json(ctx.document.feature_values(*id)?).ok()?;
                let placement = sketch_placement(ctx.document, *id);
                feature.plane = placed_plane(&feature.plane, &placement);
                Some((node.seq, *id, node.name.clone(), feature))
            })
            .collect();
        out.sort_by_key(|(seq, id, _, _)| (*seq, *id));
        out.into_iter()
            .map(|(_, id, name, feature)| (id, name, feature))
            .collect()
    }

    /// Copy `source`'s geometry into the edited sketch, mapped from its plane
    /// onto this one; its constraints come along where the planes share
    /// their axes.
    pub(crate) fn carbon_copy(&mut self, ctx: &mut WorkbenchRuntimeContext, source: FeatureId) {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return;
        };
        let (Some(target), Some(name)) = (
            self.active_sketch_id,
            ctx.document
                .get_feature_meta(source)
                .map(|n| n.name.clone()),
        ) else {
            return;
        };
        let before = (
            feature
                .sketch
                .geometry
                .iter()
                .map(GeometryElement::id)
                .collect(),
            feature.sketch.constraints.iter().map(|c| c.id).collect(),
        );
        let (count, constraints, xf) =
            match commands::carbon_copy(ctx.document, target, &mut feature.sketch, source) {
                Ok(done) => done,
                Err(why) => {
                    ctx.log_warn(format!("Carbon copy of {name}: {why}"));
                    return;
                }
            };
        ctx.record(
            "sketch.carbon_copy",
            commands::args(serde_json::json!({
                "sketch": target.0.to_string(),
                "from": source.0.to_string(),
            })),
            commands::made_since(&feature.sketch, &before),
        );
        self.sketch_picker = None;
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
        ctx.log_info(carbon_copy_log(&name, count, constraints, &xf));
    }

    /// A new sketch on the edited one's plane holding its geometry and the
    /// ticked sketches', each mapped onto the plane; the originals stay.
    pub(crate) fn merge_sketches(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(picker) = self.sketch_picker.take() else {
            return;
        };
        let Some(active) = self.active_sketch_id else {
            return;
        };
        // In history order, as the list shows them.
        let with: Vec<FeatureId> = self
            .other_sketches(ctx)
            .into_iter()
            .map(|(id, _, _)| id)
            .filter(|id| picker.checked.contains(id))
            .collect();
        match commands::merge(ctx.document, active, &with) {
            Ok(made) => {
                let name = ctx
                    .document
                    .get_feature_meta(made)
                    .map(|n| n.name.clone())
                    .unwrap_or_default();
                ctx.log_info(format!("Created {name}"));
                ctx.record(
                    "sketch.merge",
                    commands::args(serde_json::json!({
                        "sketch": active.0.to_string(),
                        "with": with.iter().map(|id| id.0.to_string()).collect::<Vec<_>>(),
                    })),
                    serde_json::json!(made.0.to_string()),
                );
            }
            Err(err) => ctx.log_warn(format!("Could not merge: {err}")),
        }
    }

    /// A new sketch on the same plane and body: this one's geometry
    /// mirrored across the sketch's Y axis.
    fn mirror_sketch(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(id) = self.active_sketch_id else {
            return InputResult::ignored();
        };
        match commands::mirror_sketch(ctx.document, id) {
            Ok(made) => {
                let name = ctx
                    .document
                    .get_feature_meta(made)
                    .map(|n| n.name.clone())
                    .unwrap_or_default();
                ctx.log_info(format!("Created {name}"));
                ctx.record(
                    "sketch.mirror_sketch",
                    commands::args(serde_json::json!({"sketch": id.0.to_string()})),
                    serde_json::json!(made.0.to_string()),
                );
            }
            Err(err) => ctx.log_error(format!("Could not create the mirrored sketch: {err}")),
        }
        InputResult::consumed()
    }

    /// Copy the selection to the clipboard; cut removes it too.
    fn clipboard_copy(&mut self, ctx: &mut WorkbenchRuntimeContext, cut: bool) -> bool {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return false;
        };
        if self.selected.is_empty() {
            ctx.log_warn("Nothing selected to copy");
            return true;
        }
        let mut clip = Sketch::new("clipboard");
        let count = tools::copy_from(
            &feature.sketch,
            &mut clip,
            &self.selected,
            &tools::Similarity::translation(glam::Vec2::ZERO),
        );
        self.clipboard = Some(clip);
        if cut {
            let mut ids: Vec<Uuid> = self.selected.iter().copied().collect();
            ids.sort();
            commands::delete_items(&mut feature.sketch, &ids);
            if let Some(id) = self.active_sketch_id {
                ctx.record(
                    "sketch.delete",
                    commands::args(serde_json::json!({
                        "sketch": id.0.to_string(),
                        "items": ids_json(&ids),
                    })),
                    serde_json::Value::Null,
                );
            }
            self.selected.clear();
            self.solve(ctx, &mut feature);
            self.store_sketch(ctx, feature);
            ctx.log_info(format!("Cut {count} element(s)"));
        } else {
            ctx.log_info(format!("Copied {count} element(s)"));
        }
        true
    }

    /// Paste the clipboard at the cursor, or a little off where it was
    /// copied when the cursor is off the plane. The pasted elements
    /// become the selection.
    fn clipboard_paste(&mut self, ctx: &mut WorkbenchRuntimeContext) -> bool {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return false;
        };
        let Some(clip) = self.clipboard.clone() else {
            ctx.log_warn("The clipboard is empty");
            return true;
        };
        let delta = match self.cursor {
            // The middle of what was copied lands under the cursor.
            Some(cursor) => {
                let (lo, hi) = clip.geometry.iter().fold(
                    (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN)),
                    |(lo, hi), g| match g {
                        GeometryElement::Point(p) => {
                            (lo.min(p.position.to_glam()), hi.max(p.position.to_glam()))
                        }
                        _ => (lo, hi),
                    },
                );
                let anchor = if lo.x <= hi.x {
                    (lo + hi) * 0.5
                } else {
                    glam::Vec2::ZERO
                };
                cursor.to_glam() - anchor
            }
            None => glam::Vec2::new(5.0, 5.0),
        };
        let before: HashSet<Uuid> = feature
            .sketch
            .geometry
            .iter()
            .map(GeometryElement::id)
            .collect();
        let constraints_before: HashSet<Uuid> =
            feature.sketch.constraints.iter().map(|c| c.id).collect();
        let by = Vec2D::new(delta.x, delta.y);
        let count = commands::paste(&mut feature.sketch, &clip, by);
        if let Some(id) = self.active_sketch_id {
            // What the paste made, its constraints among it, so a replay
            // can name them.
            let made = commands::made_since(&feature.sketch, &(before.clone(), constraints_before));
            ctx.record(
                "sketch.paste",
                commands::args(serde_json::json!({
                    "sketch": id.0.to_string(),
                    "clipboard": serde_json::to_value(&clip).unwrap_or_default(),
                    "by": [by.x, by.y],
                })),
                made,
            );
        }
        self.selected = feature
            .sketch
            .geometry
            .iter()
            .map(GeometryElement::id)
            .filter(|id| !before.contains(id))
            .collect();
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
        ctx.log_info(format!("Pasted {count} element(s)"));
        true
    }

    /// Select the conflicting (or redundant) constraints of the last
    /// diagnosis and the geometry they reference.
    fn select_offenders(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        conflicting: bool,
    ) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let diag = match &self.last_diagnosis {
            Some(d) => d.clone(),
            None => {
                let d = solver::diagnose(&feature.sketch);
                self.last_diagnosis = Some(d.clone());
                d
            }
        };
        let offenders = if conflicting {
            &diag.conflicting
        } else {
            &diag.redundant
        };
        self.selected.clear();
        self.selected_constraints.clear();
        for c in &feature.sketch.constraints {
            if offenders.contains(&c.id) {
                self.selected_constraints.insert(c.id);
                self.selected.extend(sketch::constraint_refs(&c.kind));
            }
        }
        InputResult::consumed()
    }

    /// Remove every element (and the constraints on them), or every
    /// constraint, then re-solve.
    fn delete_all(&mut self, ctx: &mut WorkbenchRuntimeContext, geometry: bool) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let ids: Vec<Uuid> = if geometry {
            feature
                .sketch
                .geometry
                .iter()
                .map(GeometryElement::id)
                .collect()
        } else {
            feature.sketch.constraints.iter().map(|c| c.id).collect()
        };
        let removed = commands::delete_items(&mut feature.sketch, &ids);
        ctx.log_info(format!("Deleted {removed} item(s)"));
        if let (Some(id), false) = (self.active_sketch_id, ids.is_empty()) {
            ctx.record(
                "sketch.delete",
                commands::args(serde_json::json!({
                    "sketch": id.0.to_string(),
                    "items": ids_json(&ids),
                })),
                serde_json::Value::Null,
            );
        }
        self.selected.clear();
        self.selected_constraints.clear();
        self.hovered = None;
        self.tool_state = ToolState::Idle;
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
        InputResult::consumed()
    }
}

/// The constraints on `elements` (a curve's own points count as the curve).
fn constraints_on(sketch: &Sketch, elements: &HashSet<Uuid>) -> HashSet<Uuid> {
    let mut touched = elements.clone();
    for id in elements {
        if let Some(geom) = sketch.get_geometry(*id) {
            touched.extend(Sketch::curve_point_ids(geom));
        }
    }
    sketch
        .constraints
        .iter()
        .filter(|c| {
            sketch::constraint_refs(&c.kind)
                .iter()
                .any(|r| touched.contains(r))
        })
        .map(|c| c.id)
        .collect()
}

/// The elements `constraints` hold.
fn elements_of(sketch: &Sketch, constraints: &HashSet<Uuid>) -> Vec<Uuid> {
    sketch
        .constraints
        .iter()
        .filter(|c| constraints.contains(&c.id))
        .flat_map(|c| sketch::constraint_refs(&c.kind))
        .collect()
}

/// The dimensional constraint tool a selection takes, if any.
pub(crate) fn dimension_for(shape: &constrain::SelectionShape) -> Option<&'static str> {
    constrain::dimension_for(shape)
}

/// The tools that move, turn, scale or mirror the selection.
pub(crate) fn is_transform_tool(tool: &str) -> bool {
    matches!(
        tool,
        "sketch.translate" | "sketch.rotate" | "sketch.scale" | "sketch.mirror"
    )
}

/// The arguments of `sketch.text` (with `at`) or `sketch.text_edit` that
/// make `spec`.
pub(crate) fn text_args(spec: &text::TextSpec, at: Option<Vec2D>) -> serde_json::Value {
    let mut args = serde_json::json!({
        "text": spec.text,
        "font": spec.font,
        "size": spec.size,
        "spacing": spec.spacing,
        "angle": spec.angle,
    });
    if let Some(at) = at {
        args["at"] = serde_json::json!([at.x, at.y]);
    }
    args
}

/// Tools that create geometry from clicks, for which object snapping can
/// be switched off.
pub(crate) fn is_draw_tool(tool: &str) -> bool {
    matches!(
        tool,
        "sketch.point"
            | "sketch.line"
            | "sketch.polyline"
            | "sketch.arc"
            | "sketch.arc3"
            | "sketch.circle"
            | "sketch.circle3"
            | "sketch.ellipse"
            | "sketch.ellipse3"
            | "sketch.ellipse_arc"
            | "sketch.parabola"
            | "sketch.hyperbola"
            | "sketch.bspline"
            | "sketch.rect"
            | "sketch.rect_center"
            | "sketch.rect3"
            | "sketch.rect_center3"
            | "sketch.rect_frame"
            | "sketch.polygon"
            | "sketch.slot"
            | "sketch.arc_slot"
            | "sketch.text"
    )
}

fn point_in_rect(p: Vec2D, min: Vec2D, max: Vec2D) -> bool {
    p.x >= min.x && p.x <= max.x && p.y >= min.y && p.y <= max.y
}

/// Whether `geom` lies FULLY inside the axis-aligned rectangle `min`..`max`
/// (box-selection containment): a point by its position; a line by both
/// endpoints; an arc by its center, endpoints and angular midpoint; a
/// circle by its center±radius bounding box. Unresolvable references are
/// never inside.
fn element_fully_inside(sketch: &Sketch, geom: &GeometryElement, min: Vec2D, max: Vec2D) -> bool {
    let inside = |p: Vec2D| point_in_rect(p, min, max);
    match geom {
        GeometryElement::Point(p) => inside(p.position),
        GeometryElement::Line(l) => {
            match (sketch.point_position(l.start), sketch.point_position(l.end)) {
                (Some(a), Some(b)) => inside(a) && inside(b),
                _ => false,
            }
        }
        GeometryElement::Arc(a) => {
            let (Some(c), Some(s), Some(e)) = (
                sketch.point_position(a.center),
                sketch.point_position(a.start),
                sketch.point_position(a.end),
            ) else {
                return false;
            };
            let sv = (s - c).to_glam();
            let radius = sv.length();
            let (start_angle, sweep) = snap::arc_angles(sv, (e - c).to_glam());
            let mid_angle = start_angle + sweep * 0.5;
            let mid = Vec2D::new(
                c.x + radius * mid_angle.cos(),
                c.y + radius * mid_angle.sin(),
            );
            inside(c) && inside(s) && inside(e) && inside(mid)
        }
        GeometryElement::Circle(circle) => match sketch.point_position(circle.center) {
            Some(c) => {
                inside(Vec2D::new(c.x - circle.radius, c.y - circle.radius))
                    && inside(Vec2D::new(c.x + circle.radius, c.y + circle.radius))
            }
            None => false,
        },
        // Sampled boundary points all inside is exact enough for selection.
        GeometryElement::Ellipse(e) => e
            .points(sketch, 32)
            .is_some_and(|points| points.into_iter().all(inside)),
        GeometryElement::Conic(c) => c
            .points(sketch, 32)
            .is_some_and(|points| points.into_iter().all(inside)),
        // The spline lies in its control polygon's convex hull, so all
        // control points inside implies the curve is inside; the points it
        // is drawn through lie on it.
        GeometryElement::BSpline(b) => b
            .point_ids()
            .iter()
            .map(|id| sketch.point_position(*id))
            .all(|p| p.is_some_and(inside)),
    }
}

fn parse_sketch_index(name: &str) -> Option<u32> {
    let lower = name.to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("sketch_") {
        r
    } else {
        lower.strip_prefix("sketch")?
    };

    let trimmed = rest.trim_start_matches(&['_', '.', ' '][..]);
    if trimmed.is_empty() {
        Some(0)
    } else {
        trimmed.parse().ok()
    }
}

/// How far past the sketch plane a section view keeps, in mm: a face the
/// sketch lies on stays whole.
const SECTION_MARGIN_MM: f32 = 1e-3;

/// The renderer's clipping equation for a section view at `plane`: it
/// keeps what lies on the far side from `eye`, and the plane itself.
pub(crate) fn section_equation(plane: &SketchPlane, eye: [f32; 3]) -> [f32; 4] {
    let n = glam::Vec3::from_array(plane.normal).normalize_or_zero();
    let o = glam::Vec3::from_array(plane.origin);
    // The side the viewer is on is the side cut away.
    let toward_eye = if n.dot(glam::Vec3::from_array(eye) - o) >= 0.0 {
        n
    } else {
        -n
    };
    let keep = -toward_eye;
    [keep.x, keep.y, keep.z, -keep.dot(o) + SECTION_MARGIN_MM]
}

/// What `source` comes to on the sketch plane `plane` (as the scene has
/// it), in the sketch's own coordinates: its edge projected onto the plane,
/// or the curves where its face crosses it.
pub(crate) fn project_source(
    ctx: &WorkbenchRuntimeContext,
    plane: &SketchPlane,
    source: &sketch::ExternalSource,
) -> Result<Vec<kernel_api::ProjectedEdge>, String> {
    if let Some(reference) = source.reference {
        return external_ref::project(ctx.document, plane, reference);
    }
    let kernel = ctx.kernel.ok_or("no kernel to project with")?;
    let body = BodyId(source.body);
    let brep = ctx
        .document
        .imported_brep_blob(body)
        .ok_or("that body has no solid shape")?;
    // The kernel works in the body's own frame: the plane goes there too,
    // and the projection keeps the sketch's own coordinates.
    let local = placed_plane(plane, &ctx.document.body_placement(body).inverse());
    let f = |v: [f32; 3]| v.map(f64::from);
    let plane = kernel_api::ProfilePlane {
        origin: f(local.origin),
        x_axis: f(local.x_axis),
        y_axis: f(local.y_axis),
        normal: f(local.normal),
    };
    if source.section {
        let curves = kernel
            .section_face(brep, f(source.point), &plane)
            .map_err(|e| e.to_string())?;
        if curves.is_empty() {
            return Err("the face does not cross the sketch plane".to_string());
        }
        return Ok(curves);
    }
    kernel
        .project_edge(brep, f(source.point), &plane)
        .map(|edge| vec![edge])
        .map_err(|e| e.to_string())
}

/// Move the sketch's geometry from other sketches' elements and datums to
/// where they stand in `values_of` (the working data of this evaluation),
/// in place: an element that is now another kind of curve, or one gone,
/// waits for the sketch to be edited. Answers whether anything moved.
fn follow_references(
    feature: &mut SketchFeature,
    id: FeatureId,
    values_of: &dyn Fn(FeatureId) -> Option<serde_json::Value>,
    document: &core_document::Document,
) -> bool {
    let groups: Vec<_> = external::groups(&feature.sketch)
        .into_iter()
        .filter(|(source, _)| source.reference.is_some())
        .collect();
    if groups.is_empty() {
        return false;
    }
    // Another sketch as it builds: solved for what formulas gave it, once
    // however many of its elements come here.
    let solved: std::cell::RefCell<HashMap<FeatureId, Option<serde_json::Value>>> =
        Default::default();
    let settled = |other: FeatureId| -> Option<serde_json::Value> {
        if let Some(known) = solved.borrow().get(&other) {
            return known.clone();
        }
        let mut data = values_of(other);
        let is_sketch = document
            .get_feature_meta(other)
            .is_some_and(|n| n.workbench_id.as_str() == "wb.sketch");
        if is_sketch && let Some(data) = data.as_mut() {
            params::settle(data);
        }
        solved.borrow_mut().insert(other, data.clone());
        data
    };
    let placed = placed_plane(&feature.plane, &sketch_placement(document, id));
    let before = serde_json::to_value(&feature.sketch.geometry).ok();
    for (source, group) in groups {
        let Some(reference) = source.reference else {
            continue;
        };
        let Ok(projected) = external_ref::project_with(document, &settled, &placed, reference)
        else {
            continue;
        };
        let kept = feature.sketch.geometry.clone();
        if !external::refresh_in_place(&mut feature.sketch, &group, &projected) {
            feature.sketch.geometry = kept;
        }
    }
    serde_json::to_value(&feature.sketch.geometry).ok() != before
}

/// A sketch as the document has it, its plane in its body's frame: its
/// formulas' values in, and solved for them.
pub(crate) fn stored_sketch(
    document: &core_document::Document,
    id: FeatureId,
) -> Option<SketchFeature> {
    document
        .feature_values(id)
        .and_then(|data| SketchFeature::from_json(data).ok())
}

/// Where the body a sketch belongs to sits.
pub(crate) fn sketch_placement(
    document: &core_document::Document,
    id: FeatureId,
) -> core_document::BodyPlacement {
    document
        .get_feature_meta(id)
        .and_then(|node| node.body)
        .map(|body| document.body_placement(body))
        .unwrap_or_default()
}

/// A plane moved by a body's placement.
pub(crate) fn placed_plane(
    plane: &SketchPlane,
    placement: &core_document::BodyPlacement,
) -> SketchPlane {
    SketchPlane {
        origin: placement.point(plane.origin),
        normal: placement.direction(plane.normal),
        x_axis: placement.direction(plane.x_axis),
        y_axis: placement.direction(plane.y_axis),
    }
}

/// A plane seen where the body sits, back in the body's frame: `stored`
/// when it is where `stored` is seen, else moved back.
fn local_plane(
    seen: &SketchPlane,
    stored: &SketchPlane,
    placement: &core_document::BodyPlacement,
) -> SketchPlane {
    let same = |a: [f32; 3], b: [f32; 3], tol: f32| (0..3).all(|k| (a[k] - b[k]).abs() <= tol);
    let placed = placed_plane(stored, placement);
    if same(placed.origin, seen.origin, 1e-4)
        && same(placed.normal, seen.normal, 1e-6)
        && same(placed.x_axis, seen.x_axis, 1e-6)
    {
        *stored
    } else {
        placed_plane(seen, &placement.inverse())
    }
}

/// The map from one sketch plane's coordinates onto another's, for planes
/// that face the same line: rotation, translation, and a mirror when their
/// normals oppose. A plane at an angle to the other has no such map (its
/// geometry would come out foreshortened), and is refused.
pub(crate) fn plane_map(
    from: &SketchPlane,
    onto: &SketchPlane,
) -> Result<tools::Similarity, &'static str> {
    use glam::Vec3;
    let v = |a: [f32; 3]| Vec3::from_array(a);
    if v(from.normal).cross(v(onto.normal)).length() > 1e-4 {
        return Err("its plane is not parallel to this one");
    }
    let (ox, oy) = (v(onto.x_axis), v(onto.y_axis));
    let m = glam::Mat2::from_cols(
        glam::Vec2::new(v(from.x_axis).dot(ox), v(from.x_axis).dot(oy)),
        glam::Vec2::new(v(from.y_axis).dot(ox), v(from.y_axis).dot(oy)),
    );
    let d = v(from.origin) - v(onto.origin);
    Ok(tools::Similarity::linear(
        m,
        glam::Vec2::new(d.dot(ox), d.dot(oy)),
    ))
}

/// Copy all of `source` into `target` under `xf`, and its constraints when
/// `xf` only moves (under a turn or a mirror, a constraint stated against
/// the sketch axes would no longer hold). Returns the elements and the
/// constraints copied.
pub(crate) fn copy_sketch_into(
    source: &Sketch,
    target: &mut Sketch,
    xf: &tools::Similarity,
) -> (usize, usize) {
    let all: HashSet<Uuid> = source.geometry.iter().map(GeometryElement::id).collect();
    let map = tools::copy_mapped(source, target, &all, xf);
    let constraints = if xf.is_translation() {
        tools::copy_constraints(source, target, &map, xf)
    } else {
        0
    };
    (map.len(), constraints)
}

pub(crate) fn carbon_copy_log(
    name: &str,
    count: usize,
    constraints: usize,
    xf: &tools::Similarity,
) -> String {
    if xf.is_translation() {
        format!("Carbon copy of {name}: {count} elements, {constraints} constraints")
    } else {
        format!(
            "Carbon copy of {name}: {count} elements; its constraints stay behind, as its \
             axes are turned against this sketch's"
        )
    }
}

#[cfg(test)]
mod constraint_filter {
    use super::*;
    use sketch::{ConstraintKind, Line, Point};

    #[test]
    fn from_elements_to_their_constraints_and_back() {
        let mut sketch = Sketch::new("t");
        let point = |sketch: &mut Sketch, x: f32| {
            sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 0.0))))
        };
        let (a, b, c) = (
            point(&mut sketch, 0.0),
            point(&mut sketch, 5.0),
            point(&mut sketch, 9.0),
        );
        let first = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let second = sketch.add_geometry(GeometryElement::Line(Line::new(b, c)));
        let level = sketch.add_constraint(ConstraintKind::Horizontal { element: second });
        let pinned = sketch.add_constraint(ConstraintKind::DistanceX {
            a,
            b: None,
            value: 0.0,
        });
        let _other = sketch.add_constraint(ConstraintKind::Vertical { element: first });
        // The first line's own point carries the distance: it counts.
        let on_first = constraints_on(&sketch, &HashSet::from([first]));
        assert!(on_first.contains(&pinned) && !on_first.contains(&level));
        assert_eq!(on_first.len(), 2, "its vertical and its point's distance");
        let held = elements_of(&sketch, &HashSet::from([level]));
        assert_eq!(held, [second]);
    }

    #[test]
    fn each_filter_lists_its_own() {
        let mut sketch = Sketch::new("t");
        let point = |sketch: &mut Sketch, x: f32| {
            sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 0.0))))
        };
        let (a, b, c) = (
            point(&mut sketch, 0.0),
            point(&mut sketch, 5.0),
            point(&mut sketch, 9.0),
        );
        let first = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let second = sketch.add_geometry(GeometryElement::Line(Line::new(b, c)));
        let level = sketch.add_constraint(ConstraintKind::Horizontal { element: second });
        let width = sketch.add_constraint(ConstraintKind::Length {
            line: first,
            length: 5.0,
        });
        let measured = sketch.add_constraint(ConstraintKind::Distance {
            point1: a,
            point2: c,
            distance: 9.0,
        });
        for constraint in &mut sketch.constraints {
            if constraint.id == width {
                constraint.name = Some("width".into());
            }
            if constraint.id == measured {
                constraint.driving = false;
                constraint.parked = true;
            }
        }
        let listed = |filter: ConstraintFilter, selected: &[Uuid], chosen: &[Uuid]| {
            let selected: HashSet<Uuid> = selected.iter().copied().collect();
            let chosen: HashSet<Uuid> = chosen.iter().copied().collect();
            sketch
                .constraints
                .iter()
                .filter(|c| filter.accepts(&sketch, c, &selected, &chosen))
                .map(|c| c.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(listed(ConstraintFilter::All, &[], &[]).len(), 3);
        assert_eq!(listed(ConstraintFilter::Geometric, &[], &[]), [level]);
        assert_eq!(
            listed(ConstraintFilter::Dimensional, &[], &[]),
            [width, measured]
        );
        assert_eq!(listed(ConstraintFilter::Named, &[], &[]), [width]);
        assert_eq!(listed(ConstraintFilter::Reference, &[], &[]), [measured]);
        assert_eq!(listed(ConstraintFilter::Parked, &[], &[]), [measured]);
        assert_eq!(listed(ConstraintFilter::Selected, &[], &[level]), [level]);
        // The first line reaches its length and, through its end at `a`,
        // the distance. The second line's level is not its own.
        assert_eq!(
            listed(ConstraintFilter::Related, &[first], &[]),
            [width, measured]
        );
        // A corner brings the constraints of the curves meeting there.
        assert_eq!(listed(ConstraintFilter::Related, &[b], &[]), [level, width]);
        assert_eq!(
            listed(ConstraintFilter::Related, &[c], &[]),
            [level, measured]
        );
    }

    /// A corner held by a Coincident constraint, not a shared point, is a
    /// corner too: selecting one side's end reaches the other side's
    /// constraints.
    #[test]
    fn related_reaches_across_a_coincident_corner() {
        let mut sketch = Sketch::new("s");
        let point = |sketch: &mut Sketch, x: f32, y: f32| {
            sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
        };
        let (a, b) = (point(&mut sketch, 0.0, 0.0), point(&mut sketch, 5.0, 0.0));
        let (c, d) = (point(&mut sketch, 5.0, 0.0), point(&mut sketch, 5.0, 4.0));
        let first = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let second = sketch.add_geometry(GeometryElement::Line(Line::new(c, d)));
        let joined = sketch.add_constraint(ConstraintKind::Coincident {
            point1: b,
            point2: c,
        });
        let upright = sketch.add_constraint(ConstraintKind::Vertical { element: second });
        let width = sketch.add_constraint(ConstraintKind::Length {
            line: first,
            length: 5.0,
        });
        let selected: HashSet<Uuid> = [b].into_iter().collect();
        let related: Vec<Uuid> = sketch
            .constraints
            .iter()
            .filter(|k| ConstraintFilter::Related.accepts(&sketch, k, &selected, &HashSet::new()))
            .map(|k| k.id)
            .collect();
        assert_eq!(related, [joined, upright, width]);
    }
}

#[cfg(all(test, feature = "egui"))]
mod icon_coverage {
    use super::*;
    use core_document::{Workbench, WorkbenchContext};

    #[test]
    fn the_bench_and_its_feature_name_an_icon_in_the_set() {
        let wb = SketchWorkbench::default();
        assert!(ui_kit::icon::exists(wb.descriptor().icon));
        let node = core_document::FeatureNode::new(
            FeatureId(uuid::Uuid::new_v4()),
            &SketchFeature::new(Sketch::new("s"), SketchPlane::default()),
        );
        assert!(ui_kit::icon::exists(wb.feature_info(&node).icon));
    }

    #[test]
    fn every_default_key_lands_on_a_tool_and_no_two_share_one() {
        let mut ctx = WorkbenchContext::default();
        SketchWorkbench::default().configure(&mut ctx);
        for (id, _) in TOOL_KEYS {
            assert!(ctx.tools().iter().any(|t| t.id == *id), "no tool {id}");
        }
        let mut seen = std::collections::HashMap::new();
        let keyed = ctx
            .tools()
            .iter()
            .map(|t| (&t.id, &t.shortcuts))
            .chain(ctx.actions().iter().map(|a| (&a.id, &a.shortcuts)));
        for (id, keys) in keyed {
            for key in keys {
                if let Some(other) = seen.insert(*key, id.clone()) {
                    panic!("{id} and {other} share {key}");
                }
            }
        }
    }

    #[test]
    fn every_tool_names_an_icon_in_the_set() {
        let mut ctx = WorkbenchContext::default();
        SketchWorkbench::default().configure(&mut ctx);
        for tool in ctx.tools() {
            let icon = tool
                .icon
                .unwrap_or_else(|| panic!("{} has no icon", tool.id));
            assert!(
                ui_kit::icon::exists(icon),
                "{}: unknown icon {icon}",
                tool.id
            );
            for variant in &tool.variants {
                assert!(
                    ui_kit::icon::exists(variant.icon),
                    "{}:{}: unknown icon {}",
                    tool.id,
                    variant.id,
                    variant.icon
                );
            }
        }
    }
}

#[cfg(test)]
mod dimension_tool {
    use super::*;
    use sketch::{Circle, Line, Point};

    #[test]
    fn the_dimension_tool_picks_what_the_selection_takes() {
        let mut sketch = Sketch::new("t");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 0.0))));
        let c = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 5.0))));
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let other = sketch.add_geometry(GeometryElement::Line(Line::new(a, c)));
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(c, 2.0)));
        let shape =
            |ids: &[Uuid]| constrain::SelectionShape::of(&sketch, &ids.iter().copied().collect());
        assert_eq!(dimension_for(&shape(&[circle])), Some("radius_diameter"));
        assert_eq!(dimension_for(&shape(&[line, other])), Some("angle"));
        assert_eq!(dimension_for(&shape(&[a, line, other])), Some("angle"));
        assert_eq!(dimension_for(&shape(&[a, b])), Some("distance"));
        assert_eq!(dimension_for(&shape(&[line, circle])), Some("distance"));
        assert_eq!(dimension_for(&shape(&[b, circle])), Some("distance"));
        assert_eq!(dimension_for(&shape(&[])), None);
    }

    #[test]
    fn the_grid_step_follows_the_zoom_in_one_two_five_steps() {
        let mut wb = SketchWorkbench::default();
        wb.options.grid_auto = true;
        // Forty pixels of grid at these zooms.
        assert_eq!(wb.grid_step(0.1), 5.0);
        assert_eq!(wb.grid_step(0.02), 1.0);
        assert_eq!(wb.grid_step(1.0), 50.0);
        wb.options.grid_auto = false;
        wb.options.grid_size = 7.5;
        assert_eq!(wb.grid_step(1.0), 7.5);
    }
}

#[cfg(test)]
mod sketch_picker {
    use super::*;
    use crate::sketch::ConstraintKind;
    use core_document::Document;
    use sketch::{Line, Point};

    /// A document with the edited sketch on `onto` and one other sketch on
    /// `from`: a 10 mm line along its X axis, horizontal and dimensioned.
    fn scene(from: SketchPlane, onto: SketchPlane) -> (Document, SketchWorkbench, FeatureId) {
        let mut doc = Document::new("t");
        let mut other = Sketch::new("other");
        let a = other.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = other.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 0.0))));
        let line = other.add_geometry(GeometryElement::Line(Line::new(a, b)));
        other.add_constraint(ConstraintKind::Horizontal { element: line });
        other.add_constraint(ConstraintKind::Length { line, length: 10.0 });
        let other_id = doc
            .add_feature_in_body(SketchFeature::new(other, from), "other".into(), None)
            .unwrap();
        let edited = doc
            .add_feature_in_body(
                SketchFeature::new(Sketch::new("edited"), onto),
                "edited".into(),
                None,
            )
            .unwrap();
        let wb = SketchWorkbench {
            active_sketch_id: Some(edited),
            ..SketchWorkbench::default()
        };
        (doc, wb, other_id)
    }

    fn ctx(doc: &mut Document) -> WorkbenchRuntimeContext<'_> {
        WorkbenchRuntimeContext::new(doc, [0.0, 0.0, 50.0], [0.0; 3], (0, 0, 800, 600))
    }

    fn points(sketch: &Sketch) -> Vec<Vec2D> {
        sketch
            .geometry
            .iter()
            .filter_map(|g| match g {
                GeometryElement::Point(p) => Some(p.position),
                _ => None,
            })
            .collect()
    }

    fn shifted(dx: f32, dy: f32, dz: f32) -> SketchPlane {
        SketchPlane {
            origin: [dx, dy, dz],
            ..SketchPlane::xy()
        }
    }

    #[test]
    fn a_carbon_copy_from_a_shifted_plane_keeps_its_place_and_constraints() {
        let (mut doc, mut wb, other) = scene(shifted(5.0, 0.0, 20.0), SketchPlane::xy());
        let mut ctx = ctx(&mut doc);
        wb.carbon_copy(&mut ctx, other);
        let copy = wb.get_active_sketch(&ctx).unwrap().sketch;
        assert_eq!(copy.geometry.len(), 3);
        assert_eq!(copy.constraints.len(), 2);
        let pts = points(&copy);
        assert!(
            pts.iter()
                .any(|p| (p.x - 5.0).abs() < 1e-4 && p.y.abs() < 1e-4)
        );
        assert!(
            pts.iter()
                .any(|p| (p.x - 15.0).abs() < 1e-4 && p.y.abs() < 1e-4)
        );
    }

    #[test]
    fn a_carbon_copy_from_a_facing_plane_is_mirrored_and_leaves_its_constraints() {
        let facing = SketchPlane {
            normal: [0.0, 0.0, -1.0],
            x_axis: [-1.0, 0.0, 0.0],
            ..SketchPlane::xy()
        };
        let (mut doc, mut wb, other) = scene(facing, SketchPlane::xy());
        let mut ctx = ctx(&mut doc);
        wb.carbon_copy(&mut ctx, other);
        let copy = wb.get_active_sketch(&ctx).unwrap().sketch;
        assert_eq!(copy.geometry.len(), 3);
        assert!(copy.constraints.is_empty());
        assert!(points(&copy).iter().any(|p| (p.x + 10.0).abs() < 1e-4));
    }

    #[test]
    fn a_plane_at_an_angle_is_refused() {
        assert!(plane_map(&SketchPlane::xz(), &SketchPlane::xy()).is_err());
        let (mut doc, mut wb, other) = scene(SketchPlane::xz(), SketchPlane::xy());
        let mut ctx = ctx(&mut doc);
        wb.carbon_copy(&mut ctx, other);
        assert!(
            wb.get_active_sketch(&ctx)
                .unwrap()
                .sketch
                .geometry
                .is_empty()
        );
    }

    #[test]
    fn merging_makes_a_new_sketch_and_keeps_the_originals() {
        let (mut doc, mut wb, other) = scene(shifted(0.0, 3.0, 0.0), SketchPlane::xy());
        let before = doc.feature_tree().all_nodes().count();
        let mut ctx = ctx(&mut doc);
        wb.open_sketch_picker(SketchPickerMode::Merge);
        wb.sketch_picker.as_mut().unwrap().checked.insert(other);
        wb.merge_sketches(&mut ctx);
        assert!(wb.sketch_picker.is_none());
        let merged: Vec<SketchFeature> = doc
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.name == "edited merged")
            .map(|(_, n)| SketchFeature::from_json(&n.data).unwrap())
            .collect();
        assert_eq!(doc.feature_tree().all_nodes().count(), before + 1);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].sketch.geometry.len(), 3);
        assert_eq!(merged[0].sketch.constraints.len(), 2);
        assert!(
            points(&merged[0].sketch)
                .iter()
                .all(|p| (p.y - 3.0).abs() < 1e-4)
        );
    }
}

#[cfg(test)]
mod placed_body {
    use super::*;
    use core_document::{BodyPlacement, Document};

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-4)
    }

    #[test]
    fn a_sketch_on_a_moved_body_is_edited_where_it_sits_and_stored_in_its_frame() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let sketch = doc
            .add_feature_in_body(
                SketchFeature::new(Sketch::new("s"), SketchPlane::xy()),
                "s".into(),
                Some(body),
            )
            .unwrap();
        let placement = BodyPlacement::new(
            glam::Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            glam::Vec3::new(0.0, 0.0, 20.0),
        );
        doc.set_body_placement(body, placement);
        let wb = SketchWorkbench {
            active_sketch_id: Some(sketch),
            ..SketchWorkbench::default()
        };
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut doc, [0.0, 0.0, 50.0], [0.0; 3], (0, 0, 800, 600));

        let seen = wb.get_active_sketch(&ctx).unwrap();
        assert!(close(seen.plane.origin, [0.0, 0.0, 20.0]));
        assert!(
            close(seen.plane.normal, [0.0, -1.0, 0.0]),
            "{:?}",
            seen.plane.normal
        );

        // An edit that does not move the plane leaves it exactly as stored.
        assert!(wb.store_sketch(&mut ctx, seen.clone()));
        let kept = stored_sketch(ctx.document, sketch).unwrap().plane;
        let xy = SketchPlane::xy();
        assert_eq!(
            (kept.origin, kept.normal, kept.x_axis, kept.y_axis),
            (xy.origin, xy.normal, xy.x_axis, xy.y_axis),
            "bit for bit"
        );

        // A plane set where the body sits is kept in the body's frame.
        let mut moved = seen;
        moved.plane.origin = [0.0, 0.0, 25.0];
        assert!(wb.store_sketch(&mut ctx, moved));
        let stored = stored_sketch(ctx.document, sketch).unwrap().plane;
        assert!(close(stored.origin, [0.0, 5.0, 0.0]), "{:?}", stored.origin);
        assert!(close(stored.normal, [0.0, 0.0, 1.0]));
    }
}

#[cfg(test)]
mod face_view {
    use super::*;

    /// A sketch made on a slanted face turns the view square to that face,
    /// centred on the point clicked on it.
    #[test]
    fn a_sketch_on_a_face_turns_the_view_to_the_face() {
        let mut doc = core_document::Document::new("t");
        let body = doc.create_body(None);
        let normal = {
            let n = glam::Vec3::new(1.0, 0.0, 1.0).normalize();
            [n.x, n.y, n.z]
        };
        let clicked = [10.0, 2.0, 5.0];
        let plane = SketchPlane::from_face(clicked, normal);
        let face = core_document::FaceRef {
            point: clicked,
            normal,
            surface: None,
            name: 0,
        };
        let support = crate::feature::FaceSupport::on(&face, plane);
        let mut wb = SketchWorkbench::default();
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        wb.create_sketch_on_plane(
            &mut ctx,
            Some(BodyId(body.0)),
            plane,
            NewSketchOn {
                face: Some(support),
                ..Default::default()
            },
        );
        let orient = ctx
            .take_requests()
            .into_iter()
            .find_map(|r| match r {
                HostRequest::OrientCamera(o) => Some(o),
                _ => None,
            })
            .expect("the view is turned");
        assert_eq!(orient.plane_normal, plane.normal);
        assert_eq!(orient.centre, Some(clicked), "on the face clicked");
    }

    /// A generator's sketch on a turned body: it is made by the generator,
    /// recorded as `sketch.new` naming it, and the view turns to the plane
    /// where the body has it, centred on where the generator stands.
    #[test]
    fn a_generated_sketch_turns_the_view_to_where_its_body_has_it() {
        let mut doc = core_document::Document::new("t");
        let body = doc.create_body(None);
        let turn = glam::Quat::from_rotation_x(30f32.to_radians());
        let mut placement = doc.body_placement(body);
        placement.rotation = turn.to_array();
        placement.translation = [5.0, 0.0, 0.0];
        doc.set_body_placement(body, placement);
        let plane = SketchPlane::from_frame([1.0, 2.0, 10.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
        let mut wb = SketchWorkbench::default();
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        wb.create_sketch_on_plane(
            &mut ctx,
            Some(body),
            plane,
            NewSketchOn {
                made_by: generator::Generator::named("sprocket"),
                ..Default::default()
            },
        );
        let outcome = core_document::HookOutcome::take(&mut ctx);
        let orient = outcome
            .requests
            .into_iter()
            .find_map(|r| match r {
                HostRequest::OrientCamera(o) => Some(o),
                _ => None,
            })
            .expect("the view is turned");
        let recorded = outcome.recorded;
        assert_eq!(recorded[0].id, "sketch.new");
        assert_eq!(recorded[0].args["generator"], "sprocket");
        let normal = turn * glam::Vec3::Z;
        assert!(glam::Vec3::from_array(orient.plane_normal).distance(normal) < 1e-5);
        let centre = turn * glam::Vec3::new(1.0, 2.0, 10.0) + glam::Vec3::new(5.0, 0.0, 0.0);
        let seen = glam::Vec3::from_array(orient.centre.expect("centred"));
        assert!(seen.distance(centre) < 1e-4, "{seen} is not {centre}");
        let id = wb.active_sketch_id.expect("the sketch is open");
        let made = stored_sketch(&doc, id).unwrap();
        assert!(made.generator.is_some(), "made by the sprocket generator");
        assert!(!made.sketch.geometry.is_empty(), "with its outline");
        assert_eq!(doc.get_feature_meta(id).unwrap().name, "Sprocket");
    }
}

#[cfg(test)]
mod section_view {
    use super::*;

    fn keeps([a, b, c, d]: [f32; 4], p: [f32; 3]) -> bool {
        a * p[0] + b * p[1] + c * p[2] + d >= 0.0
    }

    #[test]
    fn the_viewers_side_of_the_plane_is_cut_away_and_the_plane_kept() {
        let plane = SketchPlane::from_frame([0.0, 0.0, 5.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
        let from_above = section_equation(&plane, [0.0, 0.0, 50.0]);
        assert!(keeps(from_above, [3.0, 4.0, 1.0]));
        assert!(keeps(from_above, [3.0, 4.0, 5.0]), "the plane itself stays");
        assert!(!keeps(from_above, [3.0, 4.0, 6.0]));
        // Seen from below, the other side goes.
        let from_below = section_equation(&plane, [0.0, 0.0, -50.0]);
        assert!(keeps(from_below, [3.0, 4.0, 6.0]));
        assert!(!keeps(from_below, [3.0, 4.0, 1.0]));
        // A slanted plane cuts slanted.
        let slanted = SketchPlane::from_frame([0.0, 0.0, 0.0], [1.0, 0.0, 1.0], [1.0, 0.0, -1.0]);
        let eq = section_equation(&slanted, [30.0, 0.0, 30.0]);
        assert!(keeps(eq, [-1.0, 7.0, -1.0]) && !keeps(eq, [1.0, -7.0, 1.0]));
    }

    #[test]
    fn a_sketch_is_cut_on_open_unless_it_was_switched_off() {
        let mut feature = SketchFeature::new(sketch::Sketch::new("Sketch"), SketchPlane::default());
        assert!(feature.shows_section(true), "the setting cuts a new sketch");
        assert!(!feature.shows_section(false));
        feature.set_section(false);
        assert!(!feature.shows_section(true), "switched off, it stays off");
        feature.set_section(true);
        assert!(
            feature.shows_section(false),
            "switched on, it holds without the setting"
        );
        // Its choice survives a save.
        feature.set_section(false);
        let back = SketchFeature::from_json(&feature.to_json()).unwrap();
        assert!(!back.shows_section(true));
    }
}

#[cfg(test)]
mod close_sketch {
    use super::*;
    use core_document::Document;

    #[test]
    fn the_close_tool_ends_the_session_through_the_host_as_the_panel_does() {
        let mut doc = Document::new("t");
        let sketch = doc
            .add_feature_in_body(
                SketchFeature::new(Sketch::new("Sketch001"), SketchPlane::xy()),
                "Sketch001".into(),
                None,
            )
            .unwrap();
        let mut wb = SketchWorkbench {
            active_sketch_id: Some(sketch),
            ..SketchWorkbench::default()
        };
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut doc, [0.0, 0.0, 50.0], [0.0; 3], (0, 0, 800, 600));
        ctx.active_document_object = Some(sketch);
        wb.on_input(
            &WorkbenchInputEvent::ToolActivated,
            Some("sketch.finish"),
            &mut ctx,
        );
        let requests = ctx.take_requests();
        assert!(
            requests.contains(&HostRequest::FinishEditing),
            "{requests:?}"
        );
        assert!(
            requests.contains(&HostRequest::JournalLabel("Edit Sketch001".into())),
            "{requests:?}"
        );
    }
}

#[cfg(test)]
mod external_geometry {
    use super::*;
    use core_document::{BodyPlacement, Document, EdgeRef};
    use kernel_api::{KernelQueries, KernelResult, ProfilePlane, ProjectedEdge};

    /// A kernel whose every edge is a segment across the plane's X axis,
    /// from where the probe lands in the plane to 10 mm beyond it.
    struct FlatKernel;

    impl KernelQueries for FlatKernel {
        fn project_edge(
            &self,
            _brep: &[u8],
            near: [f64; 3],
            plane: &ProfilePlane,
        ) -> KernelResult<ProjectedEdge> {
            let d: Vec<f64> = (0..3).map(|k| near[k] - plane.origin[k]).collect();
            let x: f64 = (0..3).map(|k| d[k] * plane.x_axis[k]).sum();
            let y: f64 = (0..3).map(|k| d[k] * plane.y_axis[k]).sum();
            Ok(ProjectedEdge::Line {
                start: [x, y],
                end: [x + 10.0, y],
            })
        }

        /// Every face crosses the plane in two lines up the plane's Y axis,
        /// either side of where the probe lands.
        fn section_face(
            &self,
            _brep: &[u8],
            near: [f64; 3],
            plane: &ProfilePlane,
        ) -> KernelResult<Vec<ProjectedEdge>> {
            let d: Vec<f64> = (0..3).map(|k| near[k] - plane.origin[k]).collect();
            let x: f64 = (0..3).map(|k| d[k] * plane.x_axis[k]).sum();
            Ok([x - 1.0, x + 1.0]
                .map(|x| ProjectedEdge::Line {
                    start: [x, 0.0],
                    end: [x, 5.0],
                })
                .to_vec())
        }
    }

    static FLAT: FlatKernel = FlatKernel;

    #[test]
    fn a_picked_face_brings_in_where_it_crosses_the_plane_once_and_records_it() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        doc.set_imported_brep_data(body, b"ogeom shape".to_vec(), Vec::new());
        let sketch = doc
            .add_feature_in_body(
                SketchFeature::new(Sketch::new("s"), SketchPlane::xy()),
                "s".into(),
                Some(body),
            )
            .unwrap();
        let mut wb = SketchWorkbench {
            active_sketch_id: Some(sketch),
            external_refreshed: Some(sketch),
            ..SketchWorkbench::default()
        };
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut doc, [0.0, 0.0, 50.0], [0.0; 3], (0, 0, 800, 600));
        ctx.active_document_object = Some(sketch);
        ctx.kernel = Some(&FLAT);
        // The tool's intersection variant is armed; the host picks a face.
        let armed = WorkbenchInputEvent::KeyPress { key: KeyCode::A };
        wb.on_input(&armed, Some("sketch.external:intersection"), &mut ctx);
        ctx.selected_body_id = Some(body.0);
        ctx.selected_face = Some(core_document::FaceRef {
            name: 0,
            point: [4.0, 2.0, -3.0],
            normal: [1.0, 0.0, 0.0],
            surface: None,
        });
        wb.on_frame(0.016, &mut ctx);
        wb.on_frame(0.016, &mut ctx);
        let recorded = core_document::HookOutcome::take(&mut ctx).recorded;
        assert_eq!(recorded.len(), 1, "taken once: {recorded:?}");
        assert_eq!(recorded[0].id, "sketch.intersection");

        let stored = stored_sketch(ctx.document, sketch).unwrap().sketch;
        assert_eq!(stored.external.len(), 2, "both lines came in");
        assert!(stored.external.values().all(|s| s.section));
        let xs: Vec<f32> = stored
            .geometry
            .iter()
            .filter_map(|g| match g {
                GeometryElement::Point(p) => Some(p.position.x),
                _ => None,
            })
            .collect();
        assert!(
            xs.iter()
                .all(|x| (x - 3.0).abs() < 1e-5 || (x - 5.0).abs() < 1e-5)
        );
        // The edge variant again takes edges, not the face.
        wb.on_input(&armed, Some("sketch.external:edges"), &mut ctx);
        wb.on_frame(0.016, &mut ctx);
        let stored = stored_sketch(ctx.document, sketch).unwrap().sketch;
        assert_eq!(stored.external.len(), 2);
    }

    #[test]
    fn a_picked_edge_comes_in_fixed_and_out_of_profiles() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        doc.set_imported_brep_data(body, b"ogeom shape".to_vec(), Vec::new());
        // The body sits 3 up: the pick is in the scene, the edge in its body.
        doc.set_body_placement(
            body,
            BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(0.0, 3.0, 0.0)),
        );
        let sketch = doc
            .add_feature_in_body(
                SketchFeature::new(Sketch::new("s"), SketchPlane::xy()),
                "s".into(),
                Some(body),
            )
            .unwrap();
        let mut wb = SketchWorkbench {
            active_sketch_id: Some(sketch),
            last_tool: Some("sketch.external".to_string()),
            external_refreshed: Some(sketch),
            ..SketchWorkbench::default()
        };
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut doc, [0.0, 0.0, 50.0], [0.0; 3], (0, 0, 800, 600));
        ctx.active_document_object = Some(sketch);
        ctx.kernel = Some(&FLAT);
        ctx.selected_edges = vec![EdgeRef {
            faces: [0, 0],
            point: [2.0, 7.0, 0.0],
            direction: [1.0, 0.0, 0.0],
            length_mm: 10.0,
            body: body.0,
            circle: None,
        }];
        wb.on_frame(0.016, &mut ctx);
        // Taken once, however many frames the pick stays selected.
        wb.on_frame(0.016, &mut ctx);

        let stored = stored_sketch(ctx.document, sketch).unwrap().sketch;
        assert_eq!(stored.external.len(), 1);
        let (line_id, source) = stored.external.iter().next().unwrap();
        assert_eq!(source.point, [2.0, 4.0, 0.0], "kept in the body's frame");
        let Some(GeometryElement::Line(line)) = stored.get_geometry(*line_id) else {
            panic!("a line came in")
        };
        // The scene has the sketch where the body sits; in the sketch's own
        // coordinates the edge lies where it was picked.
        assert_eq!(
            stored.point_position(line.start),
            Some(Vec2D::new(2.0, 4.0))
        );
        assert!(stored.is_external(line.start), "its points are held too");
        assert!(
            profile::extract_wires(&stored).is_err(),
            "no profile from it"
        );
    }
}

#[cfg(test)]
mod formulas {
    use super::*;
    use crate::sketch::{Constraint, ConstraintKind};
    use core_document::{Document, DocumentService};
    use sketch::{Line, Point};

    /// A horizontal line fixed at the origin, with a length dimension.
    fn scene() -> (Document, SketchWorkbench, FeatureId, Uuid) {
        let mut sketch = Sketch::new("s");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 0.0))));
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        sketch
            .constraints
            .push(Constraint::new(ConstraintKind::FixedPoint {
                point: a,
                position: Vec2D::new(0.0, 0.0),
            }));
        sketch
            .constraints
            .push(Constraint::new(ConstraintKind::Horizontal {
                element: line,
            }));
        let length = Constraint::new(ConstraintKind::Length { line, length: 10.0 });
        let length_id = length.id;
        sketch.constraints.push(length);
        let plane = sketch.plane;
        let mut doc = Document::new("t");
        let sizes = doc.add_variable_set("Sizes").unwrap();
        doc.set_variable(sizes, "w", "30 mm", None).unwrap();
        let id = doc
            .add_feature(SketchFeature::new(sketch, plane), "Profile".into())
            .unwrap();
        let wb = SketchWorkbench {
            active_sketch_id: Some(id),
            ..Default::default()
        };
        (doc, wb, id, length_id)
    }

    fn line_length(wb: &SketchWorkbench, doc: &mut Document) -> f32 {
        let ctx = WorkbenchRuntimeContext::new(doc, [0.0, 0.0, 50.0], [0.0; 3], (0, 0, 800, 600));
        let feature = wb.get_active_sketch(&ctx).unwrap();
        let xs: Vec<f32> = feature
            .sketch
            .geometry
            .iter()
            .filter_map(|g| match g {
                GeometryElement::Point(p) => Some(p.position.x),
                _ => None,
            })
            .collect();
        xs.iter().cloned().fold(f32::MIN, f32::max) - xs.iter().cloned().fold(f32::MAX, f32::min)
    }

    #[test]
    fn a_dimension_set_by_a_formula_solves_and_follows_its_variable_while_open() {
        let mut registry = DocumentService::default();
        registry
            .register_workbench(Box::new(SketchWorkbench::default()))
            .unwrap();
        let (mut doc, mut wb, id, length) = scene();
        registry.evaluate(&mut doc);
        {
            let mut ctx = WorkbenchRuntimeContext::new(
                &mut doc,
                [0.0, 0.0, 50.0],
                [0.0; 3],
                (0, 0, 800, 600),
            );
            wb.set_dimension(&mut ctx, length, "Sizes.w / 2", true);
        }
        assert_eq!(
            doc.feature_formula(id, &length.to_string()),
            Some("Sizes.w / 2")
        );
        registry.evaluate(&mut doc);
        assert!((line_length(&wb, &mut doc) - 15.0).abs() < 1e-3);

        // The variable changes: the open sketch reads its solved copy.
        let sizes = doc.object_named("Sizes").unwrap();
        doc.set_variable(sizes, "w", "50 mm", None).unwrap();
        registry.evaluate(&mut doc);
        assert!((line_length(&wb, &mut doc) - 25.0).abs() < 1e-3);

        // A value typed with a unit takes the formula away.
        {
            let mut ctx = WorkbenchRuntimeContext::new(
                &mut doc,
                [0.0, 0.0, 50.0],
                [0.0; 3],
                (0, 0, 800, 600),
            );
            wb.set_dimension(&mut ctx, length, "1 in", true);
        }
        assert_eq!(doc.feature_formula(id, &length.to_string()), None);
        registry.evaluate(&mut doc);
        assert!((line_length(&wb, &mut doc) - 25.4).abs() < 1e-3);
    }

    #[test]
    fn saving_a_value_as_a_variable_makes_it_and_binds_the_dimension() {
        let mut registry = DocumentService::default();
        registry
            .register_workbench(Box::new(SketchWorkbench::default()))
            .unwrap();
        let (mut doc, mut wb, id, length) = scene();
        registry.evaluate(&mut doc);
        // Into a new set: the name is free, the set is made for it.
        {
            let mut ctx = WorkbenchRuntimeContext::new(
                &mut doc,
                [0.0, 0.0, 50.0],
                [0.0; 3],
                (0, 0, 800, 600),
            );
            let save = SaveAs {
                name: "wall".into(),
                set: None,
            };
            wb.save_dimension_as(&mut ctx, length, "12", &save);
        }
        let set = doc.object_named("Variables").expect("a set was made");
        let (_, _, data) = doc
            .variable_sets()
            .into_iter()
            .find(|(s, ..)| *s == set)
            .unwrap();
        assert_eq!(data.variable("wall").unwrap().formula, "12 mm");
        assert_eq!(
            doc.feature_formula(id, &length.to_string()),
            Some("Variables.wall")
        );
        // The dimension stands at the value at once, and follows the
        // variable from now on.
        assert!((line_length(&wb, &mut doc) - 12.0).abs() < 1e-3);
        doc.set_variable(set, "wall", "20 mm", None).unwrap();
        registry.evaluate(&mut doc);
        assert!((line_length(&wb, &mut doc) - 20.0).abs() < 1e-3);

        // Into a set that is there: a formula typed is the variable's.
        let sizes = doc.object_named("Sizes").unwrap();
        {
            let mut ctx = WorkbenchRuntimeContext::new(
                &mut doc,
                [0.0, 0.0, 50.0],
                [0.0; 3],
                (0, 0, 800, 600),
            );
            let save = SaveAs {
                name: "half".into(),
                set: Some(sizes),
            };
            wb.save_dimension_as(&mut ctx, length, "Sizes.w / 2", &save);
        }
        registry.evaluate(&mut doc);
        assert_eq!(
            doc.feature_formula(id, &length.to_string()),
            Some("Sizes.half")
        );
        assert!((line_length(&wb, &mut doc) - 15.0).abs() < 1e-3);
        assert_eq!(doc.variable_sets().len(), 2, "no set was made for it");
    }

    #[test]
    fn a_field_is_offered_the_variables_of_its_kind_under_free_names() {
        let (mut doc, ..) = scene();
        let mut registry = DocumentService::default();
        registry
            .register_workbench(Box::new(SketchWorkbench::default()))
            .unwrap();
        let sizes = doc.object_named("Sizes").unwrap();
        doc.set_variable(sizes, "tilt", "30 deg", None).unwrap();
        registry.evaluate(&mut doc);
        let names = |dim| {
            doc.variables_of(dim)
                .into_iter()
                .map(|c| c.reference)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(core_document::expr::Dim::LENGTH), ["Sizes.w"]);
        assert_eq!(names(core_document::expr::Dim::ANGLE), ["Sizes.tilt"]);
        assert_eq!(doc.unused_variable_name(Some(sizes), "w"), "w_2");
        assert_eq!(doc.unused_variable_name(None, "w"), "w");
        assert_eq!(doc.unused_object_name("Sizes"), "Sizes_2");
        assert_eq!(doc.unused_object_name("Variables"), "Variables");
    }
}

#[cfg(test)]
mod wall_thickness {
    use super::*;
    use core_document::{Document, HookOutcome, LogLevel};
    use kernel_api::{
        KernelQueries, KernelResult, MedialPath, MedialRegion, Narrowest, Profile, ProfilePlane,
        ProjectedEdge,
    };

    /// A kernel that finds every profile 0.6 mm thin along y = 1.
    struct Thin;

    impl KernelQueries for Thin {
        fn project_edge(
            &self,
            _brep: &[u8],
            _near: [f64; 3],
            _plane: &ProfilePlane,
        ) -> KernelResult<ProjectedEdge> {
            unreachable!()
        }

        fn medial_axis(&self, profile: &Profile, _: f64) -> KernelResult<Vec<MedialRegion>> {
            Ok(vec![MedialRegion {
                wires: (0..profile.wires.len()).collect(),
                paths: vec![MedialPath {
                    points: vec![[1.0, 1.0], [10.0, 1.0], [19.0, 1.0]],
                    clearance: vec![0.3; 3],
                    boundary_ends: [false; 2],
                }],
                narrowest: Some(Narrowest {
                    at: [10.0, 1.0],
                    clearance: 0.3,
                }),
            }])
        }
    }

    static THIN: Thin = Thin;

    fn view_proj() -> [[f32; 4]; 4] {
        let proj = glam::camera::rh::proj::directx::perspective(
            60f32.to_radians(),
            800.0 / 600.0,
            0.1,
            1000.0,
        );
        let flip_y = glam::Mat4::from_scale(glam::Vec3::new(1.0, -1.0, 1.0));
        let view = glam::camera::rh::view::look_at_mat4(
            glam::Vec3::new(10.0, 1.0, 50.0),
            glam::Vec3::new(10.0, 1.0, 0.0),
            glam::Vec3::Y,
        );
        (flip_y * proj * view).to_cols_array_2d()
    }

    fn ctx(doc: &mut Document, sketch: FeatureId) -> WorkbenchRuntimeContext<'_> {
        let mut ctx = WorkbenchRuntimeContext::new(
            doc,
            [10.0, 1.0, 50.0],
            [10.0, 1.0, 0.0],
            (0, 0, 800, 600),
        );
        ctx.view_proj = Some(view_proj());
        ctx.kernel = Some(&THIN);
        ctx.active_document_object = Some(sketch);
        ctx
    }

    fn call(ctx: &mut WorkbenchRuntimeContext, id: &str, args: serde_json::Value) {
        commands::run(id, &commands::args(args), ctx).unwrap();
    }

    fn scene() -> (Document, SketchWorkbench, FeatureId) {
        let mut doc = Document::new("t");
        let sketch = doc
            .add_feature(
                SketchFeature::new(Sketch::new("s"), SketchPlane::xy()),
                "s".into(),
            )
            .unwrap();
        let s = sketch.0.to_string();
        call(
            &mut ctx(&mut doc, sketch),
            "sketch.rect",
            serde_json::json!({"sketch": s, "x": 0, "y": 0, "width": 20, "height": 2}),
        );
        let wb = SketchWorkbench {
            active_sketch_id: Some(sketch),
            ..SketchWorkbench::default()
        };
        (doc, wb, sketch)
    }

    fn label_texts(wb: &SketchWorkbench, ctx: &WorkbenchRuntimeContext) -> Vec<String> {
        wb.get_screen_space_labels(ctx, None)
            .into_iter()
            .map(|l| l.text)
            .collect()
    }

    #[test]
    fn the_tool_reports_draws_and_records_the_thinnest_wall() {
        let (mut doc, mut wb, sketch) = scene();
        let mut ctx = ctx(&mut doc, sketch);
        let move_to = |x: f32| WorkbenchInputEvent::MouseMove {
            viewport_pos: (x, 300.0),
        };
        // A drawing tool was out before the check.
        wb.on_input(&move_to(100.0), Some("sketch.line"), &mut ctx);
        wb.on_input(
            &WorkbenchInputEvent::ToolActivated,
            Some("sketch.wall_thickness"),
            &mut ctx,
        );
        // The host leaves no tool once the action is handled.
        wb.on_input(&move_to(120.0), None, &mut ctx);
        let logs = ctx.drain_logs();
        assert!(
            logs.iter()
                .any(|l| l.level == LogLevel::Warn && l.message.contains("0.60 mm")),
            "{logs:?}"
        );
        let pal = ctx.sketch_palette;
        let thin_lines = wb
            .get_screen_space_overlays(&ctx, None)
            .into_iter()
            .filter(|o| o.color == pal.wall_thin)
            .count();
        assert_eq!(thin_lines, 2, "the axis draws in the warning colour");
        assert!(label_texts(&wb, &ctx).contains(&"0.60 mm".to_string()));

        let recorded = HookOutcome::take(&mut ctx).recorded;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].id, "sketch.wall_thickness");
        assert_eq!(recorded[0].result["thin"], serde_json::json!(true));

        // Escape puts it away.
        wb.on_input(
            &WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Escape,
            },
            None,
            &mut ctx,
        );
        assert!(!label_texts(&wb, &ctx).contains(&"0.60 mm".to_string()));
    }

    #[test]
    fn an_edit_to_the_profile_puts_the_check_away() {
        let (mut doc, mut wb, sketch) = scene();
        let mut ctx = ctx(&mut doc, sketch);
        let s = sketch.0.to_string();
        let args = commands::args(serde_json::json!({"sketch": s, "minimum": 0.5}));
        let report = wb
            .run_command("sketch.wall_thickness", &args, &mut ctx)
            .unwrap();
        assert_eq!(report["thin"], serde_json::json!(false), "{report}");
        assert!((report["thinnest"].as_f64().unwrap() - 0.6).abs() < 1e-9);
        wb.on_frame(0.016, &mut ctx);
        assert!(
            wb.shown_wall_check().is_some(),
            "an unchanged sketch keeps it"
        );

        call(
            &mut ctx,
            "sketch.circle",
            serde_json::json!({"sketch": s, "x": 40, "y": 0, "radius": 2}),
        );
        wb.on_frame(0.016, &mut ctx);
        assert!(wb.shown_wall_check().is_none());
    }
}

#[cfg(test)]
mod passive_regions {
    use super::*;
    use core_document::Document;
    use sketch::{Line, Point};

    /// A document holding one sketch: a triangle, closed or with one side
    /// left out.
    fn triangle(closed: bool) -> (Document, FeatureId) {
        let mut sketch = Sketch::new("t");
        let corners: Vec<Uuid> = [(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)]
            .iter()
            .map(|(x, y)| {
                sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(*x, *y))))
            })
            .collect();
        let sides = if closed { 3 } else { 2 };
        for i in 0..sides {
            sketch.add_geometry(GeometryElement::Line(Line::new(
                corners[i],
                corners[(i + 1) % 3],
            )));
        }
        let mut doc = Document::new("t");
        let id = doc
            .add_feature_in_body(
                SketchFeature::new(sketch, SketchPlane::xy()),
                "t".into(),
                None,
            )
            .unwrap();
        (doc, id)
    }

    fn region(
        bench: &SketchWorkbench,
        doc: &Document,
        id: FeatureId,
    ) -> Option<core_document::PassiveRegion> {
        let node = doc.get_feature_meta(id).unwrap();
        bench.passive_geometry(doc, id, node).unwrap().region
    }

    #[test]
    fn a_closed_sketch_offers_its_loops_to_shade_when_regions_are_shaded() {
        let (doc, id) = triangle(true);
        let mut bench = SketchWorkbench::default();
        let shaded = region(&bench, &doc, id).expect("a region");
        assert_eq!(shaded.profile.wires.len(), 1);
        assert!(shaded.profile.plane.origin[2] > 0.0, "a hair off the plane");
        bench.options.shade_regions = false;
        assert!(region(&bench, &doc, id).is_none());
    }

    #[test]
    fn an_open_sketch_shades_nothing() {
        let (doc, id) = triangle(false);
        assert!(region(&SketchWorkbench::default(), &doc, id).is_none());
    }
}
