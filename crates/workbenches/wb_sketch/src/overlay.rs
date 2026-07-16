//! Screen-space overlay generation for the sketch under edit.
//!
//! While a sketch is being edited its geometry is drawn as constant-width
//! 2D lines (crisp at any zoom); the 3D
//! tessellation is used only for sketches *not* being edited. Colors follow
//! a common CAD palette: white geometry, green selection, orange
//! hover, pale-blue tool preview.

use std::collections::HashSet;

use core_document::{ScreenSpaceOverlay, WorkbenchRuntimeContext};
use uuid::Uuid;

use crate::geom2d;
use crate::sketch::{GeometryElement, Sketch, SketchPlane, Vec2D};
use crate::snap::{arc_angles, SnapTarget};
use crate::tools::{
    self, arc_slot_shape, polygon_vertices, slot_corners, Similarity, ToolParams, ToolState,
};

pub const COLOR_GEOMETRY: [f32; 3] = [0.92, 0.92, 0.92];
pub const COLOR_CONSTRUCTION: [f32; 3] = [0.4, 0.55, 0.95];
pub const COLOR_SELECTED: [f32; 3] = [0.35, 0.95, 0.45];
pub const COLOR_HOVERED: [f32; 3] = [1.0, 0.75, 0.2];
pub const COLOR_PREVIEW: [f32; 3] = [0.55, 0.75, 1.0];
/// Base color for every non-construction element once the sketch is fully
/// constrained.
pub const COLOR_FULLY_CONSTRAINED: [f32; 3] = [0.2, 0.9, 0.2];
/// Pending auto-constraint hint (the cursor is snapping onto a curve).
pub const COLOR_AUTO_CONSTRAINT: [f32; 3] = [0.95, 0.85, 0.25];
/// Trim hover: the span that a click would remove.
pub const COLOR_TRIM: [f32; 3] = [0.95, 0.3, 0.3];
pub const COLOR_AXIS_X: [f32; 3] = [0.85, 0.35, 0.35];
pub const COLOR_AXIS_Y: [f32; 3] = [0.35, 0.75, 0.35];

const CIRCLE_SEGMENTS: usize = 48;
const ARC_SEGMENTS: usize = 32;
const POINT_HALF_PX: f32 = 3.5;
const AXIS_EXTENT_UNITS: f32 = 1.0e3;
/// Dash pattern for construction geometry and the selection box, in
/// viewport pixels (applied after projection so it is zoom-independent).
const DASH_PX: f32 = 6.0;
const DASH_GAP_PX: f32 = 4.0;

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

/// Emit one projected segment, either solid or split into ~6px dashes with
/// ~4px gaps (in viewport pixel space).
fn push_segment_px(
    out: &mut Vec<ScreenSpaceOverlay>,
    a: [f32; 2],
    b: [f32; 2],
    color: [f32; 3],
    thickness: f32,
    dashed: bool,
) {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let len = (dx * dx + dy * dy).sqrt();
    if !dashed || len <= DASH_PX {
        out.push(ScreenSpaceOverlay::new(a, b, color, thickness));
        return;
    }
    let period = DASH_PX + DASH_GAP_PX;
    let mut start = 0.0f32;
    while start < len {
        let end = (start + DASH_PX).min(len);
        let t0 = start / len;
        let t1 = end / len;
        out.push(ScreenSpaceOverlay::new(
            [a[0] + dx * t0, a[1] + dy * t0],
            [a[0] + dx * t1, a[1] + dy * t1],
            color,
            thickness,
        ));
        start += period;
    }
}

