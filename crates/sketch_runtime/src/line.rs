//! Click-click line and mixed chain. Application, solve and undo belong to the host.
use sketch_geometry::world::{ScreenPx, WorldLength, WorldPerPx, WorldPoint};

use crate::direction::{DirectionQuery, ToolAxis, axis_between};
use crate::geometry::SourceId;
use crate::pick::{Pick, SnapKind, Source};
use crate::placement::Placement;
use crate::point::{PointPlan, Rejection};
use crate::polyline::{self, ArcGeometry, PolylineCurve, PolylineMode};
use crate::scene::{ContactTolerance, PickScene};

pub const MIN_LENGTH_PX: ScreenPx = ScreenPx::new(1.0);
pub fn min_length(scale: WorldPerPx) -> WorldLength {
    scale * MIN_LENGTH_PX
}

#[derive(Debug, Clone)]
pub enum SegmentStart {
    Fresh(Pick),
    Vertex { point: SourceId, pos: WorldPoint },
}
impl SegmentStart {
    pub fn pos(&self) -> WorldPoint {
        match self {
            Self::Fresh(pick) => pick.pos,
            Self::Vertex { pos, .. } => *pos,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub enum LinePhase {
    #[default]
    Idle,
    Placing {
        start: SegmentStart,
    },
    AwaitingCommit,
}

/// Directional relation selected by the gesture, translated by the host.
#[derive(Debug, Clone, Copy)]
pub enum Alignment {
    Parallel(SourceId),
    Perpendicular(SourceId),
    Tangent(SourceId),
}

#[derive(Debug, Clone)]
pub struct LinePlan {
    pub start: SegmentStart,
    pub end: PointPlan,
    pub close: Option<SourceId>,
    pub curve: PolylineCurve,
    pub previous: Option<SourceId>,
    pub mode: PolylineMode,
    pub axis: Option<ToolAxis>,
    pub align: bool,
    pub alignment: Option<Alignment>,
    pub relation: Option<polyline::ArcRelation>,
}

/// The actual identities created/reused in the host, reported after solve.
#[derive(Debug, Clone, Copy)]
pub struct Applied {
    pub curve: SourceId,
    pub a: SourceId,
    pub b: SourceId,
}

/// Opaque receipt of one closed gesture, stable through redo and never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineHistoryId(pub u128);

#[derive(Debug, Clone)]
struct ChainStep {
    before: LinePhase,
    previous: Option<SourceId>,
    first: Option<SourceId>,
    history: Option<LineHistoryId>,
}

#[derive(Debug, Clone, Default)]
pub struct LineTool {
    pub phase: LinePhase,
    pub placement: Placement,
    pub mode: PolylineMode,
    previous: Option<SourceId>,
    first: Option<SourceId>,
    history: Vec<ChainStep>,
    pending: Option<LinePhase>,
    pub application: Option<Result<Applied, Rejection>>,
}

#[derive(Debug, Clone)]
pub enum LinePreview {
    Cursor {
        pick: Pick,
    },
    Rubber {
        start: WorldPoint,
        end: Pick,
    },
    Arc {
        geometry: ArcGeometry,
        end: Pick,
    },
    Refused {
        start: WorldPoint,
        end: Pick,
        reason: &'static str,
    },
}
impl LinePreview {
    pub fn pick(&self) -> &Pick {
        match self {
            Self::Cursor { pick } => pick,
            Self::Rubber { end, .. } | Self::Arc { end, .. } | Self::Refused { end, .. } => end,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Hover,
    Click,
    Cancel,
    External,
    Commit,
}

#[derive(Debug)]
pub struct LineStep {
    pub plan: Option<LinePlan>,
    pub preview: Option<LinePreview>,
    pub deactivate: bool,
}

impl LineTool {
    pub fn gesture_alive(&self) -> bool {
        !matches!(self.phase, LinePhase::Idle)
    }
    pub fn can_arc(&self) -> bool {
        self.previous.is_some()
    }
    pub fn previous_link(&self) -> Option<SourceId> {
        self.previous
    }

    pub fn deactivate(&mut self) {
        assert!(
            !matches!(self.phase, LinePhase::AwaitingCommit),
            "host must acknowledge the pending link"
        );
        self.phase = LinePhase::Idle;
        self.previous = None;
        self.first = None;
        self.history.clear();
        self.mode = PolylineMode::Line;
    }

    pub fn refresh_start(&mut self, scene: &PickScene) {
        if let LinePhase::Placing {
            start: SegmentStart::Vertex { point, pos },
        } = &mut self.phase
        {
            *pos = scene
                .point_position(*point)
                .expect("active chain vertex is live");
        }
    }

    pub fn snap_query(&self, scene: &PickScene) -> DirectionQuery {
        match &self.phase {
            LinePhase::Idle => DirectionQuery::Extensions,
            LinePhase::Placing { start } if self.mode == PolylineMode::Line => {
                let (pos, vertex) = match start {
                    SegmentStart::Fresh(pick) => (pick.pos, None),
                    SegmentStart::Vertex { point, .. } => (
                        scene
                            .point_position(*point)
                            .expect("active chain vertex is live"),
                        Some(*point),
                    ),
                };
                DirectionQuery::From { pos, vertex }
            }
            _ => DirectionQuery::None,
        }
    }

    pub fn pick(
        &self,
        scene: &PickScene,
        cursor: WorldPoint,
        scale: WorldPerPx,
        tolerance: ContactTolerance,
    ) -> Pick {
        let extra = scene.placement_objects(self.snap_query(scene), &self.placement);
        self.placement
            .resolve_with(cursor, |at| {
                let snap = scene
                    .snap_candidates(at, crate::snap::snap_tolerance(scale), &extra, tolerance)
                    .into_iter()
                    .min_by(crate::scene::snap_cmp)
                    .map(|snap| {
                        crate::direction::onto_curve(
                            &extra,
                            &scene.snap_objects(),
                            at,
                            crate::snap::snap_tolerance(scale),
                            snap,
                        )
                    });
                crate::placement::PlacedPick {
                    cursor: at,
                    pos: snap.as_ref().map_or(at, |s| s.snapped_pos),
                    snap,
                }
            })
            .selected()
    }

    pub fn preview_with_pick(&self, scene: &PickScene, pick: &Pick) -> LinePreview {
        match &self.phase {
            LinePhase::Idle | LinePhase::AwaitingCommit => {
                LinePreview::Cursor { pick: pick.clone() }
            }
            LinePhase::Placing { start } => {
                let pos = match start {
                    SegmentStart::Fresh(pick) => pick.pos,
                    SegmentStart::Vertex { point, .. } => {
                        scene.point_position(*point).expect("chain vertex is live")
                    }
                };
                if self.mode != PolylineMode::Line {
                    let SegmentStart::Vertex { point, .. } = start else {
                        unreachable!("arc needs a previous link")
                    };
                    return match polyline::arc_plan(
                        pos,
                        polyline::heading_into(scene, self.previous.unwrap(), *point),
                        pick.pos,
                        self.mode,
                    ) {
                        Ok(geometry) => LinePreview::Arc {
                            geometry,
                            end: pick.clone(),
                        },
                        Err(reason) => LinePreview::Refused {
                            start: pos,
                            end: pick.clone(),
                            reason,
                        },
                    };
                }
                LinePreview::Rubber {
                    start: pos,
                    end: pick.clone(),
                }
            }
        }
    }

    /// A resolved pick is authoritative: replay and numeric input do not snap again.
    pub fn step(
        &mut self,
        scene: &PickScene,
        scale: WorldPerPx,
        event: Event,
        pick: Option<Pick>,
    ) -> LineStep {
        assert!(
            !matches!(self.phase, LinePhase::AwaitingCommit),
            "host must acknowledge the pending link"
        );
        if !matches!(event, Event::Cancel | Event::External) {
            self.refresh_start(scene);
        }
        let mut step = LineStep {
            plan: None,
            preview: None,
            deactivate: false,
        };
        match event {
            Event::Cancel => {
                self.deactivate();
                step.deactivate = true;
            }
            Event::Commit => {}
            Event::External => {
                self.history.clear();
                let live = match self.phase {
                    LinePhase::Placing {
                        start: SegmentStart::Vertex { point, .. },
                    } => {
                        scene.point_position(point).is_some()
                            && self.previous.is_some_and(|id| scene.element(id).is_some())
                    }
                    _ => true,
                };
                if live {
                    self.refresh_start(scene);
                } else {
                    self.deactivate();
                }
            }
            Event::Hover | Event::Click => {
                let pick = pick.expect("a pointer event requires a selected pick");
                if event == Event::Click {
                    match &self.phase {
                        LinePhase::Idle => {
                            self.phase = LinePhase::Placing {
                                start: SegmentStart::Fresh(pick.clone()),
                            }
                        }
                        LinePhase::Placing { start } => {
                            if start.pos().distance(pick.pos)
                                > self.placement.minimum(min_length(scale))
                            {
                                let axis = pick
                                    .snap
                                    .as_ref()
                                    .filter(|s| {
                                        s.kind == SnapKind::OnAxis
                                            && s.source.source == Source::Guide
                                    })
                                    .and_then(|_| axis_between(start.pos(), pick.pos))
                                    .filter(|_| !self.placement.auto_constraints_off);
                                let close = self.first.filter(|id| {
                                    pick.snap.as_ref().is_some_and(|s| {
                                        scene.point_source(s.source) == Some(*id)
                                            && scene.point_position(*id) == Some(pick.pos)
                                    })
                                });
                                let curve = if self.mode == PolylineMode::Line {
                                    PolylineCurve::Line
                                } else {
                                    let LinePreview::Arc { geometry, .. } =
                                        self.preview_with_pick(scene, &pick)
                                    else {
                                        step.preview = Some(self.preview_with_pick(scene, &pick));
                                        return step;
                                    };
                                    PolylineCurve::Arc(geometry)
                                };
                                let alignment = (!self.placement.auto_constraints_off
                                    && close.is_none())
                                .then(|| alignment_for_pick(&pick))
                                .flatten();
                                let relation = match curve {
                                    PolylineCurve::Line => None,
                                    PolylineCurve::Arc(arc) => Some(polyline::arc_relation(
                                        scene,
                                        self.previous.unwrap(),
                                        arc,
                                        self.mode,
                                    )),
                                };
                                step.plan = Some(LinePlan {
                                    start: start.clone(),
                                    end: PointPlan::new(pick.clone()),
                                    close,
                                    curve,
                                    previous: self.previous,
                                    mode: self.mode,
                                    axis,
                                    align: !self.placement.auto_constraints_off,
                                    alignment,
                                    relation,
                                });
                                self.pending = Some(self.phase.clone());
                                self.phase = LinePhase::AwaitingCommit;
                            }
                        }
                        LinePhase::AwaitingCommit => unreachable!(),
                    }
                }
                if !matches!(self.phase, LinePhase::AwaitingCommit) {
                    step.preview = Some(self.preview_with_pick(scene, &pick));
                }
            }
        }
        step
    }

    pub fn applied(
        &mut self,
        scene: &PickScene,
        outcome: Result<Applied, Rejection>,
        history: Option<LineHistoryId>,
    ) {
        assert!(
            matches!(self.phase, LinePhase::AwaitingCommit),
            "acknowledgement requires a pending link"
        );
        self.application = Some(outcome);
        let before = self
            .pending
            .take()
            .expect("pending link has an input phase");
        let Ok(applied) = outcome else {
            self.phase = before;
            self.refresh_start(scene);
            return;
        };
        let pos = scene
            .point_position(applied.b)
            .expect("applied endpoint is live");
        assert!(
            scene.point_position(applied.a).is_some() && scene.element(applied.curve).is_some(),
            "applied identities are live"
        );
        self.history.push(ChainStep {
            before,
            previous: self.previous,
            first: self.first,
            history,
        });
        self.previous = Some(applied.curve);
        self.first.get_or_insert(applied.a);
        self.phase = LinePhase::Placing {
            start: SegmentStart::Vertex {
                point: applied.b,
                pos,
            },
        };
        if self.first == Some(applied.b) {
            self.deactivate();
        }
    }

    pub fn can_undo_in_history(&self, id: LineHistoryId) -> bool {
        self.history.last().is_some_and(|s| s.history == Some(id))
    }
    pub fn undone_in_history(&mut self, scene: &PickScene, id: LineHistoryId) {
        assert!(
            self.can_undo_in_history(id),
            "host undo must name the last chain link"
        );
        let step = self.history.pop().unwrap();
        self.phase = step.before;
        self.previous = step.previous;
        self.first = step.first;
        if self.previous.is_none() {
            self.mode = PolylineMode::Line;
        }
        self.refresh_start(scene);
    }
}

/// A directional pick states a relation; geometry kinds are checked by its host.
pub fn alignment_for_pick(pick: &Pick) -> Option<Alignment> {
    let snap = pick.snap.as_ref()?;
    let Source::Geometry(id) = snap.source.source else {
        return None;
    };
    match snap.kind {
        SnapKind::Parallel => Some(Alignment::Parallel(id)),
        SnapKind::Perpendicular | SnapKind::PerpendicularTo => Some(Alignment::Perpendicular(id)),
        SnapKind::Tangent => Some(Alignment::Tangent(id)),
        _ => None,
    }
}
