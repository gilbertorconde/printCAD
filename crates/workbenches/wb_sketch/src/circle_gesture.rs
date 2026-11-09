//! Circle gesture over the stable projection. Numeric fields, solving and
//! document history remain with the host; the runtime tool owns its picks.

use sketch_geometry::world::WorldPerPx;
use sketch_runtime::circle::{Applied, CirclePhase, CirclePlan, CirclePreview, CircleTool};
use sketch_runtime::geometry::SourceId;
use sketch_runtime::point::{Event, Rejection};
use uuid::Uuid;

use crate::line_gesture::world;
use crate::pick_projection::SelectPick;
use crate::sketch::{Circle, GeometryElement, Sketch, Vec2D};
use crate::snap::Snap;
use crate::tools::{ToolEffect, ToolState};

#[derive(Debug, Default)]
pub(crate) struct CircleGesture {
    pub picker: SelectPick,
    pub tool: CircleTool,
}

impl CircleGesture {
    pub fn leave(&mut self) {
        self.tool.deactivate();
    }

    pub fn cancel(&mut self, scale: f64) -> bool {
        let step = self.tool.step(
            &self.picker.geometry,
            WorldPerPx::new(scale),
            Event::Cancel,
            None,
        );
        assert!(step.plan.is_none());
        assert!(step.deactivate);
        step.deactivate
    }

    fn sync(&mut self, sketch: &Sketch, state: &ToolState) {
        self.picker.sync(sketch);
        if state.is_idle() {
            self.leave();
        }
    }

    pub fn preview(
        &mut self,
        sketch: &Sketch,
        state: &ToolState,
        cursor: Vec2D,
        scale: f64,
    ) -> CirclePreview {
        self.sync(sketch, state);
        let pick = self.tool.pick(
            &self.picker.geometry,
            world(cursor),
            WorldPerPx::new(scale),
            SelectPick::contact_tolerance(),
        );
        self.tool.preview_with_pick(&self.picker.geometry, &pick)
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
        let step = self.tool.step(
            &self.picker.geometry,
            WorldPerPx::new(scale),
            Event::Click,
            Some(pick),
        );
        let result = step.plan.as_ref().map(|plan| self.apply_plan(sketch, plan));
        if let Some(result) = result {
            self.tool.applied(result);
        }
        *state = match &self.tool.phase {
            CirclePhase::Idle => ToolState::Idle,
            CirclePhase::Placing { center } => ToolState::CircleFrom {
                center: self.picker.host_runtime_pick(center).placement(),
            },
        };
        let Some(result) = result else {
            return ToolEffect::none();
        };
        match result {
            Ok(_) => ToolEffect::changed("Circle"),
            Err(_) => ToolEffect::log("Circle source is unavailable"),
        }
    }

    fn apply_plan(&self, sketch: &mut Sketch, plan: &CirclePlan) -> Result<Applied, Rejection> {
        let center = self.picker.prepare_point(sketch, &plan.center)?;
        let center = center.apply(sketch);
        let made = Uuid::new_v4();
        sketch.add_geometry(GeometryElement::Circle(Circle {
            id: made,
            center,
            radius: plan.radius.0,
        }));
        Ok(Applied {
            circle: SourceId(made.as_u128()),
            center: SourceId(center.as_u128()),
        })
    }
}

impl crate::SketchWorkbench {
    pub(super) fn circle_gesture_preview(
        &self,
        ctx: &core_document::WorkbenchRuntimeContext,
        feature: &crate::SketchFeature,
    ) -> Option<CirclePreview> {
        let cursor = self.cursor?;
        let typed = self.dim_capture.typed(ctx.document);
        let cursor = self
            .dim_capture
            .steer(&self.tool_state, &feature.sketch, cursor);
        let mut circle = self.circle_gesture.borrow_mut();
        circle.tool.placement = self.line_placement(ctx, feature);
        if !typed.is_empty() {
            circle.tool.placement.position = Some(world(crate::ovp::override_cursor(
                &self.tool_state,
                &feature.sketch,
                cursor,
                &typed,
            )));
        }
        Some(circle.preview(
            &feature.sketch,
            &self.tool_state,
            cursor,
            crate::overlay::SketchProjector::new(ctx, feature.plane).units_per_px(),
        ))
    }

