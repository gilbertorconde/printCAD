//! The card for what a click leaves to set: a corner picked by the fillet
//! or chamfer tool, or the last click of a rounded rectangle or a frame,
//! opens a card beside it with that size (a corner's size and Keep corner,
//! the rounded corners' radius, the frame's wall), the tool's own setting
//! in it and the keys on it, the view showing the solved result. OK or
//! Enter writes it, and so does anything else that puts it away (another
//! tool, the sketch finished, a click in the view): left as it opened, the
//! setting is what it writes. A tool whose clicks set everything, a plain
//! rectangle among them, opens no card.
//!
//! The draft is the click itself, run again on the sketch as the click
//! found it with the size now set (`step::click`, as the recorded
//! `sketch.draw` call replays it: sizes typed before the click as its
//! typed values, the radius or wall as the tool's setting), so what the
//! view shows, what OK stores and what a replay makes are one result.

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
    /// A rectangle with more to set than its clicks: its kind, and the
    /// sizes typed before its last click with whether Enter held them by
    /// dimensions.
    Rect {
        kind: Kind,
        typed: Vec<(FieldKind, f64)>,
        constrain: bool,
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

    /// The sizes typed before the click, and whether they hold it.
    fn typed(&self) -> (&[(FieldKind, f64)], bool) {
        match &self.subject {
            Subject::Rect {
                typed, constrain, ..
            } => (typed, *constrain),
            Subject::Corner(_) => (&[], false),
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
        let mut feature = self.base.clone();
        let mut state = self.state.clone();
        let mut capture = crate::ovp::DimCapture::default();
        let (typed, constrain) = self.typed();
        let outcome = step::click(
            &mut state,
            &mut capture,
            &self.tool,
            &mut feature.sketch,
            self.cursor,
            typed,
            constrain,
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
            &self.tab.tool_state,
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

    /// The last click of a rectangle with more to set than its clicks (a
    /// rounded one's radius, a frame's wall) opens its card rather than
    /// storing it at once, holding the sizes typed before it. `None` for
    /// a rectangle its clicks set whole, a click that only picks a corner,
    /// or one refused (which the tool's own click says).
    #[allow(clippy::too_many_arguments, reason = "the click as the tool takes it")]
    pub(crate) fn open_rect_task(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        tool: &str,
        cursor: Vec2D,
        settings: &StepSettings,
        typed: &[(FieldKind, f64)],
        constrain: bool,
    ) -> Option<()> {
        let kind = Kind::of(tool)?;
        if !matches!(kind, Kind::Rounded | Kind::Frame) || self.tab.dim_capture.has_pending() {
            return None;
        }
        let sketch = &feature.sketch;
        let (at, tol) = step::land(&self.tab.tool_state, sketch, tool, cursor, typed, settings);
        crate::rect::propose(
            &self.tab.tool_state,
            sketch,
            at,
            tol,
            kind,
            &settings.params,
        )
        .ok()?;
        self.open_card(
            ctx,
            feature,
            tool,
            cursor,
            settings,
            Subject::Rect {
                kind,
                typed: typed.to_vec(),
                constrain,
            },
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
            state: std::mem::take(&mut self.tab.tool_state),
            cursor,
            settings: *settings,
            selected: self.tab.selected.iter().copied().collect(),
            record: self.tab.draw_record.take(),
            subject,
            size_text: String::new(),
            working: Err(String::new()),
        };
        task.size_text = crate::glyphs::fmt_num(task.size());
        task.working = task.calculate(self.options.auto_update);
        self.tab.corner_task = Some(task);
        self.tab.dim_capture.clear_buffers();
        self.tab.last_diagnosis = None;
    }

    /// Whether a rectangle waits in its card.
    pub fn rect_card_open(&self) -> bool {
        self.tab
            .corner_task
            .as_ref()
            .is_some_and(|t| t.rect().is_some())
    }

    /// The sketch as an open card's draft makes it, a corner's or a
    /// dimension's solved result, when one is open. Writes nothing.
    pub(crate) fn drafted_sketch(&self, ctx: &WorkbenchRuntimeContext) -> Option<SketchFeature> {
        self.tab
            .corner_task
            .as_ref()
            .filter(|task| task.seq == ctx.document.mutation_seq())
            .and_then(|task| task.working.as_ref().ok())
            .cloned()
            .or_else(|| self.dimension_preview(ctx))
    }

    /// Put an open card away, writing and recording nothing.
    pub(crate) fn discard_corner(&mut self) {
        if self.tab.corner_task.take().is_some() {
            self.tab.last_diagnosis = None;
        }
    }

    /// Put the card away when the document changed under it. Answers
    /// whether it did.
    pub(crate) fn drop_stale_corner(&mut self, ctx: &WorkbenchRuntimeContext) -> bool {
        let stale = self
            .tab
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
        self.store_corner(ctx, true);
    }

    /// [`apply_corner`](Self::apply_corner); with `hand_back`, a finished
    /// rectangle gives the pointer back to Select when Continue drawing
    /// is off, which a card taken because another tool was picked leaves
    /// to that tool.
    pub(crate) fn store_corner(&mut self, ctx: &mut WorkbenchRuntimeContext, hand_back: bool) {
        if self.drop_stale_corner(ctx) {
            return;
        }
        let Some(mut task) = self.tab.corner_task.take() else {
            return;
        };
        let Ok(feature) = task.working.clone() else {
            self.tab.corner_task = Some(task);
            return;
        };
        self.tab
            .selected
            .retain(|id| feature.sketch.get_geometry(*id).is_some());
        self.tab.unsolved_edits = !self.options.auto_update && !solver::holds(&feature.sketch);
        self.tab.last_diagnosis = None;
        if !self.store_sketch(ctx, feature) {
            return;
        }
        let label = match task.rect() {
            Some(_) => idle_name(&task.tool),
            None if task.bevel() => "Chamfer",
            None => "Fillet",
        };
        if let Some(mut record) = task.record.take() {
            let params = step::params_to_json(&task.settings.params);
            if params.is_empty() {
                record.settings.remove("params");
            } else {
                record.settings.insert("params".into(), params.into());
            }
            record.changed = true;
            self.tab.draw_record = Some(record);
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
        if hand_back && task.rect().is_some() && !self.options.continue_drawing {
            ctx.request(HostRequest::ActivateTool("sketch.select".to_string()));
        }
    }

    /// The card: what it edits and its sizes (the first with the keys as
    /// it opens), by the click; OK held back with the reason while the
    /// result is refused.
    pub(crate) fn corner_card(&self, ctx: &WorkbenchRuntimeContext) -> Option<FloatingCard> {
        let task = self.tab.corner_task.as_ref()?;
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
            Subject::Rect { .. } => {
                blocks.extend(size.map(Into::into));
                (
                    RECT_CARD,
                    idle_name(&task.tool),
                    crate::registration::tool_icon(&task.tool),
                    "rect.size",
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
        card.ok_only = true;
        Some(card)
    }

    /// A change on the card. Answers whether it was one. The tool's size
    /// and switch carry on to its next corner or rectangle.
    pub(crate) fn corner_event(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        event: &PanelEvent,
    ) -> bool {
        let Some(task) = self.tab.corner_task.as_mut() else {
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
                    (Ok(v), Some(param)) => param.set(&mut self.tab.tool_params, v),
                    (Err(why), _) => {
                        task.working = Err(why);
                        return true;
                    }
                    (Ok(_), None) => return true,
                }
            }
            PanelEvent::Toggle { id, on } if id == "corner.keep" => {
                self.tab.tool_params.corner_keep = *on;
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
        let p = self.tab.tool_params;
        if let Some(task) = self.tab.corner_task.as_mut() {
            let params = &mut task.settings.params;
            params.fillet_radius = p.fillet_radius;
            params.chamfer_length = p.chamfer_length;
            params.corner_keep = p.corner_keep;
            params.offset_distance = p.offset_distance;
            task.working = task.calculate(auto_update);
            self.tab.last_diagnosis = None;
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
