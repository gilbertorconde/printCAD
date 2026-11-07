//! Rectangle placement, numeric parameters and handle gestures. The host owns
//! draft application, rollback, dimensions, solving and history. Acknowledgements
//! carry actual logical corners, sides and the optional construction centre.

use sketch_geometry::world::{ScreenPx, WorldLength, WorldPerPx, WorldPoint, WorldVec};

use crate::geometry::SourceId;
use crate::pick::Pick;
use crate::point::{PointPlan, Rejection};
use crate::rect_shape::{
    Corner, HeightSide, RectCornerMode, RectDimPlacement, RectDraft, RectFrame, RectRejection,
    WidthSide,
};
use crate::scene::{ContactTolerance, PickScene};

#[derive(Debug, Clone)]
pub enum Effect {
    RectDraft {
        draft: RectDraft,
        bindings: RectBindings,
    },
    DraftCommit,
    DraftCancel,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRequest {
    None,
    Deactivate,
}
#[derive(Debug, Clone, Copy)]
pub enum ToolEvent {
    Cancel,
    Commit,
    External,
    Click { cursor: WorldPoint },
    Press { cursor: WorldPoint },
    Drag { cursor: WorldPoint },
    Release { cursor: WorldPoint },
    MoveTo { cursor: WorldPoint },
    Tick { cursor: WorldPoint },
}
pub struct ToolWorld<'a> {
    pub scene: &'a PickScene,
    pub view_scale: WorldPerPx,
    pub tolerance: ContactTolerance,
    pub label_offset: WorldLength,
}
/// Identities returned by the host, in canonical local corner/side order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Applied {
    pub corners: [SourceId; 4],
    pub sides: [SourceId; 4],
    pub center: Option<SourceId>,
}
/// Canonical corners and a directed contour. Hosts translate these declared
/// relations into their own constraints and return canonical identities.
#[derive(Debug, Clone)]
pub struct RectanglePlan {
    pub corners: [PointPlan; 4],
    pub center: Option<PointPlan>,
    pub order: [Corner; 4],
    pub frame: RectFrame,
    pub relations: Vec<crate::rect_shape::SideRelation>,
}
impl RectTool {
    fn refresh_sources(&mut self, scene: &PickScene) {
        let refresh = |pick: &mut Pick| {
            if let Some(snap) = &pick.snap
                && let Some(point) = scene.point_source(snap.source)
                && let Some(pos) = scene.point_position(point)
            {
                pick.pos = pos;
            }
        };
        match &mut self.phase {
            RectPhase::Placing { start } => refresh(start),
            RectPhase::PlacingSide { start, edge_end } => {
                refresh(start);
                refresh(edge_end);
            }
            _ => {}
        }
    }
    pub fn preview_in_scene(&self, scene: &PickScene, pick: Pick) -> RectPreview {
        let mut tool = self.clone();
        tool.refresh_sources(scene);
        tool.preview_with_pick(pick)
    }
    pub fn outline_plan(&self, scene: &PickScene) -> RectanglePlan {
        let draft = self.draft().expect("rectangle plan requires a draft");
        assert_eq!(draft.corner_mode, RectCornerMode::None);
        assert!(draft.wall.is_none());
        let first = self.first_corner();
        let mut corners = draft.corners().map(|pos| {
            PointPlan::new(Pick {
                cursor: pos,
                pos,
                snap: None,
            })
        });
        let mut center = self.placement.is_centered().then(|| {
            PointPlan::new(Pick {
                cursor: draft.center,
                pos: draft.center,
                snap: None,
            })
        });
        let bindings = self.bindings().unwrap();
        for (site, point) in bindings.plans() {
            match site {
                RectSite::Corner(corner) => corners[corner.index()] = point,
                RectSite::Center => center = Some(point),
            }
        }
        // Computed adjacent corners may meet an existing exact vertex. They
        // never perform a tolerance-based object snap or acquire curve bindings.
        for corner in Corner::ALL {
            if corner != first
                && corner != first.opposite()
                && !bindings.sites().any(|s| s == RectSite::Corner(corner))
            {
                let point = &mut corners[corner.index()];
                if let Some(id) = scene.point_at(point.pick.pos) {
                    point.bindings.push(crate::binding::Binding::Coincident(
                        crate::pick::SourceRef {
                            source: crate::pick::Source::Geometry(id),
                            role: crate::pick::PointRole::Start,
                        },
                    ));
                }
            }
        }
        RectanglePlan {
            corners,
            center,
            frame: draft.frame,
            relations: crate::rect_shape::side_relations(draft.frame),
            order: if matches!(draft.frame, RectFrame::Rotated(_)) {
                [
                    first,
                    first.across_width(),
                    first.opposite(),
                    first.across_width().opposite(),
                ]
            } else {
                Corner::ALL
            },
        }
    }
}

