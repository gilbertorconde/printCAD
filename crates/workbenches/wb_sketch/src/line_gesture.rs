//! Line runtime adapter. Gesture and binding policy live in sketch_runtime;
//! this module translates addresses, geometry and history receipts.

use sketch_geometry::world::{WorldLength, WorldPerPx, WorldPoint};
use sketch_runtime::direction::{ToolAxis, axis_between};
use sketch_runtime::geometry::SourceId;
use sketch_runtime::line::{
    Alignment, Applied as LineApplied, Event, LineHistoryId, LinePhase, LinePlan, LineTool,
    SegmentStart,
};
use sketch_runtime::pick::{Pick, Source};
use sketch_runtime::point::{PointPlan, Rejection};
use sketch_runtime::polyline::{ArcRelation, PolylineCurve, PolylineMode};
use uuid::Uuid;

use crate::pick_projection::SelectPick;
use crate::sketch::{
    Arc, ConstraintKind, GeometryElement, Line, ORIGIN_ID, Point, Sketch, Vec2D, X_AXIS_ID,
    Y_AXIS_ID,
};
use crate::snap::{Snap, SnapKind, SnapTarget};
use crate::tools::{ToolEffect, ToolState};

#[derive(Debug, Default)]
pub(crate) struct LineGesture {
    pub picker: SelectPick,
    pub tool: LineTool,
    pending: Option<LineApplied>,
    active: bool,
    /// The dedicated chain tool keeps the chosen mode across links.
    pub repeat_mode: bool,
}

pub(crate) fn world(p: Vec2D) -> WorldPoint {
    WorldPoint::new(p.x, p.y)
}
pub(crate) fn host(p: WorldPoint) -> Vec2D {
    Vec2D::new(p.0.x, p.0.y)
}

impl LineGesture {
    pub fn leave(&mut self) {
        self.tool.deactivate();
        self.active = false;
    }

    pub fn begin_arc(&mut self, state: &mut ToolState) -> bool {
        if !self.active || !self.tool.can_arc() {
            return false;
        }
        self.tool.mode = PolylineMode::Tangent;
        self.update_state(state);
        true
    }

    pub fn cycle_mode(&mut self, state: &mut ToolState) -> bool {
        if !self.active || !self.tool.can_arc() {
            return false;
        }
        self.tool.mode = self.tool.mode.next();
        self.update_state(state);
        true
    }

    pub fn sync(&mut self, sketch: &Sketch, state: &ToolState) {
        self.picker.sync(sketch);
        if state.is_idle() && self.pending.is_none() {
            self.tool.deactivate();
        }
        self.tool.refresh_start(&self.picker.geometry);
    }

    pub fn external(&mut self, sketch: &Sketch, state: &mut ToolState, scale: f64) {
        self.picker.sync(sketch);
        if !self.active {
            return;
        }
        self.tool.step(
            &self.picker.geometry,
            WorldPerPx::new(scale),
            Event::External,
            None,
        );
        self.update_state(state);
    }

    pub fn update_state(&self, state: &mut ToolState) {
        *state = match &self.tool.phase {
            LinePhase::Idle => ToolState::Idle,
            LinePhase::Placing { start }
                if self.repeat_mode || self.tool.mode != PolylineMode::Line =>
            {
                let previous = self.tool.previous_link();
                let heading = previous.map(|previous| {
                    let SegmentStart::Vertex { point, .. } = start else {
                        unreachable!("a continuing chain has a vertex")
                    };
                    sketch_runtime::polyline::heading_into(&self.picker.geometry, previous, *point)
                });
                ToolState::PolylineFrom {
                    from: match start {
                        SegmentStart::Fresh(pick) => self.host_pick(pick).placement(),
                        SegmentStart::Vertex { point, .. } => {
                            SnapTarget::Existing(Uuid::from_u128(point.0))
                        }
                    },
                    first: None,
                    heading: heading.map(|h| Vec2D::new(h.0.x, h.0.y)),
                    prev: previous.map(|id| Uuid::from_u128(id.0)),
                    segment: match self.tool.mode {
                        PolylineMode::Tangent => crate::tools::PolySegment::Tangent,
                        PolylineMode::Perpendicular => crate::tools::PolySegment::Perpendicular,
                        PolylineMode::Reverse => crate::tools::PolySegment::Reverse,
                        PolylineMode::Line => crate::tools::PolySegment::Line,
                    },
                }
            }
            LinePhase::Placing { start } => ToolState::LineFrom {
                from: match start {
                    SegmentStart::Fresh(pick) => self.host_pick(pick).placement(),
                    SegmentStart::Vertex { point, .. } => {
                        SnapTarget::Existing(Uuid::from_u128(point.0))
                    }
                },
                chain: matches!(start, SegmentStart::Vertex { .. }),
            },
            LinePhase::AwaitingCommit => return,
        };
    }

