//! Editable rectangles calculated outside the persisted document. The runtime
//! owns the gesture; this host owns constraints, solving and acceptance.

use serde::{Deserialize, Serialize};
use sketch_geometry::types::Angle;
use sketch_geometry::world::{WorldLength, WorldPoint};
use sketch_runtime::rect_shape::{RectCornerMode, RectDraft, RectFrame};
use uuid::Uuid;

use crate::operations::SolveOutcome;
use crate::rectangle_gesture::RectangleGesture;
use crate::sketch::{ConstraintKind, GeometryElement, Sketch};
use crate::sketch_draft::{SketchDraft, SketchSession};
use crate::{SketchFeature, SketchWorkbench};
use core_document::WorkbenchRuntimeContext;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct RectangleParameters {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub angle: f64,
    pub corners: CornerStyle,
    pub size: f64,
    pub wall: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) enum CornerStyle {
    Sharp,
    Fillet,
    Chamfer,
}

impl RectangleParameters {
    pub fn from_draft(draft: RectDraft) -> Self {
        Self {
            x: draft.center.0.x,
            y: draft.center.0.y,
            width: draft.width().0,
            height: draft.height().0,
            angle: match draft.frame {
                RectFrame::AxisAligned => 0.,
                RectFrame::Rotated(a) => a.radians().to_degrees(),
            },
            corners: match draft.corner_mode {
                RectCornerMode::None => CornerStyle::Sharp,
                RectCornerMode::Fillet => CornerStyle::Fillet,
                RectCornerMode::Chamfer => CornerStyle::Chamfer,
            },
            size: draft.corner_size.0,
            wall: draft.wall.map(|v| v.0),
        }
    }

    fn draft(self, mut draft: RectDraft) -> Result<RectDraft, String> {
        if ![
            self.x,
            self.y,
            self.width,
            self.height,
            self.angle,
            self.size,
        ]
        .into_iter()
        .all(f64::is_finite)
            || self.width <= 1e-6
            || self.height <= 1e-6
        {
            return Err("Enter finite coordinates and positive width and height".into());
        }
        if self.corners != CornerStyle::Sharp
            && (self.size <= 1e-6 || 2. * self.size >= self.width.min(self.height))
        {
            return Err("Corner size must be positive and less than half the smaller side".into());
        }
        draft.center = WorldPoint::new(self.x, self.y);
        draft.half_w = WorldLength::new(self.width / 2.);
        draft.half_h = WorldLength::new(self.height / 2.);
        draft.frame = if self.angle == 0. {
            RectFrame::AxisAligned
        } else {
            RectFrame::Rotated(Angle(self.angle.to_radians()))
        };
        draft.corner_mode = match self.corners {
            CornerStyle::Sharp => RectCornerMode::None,
            CornerStyle::Fillet => RectCornerMode::Fillet,
            CornerStyle::Chamfer => RectCornerMode::Chamfer,
        };
        draft.corner_size = WorldLength::new(self.size);
        draft.wall = self.wall.map(WorldLength::new);
        if draft.corners().iter().any(|point| !point.0.is_finite()) {
            return Err("Rectangle coordinates overflow at this size".into());
        }
        draft
            .inner_corners()
            .map_err(|_| "Wall must fit inside a rectangle with sharp corners".to_string())?;
        Ok(draft)
    }
}

pub(crate) struct RectangleSession {
    pub draft: SketchDraft,
    pub parameters: RectangleParameters,
    pub construction: bool,
}

