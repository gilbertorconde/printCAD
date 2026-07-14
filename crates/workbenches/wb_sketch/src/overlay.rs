//! Screen-space overlay generation for the sketch under edit.
//!
//! While a sketch is being edited its geometry is drawn as constant-width
//! 2D lines (crisp at any zoom, like FreeCAD's edit mode); the 3D
//! tessellation is used only for sketches *not* being edited. Colors follow
//! FreeCAD conventions loosely: white geometry, green selection, orange
//! hover, pale-blue tool preview.

use std::collections::HashSet;

use core_document::{ScreenSpaceOverlay, WorkbenchRuntimeContext};
use uuid::Uuid;

use crate::sketch::{GeometryElement, Sketch, SketchPlane, Vec2D};
use crate::snap::{arc_angles, SnapTarget};
use crate::tools::ToolState;

pub const COLOR_GEOMETRY: [f32; 3] = [0.92, 0.92, 0.92];
pub const COLOR_SELECTED: [f32; 3] = [0.35, 0.95, 0.45];
pub const COLOR_HOVERED: [f32; 3] = [1.0, 0.75, 0.2];
pub const COLOR_PREVIEW: [f32; 3] = [0.55, 0.75, 1.0];
pub const COLOR_AXIS_X: [f32; 3] = [0.85, 0.35, 0.35];
pub const COLOR_AXIS_Y: [f32; 3] = [0.35, 0.75, 0.35];

const CIRCLE_SEGMENTS: usize = 48;
const ARC_SEGMENTS: usize = 32;
const POINT_HALF_PX: f32 = 3.5;
const AXIS_EXTENT_UNITS: f32 = 1.0e3;

/// Projects sketch-plane coordinates into viewport-local pixels.
pub struct SketchProjector<'a> {
    ctx: &'a WorkbenchRuntimeContext<'a>,
    plane: SketchPlane,
}

impl<'a> SketchProjector<'a> {
    pub fn new(ctx: &'a WorkbenchRuntimeContext<'a>, plane: SketchPlane) -> Self {
        Self { ctx, plane }
    }

    pub fn to_world(&self, pos: Vec2D) -> [f32; 3] {
        let x_axis = glam::Vec3::from_array(self.plane.x_axis);
        let y_axis = glam::Vec3::from_array(self.plane.y_axis);
        let origin = glam::Vec3::from_array(self.plane.origin);
        (origin + x_axis * pos.x + y_axis * pos.y).to_array()
    }

    pub fn to_px(&self, pos: Vec2D) -> Option<[f32; 2]> {
        let (x, y) = self.ctx.world_to_viewport(self.to_world(pos))?;
        Some([x, y])
    }

    /// Sketch units per screen pixel at the plane origin; used to convert
    /// pixel-based tolerances into sketch units. Falls back to a small
    /// constant when the projection is degenerate.
    pub fn units_per_px(&self) -> f32 {
        let origin = self.to_px(Vec2D::new(0.0, 0.0));
        let unit_x = self.to_px(Vec2D::new(1.0, 0.0));
        match (origin, unit_x) {
            (Some(o), Some(ux)) => {
                let px_per_unit = ((ux[0] - o[0]).powi(2) + (ux[1] - o[1]).powi(2)).sqrt();
                if px_per_unit > 1e-6 {
                    1.0 / px_per_unit
                } else {
                    0.01
                }
            }
            _ => 0.01,
        }
    }
}

/// Emit a polyline between sketch points as overlay segments.
fn push_polyline(
    out: &mut Vec<ScreenSpaceOverlay>,
    proj: &SketchProjector,
    pts: impl Iterator<Item = Vec2D>,
    color: [f32; 3],
    thickness: f32,
) {
    let mut prev: Option<[f32; 2]> = None;
    for p in pts {
        let px = proj.to_px(p);
        if let (Some(a), Some(b)) = (prev, px) {
            out.push(ScreenSpaceOverlay::new(a, b, color, thickness));
        }
        prev = px;
    }
}

fn push_point_marker(
    out: &mut Vec<ScreenSpaceOverlay>,
    proj: &SketchProjector,
    pos: Vec2D,
    color: [f32; 3],
) {
    if let Some([x, y]) = proj.to_px(pos) {
        let h = POINT_HALF_PX;
        out.push(ScreenSpaceOverlay::new(
            [x - h, y - h],
            [x + h, y + h],
            color,
            2.0,
        ));
        out.push(ScreenSpaceOverlay::new(
            [x - h, y + h],
            [x + h, y - h],
            color,
            2.0,
        ));
    }
}

fn circle_points(center: Vec2D, radius: f32) -> impl Iterator<Item = Vec2D> {
    (0..=CIRCLE_SEGMENTS).map(move |i| {
        let a = (i as f32 / CIRCLE_SEGMENTS as f32) * std::f32::consts::TAU;
        Vec2D::new(center.x + radius * a.cos(), center.y + radius * a.sin())
    })
}

fn arc_points(center: Vec2D, start: Vec2D, end: Vec2D) -> impl Iterator<Item = Vec2D> {
    let sv = (start - center).to_glam();
    let ev = (end - center).to_glam();
    let radius = sv.length();
    let (start_angle, sweep) = arc_angles(sv, ev);
    (0..=ARC_SEGMENTS).map(move |i| {
        let a = start_angle + sweep * (i as f32 / ARC_SEGMENTS as f32);
        Vec2D::new(center.x + radius * a.cos(), center.y + radius * a.sin())
    })
}

