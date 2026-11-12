// Without the egui feature the crate is the headless subset the tests and
// the kernel bench use; the panel-only state and helpers go unused there.
#![cfg_attr(not(feature = "egui"), allow(dead_code))]

/// The feature kind sketches are stored under.
pub const KIND: &str = "wb.sketch";

mod actions;
mod appearance;
pub mod boundary;
mod candidates;
mod chain;
mod closed_parts;
mod commands;
pub mod conic;
mod constrain;
mod constraint_tools;
mod corner_task;
mod curves;
mod dim_edit;
mod dimension_task;
mod drawing;
mod dxf;
mod external;
mod external_follow;
mod external_ref;
use external_follow::{
    Projection, follow_references, project_source_in, projection_key, solid_built_from,
};
pub(crate) use external_follow::{project_source, with_edge_names};
mod feature;
mod frame_memo;
pub mod generator;
mod geom2d;
mod glyphs;
mod images;
mod input;
pub mod internal;
mod measure;
mod model_picks;
pub mod operations;
mod overlay;
mod ovp;
#[cfg(feature = "egui")]
mod panel;
mod params;
mod picture_edges;
mod placement;
pub mod profile;
mod rect;
pub mod regions;
mod registration;
mod relations;
pub mod render;
pub mod repair;
mod scan;
mod selection;
#[cfg(feature = "egui")]
mod settings;
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
mod verdict;
mod viewport_out;
mod walls;

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use boundary::{narrow, narrow3};
use core_document::{
    BodyId, FeatureId, FeatureInfo, HostRequest, InputResult, MenuItem, MenuScope, TaskInfo,
    Workbench, WorkbenchContext, WorkbenchDescriptor, WorkbenchFeature, WorkbenchInputEvent,
    WorkbenchRuntimeContext, base_tool_id, tool_variant,
};
pub use dim_edit::{DimEdit, SaveAs};
pub use feature::{AttachedSupport, DatumSupport, FaceSupport, LentFace, SketchFeature};
use operations::SolveOutcome;
use ovp::DimCapture;
use placement::local_plane;
pub(crate) use placement::{
    copy_sketch_into, placed_plane, plane_map, sketch_placement, stored_sketch,
};
use sketch::{Constraint, GeometryElement, Sketch, SketchPlane, Vec2D};
use solver::Diagnosis;
pub use tools::ToolParams;
use tools::ToolState;
use uuid::Uuid;

/// Snap / hit-test tolerance in screen pixels, converted to sketch units
/// per frame from the current zoom.
const SNAP_TOLERANCE_PX: f64 = 8.0;

/// The open sketch's locked elements with the sketch and the document
/// edit they were worked out at.
type LockedAt = ((FeatureId, u64), std::sync::Arc<HashSet<Uuid>>);

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

/// How the constraint list names a constraint: its own name, or its kind.
pub(crate) fn constraint_row_label(c: &Constraint) -> String {
    c.name
        .clone()
        .unwrap_or_else(|| sketch::constraint_label(&c.kind))
}

