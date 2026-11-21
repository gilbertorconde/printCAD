//! The sketcher's task panel, declared as widgets (`task_widgets`) and
//! drawn by `bench_panel` as a package's is: the plane picker
//! of a sketch about to be made, then the solver's problems, the sketch
//! pickers, tool settings, the selection's array, spline and text, the
//! reference pictures, one filter over the element and constraint lists,
//! and the placement, solver and display folds. What the user does comes
//! back as `PanelEvent`s, handled in `task_event` through the sketcher's
//! own commands and code paths.

use bench_api::panel;
use bench_api::{
    Badge, Bind, ButtonStyle, Card, Check, Dim, FieldGrid, FilterField, GapSize, Item, ItemList,
    NoteKind, PanelAction, PanelEvent, PanelSection, Quantity, Select, StatusLine, Stepper, Tag,
    Tone, Widget,
};
use egui::Color32;
use uuid::Uuid;

use core_document::{FeatureId, TaskOutcome, TaskRequest, WorkbenchRuntimeContext};
use ui_kit::tokens::*;

use crate::boundary::widen3;
use crate::relations::{Related, Relations};
use crate::sketch::{self, Constraint, Sketch, SketchPlane};
use crate::style::{constraint_icon, element_kind, element_row_icon};
use crate::{ConstraintFilter, ElementFilter, SketchWorkbench, overlay};

mod builders;
mod events;
mod lists;
mod sections;
mod text;

use builders::{
    action, check, choose, corner_check, count, degrees, grid, integer, length, mm, mm_quantity,
    offset, section, stepper, take_items,
};
pub(crate) use lists::RowText;
use lists::{ListFilter, dimension_field};
use sections::{TOLERANCES, calibrated};
use text::{text_button, text_choice, text_fields, text_number, text_string, text_toggle};

/// The panel's id among the app's declared panels.
const PANEL: &str = "sketch_task";

/// Where a sketch about to be made can go: its plane, and the datum or
/// face it stays on.
type Choice = (
    SketchPlane,
    Option<crate::feature::DatumSupport>,
    Option<crate::feature::FaceSupport>,
);

/// What the panel declared last, for reading its events back: the rows
/// of its lists and the choices behind its buttons, in their places.
#[derive(Default)]
pub(crate) struct PanelState {
    elements: Vec<ElementRow>,
    constraints: Vec<ConstraintRow>,
    /// The sketches (or datums) the picker lists.
    picked_from: Vec<FeatureId>,
    /// The knots the spline fold lists, and how many times each stands.
    knots: Vec<(f64, usize)>,
    /// The plane picker's datum planes and modes.
    datum_planes: Vec<Choice>,
    modes: Vec<Option<core_document::DatumAttachment>>,
    /// What the pointer is on in each list: the row, its tag, its badge.
    pointed: [Pointed; 2],
    /// What the element and constraint lists were last declared from, and
    /// their widgets while the panel is not drawing them: declared again
    /// only when the key changes. Their rows stay in `elements` and
    /// `constraints` meanwhile.
    lists: Option<(ListKey, Option<Vec<Widget>>)>,
    /// What each element row of the lists' widgets was declared from, in
    /// their order (none for the reference rows): a row whose key holds
    /// is taken over when the lists are declared again.
    element_keys: Vec<Option<ElementRowKey>>,
    /// What every key of `element_keys` was declared under: the palette
    /// and whether the sketch was fully constrained.
    element_style: Option<(core_document::SketchPalette, bool)>,
    /// Where the lists stand among the widgets of the panel being drawn.
    list_range: std::ops::Range<usize>,
    /// The font choice of the text tool and of the selected text, while
    /// it names no font yet.
    font_modes: [Option<usize>; 2],
}

type Pointed = (Option<usize>, Option<usize>, Option<usize>);

/// A selected spline with its degree, and a selected control point with
/// the spline it weighs on.
type SelectedSpline = (Option<(Uuid, u32)>, Option<(Uuid, Uuid)>);

/// What the element and constraint lists are declared from beyond the
/// sketch's shape: the document (its formulas, the edited sketch), the
/// projected elements lost, the palette, the selection and what is in
/// hand, the filters, the diagnosis, and the row being renamed or about
/// to take the keys.
#[derive(Clone, PartialEq)]
struct ListKey {
    sketch: crate::frame_memo::ActiveKey,
    lost: Vec<Uuid>,
    palette: core_document::SketchPalette,
    selected: std::collections::HashSet<Uuid>,
    selected_constraints: std::collections::HashSet<Uuid>,
    hovered: Option<Uuid>,
    hover_from_panel: bool,
    hovered_constraint: Option<Uuid>,
    element_filter: ElementFilter,
    constraint_filter: String,
    constraint_kind_filter: crate::ConstraintFilter,
    diagnosis: Option<crate::solver::Diagnosis>,
    pending_focus: Option<Uuid>,
    renaming: Option<Uuid>,
}

