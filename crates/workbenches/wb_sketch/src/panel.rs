//! The sketcher's task panel, declared as widgets (`task_widgets`) and
//! drawn by `core_document::panel` as a package's is: the plane picker
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
use crate::solver;
use crate::style::{constraint_icon, element_kind, element_name, element_row_icon};
use crate::{ConstraintFilter, ElementFilter, SketchWorkbench, overlay};

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
    /// The font choice of the text tool and of the selected text, while
    /// it names no font yet.
    font_modes: [Option<usize>; 2],
}

type Pointed = (Option<usize>, Option<usize>, Option<usize>);

/// A selected spline with its degree, and a selected control point with
/// the spline it weighs on.
type SelectedSpline = (Option<(Uuid, u32)>, Option<(Uuid, Uuid)>);

/// An element's row: the element, and the constraint behind each badge.
struct ElementRow {
    id: Uuid,
    badges: Vec<Uuid>,
}

/// A constraint's row: the constraint, and the element behind each tag.
struct ConstraintRow {
    id: Uuid,
    tags: Vec<Uuid>,
}

/// What the lists light this frame, from what last frame left in hand.
struct InHand {
    relations: Relations,
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
            self.solve_now(ctx, &mut feature);
            self.store_sketch(ctx, feature);
        }
    }

    /// Make sure a diagnosis is cached, then read the verdict.
    fn diagnosed_verdict(&mut self, sketch: &Sketch) -> crate::SolverVerdict {
        if self.last_diagnosis.is_none() {
            self.last_diagnosis = Some(solver::diagnose(sketch));
        }
        self.solver_verdict(sketch)
    }

    /// The task panel body. Returns how the task ended, if it did.
    pub(crate) fn draw_task_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        self.sync_active_sketch_from_ctx(ctx);

        if self.pending_creation.is_some() && request.cancel {
            self.pending_creation = None;
            return TaskOutcome::Cancelled;
        }
        if self.pending_creation.is_none() {
            let Some(feature) = self.get_active_sketch(ctx) else {
                return TaskOutcome::Open;
            };
            // The header's Cancel puts the sketch back; Escape never ends
            // the session: the sketcher takes it for the gesture, the tool
            // or the selection.
            let cancel = request.cancel;
            if cancel
                && let (Some(id), Some(start)) = (self.active_sketch_id, self.session_start.clone())
                && ctx.document.get_feature_data(id) != Some(&start)
            {
                // Every edit so far recorded first, then the sketch put back.
                self.flush_draw_record(ctx);
                let args = crate::commands::args(serde_json::json!({
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
                ctx.request(core_document::HostRequest::FinishEditing);
                return TaskOutcome::Accepted {
                    label: format!("Edit {}", feature.sketch.name),
                };
            }
            // A generated sketch is edited by its numbers.
            if let (Some(id), true) = (self.active_sketch_id, feature.generator.is_some()) {
                crate::generator::panel::show(ui, ctx, id);
                return TaskOutcome::Open;
            }
        }

        let before = (self.hovered, self.hovered_constraint);
        let widgets = self.task_widgets(ctx);
        let out =
            core_document::panel::show(ui, egui::Id::new(PANEL), &widgets, Some(&*ctx.document));
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
        if (self.hovered, self.hovered_constraint) != before {
            ui.ctx().request_repaint();
        }
        TaskOutcome::Open
    }

    /// Hover in the lists lights the viewport; when the panel stops
    /// hovering, the viewport's own hit-test takes over again.
    fn hover_from_lists(&mut self) {
        let [(element_row, _, badge), (constraint_row, tag, _)] = self.panel.pointed;
        let mut element = element_row.and_then(|i| self.panel.elements.get(i).map(|r| r.id));
        let mut constraint = None;
        if let (Some(i), Some(b)) = (element_row, badge) {
            constraint = self
                .panel
                .elements
                .get(i)
                .and_then(|r| r.badges.get(b).copied());
        }
        if let Some(i) = constraint_row {
            let row = self.panel.constraints.get(i);
            constraint = row.map(|r| r.id);
            if let Some(t) = tag {
                element = row.and_then(|r| r.tags.get(t).copied());
            }
        }
        if let Some(id) = element {
            self.hovered = Some(id);
            self.hover_from_panel = true;
        } else if self.hover_from_panel {
            self.hovered = None;
            self.hover_from_panel = false;
        }
        self.hovered_constraint = constraint;
    }

    /// The panel as widgets: the plane picker while a sketch waits for its
    /// plane, else the open sketch's panel.
    pub(crate) fn task_widgets(&mut self, ctx: &WorkbenchRuntimeContext) -> Vec<Widget> {
        if self.pending_creation.is_some() {
            return self.attachment_widgets(ctx);
        }
        let Some(feature) = self.get_active_sketch(ctx) else {
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
        let sketch = feature.sketch;
        let mut widgets = self.problem_widgets(&sketch);
        if self.sketch_picker.is_some() {
            widgets.extend(self.picker_widgets(ctx, &plane));
        }
        widgets.extend(self.tool_widgets());
        widgets.extend(self.array_widgets());
        widgets.extend(self.spline_widgets(&sketch));
        widgets.extend(self.text_widgets(&sketch));
        widgets.extend(self.image_widgets(ctx, &sketch));
        widgets.extend(self.list_widgets(ctx, &sketch));
        widgets.extend(self.placement_widgets(shift, on_datum));
        widgets.extend(self.settings_widgets(&sketch));
        widgets
    }

    /// The plane picker of a sketch about to be made: the face clicked,
    /// the body's datum planes (the selected one first), the origin's
    /// planes, and attachment modes on what is selected.
    fn attachment_widgets(&mut self, ctx: &WorkbenchRuntimeContext) -> Vec<Widget> {
        let Some(pending) = &self.pending_creation else {
            return Vec::new();
        };
        let body = pending.body;
        let datums = body.map_or_else(Vec::new, |body| {
            core_document::datums_of_body(ctx.document, body)
        });
        let selected_datum = pending
            .datum
            .filter(|id| datums.iter().any(|(d, ..)| d == id));
        let ask = match &pending.generator {
            Some(made_by) => format!("Choose the plane for the {}.", made_by.base_name()),
            None => "Choose the plane to sketch on.".to_string(),
        };
        let mut children = vec![
            panel::caption(
                &format!("{ask} An origin plane can be clicked in the view too."),
                Tone::Neutral,
                false,
            ),
            panel::gap(GapSize::Small),
        ];
        if pending.face.is_some() {
            children.push(panel::hinted(
                "Sketch on the face you clicked on the solid; the sketch follows the face",
                panel::button("attach.face", "Selected face", ButtonStyle::Secondary, true),
            ));
        }
        self.panel.datum_planes.clear();
        let mut datum_buttons = |only: bool, prefix: &str, out: &mut Vec<Widget>| {
            for (datum_id, name, datum) in &datums {
                if (Some(*datum_id) == selected_datum) != only {
                    continue;
                }
                for (label, hint, choice) in datum_planes(*datum_id, name, datum, prefix) {
                    let id = format!("attach.datum:{}", self.panel.datum_planes.len());
                    self.panel.datum_planes.push(choice);
                    out.push(panel::hinted(
                        hint,
                        panel::button(&id, &label, ButtonStyle::Secondary, true),
                    ));
                }
            }
        };
        datum_buttons(true, "Selected: ", &mut children);
        children.push(panel::flow_no_wrap(vec![
            panel::button("attach.base:xy", "Top (XY)", ButtonStyle::Secondary, true),
            panel::button("attach.base:xz", "Front (XZ)", ButtonStyle::Secondary, true),
            panel::button("attach.base:yz", "Side (YZ)", ButtonStyle::Secondary, true),
        ]));
        // Attached as a datum plane would be, by a mode on what is
        // selected on the body.
        self.panel.modes.clear();
        if let Some(body) = body {
            use core_document::attach::{PICK_MODES, Picked, candidate, mode_label};
            let picked = Picked::of(ctx, body);
            let frame = core_document::DatumFrame {
                origin: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
                x_axis: [1.0, 0.0, 0.0],
            };
            children.push(panel::caption(
                "Or by a mode, on what is selected",
                Tone::Muted,
                true,
            ));
            let mut modes = Vec::new();
            for (i, (mode, needs)) in PICK_MODES.iter().enumerate() {
                let made = candidate(mode, ctx, body, frame, None, &picked);
                let enabled = made.is_some();
                self.panel.modes.push(made);
                let mode_button = panel::button(
                    &format!("attach.mode:{i}"),
                    mode_label(mode),
                    ButtonStyle::PlainSmall,
                    enabled,
                );
                modes.push(if enabled || needs.is_empty() {
                    mode_button
                } else {
                    panel::hinted(needs, mode_button)
                });
            }
            children.push(panel::flow(modes));
        }
        // Datum planes of the target body attach the sketch to their
        // resolved frame; the selected one was offered first.
        datum_buttons(false, "", &mut children);
        children.push(panel::gap(GapSize::Small));
        children.push(panel::button(
            "attach.cancel",
            "Cancel",
            ButtonStyle::Secondary,
            true,
        ));
        vec![
            Card {
                icon: Some("sketch-map".into()),
                title: "Sketch attachment".into(),
                children,
                ..Default::default()
            }
            .into(),
        ]
    }

    /// What the header's status pill cannot do: a conflict or a
    /// redundancy names its constraints, and a click on it selects their
    /// geometry; with automatic update off, Solve now.
    fn problem_widgets(&mut self, sketch: &Sketch) -> Vec<Widget> {
        // Also keeps the diagnosis the header's pill reads.
        let state = self.diagnosed_verdict(sketch);
        let mut widgets = Vec::new();
        if !state.offenders.is_empty() {
            let kind = if state.kind == crate::SolverKind::Conflicting {
                NoteKind::Error
            } else {
                NoteKind::Warning
            };
            let text = match state.offenders.len() {
                1 => "Click to select the constraint's geometry.".to_string(),
                n => format!("Click to select the {n} constraints' geometry."),
            };
            widgets.push(
                bench_api::Alert {
                    id: Some("problems".into()),
                    kind,
                    title: Some(state.title),
                    text,
                }
                .into(),
            );
        }
        if !self.options.auto_update {
            widgets.push(panel::hinted(
                "Automatic update is off: solve the sketch as it stands",
                panel::button("solve_now", "Solve now", ButtonStyle::Small, true),
            ));
        }
        widgets
    }

    /// The document's other sketches, for carbon copy (a click copies one
    /// in) or merge (tick some, then merge); or, for external geometry,
    /// the other sketches and the datums. A sketch whose plane is at an
    /// angle to this one is listed but not offered.
    fn picker_widgets(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        plane: &SketchPlane,
    ) -> Vec<Widget> {
        let Some(picker) = self.sketch_picker.as_ref() else {
            return Vec::new();
        };
        self.panel.picked_from.clear();
        let mut children = Vec::new();
        if picker.mode == crate::SketchPickerMode::ExternalFrom {
            let mut choices: Vec<(FeatureId, String)> = self
                .other_sketches(ctx)
                .into_iter()
                .map(|(id, name, _)| (id, name))
                .collect();
            let mut datums: Vec<(u64, FeatureId, String)> = ctx
                .document
                .feature_tree()
                .all_nodes()
                .filter(|(_, n)| n.workbench_id.as_str() == core_document::DATUM_KIND)
                .map(|(id, n)| (n.seq, *id, n.name.clone()))
                .collect();
            datums.sort_by_key(|(seq, id, _)| (*seq, *id));
            choices.extend(datums.into_iter().map(|(_, id, name)| (id, name)));
            if choices.is_empty() {
                children.push(panel::note(
                    NoteKind::Info,
                    None,
                    "There is no other sketch and no datum in the document.",
                ));
            }
            for (i, (id, name)) in choices.into_iter().enumerate() {
                self.panel.picked_from.push(id);
                children.push(panel::button(
                    &format!("picker.copy:{i}"),
                    &name,
                    ButtonStyle::Plain,
                    true,
                ));
            }
            children.push(panel::button(
                "picker.close",
                "Close",
                ButtonStyle::Secondary,
                true,
            ));
            children.push(panel::gap(GapSize::Medium));
            return vec![section(
                "sketch_external_from",
                "External from",
                None,
                None,
                true,
                children,
            )];
        }
        let merging = picker.mode == crate::SketchPickerMode::Merge;
        let checked = picker.checked.clone();
        let others = self.other_sketches(ctx);
        if others.is_empty() {
            children.push(panel::note(
                NoteKind::Info,
                None,
                "There is no other sketch in the document.",
            ));
        }
        let why = "Its plane is at an angle to this one";
        for (i, (id, name, other)) in others.iter().enumerate() {
            self.panel.picked_from.push(*id);
            let parallel = crate::plane_map(&other.plane, plane).is_ok();
            children.push(if merging {
                Check {
                    id: format!("picker.tick:{i}"),
                    text: name.clone(),
                    on: checked.contains(id),
                    enabled: parallel,
                    hint: if parallel { String::new() } else { why.into() },
                    ..Default::default()
                }
                .into()
            } else {
                let copy = panel::button(
                    &format!("picker.copy:{i}"),
                    name,
                    ButtonStyle::Plain,
                    parallel,
                );
                if parallel {
                    copy
                } else {
                    panel::hinted(why, copy)
                }
            });
        }
        if merging {
            children.push(panel::hinted(
                "A new sketch of this one and the ticked ones; they stay as they are",
                panel::button(
                    "picker.merge",
                    "Merge into a new sketch",
                    ButtonStyle::Plain,
                    !checked.is_empty(),
                ),
            ));
        }
        children.push(panel::button(
            "picker.close",
            "Close",
            ButtonStyle::Secondary,
            true,
        ));
        children.push(panel::gap(GapSize::Medium));
        let title = if merging {
            "Merge sketches"
        } else {
            "Carbon copy"
        };
        vec![section("sketch_picker", title, None, None, true, children)]
    }

    /// Settings of the active drawing tool, when it has any.
    fn tool_widgets(&self) -> Vec<Widget> {
        let tool = self.last_tool.as_deref();
        let p = &self.tool_params;
        let fields: Vec<Widget> = match tool {
            Some("sketch.polygon") => vec![count(
                "tool.polygon_sides",
                "Sides",
                p.polygon_sides,
                3.0,
                12.0,
            )],
            Some("sketch.text") => {
                text_fields("text_draft", &self.text_draft, self.panel.font_modes[0])
            }
            Some("sketch.slot" | "sketch.arc_slot") => {
                vec![mm("tool.slot_width", "Width", p.slot_width)]
            }
            Some("sketch.fillet") => vec![
                mm("tool.fillet_radius", "Radius", p.fillet_radius),
                corner_check(p.corner_keep),
            ],
            Some("sketch.rect_rounded") => {
                vec![mm("tool.fillet_radius", "Radius", p.fillet_radius)]
            }
            Some("sketch.chamfer") => vec![
                mm("tool.chamfer_length", "Length", p.chamfer_length),
                corner_check(p.corner_keep),
            ],
            Some("sketch.rect_frame") => {
                vec![mm("tool.offset_distance", "Wall", p.offset_distance)]
            }
            Some("sketch.offset") => vec![
                mm("tool.offset_distance", "Distance", p.offset_distance),
                check(
                    "tool.offset_round",
                    "Corners",
                    "Round",
                    p.offset_round,
                    true,
                    "Arcs join the copies where they part at a corner",
                ),
                check("tool.offset_both", "Sides", "Both", p.offset_both, true, ""),
                check(
                    "tool.offset_delete",
                    "Original",
                    "Replace it",
                    p.offset_delete,
                    true,
                    "",
                ),
                check(
                    "tool.offset_linked",
                    "Copy",
                    "Follows the original",
                    p.offset_linked,
                    !p.offset_delete,
                    "One offset dimension holds the copy to the original",
                ),
            ],
            Some("sketch.mirror") => vec![
                check(
                    "tool.mirror_keep",
                    "Original",
                    "Keep it",
                    p.mirror_keep,
                    true,
                    "",
                ),
                check(
                    "tool.mirror_linked",
                    "Image",
                    "Follows the original",
                    p.mirror_linked,
                    p.mirror_keep,
                    "Symmetric constraints hold the image to the original",
                ),
                check(
                    "tool.mirror_center",
                    "About",
                    "A point",
                    p.mirror_center,
                    true,
                    "One click, the centre; off, a line or two points",
                ),
            ],
            Some(t @ ("sketch.translate" | "sketch.rotate" | "sketch.scale")) => {
                let mut fields = vec![count("tool.copies", "Copies", p.copies, 0.0, 64.0)];
                if t != "sketch.scale" {
                    let full_turn = t == "sketch.rotate" && p.rotate_full_turn;
                    fields.push(check("tool.copies_total", "Count", "Includes the original",
                        p.copies_total, true,
                        "The count is every element, the original included, and the clicks span the whole pattern"));
                    fields.push(check(
                        "tool.copies_symmetric",
                        "Sides",
                        "Symmetric",
                        p.copies_symmetric,
                        !full_turn,
                        "The copies repeat on the other side of the original",
                    ));
                    if t == "sketch.rotate" {
                        fields.push(check(
                            "tool.rotate_full_turn",
                            "Turn",
                            "Full turn",
                            p.rotate_full_turn,
                            true,
                            "The elements go evenly around 360°: click only the centre",
                        ));
                    }
                    fields.push(check(
                        "tool.copies_linked",
                        "Linked",
                        "Follow the original",
                        p.copies_linked,
                        p.copies > 0,
                        "Copies stay the original's size, spaced by one pitch",
                    ));
                }
                fields
            }
            Some("sketch.bspline") => vec![
                check(
                    "tool.bspline_periodic",
                    "Closed",
                    "Periodic",
                    p.bspline_periodic,
                    true,
                    "",
                ),
                check(
                    "tool.bspline_interpolate",
                    "Clicks",
                    "Through points",
                    p.bspline_interpolate,
                    true,
                    "",
                ),
                Quantity {
                    speed: Some(0.05),
                    ..integer(
                        "tool.bspline_degree",
                        "Degree",
                        p.bspline_degree,
                        2.0,
                        f64::from(crate::spline::MAX_DEGREE),
                    )
                }
                .into(),
            ],
            _ => return Vec::new(),
        };
        vec![section(
            "sketch_tool",
            "Tool",
            None,
            None,
            true,
            vec![grid("sketch_tool_grid", true, fields)],
        )]
    }

    /// With something selected: an array of it, its rows, columns and
    /// steps set here before it is made.
    fn array_widgets(&self) -> Vec<Widget> {
        if self.selected.is_empty() || !self.tool_state.is_idle() {
            return Vec::new();
        }
        let p = &self.tool_params;
        let fields = vec![
            count("array.rows", "Rows", p.array_rows, 1.0, 64.0),
            count("array.cols", "Columns", p.array_cols, 1.0, 64.0),
            offset("array.dx", "Column step", p.array_dx),
            offset("array.dy", "Row step", p.array_dy),
            check(
                "array.linked",
                "Linked",
                "Follow the original",
                p.copies_linked,
                true,
                "Copies stay the original's size, spaced by one pitch along the rows and one down the columns",
            ),
        ];
        vec![section(
            "sketch_array",
            "Array",
            None,
            None,
            false,
            vec![
                grid("sketch_array_grid", false, fields),
                panel::hinted(
                    "Copy the selection into these rows and columns",
                    panel::button("array.make", "Make array", ButtonStyle::Secondary, true),
                ),
            ],
        )]
    }

    /// With one spline selected: its degree and its knots, each with how
    /// many times it stands; with one of a spline's control points, its
    /// weight.
    fn spline_widgets(&mut self, sketch: &Sketch) -> Vec<Widget> {
        let (spline, weighted) = self.selected_spline(sketch);
        if spline.is_none() && weighted.is_none() {
            return Vec::new();
        }
        let mut fields = Vec::new();
        self.panel.knots.clear();
        if let Some((id, degree)) = spline {
            fields.push(stepper(
                "spline.degree",
                "Degree",
                degree as usize,
                &degree.to_string(),
                (
                    degree > 1,
                    "Lower the degree: the nearest curve of one less",
                ),
                (
                    degree < crate::spline::MAX_DEGREE,
                    "Raise the degree, the curve unchanged",
                ),
            ));
            for (i, (knot, times)) in crate::spline_edit::knots_of(sketch, id)
                .into_iter()
                .enumerate()
            {
                self.panel.knots.push((knot, times));
                let down = if times > 1 {
                    "Stand once less: a smoother curve near it"
                } else {
                    "Remove the knot: the nearest curve without it"
                };
                fields.push(stepper(
                    &format!("spline.knot:{i}"),
                    &format!("Knot {knot:.3}"),
                    times,
                    &format!("×{times}"),
                    (true, down),
                    (
                        times < degree as usize,
                        "Stand once more, the curve unchanged",
                    ),
                ));
            }
        }
        if let Some((owner, point)) = weighted {
            let now = crate::spline_edit::weight_of(sketch, owner, point).unwrap_or(1.0);
            fields.push(
                Quantity {
                    id: "spline.weight".into(),
                    label: "Weight".into(),
                    value: now,
                    dim: Dim::Number,
                    min: Some(0.01),
                    max: Some(100.0),
                    decimals: 3,
                    speed: Some(0.01),
                    ..Default::default()
                }
                .into(),
            );
        }
        vec![section(
            "sketch_spline",
            "Spline",
            None,
            None,
            true,
            vec![grid("sketch_spline_grid", false, fields)],
        )]
    }

    /// The one selected spline with its degree, and the selected control
    /// point of a spline with its owner.
    fn selected_spline(&self, sketch: &Sketch) -> SelectedSpline {
        let splines: Vec<&sketch::BSpline> = sketch
            .geometry
            .iter()
            .filter_map(|g| match g {
                sketch::GeometryElement::BSpline(b) if self.selected.contains(&b.id) => Some(b),
                _ => None,
            })
            .collect();
        let weighted = self.selected.iter().find_map(|id| {
            sketch.geometry.iter().find_map(|g| match g {
                sketch::GeometryElement::BSpline(b) if b.control_points.contains(id) => {
                    Some((b.id, *id))
                }
                _ => None,
            })
        });
        if splines.len() != 1 && weighted.is_none() {
            return (None, None);
        }
        (splines.first().map(|b| (b.id, b.degree)), weighted)
    }

    /// With a piece of a text block selected: what it says and how, made
    /// again when a field is changed.
    fn text_widgets(&self, sketch: &Sketch) -> Vec<Widget> {
        let Some(block) = self
            .selected
            .iter()
            .find_map(|id| crate::text::block_of(sketch, *id))
        else {
            return Vec::new();
        };
        let spec = crate::text::TextSpec::of(block);
        let fields = text_fields("text_block", &spec, self.panel.font_modes[1]);
        vec![section(
            "sketch_text",
            "Text",
            None,
            None,
            true,
            vec![grid("sketch_text_grid", false, fields)],
        )]
    }

    /// The sketch's reference pictures: each one's middle, width, turn and
    /// how much of it shows, its calibration, and a way to take it away.
    fn image_widgets(&self, ctx: &WorkbenchRuntimeContext, sketch: &Sketch) -> Vec<Widget> {
        if sketch.images.is_empty() {
            return Vec::new();
        }
        let mut children = Vec::new();
        for (n, image) in sketch.images.iter().enumerate() {
            children.push(panel::caption(
                &format!("Picture {}", n + 1),
                Tone::Muted,
                true,
            ));
            let field = |key: &str| format!("image:{n}.{key}");
            children.push(grid(
                &format!("sketch_image_grid:{n}"),
                false,
                vec![
                    length(&field("x"), "Middle x", image.center.x, None),
                    length(&field("y"), "Middle y", image.center.y, None),
                    length(&field("width"), "Width", image.width, Some((0.01, 1.0e6))),
                    degrees(&field("angle"), "Turn", image.angle_deg),
                    Quantity {
                        id: field("opacity"),
                        label: "Opacity".into(),
                        value: image.opacity,
                        dim: Dim::Number,
                        min: Some(0.05),
                        max: Some(1.0),
                        speed: Some(0.01),
                        ..Default::default()
                    }
                    .into(),
                ],
            ));
            match self.calibrating.as_ref().filter(|c| c.image == image.id) {
                Some(calibrating) => {
                    children.extend(calibration_widgets(ctx, image, calibrating));
                }
                None => children.push(panel::flow_no_wrap(vec![
                    panel::hinted(
                        "Pick two points on the picture and say how far apart they really are",
                        panel::button(
                            &field("calibrate"),
                            "Calibrate",
                            ButtonStyle::Secondary,
                            true,
                        ),
                    ),
                    panel::button(&field("remove"), "Remove", ButtonStyle::Secondary, true),
                ])),
            }
            children.push(panel::gap(GapSize::Small));
        }
        vec![section(
            "sketch_images",
            "Reference images",
            None,
            None,
            true,
            children,
        )]
    }

    /// One filter over both lists, then the elements and the constraints,
    /// each folding on its own.
    fn list_widgets(&mut self, ctx: &WorkbenchRuntimeContext, sketch: &Sketch) -> Vec<Widget> {
        let choice = if self.element_filter != ElementFilter::All {
            ListFilter::Elements(self.element_filter)
        } else {
            ListFilter::Constraints(self.constraint_kind_filter)
        };
        // What the rows light comes from what last frame left in hand: the
        // item the panel hovered, and the selection.
        let relations = Relations::of(sketch);
        let panel_element = self.hovered.filter(|_| self.hover_from_panel);
        let hand = InHand {
            related: relations.related(
                self.selected.iter().copied().chain(panel_element),
                self.selected_constraints
                    .iter()
                    .copied()
                    .chain(self.hovered_constraint),
            ),
            relations,
            element: self.hovered,
            constraint: self.hovered_constraint,
        };
        let elements = self.element_items(ctx, sketch, &hand);
        let mut constraint_children = Vec::new();
        let shown = self.listed_constraints(sketch);
        // While a filter narrows the list, what it shows can go at once.
        if !shown.is_empty() && shown.len() < sketch.constraints.len() {
            constraint_children.push(panel::hinted(
                "Delete every constraint the filter shows",
                panel::button(
                    "delete_shown",
                    &format!("Delete {} shown", shown.len()),
                    ButtonStyle::Small,
                    true,
                ),
            ));
        }
        let constraints = self.constraint_items(ctx, sketch, &hand, &shown);
        constraint_children.push(
            ItemList {
                id: "constraints".into(),
                items: constraints,
                empty: sketch
                    .constraints
                    .is_empty()
                    .then(|| "None yet. Select geometry and pick a constraint tool.".into()),
                ..Default::default()
            }
            .into(),
        );
        self.pending_focus = None;
        vec![
            panel::gap(GapSize::Medium),
            FilterField {
                id: "find".into(),
                hint: "Filter".into(),
                kinds: ListFilter::ALL.iter().map(|(_, k)| k.to_string()).collect(),
                kind: ListFilter::ALL
                    .iter()
                    .position(|(f, _)| *f == choice)
                    .unwrap_or(0),
                by_bench: true,
            }
            .into(),
            panel::gap(GapSize::Small),
            section(
                "sketch_elements",
                "Elements",
                Some(sketch.geometry.len()),
                None,
                true,
                vec![
                    ItemList {
                        id: "elements".into(),
                        items: elements,
                        empty: sketch
                            .geometry
                            .is_empty()
                            .then(|| "No geometry yet. Draw with the tools above.".into()),
                        ..Default::default()
                    }
                    .into(),
                ],
            ),
            section(
                "sketch_constraints",
                "Constraints",
                Some(sketch.constraints.len()),
                None,
                true,
                constraint_children,
            ),
        ]
    }

    /// The element rows: the sketch's reference geometry, then its
    /// elements as the filter passes them, each with its size and a badge
    /// per constraint on it.
    fn element_items(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        sketch: &Sketch,
        hand: &InHand,
    ) -> Vec<Item> {
        self.panel.elements.clear();
        let mut items = Vec::new();
        if sketch.geometry.is_empty() {
            return items;
        }
        let text = self.constraint_filter.trim().to_lowercase();
        let named = |name: &str| text.is_empty() || name.to_lowercase().contains(&text);
        // The reference geometry every sketch has: pickable from here as
        // well as from the viewport, but never drawn into a shape.
        if self.element_filter == ElementFilter::All {
            for (reference, id) in [
                (sketch::Reference::Origin, sketch::ORIGIN_ID),
                (sketch::Reference::XAxis, sketch::X_AXIS_ID),
                (sketch::Reference::YAxis, sketch::Y_AXIS_ID),
            ] {
                if !named(reference.label()) {
                    continue;
                }
                self.panel.elements.push(ElementRow {
                    id,
                    badges: Vec::new(),
                });
                items.push(Item {
                    icon: Some(
                        if reference.is_point() {
                            "point"
                        } else {
                            "line"
                        }
                        .into(),
                    ),
                    muted: true,
                    label: reference.label().into(),
                    value: Some("reference".into()),
                    value_tone: Tone::Muted,
                    selected: self.selected.contains(&id),
                    related: hand.related.elements.contains(&id),
                    lit: hand.element == Some(id),
                    ..Default::default()
                });
            }
        }
        let pal = ctx.sketch_palette;
        let diagnosis = self.last_diagnosis.clone().unwrap_or_default();
        let formulas = Formulas {
            document: ctx.document,
            sketch_id: self.active_sketch_id,
        };
        // Projected elements whose edge, face, element or datum is gone.
        let lost: std::collections::HashSet<Uuid> = self
            .active_sketch_id
            .map(|sketch_id| self.lost_external(ctx.document, ctx.kernel, sketch_id))
            .unwrap_or_default()
            .into_iter()
            .collect();
        let external = sketch.external_ids();
        let locked = self.locked_elements(ctx, sketch);
        for geom in &sketch.geometry {
            let id = geom.id();
            if !self.element_filter.accepts(sketch, id) {
                continue;
            }
            let name = element_name(sketch, id);
            if !named(&name) {
                continue;
            }
            let style = overlay::element_style(sketch, id, &self.selected, None, &locked, &pal);
            let color = style.color.map(|c| (c * 255.0) as u8);
            let detail = if external.contains(&id) {
                Some("projected")
            } else if sketch.is_construction(id) {
                Some("construction")
            } else {
                None
            };
            // A badge for each constraint on the element or its points.
            let on: Vec<&Constraint> = hand
                .relations
                .constraints_on(id)
                .iter()
                .filter_map(|c| sketch.constraints.iter().find(|k| k.id == *c))
                .collect();
            let badges = on
                .iter()
                .map(|c| {
                    let (tone, color) = constraint_tint(c, &diagnosis, &formulas);
                    Badge {
                        icon: constraint_icon(&c.kind).into(),
                        hint: crate::constraint_row_label(c),
                        lit: hand.constraint == Some(c.id)
                            || self.selected_constraints.contains(&c.id),
                        tone,
                        color,
                    }
                })
                .collect();
            let construction = if sketch.is_construction(id) {
                "Make normal geometry"
            } else {
                "Make construction"
            };
            self.panel.elements.push(ElementRow {
                id,
                badges: on.iter().map(|c| c.id).collect(),
            });
            items.push(Item {
                icon: Some(element_row_icon(geom, external.contains(&id)).into()),
                color: Some([color[0], color[1], color[2]]),
                label: name,
                secondary: detail.map(String::from),
                value: Some(element_value(sketch, geom)),
                flag: lost.contains(&id).then(|| StatusLine {
                    text: "lost".into(),
                    tone: Tone::Warning,
                    detail: "What it was projected from is gone: it stays where it was".into(),
                }),
                badges,
                selected: self.selected.contains(&id),
                related: hand.related.elements.contains(&id),
                lit: hand.element == Some(id),
                reveal: self.reveal(id),
                menu: vec![
                    action("construction", construction, "", false),
                    action("delete", "Delete", "", true),
                ],
                ..Default::default()
            });
        }
        items
    }

    /// The constraint rows the filter shows: each names the elements it
    /// ties, and a dimension's value is a field while the row is in hand.
    fn constraint_items(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        sketch: &Sketch,
        hand: &InHand,
        shown: &[Uuid],
    ) -> Vec<Item> {
        self.panel.constraints.clear();
        let diagnosis = self.last_diagnosis.clone().unwrap_or_default();
        let formulas = Formulas {
            document: ctx.document,
            sketch_id: self.active_sketch_id,
        };
        let listed: std::collections::HashSet<&Uuid> = shown.iter().collect();
        let mut items = Vec::new();
        for constraint in &sketch.constraints {
            if !listed.contains(&constraint.id) {
                continue;
            }
            let id = constraint.id;
            let selected = self.selected_constraints.contains(&id);
            let operands = hand.relations.operands(id).to_vec();
            let tags = operands
                .iter()
                .map(|e| Tag {
                    text: element_name(sketch, *e),
                    selected: self.selected.contains(e),
                    lit: hand.element == Some(*e),
                    ..Default::default()
                })
                .collect();
            // The value is plain text until the row is the one in hand.
            let live = selected || hand.constraint == Some(id) || self.pending_focus == Some(id);
            let (value, value_tone, value_hint, field) = if live {
                dimension_cell(
                    sketch,
                    constraint,
                    self.active_sketch_id,
                    self.pending_focus == Some(id),
                )
            } else {
                match dimension_text(sketch, constraint) {
                    Some((text, tone)) => (Some(text), tone, None, None),
                    None => (None, Tone::Neutral, None, None),
                }
            };
            let renaming = self.renaming_constraint == Some(id);
            let mut label = if renaming {
                constraint.name.clone().unwrap_or_default()
            } else {
                crate::constraint_row_label(constraint)
            };
            if constraint.parked && !renaming {
                label.push_str(" (parked)");
            }
            let mut menu = vec![action(
                "active",
                if constraint.active {
                    "Deactivate"
                } else {
                    "Activate"
                },
                "",
                false,
            )];
            if constraint.kind.is_dimensional() {
                menu.push(action(
                    "driving",
                    if constraint.driving {
                        "Make reference"
                    } else {
                        "Make driving"
                    },
                    "Reference dimensions are measured, not enforced",
                    false,
                ));
            }
            menu.push(action("rename", "Rename", "", false));
            menu.push(action(
                "park",
                if constraint.parked { "Unpark" } else { "Park" },
                "Parked symbols draw only while the parked layer shows",
                false,
            ));
            menu.push(action("delete", "Delete", "", true));
            self.panel
                .constraints
                .push(ConstraintRow { id, tags: operands });
            let (tone, color) = constraint_tint(constraint, &diagnosis, &formulas);
            items.push(Item {
                icon: Some(constraint_icon(&constraint.kind).into()),
                tone,
                color,
                muted: !constraint.active,
                problem: diagnosis.conflicting.contains(&id) || diagnosis.redundant.contains(&id),
                label,
                value,
                value_tone,
                value_hint,
                field,
                tags,
                selected,
                related: hand.related.constraints.contains(&id),
                lit: hand.constraint == Some(id),
                hint: Some(sketch::constraint_label(&constraint.kind)),
                editing: renaming,
                reveal: self.reveal(id),
                menu,
                ..Default::default()
            });
        }
        items
    }

    /// Whether the row of `id` is the one to bring into view; it is
    /// brought once.
    fn reveal(&mut self, id: Uuid) -> bool {
        let reveal = self.scroll_to_row == Some(id);
        if reveal {
            self.scroll_to_row = None;
        }
        reveal
    }

    /// Where the sketch sits off what it was placed on (its datum, plane,
    /// face or attachment): along the normal, across the plane and turned
    /// about it.
    fn placement_widgets(&self, placed: crate::feature::PlaneShift, on_datum: bool) -> Vec<Widget> {
        if self.active_sketch_id.is_none() {
            return Vec::new();
        }
        let moved = placed.offset != 0.0 || placed.shift != [0.0; 2] || placed.turn != 0.0;
        let summary = if moved {
            format!(
                "offset {:.2} mm, moved {:.2}, {:.2}, turned {:.1}°",
                placed.offset, placed.shift[0], placed.shift[1], placed.turn
            )
        } else {
            "no offset".to_string()
        };
        let mut children = Vec::new();
        if !on_datum {
            children.push(panel::caption(
                "Moves the sketch off the plane or face it was placed on",
                Tone::Muted,
                true,
            ));
        }
        children.push(grid(
            "sketch_attachment_grid",
            false,
            vec![
                offset("placement.offset", "Along normal", placed.offset),
                offset("placement.x", "Across x", placed.shift[0]),
                offset("placement.y", "Across y", placed.shift[1]),
                degrees("placement.turn", "Turned", placed.turn),
            ],
        ));
        vec![section(
            "sketch_attachment",
            "Placement",
            None,
            Some(summary),
            false,
            children,
        )]
    }

    /// What is set once and seldom changed, folded away at the panel's
    /// end: how the solver runs, and how drawing snaps and shows.
    fn settings_widgets(&self, sketch: &Sketch) -> Vec<Widget> {
        let o = &self.options;
        let exponent = -(sketch.solver.tolerance.log10().round() as i32);
        let solver_summary = format!(
            "{} · 1e-{exponent}",
            if o.auto_update { "auto" } else { "manual" }
        );
        let last = match self.last_solve {
            Some(crate::operations::SolveOutcome::Converged { iterations }) => {
                format!("Last solve: settled in {iterations} step(s)")
            }
            Some(crate::operations::SolveOutcome::NotConverged { residual }) => {
                format!("Last solve: stopped with {residual:.2e} left")
            }
            Some(crate::operations::SolveOutcome::NothingToSolve) => {
                "Last solve: nothing to solve".to_string()
            }
            None => String::new(),
        };
        let mut solver = vec![panel::flow_no_wrap(vec![
            check(
                "solver.auto_update",
                "",
                "Auto update",
                o.auto_update,
                true,
                "",
            ),
            check(
                "solver.auto_remove",
                "",
                "Auto remove redundants",
                o.auto_remove_redundant,
                true,
                "",
            ),
        ])];
        if !last.is_empty() {
            solver.push(panel::caption(&last, Tone::Muted, true));
        }
        solver.push(grid(
            "sketch_solver_grid",
            false,
            vec![
                Quantity {
                    speed: Some(1.0),
                    ..integer(
                        "solver.iterations",
                        "Most steps",
                        sketch.solver.max_iterations,
                        1.0,
                        10_000.0,
                    )
                }
                .into(),
                Select {
                    id: "solver.tolerance".into(),
                    label: "Solved within".into(),
                    options: TOLERANCES.map(|e| format!("1e-{e}")).to_vec(),
                    selected: TOLERANCES
                        .iter()
                        .position(|e| *e == exponent)
                        .unwrap_or(usize::MAX),
                    width: Some(90.0),
                    ..Default::default()
                }
                .into(),
            ],
        ));
        let snaps: Vec<&str> = [
            (!self.snap_off, "snap to objects"),
            (o.grid_snap, "snap to grid"),
            (o.grid_on, "grid"),
        ]
        .into_iter()
        .filter_map(|(on, what)| on.then_some(what))
        .collect();
        let display_summary = if snaps.is_empty() {
            "no snapping".to_string()
        } else {
            snaps.join(", ")
        };
        let toggle =
            |id: &str, text: &str, on: bool, hint: &str| check(id, "", text, on, true, hint);
        let display = vec![
            panel::inline(choose(
                "display.labels",
                "Labels show",
                &crate::glyphs::DimensionLabels::ALL,
                o.dimension_labels,
                130.0,
            )),
            panel::hinted(
                "With Shift held, a segment leaves its start in steps of this angle",
                panel::inline(choose(
                    "display.angle_step",
                    "Angle snap step",
                    &crate::ANGLE_STEPS,
                    o.angle_step_deg,
                    90.0,
                )),
            ),
            toggle("display.grid_on", "Show grid", o.grid_on, ""),
            toggle("display.grid_auto", "Grid auto spacing", o.grid_auto, ""),
            toggle("display.grid_snap", "Snap to grid", o.grid_snap, ""),
            toggle("display.snap", "Snap to objects", !self.snap_off, ""),
            toggle(
                "display.avoid_redundant",
                "Avoid redundant auto constraints",
                o.avoid_redundant_auto,
                "",
            ),
            toggle(
                "display.auto_constraints",
                "Auto constraints",
                o.auto_constraints,
                "",
            ),
            toggle(
                "display.continue_drawing",
                "Continue drawing",
                o.continue_drawing,
                "A drawing tool stays in hand after its shape; off, the pointer goes back to Select",
            ),
            toggle(
                "display.continue_constraining",
                "Continue constraining",
                o.continue_constraining,
                "A constraint tool waits for the next elements after its constraint; off, it is put away",
            ),
            toggle(
                "display.arc_helpers",
                "Arc helpers",
                o.arc_helpers,
                "While drawing, each arc's whole circle shows as a dashed guide a point can snap onto",
            ),
            panel::caption("Splines", Tone::Neutral, false),
            panel::flow(vec![
                toggle(
                    "display.spline_polygon",
                    "Control polygon",
                    o.spline_polygon,
                    "",
                ),
                toggle("display.spline_degree", "Degree", o.spline_degree, ""),
                toggle(
                    "display.spline_multiplicity",
                    "Knot multiplicity",
                    o.spline_multiplicity,
                    "",
                ),
                toggle("display.spline_weights", "Weights", o.spline_weights, ""),
            ]),
            grid(
                "sketch_edit_grid",
                false,
                vec![
                    panel::caption("Grid size", Tone::Neutral, true),
                    panel::caption("Rendering order", Tone::Neutral, true),
                    Quantity {
                        width: Some(90.0),
                        enabled: !o.grid_auto,
                        ..mm_quantity("display.grid_size", "", o.grid_size)
                    }
                    .into(),
                    choose(
                        "display.render_order",
                        "",
                        &[(false, "Normal on top"), (true, "Construction on top")],
                        o.construction_on_top,
                        130.0,
                    ),
                ],
            ),
        ];
        vec![
            section(
                "sketch_solver",
                "Solver",
                None,
                Some(solver_summary),
                false,
                solver,
            ),
            section(
                "sketch_display",
                "Display & snapping",
                None,
                Some(display_summary),
                false,
                display,
            ),
        ]
    }

    /// A change to the panel.
    pub(crate) fn task_event(&mut self, ctx: &mut WorkbenchRuntimeContext, event: &PanelEvent) {
        let id = event.id();
        if let Some(rest) = id.strip_prefix("attach.") {
            self.attachment_event(ctx, rest);
            return;
        }
        match event {
            PanelEvent::Button { id } => self.button_event(ctx, id),
            PanelEvent::Number { id, value } => self.number_event(ctx, id, *value),
            PanelEvent::Toggle { id, on } => self.toggle_event(ctx, id, *on),
            PanelEvent::Choice { id, index } => self.choice_event(ctx, id, *index),
            PanelEvent::Text { id, value } => self.text_event(ctx, id, value),
            PanelEvent::Filter { text, kind, .. } => {
                self.constraint_filter.clone_from(text);
                match ListFilter::ALL.get(*kind).map(|(f, _)| *f) {
                    Some(ListFilter::Elements(e)) => {
                        self.element_filter = e;
                        self.constraint_kind_filter = ConstraintFilter::All;
                    }
                    Some(ListFilter::Constraints(c)) => {
                        self.constraint_kind_filter = c;
                        self.element_filter = ElementFilter::All;
                    }
                    None => {}
                }
            }
            PanelEvent::HoverPart {
                id,
                index,
                tag,
                badge,
            } => {
                let list = usize::from(id == "constraints");
                self.panel.pointed[list] = (*index, *tag, *badge);
            }
            // As the viewport does: a click adds to the selection, a
            // second click takes it out.
            PanelEvent::Select { id, index } => match id.as_str() {
                "elements" => {
                    if let Some(row) = self.panel.elements.get(*index)
                        && !self.selected.remove(&row.id)
                    {
                        self.selected.insert(row.id);
                    }
                }
                "constraints" => {
                    if let Some(row) = self.panel.constraints.get(*index)
                        && !self.selected_constraints.remove(&row.id)
                    {
                        self.selected_constraints.insert(row.id);
                    }
                }
                _ => {}
            },
            // A chip picks the element it names as its row does, and
            // brings that row into view.
            PanelEvent::Tag { index, tag, .. } => {
                if let Some(element) = self
                    .panel
                    .constraints
                    .get(*index)
                    .and_then(|r| r.tags.get(*tag).copied())
                {
                    if !self.selected.remove(&element) {
                        self.selected.insert(element);
                    }
                    self.scroll_to_row = Some(element);
                }
            }
            // A badge picks its constraint as the constraint's row does.
            PanelEvent::Badge { index, badge, .. } => {
                if let Some(constraint) = self
                    .panel
                    .elements
                    .get(*index)
                    .and_then(|r| r.badges.get(*badge).copied())
                {
                    if !self.selected_constraints.remove(&constraint) {
                        self.selected_constraints.insert(constraint);
                    }
                    self.scroll_to_row = Some(constraint);
                }
            }
            PanelEvent::ContextMenu { id, index, action } => match id.as_str() {
                "elements" => {
                    if let Some(row) = self.panel.elements.get(*index) {
                        let element = row.id;
                        self.element_menu(ctx, element, action);
                    }
                }
                "constraints" => {
                    if let Some(row) = self.panel.constraints.get(*index) {
                        let constraint = row.id;
                        self.constraint_menu(ctx, constraint, action);
                    }
                }
                _ => {}
            },
            PanelEvent::ItemText {
                index, value, done, ..
            } => {
                let Some(id) = self.panel.constraints.get(*index).map(|r| r.id) else {
                    return;
                };
                if let Some((idx, mut c)) = self.constraint_at(ctx, id)
                    && c.name.as_deref().unwrap_or_default() != value
                {
                    c.name = (!value.is_empty()).then(|| value.clone());
                    self.update_constraint(ctx, idx, c);
                }
                if *done {
                    self.renaming_constraint = None;
                }
            }
            _ => {}
        }
    }

    /// Constraint `id` of the open sketch, and its place.
    fn constraint_at(
        &self,
        ctx: &WorkbenchRuntimeContext,
        id: Uuid,
    ) -> Option<(usize, Constraint)> {
        let feature = self.get_active_sketch(ctx)?;
        feature
            .sketch
            .constraints
            .iter()
            .position(|c| c.id == id)
            .map(|i| (i, feature.sketch.constraints[i].clone()))
    }

    fn element_menu(&mut self, ctx: &mut WorkbenchRuntimeContext, id: Uuid, action: &str) {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return;
        };
        match action {
            "construction" => {
                let flag = !feature.sketch.is_construction(id);
                feature.sketch.set_construction(id, flag);
                self.store_sketch(ctx, feature);
            }
            "delete" => {
                let removed = feature.sketch.remove_geometry_cascade(&[id]);
                if !removed.is_empty() {
                    for rid in &removed {
                        self.selected.remove(rid);
                    }
                    self.hovered = None;
                    self.solve(ctx, &mut feature);
                    ctx.log_info(format!("Deleted {} sketch element(s)", removed.len()));
                    self.store_sketch(ctx, feature);
                }
            }
            _ => {}
        }
    }

    fn constraint_menu(&mut self, ctx: &mut WorkbenchRuntimeContext, id: Uuid, action: &str) {
        let Some((idx, mut c)) = self.constraint_at(ctx, id) else {
            return;
        };
        match action {
            "active" => c.active = !c.active,
            "driving" => c.driving = !c.driving,
            "park" => c.parked = !c.parked,
            "rename" => {
                self.renaming_constraint = Some(id);
                return;
            }
            "delete" => {
                if let Some(mut feature) = self.get_active_sketch(ctx) {
                    feature.sketch.constraints.remove(idx);
                    self.solve(ctx, &mut feature);
                    self.store_sketch(ctx, feature);
                }
                return;
            }
            _ => return,
        }
        self.update_constraint(ctx, idx, c);
    }

    fn button_event(&mut self, ctx: &mut WorkbenchRuntimeContext, id: &str) {
        match id {
            "problems" => {
                let Some(feature) = self.get_active_sketch(ctx) else {
                    return;
                };
                let state = self.diagnosed_verdict(&feature.sketch);
                self.selected.clear();
                for constraint in &feature.sketch.constraints {
                    if state.offenders.contains(&constraint.id) {
                        self.selected
                            .extend(sketch::constraint_refs(&constraint.kind));
                    }
                }
            }
            "solve_now" => self.solve_from_panel(ctx),
            "picker.close" => self.sketch_picker = None,
            "picker.merge" => self.merge_sketches(ctx),
            "array.make" => {
                self.array_selection(ctx);
            }
            "delete_shown" => {
                if let Some(feature) = self.get_active_sketch(ctx) {
                    let shown = self.listed_constraints(&feature.sketch);
                    self.delete_items(ctx, shown);
                }
            }
            "calib.apply" => {
                let placed = self.calibration_result(ctx);
                self.calibrating = None;
                if let Some((image, placed)) = placed {
                    self.set_image(
                        ctx,
                        serde_json::json!({
                            "image": image.to_string(), "x": placed.center.x, "y": placed.center.y,
                            "width": placed.width, "angle": placed.angle_deg,
                        }),
                    );
                }
            }
            "calib.cancel" => self.calibrating = None,
            _ => {
                if let Some(i) = panel::indexed(id, "picker.copy:") {
                    self.picker_choice(ctx, i);
                } else if let Some((n, key)) = image_field(id) {
                    self.image_button(ctx, n, key);
                }
            }
        }
    }

    /// The picker's `i`th sketch or datum clicked: copied in, or brought
    /// in as external geometry.
    fn picker_choice(&mut self, ctx: &mut WorkbenchRuntimeContext, i: usize) {
        let Some(from) = self.panel.picked_from.get(i).copied() else {
            return;
        };
        let external = self
            .sketch_picker
            .as_ref()
            .is_some_and(|p| p.mode == crate::SketchPickerMode::ExternalFrom);
        if !external {
            self.carbon_copy(ctx, from);
            return;
        }
        let Some(sketch) = self.active_sketch_id else {
            return;
        };
        let args = crate::commands::args(serde_json::json!({
            "sketch": sketch.0.to_string(),
            "from": from.0.to_string(),
            "counts": !self.construction_mode(),
        }));
        match crate::commands::run("sketch.external_from", &args, ctx) {
            Ok(made) => {
                ctx.record("sketch.external_from", args, made);
                self.sketch_picker = None;
            }
            Err(err) => ctx.log_warn(err.to_string()),
        }
    }

    fn image_button(&mut self, ctx: &mut WorkbenchRuntimeContext, n: usize, key: &str) {
        let Some(image) = self.image_at(ctx, n).map(|i| i.id) else {
            return;
        };
        match key {
            "calibrate" => {
                self.calibrating = Some(crate::Calibrating {
                    image,
                    from: None,
                    to: None,
                    length: String::new(),
                    straighten: false,
                    centre: false,
                });
            }
            "remove" => self.set_image(
                ctx,
                serde_json::json!({"image": image.to_string(), "remove": true}),
            ),
            _ => {}
        }
    }

    /// The open sketch's `n`th picture.
    fn image_at(&self, ctx: &WorkbenchRuntimeContext, n: usize) -> Option<sketch::ReferenceImage> {
        self.get_active_sketch(ctx)?.sketch.images.get(n).cloned()
    }

    /// `sketch.set_image` with `args`, run and recorded.
    fn set_image(&mut self, ctx: &mut WorkbenchRuntimeContext, mut args: serde_json::Value) {
        let Some(sketch_id) = self.active_sketch_id else {
            return;
        };
        args["sketch"] = serde_json::json!(sketch_id.0.to_string());
        let args = crate::commands::args(args);
        if crate::commands::run("sketch.set_image", &args, ctx).is_ok() {
            ctx.record("sketch.set_image", args, serde_json::Value::Null);
        }
    }

    /// The picture under calibration as it would lie once applied, when
    /// both points are picked and the length reads.
    fn calibration_result(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> Option<(Uuid, sketch::ReferenceImage)> {
        let calibrating = self.calibrating.as_ref()?;
        let feature = self.get_active_sketch(ctx)?;
        let image = feature
            .sketch
            .images
            .iter()
            .find(|i| i.id == calibrating.image)?;
        calibrated(ctx, image, calibrating)
            .ok()
            .flatten()
            .map(|placed| (image.id, placed))
    }

    fn number_event(&mut self, ctx: &mut WorkbenchRuntimeContext, id: &str, value: f64) {
        let whole = || value.round().max(0.0) as u32;
        let p = &mut self.tool_params;
        match id {
            "tool.polygon_sides" => p.polygon_sides = whole(),
            "tool.slot_width" => p.slot_width = value,
            "tool.fillet_radius" => p.fillet_radius = value,
            "tool.chamfer_length" => p.chamfer_length = value,
            "tool.offset_distance" => p.offset_distance = value,
            "tool.copies" => p.copies = whole(),
            "tool.bspline_degree" => p.bspline_degree = whole(),
            "array.rows" => p.array_rows = whole(),
            "array.cols" => p.array_cols = whole(),
            "array.dx" => p.array_dx = value,
            "array.dy" => p.array_dy = value,
            "display.grid_size" => self.options.grid_size = value,
            "solver.iterations" => {
                self.solver_settings(ctx, serde_json::json!({"iterations": value.round()}));
            }
            "spline.degree" => self.spline_degree(ctx, value),
            "spline.weight" => self.spline_weight(ctx, value),
            _ => {
                if let Some(constraint) = id
                    .strip_prefix("dim:")
                    .and_then(|u| Uuid::parse_str(u).ok())
                {
                    // A value typed into a dimension's row opens its task.
                    self.open_dimension_task(ctx, constraint, &format!("{value}"));
                } else if let Some(i) = panel::indexed(id, "spline.knot:") {
                    self.knot_multiplicity(ctx, i, whole() as usize);
                } else if let Some(key) = id.strip_prefix("placement.") {
                    self.placement_event(ctx, key, value);
                } else if let Some((n, key)) = image_field(id) {
                    self.image_number(ctx, n, key, value);
                } else if let Some(field) = id.strip_prefix("text_draft.") {
                    text_number(&mut self.text_draft, field, value);
                } else if let Some(field) = id.strip_prefix("text_block.") {
                    self.text_block_edit(ctx, |spec| text_number(spec, field, value));
                }
            }
        }
    }

    fn spline_degree(&mut self, ctx: &mut WorkbenchRuntimeContext, value: f64) {
        let Some(sketch) = self.get_active_sketch(ctx).map(|f| f.sketch) else {
            return;
        };
        let Some((_, degree)) = self.selected_spline(&sketch).0 else {
            return;
        };
        let by = value.round() as i32 - degree as i32;
        if by != 0 {
            self.selection_edit(
                ctx,
                "sketch.spline_degree",
                serde_json::json!({ "by": by }),
                |s, sel| crate::spline_edit::change_degree(s, sel, by),
            );
        }
    }

    fn knot_multiplicity(&mut self, ctx: &mut WorkbenchRuntimeContext, i: usize, times: usize) {
        let Some(sketch) = self.get_active_sketch(ctx).map(|f| f.sketch) else {
            return;
        };
        let (Some((id, _)), Some(&(knot, _))) =
            (self.selected_spline(&sketch).0, self.panel.knots.get(i))
        else {
            return;
        };
        self.sketch_edit(
            ctx,
            "sketch.knot_multiplicity",
            serde_json::json!({ "spline": id.to_string(), "knot": knot, "multiplicity": times }),
            |s| {
                if times == 0 {
                    crate::spline_edit::remove_knot(s, id, knot)
                } else {
                    crate::spline_edit::set_multiplicity(s, id, knot, times)
                }
            },
        );
    }

    fn spline_weight(&mut self, ctx: &mut WorkbenchRuntimeContext, w: f64) {
        let Some(sketch) = self.get_active_sketch(ctx).map(|f| f.sketch) else {
            return;
        };
        let Some((owner, point)) = self.selected_spline(&sketch).1 else {
            return;
        };
        if crate::spline_edit::weight_of(&sketch, owner, point) == Some(w) {
            return;
        }
        self.sketch_edit(
            ctx,
            "sketch.spline_weight",
            serde_json::json!({ "spline": owner.to_string(), "point": point.to_string(), "weight": w }),
            |s| crate::spline_edit::set_weight(s, owner, point, w),
        );
    }

    fn placement_event(&mut self, ctx: &mut WorkbenchRuntimeContext, key: &str, value: f64) {
        let (Some(id), Some(feature)) = (self.active_sketch_id, self.get_active_sketch(ctx)) else {
            return;
        };
        let placed =
            feature
                .support
                .as_ref()
                .map_or(feature.offset, |s| crate::feature::PlaneShift {
                    offset: s.offset,
                    shift: s.shift,
                    turn: s.turn,
                });
        let (mut offset, mut shift, mut turn) = (placed.offset, placed.shift, placed.turn);
        match key {
            "offset" => offset = value,
            "x" => shift[0] = value,
            "y" => shift[1] = value,
            "turn" => turn = value,
            _ => return,
        }
        let args = crate::commands::args(serde_json::json!({
            "sketch": id.0.to_string(),
            "offset": offset,
            "shift": shift,
            "turn": turn,
        }));
        match crate::commands::run("sketch.attachment", &args, ctx) {
            Ok(_) => ctx.record("sketch.attachment", args, serde_json::Value::Null),
            Err(err) => ctx.log_warn(err.to_string()),
        }
    }

    fn image_number(&mut self, ctx: &mut WorkbenchRuntimeContext, n: usize, key: &str, value: f64) {
        let Some(image) = self.image_at(ctx, n) else {
            return;
        };
        let (mut x, mut y, mut width, mut angle, mut opacity) = (
            image.center.x,
            image.center.y,
            image.width,
            image.angle_deg,
            image.opacity,
        );
        match key {
            "x" => x = value,
            "y" => y = value,
            "width" => width = value,
            "angle" => angle = value,
            "opacity" => opacity = value,
            _ => return,
        }
        self.set_image(
            ctx,
            serde_json::json!({
                "image": image.id.to_string(), "x": x, "y": y, "width": width,
                "angle": angle, "opacity": opacity,
            }),
        );
    }

    /// `sketch.solver_settings` with `args`, run and recorded.
    fn solver_settings(&mut self, ctx: &mut WorkbenchRuntimeContext, mut args: serde_json::Value) {
        let Some(id) = self.active_sketch_id else {
            return;
        };
        args["sketch"] = serde_json::json!(id.0.to_string());
        let args = crate::commands::args(args);
        if crate::commands::run("sketch.solver_settings", &args, ctx).is_ok() {
            ctx.record("sketch.solver_settings", args, serde_json::Value::Null);
            self.last_diagnosis = None;
        }
    }

    fn toggle_event(&mut self, ctx: &mut WorkbenchRuntimeContext, id: &str, on: bool) {
        let p = &mut self.tool_params;
        let o = &mut self.options;
        let flag: Option<&mut bool> = match id {
            "tool.corner_keep" => Some(&mut p.corner_keep),
            "tool.offset_round" => Some(&mut p.offset_round),
            "tool.offset_both" => Some(&mut p.offset_both),
            "tool.offset_delete" => Some(&mut p.offset_delete),
            "tool.offset_linked" => Some(&mut p.offset_linked),
            "tool.mirror_keep" => Some(&mut p.mirror_keep),
            "tool.mirror_linked" => Some(&mut p.mirror_linked),
            "tool.mirror_center" => Some(&mut p.mirror_center),
            "tool.copies_total" => Some(&mut p.copies_total),
            "tool.copies_symmetric" => Some(&mut p.copies_symmetric),
            "tool.rotate_full_turn" => Some(&mut p.rotate_full_turn),
            "tool.copies_linked" | "array.linked" => Some(&mut p.copies_linked),
            "tool.bspline_periodic" => Some(&mut p.bspline_periodic),
            "tool.bspline_interpolate" => Some(&mut p.bspline_interpolate),
            "solver.auto_update" => Some(&mut o.auto_update),
            "solver.auto_remove" => Some(&mut o.auto_remove_redundant),
            "display.grid_on" => Some(&mut o.grid_on),
            "display.grid_auto" => Some(&mut o.grid_auto),
            "display.grid_snap" => Some(&mut o.grid_snap),
            "display.avoid_redundant" => Some(&mut o.avoid_redundant_auto),
            "display.auto_constraints" => Some(&mut o.auto_constraints),
            "display.continue_drawing" => Some(&mut o.continue_drawing),
            "display.continue_constraining" => Some(&mut o.continue_constraining),
            "display.arc_helpers" => Some(&mut o.arc_helpers),
            "display.spline_polygon" => Some(&mut o.spline_polygon),
            "display.spline_degree" => Some(&mut o.spline_degree),
            "display.spline_multiplicity" => Some(&mut o.spline_multiplicity),
            "display.spline_weights" => Some(&mut o.spline_weights),
            _ => None,
        };
        if let Some(flag) = flag {
            *flag = on;
            return;
        }
        match id {
            "display.snap" => self.snap_off = !on,
            "calib.straighten" | "calib.centre" => {
                if let Some(c) = self.calibrating.as_mut() {
                    if id == "calib.straighten" {
                        c.straighten = on;
                    } else {
                        c.centre = on;
                    }
                }
            }
            _ => {
                let Some(i) = panel::indexed(id, "picker.tick:") else {
                    return;
                };
                let (Some(sketch), Some(picker)) = (
                    self.panel.picked_from.get(i).copied(),
                    self.sketch_picker.as_mut(),
                ) else {
                    return;
                };
                if on {
                    picker.checked.insert(sketch);
                } else {
                    picker.checked.remove(&sketch);
                }
                let _ = ctx;
            }
        }
    }

    fn choice_event(&mut self, ctx: &mut WorkbenchRuntimeContext, id: &str, index: usize) {
        match id {
            "display.labels" => {
                if let Some((v, _)) = crate::glyphs::DimensionLabels::ALL.get(index) {
                    self.options.dimension_labels = *v;
                }
            }
            "display.angle_step" => {
                if let Some((v, _)) = crate::ANGLE_STEPS.get(index) {
                    self.options.angle_step_deg = *v;
                }
            }
            "display.render_order" => self.options.construction_on_top = index == 1,
            "solver.tolerance" => {
                if let Some(e) = TOLERANCES.get(index) {
                    self.solver_settings(ctx, serde_json::json!({"tolerance": 10f64.powi(-e)}));
                }
            }
            _ => {
                if let Some(field) = id.strip_prefix("text_draft.") {
                    let mode = &mut self.panel.font_modes[0];
                    text_choice(&mut self.text_draft, mode, field, index);
                } else if let Some(field) = id.strip_prefix("text_block.") {
                    let mut mode = self.panel.font_modes[1];
                    self.text_block_edit(ctx, |spec| text_choice(spec, &mut mode, field, index));
                    self.panel.font_modes[1] = mode;
                }
            }
        }
    }

    fn text_event(&mut self, ctx: &mut WorkbenchRuntimeContext, id: &str, value: &str) {
        if id == "calib.length" {
            if let Some(c) = self.calibrating.as_mut() {
                c.length = value.to_string();
            }
        } else if let Some(field) = id.strip_prefix("text_draft.") {
            text_string(&mut self.text_draft, field, value);
        } else if let Some(field) = id.strip_prefix("text_block.") {
            self.text_block_edit(ctx, |spec| text_string(spec, field, value));
        }
    }

    /// The selected text block made again with `change` applied to what
    /// it says and how, when `change` says it changed.
    fn text_block_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        change: impl FnOnce(&mut crate::text::TextSpec) -> bool,
    ) {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return;
        };
        let Some(block) = self
            .selected
            .iter()
            .find_map(|id| crate::text::block_of(&feature.sketch, *id))
        else {
            return;
        };
        let id = block.id;
        let mut spec = crate::text::TextSpec::of(block);
        if !change(&mut spec) {
            return;
        }
        if let Err(why) = crate::text::load_font(ctx.document, &mut spec) {
            ctx.log_warn(why);
            return;
        }
        let mut args = crate::text_args(&spec, None);
        args["block"] = serde_json::json!(id.to_string());
        self.sketch_edit(
            ctx,
            "sketch.text_edit",
            args,
            |s| match crate::text::change(s, id, &spec) {
                Ok(()) => crate::tools::ToolEffect::changed("Text changed"),
                Err(why) => crate::tools::ToolEffect::log(why),
            },
        );
    }

    /// A button of the plane picker.
    fn attachment_event(&mut self, ctx: &mut WorkbenchRuntimeContext, what: &str) {
        let Some(pending) = &self.pending_creation else {
            return;
        };
        let body = pending.body;
        let made_by = pending.generator.clone();
        let face = pending.face;
        let face_origin = pending.face_origin;
        let mut chosen: Option<Choice> = None;
        match what {
            "cancel" => self.pending_creation = None,
            "face" => {
                let Some(face) = face else {
                    return;
                };
                // On the face itself, not the drawn mesh the click met.
                let face = face.on_its_plane();
                let mut plane = SketchPlane::from_face(widen3(face.point), widen3(face.normal));
                // A generator stands where the face was clicked, a keyway
                // running along the shaft it is clicked on.
                if let Some(made_by) = &made_by {
                    let x_axis = made_by.runs_along(&face).unwrap_or(plane.x_axis);
                    plane = SketchPlane::from_frame(widen3(face.point), plane.normal, x_axis);
                }
                // A face of the sketch's own body, or one it borrows, is
                // followed.
                let follows = body.and_then(|_| {
                    crate::feature::FaceSupport::from_origin(&face, face_origin, plane)
                });
                chosen = Some((plane, None, follows));
            }
            "base:xy" => self.create_on_base_plane(ctx, core_document::BasePlane::XY),
            "base:xz" => self.create_on_base_plane(ctx, core_document::BasePlane::XZ),
            "base:yz" => self.create_on_base_plane(ctx, core_document::BasePlane::YZ),
            _ => {
                if let Some(i) = panel::indexed(what, "datum:") {
                    chosen = self.panel.datum_planes.get(i).cloned();
                } else if let (Some(i), Some(on)) = (panel::indexed(what, "mode:"), body)
                    && let Some(Some(attachment)) = self.panel.modes.get(i).cloned()
                {
                    match crate::commands::settle_attached(ctx, on, attachment, Default::default())
                    {
                        Ok(attached) => {
                            self.pending_creation = None;
                            self.create_sketch_on_plane(
                                ctx,
                                body,
                                attached.plane(),
                                crate::NewSketchOn {
                                    attached: Some(attached),
                                    made_by,
                                    ..Default::default()
                                },
                            );
                        }
                        Err(why) => ctx.log_warn(format!("Cannot attach the sketch there: {why}")),
                    }
                    return;
                }
            }
        }
        if let Some((plane, support, face)) = chosen {
            self.pending_creation = None;
            self.create_sketch_on_plane(
                ctx,
                body,
                plane,
                crate::NewSketchOn {
                    support,
                    face,
                    attached: None,
                    made_by,
                },
            );
        }
    }
}

