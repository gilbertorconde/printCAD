//! The fillet, chamfer and rectangle card: a corner picked in the view,
//! or a rectangle's last click, opens a card beside it, its sizes typed
//! straight away (a corner's size and Keep corner; a rectangle's width,
//! height and a rounded one's radius or a frame's wall), the view showing
//! the solved result, until OK writes it or Cancel drops it.
//!
//! The draft is the click itself, run again on the sketch as the click
//! found it with the sizes now set (`step::click`, as the recorded
//! `sketch.draw` call replays it: a rectangle's sizes as the click's typed
//! values, which hold it by dimensions, its radius or wall as the tool's
//! setting), so what the view shows, what Apply stores and what a replay
//! makes are one result.

use std::collections::HashSet;

use bench_api::{FloatingCard, PanelEvent, Tone, panel};
use core_document::{HostRequest, WorkbenchRuntimeContext};
use uuid::Uuid;

use crate::feature::SketchFeature;
use crate::operations::SolveOutcome;
use crate::ovp::FieldKind;
use crate::rect::Kind;
use crate::sketch::Vec2D;
use crate::step::{self, StepSettings};
use crate::tools::ToolState;
use crate::{DrawRecord, SketchWorkbench, solver};

/// What a card edits.
enum Subject {
    /// The two curves a corner joins.
    Corner([Uuid; 2]),
    /// A rectangle: its kind and its typed sizes, each field's text and
    /// the value it was set to in the card (none while it follows the
    /// click).
    Rect {
        kind: Kind,
        fields: Vec<(FieldKind, String, Option<f64>)>,
    },
}

/// A corner or rectangle open in its card.
pub(crate) struct CornerTask {
    /// The open sketch as the click found it.
    base: SketchFeature,
    /// The document's edit count when it opened: an edit from elsewhere
    /// (an undo, another view) puts the task away.
    seq: u64,
    tool: String,
    /// What the click is replayed with: the tool's state before it (a
    /// first curve or corner picked), where it landed, its settings and
    /// selection.
    state: ToolState,
    cursor: Vec2D,
    settings: StepSettings,
    selected: HashSet<Uuid>,
    /// The recorded `sketch.draw` call the click belongs to, written on
    /// Apply with the sizes then set.
    record: Option<DrawRecord>,
    subject: Subject,
    /// The tool's size (a corner's, a rounded rectangle's radius, a
    /// frame's wall) as typed in the card: a value, a unit or a formula.
    size_text: String,
    /// The sketch with the click run and solved, or why it cannot be.
    working: Result<SketchFeature, String>,
}

impl CornerTask {
    fn bevel(&self) -> bool {
        self.tool == "sketch.chamfer"
    }

    fn rect(&self) -> Option<Kind> {
        match self.subject {
            Subject::Rect { kind, .. } => Some(kind),
            Subject::Corner(_) => None,
        }
    }

    /// The tool setting the card's size field edits, if it has one.
    fn size_param(&self) -> Option<SizeParam> {
        Some(match self.rect() {
            None if self.bevel() => SizeParam::Chamfer,
            None | Some(Kind::Rounded) => SizeParam::Fillet,
            Some(Kind::Frame) => SizeParam::Wall,
            Some(_) => return None,
        })
    }

    fn size(&self) -> f64 {
        self.size_param()
            .map_or(0.0, |p| p.get(&self.settings.params))
    }

    /// The card's typed sizes as the click takes them.
    fn typed(&self) -> Vec<(FieldKind, f64)> {
        match &self.subject {
            Subject::Rect { fields, .. } => fields
                .iter()
                .filter_map(|(kind, _, value)| Some((*kind, (*value)?)))
                .collect(),
            Subject::Corner(_) => Vec::new(),
        }
    }

    /// Run the click on the base sketch at the sizes set, and solve it
    /// when the sketch solves as it is edited.
    fn calculate(&self, auto_update: bool) -> Result<SketchFeature, String> {
        if let Some(param) = self.size_param() {
            let size = self.size();
            if !(size.is_finite() && size >= 1e-6) {
                return Err(param.refusal().into());
            }
        }
        if let Subject::Rect { fields, .. } = &self.subject
            && let Some((kind, ..)) = fields
                .iter()
                .find(|(_, _, v)| v.is_some_and(|v| !(v.is_finite() && v >= 1e-6)))
        {
            return Err(format!(
                "Enter a {} greater than zero",
                if *kind == FieldKind::Height {
                    "height"
                } else {
                    "width"
                }
            ));
        }
        let mut feature = self.base.clone();
        let mut state = self.state.clone();
        let mut capture = crate::ovp::DimCapture::default();
        let typed = self.typed();
        let outcome = step::click(
            &mut state,
            &mut capture,
            &self.tool,
            &mut feature.sketch,
            self.cursor,
            &typed,
            !typed.is_empty(),
            &self.settings,
            &self.selected,
        );
        if !outcome.changed {
            return Err(outcome
                .log
                .filter(|log| !log.is_empty())
                .unwrap_or_else(|| match self.subject {
                    Subject::Corner(_) => "These curves cannot be joined at this size".into(),
                    Subject::Rect { .. } => "No rectangle fits these sizes".into(),
                }));
        }
        if auto_update
            && matches!(
                solver::solve(&mut feature.sketch),
                SolveOutcome::NotConverged { .. }
            )
        {
            return Err(match self.subject {
                Subject::Corner(_) => "The constraints cannot be solved with this corner".into(),
                Subject::Rect { .. } => {
                    "The constraints cannot be solved with this rectangle".into()
                }
            });
        }
        Ok(feature)
    }
}

