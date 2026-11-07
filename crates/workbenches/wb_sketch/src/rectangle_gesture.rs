//! Rectangle placement over the geometry projection. The host accepts the
//! completed contour; numeric dimensions and document history stay with it.

use sketch_geometry::world::WorldPerPx;
use sketch_runtime::geometry::SourceId;
use sketch_runtime::point::Rejection;
use sketch_runtime::rect::{
    Applied, Effect, RectPhase, RectPlacement, RectPreview, RectTool, RectanglePlan, ToolEvent,
    ToolWorld,
};
use sketch_runtime::rect_shape::{Side, SideRelation};
use uuid::Uuid;

use crate::line_gesture::world;
use crate::pick_projection::SelectPick;
use crate::sketch::{ConstraintKind, GeometryElement, Line, Sketch, Vec2D};
use crate::snap::Snap;
use crate::tools::{ToolEffect, ToolState};

#[derive(Debug, Default)]
pub(crate) struct RectangleGesture {
    pub picker: SelectPick,
    pub tool: RectTool,
    /// Interactive placement keeps the runtime draft open for panel edits.
    pub editable: bool,
}

impl RectangleGesture {
    pub fn supports(tool: &str) -> bool {
        matches!(
            tool,
            "sketch.rect"
                | "sketch.rect_center"
                | "sketch.rect3"
                | "sketch.rect_center3"
                | "sketch.rect_rounded"
                | "sketch.rect_frame"
        )
    }

    pub fn set_tool(&mut self, tool: &str) {
        self.tool.set_placement(match tool {
            "sketch.rect" | "sketch.rect_rounded" | "sketch.rect_frame" => {
                RectPlacement::TwoCorners
            }
            "sketch.rect_center" => RectPlacement::CenterCorner,
            "sketch.rect3" => RectPlacement::ThreeCorners,
            "sketch.rect_center3" => RectPlacement::CenterTwoCorners,
            _ => panic!("unsupported rectangle tool"),
        });
    }

    pub fn leave(&mut self) {
        self.tool.deactivate();
        self.tool.wall = None;
        self.editable = false;
    }

    fn sync(&mut self, sketch: &Sketch, state: &ToolState) {
        self.picker.sync(sketch);
        if state.is_idle() {
            let editable = self.editable;
            self.leave();
            self.editable = editable;
        }
    }

    pub fn preview(
        &mut self,
        sketch: &Sketch,
        state: &ToolState,
        cursor: Vec2D,
        scale: f64,
    ) -> RectPreview {
        self.sync(sketch, state);
        let pick = self.tool.pick(&self.world(scale), world(cursor));
        self.tool.preview_in_scene(&self.picker.geometry, pick)
    }

    pub fn click(
        &mut self,
        sketch: &mut Sketch,
        state: &mut ToolState,
        snap: Snap,
        scale: f64,
    ) -> ToolEffect {
        self.sync(sketch, state);
        let pick = self.picker.runtime_recorded_pick(snap, snap.pos);
        let step = self.tool.click_with_pick(
            &ToolWorld {
                scene: &self.picker.geometry,
                view_scale: WorldPerPx::new(scale),
                tolerance: SelectPick::contact_tolerance(),
                label_offset: sketch_geometry::world::WorldLength::ZERO,
            },
            pick,
        );
        let Some(effect) = step.effect else {
            *state = match &self.tool.phase {
                RectPhase::Idle => ToolState::Idle,
                RectPhase::Placing { start }
                    if self.tool.placement == RectPlacement::ThreeCorners =>
                {
                    ToolState::Rect3A {
                        a: self.picker.host_runtime_pick(start).placement(),
                    }
                }
                RectPhase::Placing { start }
                    if self.tool.placement == RectPlacement::CenterTwoCorners =>
                {
                    ToolState::RectCenter3At {
                        center: self.picker.host_runtime_pick(start).placement(),
                    }
                }
                RectPhase::Placing { start } if self.tool.placement.is_centered() => {
                    ToolState::RectCenterAt {
                        center: self.picker.host_runtime_pick(start).placement(),
                    }
                }
                RectPhase::Placing { start } => ToolState::RectFrom {
                    corner: self.picker.host_runtime_pick(start).placement(),
                },
                RectPhase::PlacingSide { start, edge_end } if self.tool.placement.is_centered() => {
                    ToolState::RectCenter3Corner {
                        center: self.picker.host_runtime_pick(start).placement(),
                        corner: self.picker.host_runtime_pick(edge_end).placement(),
                    }
                }
                RectPhase::PlacingSide { start, edge_end } => ToolState::Rect3B {
                    a: self.picker.host_runtime_pick(start).placement(),
                    b: self.picker.host_runtime_pick(edge_end).placement(),
                },
                _ => unreachable!("placement has not produced a draft"),
            };
            return ToolEffect::none();
        };
        assert!(matches!(effect, Effect::RectDraft { .. }));
        let plan = self.tool.outline_plan(&self.picker.geometry);
        let outcome = self.apply_plan(sketch, &plan);
        self.tool.applied(outcome);
        if outcome.is_err() {
            self.tool.retry_placement();
            return ToolEffect::log("Rectangle source is unavailable");
        }
        if self.editable {
            return ToolEffect::changed("Rectangle preview");
        }
        let closed = self.tool.step(
            &ToolWorld {
                scene: &self.picker.geometry,
                view_scale: WorldPerPx::new(scale),
                tolerance: SelectPick::contact_tolerance(),
                label_offset: sketch_geometry::world::WorldLength::ZERO,
            },
            ToolEvent::Commit,
        );
        assert!(matches!(closed.effect, Some(Effect::DraftCommit)));
        *state = ToolState::Idle;
        ToolEffect::changed("Rectangle")
    }