/// An element's row: the element, and the constraint behind each badge.
struct ElementRow {
    id: Uuid,
    badges: Vec<Uuid>,
}

/// Everything an element's row is declared from beyond what every row
/// shares (`PanelState::element_style`): the element, a line's ends (its
/// length), its number among its kind, how it is drawn (projected,
/// itself or as a projected curve's point, and whether as defining,
/// construction or locked) and lit (selected, related, hovered), whether
/// it is lost, and each constraint badge's kind, name, light and tint.
#[derive(PartialEq)]
struct ElementRowKey {
    geom: sketch::GeometryElement,
    ends: Option<[sketch::Vec2D; 2]>,
    ordinal: Option<usize>,
    style: (bool, Option<bool>, bool, bool),
    lit: (bool, bool, bool, bool),
    badges: Vec<BadgeKey>,
}

/// The element rows last declared with their keys, in their order, each
/// taken once: the next in order is looked at first, as rows mostly
/// stand where they stood, and the rest by element.
struct KeptRows {
    keys: Vec<Option<ElementRowKey>>,
    items: Vec<Item>,
    next: usize,
    index: Option<std::collections::HashMap<Uuid, usize>>,
}

impl KeptRows {
    fn take(&mut self, id: Uuid) -> Option<(ElementRowKey, Item)> {
        let at = match self.keys.get(self.next) {
            Some(Some(key)) if key.geom.id() == id => self.next,
            _ => {
                let keys = &self.keys;
                let index = self.index.get_or_insert_with(|| {
                    (keys.iter().enumerate())
                        .filter_map(|(i, key)| key.as_ref().map(|key| (key.geom.id(), i)))
                        .collect()
                });
                *index.get(&id)?
            }
        };
        self.next = at + 1;
        let item = self.items.get_mut(at)?;
        let key = self.keys[at].take()?;
        Some((key, std::mem::take(item)))
    }
}

#[derive(PartialEq)]
struct BadgeKey {
    kind: sketch::ConstraintKind,
    name: Option<String>,
    lit: bool,
    tint: (Tone, Option<[u8; 3]>),
}

/// A constraint's row: the constraint, and the element behind each tag.
struct ConstraintRow {
    id: Uuid,
    tags: Vec<Uuid>,
}

/// What the lists light this frame, from what last frame left in hand.
struct InHand {
    relations: std::sync::Arc<Relations>,
    /// The rows tied to the hovered item and the selection.
    related: Related,
    /// The element hovered, in the panel or the view.
    element: Option<Uuid>,
    /// The constraint hovered in the panel.
    constraint: Option<Uuid>,
}

impl SketchWorkbench {
    /// The panel's Solve now: the solver runs whatever the auto-update
    /// switch says.
    fn solve_from_panel(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if let Some(mut feature) = self.get_active_sketch(ctx) {
            let dof = self.solve_now(ctx, &mut feature);
            self.store_solved(ctx, feature, dof);
        }
    }

    /// Make sure a diagnosis is cached, then read the verdict.
    fn diagnosed_verdict(
        &mut self,
        feature: &std::sync::Arc<crate::SketchFeature>,
    ) -> crate::verdict::SolverVerdict {
        if self.tab.last_diagnosis.is_none() {
            self.tab.last_diagnosis = Some(self.diagnose(feature));
        }
        self.solver_verdict(feature)
    }

    /// The task panel body. Returns how the task ended, if it did.
    pub(crate) fn draw_task_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        self.sync_active_sketch_from_ctx(ctx);

        if self.tab.pending_creation.is_some() && request.cancel {
            self.tab.pending_creation = None;
            return TaskOutcome::Cancelled;
        }
        if self.tab.pending_creation.is_none() {
            let Some(feature) = self.get_active_sketch(ctx) else {
                return TaskOutcome::Open;
            };
            // The header's Cancel puts the sketch back; Escape never ends
            // the session: the sketcher takes it for the gesture, the tool
            // or the selection.
            let cancel = request.cancel;
            if cancel
                && let (Some(id), Some(start)) =
                    (self.tab.active_sketch_id, self.tab.session_start.clone())
                && ctx.document.get_feature_data(id) != Some(&start)
            {
                // Every edit so far recorded first, then the sketch put back.
                self.flush_draw_record(ctx);
                let args = crate::commands::object(serde_json::json!({
                    "sketch": id.0.to_string(),
                    "data": start,
                }));
                if crate::commands::run("sketch.restore", &args, ctx).is_ok() {
                    ctx.record("sketch.restore", args, serde_json::Value::Null);
                }
                ctx.request(core_document::HostRequest::FinishEditing);
                return TaskOutcome::Accepted {
                    label: format!("Cancel editing {}", feature.sketch.name),
                };
            }
            if request.accept || cancel {
                // What a click left waiting is written as it stands.
                self.store_corner(ctx, false);
                self.discard_corner();
                ctx.request(core_document::HostRequest::FinishEditing);
                return TaskOutcome::Accepted {
                    label: format!("Edit {}", feature.sketch.name),
                };
            }
            // A generated sketch is edited by its numbers.
            if let (Some(id), true) = (self.tab.active_sketch_id, feature.generator.is_some()) {
                crate::generator::panel::show(ui, ctx, id);
                return TaskOutcome::Open;
            }
        }