/// The tool setting a card's size field stands for.
#[derive(Clone, Copy)]
enum SizeParam {
    Fillet,
    Chamfer,
    Wall,
}

impl SizeParam {
    fn get(self, p: &crate::tools::ToolParams) -> f64 {
        match self {
            SizeParam::Fillet => p.fillet_radius,
            SizeParam::Chamfer => p.chamfer_length,
            SizeParam::Wall => p.offset_distance,
        }
    }

    fn set(self, p: &mut crate::tools::ToolParams, value: f64) {
        match self {
            SizeParam::Fillet => p.fillet_radius = value,
            SizeParam::Chamfer => p.chamfer_length = value,
            SizeParam::Wall => p.offset_distance = value,
        }
    }

    fn label(self) -> &'static str {
        match self {
            SizeParam::Fillet => "Radius",
            SizeParam::Chamfer => "Length",
            SizeParam::Wall => "Wall",
        }
    }

    fn refusal(self) -> &'static str {
        match self {
            SizeParam::Fillet => "Enter a radius greater than zero",
            SizeParam::Chamfer => "Enter a length greater than zero",
            SizeParam::Wall => "Enter a wall greater than zero",
        }
    }
}

/// The card's id and field ids for a corner and for a rectangle.
const CORNER_CARD: &str = "sketch.corner";
const RECT_CARD: &str = "sketch.rect";

fn field_id(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Height => "rect.height",
        _ => "rect.width",
    }
}

impl SketchWorkbench {
    /// A fillet or chamfer click that resolves a corner opens its task
    /// rather than cutting at once. `None` when the click only picks a
    /// first curve or nothing, which the tool's own click handles.
    pub(crate) fn open_corner_task(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        tool: &str,
        cursor: Vec2D,
        settings: &StepSettings,
    ) -> Option<()> {
        if !matches!(tool, "sketch.fillet" | "sketch.chamfer") {
            return None;
        }
        let curves = crate::tools::modify::corner_operands(
            &self.tool_state,
            &feature.sketch,
            cursor,
            settings.tol,
        )?;
        self.open_card(
            ctx,
            feature,
            tool,
            cursor,
            settings,
            Subject::Corner(curves),
        );
        Some(())
    }

    /// A plain click that finishes a rectangle opens its card rather than
    /// storing it at once. `None` when the click only picks a corner, is
    /// refused (which the tool's own click says), or comes after typed
    /// values, which already say the sizes.
    pub(crate) fn open_rect_task(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        tool: &str,
        cursor: Vec2D,
        settings: &StepSettings,
    ) -> Option<()> {
        let kind = Kind::of(tool)?;
        if self.dim_capture.has_pending() {
            return None;
        }
        let sketch = &feature.sketch;
        let (at, tol) = step::land(&self.tool_state, sketch, tool, cursor, &[], settings);
        let proposal =
            crate::rect::propose(&self.tool_state, sketch, at, tol, kind, &settings.params).ok()?;
        let (first, second) = proposal.size();
        let fields = crate::ovp::fields_for(&self.tool_state, Some(tool))
            .iter()
            .map(|&field| {
                let value = match (field, &self.tool_state) {
                    (FieldKind::Height, _) | (_, ToolState::Rect3B { .. }) => second,
                    _ => first,
                };
                (field, crate::glyphs::fmt_num(value), None)
            })
            .collect();
        self.open_card(
            ctx,
            feature,
            tool,
            cursor,
            settings,
            Subject::Rect { kind, fields },
        );
        Some(())
    }

    fn open_card(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        tool: &str,
        cursor: Vec2D,
        settings: &StepSettings,
        subject: Subject,
    ) {
        let mut task = CornerTask {
            base: feature.clone(),
            seq: ctx.document.mutation_seq(),
            tool: tool.to_string(),
            state: std::mem::take(&mut self.tool_state),
            cursor,
            settings: *settings,
            selected: self.selected.iter().copied().collect(),
            record: self.draw_record.take(),
            subject,
            size_text: String::new(),
            working: Err(String::new()),
        };
        task.size_text = crate::glyphs::fmt_num(task.size());
        task.working = task.calculate(self.options.auto_update);
        self.corner_task = Some(task);
        self.dim_capture.clear_buffers();
        self.last_diagnosis = None;
    }