/// Minimum placed side length in pixels.
///
/// Mouse jitter is a screen effect, so this threshold is independent of zoom.
pub const MIN_SIZE_PX: ScreenPx = ScreenPx::new(1.0);

/// Corner handle hit radius in pixels.
///
/// Convert through the current view scale before world-space hit tests.
pub const HANDLE_HIT_PX: ScreenPx = ScreenPx::new(21.0);

/// Placement threshold in world units at the current view scale.
pub fn min_size(view_scale: WorldPerPx) -> WorldLength {
    view_scale * MIN_SIZE_PX
}

/// Handle hit radius in world units at the current view scale.
pub fn handle_hit_radius(view_scale: WorldPerPx) -> WorldLength {
    view_scale * HANDLE_HIT_PX
}

/// Captured corner handle.
///
/// Fix grab_offset and anchor at press time; dragging uses start state plus delta.
///
/// Anchor is the opposite corner's position, not role: crossing it swaps min/max
/// roles. Looking up its role each frame would switch which corner stays fixed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandleDrag {
    pub corner: Corner,
    pub grab_offset: WorldVec,
    pub anchor: WorldPoint,
}

/// Explicit placement mode. An axis-aligned first side alone cannot distinguish
/// an ordinary rectangle from an intentionally rotated one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RectPlacement {
    /// Opposite diagonal corners, with H/V sides (`RectFrame::AxisAligned`).
    #[default]
    TwoCorners,
    /// Three corners: the first side sets rotation, the third click sets height.
    ThreeCorners,
    /// Center and corner, axis-aligned; full size is twice the center displacement.
    CenterCorner,
    /// Center, corner and direction to its neighbor on the same circumcircle.
    CenterTwoCorners,
}

impl RectPlacement {
    pub fn is_centered(self) -> bool {
        matches!(self, Self::CenterCorner | Self::CenterTwoCorners)
    }

    pub fn rotated(self) -> bool {
        matches!(self, Self::ThreeCorners | Self::CenterTwoCorners)
    }
}

/// Numeric-input anchor retaining the first click's local-frame role and position
/// across width, height and rotation edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RectAnchor {
    Corner(Corner),
    Center,
}

impl Default for RectAnchor {
    fn default() -> Self {
        Self::Corner(Corner::BottomLeft)
    }
}

impl RectAnchor {
    pub fn position(self, draft: &RectDraft) -> WorldPoint {
        match self {
            Self::Corner(corner) => draft.corner(corner),
            Self::Center => draft.center,
        }
    }

    /// Translation after size/angle edits, shared by panel and numeric placement.
    pub fn at(self, mut draft: RectDraft, position: WorldPoint) -> RectDraft {
        draft.center = draft.center + (position - self.position(&draft));
        draft
    }
}

/// Draft point role that may bind to a placement pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RectSite {
    Corner(Corner),
    /// Construction center point.
    Center,
}

impl RectSite {
    const ALL: [RectSite; 5] = [
        RectSite::Corner(Corner::BottomLeft),
        RectSite::Corner(Corner::BottomRight),
        RectSite::Corner(Corner::TopRight),
        RectSite::Corner(Corner::TopLeft),
        RectSite::Center,
    ];

    /// Point position derived from draft parameters.
    pub fn position(self, draft: &RectDraft) -> WorldPoint {
        match self {
            RectSite::Corner(corner) => draft.corner(corner),
            RectSite::Center => draft.center,
        }
    }
}

/// Placement picks bound to figure points. A corner on an endpoint, curve or
/// origin retains its addressed binding while that point stays fixed.
///
/// Bindings are explicit session parameters, resolved and validated by the host
/// before any write. A missing source is an application refusal.
///
/// Bind only actual figure points; a rotated mode's third pick supplies a
/// projection or direction rather than a point of the resulting rectangle.
#[derive(Debug, Clone, Default)]
pub struct RectBindings(Vec<(RectSite, Pick)>);

impl RectBindings {
    /// Selected placement sources, addressed by their logical rectangle site.
    pub fn picks(&self) -> impl Iterator<Item = (RectSite, &Pick)> {
        self.0.iter().map(|(site, pick)| (*site, pick))
    }