        let before = (self.tab.hovered, self.tab.hovered_constraint);
        let widgets = self.task_widgets(ctx);
        let out = bench_panel::show(ui, egui::Id::new(PANEL), &widgets, Some(&*ctx.document));
        self.keep_list_widgets(widgets);
        // A formula typed into a dimension's row opens its task, as a value
        // typed there does.
        let mut set_by_formula = Vec::new();
        for set in out.formulas {
            let (Ok(constraint), Some(text)) = (Uuid::parse_str(&set.bind.key), set.formula) else {
                continue;
            };
            self.open_dimension_task(ctx, constraint, &text);
            set_by_formula.push(dimension_field(constraint));
        }
        for event in &out.events {
            if !set_by_formula.iter().any(|id| id == event.id()) {
                self.task_event(ctx, event);
            }
        }
        self.hover_from_lists();
        // The rows draw from last frame's hand: one more frame shows a
        // change.
        if (self.tab.hovered, self.tab.hovered_constraint) != before {
            ui.ctx().request_repaint();
        }
        TaskOutcome::Open
    }

    /// Hover in the lists lights the viewport; when the panel stops
    /// hovering, the viewport's own hit-test takes over again.
    fn hover_from_lists(&mut self) {
        let [(element_row, _, badge), (constraint_row, tag, _)] = self.tab.panel.pointed;
        let mut element = element_row.and_then(|i| self.tab.panel.elements.get(i).map(|r| r.id));
        let mut constraint = None;
        if let (Some(i), Some(b)) = (element_row, badge) {
            constraint = self
                .tab
                .panel
                .elements
                .get(i)
                .and_then(|r| r.badges.get(b).copied());
        }
        if let Some(i) = constraint_row {
            let row = self.tab.panel.constraints.get(i);
            constraint = row.map(|r| r.id);
            if let Some(t) = tag {
                element = row.and_then(|r| r.tags.get(t).copied());
            }
        }
        if let Some(id) = element {
            self.tab.hovered = Some(id);
            self.tab.hover_from_panel = true;
        } else if self.tab.hover_from_panel {
            self.tab.hovered = None;
            self.tab.hover_from_panel = false;
        }
        self.tab.hovered_constraint = constraint;
    }

    /// The panel as widgets: the plane picker while a sketch waits for its
    /// plane, else the open sketch's panel.
    pub(crate) fn task_widgets(&mut self, ctx: &WorkbenchRuntimeContext) -> Vec<Widget> {
        self.tab.panel.list_range = 0..0;
        if self.tab.pending_creation.is_some() {
            return self.attachment_widgets(ctx);
        }
        let Some(feature) = self.frame_sketch(ctx) else {
            return Vec::new();
        };
        let shift =
            feature
                .support
                .as_ref()
                .map_or(feature.offset, |s| crate::feature::PlaneShift {
                    offset: s.offset,
                    shift: s.shift,
                    turn: s.turn,
                });
        let on_datum = feature.support.is_some();
        let plane = feature.plane;
        let sketch = &feature.sketch;
        let mut widgets = self.problem_widgets(&feature);
        if self.tab.sketch_picker.is_some() {
            widgets.extend(self.picker_widgets(ctx, &plane));
        }
        widgets.extend(self.tool_widgets(ctx));
        widgets.extend(self.array_widgets());
        widgets.extend(self.spline_widgets(sketch));
        widgets.extend(self.text_widgets(sketch));
        widgets.extend(self.image_widgets(ctx, sketch));
        let start = widgets.len();
        widgets.extend(self.list_widgets(ctx, &feature));
        self.tab.panel.list_range = start..widgets.len();
        widgets.extend(self.placement_widgets(shift, on_datum));
        widgets.extend(self.settings_widgets(sketch));
        widgets
    }
}