    /// Whether a rectangle waits in its card.
    pub fn rect_card_open(&self) -> bool {
        self.corner_task
            .as_ref()
            .is_some_and(|t| t.rect().is_some())
    }

    /// The sketch the view shows: the open card's solved result, else
    /// the stored sketch. Writes nothing.
    pub(crate) fn displayed_sketch(&self, ctx: &WorkbenchRuntimeContext) -> Option<SketchFeature> {
        self.corner_task
            .as_ref()
            .filter(|task| task.seq == ctx.document.mutation_seq())
            .and_then(|task| task.working.as_ref().ok())
            .cloned()
            .or_else(|| self.dimension_preview(ctx))
            .or_else(|| self.get_active_sketch(ctx))
    }

    /// Put an open card away, writing and recording nothing.
    pub(crate) fn discard_corner(&mut self) {
        if self.corner_task.take().is_some() {
            self.last_diagnosis = None;
        }
    }

    /// Put the card away when the document changed under it. Answers
    /// whether it did.
    pub(crate) fn drop_stale_corner(&mut self, ctx: &WorkbenchRuntimeContext) -> bool {
        let stale = self
            .corner_task
            .as_ref()
            .is_some_and(|task| task.seq != ctx.document.mutation_seq());
        if stale {
            self.discard_corner();
        }
        stale
    }

    /// Store the card's result as one undo step, recorded as the
    /// `sketch.draw` call that makes it. Nothing happens while the result
    /// is refused.
    pub(crate) fn apply_corner(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if self.drop_stale_corner(ctx) {
            return;
        }
        let Some(mut task) = self.corner_task.take() else {
            return;
        };
        let Ok(feature) = task.working.clone() else {
            self.corner_task = Some(task);
            return;
        };
        self.selected
            .retain(|id| feature.sketch.get_geometry(*id).is_some());
        self.unsolved_edits = !self.options.auto_update && !solver::holds(&feature.sketch);
        self.last_diagnosis = None;
        if !self.store_sketch(ctx, feature) {
            return;
        }
        let label = match task.rect() {
            Some(_) => idle_name(&task.tool),
            None if task.bevel() => "Chamfer",
            None => "Fillet",
        };
        let typed = task.typed();
        if let Some(mut record) = task.record.take() {
            let params = step::params_to_json(&task.settings.params);
            if params.is_empty() {
                record.settings.remove("params");
            } else {
                record.settings.insert("params".into(), params.into());
            }
            // Sizes set in the card are the click's typed values, held by
            // dimensions as Enter holds them.
            if !typed.is_empty()
                && let Some(event) = record.events.last_mut()
            {
                let values: serde_json::Map<String, serde_json::Value> = typed
                    .iter()
                    .map(|(k, v)| (step::field_name(*k).to_string(), serde_json::json!(v)))
                    .collect();
                let (x, y) = (task.cursor.x, task.cursor.y);
                *event = serde_json::json!({"x": x, "y": y, "typed": values, "constrain": true});
            }
            record.changed = true;
            self.draw_record = Some(record);
            self.flush_draw_record(ctx);
        }
        match task.size_param() {
            Some(_) => ctx.log_info(format!("{label} {:.2}", task.size())),
            None => ctx.log_info(label),
        }
        // The host closes the card's undo step once OK is handled.
        ctx.request(HostRequest::JournalLabel(label.into()));
        // With Continue drawing off, a finished rectangle hands the
        // pointer back to the Select tool, as a stored one does.
        if task.rect().is_some() && !self.options.continue_drawing {
            ctx.request(HostRequest::ActivateTool("sketch.select".to_string()));
        }
    }