    pub fn pick(&mut self, sketch: &Sketch, state: &ToolState, cursor: Vec2D, scale: f64) -> Pick {
        self.sync(sketch, state);
        self.tool.pick(
            &self.picker.geometry,
            world(cursor),
            WorldPerPx::new(scale),
            SelectPick::contact_tolerance(),
        )
    }

    pub fn host_pick(&self, pick: &Pick) -> Snap {
        let Some(hit) = &pick.snap else {
            return Snap::free(host(pick.pos));
        };
        let pos = host(pick.pos);
        if hit.source.source == Source::Guide {
            let start = match &self.tool.phase {
                LinePhase::Placing { start } => start.pos(),
                _ => pick.pos,
            };
            let kind = match axis_between(start, pick.pos) {
                Some(ToolAxis::Horizontal) => Some(SnapKind::Horizontal),
                Some(ToolAxis::Vertical) => Some(SnapKind::Vertical),
                _ => None,
            };
            return Snap {
                kind,
                on: hit.source2.map(|s| SelectPick::landing_kind(s.source)),
                curve2: hit.source2.map(|s| SelectPick::source_uuid(s.source)),
                ..Snap::free(pos)
            };
        }
        self.picker
            .host_geometric_snap(&sketch_runtime::snap::SnapResult {
                snapped_pos: pick.pos,
                distance: WorldLength::ZERO,
                kind: hit.kind,
                priority: 0,
                source: hit.source,
                source2: hit.source2,
            })
    }
}

impl SelectPick {
    pub(crate) fn runtime_recorded_pick(
        &self,
        snap: Snap,
        cursor: Vec2D,
    ) -> sketch_runtime::pick::Pick {
        use sketch_runtime::pick::{Axis, PointRole, SnapKind as K, Source, SourceRef};
        let source = |id: Uuid| match id {
            ORIGIN_ID => Source::Origin,
            X_AXIS_ID => Source::Axis(Axis::Horizontal),
            Y_AXIS_ID => Source::Axis(Axis::Vertical),
            _ => Source::Geometry(SourceId(id.as_u128())),
        };
        let hit = snap.kind.map(|kind| {
            let (kind, target, role) = match kind {
                SnapKind::Endpoint | SnapKind::Center => (
                    if kind == SnapKind::Center {
                        K::Center
                    } else {
                        K::Endpoint
                    },
                    source(match snap.target {
                        SnapTarget::Existing(id) => id,
                        _ => unreachable!(
                            "endpoint and center snaps must identify an existing point"
                        ),
                    }),
                    PointRole::Edge,
                ),
                SnapKind::Origin => (K::Endpoint, Source::Origin, PointRole::Edge),
                SnapKind::Horizontal | SnapKind::Vertical => {
                    (K::OnAxis, Source::Guide, PointRole::Edge)
                }
                SnapKind::Midpoint => (K::Midpoint, source(snap.curve.unwrap()), PointRole::Mid),
                SnapKind::Quadrant(q) => (
                    K::Quadrant,
                    source(snap.curve.unwrap()),
                    PointRole::Quadrant(q),
                ),
                other => (
                    match other {
                        SnapKind::Intersection => K::Intersection,
                        SnapKind::OnCurve => K::OnCurve,
                        SnapKind::OnAxis => K::OnAxis,
                        SnapKind::Parallel => K::Parallel,
                        SnapKind::Perpendicular => K::Perpendicular,
                        SnapKind::PerpendicularTo => K::PerpendicularTo,
                        SnapKind::OnExtension => K::OnExtension,
                        SnapKind::Tangent => K::Tangent,
                        _ => {
                            unreachable!("point and axis snaps must be handled before curve snaps")
                        }
                    },
                    source(snap.curve.unwrap()),
                    PointRole::Edge,
                ),
            };
            sketch_runtime::pick::Snap {
                kind,
                source: SourceRef {
                    source: target,
                    role,
                },
                source2: snap.curve2.map(|id| SourceRef {
                    source: source(id),
                    role: PointRole::Edge,
                }),
            }
        });
        sketch_runtime::pick::Pick {
            cursor: world(cursor),
            pos: world(snap.pos),
            snap: hit,
        }
    }
}