/// The exponents the solver's tolerance is chosen among: 1e-4 to 1e-12.
const TOLERANCES: [i32; 9] = [4, 5, 6, 7, 8, 9, 10, 11, 12];

/// The list's filter choices: a kind of constraint, or of element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListFilter {
    Constraints(ConstraintFilter),
    Elements(ElementFilter),
}

impl ListFilter {
    const ALL: [(ListFilter, &'static str); 10] = [
        (ListFilter::Constraints(ConstraintFilter::All), "All"),
        (
            ListFilter::Constraints(ConstraintFilter::Selected),
            "Selected",
        ),
        (
            ListFilter::Constraints(ConstraintFilter::Related),
            "Related to selection",
        ),
        (
            ListFilter::Constraints(ConstraintFilter::Geometric),
            "Relations",
        ),
        (
            ListFilter::Constraints(ConstraintFilter::Dimensional),
            "Dimensions",
        ),
        (
            ListFilter::Constraints(ConstraintFilter::Reference),
            "Reference",
        ),
        (ListFilter::Constraints(ConstraintFilter::Named), "Named"),
        (ListFilter::Constraints(ConstraintFilter::Parked), "Parked"),
        (
            ListFilter::Elements(ElementFilter::Normal),
            "Normal geometry",
        ),
        (
            ListFilter::Elements(ElementFilter::Construction),
            "Construction",
        ),
    ];
}

/// The id of constraint `id`'s value field.
fn dimension_field(id: Uuid) -> String {
    format!("dim:{id}")
}

/// A picture's field: its place among the sketch's pictures and its key.
fn image_field(id: &str) -> Option<(usize, &str)> {
    let (n, key) = id.strip_prefix("image:")?.split_once('.')?;
    Some((n.parse().ok()?, key))
}

/// An element's size as its row shows it: a line's length, a round's
/// radius, a point's place; else its kind.
fn element_value(sketch: &Sketch, geom: &sketch::GeometryElement) -> String {
    use sketch::GeometryElement as G;
    let at = |id| sketch.point_position(id);
    match geom {
        G::Point(p) => format!("{:.2}, {:.2}", p.position.x, p.position.y),
        G::Line(l) => match (at(l.start), at(l.end)) {
            (Some(a), Some(b)) => format!("{:.2} mm", (b.x - a.x).hypot(b.y - a.y)),
            _ => element_kind(geom).to_string(),
        },
        G::Arc(a) => format!("R {:.2} mm", a.radius),
        G::Circle(c) => format!("R {:.2} mm", c.radius),
        _ => element_kind(geom).to_string(),
    }
}

/// A dimension's value as plain text and its tone: a driving one as it
/// is set, a reference one measured and in parentheses.
fn dimension_text(sketch: &Sketch, constraint: &Constraint) -> Option<(String, Tone)> {
    let value = sketch::dimension_value(&constraint.kind)?;
    let unit = sketch::dimension_unit(&constraint.kind);
    let (decimals, suffix) = match unit {
        sketch::DimensionUnit::Angle => (1, "°"),
        sketch::DimensionUnit::Length => (2, " mm"),
        sketch::DimensionUnit::Ratio => (3, ""),
    };
    if constraint.driving && constraint.active {
        Some((format!("{value:.decimals$}{suffix}"), Tone::Neutral))
    } else {
        let measured = sketch::measured_value(sketch, &constraint.kind)?;
        Some((
            format!("({measured:.decimals$}{suffix})"),
            measured_tone(constraint),
        ))
    }
}

/// A measured value's tone: the accent while it counts, dim while not.
fn measured_tone(constraint: &Constraint) -> Tone {
    if constraint.active {
        Tone::Accent
    } else {
        Tone::Muted
    }
}

type Cell = (Option<String>, Tone, Option<String>, Option<Box<Widget>>);

/// The value cell of a constraint row in hand: a driving value as a field
/// formulas may set, or the measured reference value in parentheses.
fn dimension_cell(
    sketch: &Sketch,
    constraint: &Constraint,
    sketch_id: Option<FeatureId>,
    focus: bool,
) -> Cell {
    let Some(value) = sketch::dimension_value(&constraint.kind) else {
        return (None, Tone::Neutral, None, None);
    };
    let unit = sketch::dimension_unit(&constraint.kind);
    if constraint.driving && constraint.active {
        let (dim, speed, decimals) = match unit {
            sketch::DimensionUnit::Angle => (Dim::Angle, 1.0, 1),
            sketch::DimensionUnit::Length => (Dim::Length, 0.1, 2),
            sketch::DimensionUnit::Ratio => (Dim::Number, 0.01, 3),
        };
        let field = Quantity {
            id: dimension_field(constraint.id),
            value,
            dim,
            bind: sketch_id.map(|s| Bind {
                feature: s.0.to_string(),
                key: constraint.id.to_string(),
            }),
            decimals,
            speed: Some(speed),
            width: Some(96.0),
            focus,
            ..Default::default()
        };
        return (None, Tone::Neutral, None, Some(Box::new(field.into())));
    }
    let text = match sketch::measured_value(sketch, &constraint.kind) {
        Some(m) if unit == sketch::DimensionUnit::Angle => format!("({m:.1}°)"),
        Some(m) => format!("({m:.2})"),
        None => "(-)".to_string(),
    };
    (
        Some(text),
        measured_tone(constraint),
        Some("Measured value (reference dimension)".into()),
        None,
    )
}

/// What a constraint's colour reads besides the constraint: the
/// document's formulas on the open sketch.
struct Formulas<'a> {
    document: &'a core_document::Document,
    sketch_id: Option<FeatureId>,
}

