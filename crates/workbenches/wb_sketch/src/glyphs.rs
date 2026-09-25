//! Interactive constraint glyphs: viewport anchors, dimension lines, icon
//! marks, colors, hit-testing and leader lines for the edited sketch's
//! constraints.

use std::collections::{HashMap, HashSet};

use core_document::{ScreenSpaceLabel, ScreenSpaceMark, ScreenSpaceOverlay, SketchPalette};
use uuid::Uuid;

use crate::overlay::SketchProjector;
use crate::sketch::{self, ConstraintKind, GeometryElement, Sketch, Vec2D};
use crate::snap::arc_angles;
use crate::style::constraint_icon;

/// Dimension label font size in pixels.
const LABEL_SIZE: f32 = 12.0;
/// Icon glyph size in pixels.
const ICON_SIZE: f32 = 14.0;
/// Screen-space offset of symbol glyphs from their element anchor.
const SYMBOL_OFFSET_PX: f32 = 12.0;
const SYMBOL_HIT_RADIUS_PX: f32 = 10.0;
/// Leader line appears once a radial dimension label strays this far (px)
/// from its anchor.
const LEADER_MIN_PX: f32 = 40.0;
/// Default dimension-label offset from its anchor, in px (converted to
/// sketch units at the current zoom, so labels stay put when zooming).
const DEFAULT_LABEL_OFFSET_PX: f32 = 14.0;
/// Extension lines overshoot the dimension line by this much.
const EXTENSION_OVERSHOOT_PX: f32 = 5.0;
const ANGLE_ARC_SEGMENTS: usize = 24;

/// What a glyph draws.
#[derive(Debug, Clone, PartialEq)]
pub enum GlyphVisual {
    /// A dimension value in a pill.
    Pill { text: String },
    /// A constraint icon, with an index suffix when several of the same
    /// relational kind exist.
    Icon {
        name: &'static str,
        suffix: Option<usize>,
    },
}

/// One drawn constraint glyph (dimension label or symbol) and the lines
/// that belong to it.
#[derive(Debug, Clone)]
pub struct Glyph {
    pub constraint: Uuid,
    pub dimensional: bool,
    /// Label center, viewport px.
    pub pos: [f32; 2],
    /// Constrained element's anchor, viewport px (leader-line target).
    pub anchor: [f32; 2],
    pub visual: GlyphVisual,
    pub color: [f32; 3],
    /// Dimension, extension and leader lines in viewport px.
    pub lines: Vec<([f32; 2], [f32; 2])>,
}

impl Glyph {
    /// The text a pill shows; empty for icons.
    pub fn text(&self) -> &str {
        match &self.visual {
            GlyphVisual::Pill { text } => text,
            GlyphVisual::Icon { .. } => "",
        }
    }

    /// The pill label of a dimension, or the suffix label of an indexed
    /// icon.
    pub fn label(&self) -> Option<ScreenSpaceLabel> {
        match &self.visual {
            GlyphVisual::Pill { text } => Some(
                ScreenSpaceLabel::new(self.pos, text.clone(), self.color, LABEL_SIZE)
                    .pill()
                    .mono(),
            ),
            GlyphVisual::Icon {
                suffix: Some(n), ..
            } => Some(
                ScreenSpaceLabel::new(
                    [self.pos[0] + 9.0, self.pos[1] + 7.0],
                    n.to_string(),
                    self.color,
                    9.0,
                )
                .mono(),
            ),
            GlyphVisual::Icon { suffix: None, .. } => None,
        }
    }

    /// The icon mark of a symbol glyph.
    pub fn mark(&self) -> Option<ScreenSpaceMark> {
        match &self.visual {
            GlyphVisual::Icon { name, .. } => {
                Some(ScreenSpaceMark::icon(self.pos, name, ICON_SIZE, self.color))
            }
            GlyphVisual::Pill { .. } => None,
        }
    }
}

