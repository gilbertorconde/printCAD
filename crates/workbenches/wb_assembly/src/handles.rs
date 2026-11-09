//! The Move task's handles: the shared transform gizmo (`transform_gizmo`)
//! on the body the task has open, about the middle of its box along the
//! world axes. In Move its arrows and squares slide the body, in Turn its
//! rings turn it; the task panel's choice and [`MODE_ACTION`] switch them.

use core_document::{
    BodyId, BodyPlacement, ScreenSpaceLabel, ScreenSpaceMark, ScreenSpaceOverlay,
    ScreenSpacePolygon, SketchPalette, WorkbenchRuntimeContext,
};
use emath::Pos2;
use glam::{DQuat, DVec3, Quat, Vec3};
use transform_gizmo::axis::Axis;
use transform_gizmo::gizmo::{Delta, Event, Gizmo, HandleSet, Mode, Placement};
use transform_gizmo::paint::{Ink, Paint, Shape};
use transform_gizmo::view::GizmoView;

/// The action that switches the Move task's handles between moving and
/// turning.
pub const MODE_ACTION: &str = "asm.move_mode";

/// The choices of the Move task's panel, in [`Handles::mode_index`] order.
pub const MODES: [&str; 2] = ["Move", "Turn"];

/// Size of an axis letter and of the value shown while dragging, logical pixels.
const TEXT_PX: f32 = 13.0;
/// Segments a circle of the drag's feedback is drawn with.
const CIRCLE_SEGMENTS: usize = 32;

/// The middle of `body`'s box where it sits.
pub fn centre(ctx: &WorkbenchRuntimeContext, body: BodyId) -> Option<Vec3> {
    let g = ctx.document.imported_geometry(body)?;
    let (lo, hi) = g.bounds()?;
    Some((Vec3::from_array(lo) + Vec3::from_array(hi)) * 0.5)
}

/// The bench context's camera, as the gizmo asks for it.
struct View<'v, 'a>(&'v WorkbenchRuntimeContext<'a>);

impl View<'_, '_> {
    fn logical(&self, point: Pos2) -> Pos2 {
        let scale = self.0.pixels_per_point;
        Pos2::new(point.x / scale, point.y / scale)
    }

    fn physical(&self, point: Pos2) -> Pos2 {
        let scale = self.0.pixels_per_point;
        Pos2::new(point.x * scale, point.y * scale)
    }

    fn eye(&self) -> DVec3 {
        Vec3::from_array(self.0.camera_position).as_dvec3()
    }
}

impl GizmoView for View<'_, '_> {
    fn project(&self, point: DVec3) -> Option<Pos2> {
        let (x, y) = self.0.world_to_viewport(point.as_vec3().to_array())?;
        Some(self.logical(Pos2::new(x, y)))
    }

    fn ray(&self, point: Pos2) -> Option<(DVec3, DVec3)> {
        let point = self.physical(point);
        let (origin, direction) = self.0.viewport_to_ray((point.x, point.y))?;
        let direction = Vec3::from_array(direction).as_dvec3().try_normalize()?;
        Some((Vec3::from_array(origin).as_dvec3(), direction))
    }

    fn forward(&self) -> DVec3 {
        (Vec3::from_array(self.0.camera_target).as_dvec3() - self.eye())
            .try_normalize()
            .unwrap_or(DVec3::NEG_Z)
    }

    /// A short step across the line of sight at `point`, against the pixels
    /// it spans there.
    fn world_per_pixel(&self, point: DVec3) -> f64 {
        let step = ((point - self.eye()).length() * 1e-3).max(1e-6);
        let across = self.forward().any_orthonormal_vector() * step;
        let pixels = self
            .project(point)
            .zip(self.project(point + across))
            .map(|(a, b)| f64::from(a.distance(b)));
        match pixels {
            Some(pixels) if pixels > 1e-9 => step / pixels,
            _ => 1.0,
        }
    }
}

/// The body where it was when its handle was taken, and the middle it turns
/// about.
#[derive(Debug, Clone, Copy)]
struct Held {
    start: BodyPlacement,
    pivot: Vec3,
}

/// The gizmo of the Move task, between input events.
pub struct Handles {
    gizmo: Gizmo,
    mode: Mode,
    /// Where the pointer last was over the view: the handle under it lights.
    pointer: Option<Pos2>,
    held: Option<Held>,
}

impl Default for Handles {
    fn default() -> Self {
        Self {
            gizmo: Gizmo::default(),
            mode: Mode::Translate,
            pointer: None,
            held: None,
        }
    }
}