/// How a constraint's icon reads: a conflict, a redundancy, an inactive
/// one or a reference dimension by its tone; a dimension a formula sets
/// and the rest in the sketch palette's colours, which no tone names.
fn constraint_tint(
    constraint: &Constraint,
    diagnosis: &crate::Diagnosis,
    formulas: &Formulas,
) -> (Tone, Option<[u8; 3]>) {
    let palette = |color: Color32| (Tone::Neutral, Some([color.r(), color.g(), color.b()]));
    if diagnosis.conflicting.contains(&constraint.id) {
        (Tone::Bad, None)
    } else if diagnosis.redundant.contains(&constraint.id) {
        (Tone::Warning, None)
    } else if !constraint.active {
        (Tone::Muted, None)
    } else if constraint.kind.is_dimensional() && !constraint.driving {
        (Tone::Accent, None)
    } else if formulas.sketch_id.is_some_and(|id| {
        formulas
            .document
            .feature_formula(id, &constraint.id.to_string())
            .is_some()
    }) {
        palette(SKETCH_FORMULA)
    } else {
        palette(SKETCH_CONSTRAINT)
    }
}

/// The calibration of `image` under way: what to pick next, then the
/// real length (a value or a formula) and the options.
fn calibration_widgets(
    ctx: &WorkbenchRuntimeContext,
    image: &sketch::ReferenceImage,
    calibrating: &crate::Calibrating,
) -> Vec<Widget> {
    if calibrating.from.is_none() || calibrating.to.is_none() {
        let step = if calibrating.from.is_none() {
            "Click the first end of a length you know on the picture"
        } else {
            "Click the other end"
        };
        return vec![
            panel::caption(step, Tone::Neutral, true),
            panel::button("calib.cancel", "Cancel", ButtonStyle::Secondary, true),
        ];
    }
    let mut widgets = vec![grid(
        "sketch_image_calibration",
        false,
        vec![
            bench_api::Entry {
                id: "calib.length".into(),
                label: "Real length".into(),
                value: calibrating.length.clone(),
                placeholder: "A value, or a formula".into(),
                width: Some(120.0),
                mono: true,
                live: true,
                ..Default::default()
            }
            .into(),
            check(
                "calib.straighten",
                "Turn",
                "Straighten to 45°",
                calibrating.straighten,
                true,
                "Turn the picture so the line lies along the nearest 45° direction",
            ),
            check(
                "calib.centre",
                "Move",
                "Centre on the line",
                calibrating.centre,
                true,
                "Move the picture so the line's middle is the sketch's origin",
            ),
        ],
    )];
    let placed = calibrated(ctx, image, calibrating);
    if let Err(why) = &placed
        && !calibrating.length.trim().is_empty()
    {
        widgets.push(panel::caption(why, Tone::Bad, true));
    }
    widgets.push(panel::flow_no_wrap(vec![
        panel::button(
            "calib.apply",
            "Apply",
            ButtonStyle::Primary,
            placed.is_ok_and(|p| p.is_some()),
        ),
        panel::button("calib.cancel", "Cancel", ButtonStyle::Secondary, true),
    ]));
    widgets
}

