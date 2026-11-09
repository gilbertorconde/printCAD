//! Point runtime over the stable geometry projection. The host translates
//! effects and keeps its own solve, numeric fields, document and history.

use sketch_geometry::world::WorldPerPx;
use sketch_runtime::binding::Binding;
use sketch_runtime::geometry::SourceId;
use sketch_runtime::pick::Axis;
use sketch_runtime::placement::PlacedPick;
use sketch_runtime::placement::Placement;
use sketch_runtime::point::{Applied, Event, PointPlan, PointTool, Rejection};
use uuid::Uuid;

use crate::line_gesture::{host, world};
use crate::pick_projection::SelectPick;
use crate::sketch::{
    ConstraintKind, GeometryElement, ORIGIN_ID, Point, Sketch, Vec2D, X_AXIS_ID, Y_AXIS_ID,
};
use crate::snap::Snap;
use crate::tools::ToolEffect;

/// Fully resolved host writes. Preparing several endpoints is atomic: no point
/// or constraint is inserted until every source has been checked.
pub(crate) struct PreparedPoint {
    pub id: Uuid,
    fresh: Option<sketch_geometry::world::WorldPoint>,
    constraints: Vec<ConstraintKind>,
}
impl PreparedPoint {
    pub(crate) fn apply(self, sketch: &mut Sketch) -> Uuid {
        if let Some(pos) = self.fresh {
            sketch.add_geometry(GeometryElement::Point(Point {
                id: self.id,
                position: host(pos),
            }));
        }
        for kind in self.constraints {
            sketch.add_constraint(kind);
        }
        self.id
    }
}

impl SelectPick {
    pub(crate) fn host_runtime_pick(&self, pick: &sketch_runtime::pick::Pick) -> Snap {
        pick.snap.as_ref().map_or_else(
            || Snap::free(host(pick.pos)),
            |hit| {
                self.host_geometric_snap(&sketch_runtime::snap::SnapResult {
                    snapped_pos: pick.pos,
                    distance: sketch_geometry::world::WorldLength::ZERO,
                    kind: hit.kind,
                    priority: 0,
                    source: hit.source,
                    source2: hit.source2,
                })
            },
        )
    }
    pub(crate) fn point_preview(
        &mut self,
        sketch: &Sketch,
        cursor: Vec2D,
        scale: f64,
        placement: Placement,
    ) -> PlacedPick {
        self.sync(sketch);
        let pick = self.point_pick(world(cursor), WorldPerPx::new(scale), placement);
        let step = PointTool::default().step(Event::Hover, Some(pick.selected()));
        assert_eq!(
            step.preview.expect("point hover has a preview").pos,
            pick.pos
        );
        pick
    }

    fn point_pick(
        &self,
        cursor: sketch_geometry::world::WorldPoint,
        scale: WorldPerPx,
        placement: Placement,
    ) -> PlacedPick {
        let extra = self.geometry.placement_objects(
            sketch_runtime::direction::DirectionQuery::Extensions,
            &placement,
        );
        placement.resolve_with(cursor, |at| {
            let snap = self
                .geometry
                .snap_candidates(
                    at,
                    sketch_runtime::snap::snap_tolerance(scale),
                    &extra,
                    Self::contact_tolerance(),
                )
                .into_iter()
                .min_by(sketch_runtime::scene::snap_cmp);
            PlacedPick {
                cursor: at,
                pos: snap.as_ref().map_or(at, |s| s.snapped_pos),
                snap,
            }
        })
    }

    pub(crate) fn host_point_pick(&self, pick: &PlacedPick) -> Snap {
        pick.snap.as_ref().map_or_else(
            || Snap::free(host(pick.pos)),
            |hit| self.host_geometric_snap(hit),
        )
    }

    pub(crate) fn point_click(&mut self, sketch: &mut Sketch, snap: Snap) -> ToolEffect {
        self.sync(sketch);
        let pick = self.runtime_recorded_pick(snap, snap.pos);
        let mut tool = PointTool::default();
        let step = tool.step(Event::Click, Some(pick));
        let before = sketch.geometry.len();
        let outcome = self.apply_point_plan(sketch, &step.plan.expect("point click has a plan"));
        tool.applied(outcome);
        match tool
            .application
            .expect("host acknowledges point application")
        {
            Ok(applied) => {
                assert!(
                    sketch
                        .point_position(Uuid::from_u128(applied.point.0))
                        .is_some()
                );
                if sketch.geometry.len() != before {
                    ToolEffect::changed("Point")
                } else {
                    ToolEffect::log("A point is already there")
                }
            }
            Err(_) => ToolEffect::log("Point source is unavailable"),
        }
    }

