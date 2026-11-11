//! The fillet and chamfer card: a corner picked in the view opens a card
//! beside it, its size typed straight away and Keep corner switched there,
//! the view showing the solved result, until OK writes it or Cancel drops
//! it.
//!
//! The draft is the click itself, run again on the sketch as the click
//! found it with the sizes now set (`step::click`, as the recorded
//! `sketch.draw` call replays it), so what the view shows, what Apply
//! stores and what a replay makes are one result.

use std::collections::HashSet;

use bench_api::{FloatingCard, PanelEvent, Tone, panel};
use core_document::{HostRequest, WorkbenchRuntimeContext};
use uuid::Uuid;

use crate::feature::SketchFeature;
use crate::operations::SolveOutcome;
use crate::sketch::Vec2D;
use crate::step::{self, StepSettings};
use crate::tools::ToolState;
use crate::{DrawRecord, SketchWorkbench, solver};

/// A corner open in its card.
pub(crate) struct CornerTask {
    /// The open sketch as the click found it.
    base: SketchFeature,
    /// The document's edit count when it opened: an edit from elsewhere
    /// (an undo, another view) puts the task away.
    seq: u64,
    tool: String,
    /// What the click is replayed with: the tool's state before it (a
    /// first curve picked), where it landed, its settings and selection.
    state: ToolState,
    cursor: Vec2D,
    settings: StepSettings,
    selected: HashSet<Uuid>,
    /// The recorded `sketch.draw` call the click belongs to, written on
    /// Apply with the sizes then set.
    record: Option<DrawRecord>,
    /// The two curves the corner joins.
    curves: [Uuid; 2],
    /// The size as typed in the card: a value, a unit or a formula.
    size_text: String,
    /// The sketch with the corner cut and solved, or why it cannot be.
    working: Result<SketchFeature, String>,
}

impl CornerTask {
    fn bevel(&self) -> bool {
        self.tool == "sketch.chamfer"
    }

    fn size(&self) -> f64 {
        let p = &self.settings.params;
        if self.bevel() {
            p.chamfer_length
        } else {
            p.fillet_radius
        }
    }

    /// Cut the corner on the base sketch at the sizes set, and solve it
    /// when the sketch solves as it is edited.
    fn calculate(&self, auto_update: bool) -> Result<SketchFeature, String> {
        let size = self.size();
        if !(size.is_finite() && size >= 1e-6) {
            return Err(if self.bevel() {
                "Enter a length greater than zero".into()
            } else {
                "Enter a radius greater than zero".into()
            });
        }
        let mut feature = self.base.clone();
        let mut state = self.state.clone();
        let mut capture = crate::ovp::DimCapture::default();
        let outcome = step::click(
            &mut state,
            &mut capture,
            &self.tool,
            &mut feature.sketch,
            self.cursor,
            &[],
            false,
            &self.settings,
            &self.selected,
        );
        if !outcome.changed {
            return Err(outcome
                .log
                .unwrap_or_else(|| "These curves cannot be joined at this size".into()));
        }
        if auto_update
            && matches!(
                solver::solve(&mut feature.sketch),
                SolveOutcome::NotConverged { .. }
            )
        {
            return Err("The constraints cannot be solved with this corner".into());
        }
        Ok(feature)
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
        let mut task = CornerTask {
            base: feature.clone(),
            seq: ctx.document.mutation_seq(),
            tool: tool.to_string(),
            state: std::mem::take(&mut self.tool_state),
            cursor,
            settings: *settings,
            selected: self.selected.iter().copied().collect(),
            record: self.draw_record.take(),
            curves,
            size_text: String::new(),
            working: Err(String::new()),
        };
        task.size_text = crate::glyphs::fmt_num(task.size());
        task.working = task.calculate(self.options.auto_update);
        self.corner_task = Some(task);
        self.dim_capture.clear_buffers();
        self.last_diagnosis = None;
        Some(())
    }

    /// The sketch the view shows: the open corner's solved result, else
    /// the stored sketch. Writes nothing.
    pub(crate) fn displayed_sketch(&self, ctx: &WorkbenchRuntimeContext) -> Option<SketchFeature> {
        self.corner_task
            .as_ref()
            .filter(|task| task.seq == ctx.document.mutation_seq())
            .and_then(|task| task.working.as_ref().ok())
            .cloned()
            .or_else(|| self.get_active_sketch(ctx))
    }

    /// Put an open corner away, writing and recording nothing.
    pub(crate) fn discard_corner(&mut self) {
        if self.corner_task.take().is_some() {
            self.last_diagnosis = None;
        }
    }

    /// Put the corner away when the document changed under it. Answers
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