/// Compact number formatting: two decimals with trailing zeros trimmed.
pub fn fmt_num(v: f32) -> String {
    let mut s = format!("{v:.2}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}

/// Default label offset in sketch units at the current zoom.
pub fn default_label_offset(units_per_px: f32) -> Vec2D {
    let d = DEFAULT_LABEL_OFFSET_PX * units_per_px;
    Vec2D::new(d, d)
}

/// Representative sketch-space anchor of an element (glyph placement).
fn element_anchor(sketch: &Sketch, id: Uuid) -> Option<Vec2D> {
    let mid = |a: Vec2D, b: Vec2D| Vec2D::new(0.5 * (a.x + b.x), 0.5 * (a.y + b.y));
    match sketch.get_geometry(id)? {
        GeometryElement::Point(p) => Some(p.position),
        GeometryElement::Line(l) => Some(mid(
            sketch.point_position(l.start)?,
            sketch.point_position(l.end)?,
        )),
        GeometryElement::Arc(a) => {
            let c = sketch.point_position(a.center)?;
            let s = sketch.point_position(a.start)?;
            let e = sketch.point_position(a.end)?;
            let sv = (s - c).to_glam();
            let (start_angle, sweep) = arc_angles(sv, (e - c).to_glam());
            let ang = start_angle + 0.5 * sweep;
            let r = sv.length();
            Some(Vec2D::new(c.x + r * ang.cos(), c.y + r * ang.sin()))
        }
        GeometryElement::Circle(c) => {
            let center = sketch.point_position(c.center)?;
            Some(Vec2D::new(center.x, center.y + c.radius))
        }
        GeometryElement::Ellipse(e) => sketch.point_position(e.center),
        GeometryElement::BSpline(b) => {
            let pts: Option<Vec<Vec2D>> = b
                .control_points
                .iter()
                .map(|id| sketch.point_position(*id))
                .collect();
            let pts = pts?;
            let n = pts.len() as f32;
            (n > 0.0).then(|| {
                Vec2D::new(
                    pts.iter().map(|p| p.x).sum::<f32>() / n,
                    pts.iter().map(|p| p.y).sum::<f32>() / n,
                )
            })
        }
    }
}

/// Sketch-space anchor of a dimensional constraint's label.
fn dim_anchor(sketch: &Sketch, kind: &ConstraintKind) -> Option<Vec2D> {
    let mid = |a: Vec2D, b: Vec2D| Vec2D::new(0.5 * (a.x + b.x), 0.5 * (a.y + b.y));
    match *kind {
        ConstraintKind::Length { line, .. } | ConstraintKind::AngleToAxis { line, .. } => {
            element_anchor(sketch, line)
        }
        ConstraintKind::Distance { point1, point2, .. } => Some(mid(
            sketch.point_position(point1)?,
            sketch.point_position(point2)?,
        )),
        ConstraintKind::DistanceX { a, b, .. } | ConstraintKind::DistanceY { a, b, .. } => {
            let pa = sketch.point_position(a)?;
            Some(match b {
                Some(b) => mid(pa, sketch.point_position(b)?),
                None => mid(pa, Vec2D::new(0.0, 0.0)),
            })
        }
        ConstraintKind::Radius { circle, .. } | ConstraintKind::Diameter { circle, .. } => {
            element_anchor(sketch, circle)
        }
        ConstraintKind::Angle { line1, line2, .. } => Some(mid(
            element_anchor(sketch, line1)?,
            element_anchor(sketch, line2)?,
        )),
        _ => None,
    }
}

/// Value text of a dimensional constraint (driving = stored value,
/// reference = measured, in parentheses).
fn dim_text(sketch: &Sketch, constraint: &sketch::Constraint) -> String {
    let value = if constraint.driving {
        sketch::dimension_value(&constraint.kind)
    } else {
        sketch::measured_value(sketch, &constraint.kind)
            .or_else(|| sketch::dimension_value(&constraint.kind))
    };
    let val = value.map(fmt_num).unwrap_or_else(|| "-".to_string());
    let text = match constraint.kind {
        ConstraintKind::Radius { .. } => format!("R {val}"),
        ConstraintKind::Diameter { .. } => format!("Ø {val}"),
        ConstraintKind::Angle { .. } | ConstraintKind::AngleToAxis { .. } => format!("{val}°"),
        _ => val,
    };
    if constraint.driving {
        text
    } else {
        format!("({text})")
    }
}

/// Relational constraints drawn at both elements, with a shared index.
fn pair_refs(kind: &ConstraintKind) -> Option<[Uuid; 2]> {
    match *kind {
        ConstraintKind::Parallel { line1, line2 } => Some([line1, line2]),
        ConstraintKind::Perpendicular { line1, line2 } => Some([line1, line2]),
        ConstraintKind::EqualLength { line1, line2 } => Some([line1, line2]),
        ConstraintKind::EqualRadius { circle1, circle2 } => Some([circle1, circle2]),
        ConstraintKind::Tangent {
            line_or_circle1,
            item2,
        } => Some([line_or_circle1, item2]),
        _ => None,
    }
}

/// Perpendicular offset (px) from a projected line's midpoint, so H/V and
/// symmetry glyphs sit beside the line instead of on it.
fn line_perp_offset_px(sketch: &Sketch, proj: &SketchProjector, line: Uuid) -> Option<[f32; 2]> {
    let (a, b) = line_endpoints(sketch, line)?;
    let pa = proj.to_px(a)?;
    let pb = proj.to_px(b)?;
    let mid = [0.5 * (pa[0] + pb[0]), 0.5 * (pa[1] + pb[1])];
    let (dx, dy) = (pb[0] - pa[0], pb[1] - pa[1]);
    let len = (dx * dx + dy * dy).sqrt();
    let (nx, ny) = if len > 1e-4 {
        (-dy / len, dx / len)
    } else {
        (0.0, -1.0)
    };
    Some([
        mid[0] + nx * SYMBOL_OFFSET_PX,
        mid[1] + ny * SYMBOL_OFFSET_PX,
    ])
}

fn line_endpoints(sketch: &Sketch, line: Uuid) -> Option<(Vec2D, Vec2D)> {
    match sketch.get_geometry(line)? {
        GeometryElement::Line(l) => Some((
            sketch.point_position(l.start)?,
            sketch.point_position(l.end)?,
        )),
        _ => None,
    }
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn add(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

fn scale(a: [f32; 2], s: f32) -> [f32; 2] {
    [a[0] * s, a[1] * s]
}

fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

fn normalize(a: [f32; 2]) -> Option<[f32; 2]> {
    let len = dot(a, a).sqrt();
    (len > 1e-4).then(|| scale(a, 1.0 / len))
}

/// A linear dimension between projected endpoints `pa` and `pb` measured
/// along `dir`: the dimension line passes through the label, extension
/// lines reach it from both endpoints and overshoot a little.
fn linear_dimension_lines(
    pa: [f32; 2],
    pb: [f32; 2],
    dir: [f32; 2],
    label: [f32; 2],
) -> Vec<([f32; 2], [f32; 2])> {
    let Some(d) = normalize(dir) else {
        return Vec::new();
    };
    let n = [-d[1], d[0]];
    let span = dot(sub(pb, pa), d);
    let pb_on_axis = add(pa, scale(d, span));
    let offset = dot(sub(label, pa), n);
    let overshoot = EXTENSION_OVERSHOOT_PX * offset.signum();
    let a_dim = add(pa, scale(n, offset));
    let b_dim = add(pb_on_axis, scale(n, offset));
    vec![
        (a_dim, b_dim),
        (pa, add(pa, scale(n, offset + overshoot))),
        (pb, add(pb_on_axis, scale(n, offset + overshoot))),
    ]
}

/// Arc points (px) of an angle dimension: centered on the vertex, from the
/// first direction to the second through the side the label sits on.
fn angle_arc_points(
    vertex: [f32; 2],
    d1: [f32; 2],
    d2: [f32; 2],
    label: [f32; 2],
) -> Vec<[f32; 2]> {
    let (Some(d1), Some(d2)) = (normalize(d1), normalize(d2)) else {
        return Vec::new();
    };
    let radius = dot(sub(label, vertex), sub(label, vertex)).sqrt().max(8.0);
    let a1 = d1[1].atan2(d1[0]);
    let mut sweep = d2[1].atan2(d2[0]) - a1;
    while sweep <= -std::f32::consts::PI {
        sweep += 2.0 * std::f32::consts::PI;
    }
    while sweep > std::f32::consts::PI {
        sweep -= 2.0 * std::f32::consts::PI;
    }
    // Sweep through the side of the label.
    let mid_angle = a1 + 0.5 * sweep;
    let label_dir = sub(label, vertex);
    if dot([mid_angle.cos(), mid_angle.sin()], label_dir) < 0.0 {
        sweep -= 2.0 * std::f32::consts::PI * sweep.signum();
    }
    (0..=ANGLE_ARC_SEGMENTS)
        .map(|i| {
            let t = i as f32 / ANGLE_ARC_SEGMENTS as f32;
            let a = a1 + sweep * t;
            add(vertex, scale([a.cos(), a.sin()], radius))
        })
        .collect()
}

/// Lines that belong to a dimension: witness lines for linear kinds, an
/// arc for angles, a leader from the center for radial kinds.
fn dimension_lines(
    sketch: &Sketch,
    proj: &SketchProjector,
    kind: &ConstraintKind,
    label: [f32; 2],
    anchor: [f32; 2],
) -> Vec<([f32; 2], [f32; 2])> {
    let px = |p: Vec2D| proj.to_px(p);
    match *kind {
        ConstraintKind::Length { line, .. } => {
            let Some((a, b)) = line_endpoints(sketch, line) else {
                return Vec::new();
            };
            let (Some(pa), Some(pb)) = (px(a), px(b)) else {
                return Vec::new();
            };
            linear_dimension_lines(pa, pb, sub(pb, pa), label)
        }
        ConstraintKind::Distance { point1, point2, .. } => {
            let (Some(a), Some(b)) = (sketch.point_position(point1), sketch.point_position(point2))
            else {
                return Vec::new();
            };
            let (Some(pa), Some(pb)) = (px(a), px(b)) else {
                return Vec::new();
            };
            linear_dimension_lines(pa, pb, sub(pb, pa), label)
        }
        ConstraintKind::DistanceX { a, b, .. } | ConstraintKind::DistanceY { a, b, .. } => {
            let Some(pa_s) = sketch.point_position(a) else {
                return Vec::new();
            };
            let pb_s = match b {
                Some(b) => match sketch.point_position(b) {
                    Some(p) => p,
                    None => return Vec::new(),
                },
                None => Vec2D::new(0.0, 0.0),
            };
            let horizontal = matches!(kind, ConstraintKind::DistanceX { .. });
            let axis_end = if horizontal {
                Vec2D::new(pa_s.x + 1.0, pa_s.y)
            } else {
                Vec2D::new(pa_s.x, pa_s.y + 1.0)
            };
            let (Some(pa), Some(pb), Some(pax)) = (px(pa_s), px(pb_s), px(axis_end)) else {
                return Vec::new();
            };
            linear_dimension_lines(pa, pb, sub(pax, pa), label)
        }
        ConstraintKind::Radius { circle, .. } | ConstraintKind::Diameter { circle, .. } => {
            let center = match sketch.get_geometry(circle) {
                Some(GeometryElement::Circle(c)) => sketch.point_position(c.center),
                Some(GeometryElement::Arc(a)) => sketch.point_position(a.center),
                _ => None,
            };
            match center.and_then(px) {
                Some(c) => vec![(c, label)],
                None => vec![(anchor, label)],
            }
        }
        ConstraintKind::Angle { line1, line2, .. } => {
            let (Some((a1, b1)), Some((a2, b2))) =
                (line_endpoints(sketch, line1), line_endpoints(sketch, line2))
            else {
                return Vec::new();
            };
            let Some(v2) =
                crate::geom2d::line_line(a1.to_glam(), b1.to_glam(), a2.to_glam(), b2.to_glam())
            else {
                return Vec::new();
            };
            let vertex = Vec2D::new(v2.x, v2.y);
            let (Some(v), Some(p1), Some(p2)) = (px(vertex), px(b1), px(b2)) else {
                return Vec::new();
            };
            let (Some(q1), Some(q2)) = (px(a1), px(a2)) else {
                return Vec::new();
            };
            // Each line's direction pointing away from the vertex toward its
            // farther endpoint.
            let far = |p: [f32; 2], q: [f32; 2]| {
                if dot(sub(p, v), sub(p, v)) >= dot(sub(q, v), sub(q, v)) {
                    sub(p, v)
                } else {
                    sub(q, v)
                }
            };
            let pts = angle_arc_points(v, far(p1, q1), far(p2, q2), label);
            pts.windows(2).map(|w| (w[0], w[1])).collect()
        }
        ConstraintKind::AngleToAxis { line, axis, .. } => {
            let Some((a, b)) = line_endpoints(sketch, line) else {
                return Vec::new();
            };
            let axis_end = match axis {
                sketch::AxisDirection::Horizontal => Vec2D::new(a.x + 1.0, a.y),
                sketch::AxisDirection::Vertical => Vec2D::new(a.x, a.y + 1.0),
            };
            let (Some(pa), Some(pb), Some(pax)) = (px(a), px(b), px(axis_end)) else {
                return Vec::new();
            };
            let pts = angle_arc_points(pa, sub(pax, pa), sub(pb, pa), label);
            pts.windows(2).map(|w| (w[0], w[1])).collect()
        }
        _ => Vec::new(),
    }
}

/// Build every glyph for the sketch's constraints, in viewport pixels.
/// `bound` are the dimensions a formula sets: drawn in the formula colour,
/// their value marked `ƒ`.
pub fn build(
    sketch: &Sketch,
    proj: &SketchProjector,
    selected: &HashSet<Uuid>,
    bound: &HashSet<Uuid>,
    pal: &SketchPalette,
) -> Vec<Glyph> {
    // Repeated relational kinds get a shared 1-based index suffix.
    let mut totals: HashMap<&'static str, usize> = HashMap::new();
    for c in &sketch.constraints {
        if pair_refs(&c.kind).is_some() {
            *totals.entry(constraint_icon(&c.kind)).or_default() += 1;
        }
    }
    let mut counters: HashMap<&'static str, usize> = HashMap::new();

    let mut out = Vec::new();
    let units_per_px = proj.units_per_px();
    for c in &sketch.constraints {
        let color = if selected.contains(&c.id) {
            pal.selected
        } else if !c.active {
            pal.inactive
        } else if c.kind.is_dimensional() && !c.driving {
            pal.reference
        } else if bound.contains(&c.id) {
            pal.formula
        } else {
            pal.constraint
        };

        if c.kind.is_dimensional() {
            let Some(anchor) = dim_anchor(sketch, &c.kind) else {
                continue;
            };
            let offset = c
                .label_offset
                .unwrap_or_else(|| default_label_offset(units_per_px));
            let (Some(pos), Some(anchor_px)) = (proj.to_px(anchor + offset), proj.to_px(anchor))
            else {
                continue;
            };
            let lines = dimension_lines(sketch, proj, &c.kind, pos, anchor_px);
            out.push(Glyph {
                constraint: c.id,
                dimensional: true,
                pos,
                anchor: anchor_px,
                visual: GlyphVisual::Pill {
                    text: if bound.contains(&c.id) {
                        format!("ƒ {}", dim_text(sketch, c))
                    } else {
                        dim_text(sketch, c)
                    },
                },
                color,
                lines,
            });
            continue;
        }

        let icon = constraint_icon(&c.kind);
        let mut symbol_at = |pos_px: Option<[f32; 2]>, suffix: Option<usize>| {
            if let Some(pos) = pos_px {
                out.push(Glyph {
                    constraint: c.id,
                    dimensional: false,
                    pos,
                    anchor: pos,
                    visual: GlyphVisual::Icon { name: icon, suffix },
                    color,
                    lines: Vec::new(),
                });
            }
        };
        let corner_off = |sketch: &Sketch, id: Uuid| -> Option<[f32; 2]> {
            let p = proj.to_px(element_anchor(sketch, id)?)?;
            Some([p[0] + SYMBOL_OFFSET_PX * 0.7, p[1] - SYMBOL_OFFSET_PX * 0.7])
        };

        match c.kind {
            ConstraintKind::Horizontal { element } | ConstraintKind::Vertical { element } => {
                symbol_at(line_perp_offset_px(sketch, proj, element), None);
            }
            ConstraintKind::Coincident { point1, .. } => {
                symbol_at(corner_off(sketch, point1), None);
            }
            ConstraintKind::PointOnLine { point, .. }
            | ConstraintKind::PointOnCircle { point, .. }
            | ConstraintKind::PointOnEllipse { point, .. }
            | ConstraintKind::Midpoint { point, .. } => {
                symbol_at(corner_off(sketch, point), None);
            }
            ConstraintKind::Symmetric { line, .. } => {
                symbol_at(line_perp_offset_px(sketch, proj, line), None);
            }
            ConstraintKind::SymmetricAboutPoint { center, .. } => {
                symbol_at(corner_off(sketch, center), None);
            }
            ConstraintKind::Block { element } => {
                symbol_at(corner_off(sketch, element), None);
            }
            ConstraintKind::FixedPoint { point, .. } => {
                symbol_at(corner_off(sketch, point), None);
            }
            _ => {
                if let Some(refs) = pair_refs(&c.kind) {
                    let idx = {
                        let n = counters.entry(icon).or_default();
                        *n += 1;
                        *n
                    };
                    let suffix = (totals.get(icon).copied().unwrap_or(0) > 1).then_some(idx);
                    for id in refs {
                        symbol_at(corner_off(sketch, id), suffix);
                    }
                }
            }
        }
    }
    fan_out(&mut out);
    out
}

/// Icons that would land on one spot (equal and parallel on one line; a
/// coincidence, a point-on and a lock on one point) step along a row
/// beside it, each where it can be seen and clicked.
fn fan_out(glyphs: &mut [Glyph]) {
    let step = SYMBOL_HIT_RADIUS_PX * 2.0;
    let mut taken: Vec<[f32; 2]> = Vec::new();
    for glyph in glyphs.iter_mut().filter(|g| !g.dimensional) {
        let mut pos = glyph.pos;
        while taken
            .iter()
            .any(|t| (t[0] - pos[0]).abs() < step && (t[1] - pos[1]).abs() < step)
        {
            pos[0] += step;
        }
        taken.push(pos);
        glyph.pos = pos;
    }
}

/// The dimension, extension and leader lines of every glyph: 1px in the
/// glyph's color. Radial leaders appear only once the label is dragged
/// away from its anchor.
pub fn dimension_overlays(glyphs: &[Glyph]) -> Vec<ScreenSpaceOverlay> {
    let mut out = Vec::new();
    for g in glyphs {
        let radial_leader = g.lines.len() == 1 && g.lines[0].1 == g.pos;
        if radial_leader {
            let (dx, dy) = (g.pos[0] - g.anchor[0], g.pos[1] - g.anchor[1]);
            if (dx * dx + dy * dy).sqrt() <= LEADER_MIN_PX {
                continue;
            }
        }
        for (a, b) in &g.lines {
            out.push(ScreenSpaceOverlay::new(*a, *b, g.color, 1.0));
        }
    }
    out
}

/// Half-extents of a dimension pill's hit box: the monospace advance is
/// 0.6 em, plus the pill padding the painter adds.
pub fn pill_half_extents(size: f32, char_count: usize) -> (f32, f32) {
    let [pad_x, pad_y] = ui_kit_pill_pad();
    (0.3 * size * char_count as f32 + pad_x, 0.65 * size + pad_y)
}

/// The painter's pill padding, mirrored here so hit-testing and drawing
/// agree without this crate depending on the UI kit.
const fn ui_kit_pill_pad() -> [f32; 2] {
    [5.0, 2.0]
}

/// The glyph under `p` (viewport px), nearest-center first. Dimension
/// pills use their text rect; icons a small circle.
pub fn hit_test(glyphs: &[Glyph], p: [f32; 2]) -> Option<&Glyph> {
    let mut best: Option<(&Glyph, f32)> = None;
    for g in glyphs {
        let dx = p[0] - g.pos[0];
        let dy = p[1] - g.pos[1];
        let inside = if g.dimensional {
            let (hw, hh) = pill_half_extents(LABEL_SIZE, g.text().chars().count());
            dx.abs() <= hw && dy.abs() <= hh
        } else {
            dx * dx + dy * dy <= SYMBOL_HIT_RADIUS_PX * SYMBOL_HIT_RADIUS_PX
        };
        if inside {
            let d2 = dx * dx + dy * dy;
            if best.map(|(_, bd)| d2 < bd).unwrap_or(true) {
                best = Some((g, d2));
            }
        }
    }
    best.map(|(g, _)| g)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_on_one_spot_fan_out_each_clickable() {
        let icon = |pos: [f32; 2]| Glyph {
            constraint: Uuid::new_v4(),
            dimensional: false,
            pos,
            anchor: pos,
            visual: GlyphVisual::Icon {
                name: "constraint-equal",
                suffix: None,
            },
            color: [1.0; 3],
            lines: Vec::new(),
        };
        let mut glyphs = vec![
            icon([100.0, 50.0]),
            icon([100.0, 50.0]),
            icon([101.0, 50.0]),
        ];
        fan_out(&mut glyphs);
        let ids: Vec<Uuid> = glyphs
            .iter()
            .map(|g| hit_test(&glyphs, g.pos).unwrap().constraint)
            .collect();
        assert_eq!(
            ids,
            glyphs.iter().map(|g| g.constraint).collect::<Vec<_>>(),
            "each is the one found at its own spot"
        );
    }

    fn pill(pos: [f32; 2], text: &str) -> Glyph {
        Glyph {
            constraint: Uuid::new_v4(),
            dimensional: true,
            pos,
            anchor: pos,
            visual: GlyphVisual::Pill {
                text: text.to_string(),
            },
            color: [1.0; 3],
            lines: Vec::new(),
        }
    }

    fn icon(pos: [f32; 2], name: &'static str) -> Glyph {
        Glyph {
            constraint: Uuid::new_v4(),
            dimensional: false,
            pos,
            anchor: pos,
            visual: GlyphVisual::Icon { name, suffix: None },
            color: [1.0; 3],
            lines: Vec::new(),
        }
    }

    #[test]
    fn pill_half_extents_follow_the_mono_advance_and_padding() {
        let (hw, hh) = pill_half_extents(12.0, 4);
        assert!((hw - (0.3 * 12.0 * 4.0 + 5.0)).abs() < 1e-5);
        assert!((hh - (0.65 * 12.0 + 2.0)).abs() < 1e-5);
    }

    #[test]
    fn dimension_label_hit_uses_text_rect() {
        let g = [pill([100.0, 100.0], "12.5")];
        let (hw, hh) = pill_half_extents(LABEL_SIZE, 4);
        assert!(hit_test(&g, [100.0 + hw - 0.5, 100.0]).is_some());
        assert!(hit_test(&g, [100.0 + hw + 1.0, 100.0]).is_none());
        assert!(hit_test(&g, [100.0, 100.0 + hh - 0.5]).is_some());
        assert!(hit_test(&g, [100.0, 100.0 + hh + 1.0]).is_none());
    }

    #[test]
    fn symbol_hit_uses_radius_circle() {
        let g = [icon([50.0, 50.0], "constraint-horizontal")];
        assert!(hit_test(&g, [57.0, 57.0]).is_some(), "within 10px radius");
        assert!(hit_test(&g, [58.0, 58.0]).is_none(), "outside 10px radius");
    }

    #[test]
    fn nearest_glyph_wins_on_overlap() {
        let a = icon([100.0, 100.0], "constraint-horizontal");
        let b = icon([106.0, 100.0], "constraint-vertical");
        let id_b = b.constraint;
        let g = [a, b];
        let hit = hit_test(&g, [105.0, 100.0]).unwrap();
        assert_eq!(hit.constraint, id_b);
    }

    #[test]
    fn fmt_num_trims_trailing_zeros() {
        assert_eq!(fmt_num(12.5), "12.5");
        assert_eq!(fmt_num(10.0), "10");
        assert_eq!(fmt_num(0.25), "0.25");
    }

    #[test]
    fn linear_dimension_has_a_dimension_line_and_two_extensions() {
        let lines = linear_dimension_lines([0.0, 0.0], [100.0, 0.0], [1.0, 0.0], [50.0, 30.0]);
        assert_eq!(lines.len(), 3);
        // The dimension line runs through the label's offset.
        assert!((lines[0].0[1] - 30.0).abs() < 1e-4 && (lines[0].1[1] - 30.0).abs() < 1e-4);
        // Extension lines start at the endpoints and overshoot by 5px.
        assert_eq!(lines[1].0, [0.0, 0.0]);
        assert!((lines[1].1[1] - 35.0).abs() < 1e-4);
        assert!((lines[2].1[1] - 35.0).abs() < 1e-4);
    }

    #[test]
    fn angle_arc_sweeps_through_the_label_side() {
        let pts = angle_arc_points([0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [10.0, 10.0]);
        assert_eq!(pts.len(), ANGLE_ARC_SEGMENTS + 1);
        let mid = pts[ANGLE_ARC_SEGMENTS / 2];
        assert!(
            mid[0] > 0.0 && mid[1] > 0.0,
            "arc passes the first quadrant: {mid:?}"
        );
    }
}
