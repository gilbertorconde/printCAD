//! Conic placement over an independent geometric scene. The host owns model
//! insertion, constraints, numeric dimensions, solving and history.
use sketch_geometry::world::{WorldPerPx, WorldPoint};
use sketch_runtime::conic::{Applied, ConicPreview, ConicTool, Construction};
use sketch_runtime::conic_plan::{
    ConicGeometry, ConicKind, ConicPointRole as Role, HyperbolaInput,
};
use sketch_runtime::geometry::SourceId;
use sketch_runtime::point::{Event, Rejection};
use uuid::Uuid;

use crate::line_gesture::{host, world};
use crate::pick_projection::SelectPick;
use crate::sketch::{
    Arc, Circle, Conic, ConstraintKind, Ellipse, GeometryElement, InternalRole, Sketch, Vec2D,
};
use crate::snap::Snap;
use crate::tools::{ToolEffect, ToolState};

fn focal_kind(kind: ConicKind) -> crate::sketch::ConicKind {
    match kind {
        ConicKind::ArcOfParabola => crate::sketch::ConicKind::Parabola,
        ConicKind::ArcOfHyperbola => crate::sketch::ConicKind::Hyperbola,
        _ => unreachable!("focal conic kind"),
    }
}
pub(crate) fn ellipse_frame(
    center: WorldPoint,
    focus: WorldPoint,
    start: WorldPoint,
) -> (Vec2D, f64) {
    let minor = sketch_geometry::curves::ellipse_semi_minor(center, focus, start);
    let (major, _) = sketch_geometry::curves::ellipse_axes(center, focus, minor).unwrap();
    (Vec2D::new(major.0.x, major.0.y), minor.0 / major.length().0)
}
/// The axis points into the stored branch; its transverse direction follows it.
pub(crate) fn hyperbola_frame(
    center: WorldPoint,
    focus: WorldPoint,
    start: WorldPoint,
) -> (Vec2D, f64) {
    let minor = sketch_geometry::curves::hyperbola_semi_minor(center, focus, start);
    let (major, _) = sketch_geometry::curves::hyperbola_axes(center, focus, minor).unwrap();
    let sign = if (start - center).0.dot(major.0) < 0. {
        -1.
    } else {
        1.
    };
    (Vec2D::new(sign * major.0.x, sign * major.0.y), minor.0)
}
#[derive(Debug)]
pub(crate) struct ConicGesture {
    pub picker: SelectPick,
    pub tool: ConicTool,
}
impl Default for ConicGesture {
    fn default() -> Self {
        let mut tool = ConicTool::new(ConicKind::Arc3Pt);
        tool.hyperbola_input = HyperbolaInput::BranchVertex;
        Self {
            picker: SelectPick::default(),
            tool,
        }
    }
}
impl ConicGesture {
    pub fn supports(tool: &str) -> bool {
        matches!(
            tool,
            "sketch.arc"
                | "sketch.arc3"
                | "sketch.circle3"
                | "sketch.ellipse"
                | "sketch.ellipse3"
                | "sketch.ellipse_arc"
                | "sketch.hyperbola"
                | "sketch.parabola"
        )
    }
    pub fn set_tool(&mut self, tool: &str) {
        let kind = match tool {
            "sketch.arc" => ConicKind::ArcFromCenter,
            "sketch.arc3" => ConicKind::Arc3Pt,
            "sketch.circle3" => ConicKind::CircleFrom3Pts,
            "sketch.ellipse" => ConicKind::FullEllipse,
            "sketch.ellipse3" => ConicKind::EllipseFrom3Pts,
            "sketch.ellipse_arc" => ConicKind::ArcOfEllipse,
            "sketch.hyperbola" => ConicKind::ArcOfHyperbola,
            "sketch.parabola" => ConicKind::ArcOfParabola,
            _ => panic!("unsupported conic tool"),
        };
        if self.tool.kind() != kind {
            let placement = self.tool.placement;
            let input = self.tool.hyperbola_input;
            self.tool = ConicTool::new(kind);
            self.tool.placement = placement;
            self.tool.hyperbola_input = input;
        }
    }
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
        assert!(step.plan.is_none() && step.deactivate);
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
    ) -> ConicPreview {
        self.sync(sketch, state);
        let pick = self.tool.pick(
            &self.picker.geometry,
            world(cursor),
            WorldPerPx::new(scale),
            SelectPick::contact_tolerance(),
        );
        self.tool.preview_with_pick(&self.picker.geometry, pick)
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
        let outcome = step.plan.as_ref().map(|plan| self.apply_plan(sketch, plan));
        if let Some(outcome) = &outcome {
            self.tool.applied(outcome.clone());
        }
        *state = self.state();
        match outcome {
            None => ToolEffect::none(),
            Some(Err(_)) => ToolEffect::log("Conic source is unavailable"),
            Some(Ok(_)) => ToolEffect::changed(match self.tool.kind() {
                ConicKind::ArcFromCenter => "Arc",
                ConicKind::Arc3Pt => "Three-point arc",
                ConicKind::CircleFrom3Pts => "Three-point circle",
                ConicKind::FullEllipse => "Ellipse",
                ConicKind::EllipseFrom3Pts => "Three-point ellipse",
                ConicKind::ArcOfEllipse => "Arc of ellipse",
                ConicKind::ArcOfHyperbola => "Arc of hyperbola",
                ConicKind::ArcOfParabola => "Arc of parabola",
            }),
        }
    }
    fn state(&self) -> ToolState {
        let placement = |pick| self.picker.host_runtime_pick(pick).placement();
        match (self.tool.kind(), self.tool.picks()) {
            (_, []) => ToolState::Idle,
            (ConicKind::ArcFromCenter, [center]) => ToolState::ArcCenter {
                center: placement(center),
            },
            (ConicKind::ArcFromCenter, [center, start]) => ToolState::ArcStart {
                center: placement(center),
                start: placement(start),
            },
            (ConicKind::Arc3Pt, [start]) => ToolState::Arc3Start {
                start: placement(start),
            },
            (ConicKind::Arc3Pt, [start, end]) => ToolState::Arc3End {
                start: placement(start),
                end: placement(end),
            },
            (ConicKind::CircleFrom3Pts, [a]) => ToolState::Circle3One { a: host(a.pos) },
            (ConicKind::CircleFrom3Pts, [a, b]) => ToolState::Circle3Two {
                a: host(a.pos),
                b: host(b.pos),
            },
            (ConicKind::FullEllipse, [center]) => ToolState::EllipseCenter {
                center: placement(center),
            },
            (ConicKind::FullEllipse, [center, axis]) => ToolState::EllipseMajor {
                center: placement(center),
                major_pos: host(axis.pos),
            },
            (ConicKind::EllipseFrom3Pts, [a]) => ToolState::Ellipse3A { a: host(a.pos) },
            (ConicKind::EllipseFrom3Pts, [a, b]) => ToolState::Ellipse3B {
                a: host(a.pos),
                b: host(b.pos),
            },
            (ConicKind::ArcOfEllipse, [center]) => ToolState::EllipseArcCenter {
                center: placement(center),
            },
            (ConicKind::ArcOfEllipse, [center, axis]) => ToolState::EllipseArcMajor {
                center: placement(center),
                major_pos: host(axis.pos),
            },
            (ConicKind::ArcOfEllipse, [center, axis, start]) => ToolState::EllipseArcStart {
                center: placement(center),
                major_pos: host(axis.pos),
                start: host(start.pos),
            },
            (ConicKind::ArcOfHyperbola | ConicKind::ArcOfParabola, [center]) => {
                ToolState::FocalConicCenter {
                    kind: focal_kind(self.tool.kind()),
                    center: placement(center),
                }
            }
            (ConicKind::ArcOfHyperbola | ConicKind::ArcOfParabola, [center, focus]) => {
                ToolState::FocalConicFocus {
                    kind: focal_kind(self.tool.kind()),
                    center: placement(center),
                    focus: host(focus.pos),
                }
            }
            (ConicKind::ArcOfHyperbola | ConicKind::ArcOfParabola, [center, focus, start]) => {
                ToolState::FocalConicStart {
                    kind: focal_kind(self.tool.kind()),
                    center: placement(center),
                    focus: host(focus.pos),
                    start: host(start.pos),
                }
            }
            _ => unreachable!("conic commits its last pick"),
        }
    }
    fn apply_plan(&self, sketch: &mut Sketch, plan: &Construction) -> Result<Applied, Rejection> {
        let full = matches!(
            plan.curve.kind,
            ConicKind::FullEllipse | ConicKind::EllipseFrom3Pts
        );
        let live_focus = plan.curve.bindings.iter().any(|b| b.role == Role::Focus);
        let roles: &[Role] = match plan.curve.geometry {
            ConicGeometry::Circle { .. } => &[Role::Anchor],
            ConicGeometry::Arc { .. } => &[Role::Anchor, Role::Start, Role::End],
            ConicGeometry::Ellipse { .. } if full => &[Role::Anchor],
            ConicGeometry::Ellipse { .. } => &[Role::Anchor, Role::Start, Role::End],
            ConicGeometry::Hyperbola { .. } if !live_focus => {
                &[Role::Anchor, Role::Start, Role::End]
            }
            _ => &[Role::Anchor, Role::Focus, Role::Start, Role::End],
        };
        let prepared = roles
            .iter()
            .map(|role| self.picker.prepare_point(sketch, &plan.point(*role)))
            .collect::<Result<Vec<_>, _>>()?;
        let mut points: Vec<_> = roles
            .iter()
            .copied()
            .zip(prepared.into_iter().map(|p| p.apply(sketch)))
            .collect();
        let point = |role| {
            points
                .iter()
                .find(|(r, _)| *r == role)
                .expect("stored conic role")
                .1
        };
        let center = point(Role::Anchor);
        let made = Uuid::new_v4();
        match plan.curve.geometry {
            ConicGeometry::Circle { radius, .. } => {
                sketch.add_geometry(GeometryElement::Circle(Circle {
                    id: made,
                    center,
                    radius: radius.0,
                }));
            }
            ConicGeometry::Arc {
                center: c,
                start,
                clockwise,
                ..
            } => {
                let (a, b) = (point(Role::Start), point(Role::End));
                let (a, b) = if clockwise { (b, a) } else { (a, b) };
                sketch.add_geometry(GeometryElement::Arc(Arc {
                    id: made,
                    center,
                    start: a,
                    end: b,
                    radius: c.distance(start).0,
                }));
            }
            ConicGeometry::Ellipse {
                center: c,
                focus1,
                start,
                ..
            } => {
                let (major, ratio) = ellipse_frame(c, focus1, start);
                if full {
                    sketch.add_geometry(GeometryElement::Ellipse(Ellipse {
                        id: made,
                        center,
                        major,
                        ratio,
                        arc: None,
                    }));
                } else {
                    let (start, end) = (point(Role::Start), point(Role::End));
                    sketch.add_geometry(GeometryElement::Ellipse(Ellipse {
                        id: made,
                        ..Ellipse::new_arc(center, major, ratio, start, end)
                    }));
                    for point in [start, end] {
                        sketch.add_constraint(ConstraintKind::PointOnEllipse {
                            point,
                            ellipse: made,
                        });
                    }
                }
            }
            ConicGeometry::Hyperbola {
                center: c,
                focus1,
                start,
                ..
            } => {
                let (axis, minor) = hyperbola_frame(c, focus1, start);
                sketch.add_geometry(GeometryElement::Conic(Conic {
                    id: made,
                    ..Conic::new(
                        crate::sketch::ConicKind::Hyperbola,
                        center,
                        axis,
                        minor,
                        point(Role::Start),
                        point(Role::End),
                    )
                }));
                if live_focus {
                    sketch.add_constraint(ConstraintKind::InternalAlignment {
                        element: point(Role::Focus),
                        curve: made,
                        role: if (host(focus1) - host(c)).to_glam().dot(axis.to_glam()) > 0. {
                            InternalRole::Focus1
                        } else {
                            InternalRole::Focus2
                        },
                    });
                }
            }
            ConicGeometry::Parabola { vertex, focus, .. } => {
                let axis = host(focus) - host(vertex);
                sketch.add_geometry(GeometryElement::Conic(Conic {
                    id: made,
                    ..Conic::new(
                        crate::sketch::ConicKind::Parabola,
                        center,
                        axis,
                        0.,
                        point(Role::Start),
                        point(Role::End),
                    )
                }));
                sketch.add_constraint(ConstraintKind::InternalAlignment {
                    element: point(Role::Focus),
                    curve: made,
                    role: InternalRole::Focus1,
                });
            }
        }
        Ok(Applied {
            curve: SourceId(made.as_u128()),
            points: points
                .drain(..)
                .map(|(r, p)| (r, SourceId(p.as_u128())))
                .collect(),
        })
    }
}
impl crate::SketchWorkbench {
    pub(super) fn conic_gesture_preview(
        &self,
        ctx: &core_document::WorkbenchRuntimeContext,
        feature: &crate::SketchFeature,
    ) -> Option<ConicPreview> {
        let cursor = self.cursor?;
        let typed = self.dim_capture.typed(ctx.document);
        let cursor = self
            .dim_capture
            .steer(&self.tool_state, &feature.sketch, cursor);
        let mut conic = self.conic_gesture.borrow_mut();
        conic.set_tool(self.last_tool.as_deref().unwrap());
        conic.tool.placement = self.line_placement(ctx, feature);
        if !typed.is_empty() {
            conic.tool.placement.position = Some(world(crate::ovp::override_cursor(
                &self.tool_state,
                &feature.sketch,
                cursor,
                &typed,
            )));
        }
        Some(conic.preview(
            &feature.sketch,
            &self.tool_state,
            cursor,
            crate::overlay::SketchProjector::new(ctx, feature.plane).units_per_px(),
        ))
    }
    pub(super) fn conic_gesture_snap(
        &self,
        ctx: &core_document::WorkbenchRuntimeContext,
        feature: &crate::SketchFeature,
    ) -> Option<Snap> {
        let preview = self.conic_gesture_preview(ctx, feature)?;
        Some(
            self.conic_gesture
                .borrow()
                .picker
                .host_runtime_pick(preview.pick()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Line, Point};
    #[test]
    fn conic_host_resolves_all_sources_before_writes_and_acknowledges_real_vertices() {
        for kind in [
            ConicKind::Arc3Pt,
            ConicKind::ArcFromCenter,
            ConicKind::ArcOfEllipse,
        ] {
            let mut sketch = Sketch::new("conic application");
            let points: Vec<_> = [(20., 30.), (60., 70.), (20., 70.), (60., 30.)]
                .into_iter()
                .map(|(x, y)| {
                    sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
                })
                .collect();
            let a = sketch.add_geometry(GeometryElement::Line(Line::new(points[0], points[1])));
            let b = sketch.add_geometry(GeometryElement::Line(Line::new(points[2], points[3])));
            for existing in [false, true] {
                let mut host = ConicGesture {
                    tool: ConicTool::new(kind),
                    ..Default::default()
                };
                host.picker.sync(&sketch);
                let first = if existing {
                    Vec2D::new(20., 30.)
                } else {
                    Vec2D::new(40., 50.)
                };
                let mut positions = vec![
                    first,
                    first + Vec2D::new(100., 0.),
                    first + Vec2D::new(30., 40.),
                ];
                if kind == ConicKind::ArcOfEllipse {
                    positions.push(first + Vec2D::new(-30., -40.));
                }
                let scale = WorldPerPx::new(0.01);
                let mut plan = None;
                for pos in positions {
                    let pick = host.tool.pick(
                        &host.picker.geometry,
                        world(pos),
                        scale,
                        SelectPick::contact_tolerance(),
                    );
                    plan = host
                        .tool
                        .step(&host.picker.geometry, scale, Event::Click, Some(pick))
                        .plan;
                }
                let plan = plan.unwrap();
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
                let mut complete = sketch.clone();
                let applied = host.apply_plan(&mut complete, &plan).unwrap();
                for (_, point) in &applied.points {
                    assert!(complete.point_position(Uuid::from_u128(point.0)).is_some());
                }
                if existing {
                    let role = if kind == ConicKind::Arc3Pt {
                        Role::Start
                    } else {
                        Role::Anchor
                    };
                    assert!(
                        applied
                            .points
                            .contains(&(role, SourceId(points[0].as_u128())))
                    );
                }
                assert!(
                    complete
                        .get_geometry(Uuid::from_u128(applied.curve.0))
                        .is_some()
                );
                host.tool.applied(Ok(applied));
                assert!(!host.tool.gesture_alive());
            }
        }
    }
}
