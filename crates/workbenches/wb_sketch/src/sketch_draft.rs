//! Exclusive sketch task sessions, their isolated feature and input routing.

use crate::{SketchFeature, SketchWorkbench};
use core_document::{
    HostRequest, InputResult, KeyCode, WorkbenchId, WorkbenchInputEvent, WorkbenchRuntimeContext,
};
use std::collections::HashSet;
use uuid::Uuid;

/// Input and the task panel route exclusively to the open task. Switching tools
/// discards that task before another one can start; this is document state,
/// while each interaction machine stays with its tool.
#[derive(Default)]
pub(crate) enum SketchSession {
    #[default]
    None,
    Rectangle(crate::rect_session::RectangleSession),
    Corner(crate::corner_session::CornerSession),
    Dimension(crate::dimension_session::DimensionSession),
}

impl SketchSession {
    fn draft(&self) -> Option<&SketchDraft> {
        match self {
            Self::None => None,
            Self::Rectangle(s) => Some(&s.draft),
            Self::Corner(s) => Some(&s.draft),
            Self::Dimension(s) => Some(&s.draft),
        }
    }

    pub fn rectangle(&self) -> Option<&crate::rect_session::RectangleSession> {
        match self {
            Self::Rectangle(s) => Some(s),
            _ => None,
        }
    }

    pub fn rectangle_mut(&mut self) -> Option<&mut crate::rect_session::RectangleSession> {
        match self {
            Self::Rectangle(s) => Some(s),
            _ => None,
        }
    }

    pub fn corner(&self) -> Option<&crate::corner_session::CornerSession> {
        match self {
            Self::Corner(s) => Some(s),
            _ => None,
        }
    }

    pub fn corner_mut(&mut self) -> Option<&mut crate::corner_session::CornerSession> {
        match self {
            Self::Corner(s) => Some(s),
            _ => None,
        }
    }

    pub fn dimension(&self) -> Option<&crate::dimension_session::DimensionSession> {
        match self {
            Self::Dimension(s) => Some(s),
            _ => None,
        }
    }

    pub fn dimension_mut(&mut self) -> Option<&mut crate::dimension_session::DimensionSession> {
        match self {
            Self::Dimension(s) => Some(s),
            _ => None,
        }
    }
}

pub(crate) struct SketchDraft {
    pub base: SketchFeature,
    pub working: Result<SketchFeature, String>,
    seq: u64,
}

impl SketchDraft {
    pub fn new(base: SketchFeature, ctx: &WorkbenchRuntimeContext) -> Self {
        Self {
            working: Ok(base.clone()),
            base,
            seq: ctx.document.mutation_seq(),
        }
    }

    pub fn is_current(&self, ctx: &WorkbenchRuntimeContext) -> bool {
        self.seq == ctx.document.mutation_seq()
    }

    /// A refused or stale candidate leaves the confirmed feature on screen.
    pub fn shown(&self, ctx: &WorkbenchRuntimeContext) -> Option<&SketchFeature> {
        self.is_current(ctx)
            .then(|| self.working.as_ref().ok())
            .flatten()
    }

    pub fn set_sketch(&mut self, sketch: Result<crate::sketch::Sketch, String>) {
        self.working = sketch.map(|sketch| SketchFeature {
            sketch,
            ..self.base.clone()
        });
    }
}

impl SketchWorkbench {
    /// Complete solved candidate shown by the editing view, or the confirmed
    /// feature while there is no valid candidate. Does not write the document.
    pub fn displayed_sketch(&self, ctx: &WorkbenchRuntimeContext) -> Option<SketchFeature> {
        self.shown_draft(ctx)
            .cloned()
            .or_else(|| self.get_active_sketch(ctx))
    }

    /// The dimensions of the edited sketch that a formula sets.
    pub(crate) fn bound_dimensions(&self, ctx: &WorkbenchRuntimeContext) -> HashSet<Uuid> {
        let mut dimensions: HashSet<Uuid> = self
            .active_sketch_id
            .and_then(|id| ctx.document.get_feature_meta(id))
            .map(|node| {
                node.formulas
                    .keys()
                    .filter_map(|k| Uuid::parse_str(k).ok())
                    .collect()
            })
            .unwrap_or_default();
        if self
            .session
            .dimension()
            .is_some_and(|s| s.draft.shown(ctx).is_some())
            && let Some(edit) = &self.dim_edit
        {
            if edit.save_as.is_some() || !core_document::expr::is_constant(&edit.text) {
                dimensions.insert(edit.constraint);
            } else {
                dimensions.remove(&edit.constraint);
            }
        }
        dimensions
    }

    pub(crate) fn shown_draft(&self, ctx: &WorkbenchRuntimeContext) -> Option<&SketchFeature> {
        self.session.draft()?.shown(ctx)
    }

    /// A task captures input until accepted, cancelled or left by a tool switch.
    /// Invalidating before Enter prevents accepting or rebuilding a stale draft.
    pub(crate) fn sketch_task_input(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
    ) -> Option<InputResult> {
        let stale = self.invalidate_rectangle(ctx)
            | self.invalidate_corner(ctx)
            | self.invalidate_dimension(ctx);
        if stale
            && matches!(
                event,
                WorkbenchInputEvent::KeyPress {
                    key: KeyCode::Enter
                }
            )
        {
            return Some(InputResult::consumed());
        }
        // The dimension controls also serve the immediate new-dimension editor.
        if self.dim_edit.is_some() {
            if active_tool.map(str::to_string) != self.activated_tool {
                self.cancel_dim_edit();
            } else {
                match event {
                    WorkbenchInputEvent::KeyPress {
                        key: KeyCode::Enter,
                    } => self.accept_dimension(ctx),
                    WorkbenchInputEvent::KeyPress {
                        key: KeyCode::Escape,
                    } => self.cancel_dim_edit(),
                    _ => {}
                }
                return Some(InputResult::consumed());
            }
        }
        match self.session {
            SketchSession::Corner(_) => {
                if active_tool.map(str::to_string) != self.activated_tool {
                    self.discard_corner();
                } else {
                    match event {
                        WorkbenchInputEvent::KeyPress {
                            key: KeyCode::Enter,
                        } => self.accept_corner(ctx),
                        WorkbenchInputEvent::KeyPress {
                            key: KeyCode::Escape,
                        } => self.discard_corner(),
                        _ => {}
                    }
                    return Some(InputResult::consumed());
                }
            }
            SketchSession::Rectangle(_) => {
                if active_tool.map(str::to_string) != self.activated_tool {
                    self.discard_rectangle();
                } else {
                    match event {
                        WorkbenchInputEvent::KeyPress {
                            key: KeyCode::Enter,
                        } => self.accept_rectangle(ctx),
                        WorkbenchInputEvent::KeyPress {
                            key: KeyCode::Escape,
                        } => self.discard_rectangle(),
                        _ => self.rectangle_pointer(ctx, event),
                    }
                    return Some(InputResult::consumed());
                }
            }
            SketchSession::None | SketchSession::Dimension(_) => {}
        }
        None
    }

    /// Acceptance writes the shown feature through ordinary document ops. The
    /// host closes their gesture once, including dimension formula/variable ops.
    pub(crate) fn finish_sketch_task(&mut self, ctx: &mut WorkbenchRuntimeContext, label: &str) {
        self.unsolved_edits = false;
        self.last_solve = None;
        self.last_diagnosis = None;
        ctx.request(HostRequest::JournalLabel(label.into()));
        ctx.request(HostRequest::CloseToolGesture(WorkbenchId::from(
            "wb.sketch",
        )));
    }
}