    /// Placement bindings; a click without a snap creates no stored binding.
    fn placed(clicks: [(RectSite, &Pick); 2]) -> Self {
        Self(
            clicks
                .into_iter()
                .filter(|(_, pick)| pick.snap.is_some())
                .map(|(site, pick)| (site, pick.clone()))
                .collect(),
        )
    }

    /// Currently bound figure points.
    pub fn sites(&self) -> impl Iterator<Item = RectSite> + '_ {
        self.0.iter().map(|(site, _)| *site)
    }

    /// Selected point requirements use the same binding policy as other gestures.
    pub fn plans(&self) -> impl Iterator<Item = (RectSite, PointPlan)> + '_ {
        self.0
            .iter()
            .map(|(site, pick)| (*site, PointPlan::new(pick.clone())))
    }

    /// Carry bindings across old -> next only for unmoved points, even when
    /// they change roles.
    ///
    /// Keeping a moved point bound would make solving undo the numeric edit.
    /// Find the fixed panel anchor or opposite drag corner by position in next,
    /// because crossing it changes roles. DEGENERATE allows only rounding in
    /// parameter reconstruction (center ± half), not inferred user intent.
    fn carried(&self, old: &RectDraft, next: &RectDraft) -> Self {
        Self(
            self.0
                .iter()
                .filter_map(|(site, pick)| {
                    let at = site.position(old);
                    RectSite::ALL
                        .into_iter()
                        .find(|candidate| {
                            candidate.position(next).distance(at) <= WorldLength::DEGENERATE
                        })
                        .map(|moved| (moved, pick.clone()))
                })
                .collect(),
        )
    }
}

#[derive(Debug, Clone, Default)]
pub enum RectPhase {
    /// No corner placed yet.
    #[default]
    Idle,
    /// First corner or center fixed; placing the next corner.
    Placing { start: Pick },
    /// Rotated placement after a side or center and first corner.
    PlacingSide { start: Pick, edge_end: Pick },
    /// Logical draft parameters; the application lifetime belongs to the host.
    Draft {
        draft: RectDraft,
        /// Point-to-pick bindings, the second rebuild parameter.
        bindings: RectBindings,
        /// Captured handle, or None for a live session without a pointer grab.
        drag: Option<HandleDrag>,
    },
}

/// Inter-frame gesture state, independent of host document/session data.
#[derive(Debug, Clone, Default)]
pub struct RectTool {
    pub placement_input: crate::placement::Placement,
    pub numeric_anchor: RectAnchor,
    pub phase: RectPhase,
    /// Next placement mode, retained across sessions and changed only through
    /// [`RectTool::set_placement`].
    pub placement: RectPlacement,
    /// Next placement's wall; edit the current session through `set_draft`.
    pub wall: Option<WorldLength>,
    /// The last host acknowledgement. An unacknowledged or refused draft cannot commit.
    pub application: Option<Result<Applied, Rejection>>,
    placement_retry: Option<Box<RectPhase>>,
}

/// Current preview as geometry data.
#[derive(Debug, Clone)]
pub enum RectPreview {
    /// Idle crosshair at the selected pick.
    Cursor { pick: Pick },
    /// First side in ThreeCorners; the third click will determine the rectangle.
    Edge { start: WorldPoint, end: Pick },
    /// Rectangle rubber band in TwoCorners or PlacingSide.
    Rubber {
        start: WorldPoint,
        end: Pick,
        /// The same four corners the construction plan will receive.
        corners: [WorldPoint; 4],
        inner: Result<Option<[WorldPoint; 4]>, RectRejection>,
    },
    /// Draft corner handles, with the dragged handle marked by grabbed.
    Handles {
        corners: [WorldPoint; 4],
        grabbed: Option<Corner>,
    },
}

impl RectPreview {
    /// Selected pick for the host's status display.
    ///
    /// None in Draft means handle dragging does not snap.
    pub fn pick(&self) -> Option<&Pick> {
        match self {
            RectPreview::Cursor { pick } => Some(pick),
            RectPreview::Edge { end, .. } | RectPreview::Rubber { end, .. } => Some(end),
            RectPreview::Handles { .. } => None,
        }
    }
}

#[derive(Debug)]
pub struct RectStep {
    pub effect: Option<Effect>,
    /// Cursorless events (Cancel, Commit, External) also have no pick.
    pub preview: Option<RectPreview>,
    pub request: ToolRequest,
}

