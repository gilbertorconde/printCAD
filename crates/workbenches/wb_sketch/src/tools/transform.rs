//! Transform tools over the current selection: translate, rotate, scale
//! and mirror. Copies deep-copy points with fresh ids while preserving
//! point sharing *within* the copied set.

use std::collections::{HashMap, HashSet};

use glam::{Mat2, Vec2};
use uuid::Uuid;

use super::{ToolEffect, ToolState};
use crate::sketch::{Arc, BSpline, Circle, Ellipse, GeometryElement, Line, Point, Sketch, Vec2D};
use crate::snap;

/// A 2D similarity transform `p' = m·p + t` (rotation/scale/mirror plus
/// translation). Also drives the overlay ghost previews.
#[derive(Debug, Clone, Copy)]
pub struct Similarity {
    m: Mat2,
    t: Vec2,
}

impl Similarity {
    pub fn translation(d: Vec2) -> Self {
        Self {
            m: Mat2::IDENTITY,
            t: d,
        }
    }

    pub fn rotation_about(center: Vec2, angle: f32) -> Self {
        let m = Mat2::from_angle(angle);
        Self {
            m,
            t: center - m * center,
        }
    }

    pub fn scale_about(center: Vec2, factor: f32) -> Self {
        let m = Mat2::from_diagonal(Vec2::splat(factor));
        Self {
            m,
            t: center - m * center,
        }
    }

    /// Reflection about the line through `a` and `b`.
    pub fn mirror_about(a: Vec2, b: Vec2) -> Self {
        let u = (b - a).normalize();
        // Householder-style reflection: 2·uuᵀ − I.
        let m = Mat2::from_cols(
            Vec2::new(2.0 * u.x * u.x - 1.0, 2.0 * u.x * u.y),
            Vec2::new(2.0 * u.x * u.y, 2.0 * u.y * u.y - 1.0),
        );
        Self { m, t: a - m * a }
    }

    pub fn apply(&self, p: Vec2D) -> Vec2D {
        Vec2D::from_glam(self.m * p.to_glam() + self.t)
    }

    /// Linear part only (for direction vectors like an ellipse major axis).
    pub fn apply_vec(&self, v: Vec2D) -> Vec2D {
        Vec2D::from_glam(self.m * v.to_glam())
    }

    /// Uniform scale factor of the linear part.
    pub fn scale_factor(&self) -> f32 {
        self.m.determinant().abs().sqrt()
    }

    /// Whether the transform flips orientation (mirror): CCW arcs must swap
    /// their endpoints to stay CCW.
    pub fn flips_orientation(&self) -> bool {
        self.m.determinant() < 0.0
    }
}

/// Every point id the selection touches: selected point elements plus the
/// points referenced by selected curves (shared points included — the
/// solver re-solves neighbours afterwards).
fn selection_point_ids(sketch: &Sketch, selected: &HashSet<Uuid>) -> HashSet<Uuid> {
    let mut pts = HashSet::new();
    for geom in &sketch.geometry {
        if !selected.contains(&geom.id()) {
            continue;
        }
        match geom {
            GeometryElement::Point(p) => {
                pts.insert(p.id);
            }
            other => pts.extend(Sketch::curve_point_ids(other)),
        }
    }
    pts
}

/// Transform the selected geometry in place. Returns the touched point
/// count (0 = empty selection).
pub(super) fn apply_to_selection(
    sketch: &mut Sketch,
    selected: &HashSet<Uuid>,
    xf: &Similarity,
) -> usize {
    let pts = selection_point_ids(sketch, selected);
    let scale = xf.scale_factor();
    for geom in &mut sketch.geometry {
        match geom {
            GeometryElement::Point(p) if pts.contains(&p.id) => {
                p.position = xf.apply(p.position);
            }
            GeometryElement::Circle(c) if selected.contains(&c.id) => c.radius *= scale,
            GeometryElement::Arc(a) if selected.contains(&a.id) => a.radius *= scale,
            GeometryElement::Ellipse(e) if selected.contains(&e.id) => {
                e.major = xf.apply_vec(e.major);
            }
            _ => {}
        }
    }
    pts.len()
}