impl LineGesture {
    pub fn click(
        &mut self,
        sketch: &mut Sketch,
        state: &mut ToolState,
        cursor: Vec2D,
        scale: f64,
        picked: Option<Snap>,
    ) -> (ToolEffect, Snap) {
        self.sync(sketch, state);
        self.active = true;
        let pick = picked.map_or_else(
            || {
                self.tool.pick(
                    &self.picker.geometry,
                    world(cursor),
                    WorldPerPx::new(scale),
                    SelectPick::contact_tolerance(),
                )
            },
            |snap| self.picker.runtime_recorded_pick(snap, cursor),
        );
        // Record before the machine enters AwaitingCommit (tool axes refer to its start).
        let snap = self.host_pick(&pick);
        let step = self.tool.step(
            &self.picker.geometry,
            WorldPerPx::new(scale),
            Event::Click,
            Some(pick),
        );
        let Some(plan) = step.plan else {
            self.update_state(state);
            return (ToolEffect::none(), snap);
        };
        let applied = match self.apply_plan(sketch, &plan) {
            Ok(applied) => applied,
            Err(reason) => {
                self.tool.applied(&self.picker.geometry, Err(reason), None);
                self.update_state(state);
                return (ToolEffect::log("Line source is unavailable"), snap);
            }
        };
        self.pending = Some(applied);
        // This is only a view for numeric fields and host painters; the machine
        // receives its authoritative acknowledgement after solve and history.
        *state = ToolState::LineFrom {
            from: SnapTarget::Existing(Uuid::from_u128(applied.b.0)),
            chain: true,
        };
        (ToolEffect::changed("Line"), snap)
    }

