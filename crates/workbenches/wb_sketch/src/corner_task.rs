//! What a click leaves to set: a corner picked by the fillet or chamfer
//! tool, the last click of a rounded rectangle or a frame, the offset's
//! click on its side, or the text tool's click, waits in the task panel's
//! Tool section, whose fields (the tool's own settings) edit it live, the
//! view showing the solved result. Only the section's Cancel drops it:
//! its OK, Enter, Escape, a click in the view, another tool or the sketch
//! finished each write it as it stands. One click, one result: the next
//! starts when the tool is used again. A tool whose clicks set
//! everything, a plain rectangle among them, leaves nothing waiting.
//!
//! The draft is the click itself, run again on the sketch as the click
//! found it with the settings now set (`step::click`, as the recorded
//! `sketch.draw` call replays it: sizes typed before the click as its
//! typed values, the radius or wall as the tool's setting; the text
//! written as `sketch.text` writes it), so what the view shows, what OK
//! stores and what a replay makes are one result.

use std::collections::HashSet;

#[cfg(feature = "egui")]
use bench_api::{ButtonStyle, NoteKind, Tone, Widget, panel};
use core_document::{HostRequest, WorkbenchRuntimeContext};
use uuid::Uuid;

use crate::feature::SketchFeature;
use crate::operations::SolveOutcome;
use crate::ovp::FieldKind;
use crate::rect::Kind;
use crate::sketch::Vec2D;
use crate::step::{self, StepSettings};
use crate::text::{self, TextSpec};
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
    /// The selected curves copied at a distance on the click's side.
    Offset,
    /// Text where the click put it, written as the tool's text settings
    /// say, its font read without keeping it until it is written.
    Text { at: TextAt, spec: Box<TextSpec> },
}

/// Where the text tool's click puts its text.
#[derive(Clone, Copy)]
pub(crate) enum TextAt {
    /// From a point.
    Point(Vec2D),
    /// In a frame across two corners.
    Frame(Vec2D, Vec2D),
    /// Along a curve, from a point on it.
    Path(Uuid, Vec2D),
}