    fn world(&self, scale: f64) -> ToolWorld<'_> {
        ToolWorld {
            scene: &self.picker.geometry,
            view_scale: WorldPerPx::new(scale),
            tolerance: SelectPick::contact_tolerance(),
            label_offset: sketch_geometry::world::WorldLength::ZERO,
        }
    }

    pub(crate) fn apply_plan(
        &self,
        sketch: &mut Sketch,
        plan: &RectanglePlan,
    ) -> Result<Applied, Rejection> {
        let prepared = plan
            .corners
            .iter()
            .map(|p| self.picker.prepare_point(sketch, p))
            .collect::<Result<Vec<_>, _>>()?;
        let center = plan
            .center
            .as_ref()
            .map(|p| self.picker.prepare_point(sketch, p))
            .transpose()?;
        // Every source is resolved before the first host write.
        let corners: [Uuid; 4] = prepared
            .into_iter()
            .map(|p| p.apply(sketch))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let center = center.map(|p| {
            let id = p.apply(sketch);
            sketch.set_construction(id, true);
            id
        });
        let sides = std::array::from_fn::<_, 4, _>(|_| Uuid::new_v4());
        for i in 0..4 {
            let a = plan.order[i].index();
            let b = plan.order[(i + 1) % 4].index();
            let edge = Side::between(plan.order[i], plan.order[(i + 1) % 4]).index();
            let id = sides[edge];
            sketch.add_geometry(GeometryElement::Line(Line {
                id,
                start: corners[a],
                end: corners[b],
            }));
        }
        for relation in &plan.relations {
            sketch.add_constraint(match *relation {
                SideRelation::Horizontal(side) => ConstraintKind::Horizontal {
                    element: sides[side.index()],
                },
                SideRelation::Vertical(side) => ConstraintKind::Vertical {
                    element: sides[side.index()],
                },
                SideRelation::Parallel(a, b) => ConstraintKind::Parallel {
                    line1: sides[a.index()],
                    line2: sides[b.index()],
                },
                SideRelation::Perpendicular(a, b) => ConstraintKind::Perpendicular {
                    line1: sides[a.index()],
                    line2: sides[b.index()],
                },
            });
        }
        if let Some(center) = center {
            sketch.add_constraint(ConstraintKind::SymmetricAboutPoint {
                point1: corners[0],
                point2: corners[2],
                center,
            });
        }
        Ok(Applied {
            corners: corners.map(|id| SourceId(id.as_u128())),
            sides: sides.map(|id| SourceId(id.as_u128())),
            center: center.map(|id| SourceId(id.as_u128())),
        })
    }
}

