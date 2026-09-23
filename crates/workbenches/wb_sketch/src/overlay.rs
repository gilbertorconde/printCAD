//! Screen-space overlay generation for the sketch under edit.
//!
//! While a sketch is being edited its geometry is drawn as constant-width
//! 2D lines (crisp at any zoom); the 3D tessellation is used only for
//! sketches *not* being edited. Colors come from the runtime context's
//! sketch palette.

use std::collections::HashSet;

use core_document::{ScreenSpaceMark, ScreenSpaceOverlay, SketchPalette, WorkbenchRuntimeContext};
use uuid::Uuid;

use crate::geom2d;
use crate::sketch::{GeometryElement, Sketch, SketchPlane, Vec2D};
use crate::snap::{SnapTarget, arc_angles};
use crate::tools::{
    self, Similarity, ToolParams, ToolState, arc_slot_shape, polygon_vertices, slot_corners,
};

const CIRCLE_SEGMENTS: usize = 48;
const ARC_SEGMENTS: usize = 32;
/// Point marker radius in pixels; centers of circles and arcs draw smaller.
const POINT_RADIUS_PX: f32 = 3.5;
const CENTER_RADIUS_PX: f32 = 3.0;
const AXIS_EXTENT_UNITS: f32 = 1.0e3;
const AXIS_ALPHA: f32 = 0.55;
/// Dash pattern for construction geometry, in viewport pixels (the painter
/// dashes after projection so it is zoom-independent).
const CONSTRUCTION_DASH: (f32, f32) = (5.0, 4.0);
/// Dash pattern for guides: the selection box, transform anchors.
const GUIDE_DASH: (f32, f32) = (6.0, 4.0);
const CROSSHAIR_PX: f32 = 20.0;

/// Everything the sketch draws in one frame: lines, and point or icon
/// marks on top of them.
#[derive(Debug, Default, Clone)]
pub struct Overlays {
    pub lines: Vec<ScreenSpaceOverlay>,
    pub marks: Vec<ScreenSpaceMark>,
}

/// How one element draws: color, width and dash pattern.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ElementStyle {
    pub color: [f32; 3],
    pub thickness: f32,
    pub dash: Option<(f32, f32)>,
}

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

/// Emit one projected segment; the painter applies `dash`.
fn push_segment_px(
    out: &mut Vec<ScreenSpaceOverlay>,
    a: [f32; 2],
    b: [f32; 2],
    color: [f32; 3],
    thickness: f32,
    dash: Option<(f32, f32)>,
) {
    let mut seg = ScreenSpaceOverlay::new(a, b, color, thickness);
    seg.dash = dash;
    out.push(seg);
}

/// A polyline's dash pattern: `true` is the guide dash, `false` solid, and
/// an element style passes its own pattern.
#[derive(Debug, Clone, Copy)]
pub struct Dash(pub Option<(f32, f32)>);

impl From<bool> for Dash {
    fn from(on: bool) -> Self {
        Dash(on.then_some(GUIDE_DASH))
    }
}

impl From<Option<(f32, f32)>> for Dash {
    fn from(dash: Option<(f32, f32)>) -> Self {
        Dash(dash)
    }
}

/// Emit a polyline between sketch points as overlay segments.
fn push_polyline(
    out: &mut Vec<ScreenSpaceOverlay>,
    proj: &SketchProjector,
    pts: impl Iterator<Item = Vec2D>,
    color: [f32; 3],
    thickness: f32,
    dash: impl Into<Dash>,
) {
    let dash = dash.into().0;
    let mut prev: Option<[f32; 2]> = None;
    for p in pts {
        let px = proj.to_px(p);
        if let (Some(a), Some(b)) = (prev, px) {
            push_segment_px(out, a, b, color, thickness, dash);
        }
        prev = px;
    }
}

/// A filled dot at a sketch point.
fn push_point_marker(out: &mut Overlays, proj: &SketchProjector, pos: Vec2D, color: [f32; 3]) {
    push_dot(out, proj, pos, color, POINT_RADIUS_PX);
}