impl RectStep {
    fn quiet() -> Self {
        Self {
            effect: None,
            preview: None,
            request: ToolRequest::None,
        }
    }
}

impl RectTool {
    /// Immediate-commit hosts retry the selected placement after a refusal;
    /// live-draft hosts instead keep the editable rejected draft.
    pub fn retry_placement(&mut self) {
        assert!(
            self.application.as_ref().is_some_and(Result::is_err),
            "only refused placement can retry"
        );
        self.phase = *self
            .placement_retry
            .take()
            .expect("a placement was refused");
    }
    pub fn pick(&self, world: &ToolWorld, cursor: WorldPoint) -> Pick {
        self.placement_input
            .resolve_with(cursor, |at| {
                let snap = world.scene.snap_at(at, world.view_scale, world.tolerance);
                crate::placement::PlacedPick {
                    cursor: at,
                    pos: snap.as_ref().map_or(at, |s| s.snapped_pos),
                    snap,
                }
            })
            .selected()
    }
    /// Dimension sides retain the first corner's role; switching the anchor
    /// to the center and back does not redefine it.
    pub fn first_corner(&self) -> Corner {
        let dims = self.draft().expect("anchor is edited in a draft").dims;
        match (dims.height_side, dims.width_side) {
            (HeightSide::Right, WidthSide::Top) => Corner::BottomLeft,
            (HeightSide::Left, WidthSide::Top) => Corner::BottomRight,
            (HeightSide::Left, WidthSide::Bottom) => Corner::TopRight,
            (HeightSide::Right, WidthSide::Bottom) => Corner::TopLeft,
        }
    }

    /// Numeric placement uses the same parameter/session path as pointer placement.
    pub fn begin_numeric(&mut self, world: &ToolWorld) -> Effect {
        assert!(
            self.draft().is_none(),
            "numeric placement starts without a draft"
        );
        let mut shape = placement_shape(WorldPoint::new(0.0, 0.0), WorldPoint::new(40.0, 20.0));
        if self.placement.rotated() {
            shape.frame = RectFrame::Rotated(sketch_geometry::types::Angle(0.0));
        }
        if self.placement.is_centered() {
            shape.center = WorldPoint::new(0.0, 0.0);
        }
        // No placement clicks, so there are no bindings.
        self.open_session(
            world,
            shape,
            WorldVec::new(40.0, 20.0),
            RectBindings::default(),
        )
    }

    /// One gesture step with supplied geometry, without mutating a sketch.
    pub fn step(&mut self, world: &ToolWorld, ev: ToolEvent) -> RectStep {
        self.refresh_sources(world.scene);
        match ev {
            ToolEvent::Cancel => self.cancel(),
            ToolEvent::Commit => self.commit(),
            ToolEvent::Click { cursor } => self.click(world, cursor),
            ToolEvent::Press { cursor } => self.press(world, cursor),
            ToolEvent::Drag { cursor } => self.drag(world, cursor),
            ToolEvent::Release { cursor } => self.release(world, cursor),
            ToolEvent::MoveTo { cursor } | ToolEvent::Tick { cursor } => RectStep {
                effect: None,
                preview: Some(self.preview(world, cursor)),
                request: ToolRequest::None,
            },
            ToolEvent::External => RectStep::quiet(),
        }
    }

    /// A projected host or recorded command supplies its already selected pick.
    /// Handle drags do not snap and keep using `step`.
    pub fn click_with_pick(&mut self, world: &ToolWorld, pick: Pick) -> RectStep {
        self.refresh_sources(world.scene);
        self.click_resolved(world, pick)
    }

    /// Current session parameters for panel display and edits, when active.
    pub fn draft(&self) -> Option<&RectDraft> {
        match &self.phase {
            RectPhase::Draft { draft, .. } => Some(draft),
            RectPhase::Idle | RectPhase::Placing { .. } | RectPhase::PlacingSide { .. } => None,
        }
    }

    /// Change placement mode through the panel.
    ///
    /// Drop unfinished placement rather than reinterpret a corner or side from
    /// another mode. It has not changed the model. An active draft keeps its
    /// gesture-selected frame; the new mode applies to the next placement.
    pub fn set_placement(&mut self, placement: RectPlacement) {
        if self.placement == placement {
            return;
        }
        self.placement = placement;
        if matches!(
            self.phase,
            RectPhase::Placing { .. } | RectPhase::PlacingSide { .. }
        ) {
            self.phase = RectPhase::Idle;
        }
    }