/// Add a transformed deep copy of the selection: fresh point ids, sharing
/// preserved within the copy. Returns the number of copied elements.
pub(super) fn copy_selection(
    sketch: &mut Sketch,
    selected: &HashSet<Uuid>,
    xf: &Similarity,
) -> usize {
    let pts = selection_point_ids(sketch, selected);
    let mut map: HashMap<Uuid, Uuid> = HashMap::new();
    for pid in &pts {
        let Some(pos) = sketch.point_position(*pid) else {
            continue;
        };
        let new_id = sketch.add_geometry(GeometryElement::Point(Point::new(xf.apply(pos))));
        sketch.set_construction(new_id, sketch.is_construction(*pid));
        map.insert(*pid, new_id);
    }

    let scale = xf.scale_factor();
    let flip = xf.flips_orientation();
    let originals: Vec<GeometryElement> = sketch
        .geometry
        .iter()
        .filter(|g| selected.contains(&g.id()) && !matches!(g, GeometryElement::Point(_)))
        .cloned()
        .collect();
    let mut count = map.len();
    for geom in originals {
        // Skip curves with dangling references rather than panic.
        if !Sketch::curve_point_ids(&geom)
            .iter()
            .all(|pid| map.contains_key(pid))
        {
            continue;
        }
        let copy = match &geom {
            GeometryElement::Line(l) => {
                GeometryElement::Line(Line::new(map[&l.start], map[&l.end]))
            }
            GeometryElement::Arc(a) => {
                // A mirrored CCW arc reads CW: swap endpoints to stay CCW.
                let (s, e) = if flip {
                    (a.end, a.start)
                } else {
                    (a.start, a.end)
                };
                GeometryElement::Arc(Arc::new(map[&a.center], map[&s], map[&e], a.radius * scale))
            }
            GeometryElement::Circle(c) => {
                GeometryElement::Circle(Circle::new(map[&c.center], c.radius * scale))
            }
            GeometryElement::Ellipse(e) => GeometryElement::Ellipse(Ellipse::new(
                map[&e.center],
                xf.apply_vec(e.major),
                e.ratio,
            )),
            GeometryElement::BSpline(b) => GeometryElement::BSpline(BSpline::new(
                b.control_points.iter().map(|pid| map[pid]).collect(),
                b.periodic,
            )),
            GeometryElement::Point(_) => continue,
        };
        let flag = sketch.is_construction(geom.id());
        let new_id = sketch.add_geometry(copy);
        sketch.set_construction(new_id, flag);
        count += 1;
    }
    count
}

/// Cursor snapped to an existing point's *position* only.
fn snapped_pos(sketch: &Sketch, cursor: Vec2D, snap_tol: f32) -> Vec2D {
    snap::snap_to_point(sketch, cursor, snap_tol, &[])
        .position(sketch)
        .unwrap_or(cursor)
}

pub(super) fn translate(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
    selected: &HashSet<Uuid>,
    copies: u32,
) -> ToolEffect {
    let pos = snapped_pos(sketch, cursor, snap_tol);
    match *state {
        ToolState::TranslateFrom { base } => {
            *state = ToolState::Idle;
            let delta = (pos - base).to_glam();
            if selected.is_empty() || delta.length() < 1e-6 {
                return ToolEffect::none();
            }
            if copies == 0 {
                let n = apply_to_selection(sketch, selected, &Similarity::translation(delta));
                ToolEffect::changed(format!("Moved selection ({n} points)"))
            } else {
                for k in 1..=copies {
                    copy_selection(sketch, selected, &Similarity::translation(delta * k as f32));
                }
                ToolEffect::changed(format!("Created {copies} translated cop(ies)"))
            }
        }
        _ => {
            *state = ToolState::TranslateFrom { base: pos };
            ToolEffect::none()
        }
    }
}