    /// Resolve every source and relation before the first host write. The
    /// runtime projection supplies geometry; it never applies a document edit.
    fn apply_plan(&self, sketch: &mut Sketch, plan: &LinePlan) -> Result<LineApplied, Rejection> {
        let existing = |id: SourceId| {
            let point = Uuid::from_u128(id.0);
            sketch
                .point_position(point)
                .ok_or(Rejection::MissingSource(id))?;
            Ok::<_, Rejection>(point)
        };
        let start = match &plan.start {
            SegmentStart::Fresh(pick) => Some(
                self.picker
                    .prepare_point(sketch, &PointPlan::new(pick.clone()))?,
            ),
            SegmentStart::Vertex { point, .. } => {
                existing(*point)?;
                None
            }
        };
        let a = match &plan.start {
            SegmentStart::Fresh(_) => start.as_ref().unwrap().id,
            SegmentStart::Vertex { point, .. } => existing(*point)?,
        };
        let end = if let Some(id) = plan.close {
            existing(id)?;
            None
        } else {
            Some(self.picker.prepare_point(sketch, &plan.end)?)
        };
        let b = match plan.close {
            Some(id) => existing(id)?,
            None => end.as_ref().unwrap().id,
        };
        let made = Uuid::new_v4();
        let mut constraints = Vec::new();
        let require_curve = |source: SourceId| {
            let id = Uuid::from_u128(source.0);
            sketch
                .geometry
                .iter()
                .find(|g| g.id() == id)
                .ok_or(Rejection::MissingSource(source))
        };
        match plan.curve {
            PolylineCurve::Line => {
                match plan.axis {
                    Some(ToolAxis::Horizontal) => {
                        constraints.push(ConstraintKind::Horizontal { element: made })
                    }
                    Some(ToolAxis::Vertical) => {
                        constraints.push(ConstraintKind::Vertical { element: made })
                    }
                    _ => {}
                }
                if let Some(alignment) = plan.alignment {
                    let (Alignment::Parallel(source)
                    | Alignment::Perpendicular(source)
                    | Alignment::Tangent(source)) = alignment;
                    require_curve(source)?;
                    let other = Uuid::from_u128(source.0);
                    constraints.push(match alignment {
                        Alignment::Parallel(_) => ConstraintKind::Parallel {
                            line1: made,
                            line2: other,
                        },
                        Alignment::Perpendicular(_) => ConstraintKind::Perpendicular {
                            line1: made,
                            line2: other,
                        },
                        Alignment::Tangent(_) => ConstraintKind::Tangent {
                            line_or_circle1: made,
                            item2: other,
                        },
                    });
                }
            }
            PolylineCurve::Arc(_) => {
                let previous = plan.previous.expect("arc has a preceding link");
                require_curve(previous)?;
                let other = Uuid::from_u128(previous.0);
                constraints.push(
                    match plan.relation.expect("arc plan declares its relation") {
                        ArcRelation::Perpendicular(angle) => ConstraintKind::AngleAtPoint {
                            curve1: other,
                            curve2: made,
                            point: a,
                            angle_rad: angle.0,
                        },
                        ArcRelation::TangentLine(_) | ArcRelation::TangentArc { .. } => {
                            ConstraintKind::Tangent {
                                line_or_circle1: other,
                                item2: made,
                            }
                        }
                    },
                );
            }
        }
        if let Some(start) = start {
            start.apply(sketch);
        }
        if let Some(end) = end {
            end.apply(sketch);
        }
        match plan.curve {
            PolylineCurve::Line => {
                sketch.add_geometry(GeometryElement::Line(Line {
                    id: made,
                    start: a,
                    end: b,
                }));
            }
            PolylineCurve::Arc(arc) => {
                let center = Uuid::new_v4();
                sketch.add_geometry(GeometryElement::Point(Point {
                    id: center,
                    position: host(arc.center),
                }));
                let (start, end) = if arc.clockwise { (b, a) } else { (a, b) };
                sketch.add_geometry(GeometryElement::Arc(Arc {
                    id: made,
                    center,
                    start,
                    end,
                    radius: arc.center.distance(arc.start).0,
                }));
            }
        }
        for kind in constraints {
            sketch.add_constraint(kind);
        }
        Ok(LineApplied {
            curve: SourceId(made.as_u128()),
            a: SourceId(a.as_u128()),
            b: SourceId(b.as_u128()),
        })
    }
}

impl LineGesture {
    pub fn committed(
        &mut self,
        sketch: &Sketch,
        state: &mut ToolState,
        entry: Option<core_document::history::JournalEntryId>,
    ) {
        let Some(applied) = self.pending.take() else {
            return;
        };
        self.picker.sync(sketch);
        self.tool.applied(
            &self.picker.geometry,
            Ok(applied),
            entry.map(|id| LineHistoryId(id.0.as_u128())),
        );
        if !self.repeat_mode {
            self.tool.mode = PolylineMode::Line;
        }
        self.update_state(state);
    }

    pub fn can_undo(&self, entry: core_document::history::JournalEntryId) -> bool {
        self.tool
            .can_undo_in_history(LineHistoryId(entry.0.as_u128()))
    }
    pub fn undone(
        &mut self,
        sketch: &Sketch,
        state: &mut ToolState,
        entry: core_document::history::JournalEntryId,
    ) {
        self.picker.sync(sketch);
        self.tool
            .undone_in_history(&self.picker.geometry, LineHistoryId(entry.0.as_u128()));
        self.update_state(state);
    }
}

impl crate::SketchWorkbench {
    pub(super) fn is_chain_tool(tool: &str) -> bool {
        matches!(tool, "sketch.line" | "sketch.polyline")
    }

    pub(super) fn line_placement(
        &self,
        ctx: &core_document::WorkbenchRuntimeContext,
        feature: &crate::SketchFeature,
    ) -> sketch_runtime::placement::Placement {
        use sketch_runtime::placement::{GridSnap, Placement};
        Placement {
            snap_off: self.snap_off,
            auto_constraints_off: !self.options.auto_constraints,
            grid: (self.options.grid_on && self.options.grid_snap).then(|| {
                GridSnap::new(WorldLength::new(self.grid_step(
                    crate::overlay::SketchProjector::new(ctx, feature.plane).units_per_px(),
                )))
            }),
            ..Placement::default()
        }
    }