    /// Current session bindings, or None when there is no session.
    pub fn bindings(&self) -> Option<&RectBindings> {
        match &self.phase {
            RectPhase::Draft { bindings, .. } => Some(bindings),
            RectPhase::Idle | RectPhase::Placing { .. } | RectPhase::PlacingSide { .. } => None,
        }
    }

    /// Single panel entry point for session parameter edits.
    ///
    /// Return the same rebuild effect as handle dragging. None means no session
    /// or unchanged parameters, avoiding needless per-frame recalculation.
    pub fn set_draft(&mut self, next: RectDraft) -> Option<Effect> {
        let RectPhase::Draft {
            draft, bindings, ..
        } = &mut self.phase
        else {
            return None;
        };
        if *draft == next {
            return None;
        }
        *bindings = bindings.carried(draft, &next);
        *draft = next;
        self.wall = next.wall;
        self.application = None;
        Some(Effect::RectDraft {
            draft: next,
            bindings: bindings.clone(),
        })
    }

    /// Whether a logical draft is active, for host undo/redo routing.
    pub fn session_alive(&self) -> bool {
        matches!(self.phase, RectPhase::Draft { .. })
    }

    /// Whether the pointer holds a handle, selecting Drag versus MoveTo routing.
    pub fn is_dragging(&self) -> bool {
        matches!(self.phase, RectPhase::Draft { drag: Some(_), .. })
    }

    /// Includes unfinished placement before [`Self::session_alive`]; leaving
    /// must discard a first corner even though it has not changed the model.
    pub fn gesture_alive(&self) -> bool {
        !matches!(self.phase, RectPhase::Idle)
    }

    /// Leaving the tool cancels its active draft.
    ///
    /// A tool switch discards an uncommitted draft without a confirmation dialog.
    pub fn deactivate(&mut self) -> Option<Effect> {
        // Application status belongs to this session and must not leak into the next.
        self.application = None;
        self.placement_retry = None;
        match std::mem::take(&mut self.phase) {
            RectPhase::Idle | RectPhase::Placing { .. } | RectPhase::PlacingSide { .. } => None,
            RectPhase::Draft { .. } => Some(Effect::DraftCancel),
        }
    }

    fn cancel(&mut self) -> RectStep {
        match std::mem::take(&mut self.phase) {
            // Nothing to cancel: Escape leaves the tool.
            RectPhase::Idle | RectPhase::Placing { .. } | RectPhase::PlacingSide { .. } => {
                RectStep {
                    effect: None,
                    preview: None,
                    request: ToolRequest::Deactivate,
                }
            }
            RectPhase::Draft { .. } => {
                // The host's draft workflow decides whether cancellation
                // returns to selection or leaves the tool active.
                RectStep {
                    effect: Some(Effect::DraftCancel),
                    preview: None,
                    request: ToolRequest::None,
                }
            }
        }
    }

    fn commit(&mut self) -> RectStep {
        match std::mem::take(&mut self.phase) {
            // No draft exists before placement, so there is nothing to commit.
            phase @ (RectPhase::Idle
            | RectPhase::Placing { .. }
            | RectPhase::PlacingSide { .. }) => {
                self.phase = phase;
                RectStep::quiet()
            }
            RectPhase::Draft {
                draft,
                bindings,
                drag,
            } => {
                if !matches!(self.application, Some(Ok(_))) {
                    // Application is authoritative; a refusal leaves parameters editable.
                    self.phase = RectPhase::Draft {
                        draft,
                        bindings,
                        drag,
                    };
                    return RectStep::quiet();
                }
                RectStep {
                    effect: Some(Effect::DraftCommit),
                    preview: None,
                    request: ToolRequest::None,
                }
            }
        }
    }

    /// Explicit host acknowledgement; no model is inferred from the parameters.
    pub fn applied(&mut self, outcome: Result<Applied, Rejection>) {
        assert!(
            self.session_alive(),
            "rectangle acknowledgement requires a draft"
        );
        self.application = Some(outcome);
    }

    fn click(&mut self, world: &ToolWorld, cursor: WorldPoint) -> RectStep {
        let pick = self.pick(world, cursor);
        self.click_resolved(world, pick)
    }