pub(super) fn rotate(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
    selected: &HashSet<Uuid>,
    copies: u32,
) -> ToolEffect {
    let pos = snapped_pos(sketch, cursor, snap_tol);
    match *state {
        ToolState::RotateCenter { center } => {
            if (pos - center).to_glam().length() < 1e-6 {
                return ToolEffect::none();
            }
            *state = ToolState::RotateRef {
                center,
                reference: pos,
            };
            ToolEffect::none()
        }
        ToolState::RotateRef { center, reference } => {
            *state = ToolState::Idle;
            let c = center.to_glam();
            let to = pos.to_glam() - c;
            if selected.is_empty() || to.length() < 1e-6 {
                return ToolEffect::none();
            }
            let angle = (reference.to_glam() - c).angle_to(to);
            if copies == 0 {
                apply_to_selection(sketch, selected, &Similarity::rotation_about(c, angle));
                ToolEffect::changed(format!("Rotated selection by {:.1}°", angle.to_degrees()))
            } else {
                for k in 1..=copies {
                    copy_selection(
                        sketch,
                        selected,
                        &Similarity::rotation_about(c, angle * k as f32),
                    );
                }
                ToolEffect::changed(format!("Created {copies} rotated cop(ies)"))
            }
        }
        _ => {
            *state = ToolState::RotateCenter { center: pos };
            ToolEffect::none()
        }
    }
}

pub(super) fn scale(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
    selected: &HashSet<Uuid>,
) -> ToolEffect {
    let pos = snapped_pos(sketch, cursor, snap_tol);
    match *state {
        ToolState::ScaleBase { base } => {
            if (pos - base).to_glam().length() < 1e-6 {
                return ToolEffect::none();
            }
            *state = ToolState::ScaleRef {
                base,
                reference: pos,
            };
            ToolEffect::none()
        }
        ToolState::ScaleRef { base, reference } => {
            *state = ToolState::Idle;
            let b = base.to_glam();
            let factor = (pos.to_glam() - b).length() / (reference.to_glam() - b).length();
            if selected.is_empty() || factor < 1e-4 {
                return ToolEffect::none();
            }
            apply_to_selection(sketch, selected, &Similarity::scale_about(b, factor));
            ToolEffect::changed(format!("Scaled selection by {factor:.3}"))
        }
        _ => {
            *state = ToolState::ScaleBase { base: pos };
            ToolEffect::none()
        }
    }
}

/// Line element endpoints under the cursor, for one-click mirror axes.
fn line_under_cursor(sketch: &Sketch, cursor: Vec2D, tol: f32) -> Option<(Vec2D, Vec2D)> {
    sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Line(l) => {
                let d = snap::distance_to_element(sketch, g, cursor)?;
                (d <= tol).then(|| {
                    (
                        sketch.point_position(l.start),
                        sketch.point_position(l.end),
                        d,
                    )
                })
            }
            _ => None,
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .and_then(|(a, b, _)| Some((a?, b?)))
}

fn mirror_now(
    state: &mut ToolState,
    sketch: &mut Sketch,
    selected: &HashSet<Uuid>,
    a: Vec2D,
    b: Vec2D,
) -> ToolEffect {
    *state = ToolState::Idle;
    if selected.is_empty() || (b - a).to_glam().length() < 1e-6 {
        return ToolEffect::none();
    }
    let n = copy_selection(
        sketch,
        selected,
        &Similarity::mirror_about(a.to_glam(), b.to_glam()),
    );
    ToolEffect::changed(format!("Mirrored {n} element(s)"))
}

pub(super) fn mirror(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
    selected: &HashSet<Uuid>,
) -> ToolEffect {
    match *state {
        ToolState::MirrorAxisFrom { a } => {
            let b = snapped_pos(sketch, cursor, snap_tol);
            mirror_now(state, sketch, selected, a, b)
        }
        _ => {
            // A clicked point starts a two-point axis; otherwise a clicked
            // line element is the axis; empty space starts a two-point axis.
            if let snap::SnapTarget::Existing(pid) =
                snap::snap_to_point(sketch, cursor, snap_tol, &[])
            {
                if let Some(pos) = sketch.point_position(pid) {
                    *state = ToolState::MirrorAxisFrom { a: pos };
                    return ToolEffect::none();
                }
            }
            if let Some((a, b)) = line_under_cursor(sketch, cursor, snap_tol) {
                return mirror_now(state, sketch, selected, a, b);
            }
            *state = ToolState::MirrorAxisFrom { a: cursor };
            ToolEffect::none()
        }
    }
}