    /// Resolve all requirements before mutating the host. The projection is
    /// read-only: it supplies addresses, never a temporary applied document.
    fn apply_point_plan(
        &self,
        sketch: &mut Sketch,
        plan: &PointPlan,
    ) -> Result<Applied, Rejection> {
        let prepared = self.prepare_point(sketch, plan)?;
        Ok(Applied {
            point: SourceId(prepared.apply(sketch).as_u128()),
        })
    }

    pub(crate) fn prepare_point(
        &self,
        sketch: &Sketch,
        plan: &PointPlan,
    ) -> Result<PreparedPoint, Rejection> {
        let mut existing = None;
        let mut constraints = Vec::new();
        let point = Uuid::new_v4();
        let missing = |id| Rejection::MissingSource(id);
        for binding in &plan.bindings {
            let kind = match *binding {
                Binding::Coincident(source) => {
                    let sketch_runtime::pick::Source::Geometry(id) = source.source else {
                        continue;
                    };
                    let target = self
                        .geometry
                        .point_source(source)
                        .ok_or_else(|| missing(id))?;
                    let target = Uuid::from_u128(target.0);
                    sketch.point_position(target).ok_or_else(|| missing(id))?;
                    existing = Some(target);
                    continue;
                }
                Binding::Midpoint(source) => {
                    let id = Uuid::from_u128(source.0);
                    let Some(GeometryElement::Line(line)) =
                        sketch.geometry.iter().find(|g| g.id() == id)
                    else {
                        return Err(missing(source));
                    };
                    ConstraintKind::SymmetricAboutPoint {
                        point1: line.start,
                        point2: line.end,
                        center: point,
                    }
                }
                Binding::Quadrant { circle, quadrant } => {
                    let id = Uuid::from_u128(circle.0);
                    if !sketch
                        .geometry
                        .iter()
                        .any(|g| matches!(g, GeometryElement::Circle(c) if c.id == id))
                    {
                        return Err(missing(circle));
                    }
                    ConstraintKind::CircularPoint {
                        point,
                        circle: id,
                        angle_rad: f64::from(quadrant) * std::f64::consts::FRAC_PI_2,
                    }
                }
                Binding::OnCurve(source) => {
                    let id = Uuid::from_u128(source.0);
                    match sketch
                        .geometry
                        .iter()
                        .find(|g| g.id() == id)
                        .ok_or_else(|| missing(source))?
                    {
                        GeometryElement::Line(_) => ConstraintKind::PointOnLine { point, line: id },
                        GeometryElement::Circle(_) | GeometryElement::Arc(_) => {
                            ConstraintKind::PointOnCircle { point, circle: id }
                        }
                        GeometryElement::Ellipse(_)
                        | GeometryElement::Conic(_)
                        | GeometryElement::BSpline(_) => {
                            ConstraintKind::PointOnCurve { point, curve: id }
                        }
                        GeometryElement::Point(_) => continue,
                    }
                }
                Binding::OnAxis(axis) => ConstraintKind::PointOnLine {
                    point,
                    line: match axis {
                        Axis::Horizontal => X_AXIS_ID,
                        Axis::Vertical => Y_AXIS_ID,
                    },
                },
            };
            constraints.push(kind);
        }
        if let Some(point) = existing {
            return Ok(PreparedPoint {
                id: point,
                fresh: None,
                constraints: Vec::new(),
            });
        }
        let at_origin = plan.bindings.contains(&Binding::OnAxis(Axis::Horizontal))
            && plan.bindings.contains(&Binding::OnAxis(Axis::Vertical));
        if at_origin {
            constraints.retain(|kind| !matches!(kind, ConstraintKind::PointOnLine { line, .. } if *line == X_AXIS_ID || *line == Y_AXIS_ID));
            constraints.push(ConstraintKind::Coincident {
                point1: point,
                point2: ORIGIN_ID,
            });
        }
        Ok(PreparedPoint {
            id: point,
            fresh: Some(plan.pick.pos),
            constraints,
        })
    }
}