    fn click_resolved(&mut self, world: &ToolWorld, pick: Pick) -> RectStep {
        let preview_pick = pick.clone();
        let mut effect = None;

        let min = self.placement_input.minimum(min_size(world.view_scale));
        match &self.phase {
            RectPhase::Idle => {
                self.phase = RectPhase::Placing { start: pick };
            }
            RectPhase::Placing { start } => {
                match self.placement {
                    RectPlacement::TwoCorners | RectPlacement::CenterCorner => {
                        let shape = if self.placement.is_centered() {
                            center_corner_shape(start.pos, pick.pos)
                        } else {
                            placement_shape(start.pos, pick.pos)
                        };
                        // Label sides retain the first corner's role, supplied
                        // by the second click in centered placement.
                        let delta = if self.placement.is_centered() {
                            start.pos - pick.pos
                        } else {
                            pick.pos - start.pos
                        };
                        let first = gesture_corner(delta);
                        let bindings = if self.placement.is_centered() {
                            RectBindings::placed([
                                (RectSite::Center, start),
                                (RectSite::Corner(first), &pick),
                            ])
                        } else {
                            RectBindings::placed([
                                (RectSite::Corner(first), start),
                                (RectSite::Corner(first.opposite()), &pick),
                            ])
                        };
                        effect = self.try_open_session(world, shape, delta, bindings);
                    }
                    RectPlacement::ThreeCorners | RectPlacement::CenterTwoCorners => {
                        // The second click defines a side or center-to-corner radius;
                        // below the screen threshold it has no usable direction.
                        if start.pos.distance(pick.pos) > min {
                            self.phase = RectPhase::PlacingSide {
                                start: start.clone(),
                                edge_end: pick,
                            };
                        }
                    }
                }
            }
            RectPhase::PlacingSide { start, edge_end } => {
                let Some(shape) = self.side_shape(start.pos, edge_end.pos, pick.pos) else {
                    return RectStep {
                        effect: None,
                        preview: Some(self.preview_with_pick(preview_pick)),
                        request: ToolRequest::None,
                    };
                };
                // Label sides follow frame axes: width follows the first side,
                // height the third click's side.
                let delta = if self.placement.is_centered() {
                    shape.frame.to_local(start.pos - edge_end.pos)
                } else {
                    WorldVec::new(
                        shape.frame.to_local(edge_end.pos - start.pos).0.x,
                        shape.frame.to_local(pick.pos - edge_end.pos).0.y,
                    )
                };
                // Only the first two clicks are figure points: center/corner or
                // side endpoints. The third supplies a projection or direction.
                let first = gesture_corner(delta);
                let bindings = if self.placement.is_centered() {
                    RectBindings::placed([
                        (RectSite::Center, start),
                        (RectSite::Corner(first), edge_end),
                    ])
                } else {
                    RectBindings::placed([
                        (RectSite::Corner(first), start),
                        (RectSite::Corner(first.across_width()), edge_end),
                    ])
                };
                effect = self.try_open_session(world, shape, delta, bindings);
            }
            // A click presses and releases without moving a handle, so no rebuild.
            RectPhase::Draft { .. } => {}
        }

        RectStep {
            effect,
            preview: Some(self.preview_with_pick(preview_pick)),
            request: ToolRequest::None,
        }
    }

    /// Final click: open a session if the figure meets the threshold.
    /// None rejects the click while retaining the placement phase.
    ///
    /// All modes judge the resulting figure, independently of click count.
    fn try_open_session(
        &mut self,
        world: &ToolWorld,
        shape: RectDraft,
        gesture: WorldVec,
        bindings: RectBindings,
    ) -> Option<Effect> {
        let min = self.placement_input.minimum(min_size(world.view_scale));
        (shape.width() > min && shape.height() > min)
            .then(|| self.open_session(world, shape, gesture, bindings))
    }

    /// Open a logical session with the gesture's parameters.
    ///
    /// gesture points from the first corner into the local frame, selecting
    /// label sides and retaining its role even when starting from center.
    ///
    fn open_session(
        &mut self,
        world: &ToolWorld,
        shape: RectDraft,
        gesture: WorldVec,
        bindings: RectBindings,
    ) -> Effect {
        self.numeric_anchor = if self.placement.is_centered() {
            RectAnchor::Center
        } else {
            RectAnchor::Corner(gesture_corner(gesture))
        };
        let dims = RectDimPlacement {
            // Place dimensions opposite the first chosen corner.
            width_side: if gesture.0.y >= 0.0 {
                WidthSide::Top
            } else {
                WidthSide::Bottom
            },
            height_side: if gesture.0.x >= 0.0 {
                HeightSide::Right
            } else {
                HeightSide::Left
            },
            // New label offsets are positive (pixels × scale); *_side carries direction.
            offset: world.label_offset,
        };
        let draft = RectDraft {
            dims,
            wall: self.wall,
            ..shape
        };
        self.application = None;
        self.placement_retry = Some(Box::new(self.phase.clone()));
        self.phase = RectPhase::Draft {
            draft,
            bindings: bindings.clone(),
            drag: None,
        };
        Effect::RectDraft { draft, bindings }
    }

