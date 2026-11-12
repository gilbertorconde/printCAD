// Without the egui feature the crate is the headless subset the tests and
// the kernel bench use; the panel-only state and helpers go unused there.
#![cfg_attr(not(feature = "egui"), allow(dead_code))]

/// The feature kind sketches are stored under.
pub const KIND: &str = "wb.sketch";

mod appearance;
pub mod boundary;
mod candidates;
mod chain;
mod closed_parts;
mod commands;
pub mod conic;
mod constrain;
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
mod walls;

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use boundary::{narrow, narrow3, widen3};
use core_document::{
    BodyId, FeatureId, FeatureInfo, HostRequest, InputResult, MenuItem, MenuScope,
    ScreenSpaceLabel, ScreenSpaceMark, SketchPalette, StatusItems, TaskInfo, ToolDescriptor,
    ToolHint, ToolVariant, ViewportHud, Workbench, WorkbenchContext, WorkbenchDescriptor,
    WorkbenchFeature, WorkbenchInputEvent, WorkbenchRuntimeContext, base_tool_id, tool_variant,
};
pub use dim_edit::{DimEdit, SaveAs};
pub use feature::{AttachedSupport, DatumSupport, FaceSupport, LentFace, SketchFeature};
use operations::SolveOutcome;
use overlay::SketchProjector;
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
    /// Edited with automatic update off: not solved yet.
    Unsolved,
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
            SolverKind::Redundant | SolverKind::Unsolved => pal.constraint,
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
    ("sketch.view_sketch", "V"),
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
    ("sketch.constrain.group", "Shift+G"),
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
        "sketch.text" => (
            "Text",
            "Click where the text starts, a frame's two corners, or the curve to follow",
        ),
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
            commands::args(serde_json::json!({
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
        if id == "sketch.wall_thickness" {
            return self.wall_thickness_command(args, ctx);
        }
        if matches!(
            id,
            "sketch.export_dxf" | "sketch.export_svg" | "sketch.export_csv"
        ) {
            return commands::export(id, args, ctx, &self.tab.picture_edges);
        }
        if id == "sketch.fit_edge" {
            return commands::fit_edge(args, ctx, &self.tab.picture_edges);
        }
        if id == "sketch.calibrate_image" {
            return commands::calibrate_image(args, ctx, &self.tab.picture_edges);
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
        commands::register(context);
        generator::register(context);
        context.register_action(
            core_document::ActionDescriptor::new(
                POLYLINE_ARC_ACTION,
                "Polyline: switch between line and arc",
            )
            .category("create.basic")
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
                "sketch.select" | "sketch.point" | "sketch.line" => "create.basic",
                "sketch.arc" | "sketch.circle" | "sketch.ellipse" | "sketch.bspline" => {
                    "create.curves"
                }
                "sketch.rect" | "sketch.polygon" | "sketch.slot" => "create.shapes",
                "sketch.text" => "create.text",
                "sketch.fillet" | "sketch.trim" | "sketch.extend" | "sketch.split" => "modify.edit",
                _ => "modify.transform",
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
                        Some("reference.external"),
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
                        Some("reference.external"),
                    )
                    .icon("clone-geometry")
                    .row(1),
                );
                context.register_tool(
                    ToolDescriptor::new_action(
                        "sketch.external_defining",
                        "Toggle external geometry in profiles",
                        Some("reference.external"),
                    )
                    .icon("external-defining")
                    .row(1),
                );
                context.register_tool(
                    ToolDescriptor::new_action(
                        "sketch.carbon_copy",
                        "Carbon copy",
                        Some("reference.external"),
                    )
                    .icon("carbon-copy")
                    .row(1),
                );
                for (id, label, icon) in [
                    ("sketch.select_origin", "Select origin", "select-origin"),
                    (
                        "sketch.select_x_axis",
                        "Select horizontal axis",
                        "select-horizontal-axis",
                    ),
                    (
                        "sketch.select_y_axis",
                        "Select vertical axis",
                        "select-vertical-axis",
                    ),
                ] {
                    context.register_tool(
                        ToolDescriptor::new_action(id, label, Some("reference.external"))
                            .icon(icon)
                            .row(1),
                    );
                }
                context.register_tool(
                    ToolDescriptor::new_action(
                        "sketch.design_values",
                        "Dimension at design values",
                        Some("reference.fit"),
                    )
                    .icon("measure")
                    .row(1),
                );
                context.register_tool(
                    ToolDescriptor::new(
                        "sketch.fit_edge",
                        "Fit to a picture's edge",
                        Some("reference.fit"),
                    )
                    .icon("fit-selection")
                    .row(1),
                );
                context.register_tool(
                    ToolDescriptor::new_action(
                        "sketch.fit_scan",
                        "Fit the scan's section",
                        Some("reference.fit"),
                    )
                    .icon("fit-scan")
                    .row(1),
                );
                context.register_tool(keyed(
                    ToolDescriptor::new_action(
                        "sketch.construction",
                        "Construction",
                        Some("create.construction"),
                    )
                    .icon("construction-mode")
                    .row(1),
                ));
                context.register_tool(keyed(
                    ToolDescriptor::new_action(
                        "sketch.internal_geometry",
                        "Show/hide internal geometry",
                        Some("create.construction"),
                    )
                    .icon("show-hide-internal-geometry")
                    .row(1),
                ));
                continue;
            }
            if *id == "sketch.point" {
                context.register_tool(tool);
                context.register_tool(keyed(
                    ToolDescriptor::new("sketch.polyline", "Polyline", Some("create.basic"))
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
                Some("modify.transform"),
            )
            .icon("rectangular-array")
            .row(1),
        );
        context.register_tool(
            ToolDescriptor::new_action("sketch.join", "Join curves", Some("modify.edit"))
                .icon("join-curves")
                .row(1),
        );
        for (id, label, icon) in [
            (
                "sketch.bspline_convert",
                "Convert to B-spline",
                "bspline-convert",
            ),
            (
                "sketch.bspline_degree_up",
                "Raise spline degree",
                "bspline-degree-up",
            ),
            (
                "sketch.bspline_degree_down",
                "Lower spline degree",
                "bspline-degree-down",
            ),
        ] {
            context.register_tool(
                ToolDescriptor::new_action(id, label, Some("modify.bspline"))
                    .icon(icon)
                    .row(1),
            );
        }
        context.register_tool(
            ToolDescriptor::new("sketch.bspline_knot", "Insert knot", Some("modify.bspline"))
                .icon("bspline-knot")
                .row(1),
        );
        for (id, label, icon) in [
            ("sketch.bspline_comb", "Curvature comb", "bspline-comb"),
            ("sketch.bspline_knots", "Show knots", "bspline-polygon"),
            (
                "sketch.bspline_polygon",
                "Control polygon",
                "bspline-control-polygon",
            ),
            (
                "sketch.bspline_degree_label",
                "Show degree",
                "bspline-degree-label",
            ),
            (
                "sketch.bspline_multiplicity",
                "Show knot multiplicity",
                "bspline-multiplicity",
            ),
            ("sketch.bspline_weights", "Show weights", "bspline-weight"),
        ] {
            context.register_tool(
                ToolDescriptor::new_action(id, label, Some("modify.bspline"))
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
                ToolDescriptor::new_action(id, label, Some("modify.delete"))
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
            ("group", "Group", "tree-group"),
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
            "dimensions",
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
            let mut tool = constraint(id, label, icon, "dimensions");
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
                ToolDescriptor::new_action(id, label, Some("inspect.select"))
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
                "select-constraints",
            ),
            (
                "sketch.select_elements",
                "Elements of the selected constraints",
                "select-elements",
            ),
        ] {
            context.register_tool(
                ToolDescriptor::new_action(id, label, Some("inspect.select"))
                    .icon(icon)
                    .row(2),
            );
        }
        context.register_tool(
            ToolDescriptor::new_action(
                "sketch.show_constraints",
                "Show/hide constraints",
                Some("view.display"),
            )
            .icon("show-hide-constraints")
            .row(2),
        );
        context.register_tool(keyed(
            ToolDescriptor::new_action("sketch.view_sketch", "View sketch", Some("view.display"))
                .icon("sketch-view")
                .row(2),
        ));
        context.register_tool(
            ToolDescriptor::new_action("sketch.section_view", "Section view", Some("view.display"))
                .icon("clipping-plane")
                .row(2),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                "sketch.parked_layer",
                "Show parked constraints",
                Some("view.display"),
            )
            .icon("eye")
            .row(2),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                "sketch.rendering_order",
                "Rendering order",
                Some("view.display"),
            )
            .icon("rendering-order")
            .row(2),
        );
        context.register_tool(
            ToolDescriptor::new_action("sketch.grid", "Grid", Some("view.snap"))
                .icon("grid")
                .row(2),
        );
        context.register_tool(
            ToolDescriptor::new_action("sketch.snap", "Snap to objects", Some("view.snap"))
                .icon("snap")
                .row(2),
        );
        // Checks and file exports, last in their panels.
        let check = |id: &str, label: &str, icon: &'static str| {
            let category = if id.starts_with("sketch.save_") {
                "sketch.export"
            } else {
                "inspect.check"
            };
            ToolDescriptor::new_action(id, label, Some(category))
                .icon(icon)
                .row(0)
        };
        context.register_tool(check(
            "sketch.validate",
            "Validate sketch",
            "check-geometry",
        ));
        context.register_tool(check(
            "sketch.repair",
            "Repair sketch: join near ends, drop empty and doubled curves",
            "sketch-validate",
        ));
        context.register_tool(check(
            "sketch.wall_thickness",
            "Check wall thickness",
            "thickness",
        ));
        context.register_tool(check("sketch.save_dxf", "Save as DXF…", "save"));
        context.register_tool(check("sketch.save_svg", "Save as SVG…", "save"));
        context.register_tool(check("sketch.save_csv", "Save as CSV…", "save"));
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
        self.sync_active_sketch_from_ctx(ctx);
        let base = active_tool.map(base_tool_id);

        if base == Some("sketch.finish") {
            if self.tab.active_sketch_id.is_some() {
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
            // A feature offering a plane (a scan's section) takes the new
            // sketch, in a body of its own.
            if let Some(p) = ctx.active_object_plane
                && self.tab.pending_creation.is_none()
                && self.tab.active_sketch_id.is_none()
            {
                let body = ctx.document.create_body(None);
                let plane = SketchPlane {
                    origin: p.origin,
                    normal: p.normal,
                    x_axis: p.x_axis,
                    y_axis: p.y_axis,
                };
                self.create_sketch_on_plane(ctx, Some(body), plane, NewSketchOn::default());
                return InputResult::consumed();
            }
            if self.tab.pending_creation.is_none() && self.tab.active_sketch_id.is_none() {
                // A datum selected in the tree is offered first, in its body.
                let datum = ctx.active_document_object.and_then(|id| {
                    let node = ctx.document.get_feature_meta(id)?;
                    (node.workbench_id.as_str() == core_document::DATUM_KIND)
                        .then_some((id, node.body))
                });
                let body = ctx
                    .selected_body_id
                    .map(BodyId)
                    .or_else(|| datum.and_then(|(_, body)| body));
                // A face of the selected body's own solid is followed.
                let (face, origin) = match body.and_then(|b| ctx.selected_face_in(b)) {
                    Some(face) => (Some(face), core_document::FaceOrigin::OwnSolid),
                    None => (ctx.selected_face, core_document::FaceOrigin::Elsewhere),
                };
                self.begin_sketch_creation(body, face, origin, datum.map(|(id, _)| id));
            }
            return InputResult::consumed();
        }

        if base == Some("sketch.edit") {
            // The sync above already adopted a sketch selected in the tree.
            if self.tab.active_sketch_id.is_none() {
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

        if self.tab.active_sketch_id.is_none() {
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
                // Pressed again while armed, the tool is put away.
                if self.armed_as(base) {
                    self.tab.armed = None;
                    ctx.log_info("Constraint cancelled");
                    return InputResult::consumed();
                }
                return self.apply_constraint_tool(ctx, tool, which);
            }
            match base {
                "sketch.construction" => return self.toggle_construction_selected(ctx),
                "sketch.external_defining" => return self.toggle_external_defining(ctx),
                "sketch.internal_geometry" => return self.toggle_internal_geometry(ctx),
                "sketch.remove_axis_alignment" => return self.remove_axis_alignment(ctx),
                "sketch.snap" => {
                    self.tab.snap_off = !self.tab.snap_off;
                    return InputResult::consumed();
                }
                "sketch.view_sketch" => {
                    if let Some(feature) = self.get_active_sketch(ctx) {
                        Self::view_sketch(ctx, &feature);
                    }
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
                "sketch.fit_scan" => return self.fit_scan(ctx),
                "sketch.design_values" => return self.design_values(ctx),
                "sketch.save_dxf" | "sketch.save_svg" | "sketch.save_csv" => {
                    let kind = base.trim_start_matches("sketch.save_").to_ascii_uppercase();
                    return self.save_drawing(ctx, &kind);
                }
                "sketch.wall_thickness" => return self.check_walls(ctx),
                "sketch.select_malformed" => return self.select_malformed(ctx),
                "sketch.select_unconstrained" => return self.select_free(ctx, true),
                "sketch.select_dof" => return self.select_free(ctx, false),
                "sketch.select_constraints" => return self.select_constraints_of(ctx),
                "sketch.select_elements" => return self.select_elements_of(ctx),
                "sketch.select_origin" => {
                    return self.select_reference(ctx, sketch::ORIGIN_ID, "origin");
                }
                "sketch.select_x_axis" => {
                    return self.select_reference(ctx, sketch::X_AXIS_ID, "horizontal axis");
                }
                "sketch.select_y_axis" => {
                    return self.select_reference(ctx, sketch::Y_AXIS_ID, "vertical axis");
                }
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
                "sketch.bspline_polygon" => {
                    self.options.spline_polygon = !self.options.spline_polygon;
                    return InputResult::consumed();
                }
                "sketch.bspline_degree_label" => {
                    self.options.spline_degree = !self.options.spline_degree;
                    return InputResult::consumed();
                }
                "sketch.bspline_multiplicity" => {
                    self.options.spline_multiplicity = !self.options.spline_multiplicity;
                    return InputResult::consumed();
                }
                "sketch.bspline_weights" => {
                    self.options.spline_weights = !self.options.spline_weights;
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
                    self.tab.selected_constraints.clear();
                    return result;
                }
                "sketch.section_view" => return self.toggle_section_view(ctx),
                "sketch.parked_layer" => {
                    self.tab.parked_layer = !self.tab.parked_layer;
                    ctx.log_info(if self.tab.parked_layer {
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
        // An open corner's card has the keys (the host keeps Enter and
        // Escape for its OK and Cancel, and applies it before a click in
        // the view goes on); another tool, or an edit from elsewhere, puts
        // it away.
        if self.tab.corner_task.is_some() {
            if active_tool.map(str::to_string) != self.tab.activated_tool
                || self.drop_stale_corner(ctx)
            {
                self.discard_corner();
            } else {
                return match event {
                    WorkbenchInputEvent::KeyPress { .. }
                    | WorkbenchInputEvent::Text { .. }
                    | WorkbenchInputEvent::MousePress {
                        button: core_document::MouseButton::Left,
                        ..
                    }
                    | WorkbenchInputEvent::MouseRelease {
                        button: core_document::MouseButton::Left,
                        ..
                    } => InputResult::consumed(),
                    _ => InputResult::ignored(),
                };
            }
        }
        if active_tool.map(str::to_string) != self.tab.activated_tool {
            self.tab.activated_tool = active_tool.map(str::to_string);
            self.tab.wall_check = None;
            if !self.tab.tool_state.is_idle() {
                self.tab.tool_state = ToolState::Idle;
            }
            self.tab.dim_capture.clear_buffers();
            let tab = &mut *self.tab;
            tab.dim_capture.sync(&tab.tool_state, active_tool);
            self.flush_draw_record(ctx);
            if let Some(tool) = active_tool {
                self.apply_variant_settings(tool);
            }
        }
        let canonical = active_tool.and_then(canonical_tool);
        let tool = canonical.as_deref();
        self.tab.copy_mode = base == Some("sketch.copy");
        // Remember the tool so the task panel can surface its settings
        // (polygon sides, slot width, fillet radius).
        if self.tab.last_tool.as_deref() != tool {
            self.tab.last_tool = tool.map(str::to_string);
            self.tab.external_seen.clear();
        }
        // External geometry: a click is the host's, which picks the solid
        // edge under it; the frame hook projects what was picked.
        if base == Some("sketch.external") {
            self.tab.last_tool = Some("sketch.external".to_string());
            let intersection = active_tool.and_then(tool_variant) == Some("intersection");
            if intersection != self.tab.external_intersection {
                self.tab.external_intersection = intersection;
                self.tab.external_seen.clear();
            }
            return match event {
                WorkbenchInputEvent::KeyPress { key } => self.handle_key_press(ctx, tool, *key),
                _ => InputResult::ignored(),
            };
        }

        // An armed constraint tool takes its picks while the view is in
        // select mode; any other tool puts it away.
        if self.tab.armed.is_some() {
            if tool.is_some_and(|t| t != "sketch.select") {
                self.tab.armed = None;
            } else if let Some(result) = self.armed_input(ctx, event) {
                return result;
            }
        }

        // Fit to a picture's edge: a click fits the edge under it.
        if base == Some("sketch.fit_edge")
            && let WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                viewport_pos,
            } = event
            && let (Some(id), Some(feature)) =
                (self.tab.active_sketch_id, self.get_active_sketch(ctx))
        {
            if let Some(at) = Self::cursor_to_sketch(ctx, &feature.plane, *viewport_pos) {
                let args = commands::args(serde_json::json!({
                    "sketch": id.0.to_string(),
                    "x": at.x,
                    "y": at.y,
                }));
                match self.run_command("sketch.fit_edge", &args, ctx) {
                    Ok(answer) => {
                        ctx.log_info(format!(
                            "Fitted a {} to the edge: {} points, σ {:.4} mm",
                            answer["kind"].as_str().unwrap_or("curve"),
                            answer["points"],
                            answer["sigma"].as_f64().unwrap_or(0.0)
                        ));
                        ctx.record("sketch.fit_edge", args, answer);
                    }
                    Err(why) => ctx.log_warn(why.to_string()),
                }
            }
            return InputResult::consumed();
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
                if !self.tab.tool_state.is_idle() {
                    self.tab.tool_state = ToolState::Idle;
                    self.tab.dim_capture.clear_buffers();
                    ctx.log_info("Sketch: cancelled current tool operation");
                    return InputResult::consumed();
                }
                self.tab.right_press = Some(*viewport_pos);
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
            WorkbenchInputEvent::Text { text } => self.handle_text(ctx, tool, text),
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
        let mut snap = !self.tab.snap_off;
        let widgets = settings::widgets(&self.options, snap);
        let out = core_document::panel::show_filtered(
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

    fn viewport_hud(&self, ctx: &WorkbenchRuntimeContext) -> Option<ViewportHud> {
        let feature = self.frame_displayed(ctx)?;
        let proj = SketchProjector::new(ctx, feature.plane);
        let pal = ctx.sketch_palette;
        let tool = self.tab.last_tool.as_deref().unwrap_or("sketch.select");
        let calibrating = self.tab.calibrating.as_ref().map(|c| match (c.from, c.to) {
            (None, _) => "Click the first end of a length you know on the picture",
            (Some(_), None) => "Click the other end",
            _ => "Type the real length in the panel and apply it",
        });
        let (name, prompt) = match self.tab.tool_state.hint() {
            _ if calibrating.is_some() => ("Calibrate picture", calibrating.unwrap_or_default()),
            Some((name, prompt)) => (name, prompt),
            None if tool == "sketch.external" && self.tab.external_intersection => (
                "Intersection",
                "Click faces of a solid to add where they cross the sketch plane",
            ),
            None if tool == "sketch.rotate" && self.tab.tool_params.rotate_full_turn => {
                ("Rotate", "Click the centre: the copies go evenly around it")
            }
            None => idle_hint(tool),
        };
        let mut keys: Vec<(String, &'static str)> = Vec::new();
        let mut key = |key: &str, meaning: &'static str| keys.push((key.to_string(), meaning));
        // Enter locks a typed value, and otherwise finishes a chain.
        let typed = self.tab.dim_capture.has_typed_input();
        if self.tab.dim_capture.is_active() {
            key("Tab", "next field");
        }
        if typed {
            key("Enter", "lock value");
        }
        if matches!(
            self.tab.tool_state,
            ToolState::LineFrom { chain: true, .. } | ToolState::BSplineDraw { .. }
        ) {
            if !typed {
                key("Enter", "finish");
            }
        } else if let ToolState::PolylineFrom {
            segment, heading, ..
        } = self.tab.tool_state
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
            if !typed {
                key("Enter", "finish");
            }
        }
        let armed = self.armed_hint();
        if armed.is_some() {
            key("Enter", "apply");
        }
        // Selecting is what the view does at rest: it takes no card.
        let resting =
            armed.is_none() && tool == "sketch.select" && self.tab.tool_state.hint().is_none();
        if !resting {
            key("Esc", "cancel");
        }
        let verdict = self.solver_verdict(&feature.sketch);
        let zoom = 1.0 / proj.units_per_px().max(1e-6);
        let card = self.tab.cursor.and_then(|cursor| {
            // The card reads where the click lands, snapped, and stands
            // by the cursor.
            let landing = self.click_landing(ctx, &feature, cursor).0;
            let widgets = ovp::readout_fields(
                &self.tab.dim_capture,
                &self.tab.tool_state,
                self.tab.last_tool.as_deref(),
                &feature.sketch,
                landing,
                &self.tab.tool_params,
                ctx.document,
            );
            if widgets.is_empty() {
                return None;
            }
            let px = proj.to_px(cursor)?;
            // Some steps take no constraint: typed, they only place the click.
            let hint = if ovp::only_places(&self.tab.tool_state) {
                "Tab next · Enter places it"
            } else {
                "Tab next · Enter constrains · click keeps free"
            };
            Some(bench_api::FloatingCard {
                id: format!("sketch.readout:{tool}"),
                screen: Some(px),
                widgets,
                hint: Some(hint.to_string()),
                ..Default::default()
            })
        });
        Some(ViewportHud {
            tool: (!resting).then(|| match armed {
                Some(name) => ToolHint {
                    icon: "external-geometry",
                    name,
                    prompt: "Click sketch elements, or edges and vertices of the model".to_string(),
                    keys,
                },
                None => ToolHint {
                    icon: tool_icon(tool),
                    name: name.to_string(),
                    prompt: prompt.to_string(),
                    keys,
                },
            }),
            badge: Some((verdict.kind.color(&pal), format!("{} DoF", verdict.dof))),
            legend: vec![
                (pal.geometry, "Normal"),
                (pal.construction, "Construction"),
                (pal.external, "External"),
                (pal.locked, "Locked"),
                (pal.fully_constrained, "Fully constrained"),
                (pal.constraint, "Constraint"),
            ],
            footer: [
                style::placement_label(&feature),
                if self.tab.snap_off {
                    "Snap: off".to_string()
                } else {
                    "Snap: objects".to_string()
                },
                format!("Zoom {zoom:.1}×"),
            ]
            .into_iter()
            .chain(
                self.tab
                    .parked_layer
                    .then(|| "Constraints: parked layer".to_string()),
            )
            .chain(self.shown_wall_check().map(walls::WallCheck::summary))
            .collect(),
            card,
        })
    }

    /// A sketch edited in section view cuts the scene at its plane, keeping
    /// the side away from the viewer, and a hair beyond the plane so a face
    /// the sketch lies on stays.
    fn clip_plane(&self, ctx: &WorkbenchRuntimeContext) -> Option<[f32; 4]> {
        let feature = self.frame_sketch(ctx)?;
        if !feature.shows_section(self.options.section_on_open) {
            return None;
        }
        Some(section_equation(&feature.plane, widen3(ctx.camera_position)).map(narrow))
    }

    /// The grid on the edited sketch's plane while it is switched on, its
    /// lines along the sketch's axes from its origin.
    fn view_grid(&self, ctx: &WorkbenchRuntimeContext) -> Option<core_document::PlaneGrid> {
        if !self.options.grid_on {
            return None;
        }
        let plane = self.frame_sketch(ctx)?.plane;
        let look = &self.options.appearance;
        Some(core_document::PlaneGrid {
            origin: narrow3(plane.origin),
            x: narrow3(plane.x_axis),
            y: narrow3(plane.y_axis),
            base: self.grid_base() as f32,
            color: look.grid_color,
            opacity: look.grid_opacity,
            dash: look.grid_lines.dash(),
        })
    }

    fn status_items(&self, ctx: &WorkbenchRuntimeContext) -> Option<StatusItems> {
        let feature = self.frame_displayed(ctx)?;
        let pal = ctx.sketch_palette;
        let verdict = self.solver_verdict(&feature.sketch);
        let mut names: Vec<String> = feature
            .sketch
            .geometry
            .iter()
            .filter(|g| self.tab.selected.contains(&g.id()))
            .map(|g| style::element_name(&feature.sketch, g.id()))
            .collect();
        names.extend(
            feature
                .sketch
                .constraints
                .iter()
                .filter(|c| self.tab.selected_constraints.contains(&c.id))
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
                .tab
                .cursor
                .map(|c| format!("X {:.2} · Y {:.2} mm", c.x, c.y)),
            mode: Some(match self.scan_fit(&feature.sketch, ctx) {
                Some((within, all)) => format!(
                    "Sketch edit mode · scan {within} of {all} points within {} mm",
                    self.options.scan_range
                ),
                None => "Sketch edit mode".to_string(),
            }),
        })
    }

    /// The edited sketch's reference pictures, laid on its plane under
    /// its lines.
    /// Edges are being found in a reference picture.
    fn busy(&self) -> bool {
        self.tab.picture_edges.running()
    }

    fn get_screen_space_images(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceImage> {
        let Some(feature) = self.frame_displayed(ctx) else {
            return Vec::new();
        };
        let proj = SketchProjector::new(ctx, feature.plane);
        let loupe = self.loupe(ctx, &feature, &proj).map(|l| l.image);
        feature
            .sketch
            .images
            .iter()
            .filter_map(|image| {
                let decoded = self.tab.image_cache.get(image.asset, || {
                    ctx.document.asset_bytes(image.asset).map(<[u8]>::to_vec)
                })?;
                let aspect = decoded.height as f64 / decoded.width.max(1) as f64;
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
                    opacity: narrow(image.opacity),
                })
            })
            .chain(loupe)
            .collect()
    }

    /// The regions the edited sketch's curves close, shaded on its plane
    /// as the view shades the sketch out of editing
    /// (`closed_parts::Kept::passive_regions`): every region, the ones a
    /// plain profile takes stronger.
    fn get_overlay_meshes(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::OverlayMesh> {
        if !self.options.shade_regions {
            return Vec::new();
        }
        let (Some(id), Some(feature), Some(kernel)) = (
            self.tab.active_sketch_id,
            self.frame_displayed(ctx),
            ctx.kernel,
        ) else {
            return Vec::new();
        };
        let opacity = self.options.appearance.region_opacity;
        // The stored sketch is known by its data, as out of editing; an
        // open card's draft by what it holds.
        let drafting = self.tab.corner_task.is_some() || self.tab.dim_edit.is_some();
        let revision = match ctx.document.feature_values(id) {
            Some(data) if !drafting => core_document::data_revision(data),
            _ => core_document::data_revision(&feature.to_json()),
        };
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            let plane = feature.plane;
            let axes = [plane.origin, plane.normal, plane.x_axis, plane.y_axis];
            (
                revision,
                axes.map(|v| v.map(f64::to_bits)),
                opacity.to_bits(),
            )
                .hash(&mut hasher);
            hasher.finish()
        };
        let mut cached = self.tab.regions.borrow_mut();
        // A drag that moves the geometry keeps the regions shaded where it
        // started; its release has them worked out for the sketch it leaves.
        let moving = self.tab.dragging.as_ref().is_some_and(|drag| drag.moved);
        if cached.as_ref().is_none_or(|(k, _)| *k != key && !moving) {
            let meshes = self
                .tab
                .closed_parts
                .passive_regions(id, revision, &feature, REGION_LIFT_MM, opacity)
                .into_iter()
                .filter_map(|region| {
                    let mesh = kernel.profile_mesh(&region.profile).ok()?;
                    (!mesh.indices.is_empty()).then(|| (std::sync::Arc::new(mesh), region.opacity))
                })
                .collect();
            *cached = Some((key, meshes));
        }
        let Some((key, meshes)) = cached.as_ref() else {
            return Vec::new();
        };
        meshes
            .iter()
            .enumerate()
            .map(|(index, (mesh, opacity))| core_document::OverlayMesh {
                mesh: std::sync::Arc::clone(mesh),
                revision: key.wrapping_add(index as u64),
                color: ctx.sketch_palette.geometry,
                wireframe: false,
                opacity: *opacity,
                on_top: false,
            })
            .collect()
    }

    fn get_screen_space_overlays(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceOverlay> {
        let Some(feature) = self.frame_displayed(ctx) else {
            return Vec::new();
        };
        let proj = SketchProjector::new(ctx, feature.plane);
        let pal = ctx.sketch_palette;
        let mut out = Vec::new();
        // The loupe's frame, its edges and a cross on the snapped point.
        if let Some(loupe) = self.loupe(ctx, &feature, &proj) {
            let c = loupe.image.corners;
            for i in 0..4 {
                out.push(core_document::ScreenSpaceOverlay::new(
                    c[i],
                    c[(i + 1) % 4],
                    pal.preselect,
                    1.5,
                ));
            }
            for (p, q) in loupe.edges {
                out.push(core_document::ScreenSpaceOverlay::new(
                    p,
                    q,
                    pal.external,
                    1.0,
                ));
            }
            let arm = 8.0 * ctx.pixels_per_point;
            let [x, y] = loupe.middle;
            for (p, q) in [([x - arm, y], [x + arm, y]), ([x, y - arm], [x, y + arm])] {
                out.push(core_document::ScreenSpaceOverlay::new(
                    p,
                    q,
                    pal.preselect,
                    1.0,
                ));
            }
        }
        // The reference pictures' edges, as thin guides over them.
        for image in &feature.sketch.images {
            let Some(set) = self.tab.picture_edges.get(image.asset, || {
                ctx.document.asset_bytes(image.asset).map(<[u8]>::to_vec)
            }) else {
                continue;
            };
            let step = (set.width.max(set.height) / 1024).max(1);
            for chain in &set.chains {
                let points: Vec<_> = chain
                    .points
                    .iter()
                    .step_by(step)
                    .filter_map(|p| proj.to_px(picture_edges::to_sketch(image, &set, *p)))
                    .collect();
                for pair in points.windows(2) {
                    out.push(core_document::ScreenSpaceOverlay::new(
                        pair[0],
                        pair[1],
                        pal.external,
                        1.0,
                    ));
                }
            }
        }
        // The scans' section, dashed under the sketch's own lines, every
        // fifth of a millimetre at most: in the fitting colour where it
        // lies within range of the sketch's curves, the conflict colour
        // where it does not.
        let off = scan::distance_to_sketch(&feature.sketch);
        let range = self.options.scan_range;
        for chain in self.scan_guides(ctx).iter() {
            let mut kept: Vec<[f64; 2]> = Vec::new();
            for p in &chain.points {
                if kept
                    .last()
                    .is_none_or(|q| (p[0] - q[0]).hypot(p[1] - q[1]) >= 0.2)
                {
                    kept.push(*p);
                }
            }
            if chain.closed
                && let Some(first) = kept.first().copied()
            {
                kept.push(first);
            }
            for pair in kept.windows(2) {
                let at = |p: [f64; 2]| proj.to_px(Vec2D::new(p[0], p[1]));
                if let (Some(a), Some(b)) = (at(pair[0]), at(pair[1])) {
                    let color = match off(pair[0]).zip(off(pair[1])) {
                        Some((d, e)) if d.max(e) <= range => pal.fully_constrained,
                        Some(_) => pal.conflict,
                        None => pal.external,
                    };
                    out.push(
                        core_document::ScreenSpaceOverlay::new(a, b, color, 1.5).dashed(4.0, 3.0),
                    );
                }
            }
        }
        out.extend(
            self.frame_overlays(ctx, &feature, &proj, &pal)
                .lines
                .iter()
                .cloned(),
        );
        // The calibration's line, to the cursor until its second point.
        if let Some(calibrating) = &self.tab.calibrating
            && let Some(from) = calibrating.from
            && let Some(to) = calibrating.to.or(self.tab.cursor)
            && let (Some(a), Some(b)) = (proj.to_px(from), proj.to_px(to))
        {
            out.push(core_document::ScreenSpaceOverlay::new(
                a,
                b,
                pal.preview,
                2.0,
            ));
        }
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
            let glyphs = self.frame_glyphs(ctx, &feature, &proj);
            out.extend(glyphs::dimension_overlays(
                &glyphs,
                &self.options.appearance,
            ));
        }
        out
    }

    fn get_screen_space_marks(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<ScreenSpaceMark> {
        let Some(feature) = self.frame_displayed(ctx) else {
            return Vec::new();
        };
        let proj = SketchProjector::new(ctx, feature.plane);
        let pal = ctx.sketch_palette;
        let mut out = self
            .frame_overlays(ctx, &feature, &proj, &pal)
            .marks
            .clone();
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
            let size = self.options.appearance.label_px(ctx.pixels_per_point);
            out.extend(
                self.frame_glyphs(ctx, &feature, &proj)
                    .iter()
                    .filter_map(|g| g.mark(size)),
            );
        }
        out
    }

    fn get_screen_space_labels(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<ScreenSpaceLabel> {
        let Some(feature) = self.frame_displayed(ctx) else {
            return Vec::new();
        };
        let proj = SketchProjector::new(ctx, feature.plane);
        let pal = ctx.sketch_palette;
        let mut labels: Vec<ScreenSpaceLabel> = if self.options.constraints_hidden {
            Vec::new()
        } else {
            self.frame_glyphs(ctx, &feature, &proj)
                .iter()
                .filter_map(|g| g.label(&self.options.appearance, ctx.pixels_per_point))
                .collect()
        };
        // What the cursor would snap to, by name.
        labels.extend(
            self.frame_overlays(ctx, &feature, &proj, &pal)
                .labels
                .iter()
                .cloned(),
        );
        labels.extend(overlay::spline_labels(
            &proj,
            &pal,
            &feature.sketch,
            overlay::SplineLabels {
                degree: self.options.spline_degree,
                multiplicity: self.options.spline_multiplicity,
                weights: self.options.spline_weights,
            },
            SPLINE_LABEL_PT * ctx.pixels_per_point,
            ctx.pixels_per_point,
        ));
        if let Some(check) = self.shown_wall_check() {
            labels.extend(walls::labels(check, &proj, &pal));
        }
        // A picture not measured against something of known size says so.
        for image in &feature.sketch.images {
            let text = match &image.scale {
                sketch::ImageScale::Calibrated { .. } => continue,
                sketch::ImageScale::Nominal { dpi } => {
                    format!("UNCALIBRATED · {dpi:.0} dpi as the file says")
                }
                sketch::ImageScale::Given => "UNCALIBRATED · width as given".to_string(),
            };
            let aspect = self
                .tab
                .image_cache
                .get(image.asset, || {
                    ctx.document.asset_bytes(image.asset).map(<[u8]>::to_vec)
                })
                .map_or(1.0, |d| d.height as f64 / d.width.max(1) as f64);
            let [top_left, ..] = images::corners(image, aspect);
            if let Some([x, y]) = proj.to_px(top_left) {
                // Labels stand centred on their point: inside the corner.
                let half = text.chars().count() as f32 * 3.4;
                labels.push(
                    ScreenSpaceLabel::new([x + half + 8.0, y + 14.0], text, pal.conflict, 11.0)
                        .pill()
                        .mono(),
                );
            }
        }
        // The probable design value of each selected unsized curve, at its
        // middle.
        for size in candidates::sizes(&feature.sketch) {
            if !self.tab.selected.in_order().contains(&size.element) {
                continue;
            }
            let middle = feature
                .sketch
                .geometry
                .iter()
                .find(|g| g.id() == size.element)
                .and_then(|g| match g {
                    GeometryElement::Line(l) => {
                        let (a, b) = (
                            feature.sketch.point_position(l.start)?,
                            feature.sketch.point_position(l.end)?,
                        );
                        Some(Vec2D::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0))
                    }
                    GeometryElement::Arc(a) => feature.sketch.point_position(a.start),
                    GeometryElement::Circle(c) => {
                        let o = feature.sketch.point_position(c.center)?;
                        Some(Vec2D::new(o.x + c.radius, o.y))
                    }
                    _ => None,
                });
            if let Some([x, y]) = middle.and_then(|m| proj.to_px(m)) {
                let word = if size.kind == "radius" { "R " } else { "" };
                labels.push(
                    ScreenSpaceLabel::new(
                        [x, y - 16.0],
                        format!("{:.2} ≈ {word}{}", size.measured, size.design),
                        pal.formula,
                        12.0,
                    )
                    .pill()
                    .mono(),
                );
            }
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
        let mut lit = self.tab.selected_constraints.clone();
        lit.extend(self.tab.hovered_constraint);
        // An element in hand in the panel lights its constraints, as its
        // row's badges and the constraint rows do.
        if self.tab.hover_from_panel
            && let Some(id) = self.tab.hovered
        {
            lit.extend(relations::Relations::of(sketch).constraints_on(id));
        }
        glyphs::build(
            sketch,
            proj,
            &lit,
            &self.bound_dimensions(ctx),
            &ctx.sketch_palette,
            self.tab.parked_layer,
            self.options.dimension_labels,
            self.options.appearance.label_px(ctx.pixels_per_point),
        )
    }

    /// What the cursor would snap to in select mode, shown without moving
    /// anything: the object snap a drag or a drawing tool would take there
    /// (a point, a centre, the origin, a crossing, a line's middle, a curve
    /// or an axis). None while a gesture is held, a box is drawn, a
    /// dimension's label is under the cursor or a constraint tool is armed.
    fn hover_snap(
        &self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
    ) -> Option<snap::Snap> {
        if !matches!(self.tab.last_tool.as_deref(), None | Some("sketch.select"))
            || self.tab.snap_off
            || self.tab.box_select.is_some()
            || self.tab.dragging.is_some()
            || self.tab.label_drag.is_some()
            || self.tab.hovered_constraint.is_some()
            || self.tab.armed.is_some()
        {
            return None;
        }
        let tol = Self::snap_tolerance(ctx, &feature.plane);
        let snap = snap::resolve(&feature.sketch, self.tab.cursor?, tol, &Default::default());
        snap.kind.is_some().then_some(snap)
    }

    /// The edited sketch as [`Self::get_active_sketch`] reads it, read once
    /// for every hook of a frame.
    fn frame_sketch(&self, ctx: &WorkbenchRuntimeContext) -> Option<std::sync::Arc<SketchFeature>> {
        let key = frame_memo::SketchKey::of(self.tab.stamp(), ctx);
        self.frame.active(key, || self.get_active_sketch(ctx))
    }

    /// The sketch the view shows: an open card's draft
    /// ([`Self::drafted_sketch`]), else the stored sketch. Worked out once
    /// for every hook of a frame; writes nothing.
    fn frame_displayed(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> Option<std::sync::Arc<SketchFeature>> {
        let key = frame_memo::SketchKey::of(self.tab.stamp(), ctx);
        self.frame
            .displayed(key, || match self.drafted_sketch(ctx) {
                Some(draft) => Some(std::sync::Arc::new(draft)),
                None => self.frame_sketch(ctx),
            })
    }

    fn draw_key(&self, ctx: &WorkbenchRuntimeContext) -> frame_memo::DrawKey {
        let sketch = frame_memo::SketchKey::of(self.tab.stamp(), ctx);
        frame_memo::DrawKey::of(sketch, self.options, ctx)
    }

    /// [`Self::build_overlays`] of the frame's displayed sketch, built once
    /// for its lines, marks and labels.
    fn frame_overlays(
        &self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        proj: &SketchProjector,
        pal: &SketchPalette,
    ) -> std::sync::Arc<overlay::Overlays> {
        self.frame.overlays(self.draw_key(ctx), || {
            self.build_overlays(ctx, feature, proj, pal)
        })
    }

    /// [`Self::glyphs`] of the frame's displayed sketch, built once for its
    /// lines, marks and labels.
    fn frame_glyphs(
        &self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        proj: &SketchProjector,
    ) -> std::sync::Arc<Vec<glyphs::Glyph>> {
        self.frame.glyphs(self.draw_key(ctx), || {
            self.glyphs(ctx, &feature.sketch, proj)
        })
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
        let landed = self.tab.cursor.map(|c| self.click_landing(ctx, feature, c));
        let snap = landed
            .and_then(|(_, snap)| snap)
            .or_else(|| self.hover_snap(ctx, feature))
            .or_else(|| self.armed_snap(ctx, feature));
        // Selected constraints show the geometry they hold, in their own
        // colour: it is not selected, so tools do not act on it.
        let selected = self.tab.selected.clone();
        let relations = relations::Relations::of(&feature.sketch);
        let mut referenced: HashSet<Uuid> = HashSet::new();
        for c in &feature.sketch.constraints {
            if self.tab.selected_constraints.contains(&c.id)
                || self.tab.hovered_constraint == Some(c.id)
            {
                referenced.extend(relations.held_by(c.id));
            }
        }
        // Typed values place the preview where they will place the click.
        let typed = self.tab.dim_capture.typed(ctx.document);
        let preview_cursor = landed.map(|(cursor, _)| {
            if typed.is_empty() {
                cursor
            } else {
                let cursor =
                    self.tab
                        .dim_capture
                        .steer(&self.tab.tool_state, &feature.sketch, cursor);
                ovp::override_cursor(&self.tab.tool_state, &feature.sketch, cursor, &typed)
            }
        });
        let params = ovp::typed_params(&self.tab.tool_params, &self.tab.tool_state, &typed);
        let locked = self.locked_elements(ctx, &feature.sketch);
        overlay::build_overlays(
            proj,
            pal,
            &feature.sketch,
            &selected,
            &referenced,
            &locked,
            self.tab.hovered,
            &self.tab.tool_state,
            preview_cursor,
            &ToolParams {
                // The copy tool makes one copy at least.
                copies: self.effective_copies(&params),
                ..params
            },
            self.tab.box_select.as_ref().map(|b| (b.anchor, b.current)),
            self.tab.last_tool.as_deref(),
            snap_tol,
            snap,
            self.options.construction_on_top,
            &self.options.appearance,
            self.options.hint_lines,
            self.options.arc_helpers,
            self.options.spline_polygon,
            self.tool_proposal(ctx, feature).as_ref(),
        )
    }

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

    /// The open sketch's elements nothing left free can move, worked out
    /// once per document edit; none while the sketch is fully constrained,
    /// when everything draws in that colour instead.
    pub(crate) fn locked_elements(
        &self,
        ctx: &WorkbenchRuntimeContext,
        sketch: &Sketch,
    ) -> std::sync::Arc<HashSet<Uuid>> {
        let Some(id) = self.tab.active_sketch_id else {
            return Default::default();
        };
        if sketch.is_fully_constrained {
            return Default::default();
        }
        let key = (id, ctx.document.mutation_seq());
        if let Some((at, set)) = self.tab.locked.borrow().as_ref()
            && *at == key
        {
            return set.clone();
        }
        let set = std::sync::Arc::new(solver::locked_elements(sketch));
        *self.tab.locked.borrow_mut() = Some((key, set.clone()));
        set
    }

    /// The solver's verdict from the cached diagnosis, or a cheap estimate
    /// of the freedom left when none is cached yet.
    pub(crate) fn solver_verdict(&self, sketch: &Sketch) -> SolverVerdict {
        let not_converged = matches!(self.tab.last_solve, Some(SolveOutcome::NotConverged { .. }));
        let (dof, analyzed, conflicting, redundant) = match &self.tab.last_diagnosis {
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
        } else if self.tab.unsolved_edits {
            // Before any verdict on freedom or conflicts: those describe
            // the solved sketch, and the geometry shown is not it.
            verdict(
                SolverKind::Unsolved,
                "Not solved",
                "Automatic update is off: the geometry waits for Solve now.".into(),
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

    /// The solver's verdict as the panel's header shows it: a few words
    /// and a tone, the reason on hover.
    fn task_status(&self, sketch: &Sketch) -> core_document::TaskStatus {
        use core_document::StatusTone;
        let verdict = self.solver_verdict(sketch);
        let (text, tone) = match verdict.kind {
            SolverKind::Fully => ("Fully constrained".to_string(), StatusTone::Good),
            SolverKind::Under => (
                match verdict.dof {
                    1 => "1 degree of freedom left".to_string(),
                    n => format!("{n} degrees of freedom left"),
                },
                StatusTone::Warning,
            ),
            SolverKind::Conflicting => (verdict.title.clone(), StatusTone::Bad),
            SolverKind::Redundant | SolverKind::Unsolved => {
                (verdict.title.clone(), StatusTone::Warning)
            }
            _ => (verdict.title.clone(), StatusTone::Info),
        };
        core_document::TaskStatus {
            text,
            detail: verdict.body,
            tone,
        }
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
    /// is added, dimensional ones at their measured value. A selection the
    /// tool does not take yet, but could with more picks, arms the tool
    /// (`model_picks`).
    fn apply_constraint_tool(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: &str,
        which: &str,
    ) -> InputResult {
        self.tab.armed = None;
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let shape = constrain::SelectionShape::picked_in(
            &feature.sketch,
            &self.tab.selected,
            self.tab.selected.in_order(),
        );
        // "dimension" is whichever dimensional tool the selection takes.
        let resolved = if which == "dimension" {
            constrain::dimension_in(&shape, &feature.sketch)
        } else {
            Some(which)
        };
        let mut items: Vec<Uuid> = self.tab.selected.iter().copied().collect();
        items.sort();
        let own_ends = constrain::joins_own_ends(&feature.sketch, &shape.points_picked());
        let done = resolved.is_some_and(|resolved| {
            self.constrain_items(ctx, feature, resolved, &items, Vec::new())
        });
        if !done && which == "coincident" && own_ends {
            ctx.log_warn(
                "The two ends of one line or arc cannot meet: draw a circle for a closed round",
            );
        } else if !done && !self.arm_constraint(ctx, tool, which, &shape) {
            ctx.log_warn(if which == "dimension" {
                "Select a line, circles, or two items to dimension".to_string()
            } else {
                format!("The {which} constraint does not fit the current selection")
            });
        }
        self.tab.selection_shape = if done { Default::default() } else { shape };
        if done {
            self.continue_constraint(ctx, tool, which);
        }
        InputResult::consumed()
    }

    /// Add the constraints the tool `which` makes of `items` to `feature`'s
    /// sketch and store it, recording `first` (the calls that made what the
    /// items name) and then the constraint. Whether the items fit.
    pub(crate) fn constrain_items(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        mut feature: SketchFeature,
        which: &str,
        items: &[Uuid],
        first: Vec<(&'static str, core_document::CommandArgs, serde_json::Value)>,
    ) -> bool {
        // A dimension the sketch holds already is not made twice: the one
        // it holds is selected instead, its value there to change.
        if let Some(held) = commands::held_dimension(&feature.sketch, which, items) {
            let label = feature
                .sketch
                .constraints
                .iter()
                .find(|c| c.id == held)
                .map(|c| sketch::constraint_label(&c.kind))
                .unwrap_or_default();
            ctx.log_warn(format!(
                "The sketch already holds this dimension ({label}); change its value instead"
            ));
            self.tab.selected.clear();
            self.tab.selected_constraints = HashSet::from([held]);
            return true;
        }
        let Ok(made) = commands::constrain(&mut feature.sketch, which, items, None) else {
            return false;
        };
        let remove_redundant = self.options.auto_remove_redundant;
        let removed = if remove_redundant {
            commands::remove_superseded(&mut feature.sketch, &made)
        } else {
            0
        };
        let mut calls = Vec::new();
        if let Some(sketch_id) = self.tab.active_sketch_id {
            calls = first;
            let mut args = serde_json::json!({
                "sketch": sketch_id.0.to_string(),
                "kind": which,
                "items": ids_json(items),
            });
            if remove_redundant {
                args["remove_redundant"] = serde_json::json!(true);
            }
            calls.push((
                "sketch.constrain",
                commands::args(args),
                serde_json::json!(made),
            ));
        }
        if removed > 0 {
            ctx.log_info(format!(
                "Removed {removed} constraint(s) the new one made redundant"
            ));
            self.tab
                .selected_constraints
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
        // What the constraint took is done with: the next picks start afresh.
        let picked = self.tab.selected.in_order().to_vec();
        self.tab.selected.clear();
        // A new dimension asks its value in a card, and waits there with
        // the sketch and the calls that make it until OK.
        if let Some(id) = dimension.filter(|_| self.options.ask_dimension_value) {
            self.open_new_dimension(ctx, &feature, id);
            if let Some(edit) = self.tab.dim_edit.as_mut() {
                edit.restore = picked;
                edit.made = Some(dim_edit::MadeDimension { feature, calls });
            }
            return true;
        }
        self.tab.pending_focus = dimension;
        for (id, args, result) in calls {
            ctx.record(id, args, result);
        }
        self.store_sketch(ctx, feature);
        true
    }

    /// Flip one flag of the selected constraints, each for itself, through
    /// `sketch.set_constraint`'s code.
    fn toggle_constraint_flag(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        which: ConstraintFlag,
    ) -> InputResult {
        let (Some(mut feature), Some(id)) =
            (self.get_active_sketch(ctx), self.tab.active_sketch_id)
        else {
            return InputResult::ignored();
        };
        // The selected constraints, split by the flag each ends with.
        let mut to: [Vec<Uuid>; 2] = [Vec::new(), Vec::new()];
        for c in &feature.sketch.constraints {
            if self.tab.selected_constraints.contains(&c.id) {
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

    /// Repair the sketch being edited, ends within the snap reach joined,
    /// as `sketch.repair` does.
    /// Where the visible scans cross the open sketch's plane, cut again
    /// when the plane or a scan moves.
    fn scan_guides(&self, ctx: &WorkbenchRuntimeContext) -> std::sync::Arc<Vec<scan::Chain2>> {
        let (Some(feature), Some(kernel)) = (self.frame_sketch(ctx), ctx.kernel) else {
            return Default::default();
        };
        let Some(key) = scan::key(ctx.document, &feature.plane) else {
            return Default::default();
        };
        let mut cached = self.tab.scan_guides.borrow_mut();
        if let Some((k, chains)) = cached.as_ref()
            && *k == key
        {
            return chains.clone();
        }
        let chains = std::sync::Arc::new(scan::chains(ctx.document, kernel, &feature.plane));
        *cached = Some((key, chains.clone()));
        chains
    }

    /// The selected lines and rounds (every unsized one when none is)
    /// held at their probable design values.
    fn design_values(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(id) = self.tab.active_sketch_id else {
            return InputResult::ignored();
        };
        let picked: Vec<String> = self
            .tab
            .selected
            .in_order()
            .iter()
            .map(|u| u.to_string())
            .collect();
        let mut args = serde_json::json!({"sketch": id.0.to_string()});
        if !picked.is_empty() {
            args["elements"] = serde_json::json!(picked);
        }
        let args = commands::args(args);
        match self.run_command("sketch.design_values", &args, ctx) {
            Ok(answer) => ctx.record("sketch.design_values", args, answer),
            Err(why) => ctx.log_warn(why.to_string()),
        }
        InputResult::consumed()
    }

    /// How many of the scan slice's points lie within range of the
    /// sketch's curves, and how many there are; `None` without a slice or
    /// curves.
    fn scan_fit(&self, sketch: &Sketch, ctx: &WorkbenchRuntimeContext) -> Option<(usize, usize)> {
        let guides = self.scan_guides(ctx);
        let off = scan::distance_to_sketch(sketch);
        let mut counts = (0, 0);
        for p in guides.iter().flat_map(|c| &c.points) {
            counts.0 += usize::from(off(*p)? <= self.options.scan_range);
            counts.1 += 1;
        }
        (counts.1 > 0).then_some(counts)
    }

    /// The open sketch at true scale as `kind` (DXF, SVG or CSV), to a
    /// file the user picks.
    fn save_drawing(&mut self, ctx: &mut WorkbenchRuntimeContext, kind: &str) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let sketch = &feature.sketch;
        let pictures = commands::pictures(sketch, ctx, &self.tab.picture_edges);
        let trace = drawing::traceability(sketch, &feature.plane, &pictures, "mm");
        let contents = match kind {
            "SVG" => {
                let (drawn, guides) = drawing::polylines(sketch);
                Ok(drawing::svg(&drawn, &guides, &trace))
            }
            "CSV" => Ok(drawing::csv(sketch, &trace).0),
            _ => ctx
                .kernel
                .ok_or_else(|| "there is no kernel to write with".to_string())
                .and_then(|k| {
                    k.write_dxf_drawing(&drawing::sheet(sketch, &pictures, &trace, false))
                        .map_err(|e| e.to_string())
                }),
        };
        match contents {
            Ok(text) => ctx.request(core_document::HostRequest::SaveFile {
                name: feature.sketch.name.clone(),
                kind: kind.to_string(),
                extension: kind.to_ascii_lowercase(),
                contents: text.into_bytes(),
            }),
            Err(why) => ctx.log_warn(format!("Save as {kind}: {why}")),
        }
        InputResult::consumed()
    }

    fn fit_scan(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(id) = self.tab.active_sketch_id else {
            return InputResult::ignored();
        };
        let args = commands::args(serde_json::json!({
            "sketch": id.0.to_string(),
            "construction": self.construction_mode(),
        }));
        match self.run_command("sketch.fit_scan", &args, ctx) {
            Ok(answer) => {
                ctx.log_info(format!(
                    "Fitted the scan's section: {} lines, {} arcs, {} circles",
                    answer["lines"], answer["arcs"], answer["circles"]
                ));
                ctx.record("sketch.fit_scan", args, answer);
            }
            Err(why) => ctx.log_warn(why.to_string()),
        }
        InputResult::consumed()
    }

    fn repair(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let Some(id) = self.tab.active_sketch_id else {
            return InputResult::ignored();
        };
        let tolerance = Self::snap_tolerance(ctx, &feature.plane) * 0.5;
        let done = repair::repair(&mut feature.sketch, tolerance);
        if !done.is_empty() {
            self.tab.selected.clear();
            self.tab.selected_constraints.clear();
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

    /// Select the constraints on the selected elements (a curve's own
    /// points count as the curve), leaving the elements.
    fn select_constraints_of(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        self.tab.selected_constraints = constraints_on(&feature.sketch, &self.tab.selected);
        self.tab.selected.clear();
        ctx.log_info(format!(
            "{} constraint(s) on the selection",
            self.tab.selected_constraints.len()
        ));
        InputResult::consumed()
    }

    /// Select the elements the selected constraints hold, leaving the
    /// constraints.
    fn select_elements_of(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let elements = elements_of(&feature.sketch, &self.tab.selected_constraints);
        self.tab.selected_constraints.clear();
        self.tab.selected.clear();
        self.tab.selected.extend(elements);
        ctx.log_info(format!("{} element(s) selected", self.tab.selected.len()));
        InputResult::consumed()
    }

    /// Add the sketch's reference `id` (its origin or an axis) to the
    /// selection, or, while a constraint tool waits for its picks, hand it
    /// to the tool as the next one.
    fn select_reference(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        id: Uuid,
        name: &str,
    ) -> InputResult {
        if self.get_active_sketch(ctx).is_none() {
            return InputResult::ignored();
        }
        if self.tab.armed.is_some() {
            self.armed_take(ctx, id);
        } else {
            self.tab.selected.insert(id);
            ctx.log_info(format!("Selected the {name}"));
        }
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
        self.tab.selected.clear();
        self.tab.selected_constraints.clear();
        for c in &feature.sketch.constraints {
            if sketch::constraint_refs(&c.kind)
                .iter()
                .any(|r| !ids.contains(r))
            {
                self.tab.selected_constraints.insert(c.id);
            }
        }
        ctx.log_info(format!(
            "{} malformed constraint(s)",
            self.tab.selected_constraints.len()
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
        self.tab.selected.clear();
        self.tab.selected_constraints.clear();
        self.tab.selected.extend(free.iter().copied());
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
                    self.tab.selected.insert(g.id());
                }
            }
        }
        ctx.log_info(format!(
            "{} element(s) with a degree of freedom",
            self.tab.selected.len()
        ));
        InputResult::consumed()
    }

    /// The selection repeated in a grid, as the panel's array settings say.
    fn array_selection(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let p = self.tab.tool_params;
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
            &self.tab.selected,
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
            if let Some(id) = self.tab.active_sketch_id {
                let mut items: Vec<Uuid> = self.tab.selected.iter().copied().collect();
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
        let effect = tools::join(
            &mut feature.sketch,
            &self.tab.selected,
            tools::JOIN_TOLERANCE,
        );
        if !effect.changed {
            if let Some(why) = effect.log {
                ctx.log_warn(why);
            }
            return InputResult::consumed();
        }
        if let Some(log) = effect.log {
            ctx.log_info(log);
        }
        if let Some(id) = self.tab.active_sketch_id {
            let mut items: Vec<Uuid> = self.tab.selected.iter().copied().collect();
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
        self.tab.selected.retain(|id| alive.contains(id));
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
        let mut items: Vec<Uuid> = self.tab.selected.iter().copied().collect();
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
        if let Some(id) = self.tab.active_sketch_id {
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
        self.tab.selected.retain(|id| alive.contains(id));
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
        InputResult::consumed()
    }

    /// Copy the selection to the clipboard; cut removes it too.
    fn clipboard_copy(&mut self, ctx: &mut WorkbenchRuntimeContext, cut: bool) -> bool {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return false;
        };
        if self.tab.selected.is_empty() {
            ctx.log_warn("Nothing selected to copy");
            return true;
        }
        let mut clip = Sketch::new("clipboard");
        let count = tools::copy_from(
            &feature.sketch,
            &mut clip,
            &self.tab.selected,
            &tools::Similarity::translation(glam::DVec2::ZERO),
        );
        self.tab.clipboard = Some(clip);
        if cut {
            let mut ids: Vec<Uuid> = self.tab.selected.iter().copied().collect();
            ids.sort();
            commands::delete_items(&mut feature.sketch, &ids);
            if let Some(id) = self.tab.active_sketch_id {
                ctx.record(
                    "sketch.delete",
                    commands::args(serde_json::json!({
                        "sketch": id.0.to_string(),
                        "items": ids_json(&ids),
                    })),
                    serde_json::Value::Null,
                );
            }
            self.tab.selected.clear();
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
        let Some(clip) = self.tab.clipboard.clone() else {
            ctx.log_warn("The clipboard is empty");
            return true;
        };
        let delta = match self.tab.cursor {
            // The middle of what was copied lands under the cursor.
            Some(cursor) => {
                let (lo, hi) = clip.geometry.iter().fold(
                    (glam::DVec2::splat(f64::MAX), glam::DVec2::splat(f64::MIN)),
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
                    glam::DVec2::ZERO
                };
                cursor.to_glam() - anchor
            }
            None => glam::DVec2::new(5.0, 5.0),
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
        if let Some(id) = self.tab.active_sketch_id {
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
        self.tab.selected = feature
            .sketch
            .geometry
            .iter()
            .map(GeometryElement::id)
            .filter(|id| !before.contains(id) && !feature.sketch.derived.contains(id))
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
        let diag = match &self.tab.last_diagnosis {
            Some(d) => d.clone(),
            None => {
                let d = solver::diagnose(&feature.sketch);
                self.tab.last_diagnosis = Some(d.clone());
                d
            }
        };
        let offenders = if conflicting {
            &diag.conflicting
        } else {
            &diag.redundant
        };
        self.tab.selected.clear();
        self.tab.selected_constraints.clear();
        for c in &feature.sketch.constraints {
            if offenders.contains(&c.id) {
                self.tab.selected_constraints.insert(c.id);
                self.tab.selected.extend(sketch::constraint_refs(&c.kind));
            }
        }
        InputResult::consumed()
    }

    /// Remove every element (and the constraints on them), or every
    /// constraint, then re-solve.
    fn delete_all(&mut self, ctx: &mut WorkbenchRuntimeContext, geometry: bool) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
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
        self.delete_items(ctx, ids)
    }

    /// Delete `ids` (elements or constraints) from the edited sketch, as
    /// `sketch.delete` does, and record it.
    fn delete_items(&mut self, ctx: &mut WorkbenchRuntimeContext, ids: Vec<Uuid>) -> InputResult {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let removed = commands::delete_items(&mut feature.sketch, &ids);
        ctx.log_info(format!("Deleted {removed} item(s)"));
        if let (Some(id), false) = (self.tab.active_sketch_id, ids.is_empty()) {
            ctx.record(
                "sketch.delete",
                commands::args(serde_json::json!({
                    "sketch": id.0.to_string(),
                    "items": ids_json(&ids),
                })),
                serde_json::Value::Null,
            );
        }
        self.tab.selected.clear();
        self.tab.selected_constraints.clear();
        self.tab.hovered = None;
        self.tab.tool_state = ToolState::Idle;
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
        let args = commands::args(serde_json::json!({"sketch": sketch.0.to_string()}));
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
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
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
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