/// The picture as a calibration would lay it: the length as it reads (an
/// error when it does not), then the picture when both points are picked.
fn calibrated(
    ctx: &WorkbenchRuntimeContext,
    image: &sketch::ReferenceImage,
    calibrating: &crate::Calibrating,
) -> Result<Option<sketch::ReferenceImage>, String> {
    let length = ctx.document.evaluate_formula(
        calibrating.length.trim(),
        Some(core_document::expr::Dim::LENGTH),
    )?;
    let (Some(from), Some(to)) = (calibrating.from, calibrating.to) else {
        return Ok(None);
    };
    Ok(crate::images::calibrate(
        image,
        &crate::images::Calibration {
            from,
            to,
            length: length.value,
            straighten: calibrating.straighten,
            centre: calibrating.centre,
        },
    ))
}

/// The font chooser's entries past the bundled fonts: an installed font,
/// by family, or a font file.
const INSTALLED: usize = usize::MAX - 1;
const FILE: usize = usize::MAX;

/// Which font entry a text shows: a bundled font (unless a mode was
/// chosen before one is named), an installed family, or a file.
fn font_mode(spec: &crate::text::TextSpec, mode: Option<usize>) -> usize {
    match crate::text::FONTS
        .iter()
        .position(|(name, _)| *name == spec.font)
    {
        Some(i) => mode.filter(|m| *m >= INSTALLED).unwrap_or(i),
        None if crate::text::is_file(&spec.font) => FILE,
        None => INSTALLED,
    }
}