/// Uses the same host plan application and corner operations as drawing tools.
/// A failure discards the whole candidate, including solver movement.
pub(crate) fn calculate(
    rect: &mut RectangleGesture,
    base: &Sketch,
    parameters: RectangleParameters,
    construction: bool,
) -> Result<Sketch, String> {
    rect.tool
        .applied(Err(sketch_runtime::point::Rejection::Refused));
    let draft = parameters.draft(*rect.tool.draft().expect("live rectangle"))?;
    rect.tool.set_draft(draft);
    rect.picker.sync(base);
    // Outline application is shared; corner treatment and inner contours are
    // host operations on the same isolated candidate.
    let mut outline = rect.tool.clone();
    outline.set_draft(RectDraft {
        corner_mode: RectCornerMode::None,
        wall: None,
        ..draft
    });
    let plan = outline.outline_plan(&rect.picker.geometry);
    let mut working = base.clone();
    let applied = rect
        .apply_plan(&mut working, &plan)
        .map_err(|_| "A rectangle source is unavailable".to_string())?;
    let corners = applied.corners.map(|id| Uuid::from_u128(id.0));
    let sides = applied.sides.map(|id| Uuid::from_u128(id.0));
    for (a, b, distance) in [(0, 1, parameters.width), (0, 3, parameters.height)] {
        let constraint = match draft.frame {
            RectFrame::AxisAligned if b == 1 => ConstraintKind::DistanceX {
                a: corners[a],
                b: Some(corners[b]),
                value: distance,
            },
            RectFrame::AxisAligned => ConstraintKind::DistanceY {
                a: corners[a],
                b: Some(corners[b]),
                value: distance,
            },
            RectFrame::Rotated(_) => ConstraintKind::Distance {
                point1: corners[a],
                point2: corners[b],
                distance,
            },
        };
        working.add_constraint(constraint);
    }
    if matches!(draft.frame, RectFrame::Rotated(_)) {
        let GeometryElement::Line(side) = working.get_geometry(sides[0]).unwrap() else {
            unreachable!("rectangle outline plan must create line sides")
        };
        let direction =
            working.point_position(side.end).unwrap() - working.point_position(side.start).unwrap();
        working.add_constraint(ConstraintKind::AngleToAxis {
            line: side.id,
            axis: crate::sketch::AxisDirection::Horizontal,
            angle_rad: direction.y.atan2(direction.x),
        });
    }
    if parameters.corners != CornerStyle::Sharp {
        for (i, point) in corners.into_iter().enumerate() {
            let adjacent = sides
                .into_iter()
                .filter(|id| {
                    let GeometryElement::Line(line) = working.get_geometry(*id).unwrap() else {
                        unreachable!("untreated rectangle sides must remain lines")
                    };
                    line.start == point || line.end == point
                })
                .collect::<Vec<_>>();
            let curves: [Uuid; 2] = adjacent
                .try_into()
                .expect("two rectangle sides at each logical corner");
            if !crate::tools::modify::rectangle_corner(
                &mut working,
                point,
                curves,
                parameters.corners == CornerStyle::Fillet,
                parameters.size,
            ) {
                return Err(format!("Corner {} cannot be treated at this size", i + 1));
            }
        }
    }
    if let Some(inner) = draft.inner_corners().unwrap() {
        // The inner contour is generated geometry, without picked sources.
        let inner_plan = sketch_runtime::rect::RectanglePlan {
            corners: inner.map(|pos| {
                sketch_runtime::point::PointPlan::new(sketch_runtime::pick::Pick {
                    cursor: pos,
                    pos,
                    snap: None,
                })
            }),
            center: None,
            ..plan.clone()
        };
        let inner_applied = rect
            .apply_plan(&mut working, &inner_plan)
            .map_err(|_| "Inner contour cannot be built".to_string())?;
        let inner_ids = inner_applied.corners.map(|id| Uuid::from_u128(id.0));
        let outer = draft.corners();
        for i in [0, 2] {
            let delta = inner[i] - outer[i];
            working.add_constraint(ConstraintKind::DistanceX {
                a: corners[i],
                b: Some(inner_ids[i]),
                value: delta.0.x,
            });
            working.add_constraint(ConstraintKind::DistanceY {
                a: corners[i],
                b: Some(inner_ids[i]),
                value: delta.0.y,
            });
        }
    }
    if construction {
        for id in working
            .geometry
            .iter()
            .map(GeometryElement::id)
            .filter(|id| base.get_geometry(*id).is_none())
            .collect::<Vec<_>>()
        {
            working.set_construction(id, true);
        }
    }
    let solved = crate::solver::solve(&mut working);
    if matches!(solved, SolveOutcome::NotConverged { .. }) {
        return Err("Constraints cannot be solved for this rectangle".into());
    }
    rect.tool.applied(Ok(applied));
    Ok(working)
}