    /// The card: what it edits and its sizes (the first with the keys as
    /// it opens), by the click; OK held back with the reason while the
    /// result is refused.
    pub(crate) fn corner_card(&self, ctx: &WorkbenchRuntimeContext) -> Option<FloatingCard> {
        let task = self.corner_task.as_ref()?;
        let size = task.size_param().map(|param| {
            let id = if task.rect().is_some() {
                "rect.size"
            } else {
                "corner.size"
            };
            let mut size = panel::entry_with(id, param.label(), &task.size_text, "A size");
            size.live = true;
            size.mono = true;
            size.formulas = true;
            size
        });
        let mut blocks: Vec<bench_api::Widget> = Vec::new();
        let (id, title, icon, focus) = match &task.subject {
            Subject::Corner(curves) => {
                let names = curves.map(|id| crate::style::element_name(&task.base.sketch, id));
                blocks.push(panel::caption(
                    &format!("{} + {}", names[0], names[1]),
                    Tone::Neutral,
                    true,
                ));
                blocks.extend(size.map(Into::into));
                let mut keep = panel::check(
                    "corner.keep",
                    "Corner",
                    "Keep it",
                    task.settings.params.corner_keep,
                );
                keep.hint = "The corner stays as a construction point on both curves, with its \
                             constraints"
                    .into();
                blocks.push(keep.into());
                let (title, icon) = if task.bevel() {
                    ("Chamfer", "sketch-chamfer")
                } else {
                    ("Fillet", "sketch-fillet")
                };
                (CORNER_CARD, title, icon, "corner.size")
            }
            Subject::Rect { fields, .. } => {
                for (kind, text, _) in fields {
                    let label = if *kind == FieldKind::Height {
                        "Height"
                    } else {
                        "Width"
                    };
                    let mut entry = panel::entry_with(field_id(*kind), label, text, "A size");
                    entry.live = true;
                    entry.mono = true;
                    entry.formulas = true;
                    blocks.push(entry.into());
                }
                blocks.extend(size.map(Into::into));
                (
                    RECT_CARD,
                    idle_name(&task.tool),
                    crate::tool_icon(&task.tool),
                    "rect.width",
                )
            }
        };
        let stale = task.seq != ctx.document.mutation_seq();
        let refusal = match (&task.working, stale) {
            (_, true) => Some("The sketch changed: draw it again".to_string()),
            (Err(why), _) => Some(why.clone()),
            (Ok(_), _) => None,
        };
        let plane = &task.base.plane;
        let at = std::array::from_fn(|i| {
            (plane.origin[i] + plane.x_axis[i] * task.cursor.x + plane.y_axis[i] * task.cursor.y)
                as f32
        });
        let mut card = panel::floating_card(id, title, at, Some(focus), blocks);
        card.icon = Some(icon.into());
        card.refusal = refusal;
        Some(card)
    }

    /// A change on the card. Answers whether it was one. The tool's size
    /// and switch carry on to its next corner or rectangle.
    pub(crate) fn corner_event(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        event: &PanelEvent,
    ) -> bool {
        let Some(task) = self.corner_task.as_mut() else {
            return false;
        };
        let length = |value: &str| {
            ctx.document
                .evaluate_formula(value, Some(core_document::expr::Dim::LENGTH))
                .map(|q| q.value)
                .map_err(|why| {
                    if value.trim().is_empty() {
                        "Type a size".to_string()
                    } else {
                        why
                    }
                })
        };
        match event {
            PanelEvent::CardClose { id, accept } if id == CORNER_CARD || id == RECT_CARD => {
                if *accept {
                    self.apply_corner(ctx);
                } else {
                    self.discard_corner();
                }
                return true;
            }
            PanelEvent::Text { id, value } if id == "corner.size" || id == "rect.size" => {
                task.size_text.clone_from(value);
                match (length(value), task.size_param()) {
                    (Ok(v), Some(param)) => param.set(&mut self.tool_params, v),
                    (Err(why), _) => {
                        task.working = Err(why);
                        return true;
                    }
                    (Ok(_), None) => return true,
                }
            }
            PanelEvent::Text { id, value } if id == "rect.width" || id == "rect.height" => {
                let Subject::Rect { fields, .. } = &mut task.subject else {
                    return true;
                };
                let parsed = length(value);
                for (kind, text, typed) in fields.iter_mut() {
                    if field_id(*kind) == id {
                        text.clone_from(value);
                        *typed = parsed.as_ref().ok().copied();
                    }
                }
                if let Err(why) = parsed {
                    task.working = Err(why);
                    return true;
                }
            }
            PanelEvent::Toggle { id, on } if id == "corner.keep" => {
                self.tool_params.corner_keep = *on;
            }
            _ if event.id().starts_with("corner.") || event.id().starts_with("rect.") => {
                return true;
            }
            _ => return false,
        }
        self.recalculate_corner();
        true
    }

    /// Work the card's click out again at the tool's sizes.
    fn recalculate_corner(&mut self) {
        let auto_update = self.options.auto_update;
        let p = self.tool_params;
        if let Some(task) = self.corner_task.as_mut() {
            let params = &mut task.settings.params;
            params.fillet_radius = p.fillet_radius;
            params.chamfer_length = p.chamfer_length;
            params.corner_keep = p.corner_keep;
            params.offset_distance = p.offset_distance;
            task.working = task.calculate(auto_update);
            self.last_diagnosis = None;
        }
    }
}

/// A rectangle tool's name, as its card and undo step say it.
fn idle_name(tool: &str) -> &'static str {
    match tool {
        "sketch.rect_rounded" => "Rounded rectangle",
        "sketch.rect_frame" => "Frame",
        _ => "Rectangle",
    }
}