    /// Store the corner's result as one undo step, recorded as the
    /// `sketch.draw` call that makes it. Nothing happens while the result
    /// is refused.
    pub(crate) fn apply_corner(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if self.drop_stale_corner(ctx) {
            return;
        }
        let Some(task) = self.corner_task.take() else {
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
        let label = if task.bevel() { "Chamfer" } else { "Fillet" };
        let size = task.size();
        if let Some(mut record) = task.record {
            let params = step::params_to_json(&task.settings.params);
            if params.is_empty() {
                record.settings.remove("params");
            } else {
                record.settings.insert("params".into(), params.into());
            }
            record.changed = true;
            self.draw_record = Some(record);
            self.flush_draw_record(ctx);
        }
        ctx.log_info(format!("{label} {size:.2}"));
        // The host closes the card's undo step once OK is handled.
        ctx.request(HostRequest::JournalLabel(label.into()));
    }

    /// The card: the curves joined, the size (with the keys as it opens),
    /// Keep corner, by the corner; OK held back with the reason while the
    /// corner cannot be cut.
    pub(crate) fn corner_card(&self, ctx: &WorkbenchRuntimeContext) -> Option<FloatingCard> {
        let task = self.corner_task.as_ref()?;
        let names = task
            .curves
            .map(|id| crate::style::element_name(&task.base.sketch, id));
        let label = if task.bevel() { "Length" } else { "Radius" };
        let mut size = panel::entry_with("corner.size", label, &task.size_text, "A size");
        size.live = true;
        size.mono = true;
        size.formulas = true;
        let mut keep = panel::check(
            "corner.keep",
            "Corner",
            "Keep it",
            task.settings.params.corner_keep,
        );
        keep.hint =
            "The corner stays as a construction point on both curves, with its constraints".into();
        let stale = task.seq != ctx.document.mutation_seq();
        let refusal = match (&task.working, stale) {
            (_, true) => Some("The sketch changed: pick the corner again".to_string()),
            (Err(why), _) => Some(why.clone()),
            (Ok(_), _) => None,
        };
        let plane = &task.base.plane;
        let at = std::array::from_fn(|i| {
            (plane.origin[i] + plane.x_axis[i] * task.cursor.x + plane.y_axis[i] * task.cursor.y)
                as f32
        });
        let mut card = panel::floating_card(
            "sketch.corner",
            if task.bevel() { "Chamfer" } else { "Fillet" },
            at,
            Some("corner.size"),
            [
                panel::caption(&format!("{} + {}", names[0], names[1]), Tone::Neutral, true),
                size.into(),
                keep.into(),
            ],
        );
        card.icon = Some(
            if task.bevel() {
                "sketch-chamfer"
            } else {
                "sketch-fillet"
            }
            .into(),
        );
        card.refusal = refusal;
        Some(card)
    }

    /// A change on the corner's card. Answers whether it was one. The size
    /// and switch carry on to the tool's next corner.
    pub(crate) fn corner_event(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        event: &PanelEvent,
    ) -> bool {
        let Some(task) = self.corner_task.as_mut() else {
            return false;
        };
        match event {
            PanelEvent::CardClose { id, accept } if id == "sketch.corner" => {
                if *accept {
                    self.apply_corner(ctx);
                } else {
                    self.discard_corner();
                }
                return true;
            }
            PanelEvent::Text { id, value } if id == "corner.size" => {
                task.size_text.clone_from(value);
                match ctx
                    .document
                    .evaluate_formula(value, Some(core_document::expr::Dim::LENGTH))
                {
                    Ok(q) if task.bevel() => self.tool_params.chamfer_length = q.value,
                    Ok(q) => self.tool_params.fillet_radius = q.value,
                    Err(why) => {
                        task.working = Err(if value.trim().is_empty() {
                            "Type a size".into()
                        } else {
                            why
                        });
                        return true;
                    }
                }
            }
            PanelEvent::Toggle { id, on } if id == "corner.keep" => {
                self.tool_params.corner_keep = *on;
            }
            _ if event.id().starts_with("corner.") => return true,
            _ => return false,
        }
        self.recalculate_corner();
        true
    }

    /// Work the corner out again at the tool's sizes.
    fn recalculate_corner(&mut self) {
        let auto_update = self.options.auto_update;
        let p = self.tool_params;
        if let Some(task) = self.corner_task.as_mut() {
            let params = &mut task.settings.params;
            params.fillet_radius = p.fillet_radius;
            params.chamfer_length = p.chamfer_length;
            params.corner_keep = p.corner_keep;
            task.working = task.calculate(auto_update);
            self.last_diagnosis = None;
        }
    }
}