/// What a constraint ties, as the list says it beside its name: the
/// elements it names, each once, in order ("Line 2 · Point 4").
pub(crate) fn constraint_operands(sketch: &Sketch, c: &Constraint) -> String {
    let mut seen = Vec::new();
    for id in sketch::constraint_refs(&c.kind) {
        if !seen.contains(&id) {
            seen.push(id);
        }
    }
    seen.iter()
        .map(|id| style::element_name(sketch, *id))
        .collect::<Vec<_>>()
        .join(" · ")
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
    /// The datum selected when the request was made, offered first as a
    /// face is.
    datum: Option<FeatureId>,
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
    /// A point dragged alone: the point or curve it is over, and since
    /// when. Let go there after the delay, the point is joined to it.
    target: Option<(Uuid, Instant)>,
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

/// A picture's calibration under way: the points picked on it so far,
/// and the real length and options the panel holds.
#[derive(Debug, Clone)]
pub(crate) struct Calibrating {
    pub image: Uuid,
    pub from: Option<Vec2D>,
    pub to: Option<Vec2D>,
    /// The real distance between the points, as typed: a value or a
    /// formula.
    pub length: String,
    pub straighten: bool,
    pub centre: bool,
}

/// The steps the angle snap offers, in degrees.
pub const ANGLE_STEPS: [(f64, &str); 7] = [
    (1.0, "1°"),
    (5.0, "5°"),
    (10.0, "10°"),
    (15.0, "15°"),
    (30.0, "30°"),
    (45.0, "45°"),
    (90.0, "90°"),
];
/// The size of a spline's degree, multiplicity and weight labels, in
/// logical pixels.
const SPLINE_LABEL_PT: f32 = 11.0;
/// How far the shaded regions stand off the sketch plane, in mm.
const REGION_LIFT_MM: f64 = 0.01;

/// The edited sketch's regions, each meshed with its opacity, by a key of
/// what they were made from.
type EditedRegions = (u64, Vec<(std::sync::Arc<kernel_api::TriMesh>, f32)>);

/// The switches on the sketcher's panel and Preferences page.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SketchOptions {
    /// A grid on the sketch plane.
    pub grid_on: bool,
    /// The grid step follows the zoom instead of `grid_size`.
    pub grid_auto: bool,
    /// The grid step in sketch units when it does not follow the zoom.
    pub grid_size: f64,
    /// Drawing tools land on grid points.
    pub grid_snap: bool,
    /// Constraint glyphs stay off the viewport.
    pub constraints_hidden: bool,
    /// Lines snapped level, plumb, parallel or square get the matching
    /// constraint as drawn, and a dragged point let go over another point
    /// or a curve is held there.
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
    pub min_wall: f64,
    /// A dimension a tool adds opens its value editor at once.
    pub ask_dimension_value: bool,
    /// What dimensions' labels show.
    #[serde(default)]
    pub dimension_labels: glyphs::DimensionLabels,
    /// Splines draw their curvature combs.
    pub spline_comb: bool,
    /// Splines mark their knots.
    pub spline_knots: bool,
    /// Splines draw their control polygon.
    pub spline_polygon: bool,
    /// Splines label their degree.
    pub spline_degree: bool,
    /// Splines label each interior knot's multiplicity.
    pub spline_multiplicity: bool,
    /// Splines label each control point's weight.
    pub spline_weights: bool,
    /// Editing a sketch cuts the view at its plane unless the sketch was
    /// set otherwise; off, only a sketch switched on is cut.
    pub section_on_open: bool,
    /// The regions the profile closes are shaded while editing.
    pub shade_regions: bool,
    /// How the edited sketch draws.
    pub appearance: appearance::Appearance,
    /// Snaps that align the segment being drawn (level, plumb, parallel,
    /// square) or put a point on a line's extension draw their dashed
    /// guide lines.
    pub hint_lines: bool,
    /// With Shift held, a segment leaves its start in whole steps of this
    /// many degrees from the X axis.
    pub angle_step_deg: f64,
    /// A drawing tool stays in hand once its shape is drawn; off, the
    /// pointer goes back to the Select tool.
    pub continue_drawing: bool,
    /// A constraint tool, once its constraint is on, waits for the next
    /// elements to constrain the same way; off, it is put away.
    pub continue_constraining: bool,
    /// While a drawing tool is active, each arc's whole circle draws as a
    /// faint dashed guide, and a point snaps onto it past the arc's ends.
    pub arc_helpers: bool,
    /// How long, in milliseconds, a dragged point rests over another point
    /// or a curve before its release there joins it with a constraint.
    pub drag_constraint_delay_ms: f32,
    /// A scan's slice points within this of the sketch's curves, mm, draw
    /// as fitting.
    pub scan_range: f64,
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
            spline_polygon: true,
            spline_degree: false,
            spline_multiplicity: false,
            spline_weights: false,
            section_on_open: false,
            shade_regions: true,
            appearance: appearance::Appearance::default(),
            hint_lines: true,
            angle_step_deg: 15.0,
            continue_drawing: true,
            continue_constraining: false,
            arc_helpers: true,
            drag_constraint_delay_ms: 400.0,
            scan_range: 0.1,
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

/// What the bench holds for the document on screen, moved with its tab:
/// everything but its settings.
#[derive(Default)]
struct Tab {
    /// Reference pictures decoded so far, by asset.
    image_cache: images::Cache,
    /// The reference pictures' edges, found away from the window.
    picture_edges: picture_edges::Edges,
    /// A left press of the line tool, held to tell a click from a drag:
    /// dragged from the end it draws from, it draws an arc.
    line_press: Option<LinePress>,
    /// The line tool's next click is its arc (`step::line_arc_click`).
    arc_click: bool,
    /// The segments the line or polyline gesture has committed, each with
    /// its undo step.
    chain_links: chain::Links,
    /// The shaded regions of the sketch being edited, each meshed with
    /// its opacity, by the sketch and the opacity they were made from.
    regions: std::cell::RefCell<Option<EditedRegions>>,
    /// Each closed sketch's regions, as the view shades and picks them.
    closed_parts: closed_parts::Kept,
    /// Where the scans cross the open sketch's plane, by what they were cut
    /// from.
    scan_guides: std::cell::RefCell<Option<(u64, std::sync::Arc<Vec<scan::Chain2>>)>>,
    /// The elements of the open sketch nothing left free can move, by the
    /// sketch and the document edit they were worked out at.
    locked: std::cell::RefCell<Option<LockedAt>>,
    /// The panel's list of other sketches, when carbon copy or merge is
    /// picking.
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
    /// The sketches in construction mode, in which every newly created
    /// element (from any drawing tool) is flagged as construction
    /// geometry. Each sketch keeps its own mode; the `sketch.construction`
    /// action toggles the edited one's when nothing is selected.
    construction_sketches: HashSet<FeatureId>,
    /// Last solver outcome, surfaced in the panel.
    last_solve: Option<SolveOutcome>,
    /// With automatic update off: the edited sketch's geometry does not
    /// meet its constraints and waits for Solve now. Measured where the
    /// sketch changes (an edit left unsolved, an undo), not every frame.
    unsolved_edits: bool,
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
    /// The constraint whose row the panel's list is under the pointer: its
    /// glyph and the geometry it holds draw as if selected.
    hovered_constraint: Option<Uuid>,
    /// The element or constraint whose list row scrolls into view on the
    /// panel's next frame (a chip or badge naming it was clicked).
    scroll_to_row: Option<Uuid>,
    /// Selected constraint ids (glyph click; ctrl additive).
    selected_constraints: HashSet<Uuid>,
    /// Dimension label being dragged (select mode).
    label_drag: Option<LabelDrag>,
    /// Pending in-viewport dimension edit.
    dim_edit: Option<DimEdit>,
    /// Last glyph press, for double-click detection.
    last_glyph_click: Option<(Uuid, Instant)>,
    /// Last select-mode press on geometry, and on empty space, for
    /// double-click detection.
    last_geometry_click: Option<(Uuid, Instant)>,
    last_empty_click: Option<Instant>,
    /// Each sketch's external elements whose edge, face, element or datum
    /// could not be found, with what that was worked out for.
    lost_external: std::sync::Mutex<HashMap<FeatureId, (u64, Vec<Uuid>)>>,
    /// What each sketch's solid edges and faces came to when evaluation
    /// last projected them, by what they were projected from
    /// (`projection_key`).
    solid_projections: std::sync::Mutex<HashMap<FeatureId, HashMap<u64, Projection>>>,
    /// A picture being calibrated from two points picked on it.
    calibrating: Option<Calibrating>,
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
    /// What the task panel declared last, for reading its events.
    #[cfg(feature = "egui")]
    panel: panel::PanelState,
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
    /// A fillet or chamfer open in its card, its result previewed.
    corner_task: Option<corner_task::CornerTask>,
    /// The open dimension task's draft, beside its `dim_edit`.
    dim_draft: Option<dimension_task::DimensionDraft>,
    /// The last wall thickness check, drawn until the sketch changes, the
    /// session ends, Escape or another tool.
    wall_check: Option<walls::WallCheck>,
    /// A constraint tool waiting for its operands, sketch elements or
    /// edges and vertices of the model.
    armed: Option<model_picks::Armed>,
    /// The constraint tools the selection, as it is or with more picks,
    /// could take, and the selection they were worked out for.
    armable: (Option<constrain::SelectionShape>, HashSet<&'static str>),
}

/// Sketch workbench: 2D drawing with constraints.
#[derive(Default)]
pub struct SketchWorkbench {
    /// The first key of each of the workbench's tools and actions, as the
    /// user set them, for the hints that name a key.
    action_keys: HashMap<String, String>,
    /// The panel's and the Preferences page's switches.
    pub options: SketchOptions,
    /// What the bench holds for the document on screen, moved with its
    /// tab.
    tab: frame_memo::Stamped<Tab>,
    /// What this frame's hooks have read of the edited sketch.
    frame: frame_memo::FrameMemo,
}

/// A flag each constraint carries, as the toolbar flips it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConstraintFlag {
    Driving,
    Active,
    Parked,
}

/// The most curves one pointer move of a trim stroke trims.
const MAX_STROKE_TRIMS: usize = 64;

/// The polyline's switch between a straight and a tangent-arc segment.
const POLYLINE_ARC_ACTION: &str = "sketch.polyline_arc";

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

impl SketchWorkbench {
    /// The dimensions of the edited sketch that a formula sets.
    fn bound_dimensions(&self, ctx: &WorkbenchRuntimeContext) -> HashSet<Uuid> {
        self.tab
            .active_sketch_id
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
        let id = self.tab.active_sketch_id?;
        let mut feature = stored_sketch(ctx.document, id)?;
        let placement = sketch_placement(ctx.document, id);
        feature.plane = placed_plane(&feature.plane, &placement);
        feature.sketch.plane = placed_plane(&feature.sketch.plane, &placement);
        if let Some(LabelDrag {
            constraint,
            current: Some(offset),
            ..
        }) = &self.tab.label_drag
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
        if stored && let Some(id) = self.tab.active_sketch_id {
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
        let Some(id) = self.tab.active_sketch_id else {
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
        self.tab.own_seq.store(
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
        self.tab
            .selected
            .retain(|id| feature.sketch.get_geometry(*id).is_some());
        self.tab
            .selected_constraints
            .retain(|id| feature.sketch.constraints.iter().any(|c| c.id == *id));
        if !self.options.auto_update {
            // The panel's Solve now button runs it.
            self.tab.last_diagnosis = None;
            self.tab.unsolved_edits = !solver::holds(&feature.sketch);
            return;
        }
        self.solve_now(ctx, feature);
    }

    /// Run the solver whatever the auto-update switch says.
    fn solve_now(&mut self, ctx: &mut WorkbenchRuntimeContext, feature: &mut SketchFeature) {
        let outcome = solver::solve(&mut feature.sketch);
        self.tab.unsolved_edits = false;
        self.tab.last_solve = Some(outcome);
        self.tab.last_diagnosis = None; // recomputed lazily by the panel
        if let SolveOutcome::NotConverged { residual } = outcome {
            ctx.log_warn(format!(
                "Constraints did not converge (residual {residual:.2e}); check for contradictions"
            ));
        }
    }

    fn clear_interaction_state(&mut self) {
        self.tab.tool_state = ToolState::Idle;
        self.tab.corner_task = None;
        self.tab.selected.clear();
        self.tab.hovered = None;
        self.tab.hover_from_panel = false;
        self.tab.hovered_constraint = None;
        self.tab.cursor = None;
        self.tab.dragging = None;
        self.tab.box_select = None;
        self.tab.trim_stroke = None;
        self.tab.right_press = None;
        self.tab.last_diagnosis = None;
        self.tab.pending_focus = None;
        self.tab.selected_constraints.clear();
        self.tab.label_drag = None;
        self.tab.dim_edit = None;
        self.tab.dim_draft = None;
        self.tab.last_glyph_click = None;
        self.tab.last_geometry_click = None;
        self.tab.last_empty_click = None;
        self.tab.calibrating = None;
        self.tab.dim_capture = DimCapture::default();
        self.tab.armed = None;
    }

    fn sync_active_sketch_from_ctx(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if let Some(feature_id) = ctx.active_document_object
            && self.is_sketch_feature(ctx, feature_id)
            && self.tab.active_sketch_id != Some(feature_id)
        {
            self.tab.active_sketch_id = Some(feature_id);
            self.tab.session_start = ctx.document.get_feature_data(feature_id).cloned();
            self.clear_interaction_state();

            // A generated sketch is centred on where its generator stands.
            if let Some(sketch_feature) = self.get_active_sketch(ctx) {
                let plane = sketch_feature.plane;
                let centre = sketch_feature.generator.is_some().then_some(plane.origin);
                ctx.request(HostRequest::OrientCamera(
                    core_document::CameraOrientRequest {
                        plane_origin: narrow3(plane.origin),
                        plane_normal: narrow3(plane.normal),
                        plane_up: narrow3(plane.y_axis),
                        centre: centre.map(narrow3),
                    },
                ));
            }
        }
    }

    fn is_sketch_feature(&self, ctx: &WorkbenchRuntimeContext, feature_id: FeatureId) -> bool {
        ctx.document
            .get_feature_meta(feature_id)
            .map(|meta| meta.workbench_id.as_str() == crate::KIND)
            .unwrap_or(false)
    }

    /// Delete everything selected, elements and constraints alike, as one
    /// edit.
    fn delete_selected(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        if self.tab.selected.is_empty() && self.tab.selected_constraints.is_empty() {
            return InputResult::ignored();
        }
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let mut doomed: Vec<Uuid> = self
            .tab
            .selected
            .drain()
            .chain(self.tab.selected_constraints.drain())
            .collect();
        doomed.sort();
        let removed = commands::delete_items(&mut feature.sketch, &doomed);
        self.tab.hovered = None;
        if removed == 0 {
            return InputResult::consumed();
        }
        if let Some(sketch_id) = self.tab.active_sketch_id {
            ctx.record(
                "sketch.delete",
                commands::object(serde_json::json!({
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

    /// Whether the edited sketch is in construction mode.
    fn construction_mode(&self) -> bool {
        self.tab
            .active_sketch_id
            .is_some_and(|sketch| self.tab.construction_sketches.contains(&sketch))
    }

    /// The copy count the transform tools take: the copy tool makes one
    /// copy at least (two elements, when the count includes the original).
    fn effective_copies(&self, params: &ToolParams) -> u32 {
        let least = if params.copies_total { 2 } else { 1 };
        if self.tab.copy_mode {
            params.copies.max(least)
        } else {
            params.copies
        }
    }

    /// The `sketch.construction` action. With a selection: flip each
    /// selected element's construction flag individually (mixed selections
    /// end up mixed-inverted). With nothing selected: toggle construction
    /// *mode*, under which all newly drawn geometry is construction.
    fn toggle_construction_selected(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        if self.tab.selected.is_empty() {
            let Some(sketch) = self.tab.active_sketch_id else {
                return InputResult::consumed();
            };
            if !self.tab.construction_sketches.remove(&sketch) {
                self.tab.construction_sketches.insert(sketch);
            }
            ctx.log_info(if self.construction_mode() {
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
            .tab
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
        if let Some(sketch_id) = self.tab.active_sketch_id {
            let mut drawn = drawn.clone();
            drawn.sort();
            if !drawn.is_empty() {
                ctx.record(
                    "sketch.construction",
                    commands::object(serde_json::json!({
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
                    commands::object(serde_json::json!({
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
            (self.get_active_sketch(ctx), self.tab.active_sketch_id)
        else {
            return InputResult::ignored();
        };
        let mut items: Vec<Uuid> = self.tab.selected.iter().copied().collect();
        items.sort();
        if internal::curves_of(&feature.sketch, &items).is_empty() {
            ctx.log_warn("Select an ellipse, a parabola, a hyperbola or a B-spline");
            return InputResult::consumed();
        }
        let (shown, changed) = internal::toggle(&mut feature.sketch, &items, None);
        ctx.record(
            "sketch.internal_geometry",
            commands::object(serde_json::json!({
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
            self.tab.selected.retain(|id| !changed.contains(id));
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
            (self.get_active_sketch(ctx), self.tab.active_sketch_id)
        else {
            return InputResult::ignored();
        };
        let on = !feature.shows_section(self.options.section_on_open);
        feature.set_section(on);
        self.tab.section_view = on;
        ctx.record(
            "sketch.section_view",
            commands::object(serde_json::json!({
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
            (self.get_active_sketch(ctx), self.tab.active_sketch_id)
        else {
            return InputResult::ignored();
        };
        let mut items: Vec<Uuid> = self.tab.selected.iter().copied().collect();
        items.sort();
        let set: HashSet<Uuid> = items.iter().copied().collect();
        let changed = tools::remove_axis_alignment(&mut feature.sketch, &set);
        if changed == 0 {
            ctx.log_warn("None of the selected lines is held horizontal or vertical");
            return InputResult::consumed();
        }
        ctx.record(
            "sketch.remove_axis_alignment",
            commands::object(serde_json::json!({
                "sketch": sketch_id.0.to_string(),
                "items": ids_json(&items),
            })),
            serde_json::json!(changed),
        );
        self.tab
            .selected_constraints
            .retain(|id| feature.sketch.constraints.iter().any(|c| c.id == *id));
        ctx.log_info(format!(
            "Removed axis alignment: {changed} constraint(s) now relative"
        ));
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
        InputResult::consumed()
    }

    /// The constraint list's kind filter (the panel's dropdown; also set by
    /// integration tests).
    pub fn constraint_filter_mut(&mut self) -> &mut ConstraintFilter {
        &mut self.tab.constraint_kind_filter
    }

    /// The constraints the panel's list shows, in sketch order, as its text
    /// and kind filters and the selection leave it.
    pub fn listed_constraints(&self, sketch: &Sketch) -> Vec<Uuid> {
        let text = self.tab.constraint_filter.trim().to_lowercase();
        sketch
            .constraints
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                (text.is_empty()
                    || constraint_row_label(c).to_lowercase().contains(&text)
                    || constraint_operands(sketch, c)
                        .to_lowercase()
                        .contains(&text))
                    && self.tab.constraint_kind_filter.accepts(
                        sketch,
                        c,
                        &self.tab.selected,
                        &self.tab.selected_constraints,
                    )
            })
            .map(|(_, c)| c.id)
            .collect()
    }

    /// Panel-editable tool parameters (also used by integration tests to
    /// set copy counts, offset distances, …).
    pub fn tool_params_mut(&mut self) -> &mut ToolParams {
        &mut self.tab.tool_params
    }
}

impl Workbench for SketchWorkbench {
    fn floating_card(&mut self, ctx: &WorkbenchRuntimeContext) -> Option<bench_api::FloatingCard> {
        self.corner_card(ctx).or_else(|| self.dimension_card(ctx))
    }

    fn card_event(&mut self, ctx: &mut WorkbenchRuntimeContext, event: &bench_api::PanelEvent) {
        if !self.corner_event(ctx, event) {
            self.dimension_event(ctx, event);
        }
    }

    fn tool_history(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        reply: core_document::ToolHistoryReply,
    ) {
        match reply {
            core_document::ToolHistoryReply::Closed(entry) => self.tab.chain_links.closed(entry),
            core_document::ToolHistoryReply::Undone { .. } => {
                if let Some(feature) = self.get_active_sketch(ctx) {
                    self.follow_history(ctx, &feature.sketch);
                }
            }
        }
    }

    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(
            crate::KIND,
            "Sketch",
            "2D sketching environment with constraints and profiles.",
        )
        .icon("workbench-sketcher")
        .feature_kinds([crate::KIND])
        .radial([
            "sketch.line",
            "sketch.polyline",
            "sketch.arc",
            "sketch.circle",
            "sketch.rect",
            "sketch.trim",
            "sketch.constrain.dimension",
            "sketch.constrain.coincident",
        ])
        .modal()
    }

    fn feature_warning(
        &self,
        document: &core_document::Document,
        kernel: Option<&dyn kernel_api::KernelQueries>,
        id: FeatureId,
    ) -> Option<String> {
        let lost = self.lost_external(document, kernel, id).len();
        (lost > 0).then(|| {
            format!(
                "{lost} projected element(s) lost: the edge, face, element or datum they came \
                 from is gone. They stay where they were; edit the sketch to remove or replace \
                 them."
            )
        })
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
        self.tab.pending_creation.is_some() && self.tab.active_sketch_id.is_none()
    }

    fn takes_numeric_input(&self) -> bool {
        self.tab.active_sketch_id.is_some()
            && !ovp::fields_for(&self.tab.tool_state, self.tab.last_tool.as_deref()).is_empty()
    }

    fn takes_text_input(&self) -> bool {
        self.tab.active_sketch_id.is_some() && self.tab.dim_capture.takes_text()
    }

    fn settings_json(&self) -> Option<serde_json::Value> {
        serde_json::to_value(self.options).ok()
    }

    fn apply_settings_json(&mut self, value: &serde_json::Value) {
        if let Ok(options) = serde_json::from_value(value.clone()) {
            self.options = options;
        }
    }

    fn tab_state(&mut self) -> Option<&mut dyn core_document::TabSlot> {
        self.frame.clear();
        Some(&mut self.tab)
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
        commands::run_on(self, id, args, ctx)
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
        // Text an older file stored as outlines, in a font the document
        // keeps, reads its letters from it.
        moved |= crate::text::upgrade_stored(&mut feature.sketch, document);
        if moved {
            *values = feature.to_json();
        }
        moved
    }

    /// A sketch's geometry projected from solids' edges and faces follows
    /// the solids: projected again when what it comes from changed, and
    /// moved in place where it is the same kinds of curve. What comes from
    /// the sketch's own body is what the body's build projected of its
    /// solid where the sketch stands in the history, so the features built
    /// from the sketch never move it; until a build has, a solid built
    /// from the sketch waits. An edge gone or one now another kind of
    /// curve waits for the sketch to be edited.
    fn derive_on_geometry(
        &self,
        node: &core_document::FeatureNode,
        values: &mut serde_json::Value,
        document: &core_document::Document,
        kernel: Option<&dyn kernel_api::KernelQueries>,
    ) -> bool {
        let Ok(mut feature) = SketchFeature::from_json(values) else {
            return false;
        };
        let built = document.projected_references(node.id);
        let groups: Vec<_> = external::groups(&feature.sketch)
            .into_iter()
            .filter(|(source, _)| source.reference.is_none())
            .filter_map(|(source, group)| {
                let answered = (node.body == Some(BodyId(source.body)))
                    .then(|| built?.answer(&feature::projection_probe(&feature.plane, &source)))
                    .flatten()
                    .cloned();
                if answered.is_none() && solid_built_from(document, node.id, BodyId(source.body)) {
                    return None;
                }
                Some((source, group, answered))
            })
            .collect();
        if groups.is_empty() {
            return false;
        }
        let plane = placed_plane(&feature.plane, &sketch_placement(document, node.id));
        let Ok(mut cache) = self.tab.solid_projections.lock() else {
            return false;
        };
        let known = cache.remove(&node.id).unwrap_or_default();
        let mut kept = HashMap::new();
        let mut moved = false;
        for (source, group, answered) in groups {
            let projected = match answered {
                Some(answered) => answered,
                None => {
                    let Some(kernel) = kernel else {
                        continue;
                    };
                    let key = projection_key(document, &plane, &source);
                    let projected = known.get(&key).cloned().unwrap_or_else(|| {
                        project_source_in(document, Some(kernel), &plane, &source)
                    });
                    kept.insert(key, projected.clone());
                    projected
                }
            };
            if let Ok(curves) = &projected
                && !external::already_holds(&feature.sketch, &group, curves)
            {
                let was = feature.sketch.geometry.clone();
                if external::refresh_in_place(&mut feature.sketch, &group, curves) {
                    moved = true;
                } else {
                    feature.sketch.geometry = was;
                }
            }
        }
        cache.insert(node.id, kept);
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
            let plane = feature
                .offset
                .apply(attached.answered(&probed.answers).plane());
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
        let plane = feature.offset.apply(face.plane_at(*point, *normal));
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
        let revision = core_document::data_revision(data);
        // Every region its curves close, shaded on its own.
        let regions = if self.options.shade_regions {
            self.tab.closed_parts.passive_regions(
                id,
                revision,
                &feature,
                REGION_LIFT_MM,
                self.options.appearance.region_opacity,
            )
        } else {
            Vec::new()
        };
        Some(core_document::PassiveGeometry {
            mesh: render::sketch_to_lines(&feature.sketch, &feature.plane),
            revision,
            tint: core_document::PassiveTint::Plain,
            regions,
        })
    }

    fn pick_part(
        &self,
        document: &core_document::Document,
        id: FeatureId,
        _node: &core_document::FeatureNode,
        pick: &core_document::ViewportPick,
        tolerance_px: f32,
    ) -> Option<core_document::PartPick> {
        let data = document.feature_values(id)?;
        let feature = SketchFeature::from_json(data).ok()?;
        self.tab.closed_parts.pick(
            id,
            core_document::data_revision(data),
            &feature,
            pick,
            tolerance_px,
            self.options.shade_regions,
        )
    }

    fn part_lines(
        &self,
        document: &core_document::Document,
        id: FeatureId,
        part: &core_document::FeaturePart,
    ) -> Option<kernel_api::TriMesh> {
        let core_document::FeaturePart::Edge(curve) = part else {
            return None;
        };
        let feature = document.feature_as::<SketchFeature>(id)?;
        render::curve_lines(&feature.sketch, &feature.plane, curve.parse().ok()?)
    }

    /// Its chains of curves (construction left out, as a profile leaves
    /// it) and its points that are no curve's end or centre.
    fn feature_paths(
        &self,
        document: &core_document::Document,
        id: FeatureId,
        _node: &core_document::FeatureNode,
    ) -> Option<core_document::FeaturePaths> {
        let feature = document.feature_as::<SketchFeature>(id)?;
        let sketch = &feature.sketch;
        let chains = profile::extract_chains(sketch).unwrap_or_default();
        let held: HashSet<Uuid> = sketch
            .geometry
            .iter()
            .flat_map(Sketch::curve_point_ids)
            .collect();
        let points: Vec<[f64; 2]> = sketch
            .geometry
            .iter()
            .filter_map(|g| match g {
                GeometryElement::Point(p) if !held.contains(&p.id) => {
                    Some([p.position.x, p.position.y])
                }
                _ => None,
            })
            .collect();
        if chains.is_empty() && points.is_empty() {
            return None;
        }
        Some(core_document::FeaturePaths {
            plane: profile::plane_of(&feature.plane),
            chains,
            points,
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
        let feature = document.feature_as::<SketchFeature>(id)?;
        let (_, pos, px_per_unit) = closed_parts::cursor_in_sketch(&feature, pick)?;
        snap::nearest_curve_distance(&feature.sketch, pos).map(|units| narrow(units) * px_per_unit)
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        self.register_tools(context);
    }

    fn on_activate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        ctx.log_info("Sketch workbench activated");
    }

    fn on_deactivate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        ctx.log_info("Sketch workbench deactivated");
    }

    fn cancel_pointer_gesture(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.cancel_pointer(ctx);
    }

    fn on_input(
        &mut self,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        self.handle_input(event, active_tool, ctx)
    }

    fn has_settings(&self) -> bool {
        true
    }

    fn task(&self, ctx: &WorkbenchRuntimeContext) -> Option<TaskInfo> {
        if self.tab.pending_creation.is_some() {
            return Some(TaskInfo {
                title: "New sketch".to_string(),
                icon: "sketch-new",
                confirmable: false,
                stepwise: false,
                ..Default::default()
            });
        }
        let id = self.tab.active_sketch_id?;
        let name = ctx.document.get_feature_meta(id)?.name.clone();
        let feature = self.frame_displayed(ctx);
        Some(TaskInfo {
            title: name,
            subtitle: feature
                .as_ref()
                .map(|f| crate::style::placement_label(f))
                .unwrap_or_default(),
            status: feature.map(|f| self.task_status(&f.sketch)),
            icon: "sketch-edit",
            // Cancel puts the sketch back as the session found it.
            confirmable: true,
            // Every line drawn or constraint added is a step of its own.
            stepwise: true,
        })
    }

    fn on_frame(&mut self, _dt: f32, ctx: &mut WorkbenchRuntimeContext) {
        // A shape is recorded once it is done: its tool back at rest, or
        // another tool picked.
        let shape_done = self.tab.draw_record.as_ref().is_some_and(|r| {
            self.tab.tool_state.is_idle() || self.tab.last_tool.as_deref() != Some(r.tool.as_str())
        });
        if shape_done {
            self.flush_draw_record(ctx);
        }
        // The picker opens as the bench does, not at the next input.
        self.take_attach_request(ctx);
        self.sync_active_sketch_from_ctx(ctx);
        if self.tab.active_sketch_id.is_some()
            && self.tab.external_refreshed != self.tab.active_sketch_id
        {
            self.tab.external_refreshed = self.tab.active_sketch_id;
            self.refresh_external(ctx);
        }
        if self.tab.last_tool.as_deref() == Some("sketch.external") {
            if self.tab.external_intersection {
                self.take_intersection_picks(ctx);
            } else {
                self.take_external_picks(ctx);
            }
        }
        if self.tab.active_sketch_id.is_none() {
            self.tab.armed = None;
        }
        self.take_armed_picks(ctx);
        self.drop_stale_wall_check(ctx);
        if !chain::is_chain(&self.tab.tool_state) {
            self.tab.chain_links.clear();
        }
        let seq = ctx.document.mutation_seq();
        let from_outside = self
            .tab
            .own_seq
            .swap(seq, std::sync::atomic::Ordering::Relaxed)
            != seq;
        self.tab.selection_shape = match self.get_active_sketch(ctx) {
            Some(feature) => {
                // What was selected may be gone, by an edit here that took
                // more with it (an element's constraints) or one from outside.
                self.tab.selected.retain(|id| {
                    feature.sketch.get_geometry(*id).is_some()
                        || sketch::Reference::of(*id).is_some()
                });
                self.tab
                    .selected_constraints
                    .retain(|id| feature.sketch.constraints.iter().any(|c| c.id == *id));
                if from_outside {
                    // A segment taken back puts the gesture before it.
                    self.follow_chain(&feature.sketch);
                    // The verdict is of a sketch that is not there any more.
                    self.tab.last_solve = None;
                    self.tab.last_diagnosis = None;
                    // An undo may bring back geometry that meets its
                    // constraints or geometry that waits for a solve.
                    self.tab.unsolved_edits =
                        !self.options.auto_update && !solver::holds(&feature.sketch);
                }
                self.tab.section_view = feature.shows_section(self.options.section_on_open);
                let selected: Vec<Uuid> = self.tab.selected.iter().copied().collect();
                self.tab.internal_target =
                    !internal::curves_of(&feature.sketch, &selected).is_empty();
                constrain::SelectionShape::picked_in(
                    &feature.sketch,
                    &self.tab.selected,
                    self.tab.selected.in_order(),
                )
            }
            None => constrain::SelectionShape::default(),
        };
        if self.tab.armable.0.as_ref() != Some(&self.tab.selection_shape) {
            let armable = std::iter::once("dimension")
                .chain(constrain::TOOLS.iter().copied())
                .filter(|which| model_picks::completable(which, &self.tab.selection_shape))
                .collect();
            self.tab.armable = (Some(self.tab.selection_shape.clone()), armable);
        }
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        if self.editing_generated(ctx) {
            return tool_id == "sketch.finish";
        }
        let editing = self.tab.active_sketch_id.is_some();
        if let Some(rest) = tool_id.strip_prefix("sketch.constrain.") {
            let which = tool_variant(tool_id).unwrap_or(rest);
            // A tool the selection does not fit yet arms, to take more picks.
            let armable = self.tab.armable.1.contains(which);
            if which == "dimension" {
                return editing && (dimension_for(&self.tab.selection_shape).is_some() || armable);
            }
            return editing && (constrain::fits(which, &self.tab.selection_shape) || armable);
        }
        match tool_id {
            "sketch.create" => ctx.selected_body_id.is_some() || ctx.active_object_plane.is_some(),
            "sketch.edit" => {
                !editing
                    && ctx
                        .active_document_object
                        .is_some_and(|id| self.is_sketch_feature(ctx, id))
            }
            "sketch.attach" => editing && ctx.selected_face.is_some(),
            "sketch.fit_scan" => editing && !self.scan_guides(ctx).is_empty(),
            "sketch.design_values" => editing,
            "sketch.fit_edge" => {
                editing
                    && self
                        .get_active_sketch(ctx)
                        .is_some_and(|f| !f.sketch.images.is_empty())
            }
            "sketch.save_dxf" | "sketch.save_svg" | "sketch.save_csv" => editing,
            "sketch.array" => editing && !self.tab.selected.is_empty(),
            "sketch.bspline_convert" => {
                editing
                    && self.tab.selection_shape.all.len() > self.tab.selection_shape.points.len()
            }
            "sketch.bspline_degree_up" | "sketch.bspline_degree_down" => {
                editing && !self.tab.selection_shape.others.is_empty()
            }
            "sketch.internal_geometry" => editing && self.tab.internal_target,
            "sketch.remove_axis_alignment" => editing && !self.tab.selection_shape.lines.is_empty(),
            "sketch.join" => {
                editing
                    && self.tab.selection_shape.all.len() - self.tab.selection_shape.points.len()
                        >= 2
            }
            "sketch.toggle_driving" | "sketch.toggle_active" | "sketch.park_constraints" => {
                editing && !self.tab.selected_constraints.is_empty()
            }
            "sketch.select_constraints" => editing && !self.tab.selected.is_empty(),
            "sketch.select_elements" => editing && !self.tab.selected_constraints.is_empty(),
            "sketch.select_conflicting" => {
                editing
                    && self
                        .tab
                        .last_diagnosis
                        .as_ref()
                        .is_some_and(|d| !d.conflicting.is_empty())
            }
            "sketch.select_redundant" => {
                editing
                    && self
                        .tab
                        .last_diagnosis
                        .as_ref()
                        .is_some_and(|d| !d.redundant.is_empty())
            }
            _ => editing,
        }
    }

    fn tool_toggled(&self, tool_id: &str) -> bool {
        match tool_id {
            _ if self.armed_as(tool_id) => true,
            "sketch.construction" => self.construction_mode(),
            "sketch.snap" => !self.tab.snap_off,
            "sketch.show_constraints" => self.options.constraints_hidden,
            "sketch.grid" => self.options.grid_on,
            "sketch.rendering_order" => self.options.construction_on_top,
            "sketch.bspline_comb" => self.options.spline_comb,
            "sketch.bspline_knots" => self.options.spline_knots,
            "sketch.bspline_polygon" => self.options.spline_polygon,
            "sketch.bspline_degree_label" => self.options.spline_degree,
            "sketch.bspline_multiplicity" => self.options.spline_multiplicity,
            "sketch.bspline_weights" => self.options.spline_weights,
            "sketch.parked_layer" => self.tab.parked_layer,
            "sketch.section_view" => self.tab.section_view,
            "sketch.carbon_copy" | "sketch.merge" => {
                self.tab.sketch_picker.as_ref().is_some_and(|p| {
                    (p.mode == SketchPickerMode::Merge) == (tool_id == "sketch.merge")
                })
            }
            _ => false,
        }
    }

    fn editing_feature(&self) -> Option<FeatureId> {
        self.tab.active_sketch_id
    }

    fn finish_editing(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.flush_draw_record(ctx);
        if self.tab.active_sketch_id.is_some() {
            self.tab.active_sketch_id = None;
            self.tab.session_start = None;
            self.tab.sketch_picker = None;
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

    fn viewport(&self, ctx: &WorkbenchRuntimeContext) -> core_document::ViewportOutput {
        core_document::ViewportOutput {
            meshes: self.overlay_meshes(ctx, ctx.active_document_object),
            overlays: self.screen_overlays(ctx, ctx.active_document_object),
            images: self.screen_images(ctx, ctx.active_document_object),
            marks: self.screen_marks(ctx, ctx.active_document_object),
            labels: self.screen_labels(ctx, ctx.active_document_object),
            hud: self.viewport_hud(ctx),
            status: self.status_items(ctx),
            clip_plane: self.clip_plane(ctx),
            view_grid: self.view_grid(ctx),
            ..Default::default()
        }
    }

    /// The edited sketch's reference pictures, laid on its plane under
    /// its lines.
    /// Edges are being found in a reference picture.
    fn busy(&self) -> bool {
        self.tab.picture_edges.running()
    }
}

#[cfg(feature = "egui")]
impl bench_panel::BenchUi for SketchWorkbench {
    /// The sketcher draws nothing under the tree: its UI lives in the
    /// task panel.
    fn ui_left_panel(&mut self, _ui: &mut egui::Ui, ctx: &mut WorkbenchRuntimeContext) {
        self.sync_active_sketch_from_ctx(ctx);
    }

    /// The Sketcher preferences page: the solver, snap and view switches,
    /// the minimum wall, and the palette, read-only.
    fn ui_settings(&mut self, ui: &mut egui::Ui, filter: &str) {
        let mut snap = !self.tab.snap_off;
        let widgets = settings::widgets(&self.options, snap);
        let out = bench_panel::show_filtered(
            ui,
            egui::Id::new("sketch_settings"),
            &widgets,
            None,
            filter,
        );
        for event in &out.events {
            settings::apply(&mut self.options, &mut snap, event);
        }
        self.tab.snap_off = !snap;
    }

    fn ui_task_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: core_document::TaskRequest,
    ) -> core_document::TaskOutcome {
        self.draw_task_panel(ui, ctx, request)
    }
}

impl SketchWorkbench {
    /// Where a click of the tool in hand at `cursor` lands before typed
    /// values place it, and the snap that took it there: the grid, the
    /// angle step and the cue's snap, worked out as the click works them
    /// out. The preview and the on-view card both read it.
    fn click_landing(
        &self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        cursor: Vec2D,
    ) -> (Vec2D, Option<snap::Snap>) {
        let Some(tool) = self.tab.last_tool.as_deref() else {
            return (cursor, None);
        };
        if is_transform_tool(tool) {
            return (self.landing(ctx, feature, tool, cursor).0, None);
        }
        if !is_draw_tool(tool) || self.tab.box_select.is_some() {
            return (cursor, None);
        }
        let (at, tol) = self.landing(ctx, feature, tool, cursor);
        if !self.tab.dim_capture.typed(ctx.document).is_empty() {
            return (at, None);
        }
        let snap = tools::snap_at(
            &self.tab.tool_state,
            &feature.sketch,
            at,
            tol,
            &self.snap_params(),
        );
        (snap.pos, Some(snap))
    }

    /// What a line, polyline or rectangle click at the cursor would add,
    /// as the click resolves and proposes it.
    fn tool_proposal(
        &self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
    ) -> Option<overlay::Proposed> {
        let tool = self.tab.last_tool.as_deref()?;
        let chain = chain::is_chain(&self.tab.tool_state);
        let rect = rect::Kind::of(tool);
        if !chain && rect.is_none() || self.tab.box_select.is_some() {
            return None;
        }
        let (cursor, tol) = self.landing(ctx, feature, tool, self.tab.cursor?);
        let typed = self.tab.dim_capture.typed(ctx.document);
        let sketch = &feature.sketch;
        let cursor = self
            .tab
            .dim_capture
            .steer(&self.tab.tool_state, sketch, cursor);
        let settings = step::StepSettings {
            tol,
            params: ovp::typed_params(&self.click_params(), &self.tab.tool_state, &typed),
            construction: false,
            avoid_redundant: false,
        };
        let (at, tol) = step::land(
            &self.tab.tool_state,
            sketch,
            tool,
            cursor,
            &typed,
            &settings,
        );
        let params = &settings.params;
        if let Some(kind) = rect {
            return match rect::propose(&self.tab.tool_state, sketch, at, tol, kind, params) {
                Ok(proposal) => Some(overlay::Proposed {
                    outlines: rect::outline(&proposal),
                    refused: false,
                }),
                Err(rect::Rejection::NoRoom { outline, .. }) => Some(overlay::Proposed {
                    outlines: vec![outline.into_iter().chain([outline[0]]).collect()],
                    refused: true,
                }),
                Err(_) => None,
            };
        }
        let chain::Resolved::End(end) =
            chain::resolve(&self.tab.tool_state, sketch, at, tol, params)
        else {
            return None;
        };
        let proposal = chain::propose(&self.tab.tool_state, sketch, &end, params).ok()?;
        Some(overlay::Proposed {
            outlines: vec![chain::outline(&proposal, overlay::CIRCLE_SEGMENTS)],
            refused: false,
        })
    }

    /// The settings a tool variant carries (a hexagon's six sides, a
    /// periodic spline), put in the panel when it is picked.
    fn apply_variant_settings(&mut self, tool: &str) {
        match (base_tool_id(tool), tool_variant(tool)) {
            ("sketch.polygon", Some(sides)) => {
                if let Ok(n) = sides.parse::<u32>() {
                    self.tab.tool_params.polygon_sides = n.clamp(3, 12);
                }
            }
            ("sketch.bspline", Some(variant)) => {
                self.tab.tool_params.bspline_periodic =
                    matches!(variant, "periodic" | "through_periodic");
                self.tab.tool_params.bspline_interpolate =
                    matches!(variant, "through" | "through_periodic");
            }
            _ => {}
        }
    }
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

/// Keep the formulas of every text of sketch `sketch` (and take away those
/// of `gone`, a text no longer there) as the commands do.
pub(crate) fn keep_text_formulas(
    ctx: &mut WorkbenchRuntimeContext,
    sketch: Option<FeatureId>,
    gone: Option<Uuid>,
) {
    let Some(sketch) = sketch else {
        return;
    };
    let Some(feature) = placement::stored_sketch(ctx.document, sketch) else {
        return;
    };
    let kept = feature.sketch.texts.iter().map(|b| (b.id, b.text.clone()));
    for (block, text) in kept.chain(gone.map(|b| (b, String::new()))) {
        if let Err(why) = text::keep_formulas(ctx.document, sketch, block, &text) {
            ctx.log_warn(why);
        }
    }
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
    if let Some(asset) = spec.font_asset {
        args["font_asset"] = serde_json::json!(asset.to_string());
    }
    let word = |v: &dyn std::fmt::Debug| format!("{v:?}").to_ascii_lowercase();
    if spec.align != text::Align::Start {
        args["align"] = serde_json::json!(word(&spec.align));
    }
    if spec.valign != text::VAlign::Baseline {
        args["valign"] = serde_json::json!(word(&spec.valign));
    }
    if spec.line_spacing != 1.0 {
        args["line_spacing"] = serde_json::json!(spec.line_spacing);
    }
    match &spec.layout {
        text::TextLayout::Point => {}
        text::TextLayout::Frame { wrap, fit, .. } => {
            args["wrap"] = serde_json::json!(wrap);
            args["fit"] = serde_json::json!(fit);
        }
        text::TextLayout::Path {
            side,
            offset,
            fit,
            reversed,
            ..
        } => {
            args["side"] = serde_json::json!(word(side));
            args["offset"] = serde_json::json!(offset);
            args["fit"] = serde_json::json!(fit);
            args["reversed"] = serde_json::json!(reversed);
        }
    }
    if let Some(at) = at {
        args["at"] = serde_json::json!([at.x, at.y]);
    }
    args
}

/// Tools that create geometry from clicks, for which object snapping can
/// be switched off.
/// What [`SketchWorkbench::loupe`] draws.
struct Loupe {
    image: core_document::ScreenSpaceImage,
    /// Where the snapped point shows in it, viewport pixels.
    middle: [f32; 2],
    /// The picture's edges inside it, magnified with it.
    edges: Vec<([f32; 2], [f32; 2])>,
}

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

/// How far past the sketch plane a section view keeps, in mm: a face the
/// sketch lies on stays whole.
const SECTION_MARGIN_MM: f64 = 1e-3;

/// The renderer's clipping equation for a section view at `plane`: it
/// keeps what lies on the far side from `eye`, and the plane itself.
pub(crate) fn section_equation(plane: &SketchPlane, eye: [f64; 3]) -> [f64; 4] {
    let n = glam::DVec3::from_array(plane.normal).normalize_or_zero();
    let o = glam::DVec3::from_array(plane.origin);
    // The side the viewer is on is the side cut away.
    let toward_eye = if n.dot(glam::DVec3::from_array(eye) - o) >= 0.0 {
        n
    } else {
        -n
    };
    let keep = -toward_eye;
    [keep.x, keep.y, keep.z, -keep.dot(o) + SECTION_MARGIN_MM]
}

/// A sketch feature named `name` added to `doc` in `body`.
#[cfg(test)]
fn add_sketch(
    doc: &mut core_document::Document,
    sketch: Sketch,
    plane: SketchPlane,
    name: &str,
    body: Option<BodyId>,
) -> FeatureId {
    let feature = SketchFeature::new(sketch, plane);
    doc.add_feature_in_body(feature, name.into(), body).unwrap()
}

/// The tests' reading of a sketch's elements by kind: `$all` lists them,
/// `$first` takes the first.
#[cfg(test)]
pub(crate) mod test_kinds {
    use crate::sketch::{self, GeometryElement, Sketch};

    macro_rules! kind {
        ($all:ident, $variant:ident $(, $first:ident)?) => {
            pub(crate) fn $all(sketch: &Sketch) -> Vec<sketch::$variant> {
                let of = |g: &GeometryElement| match g {
                    GeometryElement::$variant(e) => Some(e.clone()),
                    _ => None,
                };
                sketch.geometry.iter().filter_map(of).collect()
            }
            $(pub(crate) fn $first(sketch: &Sketch) -> sketch::$variant {
                $all(sketch)
                    .into_iter()
                    .next()
                    .expect("an element of the kind")
            })?
        };
    }
    kind!(all_points, Point);
    kind!(all_lines, Line, line_of);
    kind!(all_arcs, Arc, arc_of);
    kind!(all_circles, Circle, circle_of);
    kind!(all_ellipses, Ellipse, ellipse_of);
    kind!(all_splines, BSpline, spline_of);
    kind!(all_conics, Conic, conic_of);
}

#[cfg(test)]
mod constraint_filter {
    use super::*;
    use crate::actions::{constraints_on, elements_of};
    use sketch::{ConstraintKind, Line, Point};

    #[test]
    fn from_elements_to_their_constraints_and_back() {
        let mut sketch = Sketch::new("t");
        let point = |sketch: &mut Sketch, x: f64| {
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
        let upright = sketch.add_constraint(ConstraintKind::Vertical { element: first });
        // The first line's own point carries the distance: it counts.
        let on_first = constraints_on(&sketch, &HashSet::from([first]));
        assert!(on_first.contains(&pinned) && !on_first.contains(&level));
        assert_eq!(on_first.len(), 2, "its vertical and its point's distance");
        let held = elements_of(&sketch, &HashSet::from([level]));
        assert_eq!(held, [second]);
        // The panel's badges and related rows read the same ties.
        let relations = relations::Relations::of(&sketch);
        assert_eq!(relations.constraints_on(first), [pinned, upright]);
        assert_eq!(relations.constraints_on(a), [pinned]);
        assert!(relations.constraints_on(b).is_empty());
        let set = |ids: &[Uuid]| ids.iter().copied().collect::<HashSet<Uuid>>();
        // A constraint on a point lights the point and the curve it ends.
        let to_pinned = relations.related([], [pinned]);
        assert_eq!(to_pinned.elements, set(&[a, first]));
        assert!(to_pinned.constraints.is_empty());
        // A corner lights the curves meeting there; a curve its points,
        // its neighbours and its constraints.
        assert_eq!(relations.related([b], []).elements, set(&[first, second]));
        let to_first = relations.related([first], []);
        assert_eq!(to_first.elements, set(&[a, b, second]));
        assert_eq!(to_first.constraints, set(&[pinned, upright]));
    }

    #[test]
    fn each_filter_lists_its_own() {
        let mut sketch = Sketch::new("t");
        let point = |sketch: &mut Sketch, x: f64| {
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
        let point = |sketch: &mut Sketch, x: f64, y: f64| {
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

#[cfg(test)]
mod section_view {
    use super::*;

    fn keeps([a, b, c, d]: [f64; 4], p: [f64; 3]) -> bool {
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
        let sketch = add_sketch(
            &mut doc,
            Sketch::new("Sketch001"),
            SketchPlane::xy(),
            "Sketch001",
            None,
        );
        let mut wb = SketchWorkbench {
            tab: crate::Tab {
                active_sketch_id: Some(sketch),
                ..Default::default()
            }
            .into(),
            ..Default::default()
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
mod scan_section {
    use super::*;
    use core_document::Document;

    /// The plane cuts every scan as a 10 × 6 rectangle about the plane's
    /// origin, five points a side, which the fit gives back as the
    /// polyline through the corners.
    struct Rectangle;

    impl kernel_api::KernelQueries for Rectangle {
        fn project_edge(
            &self,
            _brep: &[u8],
            _near: [f64; 3],
            _plane: &kernel_api::ProfilePlane,
        ) -> kernel_api::KernelResult<kernel_api::ProjectedEdge> {
            Err(kernel_api::KernelError::Unsupported("project_edge".into()))
        }

        fn mesh_sections(
            &self,
            _mesh: &kernel_api::TriMesh,
            o: [f64; 3],
            _normal: [f64; 3],
            _offsets: &[f64],
        ) -> kernel_api::KernelResult<Vec<Vec<kernel_api::SectionChain>>> {
            let at = |x: f64, y: f64| [o[0] + x, o[1] + y, o[2]];
            let corners = [(0.0, 0.0), (10.0, 0.0), (10.0, 6.0), (0.0, 6.0)];
            let points: Vec<[f64; 3]> = (0..4)
                .flat_map(|i| {
                    let ((x0, y0), (x1, y1)) = (corners[i], corners[(i + 1) % 4]);
                    (0..5).map(move |k| {
                        let t = f64::from(k) / 5.0;
                        (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t)
                    })
                })
                .map(|(x, y)| at(x, y))
                .collect();
            Ok(vec![vec![kernel_api::SectionChain {
                outward: vec![[0.0; 3]; points.len()],
                points,
                closed: true,
            }]])
        }

        fn fit_chain_2d(
            &self,
            points: &[[f64; 2]],
            fit: &kernel_api::ChainFit,
        ) -> kernel_api::KernelResult<kernel_api::DrawingShape> {
            Ok(kernel_api::DrawingShape::Polyline {
                points: points.iter().step_by(5).copied().collect(),
                bulges: Vec::new(),
                closed: fit.closed,
            })
        }
    }

    /// A sketch through a scan fits the section into a closed profile of
    /// its own lines, in the sketch's coordinates, each keeping the slice
    /// points along it, the slice then reading as on them; with no scan the
    /// command refuses.
    #[test]
    fn a_scans_section_fits_into_a_closed_profile() {
        let mut doc = Document::new("t");
        let scan = doc.create_body(Some("Scan".into()));
        let asset = doc.add_asset(core_document::AssetReference::new(
            "assets/scan.stl",
            core_document::AssetType::Stl,
            serde_json::Value::Null,
        ));
        doc.set_imported_geometry(
            scan,
            core_document::ImportedGeometry {
                mesh: std::sync::Arc::new(kernel_api::TriMesh {
                    positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                    indices: vec![0, 1, 2],
                    ..Default::default()
                }),
                source_asset: Some(asset),
                revision: 0,
                bounds_mm: None,
                brep_blob_path: None,
                mesh_path: None,
                face_colors_path: None,
                health: None,
            },
        );
        let sketch = add_sketch(&mut doc, Sketch::new("S"), SketchPlane::xy(), "S", None);
        let mut wb = SketchWorkbench::default();
        let args = commands::object(serde_json::json!({"sketch": sketch.0.to_string()}));
        let mut ctx = core_document::testing::ctx(&mut doc);
        ctx.kernel = Some(&Rectangle);
        assert!(
            wb.run_command("sketch.fit_scan", &args, &mut ctx).is_err(),
            "not a scan yet"
        );
        ctx.document
            .set_body_scan(scan, Some(core_document::ScanInfo::default()));
        let answer = wb.run_command("sketch.fit_scan", &args, &mut ctx).unwrap();
        assert_eq!(
            (answer["chains"].clone(), answer["lines"].clone()),
            (serde_json::json!(1), serde_json::json!(4))
        );
        let stored = stored_sketch(ctx.document, sketch).unwrap().sketch;
        let wires = profile::extract_wires(&stored).expect("a closed profile");
        // Each line keeps the slice points along it, which settle what its
        // constraints leave free.
        assert_eq!(stored.soft.len(), 4);
        // The slice reads as fitting where it lies on the curves.
        let off = scan::distance_to_sketch(&stored);
        assert!(off([5.0, 0.0]).unwrap() < 1e-9 && (off([5.0, 0.3]).unwrap() - 0.3).abs() < 1e-9);
        assert_eq!(
            scan::distance_to_sketch(&Sketch::new("Empty"))([0.0; 2]),
            None
        );
        assert!(stored.soft.iter().all(|d| d.points.len() >= 4));
        assert_eq!(wires.len(), 1);
        let xs: Vec<f64> = crate::test_kinds::all_points(&stored)
            .into_iter()
            .map(|p| p.position.x)
            .collect();
        assert!(xs.iter().any(|x| (x - 10.0).abs() < 1e-9) && xs.iter().any(|x| x.abs() < 1e-9));
    }

    /// Create sketch with a feature offering a plane selected puts the new
    /// sketch there, in a body of its own.
    #[test]
    fn create_sketch_takes_the_plane_a_feature_offers() {
        let mut doc = Document::new("t");
        let mut wb = SketchWorkbench::default();
        let mut ctx = core_document::testing::ctx(&mut doc);
        let plane = kernel_api::ProfilePlane {
            origin: [0.0, 0.0, 7.0],
            x_axis: [0.0, 1.0, 0.0],
            y_axis: [-1.0, 0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
        };
        ctx.active_object_plane = Some(plane);
        assert!(wb.is_tool_enabled("sketch.create", &ctx));
        wb.on_input(
            &WorkbenchInputEvent::ToolActivated,
            Some("sketch.create"),
            &mut ctx,
        );
        let made = wb.tab.active_sketch_id.expect("a sketch is open");
        let stored = stored_sketch(ctx.document, made).unwrap();
        assert_eq!(
            (stored.plane.origin, stored.plane.x_axis),
            (plane.origin, plane.x_axis)
        );
        assert!(ctx.document.get_feature_meta(made).unwrap().body.is_some());
    }
}

#[cfg(test)]
mod passive_regions {
    use super::*;
    use core_document::Document;
    use sketch::{Circle, Line, Point};

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
        let id = add_sketch(&mut doc, sketch, SketchPlane::xy(), "t", None);
        (doc, id)
    }

    /// A document holding one sketch: a circle of radius 10 with one of
    /// radius 4 inside it, about the origin, and the two circles' ids.
    fn nested_circles() -> (Document, FeatureId, Uuid, Uuid) {
        let mut sketch = Sketch::new("n");
        let centre = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let outer = sketch.add_geometry(GeometryElement::Circle(Circle::new(centre, 10.0)));
        let inner = sketch.add_geometry(GeometryElement::Circle(Circle::new(centre, 4.0)));
        let mut doc = Document::new("n");
        let id = add_sketch(&mut doc, sketch, SketchPlane::xy(), "n", None);
        (doc, id, outer, inner)
    }

    fn regions(
        bench: &SketchWorkbench,
        doc: &Document,
        id: FeatureId,
    ) -> Vec<core_document::PassiveRegion> {
        let node = doc.get_feature_meta(id).unwrap();
        bench.passive_geometry(doc, id, node).unwrap().regions
    }

    /// A closed sketch offers each region to shade on its own while
    /// regions are shaded, the ones a plain profile takes stronger (the
    /// ring) than the rest (the disc it holds); an open one shades
    /// nothing.
    #[test]
    fn a_closed_sketch_shades_every_region_the_plain_profile_stronger() {
        let (doc, id) = triangle(true);
        let mut bench = SketchWorkbench::default();
        let shaded = regions(&bench, &doc, id);
        assert_eq!(shaded.len(), 1);
        assert_eq!(shaded[0].profile.wires.len(), 1);
        assert!(
            shaded[0].profile.plane.origin[2] > 0.0,
            "a hair off the plane"
        );
        let (doc, id, ..) = nested_circles();
        let shaded = regions(&bench, &doc, id);
        let mut by_wires: Vec<(usize, f32)> = shaded
            .iter()
            .map(|r| (r.profile.wires.len(), r.opacity))
            .collect();
        by_wires.sort_by_key(|(wires, _)| *wires);
        let [(1, disc), (2, ring)] = by_wires[..] else {
            panic!("a disc and a ring with its hole: {by_wires:?}");
        };
        assert!(ring > disc && disc > 0.0, "ring {ring}, disc {disc}");
        assert!(shaded.iter().all(|r| r.part.is_some()), "each pickable");
        let (open, open_id) = triangle(false);
        assert!(regions(&bench, &open, open_id).is_empty(), "open: none");
        bench.options.shade_regions = false;
        assert!(regions(&bench, &doc, id).is_empty());
    }

    /// Looking down on the sketch, the cursor over the inner disc picks
    /// the disc, over the ring the ring, on a circle that circle, whose
    /// lines the view can draw; away from it all, nothing.
    #[test]
    fn the_cursor_picks_a_region_or_a_curve() {
        use core_document::{FeaturePart, ViewportPick};
        let (doc, id, outer, _) = nested_circles();
        let bench = SketchWorkbench::default();
        let node = doc.get_feature_meta(id).unwrap();
        let view_proj =
            glam::camera::rh::proj::directx::orthographic(-20.0, 20.0, -20.0, 20.0, -100.0, 100.0)
                * glam::camera::rh::view::look_at_mat4(
                    glam::Vec3::Z * 50.0,
                    glam::Vec3::ZERO,
                    glam::Vec3::Y,
                );
        let viewport = (0, 0, 400, 400);
        let at = |x: f32, y: f32| {
            let view_proj = view_proj.to_cols_array_2d();
            let cursor =
                core_document::runtime::world_to_viewport(view_proj, viewport, [x, y, 0.0])
                    .unwrap();
            let pick = ViewportPick {
                view_proj,
                viewport,
                cursor,
            };
            bench.pick_part(&doc, id, node, &pick, 8.0).map(|p| p.part)
        };
        let names: Vec<String> = profile::regions(
            &SketchFeature::from_json(doc.feature_values(id).unwrap())
                .unwrap()
                .sketch,
        )
        .unwrap()
        .into_iter()
        .map(|r| r.name)
        .collect();
        let Some(FeaturePart::Region(disc)) = at(1.0, 1.0) else {
            panic!("the disc");
        };
        let Some(FeaturePart::Region(ring)) = at(7.0, 0.0) else {
            panic!("the ring");
        };
        assert!(disc != ring && names.contains(&disc) && names.contains(&ring));
        let edge = FeaturePart::Edge(outer.to_string());
        assert_eq!(at(0.0, 10.0), Some(edge.clone()), "on the outer circle");
        assert_eq!(at(15.0, 15.0), None);
        let lines = bench.part_lines(&doc, id, &edge).expect("its lines");
        assert!(!lines.edges.is_empty());
        assert!(
            bench
                .part_lines(&doc, id, &FeaturePart::Region(disc))
                .is_none()
        );
    }
}