impl crate::SketchWorkbench {
    pub(super) fn point_placement_snap(
        &self,
        ctx: &core_document::WorkbenchRuntimeContext,
        feature: &crate::SketchFeature,
    ) -> Option<Snap> {
        let mut placement = self.point_placement(ctx, feature, self.cursor?);
        let typed = self.dim_capture.typed(ctx.document);
        let cursor = self.cursor?;
        if !typed.is_empty() {
            placement.position = Some(world(crate::ovp::override_cursor(
                &self.tool_state,
                &feature.sketch,
                cursor,
                &typed,
            )));
        }
        let mut picker = self.select_pick.borrow_mut();
        let preview = picker.point_preview(
            &feature.sketch,
            cursor,
            crate::overlay::SketchProjector::new(ctx, feature.plane).units_per_px(),
            placement,
        );
        Some(picker.host_point_pick(&preview))
    }

    /// Image edges supply a document-specific cursor position before the
    /// shared placement policy. Alt bypasses those edges.
    pub(super) fn point_placement(
        &self,
        ctx: &core_document::WorkbenchRuntimeContext,
        feature: &crate::SketchFeature,
        cursor: Vec2D,
    ) -> Placement {
        let mut placement = self.line_placement(ctx, feature);
        if !placement.snap_off
            && let Some((edge, _)) = self.edge_snap(
                ctx,
                feature,
                "sketch.point",
                cursor,
                Self::snap_tolerance(ctx, &feature.plane),
            )
        {
            let mut picker = self.select_pick.borrow_mut();
            let preview = picker.point_preview(
                &feature.sketch,
                cursor,
                crate::overlay::SketchProjector::new(ctx, feature.plane).units_per_px(),
                Placement {
                    grid: None,
                    ..placement
                },
            );
            if !preview.snap.as_ref().is_some_and(|s| s.kind.is_point()) {
                placement.position = Some(world(edge));
            }
        }
        placement
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::Line;

    #[test]
    fn host_point_application_keeps_projection_read_only_and_refuses_missing_sources_atomically() {
        let mut sketch = Sketch::new("point application");
        let points: Vec<_> = [(20., 30.), (60., 70.), (20., 70.), (60., 30.)]
            .into_iter()
            .map(|(x, y)| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y)))))
            .collect();
        let a = sketch.add_geometry(GeometryElement::Line(Line::new(points[0], points[1])));
        let b = sketch.add_geometry(GeometryElement::Line(Line::new(points[2], points[3])));
        let mut picker = SelectPick::default();
        picker.sync(&sketch);
        let pick = picker.point_pick(
            world(Vec2D::new(40.1, 50.)),
            WorldPerPx::new(0.05),
            Placement::default(),
        );
        let mut tool = PointTool::default();
        let plan = tool.step(Event::Click, Some(pick.selected())).plan.unwrap();
        let projection = picker
            .geometry
            .snap_at(
                pick.cursor,
                WorldPerPx::new(0.05),
                SelectPick::contact_tolerance(),
            )
            .unwrap();
        let mut incomplete = sketch.clone();
        incomplete.geometry.retain(|g| g.id() != b);
        let before = serde_json::to_value(&incomplete).unwrap();
        let refused = picker.apply_point_plan(&mut incomplete, &plan);
        tool.applied(refused);
        assert_eq!(
            tool.application,
            Some(Err(Rejection::MissingSource(SourceId(b.as_u128()))))
        );
        assert_eq!(serde_json::to_value(&incomplete).unwrap(), before);
        let applied = picker.apply_point_plan(&mut sketch, &plan).unwrap();
        tool.applied(Ok(applied));
        let point = Uuid::from_u128(applied.point.0);
        assert_eq!(sketch.point_position(point), Some(Vec2D::new(40., 50.)));
        let sources: std::collections::HashSet<_> = sketch
            .constraints
            .iter()
            .filter_map(|c| match c.kind {
                ConstraintKind::PointOnLine { point: p, line } if p == point => Some(line),
                _ => None,
            })
            .collect();
        assert_eq!(sources, [a, b].into_iter().collect());
        let after = picker
            .geometry
            .snap_at(
                pick.cursor,
                WorldPerPx::new(0.05),
                SelectPick::contact_tolerance(),
            )
            .unwrap();
        assert_eq!(
            (after.snapped_pos, after.source, after.source2, after.kind),
            (
                projection.snapped_pos,
                projection.source,
                projection.source2,
                projection.kind
            )
        );
    }
}