/// Emit a polyline between sketch points as overlay segments. `dashed`
/// renders each projected segment as a pixel-space dash pattern
/// (construction geometry, selection box).
fn push_polyline(
    out: &mut Vec<ScreenSpaceOverlay>,
    proj: &SketchProjector,
    pts: impl Iterator<Item = Vec2D>,
    color: [f32; 3],
    thickness: f32,
    dashed: bool,
) {
    let mut prev: Option<[f32; 2]> = None;
    for p in pts {
        let px = proj.to_px(p);
        if let (Some(a), Some(b)) = (prev, px) {
            push_segment_px(out, a, b, color, thickness, dashed);
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

/// Color + thickness + dashing for one element. Selection and hover win;
/// construction geometry is drawn blue, thinner and dashed (made
/// unmistakable by the dashes); everything else turns green once the
/// sketch is fully constrained.
fn element_style(
    sketch: &Sketch,
    id: Uuid,
    selected: &HashSet<Uuid>,
    hovered: Option<Uuid>,
) -> ([f32; 3], f32, bool) {
    if selected.contains(&id) {
        (COLOR_SELECTED, 2.0, false)
    } else if hovered == Some(id) {
        (COLOR_HOVERED, 2.0, false)
    } else if sketch.is_construction(id) {
        (COLOR_CONSTRUCTION, 1.0, true)
    } else if sketch.is_fully_constrained {
        (COLOR_FULLY_CONSTRAINED, 2.0, false)
    } else {
        (COLOR_GEOMETRY, 2.0, false)
    }
}

/// Small diamond marker: the pending point-on-curve auto-constraint hint at
/// the projected snap position.
fn push_diamond_marker(
    out: &mut Vec<ScreenSpaceOverlay>,
    proj: &SketchProjector,
    pos: Vec2D,
    color: [f32; 3],
) {
    if let Some([x, y]) = proj.to_px(pos) {
        let h = POINT_HALF_PX + 2.5;
        let corners = [[x - h, y], [x, y - h], [x + h, y], [x, y + h], [x - h, y]];
        for pair in corners.windows(2) {
            out.push(ScreenSpaceOverlay::new(pair[0], pair[1], color, 2.0));
        }
    }
}

fn push_element(
    out: &mut Vec<ScreenSpaceOverlay>,
    proj: &SketchProjector,
    sketch: &Sketch,
    geom: &GeometryElement,
    color: [f32; 3],
    thickness: f32,
    dashed: bool,
) {
    match geom {
        // Point markers stay solid (a dashed 7px cross would just vanish).
        GeometryElement::Point(p) => push_point_marker(out, proj, p.position, color),
        GeometryElement::Line(l) => {
            if let (Some(a), Some(b)) =
                (sketch.point_position(l.start), sketch.point_position(l.end))
            {
                push_polyline(out, proj, [a, b].into_iter(), color, thickness, dashed);
            }
        }
        GeometryElement::Circle(c) => {
            if let Some(center) = sketch.point_position(c.center) {
                push_polyline(
                    out,
                    proj,
                    circle_points(center, c.radius),
                    color,
                    thickness,
                    dashed,
                );
            }
        }
        GeometryElement::Arc(a) => {
            if let (Some(c), Some(s), Some(e)) = (
                sketch.point_position(a.center),
                sketch.point_position(a.start),
                sketch.point_position(a.end),
            ) {
                push_polyline(out, proj, arc_points(c, s, e), color, thickness, dashed);
            }
        }
        GeometryElement::Ellipse(e) => {
            if let Some(c) = sketch.point_position(e.center) {
                push_polyline(
                    out,
                    proj,
                    geom2d::ellipse_points(c, e.major, e.ratio, CIRCLE_SEGMENTS).into_iter(),
                    color,
                    thickness,
                    dashed,
                );
            }
        }
        GeometryElement::BSpline(b) => {
            let ctrl: Option<Vec<Vec2D>> = b
                .control_points
                .iter()
                .map(|id| sketch.point_position(*id))
                .collect();
            if let Some(ctrl) = ctrl {
                push_polyline(
                    out,
                    proj,
                    geom2d::bspline_points(&ctrl, b.periodic, 64).into_iter(),
                    color,
                    thickness,
                    dashed,
                );
                // Control polygon: always dashed guide-style (visual only).
                let mut poly = ctrl.clone();
                if b.periodic {
                    poly.extend(ctrl.first().copied());
                }
                push_polyline(out, proj, poly.into_iter(), COLOR_CONSTRUCTION, 1.0, true);
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
        false,
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
        false,
    );
}

/// Ghost of every selected element under a similarity transform (previews
/// for the translate/rotate/scale/mirror tools).
fn push_ghost(
    out: &mut Vec<ScreenSpaceOverlay>,
    proj: &SketchProjector,
    sketch: &Sketch,
    selected: &HashSet<Uuid>,
    xf: &Similarity,
) {
    let pt = |id: Uuid| sketch.point_position(id).map(|p| xf.apply(p));
    for geom in &sketch.geometry {
        if !selected.contains(&geom.id()) {
            continue;
        }
        match geom {
            GeometryElement::Point(p) => {
                push_point_marker(out, proj, xf.apply(p.position), COLOR_PREVIEW);
            }
            GeometryElement::Line(l) => {
                if let (Some(a), Some(b)) = (pt(l.start), pt(l.end)) {
                    push_polyline(out, proj, [a, b].into_iter(), COLOR_PREVIEW, 1.5, false);
                }
            }
            GeometryElement::Circle(c) => {
                if let Some(center) = pt(c.center) {
                    push_polyline(
                        out,
                        proj,
                        circle_points(center, c.radius * xf.scale_factor()),
                        COLOR_PREVIEW,
                        1.5,
                        false,
                    );
                }
            }
            GeometryElement::Arc(a) => {
                if let (Some(c), Some(s), Some(e)) = (pt(a.center), pt(a.start), pt(a.end)) {
                    // Mirrored arcs read CW: swap the endpoints to draw CCW.
                    let (s, e) = if xf.flips_orientation() {
                        (e, s)
                    } else {
                        (s, e)
                    };
                    push_polyline(out, proj, arc_points(c, s, e), COLOR_PREVIEW, 1.5, false);
                }
            }
            GeometryElement::Ellipse(e) => {
                if let Some(c) = pt(e.center) {
                    push_polyline(
                        out,
                        proj,
                        geom2d::ellipse_points(c, xf.apply_vec(e.major), e.ratio, CIRCLE_SEGMENTS)
                            .into_iter(),
                        COLOR_PREVIEW,
                        1.5,
                        false,
                    );
                }
            }
            GeometryElement::BSpline(b) => {
                let ctrl: Option<Vec<Vec2D>> = b.control_points.iter().map(|id| pt(*id)).collect();
                if let Some(ctrl) = ctrl {
                    push_polyline(
                        out,
                        proj,
                        geom2d::bspline_points(&ctrl, b.periodic, 48).into_iter(),
                        COLOR_PREVIEW,
                        1.5,
                        false,
                    );
                }
            }
        }
    }
}

/// Preview of the in-progress tool shape from its anchors to `cursor`.
#[allow(clippy::too_many_arguments)]
fn push_preview(
    out: &mut Vec<ScreenSpaceOverlay>,
    proj: &SketchProjector,
    sketch: &Sketch,
    state: &ToolState,
    cursor: Vec2D,
    params: &ToolParams,
    selected: &HashSet<Uuid>,
) {
    let pos = |t: &SnapTarget| t.position(sketch);
    match state {
        ToolState::Idle => {}
        ToolState::LineFrom { from, .. } => {
            if let Some(a) = pos(from) {
                push_polyline(
                    out,
                    proj,
                    [a, cursor].into_iter(),
                    COLOR_PREVIEW,
                    1.5,
                    false,
                );
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
                    false,
                );
            }
        }
        ToolState::CircleFrom { center } => {
            if let Some(c) = pos(center) {
                let r = (cursor - c).to_glam().length();
                if r > 1e-6 {
                    push_polyline(out, proj, circle_points(c, r), COLOR_PREVIEW, 1.5, false);
                }
                push_point_marker(out, proj, c, COLOR_PREVIEW);
            }
        }
        ToolState::ArcCenter { center } => {
            if let Some(c) = pos(center) {
                push_point_marker(out, proj, c, COLOR_PREVIEW);
                push_polyline(
                    out,
                    proj,
                    [c, cursor].into_iter(),
                    COLOR_PREVIEW,
                    1.0,
                    false,
                );
            }
        }
        ToolState::ArcStart { center, start } => {
            if let (Some(c), Some(s)) = (pos(center), pos(start)) {
                let r = (s - c).to_glam().length();
                let dir = (cursor - c).to_glam();
                if r > 1e-6 && dir.length() > 1e-6 {
                    let end = Vec2D::from_glam(c.to_glam() + dir.normalize() * r);
                    push_polyline(out, proj, arc_points(c, s, end), COLOR_PREVIEW, 1.5, false);
                }
                push_point_marker(out, proj, c, COLOR_PREVIEW);
            }
        }
        ToolState::PolygonCenter { center } => {
            if let Some(c) = pos(center) {
                if (cursor - c).to_glam().length() > 1e-6 {
                    // The polygon rotates with the cursor: the first vertex
                    // follows it exactly, like the committed shape will.
                    let verts = polygon_vertices(c, cursor, params.polygon_sides);
                    let closed = verts.iter().copied().chain(verts.first().copied());
                    push_polyline(out, proj, closed, COLOR_PREVIEW, 1.5, false);
                }
                push_point_marker(out, proj, c, COLOR_PREVIEW);
            }
        }
        ToolState::SlotFrom { from } => {
            if let Some(a) = pos(from) {
                if let Some((p1, p2, p3, p4)) = slot_corners(a, cursor, params.slot_width) {
                    push_polyline(out, proj, [p1, p2].into_iter(), COLOR_PREVIEW, 1.5, false);
                    push_polyline(out, proj, [p3, p4].into_iter(), COLOR_PREVIEW, 1.5, false);
                    push_polyline(
                        out,
                        proj,
                        arc_points(cursor, p3, p2),
                        COLOR_PREVIEW,
                        1.5,
                        false,
                    );
                    push_polyline(out, proj, arc_points(a, p1, p4), COLOR_PREVIEW, 1.5, false);
                }
                push_point_marker(out, proj, a, COLOR_PREVIEW);
            }
        }
        ToolState::RectCenterAt { center } => {
            let o = Vec2D::new(2.0 * center.x - cursor.x, 2.0 * center.y - cursor.y);
            let b = Vec2D::new(cursor.x, o.y);
            let d = Vec2D::new(o.x, cursor.y);
            push_polyline(
                out,
                proj,
                [o, b, cursor, d, o].into_iter(),
                COLOR_PREVIEW,
                1.5,
                false,
            );
            push_point_marker(out, proj, *center, COLOR_PREVIEW);
        }
        ToolState::Circle3One { a } => {
            push_polyline(
                out,
                proj,
                [*a, cursor].into_iter(),
                COLOR_PREVIEW,
                1.0,
                false,
            );
        }
        ToolState::Circle3Two { a, b } => {
            match geom2d::circumcenter(a.to_glam(), b.to_glam(), cursor.to_glam()) {
                Some(c) => {
                    let center = Vec2D::from_glam(c);
                    let r = (a.to_glam() - c).length();
                    push_polyline(
                        out,
                        proj,
                        circle_points(center, r),
                        COLOR_PREVIEW,
                        1.5,
                        false,
                    );
                }
                None => {
                    push_polyline(out, proj, [*a, *b].into_iter(), COLOR_PREVIEW, 1.0, false);
                }
            }
        }
        ToolState::Arc3Start { start } => {
            if let Some(s) = pos(start) {
                push_polyline(
                    out,
                    proj,
                    [s, cursor].into_iter(),
                    COLOR_PREVIEW,
                    1.0,
                    false,
                );
            }
        }
        ToolState::Arc3End { start, end } => {
            if let (Some(s), Some(e)) = (pos(start), pos(end)) {
                match geom2d::circumcenter(s.to_glam(), e.to_glam(), cursor.to_glam()) {
                    Some(c) => {
                        let center = Vec2D::from_glam(c);
                        // The rim point picks the side, exactly like the tool.
                        let (a, b) = if geom2d::point_on_arc(
                            c,
                            s.to_glam(),
                            e.to_glam(),
                            cursor.to_glam(),
                        ) {
                            (s, e)
                        } else {
                            (e, s)
                        };
                        push_polyline(
                            out,
                            proj,
                            arc_points(center, a, b),
                            COLOR_PREVIEW,
                            1.5,
                            false,
                        );
                    }
                    None => {
                        push_polyline(out, proj, [s, e].into_iter(), COLOR_PREVIEW, 1.0, false);
                    }
                }
            }
        }
        ToolState::ArcSlotCenter { center } => {
            if let Some(c) = pos(center) {
                push_point_marker(out, proj, c, COLOR_PREVIEW);
                push_polyline(
                    out,
                    proj,
                    [c, cursor].into_iter(),
                    COLOR_PREVIEW,
                    1.0,
                    false,
                );
            }
        }
        ToolState::ArcSlotStart { center, start } => {
            if let (Some(c), Some(s)) = (pos(center), pos(start)) {
                if let Some(shape) = arc_slot_shape(c, s, cursor, params.slot_width) {
                    push_polyline(
                        out,
                        proj,
                        arc_points(c, shape.outer_a, shape.outer_b),
                        COLOR_PREVIEW,
                        1.5,
                        false,
                    );
                    push_polyline(
                        out,
                        proj,
                        arc_points(c, shape.inner_a, shape.inner_b),
                        COLOR_PREVIEW,
                        1.5,
                        false,
                    );
                    push_polyline(
                        out,
                        proj,
                        arc_points(shape.cap_b, shape.outer_b, shape.inner_b),
                        COLOR_PREVIEW,
                        1.5,
                        false,
                    );
                    push_polyline(
                        out,
                        proj,
                        arc_points(shape.cap_a, shape.inner_a, shape.outer_a),
                        COLOR_PREVIEW,
                        1.5,
                        false,
                    );
                } else {
                    push_polyline(out, proj, [c, s].into_iter(), COLOR_PREVIEW, 1.0, false);
                }
                push_point_marker(out, proj, c, COLOR_PREVIEW);
            }
        }
        ToolState::EllipseCenter { center } => {
            if let Some(c) = pos(center) {
                push_point_marker(out, proj, c, COLOR_PREVIEW);
                push_polyline(
                    out,
                    proj,
                    [c, cursor].into_iter(),
                    COLOR_PREVIEW,
                    1.0,
                    false,
                );
            }
        }
        ToolState::EllipseMajor { center, major_pos } => {
            if let Some(c) = pos(center) {
                let major = (*major_pos - c).to_glam();
                let minor = if major.length() > 1e-6 {
                    (major / major.length())
                        .perp_dot((cursor - c).to_glam())
                        .abs()
                } else {
                    0.0
                };
                if minor > 1e-6 {
                    let ratio = (minor / major.length()).min(1.0);
                    push_polyline(
                        out,
                        proj,
                        geom2d::ellipse_points(c, Vec2D::from_glam(major), ratio, CIRCLE_SEGMENTS)
                            .into_iter(),
                        COLOR_PREVIEW,
                        1.5,
                        false,
                    );
                }
                push_polyline(
                    out,
                    proj,
                    [c, *major_pos].into_iter(),
                    COLOR_PREVIEW,
                    1.0,
                    true,
                );
                push_point_marker(out, proj, c, COLOR_PREVIEW);
            }
        }
        ToolState::BSplineDraw { points } => {
            let mut ctrl: Vec<Vec2D> = points.iter().filter_map(pos).collect();
            ctrl.push(cursor);
            // Dashed control polygon + the spline it would produce.
            push_polyline(
                out,
                proj,
                ctrl.iter().copied(),
                COLOR_CONSTRUCTION,
                1.0,
                true,
            );
            push_polyline(
                out,
                proj,
                geom2d::bspline_points(&ctrl, params.bspline_periodic, 48).into_iter(),
                COLOR_PREVIEW,
                1.5,
                false,
            );
        }
        ToolState::TranslateFrom { base } => {
            push_polyline(
                out,
                proj,
                [*base, cursor].into_iter(),
                COLOR_PREVIEW,
                1.0,
                true,
            );
            let delta = (cursor - *base).to_glam();
            push_ghost(out, proj, sketch, selected, &Similarity::translation(delta));
        }
        ToolState::RotateCenter { center } => {
            push_point_marker(out, proj, *center, COLOR_PREVIEW);
            push_polyline(
                out,
                proj,
                [*center, cursor].into_iter(),
                COLOR_PREVIEW,
                1.0,
                true,
            );
        }
        ToolState::RotateRef { center, reference } => {
            let c = center.to_glam();
            let to = cursor.to_glam() - c;
            push_polyline(
                out,
                proj,
                [*center, cursor].into_iter(),
                COLOR_PREVIEW,
                1.0,
                true,
            );
            if to.length() > 1e-6 && (reference.to_glam() - c).length() > 1e-6 {
                let angle = (reference.to_glam() - c).angle_to(to);
                push_ghost(
                    out,
                    proj,
                    sketch,
                    selected,
                    &Similarity::rotation_about(c, angle),
                );
            }
        }
        ToolState::ScaleBase { base } => {
            push_point_marker(out, proj, *base, COLOR_PREVIEW);
            push_polyline(
                out,
                proj,
                [*base, cursor].into_iter(),
                COLOR_PREVIEW,
                1.0,
                true,
            );
        }
        ToolState::ScaleRef { base, reference } => {
            let b = base.to_glam();
            let ref_len = (reference.to_glam() - b).length();
            if ref_len > 1e-6 {
                let factor = (cursor.to_glam() - b).length() / ref_len;
                if factor > 1e-4 {
                    push_ghost(
                        out,
                        proj,
                        sketch,
                        selected,
                        &Similarity::scale_about(b, factor),
                    );
                }
            }
        }
        ToolState::MirrorAxisFrom { a } => {
            push_polyline(
                out,
                proj,
                [*a, cursor].into_iter(),
                COLOR_PREVIEW,
                1.0,
                true,
            );
            if (cursor - *a).to_glam().length() > 1e-6 {
                push_ghost(
                    out,
                    proj,
                    sketch,
                    selected,
                    &Similarity::mirror_about(a.to_glam(), cursor.to_glam()),
                );
            }
        }
    }
}

/// Dashed rectangle for an in-progress box selection (corners in sketch
/// coordinates, drawn in the preview color).
fn push_selection_box(
    out: &mut Vec<ScreenSpaceOverlay>,
    proj: &SketchProjector,
    a: Vec2D,
    b: Vec2D,
) {
    let corners = [a, Vec2D::new(b.x, a.y), b, Vec2D::new(a.x, b.y), a];
    push_polyline(out, proj, corners.into_iter(), COLOR_PREVIEW, 1.0, true);
}

/// Build the full overlay set for one frame of sketch editing.
/// `selection_box` is an in-progress box selection (anchor, current corner)
/// in sketch coordinates; `active_tool`/`snap_tol` drive tool-specific
/// hover feedback (the trim tool highlights the span a click would remove).
#[allow(clippy::too_many_arguments)]
pub fn build_overlays(
    proj: &SketchProjector,
    sketch: &Sketch,
    selected: &HashSet<Uuid>,
    hovered: Option<Uuid>,
    tool_state: &ToolState,
    cursor: Option<Vec2D>,
    params: &ToolParams,
    selection_box: Option<(Vec2D, Vec2D)>,
    active_tool: Option<&str>,
    snap_tol: f32,
) -> Vec<ScreenSpaceOverlay> {
    let mut out = Vec::new();
    push_axes(&mut out, proj);

    // Curves first, then points on top so vertices stay visible.
    for geom in &sketch.geometry {
        if !matches!(geom, GeometryElement::Point(_)) {
            let (color, thickness, dashed) = element_style(sketch, geom.id(), selected, hovered);
            push_element(&mut out, proj, sketch, geom, color, thickness, dashed);
        }
    }
    for geom in &sketch.geometry {
        if matches!(geom, GeometryElement::Point(_)) {
            let (color, thickness, dashed) = element_style(sketch, geom.id(), selected, hovered);
            push_element(&mut out, proj, sketch, geom, color, thickness, dashed);
        }
    }

    if let Some((a, b)) = selection_box {
        push_selection_box(&mut out, proj, a, b);
    } else if let Some(cursor) = cursor {
        push_preview(&mut out, proj, sketch, tool_state, cursor, params, selected);
        if active_tool == Some("sketch.trim") {
            if let Some(span) = tools::trim_preview(sketch, cursor, snap_tol) {
                push_polyline(&mut out, proj, span.into_iter(), COLOR_TRIM, 3.0, false);
            }
        }
        // Pending auto-constraint hint: tools that attach new points onto
        // curves show a diamond at the projected snap position (only when
        // no point snap wins — shared point ids need no constraint).
        if matches!(
            active_tool,
            Some("sketch.line" | "sketch.circle" | "sketch.rect" | "sketch.arc")
        ) && matches!(
            crate::snap::snap_to_point(sketch, cursor, snap_tol, &[]),
            SnapTarget::New(_)
        ) {
            if let Some((_, projected)) = crate::snap::snap_to_curve(sketch, cursor, snap_tol, &[])
            {
                push_diamond_marker(&mut out, proj, projected, COLOR_AUTO_CONSTRAINT);
            }
        }
    }
    out
}