fn push_dot(out: &mut Overlays, proj: &SketchProjector, pos: Vec2D, color: [f32; 3], radius: f32) {
    if let Some(px) = proj.to_px(pos) {
        out.marks.push(ScreenSpaceMark::dot(px, radius, color));
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

/// Color, width and dashing for one element. Selection and preselection
/// win; construction geometry is thinner and dashed; everything else takes
/// the fully-constrained color once the sketch has no freedom left.
pub fn element_style(
    sketch: &Sketch,
    id: Uuid,
    selected: &HashSet<Uuid>,
    hovered: Option<Uuid>,
    pal: &SketchPalette,
) -> ElementStyle {
    let (color, thickness, dash) = if selected.contains(&id) {
        (pal.selected, 2.5, None)
    } else if hovered == Some(id) {
        (pal.preselect, 2.0, None)
    } else if sketch.is_construction(id) {
        (pal.construction, 1.5, Some(CONSTRUCTION_DASH))
    } else if sketch.is_fully_constrained {
        (pal.fully_constrained, 2.0, None)
    } else {
        (pal.geometry, 2.0, None)
    };
    ElementStyle {
        color,
        thickness,
        dash,
    }
}

/// Small diamond marker: the pending point-on-curve auto-constraint hint at
/// the projected snap position.
fn push_diamond_marker(out: &mut Overlays, proj: &SketchProjector, pos: Vec2D, color: [f32; 3]) {
    if let Some([x, y]) = proj.to_px(pos) {
        let h = POINT_RADIUS_PX + 2.5;
        let corners = [[x - h, y], [x, y - h], [x + h, y], [x, y + h], [x - h, y]];
        for pair in corners.windows(2) {
            out.lines
                .push(ScreenSpaceOverlay::new(pair[0], pair[1], color, 2.0));
        }
    }
}

fn push_element(
    out: &mut Overlays,
    proj: &SketchProjector,
    pal: &SketchPalette,
    sketch: &Sketch,
    geom: &GeometryElement,
    style: ElementStyle,
    centers: &HashSet<Uuid>,
) {
    let ElementStyle {
        color,
        thickness,
        dash,
    } = style;
    let dashed = dash;
    match geom {
        // Centers of circles and arcs draw a little smaller than vertices.
        GeometryElement::Point(p) => {
            let radius = if centers.contains(&p.id) {
                CENTER_RADIUS_PX
            } else {
                POINT_RADIUS_PX
            };
            push_dot(out, proj, p.position, color, radius);
        }
        GeometryElement::Line(l) => {
            if let (Some(a), Some(b)) =
                (sketch.point_position(l.start), sketch.point_position(l.end))
            {
                push_polyline(
                    &mut out.lines,
                    proj,
                    [a, b].into_iter(),
                    color,
                    thickness,
                    dashed,
                );
            }
        }
        GeometryElement::Circle(c) => {
            if let Some(center) = sketch.point_position(c.center) {
                push_polyline(
                    &mut out.lines,
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
                push_polyline(
                    &mut out.lines,
                    proj,
                    arc_points(c, s, e),
                    color,
                    thickness,
                    dashed,
                );
            }
        }
        GeometryElement::Ellipse(e) => {
            if let Some(points) = e.points(sketch, CIRCLE_SEGMENTS) {
                push_polyline(
                    &mut out.lines,
                    proj,
                    points.into_iter(),
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
                    &mut out.lines,
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
                push_polyline(
                    &mut out.lines,
                    proj,
                    poly.into_iter(),
                    pal.construction,
                    1.0,
                    true,
                );
            }
        }
    }
}

/// The sketch's reference geometry: the two axes and the origin they cross
/// at. Faint until picked — they take constraints like anything else, so
/// selection and hover have to read.
fn push_axes(
    out: &mut Overlays,
    proj: &SketchProjector,
    pal: &SketchPalette,
    selected: &HashSet<Uuid>,
    hovered: Option<Uuid>,
) {
    let state = |id: Uuid, base: [f32; 3]| {
        if selected.contains(&id) {
            (pal.selected, 2.0, 1.0)
        } else if hovered == Some(id) {
            (pal.preselect, 1.5, 1.0)
        } else {
            (base, 1.0, AXIS_ALPHA)
        }
    };
    let axes = [
        (
            crate::sketch::X_AXIS_ID,
            Vec2D::new(-AXIS_EXTENT_UNITS, 0.0),
            Vec2D::new(AXIS_EXTENT_UNITS, 0.0),
            pal.axis_x,
        ),
        (
            crate::sketch::Y_AXIS_ID,
            Vec2D::new(0.0, -AXIS_EXTENT_UNITS),
            Vec2D::new(0.0, AXIS_EXTENT_UNITS),
            pal.axis_y,
        ),
    ];
    for (id, a, b, base) in axes {
        let (color, thickness, alpha) = state(id, base);
        if let (Some(pa), Some(pb)) = (proj.to_px(a), proj.to_px(b)) {
            out.lines
                .push(ScreenSpaceOverlay::new(pa, pb, color, thickness).with_alpha(alpha));
        }
    }
    let (color, _, alpha) = state(crate::sketch::ORIGIN_ID, pal.reference);
    if let Some(p) = proj.to_px(Vec2D::new(0.0, 0.0)) {
        let picked = selected.contains(&crate::sketch::ORIGIN_ID)
            || hovered == Some(crate::sketch::ORIGIN_ID);
        out.marks
            .push(ScreenSpaceMark::dot(p, if picked { 4.5 } else { 3.0 }, color).with_alpha(alpha));
    }
}

/// Ghost of every selected element under a similarity transform (previews
/// for the translate/rotate/scale/mirror tools).
fn push_ghost(
    out: &mut Overlays,
    proj: &SketchProjector,
    pal: &SketchPalette,
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
                push_point_marker(out, proj, xf.apply(p.position), pal.preview);
            }
            GeometryElement::Line(l) => {
                if let (Some(a), Some(b)) = (pt(l.start), pt(l.end)) {
                    push_polyline(
                        &mut out.lines,
                        proj,
                        [a, b].into_iter(),
                        pal.preview,
                        1.5,
                        false,
                    );
                }
            }
            GeometryElement::Circle(c) => {
                if let Some(center) = pt(c.center) {
                    push_polyline(
                        &mut out.lines,
                        proj,
                        circle_points(center, c.radius * xf.scale_factor()),
                        pal.preview,
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
                    push_polyline(
                        &mut out.lines,
                        proj,
                        arc_points(c, s, e),
                        pal.preview,
                        1.5,
                        false,
                    );
                }
            }
            // The curve's own samples carried by the transform: exact for an
            // arc too, and for a mirror, which turns an arc's direction.
            GeometryElement::Ellipse(e) => {
                if let Some(points) = e.points(sketch, CIRCLE_SEGMENTS) {
                    push_polyline(
                        &mut out.lines,
                        proj,
                        points.into_iter().map(|p| xf.apply(p)),
                        pal.preview,
                        1.5,
                        false,
                    );
                }
            }
            GeometryElement::BSpline(b) => {
                let ctrl: Option<Vec<Vec2D>> = b.control_points.iter().map(|id| pt(*id)).collect();
                if let Some(ctrl) = ctrl {
                    push_polyline(
                        &mut out.lines,
                        proj,
                        geom2d::bspline_points(&ctrl, b.periodic, 48).into_iter(),
                        pal.preview,
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
    out: &mut Overlays,
    proj: &SketchProjector,
    pal: &SketchPalette,
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
                    &mut out.lines,
                    proj,
                    [a, cursor].into_iter(),
                    pal.preview,
                    1.5,
                    false,
                );
            }
        }
        ToolState::PolylineFrom {
            from, heading, arc, ..
        } => {
            if let Some(a) = pos(from) {
                let arc_points = heading
                    .filter(|_| *arc)
                    .and_then(|h| geom2d::tangent_arc(a, h, cursor))
                    .map(|(c, r, ccw)| {
                        let angle = |p: Vec2D| (p.y - c.y).atan2(p.x - c.x);
                        let (t0, mut t1) = (angle(a), angle(cursor));
                        if ccw {
                            while t1 <= t0 {
                                t1 += std::f32::consts::TAU;
                            }
                        } else {
                            while t1 >= t0 {
                                t1 -= std::f32::consts::TAU;
                            }
                        }
                        (0..=CIRCLE_SEGMENTS)
                            .map(|i| {
                                let t = t0 + (t1 - t0) * i as f32 / CIRCLE_SEGMENTS as f32;
                                Vec2D::new(c.x + r * t.cos(), c.y + r * t.sin())
                            })
                            .collect::<Vec<_>>()
                    });
                let points = arc_points.unwrap_or_else(|| vec![a, cursor]);
                push_polyline(
                    &mut out.lines,
                    proj,
                    points.into_iter(),
                    pal.preview,
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
                    &mut out.lines,
                    proj,
                    [a, b, cursor, d, a].into_iter(),
                    pal.preview,
                    1.5,
                    false,
                );
            }
        }
        ToolState::CircleFrom { center } => {
            if let Some(c) = pos(center) {
                let r = (cursor - c).to_glam().length();
                if r > 1e-6 {
                    push_polyline(
                        &mut out.lines,
                        proj,
                        circle_points(c, r),
                        pal.preview,
                        1.5,
                        false,
                    );
                }
                push_point_marker(out, proj, c, pal.preview);
            }
        }
        ToolState::ArcCenter { center } => {
            if let Some(c) = pos(center) {
                push_point_marker(out, proj, c, pal.preview);
                push_polyline(
                    &mut out.lines,
                    proj,
                    [c, cursor].into_iter(),
                    pal.preview,
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
                    push_polyline(
                        &mut out.lines,
                        proj,
                        arc_points(c, s, end),
                        pal.preview,
                        1.5,
                        false,
                    );
                }
                push_point_marker(out, proj, c, pal.preview);
            }
        }
        ToolState::PolygonCenter { center } => {
            if let Some(c) = pos(center) {
                if (cursor - c).to_glam().length() > 1e-6 {
                    // The polygon rotates with the cursor: the first vertex
                    // follows it exactly, like the committed shape will.
                    let verts = polygon_vertices(c, cursor, params.polygon_sides);
                    let closed = verts.iter().copied().chain(verts.first().copied());
                    push_polyline(&mut out.lines, proj, closed, pal.preview, 1.5, false);
                }
                push_point_marker(out, proj, c, pal.preview);
            }
        }
        ToolState::SlotFrom { from } => {
            if let Some(a) = pos(from) {
                if let Some((p1, p2, p3, p4)) = slot_corners(a, cursor, params.slot_width) {
                    push_polyline(
                        &mut out.lines,
                        proj,
                        [p1, p2].into_iter(),
                        pal.preview,
                        1.5,
                        false,
                    );
                    push_polyline(
                        &mut out.lines,
                        proj,
                        [p3, p4].into_iter(),
                        pal.preview,
                        1.5,
                        false,
                    );
                    push_polyline(
                        &mut out.lines,
                        proj,
                        arc_points(cursor, p3, p2),
                        pal.preview,
                        1.5,
                        false,
                    );
                    push_polyline(
                        &mut out.lines,
                        proj,
                        arc_points(a, p1, p4),
                        pal.preview,
                        1.5,
                        false,
                    );
                }
                push_point_marker(out, proj, a, pal.preview);
            }
        }
        ToolState::RectCenterAt { center } => {
            let o = Vec2D::new(2.0 * center.x - cursor.x, 2.0 * center.y - cursor.y);
            let b = Vec2D::new(cursor.x, o.y);
            let d = Vec2D::new(o.x, cursor.y);
            push_polyline(
                &mut out.lines,
                proj,
                [o, b, cursor, d, o].into_iter(),
                pal.preview,
                1.5,
                false,
            );
            push_point_marker(out, proj, *center, pal.preview);
        }
        ToolState::Circle3One { a } => {
            push_polyline(
                &mut out.lines,
                proj,
                [*a, cursor].into_iter(),
                pal.preview,
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
                        &mut out.lines,
                        proj,
                        circle_points(center, r),
                        pal.preview,
                        1.5,
                        false,
                    );
                }
                None => {
                    push_polyline(
                        &mut out.lines,
                        proj,
                        [*a, *b].into_iter(),
                        pal.preview,
                        1.0,
                        false,
                    );
                }
            }
        }
        ToolState::Arc3Start { start } => {
            if let Some(s) = pos(start) {
                push_polyline(
                    &mut out.lines,
                    proj,
                    [s, cursor].into_iter(),
                    pal.preview,
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
                            &mut out.lines,
                            proj,
                            arc_points(center, a, b),
                            pal.preview,
                            1.5,
                            false,
                        );
                    }
                    None => {
                        push_polyline(
                            &mut out.lines,
                            proj,
                            [s, e].into_iter(),
                            pal.preview,
                            1.0,
                            false,
                        );
                    }
                }
            }
        }
        ToolState::ArcSlotCenter { center } => {
            if let Some(c) = pos(center) {
                push_point_marker(out, proj, c, pal.preview);
                push_polyline(
                    &mut out.lines,
                    proj,
                    [c, cursor].into_iter(),
                    pal.preview,
                    1.0,
                    false,
                );
            }
        }
        ToolState::ArcSlotStart { center, start } => {
            if let (Some(c), Some(s)) = (pos(center), pos(start)) {
                if let Some(shape) = arc_slot_shape(c, s, cursor, params.slot_width) {
                    push_polyline(
                        &mut out.lines,
                        proj,
                        arc_points(c, shape.outer_a, shape.outer_b),
                        pal.preview,
                        1.5,
                        false,
                    );
                    push_polyline(
                        &mut out.lines,
                        proj,
                        arc_points(c, shape.inner_a, shape.inner_b),
                        pal.preview,
                        1.5,
                        false,
                    );
                    push_polyline(
                        &mut out.lines,
                        proj,
                        arc_points(shape.cap_b, shape.outer_b, shape.inner_b),
                        pal.preview,
                        1.5,
                        false,
                    );
                    push_polyline(
                        &mut out.lines,
                        proj,
                        arc_points(shape.cap_a, shape.inner_a, shape.outer_a),
                        pal.preview,
                        1.5,
                        false,
                    );
                } else {
                    push_polyline(
                        &mut out.lines,
                        proj,
                        [c, s].into_iter(),
                        pal.preview,
                        1.0,
                        false,
                    );
                }
                push_point_marker(out, proj, c, pal.preview);
            }
        }
        ToolState::EllipseCenter { center } => {
            if let Some(c) = pos(center) {
                push_point_marker(out, proj, c, pal.preview);
                push_polyline(
                    &mut out.lines,
                    proj,
                    [c, cursor].into_iter(),
                    pal.preview,
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
                        &mut out.lines,
                        proj,
                        geom2d::ellipse_points(c, Vec2D::from_glam(major), ratio, CIRCLE_SEGMENTS)
                            .into_iter(),
                        pal.preview,
                        1.5,
                        false,
                    );
                }
                push_polyline(
                    &mut out.lines,
                    proj,
                    [c, *major_pos].into_iter(),
                    pal.preview,
                    1.0,
                    true,
                );
                push_point_marker(out, proj, c, pal.preview);
            }
        }
        ToolState::Ellipse3A { a } => {
            push_point_marker(out, proj, *a, pal.preview);
            push_polyline(
                &mut out.lines,
                proj,
                [*a, cursor].into_iter(),
                pal.preview,
                1.0,
                true,
            );
        }
        ToolState::Ellipse3B { a, b } => {
            let center = Vec2D::from_glam((a.to_glam() + b.to_glam()) * 0.5);
            if let Some((major, ratio)) = geom2d::ellipse_through(center, *b, cursor) {
                push_polyline(
                    &mut out.lines,
                    proj,
                    geom2d::ellipse_points(center, major, ratio, CIRCLE_SEGMENTS).into_iter(),
                    pal.preview,
                    1.5,
                    false,
                );
            }
            push_polyline(
                &mut out.lines,
                proj,
                [*a, *b].into_iter(),
                pal.preview,
                1.0,
                true,
            );
            push_point_marker(out, proj, *a, pal.preview);
            push_point_marker(out, proj, *b, pal.preview);
        }
        ToolState::EllipseArcCenter { center } => {
            if let Some(c) = pos(center) {
                push_point_marker(out, proj, c, pal.preview);
                push_polyline(
                    &mut out.lines,
                    proj,
                    [c, cursor].into_iter(),
                    pal.preview,
                    1.0,
                    false,
                );
            }
        }
        ToolState::EllipseArcMajor { center, major_pos } => {
            if let Some(c) = pos(center) {
                if let Some((major, ratio)) = geom2d::ellipse_through(c, *major_pos, cursor) {
                    push_polyline(
                        &mut out.lines,
                        proj,
                        geom2d::ellipse_points(c, major, ratio, CIRCLE_SEGMENTS).into_iter(),
                        pal.preview,
                        1.0,
                        true,
                    );
                    push_point_marker(out, proj, cursor, pal.preview);
                }
                push_polyline(
                    &mut out.lines,
                    proj,
                    [c, *major_pos].into_iter(),
                    pal.preview,
                    1.0,
                    true,
                );
                push_point_marker(out, proj, c, pal.preview);
            }
        }
        ToolState::EllipseArcStart {
            center,
            major,
            ratio,
            start,
        } => {
            if let Some(c) = pos(center) {
                // The whole ellipse faintly, the arc it will keep firmly.
                push_polyline(
                    &mut out.lines,
                    proj,
                    geom2d::ellipse_points(c, *major, *ratio, CIRCLE_SEGMENTS).into_iter(),
                    pal.preview,
                    1.0,
                    true,
                );
                let probe = crate::sketch::Ellipse::new(uuid::Uuid::nil(), *major, *ratio);
                let t0 = probe.param_at(c, *start);
                let mut t1 = probe.param_at(c, cursor);
                while t1 <= t0 {
                    t1 += std::f32::consts::TAU;
                }
                push_polyline(
                    &mut out.lines,
                    proj,
                    geom2d::ellipse_arc_points(c, *major, *ratio, t0, t1, CIRCLE_SEGMENTS)
                        .into_iter(),
                    pal.preview,
                    1.5,
                    false,
                );
                push_point_marker(out, proj, *start, pal.preview);
            }
        }
        ToolState::BSplineDraw { points } => {
            let mut ctrl: Vec<Vec2D> = points.iter().filter_map(pos).collect();
            ctrl.push(cursor);
            // Dashed control polygon + the spline it would produce.
            push_polyline(
                &mut out.lines,
                proj,
                ctrl.iter().copied(),
                pal.construction,
                1.0,
                true,
            );
            push_polyline(
                &mut out.lines,
                proj,
                geom2d::bspline_points(&ctrl, params.bspline_periodic, 48).into_iter(),
                pal.preview,
                1.5,
                false,
            );
        }
        ToolState::TranslateFrom { base } => {
            push_polyline(
                &mut out.lines,
                proj,
                [*base, cursor].into_iter(),
                pal.preview,
                1.0,
                true,
            );
            let delta = (cursor - *base).to_glam();
            push_ghost(
                out,
                proj,
                pal,
                sketch,
                selected,
                &Similarity::translation(delta),
            );
        }
        ToolState::RotateCenter { center } => {
            push_point_marker(out, proj, *center, pal.preview);
            push_polyline(
                &mut out.lines,
                proj,
                [*center, cursor].into_iter(),
                pal.preview,
                1.0,
                true,
            );
        }
        ToolState::RotateRef { center, reference } => {
            let c = center.to_glam();
            let to = cursor.to_glam() - c;
            push_polyline(
                &mut out.lines,
                proj,
                [*center, cursor].into_iter(),
                pal.preview,
                1.0,
                true,
            );
            if to.length() > 1e-6 && (reference.to_glam() - c).length() > 1e-6 {
                let angle = (reference.to_glam() - c).angle_to(to);
                push_ghost(
                    out,
                    proj,
                    pal,
                    sketch,
                    selected,
                    &Similarity::rotation_about(c, angle),
                );
            }
        }
        ToolState::ScaleBase { base } => {
            push_point_marker(out, proj, *base, pal.preview);
            push_polyline(
                &mut out.lines,
                proj,
                [*base, cursor].into_iter(),
                pal.preview,
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
                        pal,
                        sketch,
                        selected,
                        &Similarity::scale_about(b, factor),
                    );
                }
            }
        }
        ToolState::MirrorAxisFrom { a } => {
            push_polyline(
                &mut out.lines,
                proj,
                [*a, cursor].into_iter(),
                pal.preview,
                1.0,
                true,
            );
            if (cursor - *a).to_glam().length() > 1e-6 {
                push_ghost(
                    out,
                    proj,
                    pal,
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
    out: &mut Overlays,
    proj: &SketchProjector,
    pal: &SketchPalette,
    a: Vec2D,
    b: Vec2D,
) {
    let corners = [a, Vec2D::new(b.x, a.y), b, Vec2D::new(a.x, b.y), a];
    push_polyline(
        &mut out.lines,
        proj,
        corners.into_iter(),
        pal.preview,
        1.0,
        true,
    );
}

/// Build the full overlay set for one frame of sketch editing.
/// `selection_box` is an in-progress box selection (anchor, current corner)
/// in sketch coordinates; `active_tool`/`snap_tol` drive tool-specific
/// hover feedback (the trim tool highlights the span a click would remove).
#[allow(clippy::too_many_arguments)]
pub fn build_overlays(
    proj: &SketchProjector,
    pal: &SketchPalette,
    sketch: &Sketch,
    selected: &HashSet<Uuid>,
    hovered: Option<Uuid>,
    tool_state: &ToolState,
    cursor: Option<Vec2D>,
    params: &ToolParams,
    selection_box: Option<(Vec2D, Vec2D)>,
    active_tool: Option<&str>,
    snap_tol: f32,
    construction_on_top: bool,
) -> Overlays {
    let mut out = Overlays::default();
    push_axes(&mut out, proj, pal, selected, hovered);

    let centers: HashSet<Uuid> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Circle(c) => Some(c.center),
            GeometryElement::Arc(a) => Some(a.center),
            _ => None,
        })
        .collect();

    // Curves first, then points on top so vertices stay visible; within
    // each, whichever of construction and normal geometry is on top draws
    // last.
    let mut order: Vec<&GeometryElement> = sketch.geometry.iter().collect();
    order.sort_by_key(|g| sketch.is_construction(g.id()) == construction_on_top);
    for pass_points in [false, true] {
        for geom in order.iter().copied() {
            if matches!(geom, GeometryElement::Point(_)) != pass_points {
                continue;
            }
            let style = element_style(sketch, geom.id(), selected, hovered, pal);
            push_element(&mut out, proj, pal, sketch, geom, style, &centers);
        }
    }

    if let Some((a, b)) = selection_box {
        push_selection_box(&mut out, proj, pal, a, b);
    } else if let Some(cursor) = cursor {
        push_preview(
            &mut out, proj, pal, sketch, tool_state, cursor, params, selected,
        );
        if active_tool == Some("sketch.trim")
            && let Some(span) = tools::trim_preview(sketch, cursor, snap_tol)
        {
            push_polyline(&mut out.lines, proj, span.into_iter(), pal.trim, 3.0, false);
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
        ) && let Some((_, projected)) = crate::snap::snap_to_curve(sketch, cursor, snap_tol, &[])
        {
            push_diamond_marker(&mut out, proj, projected, pal.preselect);
        }
        // A crosshair follows the cursor while a drawing tool is armed.
        if active_tool.is_some_and(|t| t != "sketch.select")
            && let Some(px) = proj.to_px(cursor)
        {
            out.marks
                .push(ScreenSpaceMark::crosshair(px, CROSSHAIR_PX, pal.geometry));
        }
    }
    out
}
