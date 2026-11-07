//! Screen-space overlay generation for the sketch under edit.
//!
//! While a sketch is being edited its geometry is drawn as constant-width
//! 2D lines (crisp at any zoom); the 3D tessellation is used only for
//! sketches *not* being edited. Colors come from the runtime context's
//! sketch palette.

use std::collections::HashSet;

use core_document::{ScreenSpaceMark, ScreenSpaceOverlay, SketchPalette, WorkbenchRuntimeContext};
use uuid::Uuid;

use crate::appearance::Appearance;
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
const AXIS_EXTENT_UNITS: f64 = 1.0e3;
const AXIS_ALPHA: f32 = 0.55;
/// Dash pattern for construction geometry, in viewport pixels (the painter
/// dashes after projection so it is zoom-independent).
const CONSTRUCTION_DASH: (f32, f32) = (5.0, 4.0);
/// Dash pattern for guides: the selection box, transform anchors.
const GUIDE_DASH: (f32, f32) = (6.0, 4.0);
const CROSSHAIR_PX: f32 = 20.0;
/// The ring round a loose end.
const LOOSE_END_PX: f32 = 7.0;

/// Everything the sketch draws in one frame: lines, and point or icon
/// marks on top of them.
#[derive(Debug, Default, Clone)]
pub struct Overlays {
    pub lines: Vec<ScreenSpaceOverlay>,
    pub marks: Vec<ScreenSpaceMark>,
    pub labels: Vec<core_document::ScreenSpaceLabel>,
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

    pub fn to_world(&self, pos: Vec2D) -> [f64; 3] {
        let x_axis = glam::DVec3::from_array(self.plane.x_axis);
        let y_axis = glam::DVec3::from_array(self.plane.y_axis);
        let origin = glam::DVec3::from_array(self.plane.origin);
        (origin + x_axis * pos.x + y_axis * pos.y).to_array()
    }

    pub fn to_px(&self, pos: Vec2D) -> Option<[f32; 2]> {
        let (x, y) = self
            .ctx
            .world_to_viewport(crate::boundary::narrow3(self.to_world(pos)))?;
        Some([x, y])
    }

    /// Intersect a viewport-local pixel with this sketch's plane.
    pub fn sketch_at(&self, pos: [f32; 2]) -> Option<Vec2D> {
        let world = crate::boundary::ctx_cursor_on_plane(
            self.ctx,
            (pos[0], pos[1]),
            self.plane.origin,
            self.plane.normal,
        )?;
        let rel = glam::DVec3::from_array(world) - glam::DVec3::from_array(self.plane.origin);
        Some(Vec2D::new(
            rel.dot(glam::DVec3::from_array(self.plane.x_axis)),
            rel.dot(glam::DVec3::from_array(self.plane.y_axis)),
        ))
    }

    /// The box in sketch coordinates that holds what the viewport shows of
    /// the plane, a margin around it; `None` when a corner of the view
    /// does not meet the plane (seen edge on, or past its horizon).
    pub fn visible_extent(&self) -> Option<(Vec2D, Vec2D)> {
        let (_, _, w, h) = self.ctx.viewport;
        let (w, h) = (w as f32, h as f32);
        let corners = [[0.0, 0.0], [w, 0.0], [0.0, h], [w, h]]
            .into_iter()
            .map(|c| self.sketch_at(c))
            .collect::<Option<Vec<_>>>()?;
        let (mut lo, mut hi) = (corners[0], corners[0]);
        for c in &corners[1..] {
            lo = Vec2D::new(lo.x.min(c.x), lo.y.min(c.y));
            hi = Vec2D::new(hi.x.max(c.x), hi.y.max(c.y));
        }
        let margin = (hi.x - lo.x).max(hi.y - lo.y) * 0.1;
        Some((
            Vec2D::new(lo.x - margin, lo.y - margin),
            Vec2D::new(hi.x + margin, hi.y + margin),
        ))
    }