/// A text's string, font, size, spacing and turn as grid fields, ids
/// under `salt`.
fn text_fields(salt: &str, spec: &crate::text::TextSpec, mode: Option<usize>) -> Vec<Widget> {
    let fonts = crate::text::FONTS;
    let which = font_mode(spec, mode);
    let mut options: Vec<String> = fonts.iter().map(|(name, _)| name.to_string()).collect();
    options.push("An installed font".into());
    options.push("A font file".into());
    let selected = match which {
        INSTALLED => fonts.len(),
        FILE => fonts.len() + 1,
        i => i,
    };
    let mut fields = vec![
        bench_api::Entry {
            id: format!("{salt}.text"),
            label: "Text".into(),
            value: spec.text.clone(),
            multiline: true,
            width: Some(160.0),
            ..Default::default()
        }
        .into(),
        Select {
            id: format!("{salt}.font"),
            label: "Font".into(),
            options,
            selected,
            width: Some(160.0),
            ..Default::default()
        }
        .into(),
    ];
    let named = !crate::text::is_bundled(&spec.font);
    if which == INSTALLED {
        let families = crate::text::installed_families();
        fields.push(
            Select {
                id: format!("{salt}.family"),
                label: "Family".into(),
                options: families.to_vec(),
                selected: families
                    .iter()
                    .position(|f| named && f.eq_ignore_ascii_case(&spec.font))
                    .unwrap_or(usize::MAX),
                width: Some(160.0),
                ..Default::default()
            }
            .into(),
        );
    }
    if which == FILE {
        fields.push(
            bench_api::Entry {
                id: format!("{salt}.file"),
                label: "File".into(),
                value: if named {
                    spec.font.clone()
                } else {
                    String::new()
                },
                placeholder: "/path/to/font.ttf".into(),
                width: Some(160.0),
                ..Default::default()
            }
            .into(),
        );
    }
    fields.push(mm(&format!("{salt}.size"), "Size", spec.size));
    fields.push(mm(&format!("{salt}.spacing"), "Spacing", spec.spacing));
    fields.push(degrees(&format!("{salt}.angle"), "Angle", spec.angle));
    fields
}