    fn press(&mut self, world: &ToolWorld, cursor: WorldPoint) -> RectStep {
        if let RectPhase::Draft { draft, drag, .. } = &mut self.phase {
            // Resolve capture once at press time; later steps only move that handle.
            *drag = grabbed_handle(draft, cursor, handle_hit_radius(world.view_scale));
        }
        RectStep {
            effect: None,
            preview: Some(self.preview(world, cursor)),
            request: ToolRequest::None,
        }
    }

    fn drag(&mut self, world: &ToolWorld, cursor: WorldPoint) -> RectStep {
        let mut effect = None;
        if let RectPhase::Draft {
            draft,
            bindings,
            drag,
        } = &mut self.phase
            && let Some(held) = *drag
        {
            let moved_corner = cursor - held.grab_offset;
            // The opposite corner stays fixed, read from capture state rather
            // than current parameters.
            let anchor = held.anchor;
            // Measure in frame axes; world projections do not measure rotated sides.
            let size = draft.frame.to_local(moved_corner - anchor).0.abs();
            let min = min_size(world.view_scale).0;
            if size.x > min && size.y > min {
                // Retain the session frame: dragging resizes without rotating.
                let next = RectDraft {
                    wall: draft.wall,
                    ..RectDraft::from_diagonal(
                        moved_corner,
                        anchor,
                        draft.frame,
                        draft.corner_mode,
                        draft.corner_size,
                        draft.dims,
                    )
                };
                // Only the fixed corner's binding survives through carried.
                *bindings = bindings.carried(draft, &next);
                *draft = next;
                self.application = None;
                effect = Some(Effect::RectDraft {
                    draft: next,
                    bindings: bindings.clone(),
                });
            }
        }
        RectStep {
            effect,
            preview: Some(self.preview(world, cursor)),
            request: ToolRequest::None,
        }
    }

    fn release(&mut self, world: &ToolWorld, cursor: WorldPoint) -> RectStep {
        if let RectPhase::Draft { drag, .. } = &mut self.phase {
            // Release keeps the draft alive until Enter/Escape. The previous
            // Drag calculated the result; do not recompute at release.
            *drag = None;
        }
        RectStep {
            effect: None,
            preview: Some(self.preview(world, cursor)),
            request: ToolRequest::None,
        }
    }

    /// Preview shape for the current phase.
    ///
    /// Shared with [`Self::step`] so preview picks and corners agree with application.
    pub fn preview(&self, world: &ToolWorld, cursor: WorldPoint) -> RectPreview {
        self.preview_in_scene(world.scene, self.pick(world, cursor))
    }

    /// Preview consumes the same pick as placement; it never runs another query.
    pub fn preview_with_pick(&self, pick: Pick) -> RectPreview {
        match &self.phase {
            RectPhase::Idle => RectPreview::Cursor { pick },
            RectPhase::Placing { start } => {
                let end = pick;
                match self.placement {
                    RectPlacement::TwoCorners | RectPlacement::CenterCorner => {
                        // Use the plan that the second click will apply.
                        let shape = if self.placement.is_centered() {
                            center_corner_shape(start.pos, end.pos)
                        } else {
                            placement_shape(start.pos, end.pos)
                        };
                        let shape = RectDraft {
                            wall: self.wall,
                            ..shape
                        };
                        RectPreview::Rubber {
                            start: start.pos,
                            end,
                            corners: shape.corners(),
                            inner: shape.inner_corners(),
                        }
                    }
                    RectPlacement::ThreeCorners | RectPlacement::CenterTwoCorners => {
                        RectPreview::Edge {
                            start: start.pos,
                            end,
                        }
                    }
                }
            }
            RectPhase::PlacingSide { start, edge_end } => {
                let end = pick;
                // The same figure the third click will open.
                let Some(shape) = self.side_shape(start.pos, edge_end.pos, end.pos) else {
                    return RectPreview::Edge {
                        start: start.pos,
                        end,
                    };
                };
                let shape = RectDraft {
                    wall: self.wall,
                    ..shape
                };
                RectPreview::Rubber {
                    start: start.pos,
                    end,
                    corners: shape.corners(),
                    inner: shape.inner_corners(),
                }
            }
            RectPhase::Draft { draft, drag, .. } => RectPreview::Handles {
                corners: draft.corners(),
                grabbed: drag.map(|held| held.corner),
            },
        }
    }