impl TextAt {
    /// Write `spec` into `sketch` here. Returns the block's id.
    pub(crate) fn add(
        self,
        sketch: &mut crate::sketch::Sketch,
        spec: &TextSpec,
    ) -> Result<Uuid, String> {
        match self {
            TextAt::Point(at) => text::add(sketch, at, spec),
            TextAt::Frame(a, b) => text::add_in_frame(sketch, a, b, spec),
            TextAt::Path(curve, at) => text::add_on_path(sketch, &[curve], Some(at), spec),
        }
    }
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
            Subject::Corner(_) | Subject::Offset | Subject::Text { .. } => None,
        }
    }

    /// The tool setting the card's size field edits, if it has one.
    fn size_param(&self) -> Option<SizeParam> {
        Some(match self.rect() {
            None if matches!(self.subject, Subject::Text { .. }) => return None,
            None if matches!(self.subject, Subject::Offset) => SizeParam::Offset,
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
            Subject::Corner(_) | Subject::Offset | Subject::Text { .. } => (&[], false),
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
        if let Subject::Text { at, spec } = &self.subject {
            let mut feature = self.base.clone();
            at.add(&mut feature.sketch, spec)?;
            if auto_update
                && matches!(
                    solver::solve(&mut feature.sketch),
                    SolveOutcome::NotConverged { .. }
                )
            {
                return Err("The constraints cannot be solved with this text".into());
            }
            return Ok(feature);
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
                    Subject::Offset => "These curves cannot be offset by this distance".into(),
                    Subject::Text { .. } => "The text cannot be written here".into(),
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
                Subject::Offset => "The constraints cannot be solved with this offset".into(),
                Subject::Text { .. } => unreachable!("worked out above"),
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
    Offset,
}

impl SizeParam {
    fn get(self, p: &crate::tools::ToolParams) -> f64 {
        match self {
            SizeParam::Fillet => p.fillet_radius,
            SizeParam::Chamfer => p.chamfer_length,
            SizeParam::Wall | SizeParam::Offset => p.offset_distance,
        }
    }

    fn refusal(self) -> &'static str {
        match self {
            SizeParam::Fillet => "Enter a radius greater than zero",
            SizeParam::Chamfer => "Enter a length greater than zero",
            SizeParam::Wall => "Enter a wall greater than zero",
            SizeParam::Offset => "Enter a distance greater than zero",
        }
    }
}

/// The panel's buttons for what waits.
pub(crate) const PENDING_OK: &str = "pending.ok";
pub(crate) const PENDING_CANCEL: &str = "pending.cancel";

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
        self.open_pending(
            ctx,
            feature,
            tool,
            cursor,
            settings,
            Subject::Corner(curves),
        );
        Some(())
    }

    /// The offset's click opens its card rather than copying at once.
    /// `None` when the click would copy nothing (no curve selected that it
    /// takes), which the tool's own click says.
    pub(crate) fn open_offset_task(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        tool: &str,
        cursor: Vec2D,
        settings: &StepSettings,
    ) -> Option<()> {
        if tool != "sketch.offset" {
            return None;
        }
        let mut trial = feature.sketch.clone();
        let mut state = self.tab.tool_state.clone();
        let selected: HashSet<Uuid> = self.tab.selected.iter().copied().collect();
        let outcome = step::click(
            &mut state,
            &mut crate::ovp::DimCapture::default(),
            tool,
            &mut trial,
            cursor,
            &[],
            false,
            settings,
            &selected,
        );
        if !outcome.changed {
            return None;
        }
        self.open_pending(ctx, feature, tool, cursor, settings, Subject::Offset);
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
        let (at, tol, _) = step::land(&self.tab.tool_state, sketch, tool, cursor, typed, settings);
        crate::rect::propose(
            &self.tab.tool_state,
            sketch,
            at,
            tol,
            kind,
            &settings.params,
        )
        .ok()?;
        self.open_pending(
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

    fn open_pending(
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
            working: Err(String::new()),
        };
        task.working = task.calculate(self.options.auto_update);
        self.tab.corner_task = Some(task);
        self.tab.dim_capture.clear_buffers();
        self.tab.last_diagnosis = None;
    }

    /// The text tool's click: the text waits at `at`, written as the
    /// tool's text settings say until it is accepted.
    pub(crate) fn open_text_task(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        at: TextAt,
        settings: &StepSettings,
    ) {
        let cursor = match at {
            TextAt::Point(p) | TextAt::Frame(p, _) | TextAt::Path(_, p) => p,
        };
        let spec = self.tab.text_draft.clone();
        self.open_pending(
            ctx,
            feature,
            "sketch.text",
            cursor,
            settings,
            Subject::Text {
                at,
                spec: Box::new(spec),
            },
        );
        self.recalculate_corner(ctx);
    }

    /// Whether a shape the tool drew (a rectangle, an offset, a text)
    /// waits in the panel, rather than a picked corner.
    pub fn shape_card_open(&self) -> bool {
        self.tab
            .corner_task
            .as_ref()
            .is_some_and(|t| !matches!(t.subject, Subject::Corner(_)))
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
        if let Subject::Text { at, .. } = task.subject {
            let spec = self.tab.text_draft.clone();
            self.place_text(ctx, at, spec);
            ctx.request(HostRequest::JournalLabel("Text".into()));
            if hand_back && !self.options.continue_drawing {
                ctx.request(HostRequest::ActivateTool("sketch.select".to_string()));
            }
            return;
        }
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
            None if matches!(task.subject, Subject::Offset) => "Offset",
            None if matches!(task.subject, Subject::Text { .. }) => "Text",
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
        // The host closes its undo step once OK is handled.
        ctx.request(HostRequest::JournalLabel(label.into()));
        // With Continue drawing off, the pointer goes back to the Select
        // tool, as a stored shape hands it back.
        if hand_back && !self.options.continue_drawing {
            ctx.request(HostRequest::ActivateTool("sketch.select".to_string()));
        }
    }

    #[cfg(feature = "egui")]
    /// The Tool section's close for what waits: what it is, why it
    /// cannot be written yet, and its OK and Cancel. Empty with nothing
    /// waiting.
    pub(crate) fn pending_widgets(&self, ctx: &WorkbenchRuntimeContext) -> Vec<Widget> {
        let Some(task) = self.tab.corner_task.as_ref() else {
            return Vec::new();
        };
        let mut widgets = Vec::new();
        if let Subject::Corner(curves) = &task.subject {
            let names = curves.map(|id| crate::style::element_name(&task.base.sketch, id));
            widgets.push(panel::caption(
                &format!("{} + {}", names[0], names[1]),
                Tone::Neutral,
                true,
            ));
        }
        let stale = task.seq != ctx.document.mutation_seq();
        let refusal = match (&task.working, stale) {
            (_, true) => Some("The sketch changed: draw it again".to_string()),
            (Err(why), _) if !why.is_empty() => Some(why.clone()),
            (Err(_), _) => Some(String::new()),
            (Ok(_), _) => None,
        };
        if let Some(why) = refusal.as_ref().filter(|why| !why.is_empty()) {
            widgets.push(panel::note(NoteKind::Warning, None, why));
        }
        widgets.push(panel::flow([
            panel::hinted(
                "Write it as it stands; so do Enter, Escape, a click in the view and another tool",
                panel::button(PENDING_OK, "OK", ButtonStyle::Primary, refusal.is_none()),
            ),
            panel::hinted(
                "Drop it: the sketch stays as it was before the click",
                panel::button(PENDING_CANCEL, "Cancel", ButtonStyle::Secondary, true),
            ),
        ]));
        widgets
    }

    /// Whether a click's result waits in the Tool section: `Ok` when its
    /// OK would write it, else why not.
    pub fn waiting(&self, ctx: &WorkbenchRuntimeContext) -> Option<Result<(), String>> {
        let task = self.tab.corner_task.as_ref()?;
        Some(if task.seq != ctx.document.mutation_seq() {
            Err("The sketch changed: draw it again".into())
        } else {
            task.working.as_ref().map(|_| ()).map_err(Clone::clone)
        })
    }

    #[cfg(feature = "egui")]
    /// The Tool section's OK or Cancel. Answers whether `id` was one.
    pub(crate) fn pending_button(&mut self, ctx: &mut WorkbenchRuntimeContext, id: &str) -> bool {
        match id {
            PENDING_OK => self.apply_corner(ctx),
            PENDING_CANCEL => self.discard_corner(),
            _ => return false,
        }
        true
    }

    /// Work what waits out again at the tool's settings, after one of
    /// them changed in the panel.
    pub(crate) fn recalculate_corner(&mut self, ctx: &WorkbenchRuntimeContext) {
        let auto_update = self.options.auto_update;
        let params = self.click_params();
        let draft = self.tab.text_draft.clone();
        let Some(task) = self.tab.corner_task.as_mut() else {
            return;
        };
        let mut refused = None;
        match &mut task.subject {
            Subject::Text { spec, .. } => {
                **spec = draft;
                if let Err(why) = text::preview_font(ctx.document, spec) {
                    refused = Some(why);
                } else {
                    match text::values_of(ctx.document, &spec.text) {
                        Ok(values) => spec.values = values,
                        Err(why) => refused = Some(why),
                    }
                }
            }
            _ => {
                let corner = task.settings.params;
                task.settings.params = crate::tools::ToolParams {
                    copies: corner.copies,
                    auto_constraints: corner.auto_constraints,
                    arc_helpers: corner.arc_helpers,
                    ..params
                };
            }
        }
        task.working = match refused {
            Some(why) => Err(why),
            None => task.calculate(auto_update),
        };
        self.tab.last_diagnosis = None;
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