/// What the handles draw, in the bench seam's kinds.
#[derive(Default)]
pub struct Drawing {
    pub overlays: Vec<ScreenSpaceOverlay>,
    pub polygons: Vec<ScreenSpacePolygon>,
    pub marks: Vec<ScreenSpaceMark>,
    pub labels: Vec<ScreenSpaceLabel>,
}

/// The classic three world axes about `pivot`.
fn axes(pivot: Vec3) -> HandleSet<Axis> {
    HandleSet::placement_axes(Placement {
        pivot: pivot.as_dvec3(),
        orientation: DQuat::IDENTITY,
    })
}

impl Handles {
    /// The panel choice the handles show, in [`MODES`] order.
    pub fn mode_index(&self) -> usize {
        match self.mode {
            Mode::Rotate => 1,
            Mode::Translate | Mode::Scale => 0,
        }
    }

    /// Show the [`MODES`] choice `index`; a handle held lets go, the body
    /// back where it was taken.
    pub fn set_mode(&mut self, ctx: &mut WorkbenchRuntimeContext, body: BodyId, index: usize) {
        self.cancel(ctx, body);
        self.mode = if index == 1 {
            Mode::Rotate
        } else {
            Mode::Translate
        };
    }

    /// Switch between moving and turning.
    pub fn switch(&mut self, ctx: &mut WorkbenchRuntimeContext, body: BodyId) {
        let next = 1 - self.mode_index();
        self.set_mode(ctx, body, next);
    }

    /// A press at `at`: takes the handle under it. `false` when there is
    /// none, so the press goes on to the rest of the bench.
    pub fn press(&mut self, ctx: &WorkbenchRuntimeContext, body: BodyId, at: Pos2) -> bool {
        if self.gizmo.is_dragging() {
            return false;
        }
        let Some(pivot) = centre(ctx, body) else {
            return false;
        };
        let view = View(ctx);
        let at = view.logical(at);
        let set = axes(pivot);
        let Some(handle) = self
            .gizmo
            .layout(&view, &set, self.mode)
            .hit(at, self.gizmo.style.hit_radius)
        else {
            return false;
        };
        if self
            .gizmo
            .press(&view, &set, self.mode, handle, at)
            .is_none()
        {
            return false;
        }
        self.held = Some(Held {
            start: ctx.document.body_placement(body),
            pivot,
        });
        true
    }

    /// The pointer at `at`: a held handle moves the body; `false` when none
    /// is held.
    pub fn drag(&mut self, ctx: &mut WorkbenchRuntimeContext, body: BodyId, at: Pos2) -> bool {
        let at = View(ctx).logical(at);
        self.pointer = Some(at);
        if !self.gizmo.is_dragging() {
            return false;
        }
        let event = self.gizmo.drag(&View(ctx), Some(at), ctx.shift_down);
        self.apply(ctx, body, event);
        true
    }

    /// The press released: the body stays where the handle took it.
    pub fn release(&mut self, ctx: &mut WorkbenchRuntimeContext, body: BodyId) -> bool {
        if !self.gizmo.is_dragging() {
            return false;
        }
        let event = self.gizmo.release();
        self.apply(ctx, body, event);
        self.held = None;
        true
    }

    /// Let go of a held handle with the body back where it was taken;
    /// `false` when none is held.
    pub fn cancel(&mut self, ctx: &mut WorkbenchRuntimeContext, body: BodyId) -> bool {
        if self.gizmo.cancel().is_none() {
            return false;
        }
        if let Some(held) = self.held.take() {
            crate::components::move_with_unit(ctx.document, body, held.start);
        }
        true
    }

    /// Forget a held handle, the task it was for being gone.
    pub fn reset(&mut self) {
        self.gizmo.cancel();
        self.held = None;
        self.pointer = None;
    }

    fn apply(&self, ctx: &mut WorkbenchRuntimeContext, body: BodyId, event: Event) {
        let (Some(held), Event::Updated(delta) | Event::Finished(delta)) = (self.held, event)
        else {
            return;
        };
        let placement = match delta {
            Delta::Translation(offset) => {
                BodyPlacement::new(held.start.quat(), held.start.offset() + offset.as_vec3())
            }
            Delta::Rotation { axis, angle } => {
                let turn = Quat::from_axis_angle(axis.as_vec3(), angle as f32);
                BodyPlacement::new(turn, held.pivot - turn * held.pivot).after(&held.start)
            }
            Delta::Scale(_) => unreachable!("the Move task shows no scale handles"),
        };
        crate::components::move_with_unit(ctx.document, body, placement);
    }

