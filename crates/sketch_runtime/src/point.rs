//! One-click point gesture, independent of picking and document storage.

use crate::binding::{self, Binding};
use crate::geometry::SourceId;
use crate::pick::Pick;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Hover,
    Click,
    Cancel,
    External,
    Commit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PointPlan {
    pub pick: Pick,
    pub bindings: Vec<Binding>,
}

impl PointPlan {
    pub fn new(pick: Pick) -> Self {
        let bindings = pick.snap.as_ref().map_or_else(Vec::new, binding::for_snap);
        Self { pick, bindings }
    }
}

/// The actual identity created or reused by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Applied {
    pub point: SourceId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    MissingSource(SourceId),
    Refused,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PointStep {
    pub plan: Option<PointPlan>,
    pub preview: Option<Pick>,
    pub deactivate: bool,
}

/// No phase survives a click. The acknowledgement is retained explicitly so
/// a refusal cannot be mistaken for an applied edit by the host's UI.
#[derive(Debug, Clone, Default)]
pub struct PointTool {
    pub application: Option<Result<Applied, Rejection>>,
}

impl PointTool {
    /// Free points can bind to curve extensions as well as ordinary scene snaps.
    pub fn pick(
        &self,
        scene: &crate::scene::PickScene,
        cursor: sketch_geometry::world::WorldPoint,
        scale: sketch_geometry::world::WorldPerPx,
        tolerance: crate::scene::ContactTolerance,
        placement: crate::placement::Placement,
    ) -> Pick {
        let extra = scene.direction_objects(crate::direction::DirectionQuery::Extensions);
        placement
            .resolve_with(cursor, |at| {
                let snap = scene
                    .snap_candidates(at, crate::snap::snap_tolerance(scale), &extra, tolerance)
                    .into_iter()
                    .min_by(crate::scene::snap_cmp);
                crate::placement::PlacedPick {
                    cursor: at,
                    pos: snap.as_ref().map_or(at, |s| s.snapped_pos),
                    snap,
                }
            })
            .selected()
    }

    pub fn step(&mut self, event: Event, pick: Option<Pick>) -> PointStep {
        match event {
            Event::Cancel | Event::External | Event::Commit => PointStep {
                plan: None,
                preview: None,
                deactivate: event == Event::Cancel,
            },
            Event::Hover | Event::Click => {
                let pick = pick.expect("a pointer event requires a selected pick");
                PointStep {
                    plan: (event == Event::Click).then(|| PointPlan::new(pick.clone())),
                    preview: Some(pick),
                    deactivate: false,
                }
            }
        }
    }

    pub fn applied(&mut self, outcome: Result<Applied, Rejection>) {
        self.application = Some(outcome);
    }
}