fn element_color(id: Uuid, selected: &HashSet<Uuid>, hovered: Option<Uuid>) -> [f32; 3] {
    if selected.contains(&id) {
        COLOR_SELECTED
    } else if hovered == Some(id) {
        COLOR_HOVERED
    } else {
        COLOR_GEOMETRY
    }
}

fn push_element(
    out: &mut Vec<ScreenSpaceOverlay>,
    proj: &SketchProjector,
    sketch: &Sketch,
    geom: &GeometryElement,
    color: [f32; 3],
    thickness: f32,
) {
    match geom {
        GeometryElement::Point(p) => push_point_marker(out, proj, p.position, color),
        GeometryElement::Line(l) => {
            if let (Some(a), Some(b)) =
                (sketch.point_position(l.start), sketch.point_position(l.end))
            {
                push_polyline(out, proj, [a, b].into_iter(), color, thickness);
            }
        }
        GeometryElement::Circle(c) => {
            if let Some(center) = sketch.point_position(c.center) {
                push_polyline(out, proj, circle_points(center, c.radius), color, thickness);
            }
        }
        GeometryElement::Arc(a) => {
            if let (Some(c), Some(s), Some(e)) = (
                sketch.point_position(a.center),
                sketch.point_position(a.start),
                sketch.point_position(a.end),
            ) {
                push_polyline(out, proj, arc_points(c, s, e), color, thickness);
            }
        }
    }
}

/// Overlays for the sketch's own origin + axes (drawn subtly under the
/// geometry so the user can orient themselves).
fn push_axes(out: &mut Vec<ScreenSpaceOverlay>, proj: &SketchProjector) {
    push_polyline(
        out,
        proj,
        [
            Vec2D::new(-AXIS_EXTENT_UNITS, 0.0),
            Vec2D::new(AXIS_EXTENT_UNITS, 0.0),
        ]
        .into_iter(),
        COLOR_AXIS_X,
        1.0,
    );
    push_polyline(
        out,
        proj,
        [
            Vec2D::new(0.0, -AXIS_EXTENT_UNITS),
            Vec2D::new(0.0, AXIS_EXTENT_UNITS),
        ]
        .into_iter(),
        COLOR_AXIS_Y,
        1.0,
    );
}

/// Preview of the in-progress tool shape from its anchors to `cursor`.
fn push_preview(
    out: &mut Vec<ScreenSpaceOverlay>,
    proj: &SketchProjector,
    sketch: &Sketch,
    state: &ToolState,
    cursor: Vec2D,
) {
    let pos = |t: &SnapTarget| t.position(sketch);
    match state {
        ToolState::Idle => {}
        ToolState::LineFrom { from, .. } => {
            if let Some(a) = pos(from) {
                push_polyline(out, proj, [a, cursor].into_iter(), COLOR_PREVIEW, 1.5);
            }
        }
        ToolState::RectFrom { corner } => {
            if let Some(a) = pos(corner) {
                let b = Vec2D::new(cursor.x, a.y);
                let d = Vec2D::new(a.x, cursor.y);
                push_polyline(
                    out,
                    proj,
                    [a, b, cursor, d, a].into_iter(),
                    COLOR_PREVIEW,
                    1.5,
                );
            }
        }
        ToolState::CircleFrom { center } => {
            if let Some(c) = pos(center) {
                let r = (cursor - c).to_glam().length();
                if r > 1e-6 {
                    push_polyline(out, proj, circle_points(c, r), COLOR_PREVIEW, 1.5);
                }
                push_point_marker(out, proj, c, COLOR_PREVIEW);
            }
        }
        ToolState::ArcCenter { center } => {
            if let Some(c) = pos(center) {
                push_point_marker(out, proj, c, COLOR_PREVIEW);
                push_polyline(out, proj, [c, cursor].into_iter(), COLOR_PREVIEW, 1.0);
            }
        }
        ToolState::ArcStart { center, start } => {
            if let (Some(c), Some(s)) = (pos(center), pos(start)) {
                let r = (s - c).to_glam().length();
                let dir = (cursor - c).to_glam();
                if r > 1e-6 && dir.length() > 1e-6 {
                    let end = Vec2D::from_glam(c.to_glam() + dir.normalize() * r);
                    push_polyline(out, proj, arc_points(c, s, end), COLOR_PREVIEW, 1.5);
                }
                push_point_marker(out, proj, c, COLOR_PREVIEW);
            }
        }
    }
}

/// Build the full overlay set for one frame of sketch editing.
pub fn build_overlays(
    proj: &SketchProjector,
    sketch: &Sketch,
    selected: &HashSet<Uuid>,
    hovered: Option<Uuid>,
    tool_state: &ToolState,
    cursor: Option<Vec2D>,
) -> Vec<ScreenSpaceOverlay> {
    let mut out = Vec::new();
    push_axes(&mut out, proj);

    // Curves first, then points on top so vertices stay visible.
    for geom in &sketch.geometry {
        if !matches!(geom, GeometryElement::Point(_)) {
            let color = element_color(geom.id(), selected, hovered);
            push_element(&mut out, proj, sketch, geom, color, 2.0);
        }
    }
    for geom in &sketch.geometry {
        if matches!(geom, GeometryElement::Point(_)) {
            let color = element_color(geom.id(), selected, hovered);
            push_element(&mut out, proj, sketch, geom, color, 2.0);
        }
    }

    if let Some(cursor) = cursor {
        push_preview(&mut out, proj, sketch, tool_state, cursor);
    }
    out
}
