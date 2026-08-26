//! Interactive constraint glyphs: viewport anchors, labels, colors,
//! hit-testing and leader lines for the edited sketch's constraints.

use std::collections::{HashMap, HashSet};

use core_document::{ScreenSpaceLabel, ScreenSpaceOverlay};
use uuid::Uuid;

use crate::overlay::{SketchProjector, COLOR_SELECTED};
use crate::sketch::{self, ConstraintKind, GeometryElement, Sketch, Vec2D};
use crate::snap::arc_angles;

pub const COLOR_DRIVING: [f32; 3] = [0.95, 0.3, 0.25];
pub const COLOR_REFERENCE: [f32; 3] = [0.35, 0.55, 0.95];
pub const COLOR_INACTIVE: [f32; 3] = [0.55, 0.55, 0.55];

const GLYPH_SIZE: f32 = 13.0;
/// Screen-space offset of symbol glyphs from their element anchor.
const SYMBOL_OFFSET_PX: f32 = 12.0;
const SYMBOL_HIT_RADIUS_PX: f32 = 10.0;
/// Leader line appears once a dimension label strays this far (px) from
/// its anchor.
const LEADER_MIN_PX: f32 = 40.0;
/// Default dimension-label offset from its anchor, in px (converted to
/// sketch units at the current zoom, so labels stay put when zooming).
const DEFAULT_LABEL_OFFSET_PX: f32 = 14.0;

/// One drawn constraint glyph (symbol or dimension label).
pub struct Glyph {
    pub constraint: Uuid,
    pub dimensional: bool,
    /// Label center, viewport px.
    pub pos: [f32; 2],
    /// Constrained element's anchor, viewport px (leader-line target).
    pub anchor: [f32; 2],
    pub text: String,
    pub size: f32,
    pub color: [f32; 3],
    pub background: bool,
}