    /// Shared third-click resolution. In centered mode the cursor defines
    /// the neighbor's direction, not its radius.
    fn side_shape(
        &self,
        start: WorldPoint,
        corner: WorldPoint,
        toward: WorldPoint,
    ) -> Option<RectDraft> {
        if self.placement == RectPlacement::CenterTwoCorners {
            center_two_corner_shape(start, corner, toward)
        } else {
            Some(three_corner_shape(start, corner, toward))
        }
    }
}

fn center_corner_shape(center: WorldPoint, corner: WorldPoint) -> RectDraft {
    let opposite = center - (corner - center);
    RectDraft {
        center,
        ..placement_shape(opposite, corner)
    }
}

/// Project the second corner direction onto the first corner's circumcircle;
/// reflect the first in the center for its opposite. Adjacent chords are
/// perpendicular by Thales' theorem. Zero directions and equal/opposite corners
/// leave the gesture unfinished.
fn center_two_corner_shape(
    center: WorldPoint,
    corner: WorldPoint,
    toward: WorldPoint,
) -> Option<RectDraft> {
    let radius = corner.distance(center).0;
    let direction = (toward - center).0;
    if radius <= WorldLength::DEGENERATE.0 || direction.length() <= WorldLength::DEGENERATE.0 {
        return None;
    }
    let next = center + WorldVec::from_dvec2(direction.normalize() * radius);
    let opposite = center - (corner - center);
    if next.distance(corner) <= WorldLength::DEGENERATE
        || next.distance(opposite) <= WorldLength::DEGENERATE
    {
        return None;
    }
    Some(RectDraft {
        center,
        ..three_corner_shape(corner, next, opposite)
    })
}

/// Two-diagonal-corner shape shared by preview and application.
///
/// start_dims is arbitrary: geometry does not depend on dimension placement.
/// open_session chooses the actual sides from the final gesture.
pub fn placement_shape(start: WorldPoint, end: WorldPoint) -> RectDraft {
    RectDraft::from_diagonal(
        start,
        end,
        RectFrame::AxisAligned,
        RectCornerMode::None,
        WorldLength::ZERO,
        start_dims(),
    )
}

/// ThreeCorners counterpart to [`placement_shape`], shared by preview and application.
pub fn three_corner_shape(p0: WorldPoint, p1: WorldPoint, p2: WorldPoint) -> RectDraft {
    RectDraft::from_three_corners(
        p0,
        p1,
        p2,
        RectCornerMode::None,
        WorldLength::ZERO,
        start_dims(),
    )
}

/// Dimension placement for the placement preview.
///
/// Preview draws no dimensions and its corners do not depend on their placement.
/// open_session chooses the actual sides from the gesture.
fn start_dims() -> RectDimPlacement {
    RectDimPlacement {
        width_side: WidthSide::Top,
        height_side: HeightSide::Right,
        offset: WorldLength::ZERO,
    }
}

/// Initial corner role from local gesture direction: right/up means bottom-left.
/// One table serves the panel anchor and placement bindings.
fn gesture_corner(gesture: WorldVec) -> Corner {
    match (gesture.0.x >= 0.0, gesture.0.y >= 0.0) {
        (true, true) => Corner::BottomLeft,
        (false, true) => Corner::BottomRight,
        (false, false) => Corner::TopRight,
        (true, false) => Corner::TopLeft,
    }
}

/// Nearest handle within tolerance, independent of iteration order when hit
/// regions overlap on a small rectangle.
fn grabbed_handle(
    draft: &RectDraft,
    cursor: WorldPoint,
    hit_radius: WorldLength,
) -> Option<HandleDrag> {
    Corner::ALL
        .into_iter()
        .map(|corner| (corner, draft.corner(corner)))
        .filter(|(_, pos)| cursor.distance(*pos) < hit_radius)
        .min_by(|(_, lhs), (_, rhs)| {
            cursor
                .distance(*lhs)
                .partial_cmp(&cursor.distance(*rhs))
                .expect("corner positions must be finite: gesture input rejects NaN")
        })
        .map(|(corner, pos)| HandleDrag {
            corner,
            grab_offset: cursor - pos,
            anchor: draft.corner(corner.opposite()),
        })
}