    /// What the handles draw this frame, in the context's palette.
    pub fn drawing(&self, ctx: &WorkbenchRuntimeContext, body: BodyId) -> Drawing {
        let Some(pivot) = self.held.map(|h| h.pivot).or_else(|| centre(ctx, body)) else {
            return Drawing::default();
        };
        let view = View(ctx);
        let layout = self.gizmo.layout(&view, &axes(pivot), self.mode);
        let lit = self.gizmo.active().or_else(|| {
            self.pointer
                .and_then(|p| layout.hit(p, self.gizmo.style.hit_radius))
        });
        let mut drawing = Drawing::default();
        for shape in self.gizmo.shapes(&view, &layout, lit) {
            drawing.add(shape, &ctx.sketch_palette, ctx.pixels_per_point);
        }
        drawing
    }
}

/// A colour role in the bench palette, with its opacity.
fn colour(palette: &SketchPalette, paint: Paint) -> ([f32; 3], f32) {
    let (rgb, alpha) = match paint.ink {
        Ink::Axis(Axis::X) => (palette.axis_x, 1.0),
        Ink::Axis(Axis::Y) => (palette.axis_y, 1.0),
        Ink::Axis(Axis::Z) => (palette.selected, 1.0),
        Ink::SameLetterPlane => (palette.construction, 1.0),
        Ink::View => (palette.geometry, 1.0),
        Ink::Highlight => (palette.preselect, 1.0),
        Ink::Feedback => (palette.preview, 1.0),
        Ink::Black(a) => (palette.pill_fill, f32::from(a) / 255.0),
        Ink::White(a) => (palette.geometry, f32::from(a) / 255.0),
    };
    (rgb, alpha * paint.fade)
}

impl Drawing {
    fn add(&mut self, shape: Shape, palette: &SketchPalette, scale: f32) {
        let physical = |point: Pos2| [point.x * scale, point.y * scale];
        let line = |points: &[Pos2], width: f32, paint: Paint, dash: Option<(f32, f32)>| {
            let (rgb, alpha) = colour(palette, paint);
            points
                .windows(2)
                .map(|w| {
                    let segment =
                        ScreenSpaceOverlay::new(physical(w[0]), physical(w[1]), rgb, width * scale)
                            .with_alpha(alpha);
                    match dash {
                        Some((dash, gap)) => segment.dashed(dash * scale, gap * scale),
                        None => segment,
                    }
                })
                .collect::<Vec<_>>()
        };
        match shape {
            Shape::Line {
                points,
                stroke,
                dash,
            } => self.overlays.extend(line(
                &points,
                stroke.width,
                stroke.paint,
                dash.map(|d| (d.dash, d.gap)),
            )),
            Shape::Fan { points, paint } => {
                let (color, alpha) = colour(palette, paint);
                self.polygons.push(ScreenSpacePolygon {
                    points: points.into_iter().map(physical).collect(),
                    color,
                    alpha,
                });
            }
            Shape::Dot { at, radius, paint } => {
                let (rgb, alpha) = colour(palette, paint);
                self.marks.push(
                    ScreenSpaceMark::dot(physical(at), radius * scale, rgb).with_alpha(alpha),
                );
            }
            Shape::Circle { at, radius, stroke } => {
                let points: Vec<Pos2> = (0..=CIRCLE_SEGMENTS)
                    .map(|i| {
                        let angle = std::f32::consts::TAU * i as f32 / CIRCLE_SEGMENTS as f32;
                        at + emath::vec2(angle.cos(), angle.sin()) * radius
                    })
                    .collect();
                self.overlays
                    .extend(line(&points, stroke.width, stroke.paint, None));
            }
            Shape::Letter { at, text, paint } => {
                let (rgb, _) = colour(palette, paint);
                self.labels.push(ScreenSpaceLabel::new(
                    physical(at),
                    text,
                    rgb,
                    TEXT_PX * scale,
                ));
            }
            Shape::Plate { at, text } => {
                // The plate is placed by its top left; a label by its centre,
                // and a monospace character is about 0.6 of its size wide.
                let half = emath::vec2(text.chars().count() as f32 * 0.3, 0.5) * TEXT_PX;
                self.labels.push(
                    ScreenSpaceLabel::new(
                        physical(at + half),
                        text,
                        palette.preselect,
                        TEXT_PX * scale,
                    )
                    .mono()
                    .pill(),
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "handles_tests.rs"]
mod tests;