impl SketchWorkbench {
    pub(crate) fn rectangle_is_current(&self, ctx: &WorkbenchRuntimeContext) -> bool {
        self.session
            .rectangle()
            .is_some_and(|s| s.draft.is_current(ctx))
    }

    pub(crate) fn discard_rectangle(&mut self) {
        if self.session.rectangle().is_some() {
            self.session = SketchSession::None;
        }
        self.rectangle_gesture.borrow_mut().leave();
        self.tool_state = crate::tools::ToolState::Idle;
        self.dim_capture.clear_buffers();
        self.dim_capture.sync(&self.tool_state, None);
        self.draw_record = None;
        self.last_diagnosis = None;
    }

    pub(crate) fn invalidate_rectangle(&mut self, ctx: &WorkbenchRuntimeContext) -> bool {
        if self.session.rectangle().is_some() && !self.rectangle_is_current(ctx) {
            self.discard_rectangle();
            return true;
        }
        false
    }

    pub(crate) fn open_rectangle(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        base: SketchFeature,
        typed: &[(crate::ovp::FieldKind, f64)],
    ) {
        let draft = *self.rectangle_gesture.borrow().tool.draft().unwrap();
        let mut parameters = RectangleParameters::from_draft(draft);
        if !self.rectangle_gesture.borrow().tool.placement.rotated() {
            for &(field, value) in typed {
                match field {
                    crate::ovp::FieldKind::Width => parameters.width = value,
                    crate::ovp::FieldKind::Height => parameters.height = value,
                    _ => {}
                }
            }
        }
        match self.last_tool.as_deref() {
            Some("sketch.rect_rounded") => {
                parameters.corners = CornerStyle::Fillet;
                parameters.size = self.tool_params.fillet_radius;
            }
            Some("sketch.rect_frame") => parameters.wall = Some(self.tool_params.offset_distance),
            _ => {}
        }
        let construction = self.construction_mode();
        let mut document = SketchDraft::new(base, ctx);
        document.set_sketch(calculate(
            &mut self.rectangle_gesture.borrow_mut(),
            &document.base.sketch,
            parameters,
            construction,
        ));
        self.session = SketchSession::Rectangle(RectangleSession {
            draft: document,
            parameters,
            construction,
        });
        self.tool_state = crate::tools::ToolState::Idle;
        self.dim_capture.clear_buffers();
        self.dim_capture.sync(&self.tool_state, None);
        self.last_diagnosis = None;
    }

    pub(crate) fn recalculate_rectangle(&mut self) {
        let session = self.session.rectangle_mut().expect("rectangle session");
        session.draft.set_sketch(calculate(
            &mut self.rectangle_gesture.borrow_mut(),
            &session.draft.base.sketch,
            session.parameters,
            session.construction,
        ));
        self.last_diagnosis = None;
    }

    pub(crate) fn accept_rectangle(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if self.invalidate_rectangle(ctx) {
            return;
        }
        if self.session.rectangle().is_none() {
            return;
        }
        let SketchSession::Rectangle(session) = std::mem::take(&mut self.session) else {
            unreachable!("rectangle acceptance requires a rectangle session")
        };
        let Ok(feature) = &session.draft.working else {
            self.session = SketchSession::Rectangle(session);
            return;
        };
        if !self.store_sketch(ctx, feature.clone()) {
            self.session = SketchSession::Rectangle(session);
            return;
        }
        if let Some(record) = self.draw_record.as_mut() {
            record.changed = true;
            record.settings.insert(
                "construction".into(),
                serde_json::json!(session.construction),
            );
            record.settings.insert(
                "rectangle".into(),
                serde_json::to_value(session.parameters).unwrap(),
            );
        }
        self.rectangle_gesture.borrow_mut().leave();
        self.tool_state = crate::tools::ToolState::Idle;
        self.dim_capture.clear_buffers();
        self.flush_draw_record(ctx);
        self.finish_sketch_task(ctx, "Rectangle");
    }