    pub(super) fn line_gesture_preview(
        &self,
        ctx: &core_document::WorkbenchRuntimeContext,
        feature: &crate::SketchFeature,
    ) -> Option<sketch_runtime::line::LinePreview> {
        let mut cursor = self.cursor?;
        let mut line = self.line_gesture.borrow_mut();
        line.tool.placement = self.line_placement(ctx, feature);
        let typed = self.dim_capture.typed(ctx.document);
        if !typed.is_empty() {
            cursor = self
                .dim_capture
                .steer(&self.tool_state, &feature.sketch, cursor);
            line.tool.placement.position = Some(world(crate::ovp::override_cursor(
                &self.tool_state,
                &feature.sketch,
                cursor,
                &typed,
            )));
        } else if ctx.shift_down
            && let Some(from) = crate::tools::snap_context(&self.tool_state, &feature.sketch).from
        {
            cursor = crate::snap::angle_step(from, cursor, crate::ANGLE_STEP_DEG);
        }
        let pick = line.pick(
            &feature.sketch,
            &self.tool_state,
            cursor,
            crate::overlay::SketchProjector::new(ctx, feature.plane).units_per_px(),
        );
        Some(line.tool.preview_with_pick(&line.picker.geometry, &pick))
    }

    pub(super) fn line_gesture_snap(
        &self,
        ctx: &core_document::WorkbenchRuntimeContext,
        feature: &crate::SketchFeature,
    ) -> Option<Snap> {
        let preview = self.line_gesture_preview(ctx, feature)?;
        Some(self.line_gesture.borrow().host_pick(preview.pick()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_line_plan_rejects_lost_sources_before_writes_and_uses_no_document_projection() {
        let mut sketch = Sketch::new("line application");
        let points: Vec<_> = [(20., 30.), (60., 70.), (20., 70.), (60., 30.)]
            .into_iter()
            .map(|(x, y)| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y)))))
            .collect();
        let a = sketch.add_geometry(GeometryElement::Line(Line::new(points[0], points[1])));
        let b = sketch.add_geometry(GeometryElement::Line(Line::new(points[2], points[3])));
        let mut line = LineGesture::default();
        let mut state = ToolState::Idle;
        line.sync(&sketch, &state);
        let scale = WorldPerPx::new(0.05);
        let pick = Pick {
            cursor: world(Vec2D::new(80., 90.)),
            pos: world(Vec2D::new(80., 90.)),
            snap: None,
        };
        line.tool
            .step(&line.picker.geometry, scale, Event::Click, Some(pick));
        let pick = line.tool.pick(
            &line.picker.geometry,
            world(Vec2D::new(40.1, 50.)),
            scale,
            SelectPick::contact_tolerance(),
        );
        let plan = line
            .tool
            .step(&line.picker.geometry, scale, Event::Click, Some(pick))
            .plan
            .unwrap();
        assert_eq!(
            plan.end.bindings.len(),
            2,
            "intersection retains both carriers"
        );
        for missing in [a, b] {
            let mut incomplete = sketch.clone();
            incomplete.geometry.retain(|g| g.id() != missing);
            let before = serde_json::to_value(&incomplete).unwrap();
            assert_eq!(
                line.apply_plan(&mut incomplete, &plan).unwrap_err(),
                Rejection::MissingSource(SourceId(missing.as_u128()))
            );
            assert_eq!(serde_json::to_value(&incomplete).unwrap(), before);
        }
        let applied = line.apply_plan(&mut sketch, &plan).unwrap();
        line.pending = Some(applied);
        line.committed(&sketch, &mut state, None);
        assert_eq!(
            sketch.point_position(Uuid::from_u128(applied.b.0)),
            Some(Vec2D::new(40., 50.))
        );
        let carriers: std::collections::HashSet<_> = sketch
            .constraints
            .iter()
            .filter_map(|c| match c.kind {
                ConstraintKind::PointOnLine { point, line }
                    if point == Uuid::from_u128(applied.b.0) =>
                {
                    Some(line)
                }
                _ => None,
            })
            .collect();
        assert_eq!(carriers, [a, b].into_iter().collect());
        assert!(
            !line.tool.can_undo_in_history(LineHistoryId(1)),
            "held script has no per-link receipt"
        );
    }
}