/// A text's string or font file typed in. Whether it changed.
fn text_string(spec: &mut crate::text::TextSpec, field: &str, value: &str) -> bool {
    match field {
        "text" => spec.text = value.to_string(),
        "file" => {
            spec.font = value.to_string();
            spec.font_asset = None;
        }
        _ => return false,
    }
    true
}

/// A text's size, spacing or turn set. Whether it changed.
fn text_number(spec: &mut crate::text::TextSpec, field: &str, value: f64) -> bool {
    match field {
        "size" => spec.size = value.max(0.1),
        "spacing" => spec.spacing = value,
        "angle" => spec.angle = value,
        _ => return false,
    }
    true
}

/// A text's font or family chosen. A bundled font or a family is set at
/// once; choosing to name an installed font or a file is remembered in
/// `mode` until one is named. Whether the text changed.
fn text_choice(
    spec: &mut crate::text::TextSpec,
    mode: &mut Option<usize>,
    field: &str,
    index: usize,
) -> bool {
    let fonts = crate::text::FONTS;
    match field {
        "font" if index < fonts.len() => {
            spec.font = fonts[index].0.to_string();
            spec.font_asset = None;
            *mode = None;
            true
        }
        "font" => {
            *mode = Some(if index == fonts.len() {
                INSTALLED
            } else {
                FILE
            });
            false
        }
        "family" => match crate::text::installed_families().get(index) {
            Some(name) => {
                spec.font = name.clone();
                spec.font_asset = None;
                true
            }
            None => false,
        },
        _ => false,
    }
}