    pub(super) fn circle_gesture_snap(
        &self,
        ctx: &core_document::WorkbenchRuntimeContext,
        feature: &crate::SketchFeature,
    ) -> Option<Snap> {
        let preview = self.circle_gesture_preview(ctx, feature)?;
        Some(
            self.circle_gesture
                .borrow()
                .picker
                .host_runtime_pick(preview.pick()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{ConstraintKind, Line, Point};

    #[test]
    fn circle_host_applies_selected_center_atomically_without_a_document_projection() {
        for existing in [false, true] {
            let mut sketch = Sketch::new("circle application");
            let points: Vec<_> = [(20., 30.), (60., 70.), (20., 70.), (60., 30.)]
                .into_iter()
                .map(|(x, y)| {
                    sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
                })
                .collect();
            let a = sketch.add_geometry(GeometryElement::Line(Line::new(points[0], points[1])));
            let b = sketch.add_geometry(GeometryElement::Line(Line::new(points[2], points[3])));
            let mut host = CircleGesture::default();
            let scale = WorldPerPx::new(0.05);
            host.picker.sync(&sketch);
            let at = world(if existing {
                Vec2D::new(20., 30.)
            } else {
                Vec2D::new(40., 50.)
            });
            let center = host.tool.pick(
                &host.picker.geometry,
                at,
                scale,
                SelectPick::contact_tolerance(),
            );
            host.tool
                .step(&host.picker.geometry, scale, Event::Click, Some(center));
            // A new candidate at the selected intersection must not steal its sources.
            sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(40., 50.))));
            host.picker.sync(&sketch);
            let rim = host.tool.pick(
                &host.picker.geometry,
                world(Vec2D::new(20., 70.)),
                scale,
                SelectPick::contact_tolerance(),
            );
            let preview = host.tool.preview_with_pick(&host.picker.geometry, &rim);
            let plan = host
                .tool
                .step(&host.picker.geometry, scale, Event::Click, Some(rim))
                .plan
                .unwrap();
            let CirclePreview::Rubber { radius, .. } = preview else {
                unreachable!("a circle with its center picked must show a radius preview")
            };
            assert_eq!(plan.radius, radius);
            for missing in if existing {
                vec![points[0]]
            } else {
                vec![a, b]
            } {
                let mut incomplete = sketch.clone();
                incomplete.geometry.retain(|g| g.id() != missing);
                let before = serde_json::to_value(&incomplete).unwrap();
                assert!(matches!(
                    host.apply_plan(&mut incomplete, &plan),
                    Err(Rejection::MissingSource(_))
                ));
                assert_eq!(serde_json::to_value(&incomplete).unwrap(), before);
            }
            let before = sketch.geometry.len();
            let applied = host.apply_plan(&mut sketch, &plan).unwrap();
            host.tool.applied(Ok(applied));
            let center = Uuid::from_u128(applied.center.0);
            assert_eq!(sketch.geometry.len(), before + if existing { 1 } else { 2 });
            if existing {
                assert_eq!(center, points[0]);
            }
            let memberships: std::collections::HashSet<_> = sketch
                .constraints
                .iter()
                .filter_map(|c| {
                    if let ConstraintKind::PointOnLine { point, line } = c.kind
                        && point == center
                    {
                        Some(line)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(
                memberships,
                if existing {
                    std::collections::HashSet::new()
                } else {
                    [a, b].into_iter().collect()
                }
            );
            assert!(
                sketch
                    .constraints
                    .iter()
                    .all(|c| !matches!(c.kind, ConstraintKind::PointOnCircle { .. }))
            );
            assert!(
                sketch
                    .get_geometry(Uuid::from_u128(applied.circle.0))
                    .is_some()
            );
        }
    }
}
