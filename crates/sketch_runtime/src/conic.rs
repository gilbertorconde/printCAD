//! Conic gesture: collect the kind's picks, then ask the host to apply the
//! same geometry shown in preview. Document identities and history stay outside.
use sketch_geometry::world::{ScreenPx, WorldLength, WorldPerPx, WorldPoint};

use crate::conic_plan::{
    ConicGeometry, ConicKind, ConicOutcome, ConicPlan, ConicPointRole, ConicRejection,
    HyperbolaInput, plan_conic_with_hyperbola_input,
};
use crate::geometry::SourceId;
use crate::pick::Pick;
use crate::placement::{PlacedPick, Placement};
use crate::point::{Event, PointPlan, Rejection};
use crate::scene::{ContactTolerance, PickScene};

pub const MIN_PICK_GAP_PX: ScreenPx = ScreenPx::new(1.0);
pub fn min_pick_gap(scale: WorldPerPx) -> WorldLength {
    scale * MIN_PICK_GAP_PX
}

#[derive(Debug, Clone, PartialEq)]
pub struct Construction {
    pub curve: ConicPlan,
    pub picks: Vec<Pick>,
}
impl Construction {
    /// Only roles declared by the geometric constructor inherit a pick.
    /// Derived centres, foci and projected ends are free model points.
    pub fn point(&self, role: ConicPointRole) -> PointPlan {
        if let Some(binding) = self.curve.bindings.iter().find(|b| b.role == role) {
            return PointPlan::new(self.picks[binding.pick].clone());
        }
        let pos = match (self.curve.geometry, role) {
            (
                ConicGeometry::Circle { center, .. }
                | ConicGeometry::Arc { center, .. }
                | ConicGeometry::Ellipse { center, .. }
                | ConicGeometry::Hyperbola { center, .. },
                ConicPointRole::Anchor,
            ) => center,
            (ConicGeometry::Parabola { vertex, .. }, ConicPointRole::Anchor) => vertex,
            (
                ConicGeometry::Ellipse { focus1, .. } | ConicGeometry::Hyperbola { focus1, .. },
                ConicPointRole::Focus,
            ) => focus1,
            (ConicGeometry::Parabola { focus, .. }, ConicPointRole::Focus) => focus,
            (
                ConicGeometry::Arc { start, .. }
                | ConicGeometry::Ellipse { start, .. }
                | ConicGeometry::Hyperbola { start, .. }
                | ConicGeometry::Parabola { start, .. },
                ConicPointRole::Start,
            ) => start,
            (
                ConicGeometry::Arc { end, .. }
                | ConicGeometry::Ellipse { end, .. }
                | ConicGeometry::Hyperbola { end, .. }
                | ConicGeometry::Parabola { end, .. },
                ConicPointRole::End,
            ) => end,
            _ => panic!("conic has no requested point role"),
        };
        PointPlan::new(Pick {
            cursor: pos,
            pos,
            snap: None,
        })
    }
}
/// Actual host identities. A host exposes only the model roles it stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub curve: SourceId,
    pub points: Vec<(ConicPointRole, SourceId)>,
}
#[derive(Debug, Clone)]
pub enum ConicPreview {
    Cursor {
        pick: Pick,
    },
    Frame {
        picks: Vec<Pick>,
        cursor: Pick,
    },
    Curve {
        plan: ConicPlan,
        picks: Vec<Pick>,
        cursor: Pick,
    },
    Rejected {
        reason: ConicRejection,
        picks: Vec<Pick>,
        cursor: Pick,
    },
}
impl ConicPreview {
    pub fn pick(&self) -> &Pick {
        match self {
            Self::Cursor { pick } => pick,
            Self::Frame { cursor, .. }
            | Self::Curve { cursor, .. }
            | Self::Rejected { cursor, .. } => cursor,
        }
    }
}
#[derive(Debug, Clone)]
pub struct ConicStep {
    pub plan: Option<Construction>,
    pub preview: Option<ConicPreview>,
    pub deactivate: bool,
}
#[derive(Debug, Clone)]
pub struct ConicTool {
    kind: ConicKind,
    picks: Vec<Pick>,
    pending: Option<Vec<Pick>>,
    pub placement: Placement,
    pub hyperbola_input: HyperbolaInput,
    pub application: Option<Result<Applied, Rejection>>,
}
impl ConicTool {
    pub fn new(kind: ConicKind) -> Self {
        Self {
            kind,
            picks: Vec::new(),
            pending: None,
            placement: Placement::default(),
            hyperbola_input: HyperbolaInput::default(),
            application: None,
        }
    }
    pub fn kind(&self) -> ConicKind {
        self.kind
    }
    pub fn collected(&self) -> usize {
        self.picks.len()
    }
    pub fn picks(&self) -> &[Pick] {
        &self.picks
    }
    pub fn gesture_alive(&self) -> bool {
        !self.picks.is_empty() || self.pending.is_some()
    }
    pub fn deactivate(&mut self) {
        self.picks.clear();
        self.pending = None;
    }
    pub fn pick(
        &self,
        scene: &PickScene,
        cursor: WorldPoint,
        scale: WorldPerPx,
        tolerance: ContactTolerance,
    ) -> Pick {
        self.placement
            .resolve_with(cursor, |at| {
                let snap = scene.snap_at(at, scale, tolerance);
                PlacedPick {
                    cursor: at,
                    pos: snap.as_ref().map_or(at, |s| s.snapped_pos),
                    snap,
                }
            })
            .selected()
    }
    /// Refresh selected live vertices without selecting a new candidate.
    /// Missing addresses survive for the host's atomic refusal.
    fn live_picks(&self, scene: &PickScene) -> Vec<Pick> {
        self.picks
            .iter()
            .map(|pick| {
                let mut pick = pick.clone();
                if let Some(snap) = &pick.snap
                    && let Some(point) = scene.point_source(snap.source)
                    && let Some(pos) = scene.point_position(point)
                {
                    pick.pos = pos;
                }
                pick
            })
            .collect()
    }
    fn plan_with(&self, picks: &[Pick], last: &Pick) -> ConicOutcome {
        let mut positions: Vec<_> = picks.iter().map(|p| p.pos).collect();
        positions.push(last.pos);
        plan_conic_with_hyperbola_input(self.kind, &positions, self.hyperbola_input)
    }
    pub fn preview_with_pick(&self, scene: &PickScene, pick: Pick) -> ConicPreview {
        let picks = self.live_picks(scene);
        if picks.is_empty() {
            return ConicPreview::Cursor { pick };
        }
        if picks.len() + 1 < self.kind.arity() {
            return ConicPreview::Frame {
                picks,
                cursor: pick,
            };
        }
        match self.plan_with(&picks, &pick) {
            ConicOutcome::Built(plan) => ConicPreview::Curve {
                plan,
                picks,
                cursor: pick,
            },
            ConicOutcome::Rejected(reason) => ConicPreview::Rejected {
                reason,
                picks,
                cursor: pick,
            },
        }
    }
    pub fn step(
        &mut self,
        scene: &PickScene,
        scale: WorldPerPx,
        event: Event,
        pick: Option<Pick>,
    ) -> ConicStep {
        let mut plan = None;
        match event {
            Event::Cancel => self.deactivate(),
            Event::External | Event::Commit => {}
            Event::Hover | Event::Click => {
                assert!(
                    self.pending.is_none(),
                    "conic application needs an acknowledgement"
                );
                let pick = pick.expect("a pointer event requires a selected pick");
                if event == Event::Click {
                    self.picks = self.live_picks(scene);
                    let gap = self.placement.minimum(min_pick_gap(scale));
                    if !self.picks.iter().any(|p| p.pos.distance(pick.pos) <= gap) {
                        if self.picks.len() + 1 < self.kind.arity() {
                            self.picks.push(pick.clone());
                        } else if let ConicOutcome::Built(curve) =
                            self.plan_with(&self.picks, &pick)
                        {
                            let previous = std::mem::take(&mut self.picks);
                            let mut picks = previous.clone();
                            picks.push(pick.clone());
                            plan = Some(Construction { curve, picks });
                            self.pending = Some(previous);
                        }
                    }
                }
                return ConicStep {
                    plan,
                    preview: Some(self.preview_with_pick(scene, pick)),
                    deactivate: false,
                };
            }
        }
        ConicStep {
            plan,
            preview: None,
            deactivate: event == Event::Cancel,
        }
    }
    pub fn applied(&mut self, outcome: Result<Applied, Rejection>) {
        let previous = self
            .pending
            .take()
            .expect("conic acknowledgement follows a plan");
        if outcome.is_err() {
            self.picks = previous;
        }
        self.application = Some(outcome);
    }
}