    /// Sketch units per screen pixel at the plane origin; used to convert
    /// pixel-based tolerances into sketch units. Falls back to a small
    /// constant when the projection is degenerate.
    pub fn units_per_px(&self) -> f64 {
        let origin = self.to_px(Vec2D::new(0.0, 0.0));
        let unit_x = self.to_px(Vec2D::new(1.0, 0.0));
        match (origin, unit_x) {
            (Some(o), Some(ux)) => {
                let px_per_unit =
                    f64::from(((ux[0] - o[0]).powi(2) + (ux[1] - o[1]).powi(2)).sqrt());
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
    out.extend(ScreenSpaceOverlay::polyline(
        pts.map(|p| proj.to_px(p)),
        color,
        thickness,
        dash.into().0,
    ));
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

fn circle_points(center: Vec2D, radius: f64) -> impl Iterator<Item = Vec2D> {
    (0..=CIRCLE_SEGMENTS).map(move |i| {
        let a = (i as f64 / CIRCLE_SEGMENTS as f64) * std::f64::consts::TAU;
        Vec2D::new(center.x + radius * a.cos(), center.y + radius * a.sin())
    })
}

fn arc_points(center: Vec2D, start: Vec2D, end: Vec2D) -> impl Iterator<Item = Vec2D> {
    let sv = (start - center).to_glam();
    let ev = (end - center).to_glam();
    let radius = sv.length();
    let (start_angle, sweep) = arc_angles(sv, ev);
    (0..=ARC_SEGMENTS).map(move |i| {
        let a = start_angle + sweep * (i as f64 / ARC_SEGMENTS as f64);
        Vec2D::new(center.x + radius * a.cos(), center.y + radius * a.sin())
    })
}

/// Color, width and dashing for one element. Selection and preselection
/// win; external geometry takes its own colour, dashed like construction
/// while it only guides and solid once it counts in the profile;
/// construction geometry is thinner and dashed; everything else takes
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
    } else if let Some(source) = sketch.external.get(&id) {
        if source.defining {
            (pal.external, 2.0, None)
        } else {
            (pal.external, 1.5, Some(CONSTRUCTION_DASH))
        }
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

/// Half the size of a snap marker, in pixels.
const SNAP_MARK_PX: f32 = POINT_RADIUS_PX + 3.0;

/// Half the height of a snap's name pill (11 px text and its padding),
/// and the room kept between it and the readout card, in pixels.
const SNAP_LABEL_HALF_HEIGHT_PX: f32 = 10.0;
const SNAP_LABEL_GAP_PX: f32 = 4.0;

/// The cue for where a click will land: a mark shaped for what it snapped
/// to (a square on an endpoint, a ring on a centre, a ringed cross on the
/// origin, an X on a crossing, a triangle on a midpoint, a square corner
/// at a perpendicular's foot, a ring and a touching line at a tangent
/// point, a diamond on a curve or an axis, two strokes for a parallel, an
/// upturned T for a square direction, a ringed dash on an extension), its
/// name beside it, clear of the readout card at the cursor (`cursor_px`),
/// and with `hint_lines` its guide: dashed back to the point being drawn
/// from when level or plumb with it, dashed along a parallel or square
/// direction with the line it follows lit, dashed from a line's end out
/// along its extension.
#[allow(clippy::too_many_arguments)]
fn push_snap_marker(
    out: &mut Overlays,
    proj: &SketchProjector,
    pal: &SketchPalette,
    sketch: &Sketch,
    snap: &crate::snap::Snap,
    from: Option<Vec2D>,
    hint_lines: bool,
    cursor_px: Option<[f32; 2]>,
) {
    use crate::snap::SnapKind;
    let Some(kind) = snap.kind else {
        return;
    };
    let Some([x, y]) = proj.to_px(snap.pos) else {
        return;
    };
    let color = pal.preselect;
    let h = SNAP_MARK_PX;
    let mut path = |points: &[[f32; 2]]| {
        for pair in points.windows(2) {
            out.lines
                .push(ScreenSpaceOverlay::new(pair[0], pair[1], color, 2.0));
        }
    };
    let ring = |r: f32| -> Vec<[f32; 2]> {
        (0..=24)
            .map(|i| {
                let t = i as f32 / 24.0 * std::f32::consts::TAU;
                [x + r * t.cos(), y + r * t.sin()]
            })
            .collect()
    };
    match kind {
        SnapKind::Endpoint => path(&[
            [x - h, y - h],
            [x + h, y - h],
            [x + h, y + h],
            [x - h, y + h],
            [x - h, y - h],
        ]),
        SnapKind::Center => path(&ring(h)),
        SnapKind::Quadrant(_) => {
            path(&[[x, y - h], [x + h, y], [x, y + h], [x - h, y], [x, y - h]])
        }
        SnapKind::Origin => {
            path(&ring(h));
            path(&[[x - h, y], [x + h, y]]);
            path(&[[x, y - h], [x, y + h]]);
        }
        SnapKind::Intersection => {
            path(&[[x - h, y - h], [x + h, y + h]]);
            path(&[[x - h, y + h], [x + h, y - h]]);
        }
        // Screen y runs down: the apex is above.
        SnapKind::Midpoint => path(&[
            [x, y - h],
            [x + h, y + h * 0.8],
            [x - h, y + h * 0.8],
            [x, y - h],
        ]),
        // A square corner: the foot of a perpendicular.
        SnapKind::Perpendicular => {
            path(&[[x - h, y + h], [x + h, y + h]]);
            path(&[[x, y + h], [x, y - h]]);
            path(&[
                [x, y + h * 0.4],
                [x + h * 0.6, y + h * 0.4],
                [x + h * 0.6, y + h],
            ]);
        }
        // A ring with a line touching its top.
        SnapKind::Tangent => {
            path(&ring(h * 0.7));
            path(&[[x - h * 1.3, y - h * 0.7], [x + h * 1.3, y - h * 0.7]]);
        }
        SnapKind::OnCurve | SnapKind::OnAxis => {
            path(&[[x - h, y], [x, y - h], [x + h, y], [x, y + h], [x - h, y]]);
        }
        SnapKind::Horizontal | SnapKind::Vertical => {}
        SnapKind::Parallel => {
            path(&[[x - h, y + h * 0.7], [x - h * 0.2, y - h * 0.7]]);
            path(&[[x + h * 0.2, y + h * 0.7], [x + h, y - h * 0.7]]);
        }
        SnapKind::PerpendicularTo => {
            path(&[[x - h, y + h], [x + h, y + h]]);
            path(&[[x, y + h], [x, y - h]]);
        }
        SnapKind::OnExtension => {
            path(&ring(h * 0.5));
            path(&[[x - h * 1.3, y], [x + h * 1.3, y]]);
        }
    }
    // A direction carried onto a curve marks the curve too.
    if snap.on.is_some() {
        path(&[[x - h, y], [x, y - h], [x + h, y], [x, y + h], [x - h, y]]);
    }
    if hint_lines {
        push_snap_guide(out, proj, pal, sketch, snap, kind, from, [x, y]);
    }
    // Above the marker, and above the readout card beside the cursor so
    // neither covers the other.
    let mut label_y = y - h - 8.0;
    if let Some([_, cy]) = cursor_px {
        let card_top = cy + crate::ovp::CARD_OFFSET_PX[1];
        label_y = label_y.min(card_top - SNAP_LABEL_GAP_PX - SNAP_LABEL_HALF_HEIGHT_PX);
    }
    let label = match snap.on {
        Some(on) => format!("{} · {}", kind.label(), on.label()),
        None => kind.label().to_string(),
    };
    out.labels.push(
        core_document::ScreenSpaceLabel::new([x + h + 30.0, label_y], label, color, 11.0).pill(),
    );
}

/// How far a parallel or square direction's guide runs past the point
/// drawn from and the point drawn to, in pixels.
const GUIDE_REACH_PX: f32 = 60.0;

/// The dashed guide a snap of `kind` at `at` (pixels) draws, if any.
#[allow(clippy::too_many_arguments)]
fn push_snap_guide(
    out: &mut Overlays,
    proj: &SketchProjector,
    pal: &SketchPalette,
    sketch: &Sketch,
    snap: &crate::snap::Snap,
    kind: crate::snap::SnapKind,
    from: Option<Vec2D>,
    at: [f32; 2],
) {
    use crate::snap::SnapKind;
    let color = pal.preselect;
    let start = from.and_then(|f| proj.to_px(f));
    let line_ends = snap.curve.and_then(|id| match sketch.get_geometry(id) {
        Some(GeometryElement::Line(l)) => sketch
            .point_position(l.start)
            .zip(sketch.point_position(l.end)),
        _ => None,
    });
    match kind {
        SnapKind::Horizontal | SnapKind::Vertical => {
            if let Some(start) = start {
                push_segment_px(&mut out.lines, start, at, color, 1.0, Some(GUIDE_DASH));
            }
        }
        SnapKind::Parallel | SnapKind::PerpendicularTo => {
            if let Some(start) = start {
                let (a, b) = (glam::Vec2::from(start), glam::Vec2::from(at));
                let u = (b - a).normalize_or_zero() * GUIDE_REACH_PX;
                push_segment_px(
                    &mut out.lines,
                    (a - u).into(),
                    (b + u).into(),
                    color,
                    1.0,
                    Some(GUIDE_DASH),
                );
            }
            if let Some((a, b)) = line_ends
                && let (Some(a), Some(b)) = (proj.to_px(a), proj.to_px(b))
            {
                push_segment_px(&mut out.lines, a, b, color, 2.0, None);
            }
        }
        SnapKind::OnExtension => {
            if let Some((a, b)) = line_ends {
                let p = snap.pos;
                let near = if (a - p).to_glam().length() < (b - p).to_glam().length() {
                    a
                } else {
                    b
                };
                if let Some(end) = proj.to_px(near) {
                    push_segment_px(&mut out.lines, end, at, color, 1.0, Some(GUIDE_DASH));
                }
            }
        }
        _ => {}
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
        GeometryElement::Conic(c) => {
            if let Some(points) = c.points(sketch, CIRCLE_SEGMENTS) {
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
            if let (Some(ctrl), Some(curve)) = (ctrl, b.polyline(sketch, 64)) {
                push_polyline(
                    &mut out.lines,
                    proj,
                    curve.into_iter(),
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
/// at. Faint until picked: they take constraints like anything else, so
/// selection and hover have to read.
fn push_axes(
    out: &mut Overlays,
    proj: &SketchProjector,
    pal: &SketchPalette,
    selected: &HashSet<Uuid>,
    hovered: Option<Uuid>,
    look: &Appearance,
) {
    let width = look.axis_width;
    let state = |id: Uuid, base: [f32; 3]| {
        if selected.contains(&id) {
            (pal.selected, width + 1.0, 1.0)
        } else if hovered == Some(id) {
            (pal.preselect, width + 0.5, 1.0)
        } else {
            (base, width, AXIS_ALPHA)
        }
    };
    // Each axis runs across the part of the plane the view shows, so both
    // its ends stay in front of the camera however close or turned it is;
    // a fixed far extent can put one behind it and lose the line.
    let (lo, hi) = proj.visible_extent().unwrap_or((
        Vec2D::new(-AXIS_EXTENT_UNITS, -AXIS_EXTENT_UNITS),
        Vec2D::new(AXIS_EXTENT_UNITS, AXIS_EXTENT_UNITS),
    ));
    let axes = [
        (
            crate::sketch::X_AXIS_ID,
            Vec2D::new(lo.x, 0.0),
            Vec2D::new(hi.x, 0.0),
            pal.axis_x,
            lo.y <= 0.0 && 0.0 <= hi.y,
        ),
        (
            crate::sketch::Y_AXIS_ID,
            Vec2D::new(0.0, lo.y),
            Vec2D::new(0.0, hi.y),
            pal.axis_y,
            lo.x <= 0.0 && 0.0 <= hi.x,
        ),
    ];
    for (id, a, b, base, shown) in axes {
        if !shown {
            continue;
        }
        let (color, thickness, alpha) = state(id, base);
        if let (Some(pa), Some(pb)) = (proj.to_px(a), proj.to_px(b)) {
            let mut line = ScreenSpaceOverlay::new(pa, pb, color, thickness).with_alpha(alpha);
            line.dash = Appearance::dash(look.axis_dashed);
            out.lines.push(line);
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
            GeometryElement::Conic(c) => {
                if let Some(points) = c.points(sketch, CIRCLE_SEGMENTS) {
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
                if let Some(points) = b.polyline(sketch, 48) {
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
        }
    }
}

/// The outline of the rectangle from `a` to `c` with its corners rounded
/// at `radius` (no more than half its shorter side, as the fillet can).
fn rounded_rect_points(a: Vec2D, c: Vec2D, radius: f64) -> Vec<Vec2D> {
    let (x0, x1) = (a.x.min(c.x), a.x.max(c.x));
    let (y0, y1) = (a.y.min(c.y), a.y.max(c.y));
    let r = radius.min((x1 - x0) * 0.5).min((y1 - y0) * 0.5).max(0.0);
    let mut out = Vec::new();
    // Counter-clockwise from the bottom right corner's arc.
    let corners = [
        (x1 - r, y0 + r, -90.0f64),
        (x1 - r, y1 - r, 0.0),
        (x0 + r, y1 - r, 90.0),
        (x0 + r, y0 + r, 180.0),
    ];
    for (cx, cy, start) in corners {
        for step in 0..=8 {
            let t = (start + step as f64 * 90.0 / 8.0).to_radians();
            out.push(Vec2D::new(cx + r * t.cos(), cy + r * t.sin()));
        }
    }
    out.push(out[0]);
    out
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
    active_tool: Option<&str>,
) {
    let pos = |t: &SnapTarget| t.position(sketch);
    match state {
        ToolState::FocalConicCenter { .. }
        | ToolState::FocalConicFocus { .. }
        | ToolState::FocalConicStart { .. } => {
            unreachable!("focal conic preview comes from the shared tool")
        }
        ToolState::Idle | ToolState::CornerFirst { .. } => {}
        ToolState::LineFrom { .. }
        | ToolState::PolylineFrom { .. }
        | ToolState::CircleFrom { .. } => unreachable!("shared drawing preview must be supplied"),
        ToolState::RectFrom { corner } => {
            assert!(matches!(
                active_tool,
                Some("sketch.rect_rounded" | "sketch.rect_frame")
            ));
            if let Some(a) = pos(corner) {
                let rounded = active_tool == Some("sketch.rect_rounded");
                let outline = if rounded {
                    rounded_rect_points(a, cursor, params.fillet_radius)
                } else {
                    let b = Vec2D::new(cursor.x, a.y);
                    let d = Vec2D::new(a.x, cursor.y);
                    vec![a, b, cursor, d, a]
                };
                push_polyline(
                    &mut out.lines,
                    proj,
                    outline.into_iter(),
                    pal.preview,
                    1.5,
                    false,
                );
                if active_tool == Some("sketch.rect_frame")
                    && let Some(inner) =
                        crate::tools::frame_inner_corners(a, cursor, params.offset_distance)
                {
                    push_polyline(
                        &mut out.lines,
                        proj,
                        inner.into_iter().chain([inner[0]]),
                        pal.preview,
                        1.5,
                        false,
                    );
                }
            }
        }
        ToolState::Rect3A { a } | ToolState::RectCenter3At { center: a } => {
            if let Some(a) = pos(a) {
                push_polyline(
                    &mut out.lines,
                    proj,
                    [a, cursor].into_iter(),
                    pal.preview,
                    1.5,
                    false,
                );
                push_point_marker(out, proj, a, pal.preview);
            }
        }
        ToolState::Rect3B { a, b } => {
            if let (Some(a), Some(b)) = (pos(a), pos(b)) {
                let outline = match crate::tools::rect3_corners(a, b, cursor) {
                    Some((c, d)) => vec![a, b, c, d, a],
                    None => vec![a, b],
                };
                push_polyline(
                    &mut out.lines,
                    proj,
                    outline.into_iter(),
                    pal.preview,
                    1.5,
                    false,
                );
            }
        }
        ToolState::RectCenter3Corner { center, corner } => {
            if let (Some(c), Some(k)) = (pos(center), pos(corner)) {
                match crate::tools::rect_center3_corners(c, k, cursor) {
                    Some(corners) => push_polyline(
                        &mut out.lines,
                        proj,
                        corners.into_iter().chain([corners[0]]),
                        pal.preview,
                        1.5,
                        false,
                    ),
                    None => push_polyline(
                        &mut out.lines,
                        proj,
                        [c, k].into_iter(),
                        pal.preview,
                        1.0,
                        true,
                    ),
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
            let Some(center) = center.position(sketch) else {
                return;
            };
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
            push_point_marker(out, proj, center, pal.preview);
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
                    let (major, ratio) = geom2d::ellipse_axes(major, minor);
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
            major_pos,
            start,
        } => {
            if let Some(c) = pos(center)
                && let Some((major, ratio)) = geom2d::ellipse_through(c, *major_pos, *start)
            {
                // The whole ellipse faintly, the arc it will keep firmly.
                push_polyline(
                    &mut out.lines,
                    proj,
                    geom2d::ellipse_points(c, major, ratio, CIRCLE_SEGMENTS).into_iter(),
                    pal.preview,
                    1.0,
                    true,
                );
                let probe = crate::sketch::Ellipse::new(uuid::Uuid::nil(), major, ratio);
                let t0 = probe.param_at(c, *start);
                let mut t1 = probe.param_at(c, cursor);
                while t1 <= t0 {
                    t1 += std::f64::consts::TAU;
                }
                push_polyline(
                    &mut out.lines,
                    proj,
                    geom2d::ellipse_arc_points(c, major, ratio, t0, t1, CIRCLE_SEGMENTS)
                        .into_iter(),
                    pal.preview,
                    1.5,
                    false,
                );
                // Where the commit puts the start: on the rim at its angle.
                let on_rim = geom2d::ellipse_arc_points(c, major, ratio, t0, t0, 1)
                    .first()
                    .copied()
                    .unwrap_or(*start);
                push_point_marker(out, proj, on_rim, pal.preview);
            }
        }
        ToolState::ConicAt { center, .. } => {
            if let Some(c) = pos(center) {
                push_point_marker(out, proj, c, pal.preview);
                push_polyline(
                    &mut out.lines,
                    proj,
                    [c, cursor].into_iter(),
                    pal.preview,
                    1.0,
                    true,
                );
            }
        }
        ToolState::ConicAxis { kind, center, axis } => {
            if let Some(c) = pos(center) {
                let minor = match kind {
                    crate::sketch::ConicKind::Parabola => Some(0.0),
                    crate::sketch::ConicKind::Hyperbola => {
                        crate::conic::hyperbola_minor(c, c + *axis, cursor)
                    }
                };
                if let Some(shape) =
                    minor.and_then(|m| crate::conic::Shape::new(*kind, c, *axis, m))
                {
                    let t = shape.param([cursor.x, cursor.y]);
                    let reach = t.abs().max(1.0) * 1.5;
                    push_polyline(
                        &mut out.lines,
                        proj,
                        shape.sample(-reach, reach, CIRCLE_SEGMENTS).into_iter(),
                        pal.preview,
                        1.0,
                        true,
                    );
                    let [x, y] = shape.point(t);
                    push_point_marker(out, proj, Vec2D::new(x, y), pal.preview);
                }
                push_polyline(
                    &mut out.lines,
                    proj,
                    [c, c + *axis].into_iter(),
                    pal.preview,
                    1.0,
                    true,
                );
                push_point_marker(out, proj, c, pal.preview);
            }
        }
        ToolState::ConicStart {
            kind,
            center,
            axis,
            minor,
            start,
        } => {
            if let Some(shape) =
                pos(center).and_then(|c| crate::conic::Shape::new(*kind, c, *axis, *minor))
            {
                let param = |p: Vec2D| shape.param([p.x, p.y]);
                let (t0, t1) = (param(*start), param(cursor));
                let reach = t0.abs().max(t1.abs()).max(1.0) * 1.5;
                // The whole curve faintly, the arc it will keep firmly.
                push_polyline(
                    &mut out.lines,
                    proj,
                    shape.sample(-reach, reach, CIRCLE_SEGMENTS).into_iter(),
                    pal.preview,
                    1.0,
                    true,
                );
                push_polyline(
                    &mut out.lines,
                    proj,
                    shape.sample(t0, t1, CIRCLE_SEGMENTS).into_iter(),
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
            // Dashed polygon through the clicks + the spline they would
            // produce, as the finished spline will be.
            push_polyline(
                &mut out.lines,
                proj,
                ctrl.iter().copied(),
                pal.construction,
                1.0,
                true,
            );
            if let Some(curve) = crate::tools::bspline_preview(&ctrl, params) {
                push_polyline(
                    &mut out.lines,
                    proj,
                    curve.into_iter(),
                    pal.preview,
                    1.5,
                    false,
                );
            }
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
            // One ghost per copy, as the click makes them (the moved
            // selection itself when there are none).
            let delta = (cursor - *base).to_glam();
            for m in crate::tools::pattern_multiples(params).unwrap_or_else(|| vec![1.0]) {
                push_ghost(
                    out,
                    proj,
                    pal,
                    sketch,
                    selected,
                    &Similarity::translation(delta * m),
                );
            }
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
                for m in crate::tools::pattern_multiples(params).unwrap_or_else(|| vec![1.0]) {
                    push_ghost(
                        out,
                        proj,
                        pal,
                        sketch,
                        selected,
                        &Similarity::rotation_about(c, angle * m),
                    );
                }
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
/// Whether a box from `anchor` to `current` was drawn right to left on
/// screen: one that takes whatever it touches, not only what it holds.
pub fn is_crossing(proj: &SketchProjector, anchor: Vec2D, current: Vec2D) -> bool {
    matches!((proj.to_px(anchor), proj.to_px(current)), (Some(a), Some(b)) if b[0] < a[0])
}

/// The box being drawn: solid while it takes what it holds whole, dashed
/// while it takes whatever it touches.
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
        is_crossing(proj, a, b),
    );
}

/// A ring in the conflict colour round every loose end of the profile's
/// curves: the points the profile leaves out with their curves.
fn push_loose_ends(
    out: &mut Overlays,
    proj: &SketchProjector,
    pal: &SketchPalette,
    sketch: &Sketch,
) {
    let radius = f64::from(LOOSE_END_PX) * proj.units_per_px();
    for id in crate::profile::loose_ends(sketch) {
        if let Some(at) = sketch.point_position(id) {
            push_polyline(
                &mut out.lines,
                proj,
                circle_points(at, radius),
                pal.conflict,
                1.5,
                false,
            );
        }
    }
}

fn push_shared_arc(
    out: &mut Overlays,
    proj: &SketchProjector,
    geometry: &sketch_runtime::conic_plan::ConicGeometry,
    color: [f32; 3],
) {
    let sketch_runtime::conic_plan::ConicGeometry::Arc { center, start, .. } = *geometry else {
        unreachable!("circular arc preview requires circular arc geometry")
    };
    let (a, b) = geometry.arc_angles();
    let r = center.distance(start).0;
    push_polyline(
        &mut out.lines,
        proj,
        (0..=64).map(|i| {
            let t = a.0 + (b.0 - a.0) * i as f64 / 64.;
            Vec2D::new(center.0.x + r * t.cos(), center.0.y + r * t.sin())
        }),
        color,
        1.5,
        false,
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
    referenced: &HashSet<Uuid>,
    hovered: Option<Uuid>,
    tool_state: &ToolState,
    cursor: Option<Vec2D>,
    params: &ToolParams,
    selection_box: Option<(Vec2D, Vec2D)>,
    active_tool: Option<&str>,
    snap_tol: f64,
    snap: Option<crate::snap::Snap>,
    construction_on_top: bool,
    look: &Appearance,
    hint_lines: bool,
    line_gesture: Option<&sketch_runtime::line::LinePreview>,
    circle_gesture: Option<&sketch_runtime::circle::CirclePreview>,
    rectangle_gesture: Option<&sketch_runtime::rect::RectPreview>,
    conic_gesture: Option<&sketch_runtime::conic::ConicPreview>,
) -> Overlays {
    let mut out = Overlays::default();
    push_axes(&mut out, proj, pal, selected, hovered, look);

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
            let mut style = element_style(sketch, geom.id(), selected, hovered, pal);
            if referenced.contains(&geom.id()) && !selected.contains(&geom.id()) {
                style.color = pal.constraint;
                style.thickness = 2.5;
            }
            push_element(&mut out, proj, pal, sketch, geom, style, &centers);
        }
    }

    // Where the profile stops: each loose end ringed, while no shape is
    // being drawn (mid-shape, the end drawn from is always loose).
    if tool_state.is_idle() {
        push_loose_ends(&mut out, proj, pal, sketch);
    }

    if let Some((a, b)) = selection_box {
        push_selection_box(&mut out, proj, pal, a, b);
    } else if let Some(raw) = cursor {
        // The preview runs to where the click would land.
        let cursor = snap.map_or(raw, |s| s.pos);
        if let Some(preview) = line_gesture {
            use sketch_runtime::line::LinePreview;
            if let LinePreview::Rubber { start, end } = preview {
                let host = |p: sketch_geometry::world::WorldPoint| Vec2D::new(p.0.x, p.0.y);
                push_polyline(
                    &mut out.lines,
                    proj,
                    [host(*start), host(end.pos)].into_iter(),
                    pal.preview,
                    1.5,
                    false,
                );
            }
            if let LinePreview::Arc { geometry, .. } = preview {
                let arc = sketch_runtime::conic_plan::ConicGeometry::Arc {
                    center: geometry.center,
                    start: geometry.start,
                    end: geometry.end,
                    clockwise: geometry.clockwise,
                };
                push_shared_arc(&mut out, proj, &arc, pal.preview);
            }
        } else if let Some(preview) = conic_gesture {
            use sketch_runtime::conic::ConicPreview;
            match preview {
                ConicPreview::Cursor { .. } => {}
                ConicPreview::Curve { plan, .. } => {
                    if let sketch_runtime::conic_plan::ConicGeometry::Circle { center, radius } =
                        plan.geometry
                    {
                        push_polyline(
                            &mut out.lines,
                            proj,
                            circle_points(crate::line_gesture::host(center), radius.0),
                            pal.preview,
                            1.5,
                            false,
                        );
                    } else if let sketch_runtime::conic_plan::ConicGeometry::Ellipse {
                        center,
                        focus1,
                        start,
                        end,
                        clockwise,
                    } = plan.geometry
                    {
                        let (major, ratio) =
                            crate::conic_gesture::ellipse_frame(center, focus1, start);
                        assert!(!clockwise, "host ellipse storage is CCW");
                        let points =
                            if plan.kind == sketch_runtime::conic_plan::ConicKind::FullEllipse {
                                geom2d::ellipse_points(
                                    crate::line_gesture::host(center),
                                    major,
                                    ratio,
                                    CIRCLE_SEGMENTS,
                                )
                            } else {
                                let center = crate::line_gesture::host(center);
                                let probe =
                                    crate::sketch::Ellipse::new(uuid::Uuid::nil(), major, ratio);
                                let t0 = probe.param_at(center, crate::line_gesture::host(start));
                                let mut t1 = probe.param_at(center, crate::line_gesture::host(end));
                                while t1 <= t0 {
                                    t1 += std::f64::consts::TAU;
                                }
                                geom2d::ellipse_arc_points(
                                    center,
                                    major,
                                    ratio,
                                    t0,
                                    t1,
                                    CIRCLE_SEGMENTS,
                                )
                            };
                        push_polyline(
                            &mut out.lines,
                            proj,
                            points.into_iter(),
                            pal.preview,
                            1.5,
                            false,
                        );
                    } else if matches!(
                        plan.geometry,
                        sketch_runtime::conic_plan::ConicGeometry::Hyperbola { .. }
                            | sketch_runtime::conic_plan::ConicGeometry::Parabola { .. }
                    ) {
                        use sketch_runtime::conic_plan::ConicGeometry;
                        let (kind, center, axis, minor, start, end) = match plan.geometry {
                            ConicGeometry::Hyperbola {
                                center,
                                focus1,
                                start,
                                end,
                                ..
                            } => {
                                let (axis, minor) =
                                    crate::conic_gesture::hyperbola_frame(center, focus1, start);
                                (
                                    crate::sketch::ConicKind::Hyperbola,
                                    center,
                                    axis,
                                    minor,
                                    start,
                                    end,
                                )
                            }
                            ConicGeometry::Parabola {
                                vertex,
                                focus,
                                start,
                                end,
                                ..
                            } => (
                                crate::sketch::ConicKind::Parabola,
                                vertex,
                                crate::line_gesture::host(focus)
                                    - crate::line_gesture::host(vertex),
                                0.,
                                start,
                                end,
                            ),
                            _ => {
                                unreachable!("focal conic preview must be a hyperbola or parabola")
                            }
                        };
                        let shape = crate::conic::Shape::new(
                            kind,
                            crate::line_gesture::host(center),
                            axis,
                            minor,
                        )
                        .unwrap();
                        push_polyline(
                            &mut out.lines,
                            proj,
                            shape
                                .sample(
                                    shape.param([start.0.x, start.0.y]),
                                    shape.param([end.0.x, end.0.y]),
                                    CIRCLE_SEGMENTS,
                                )
                                .into_iter(),
                            pal.preview,
                            1.5,
                            false,
                        );
                    } else {
                        push_shared_arc(&mut out, proj, &plan.geometry, pal.preview);
                    }
                }
                ConicPreview::Frame { picks, cursor }
                | ConicPreview::Rejected { picks, cursor, .. } => {
                    let rejected = matches!(preview, ConicPreview::Rejected { .. });
                    let color = if rejected { pal.conflict } else { pal.preview };
                    push_polyline(
                        &mut out.lines,
                        proj,
                        picks
                            .iter()
                            .chain(std::iter::once(cursor))
                            .map(|p| crate::line_gesture::host(p.pos)),
                        color,
                        1.5,
                        true,
                    );
                    if rejected
                        && let Some([x, y]) = proj.to_px(crate::line_gesture::host(cursor.pos))
                    {
                        out.labels.push(
                            core_document::ScreenSpaceLabel::new(
                                [x, y - 30.],
                                match preview {
                                    ConicPreview::Rejected { reason: sketch_runtime::conic_plan::ConicRejection::DegenerateConic, .. } => "Zero minor radius: choose a point off the first axis",
                                    ConicPreview::Rejected { reason: sketch_runtime::conic_plan::ConicRejection::NoAxis, .. } => "No axis: choose a different axis point",
                                    ConicPreview::Rejected { reason: sketch_runtime::conic_plan::ConicRejection::EmptySpan, .. } => "Empty arc: choose a different end point",
                                    _ => "Collinear points: choose a point off the chord",
                                },
                                color,
                                11.,
                            )
                            .pill(),
                        );
                    }
                }
            }
        } else if let Some(preview) = circle_gesture {
            if let sketch_runtime::circle::CirclePreview::Rubber { center, radius, .. } = preview {
                let center = crate::line_gesture::host(*center);
                push_polyline(
                    &mut out.lines,
                    proj,
                    circle_points(center, radius.0),
                    pal.preview,
                    1.5,
                    false,
                );
                push_point_marker(&mut out, proj, center, pal.preview);
            }
        } else if let Some(preview) = rectangle_gesture {
            if let sketch_runtime::rect::RectPreview::Rubber { start, corners, .. } = preview {
                push_polyline(
                    &mut out.lines,
                    proj,
                    corners
                        .iter()
                        .chain(corners.first())
                        .map(|p| crate::line_gesture::host(*p)),
                    pal.preview,
                    1.5,
                    false,
                );
                if matches!(
                    tool_state,
                    ToolState::RectCenterAt { .. } | ToolState::RectCenter3Corner { .. }
                ) {
                    push_point_marker(
                        &mut out,
                        proj,
                        crate::line_gesture::host(*start),
                        pal.preview,
                    );
                }
            } else if let sketch_runtime::rect::RectPreview::Edge { start, end } = preview {
                push_polyline(
                    &mut out.lines,
                    proj,
                    [
                        crate::line_gesture::host(*start),
                        crate::line_gesture::host(end.pos),
                    ]
                    .into_iter(),
                    pal.preview,
                    1.5,
                    false,
                );
                push_point_marker(
                    &mut out,
                    proj,
                    crate::line_gesture::host(*start),
                    pal.preview,
                );
            }
        } else {
            push_preview(
                &mut out,
                proj,
                pal,
                sketch,
                tool_state,
                cursor,
                params,
                selected,
                active_tool,
            );
        }
        // A full turn is made at the centre's click: its copies show
        // about the cursor before it.
        if active_tool == Some("sketch.rotate")
            && params.rotate_full_turn
            && tool_state.is_idle()
            && let Some(angles) = tools::full_turn_angles(params)
        {
            push_point_marker(&mut out, proj, cursor, pal.preview);
            for angle in angles {
                let turn = Similarity::rotation_about(cursor.to_glam(), angle);
                push_ghost(&mut out, proj, pal, sketch, selected, &turn);
            }
        }
        if active_tool == Some("sketch.trim")
            && let Some(span) = tools::trim_preview(sketch, cursor, snap_tol, params.cut_at_axes)
        {
            push_polyline(&mut out.lines, proj, span.into_iter(), pal.trim, 3.0, false);
        }
        if let Some(snap) = &snap {
            let from = crate::tools::snap_context(tool_state, sketch).from;
            push_snap_marker(
                &mut out,
                proj,
                pal,
                sketch,
                snap,
                from,
                hint_lines,
                proj.to_px(raw),
            );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{ExternalReference, ExternalSource, GeometryElement, Line, Point, Vec2D};

    /// Projected geometry reads as what it is: dashed like construction
    /// while it only guides, solid once it counts in the profile.
    #[test]
    fn projected_geometry_is_dashed_until_it_counts() {
        let mut sketch = Sketch::new("s");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, 0.0))));
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let source = ExternalSource::of_reference(ExternalReference::Datum {
            datum: Uuid::new_v4(),
        });
        sketch.external.insert(line, source);
        let pal = SketchPalette::default();
        let style = |sketch: &Sketch| element_style(sketch, line, &HashSet::new(), None, &pal);
        assert!(style(&sketch).dash.is_some(), "a guide is dashed");
        sketch.external.get_mut(&line).unwrap().defining = true;
        assert!(style(&sketch).dash.is_none(), "counting, it is solid");
    }
}