/// A datum's planes in the plane picker: one for a plane, one per plane
/// of a coordinate system, each named after `prefix`, with its words on
/// hover and the sketch it makes.
fn datum_planes(
    datum_id: FeatureId,
    name: &str,
    datum: &core_document::DatumFeature,
    prefix: &str,
) -> Vec<(String, &'static str, Choice)> {
    let frame = datum.frame();
    let planes: Vec<(Option<String>, core_document::DatumFrame)> = match datum.shape {
        core_document::DatumShape::Plane { .. } => vec![(None, frame)],
        core_document::DatumShape::CoordinateSystem { .. } => frame
            .planes()
            .into_iter()
            .map(|(plane, at)| (Some(plane.to_string()), at))
            .collect(),
        _ => Vec::new(),
    };
    planes
        .into_iter()
        .map(|(plane, at)| {
            let label = match &plane {
                Some(plane) => format!("{prefix}{name} {plane}"),
                None => format!("{prefix}{name}"),
            };
            let hint = if plane.is_some() {
                "Sketch on this plane of the coordinate system"
            } else {
                "Sketch on this datum plane"
            };
            let choice = (
                SketchPlane::from_frame(widen3(at.origin), widen3(at.normal), widen3(at.x_axis)),
                Some(crate::feature::DatumSupport {
                    datum: datum_id,
                    plane,
                    offset: 0.0,
                    shift: [0.0, 0.0],
                    turn: 0.0,
                }),
                None,
            );
            (label, hint, choice)
        })
        .collect()
}