impl Glyph {
    pub fn into_label(self) -> ScreenSpaceLabel {
        ScreenSpaceLabel {
            pos: self.pos,
            text: self.text,
            color: self.color,
            size: self.size,
            background: self.background,
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
/// reference = measured).
fn dim_text(sketch: &Sketch, constraint: &sketch::Constraint) -> String {
    let value = if constraint.driving {
        sketch::dimension_value(&constraint.kind)
    } else {
        sketch::measured_value(sketch, &constraint.kind)
            .or_else(|| sketch::dimension_value(&constraint.kind))
    };
    let val = value.map(fmt_num).unwrap_or_else(|| "—".to_string());
    match constraint.kind {
        ConstraintKind::Radius { .. } => format!("R {val}"),
        ConstraintKind::Diameter { .. } => format!("⌀ {val}"),
        ConstraintKind::Angle { .. } | ConstraintKind::AngleToAxis { .. } => format!("{val}°"),
        ConstraintKind::DistanceX { .. } => format!("↔ {val}"),
        ConstraintKind::DistanceY { .. } => format!("↕ {val}"),
        _ => val,
    }
}

/// Pair-symbol text for relational constraints drawn at both elements.
fn pair_symbol(kind: &ConstraintKind) -> Option<(&'static str, [Uuid; 2])> {
    match *kind {
        ConstraintKind::Parallel { line1, line2 } => Some(("∥", [line1, line2])),
        ConstraintKind::Perpendicular { line1, line2 } => Some(("⊥", [line1, line2])),
        ConstraintKind::EqualLength { line1, line2 } => Some(("=", [line1, line2])),
        ConstraintKind::EqualRadius { circle1, circle2 } => Some(("=", [circle1, circle2])),
        ConstraintKind::Tangent {
            line_or_circle1,
            item2,
        } => Some(("⌒", [line_or_circle1, item2])),
        _ => None,
    }
}

/// Perpendicular offset (px) from a projected line's midpoint, so H/V and
/// symmetry glyphs sit beside the line instead of on it.
fn line_perp_offset_px(sketch: &Sketch, proj: &SketchProjector, line: Uuid) -> Option<[f32; 2]> {
    let (a, b) = match sketch.get_geometry(line)? {
        GeometryElement::Line(l) => (
            sketch.point_position(l.start)?,
            sketch.point_position(l.end)?,
        ),
        _ => return None,
    };
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

/// Build every glyph for the sketch's constraints, in viewport pixels.
pub fn build(sketch: &Sketch, proj: &SketchProjector, selected: &HashSet<Uuid>) -> Vec<Glyph> {
    // Repeated relational groups get a shared 1-based index suffix.
    let mut totals: HashMap<&'static str, usize> = HashMap::new();
    for c in &sketch.constraints {
        if let Some((sym, _)) = pair_symbol(&c.kind) {
            *totals.entry(sym).or_default() += 1;
        }
    }
    let mut counters: HashMap<&'static str, usize> = HashMap::new();

    let mut out = Vec::new();
    let units_per_px = proj.units_per_px();
    for c in &sketch.constraints {
        let color = if selected.contains(&c.id) {
            COLOR_SELECTED
        } else if !c.active {
            COLOR_INACTIVE
        } else if c.kind.is_dimensional() {
            if c.driving {
                COLOR_DRIVING
            } else {
                COLOR_REFERENCE
            }
        } else {
            COLOR_REFERENCE
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
            out.push(Glyph {
                constraint: c.id,
                dimensional: true,
                pos,
                anchor: anchor_px,
                text: dim_text(sketch, c),
                size: GLYPH_SIZE,
                color,
                background: true,
            });
            continue;
        }

        let mut symbol_at = |pos_px: Option<[f32; 2]>, text: String| {
            if let Some(pos) = pos_px {
                out.push(Glyph {
                    constraint: c.id,
                    dimensional: false,
                    pos,
                    anchor: pos,
                    text,
                    size: GLYPH_SIZE,
                    color,
                    background: false,
                });
            }
        };
        let corner_off = |sketch: &Sketch, id: Uuid| -> Option<[f32; 2]> {
            let p = proj.to_px(element_anchor(sketch, id)?)?;
            Some([p[0] + SYMBOL_OFFSET_PX * 0.7, p[1] - SYMBOL_OFFSET_PX * 0.7])
        };

        match c.kind {
            ConstraintKind::Horizontal { element } => {
                symbol_at(line_perp_offset_px(sketch, proj, element), "H".to_string());
            }
            ConstraintKind::Vertical { element } => {
                symbol_at(line_perp_offset_px(sketch, proj, element), "V".to_string());
            }
            ConstraintKind::Coincident { point1, .. } => {
                symbol_at(corner_off(sketch, point1), "●".to_string());
            }
            ConstraintKind::PointOnLine { point, .. }
            | ConstraintKind::PointOnCircle { point, .. }
            | ConstraintKind::PointOnEllipse { point, .. }
            | ConstraintKind::Midpoint { point, .. } => {
                symbol_at(corner_off(sketch, point), "○".to_string());
            }
            ConstraintKind::Symmetric { line, .. } => {
                symbol_at(line_perp_offset_px(sketch, proj, line), "⋈".to_string());
            }
            ConstraintKind::SymmetricAboutPoint { center, .. } => {
                symbol_at(corner_off(sketch, center), "⋈".to_string());
            }
            ConstraintKind::Block { element } => {
                symbol_at(corner_off(sketch, element), "▣".to_string());
            }
            ConstraintKind::FixedPoint { point, .. } => {
                symbol_at(corner_off(sketch, point), "▪".to_string());
            }
            _ => {
                if let Some((sym, refs)) = pair_symbol(&c.kind) {
                    let idx = {
                        let n = counters.entry(sym).or_default();
                        *n += 1;
                        *n
                    };
                    let text = if totals.get(sym).copied().unwrap_or(0) > 1 {
                        format!("{sym}{idx}")
                    } else {
                        sym.to_string()
                    };
                    for id in refs {
                        symbol_at(corner_off(sketch, id), text.clone());
                    }
                }
            }
        }
    }
    out
}

/// Leader lines from far-dragged dimension labels back to their anchors.
pub fn leader_overlays(glyphs: &[Glyph]) -> Vec<ScreenSpaceOverlay> {
    glyphs
        .iter()
        .filter(|g| g.dimensional)
        .filter_map(|g| {
            let (dx, dy) = (g.pos[0] - g.anchor[0], g.pos[1] - g.anchor[1]);
            ((dx * dx + dy * dy).sqrt() > LEADER_MIN_PX)
                .then(|| ScreenSpaceOverlay::new(g.pos, g.anchor, g.color, 1.0))
        })
        .collect()
}

/// Half-extents of a dimension label's estimated hit box.
pub fn label_half_extents(size: f32, char_count: usize) -> (f32, f32) {
    (0.31 * size * char_count as f32, 0.675 * size)
}

/// The glyph under `p` (viewport px), nearest-center first. Dimension
/// labels use their estimated text rect; symbols a small circle.
pub fn hit_test(glyphs: &[Glyph], p: [f32; 2]) -> Option<&Glyph> {
    let mut best: Option<(&Glyph, f32)> = None;
    for g in glyphs {
        let dx = p[0] - g.pos[0];
        let dy = p[1] - g.pos[1];
        let inside = if g.dimensional {
            let (hw, hh) = label_half_extents(g.size, g.text.chars().count());
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

    fn glyph(pos: [f32; 2], text: &str, dimensional: bool) -> Glyph {
        Glyph {
            constraint: Uuid::new_v4(),
            dimensional,
            pos,
            anchor: pos,
            text: text.to_string(),
            size: 13.0,
            color: COLOR_DRIVING,
            background: dimensional,
        }
    }

    #[test]
    fn label_half_extents_scale_with_text_and_size() {
        let (hw, hh) = label_half_extents(13.0, 4);
        assert!((hw - 0.31 * 13.0 * 4.0).abs() < 1e-5);
        assert!((hh - 0.675 * 13.0).abs() < 1e-5);
        let (hw2, _) = label_half_extents(13.0, 8);
        assert!((hw2 - 2.0 * hw).abs() < 1e-4, "width grows with chars");
    }

    #[test]
    fn dimension_label_hit_uses_text_rect() {
        let g = [glyph([100.0, 100.0], "12.5", true)];
        let (hw, hh) = label_half_extents(13.0, 4);
        assert!(hit_test(&g, [100.0 + hw - 0.5, 100.0]).is_some());
        assert!(hit_test(&g, [100.0 + hw + 1.0, 100.0]).is_none());
        assert!(hit_test(&g, [100.0, 100.0 + hh - 0.5]).is_some());
        assert!(hit_test(&g, [100.0, 100.0 + hh + 1.0]).is_none());
    }

    #[test]
    fn symbol_hit_uses_radius_circle() {
        let g = [glyph([50.0, 50.0], "H", false)];
        assert!(hit_test(&g, [57.0, 57.0]).is_some(), "within 10px radius");
        assert!(hit_test(&g, [58.0, 58.0]).is_none(), "outside 10px radius");
    }

    #[test]
    fn nearest_glyph_wins_on_overlap() {
        let a = glyph([100.0, 100.0], "H", false);
        let b = glyph([106.0, 100.0], "V", false);
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
}