    pub(crate) fn rectangle_handles(&self) -> Option<[WorldPoint; 4]> {
        let working = &self
            .session
            .rectangle()?
            .draft
            .working
            .as_ref()
            .ok()?
            .sketch;
        let rect = self.rectangle_gesture.borrow();
        let applied = rect.tool.application.as_ref()?.as_ref().ok()?;
        Some(applied.corners.map(|id| {
            let point = working
                .point_position(Uuid::from_u128(id.0))
                .expect("solved logical corner");
            WorldPoint::new(point.x, point.y)
        }))
    }

    pub(crate) fn rectangle_pointer(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        event: &core_document::WorkbenchInputEvent,
    ) {
        use core_document::{MouseButton, WorkbenchInputEvent};
        use sketch_runtime::rect::ToolEvent;
        let (position, kind) = match event {
            WorkbenchInputEvent::MousePress {
                button: MouseButton::Left,
                viewport_pos,
            } => (*viewport_pos, 0),
            WorkbenchInputEvent::MouseMove { viewport_pos } => (*viewport_pos, 1),
            WorkbenchInputEvent::MouseRelease {
                button: MouseButton::Left,
                viewport_pos,
            } => (*viewport_pos, 2),
            _ => return,
        };
        let session = self.session.rectangle().unwrap();
        let Some(cursor) = Self::cursor_to_sketch(ctx, &session.draft.base.plane, position) else {
            return;
        };
        let cursor = WorldPoint::new(cursor.x, cursor.y);
        let scale = sketch_geometry::world::WorldPerPx::new(
            crate::overlay::SketchProjector::new(ctx, session.draft.base.plane).units_per_px(),
        );
        let handles = self.rectangle_handles();
        let mut rect = self.rectangle_gesture.borrow_mut();
        if kind == 0
            && let Some(corners) = handles
            && corners.iter().any(|point| {
                point.distance(cursor) <= sketch_runtime::rect::handle_hit_radius(scale)
            })
        {
            // A new handle gesture starts at the displayed solution. The solver
            // can translate the rectangle together with its existing sources.
            let mut draft = *rect.tool.draft().unwrap();
            draft.center = WorldPoint::new(
                corners.iter().map(|p| p.0.x).sum::<f64>() / 4.,
                corners.iter().map(|p| p.0.y).sum::<f64>() / 4.,
            );
            let applied = *rect.tool.application.as_ref().unwrap().as_ref().unwrap();
            rect.tool.set_draft(draft);
            rect.tool.applied(Ok(applied));
        }
        let input = match kind {
            0 => ToolEvent::Press { cursor },
            1 if rect.tool.is_dragging() => ToolEvent::Drag { cursor },
            2 => ToolEvent::Release { cursor },
            _ => return,
        };
        let RectangleGesture { tool, picker, .. } = &mut *rect;
        let world = sketch_runtime::rect::ToolWorld {
            scene: &picker.geometry,
            view_scale: scale,
            tolerance: crate::pick_projection::SelectPick::contact_tolerance(),
            label_offset: WorldLength::ZERO,
        };
        let step = tool.step(&world, input);
        if matches!(
            step.effect,
            Some(sketch_runtime::rect::Effect::RectDraft { .. })
        ) {
            let parameters = RectangleParameters::from_draft(*tool.draft().unwrap());
            drop(rect);
            self.session.rectangle_mut().unwrap().parameters = parameters;
            self.recalculate_rectangle();
        }
    }
}