// ------------------------------------------------------------ builders

fn section(
    id: &str,
    title: &str,
    count: Option<usize>,
    summary: Option<String>,
    open: bool,
    children: Vec<Widget>,
) -> Widget {
    PanelSection {
        id: id.into(),
        title: title.into(),
        count,
        summary,
        open,
        children,
        ..Default::default()
    }
    .into()
}

fn grid(id: &str, roomy: bool, children: Vec<Widget>) -> Widget {
    FieldGrid {
        id: id.into(),
        children,
        roomy,
        ..Default::default()
    }
    .into()
}

fn action(id: &str, label: &str, hint: &str, separated: bool) -> PanelAction {
    PanelAction {
        id: id.into(),
        label: label.into(),
        hint: hint.into(),
        separated,
        ..Default::default()
    }
}

fn check(id: &str, label: &str, text: &str, on: bool, enabled: bool, hint: &str) -> Widget {
    Check {
        id: id.into(),
        label: label.into(),
        text: text.into(),
        on,
        enabled,
        hint: hint.into(),
    }
    .into()
}

/// The fillet and chamfer tools' switch that keeps the corner.
fn corner_check(on: bool) -> Widget {
    check(
        "tool.corner_keep",
        "Corner",
        "Keep it",
        on,
        true,
        "The corner stays as a construction point on both curves, with its constraints",
    )
}

/// One of `options`, `width` wide.
fn choose<T: PartialEq>(
    id: &str,
    label: &str,
    options: &[(T, &str)],
    now: T,
    width: f32,
) -> Widget {
    Select {
        id: id.into(),
        label: label.into(),
        options: options.iter().map(|(_, o)| o.to_string()).collect(),
        selected: options
            .iter()
            .position(|(v, _)| *v == now)
            .unwrap_or(usize::MAX),
        width: Some(width),
        ..Default::default()
    }
    .into()
}

/// A whole number from `min` to `max`.
fn integer(id: &str, label: &str, value: u32, min: f64, max: f64) -> Quantity {
    Quantity {
        id: id.into(),
        label: label.into(),
        value: f64::from(value),
        dim: Dim::Number,
        min: Some(min),
        max: Some(max),
        decimals: 0,
        speed: Some(0.1),
        ..Default::default()
    }
}

fn count(id: &str, label: &str, value: u32, min: f64, max: f64) -> Widget {
    integer(id, label, value, min, max).into()
}

/// A length, above zero.
fn mm_quantity(id: &str, label: &str, value: f64) -> Quantity {
    Quantity {
        id: id.into(),
        label: label.into(),
        value,
        dim: Dim::Length,
        min: Some(0.001),
        max: Some(1.0e6),
        ..Default::default()
    }
}

fn mm(id: &str, label: &str, value: f64) -> Widget {
    mm_quantity(id, label, value).into()
}

/// A position or a shift: either way along its axis.
fn offset(id: &str, label: &str, value: f64) -> Widget {
    Quantity {
        min: Some(-1.0e6),
        max: Some(1.0e6),
        ..mm_quantity(id, label, value)
    }
    .into()
}

/// A length in `range`, or any.
fn length(id: &str, label: &str, value: f64, range: Option<(f64, f64)>) -> Widget {
    Quantity {
        min: range.map(|r| r.0),
        max: range.map(|r| r.1),
        ..mm_quantity(id, label, value)
    }
    .into()
}

fn degrees(id: &str, label: &str, value: f64) -> Widget {
    Quantity {
        id: id.into(),
        label: label.into(),
        value,
        dim: Dim::Angle,
        speed: Some(0.5),
        ..Default::default()
    }
    .into()
}

/// A whole number between a down and an up button, each with its state
/// and its words on hover.
fn stepper(
    id: &str,
    label: &str,
    value: usize,
    text: &str,
    down: (bool, &str),
    up: (bool, &str),
) -> Widget {
    Quantity {
        id: id.into(),
        label: label.into(),
        value: value as f64,
        dim: Dim::Number,
        decimals: 0,
        stepper: Some(Stepper {
            text: text.into(),
            down: down.0,
            up: up.0,
            down_hint: down.1.into(),
            up_hint: up.1.into(),
        }),
        ..Default::default()
    }
    .into()
}