impl crate::SketchWorkbench {
    pub(super) fn rectangle_gesture_preview(
        &self,
        ctx: &core_document::WorkbenchRuntimeContext,
        feature: &crate::SketchFeature,
    ) -> Option<RectPreview> {
        if self.session.rectangle().is_some() {
            return None;
        }
        let cursor = self.cursor?;
        let typed = self.dim_capture.typed(ctx.document);
        let cursor = self
            .dim_capture
            .steer(&self.tool_state, &feature.sketch, cursor);
        let mut rect = self.rectangle_gesture.borrow_mut();
        rect.set_tool(self.last_tool.as_deref().expect("rectangle tool active"));
        rect.tool.placement_input = self.line_placement(ctx, feature);
        if !typed.is_empty() {
            rect.tool.placement_input.position = Some(world(crate::ovp::override_cursor(
                &self.tool_state,
                &feature.sketch,
                cursor,
                &typed,
            )));
        }
        Some(rect.preview(
            &feature.sketch,
            &self.tool_state,
            cursor,
            crate::overlay::SketchProjector::new(ctx, feature.plane).units_per_px(),
        ))
    }
    pub(super) fn rectangle_gesture_snap(
        &self,
        ctx: &core_document::WorkbenchRuntimeContext,
        feature: &crate::SketchFeature,
    ) -> Option<Snap> {
        let preview = self.rectangle_gesture_preview(ctx, feature)?;
        Some(
            self.rectangle_gesture
                .borrow()
                .picker
                .host_runtime_pick(preview.pick().expect("placement has a pick")),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::Point;

    #[test]
    fn rectangle_application_is_atomic_and_returns_real_shared_vertices() {
        for centered in [false, true] {
            for vertex in [false, true] {
                let mut sketch = Sketch::new("rectangle application");
                let points: Vec<_> = [(20., 30.), (60., 70.), (20., 70.), (60., 30.)]
                    .into_iter()
                    .map(|(x, y)| {
                        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
                    })
                    .collect();
                let a = sketch.add_geometry(GeometryElement::Line(Line::new(points[0], points[1])));
                let b = sketch.add_geometry(GeometryElement::Line(Line::new(points[2], points[3])));
                let mut host = RectangleGesture::default();
                host.tool.set_placement(if centered {
                    RectPlacement::CenterCorner
                } else {
                    RectPlacement::TwoCorners
                });
                host.picker.sync(&sketch);
                let at = if vertex {
                    Vec2D::new(20., 30.)
                } else {
                    Vec2D::new(40., 50.)
                };
                let first = host.tool.pick(&host.world(0.05), world(at));
                let input = ToolWorld {
                    scene: &host.picker.geometry,
                    view_scale: WorldPerPx::new(0.05),
                    tolerance: SelectPick::contact_tolerance(),
                    label_offset: sketch_geometry::world::WorldLength::ZERO,
                };
                host.tool.click_with_pick(&input, first);
                host.tool.click_with_pick(
                    &input,
                    sketch_runtime::pick::Pick {
                        cursor: world(Vec2D::new(90., 100.)),
                        pos: world(Vec2D::new(90., 100.)),
                        snap: None,
                    },
                );
                let plan = host.tool.outline_plan(&host.picker.geometry);
                for missing in if vertex { vec![points[0]] } else { vec![a, b] } {
                    let mut incomplete = sketch.clone();
                    incomplete.geometry.retain(|g| g.id() != missing);
                    let before = serde_json::to_value(&incomplete).unwrap();
                    assert!(matches!(
                        host.apply_plan(&mut incomplete, &plan),
                        Err(Rejection::MissingSource(_))
                    ));
                    assert_eq!(serde_json::to_value(&incomplete).unwrap(), before);
                }
                let applied = host.apply_plan(&mut sketch, &plan).unwrap();
                host.tool.applied(Ok(applied));
                for id in applied
                    .corners
                    .into_iter()
                    .chain(applied.sides)
                    .chain(applied.center)
                {
                    assert!(sketch.get_geometry(Uuid::from_u128(id.0)).is_some());
                }
                let bound = if centered {
                    applied.center.unwrap()
                } else {
                    applied.corners[0]
                };
                if vertex {
                    assert_eq!(bound, SourceId(points[0].as_u128()));
                } else {
                    for line in [a, b] {
                        assert!(sketch.constraints.iter().any(|c| matches!(c.kind, ConstraintKind::PointOnLine { point, line: source } if point.as_u128() == bound.0 && source == line)));
                    }
                }
                let segments: Vec<_> = applied
                    .sides
                    .into_iter()
                    .map(
                        |id| match sketch.get_geometry(Uuid::from_u128(id.0)).unwrap() {
                            GeometryElement::Line(l) => l,
                            _ => unreachable!("rectangle plan must create line sides"),
                        },
                    )
                    .collect();
                for i in 0..4 {
                    assert_eq!(segments[i].end, segments[(i + 1) % 4].start);
                }
            }
        }
    }
}
