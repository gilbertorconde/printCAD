//! Centre/radius gesture. Only the centre has point bindings; the rim
//! measures a radius and never constrains another object's point.
use sketch_geometry::world::{ScreenPx, WorldLength, WorldPerPx, WorldPoint};

use crate::geometry::SourceId;
use crate::pick::Pick;
use crate::placement::Placement;
use crate::point::{Event, PointPlan, Rejection};
use crate::scene::{ContactTolerance, PickScene};

pub const MIN_RADIUS_PX: ScreenPx = ScreenPx::new(1.0);
pub fn min_radius(scale: WorldPerPx) -> WorldLength {
    scale * MIN_RADIUS_PX
}

#[derive(Debug, Clone, Default)]
pub enum CirclePhase {
    #[default]
    Idle,
    Placing {
        center: Pick,
    },
}
#[derive(Debug, Clone)]
pub struct CirclePlan {
    pub center: PointPlan,
    pub rim: Pick,
    pub radius: WorldLength,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Applied {
    pub circle: SourceId,
    pub center: SourceId,
}
#[derive(Debug, Clone)]
pub enum CirclePreview {
    Cursor {
        pick: Pick,
    },
    Rubber {
        center: WorldPoint,
        rim: Pick,
        radius: WorldLength,
    },
}
impl CirclePreview {
    pub fn pick(&self) -> &Pick {
        match self {
            Self::Cursor { pick } | Self::Rubber { rim: pick, .. } => pick,
        }
    }
}
#[derive(Debug, Clone)]
pub struct CircleStep {
    pub plan: Option<CirclePlan>,
    pub preview: Option<CirclePreview>,
    pub deactivate: bool,
}
#[derive(Debug, Clone, Default)]
pub struct CircleTool {
    pub phase: CirclePhase,
    pub placement: Placement,
    pending: Option<Pick>,
    pub application: Option<Result<Applied, Rejection>>,
}
impl CircleTool {
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
                crate::placement::PlacedPick {
                    cursor: at,
                    pos: snap.as_ref().map_or(at, |s| s.snapped_pos),
                    snap,
                }
            })
            .selected()
    }
    /// Refresh a model vertex without repeating candidate selection. Missing
    /// sources stay in the plan so the host can refuse before its first write.
    fn center(&self, scene: &PickScene, center: &Pick) -> Pick {
        let mut center = center.clone();
        if let Some(snap) = &center.snap
            && let Some(point) = scene.point_source(snap.source)
            && let Some(pos) = scene.point_position(point)
        {
            center.pos = pos;
        }
        center
    }
    pub fn preview_with_pick(&self, scene: &PickScene, pick: &Pick) -> CirclePreview {
        match &self.phase {
            CirclePhase::Idle => CirclePreview::Cursor { pick: pick.clone() },
            CirclePhase::Placing { center } => {
                let center = self.center(scene, center).pos;
                CirclePreview::Rubber {
                    center,
                    rim: pick.clone(),
                    radius: center.distance(pick.pos),
                }
            }
        }
    }
    pub fn step(
        &mut self,
        scene: &PickScene,
        scale: WorldPerPx,
        event: Event,
        pick: Option<Pick>,
    ) -> CircleStep {
        let mut plan = None;
        match event {
            Event::Cancel => self.deactivate(),
            Event::External | Event::Commit => {}
            Event::Hover | Event::Click => {
                assert!(
                    self.pending.is_none(),
                    "circle application needs an acknowledgement"
                );
                let pick = pick.expect("a pointer event requires a selected pick");
                if event == Event::Click {
                    match &self.phase {
                        CirclePhase::Idle => {
                            self.phase = CirclePhase::Placing {
                                center: pick.clone(),
                            }
                        }
                        CirclePhase::Placing { center } => {
                            let center = self.center(scene, center);
                            let radius = center.pos.distance(pick.pos);
                            if radius > self.placement.minimum(min_radius(scale)) {
                                plan = Some(CirclePlan {
                                    center: PointPlan::new(center.clone()),
                                    rim: pick.clone(),
                                    radius,
                                });
                                self.pending = Some(center);
                                self.phase = CirclePhase::Idle;
                            }
                        }
                    }
                }
                return CircleStep {
                    plan,
                    preview: Some(self.preview_with_pick(scene, &pick)),
                    deactivate: false,
                };
            }
        }
        CircleStep {
            plan,
            preview: None,
            deactivate: event == Event::Cancel,
        }
    }
    pub fn applied(&mut self, outcome: Result<Applied, Rejection>) {
        let center = self
            .pending
            .take()
            .expect("circle acknowledgement follows a plan");
        if outcome.is_err() {
            self.phase = CirclePhase::Placing { center };
        }
        self.application = Some(outcome);
    }
    pub fn gesture_alive(&self) -> bool {
        matches!(self.phase, CirclePhase::Placing { .. }) || self.pending.is_some()
    }
    pub fn deactivate(&mut self) {
        self.phase = CirclePhase::Idle;
        self.pending = None;
    }
}
