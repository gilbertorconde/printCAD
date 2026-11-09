//! The sketch's selection: a set of element ids that also remembers the
//! order they were picked in, for the tools where it matters (the vertex of
//! a three-point angle, the centre of a symmetry about a point).

use std::collections::HashSet;
use std::ops::Deref;

use crate::sketch::{GeometryElement, Sketch, Vec2D};
use crate::{profile, snap};
use uuid::Uuid;

/// Reads as the set it holds; changes go through its own methods so the
/// order follows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    set: HashSet<Uuid>,
    order: Vec<Uuid>,
}

impl Deref for Selection {
    type Target = HashSet<Uuid>;

    fn deref(&self) -> &HashSet<Uuid> {
        &self.set
    }
}

impl Selection {
    /// Adds `id` after those already picked; `false` when it was there.
    pub fn insert(&mut self, id: Uuid) -> bool {
        let added = self.set.insert(id);
        if added {
            self.order.push(id);
        }
        added
    }

    /// `false` when it was not there.
    pub fn remove(&mut self, id: &Uuid) -> bool {
        let had = self.set.remove(id);
        if had {
            self.order.retain(|other| other != id);
        }
        had
    }

    pub fn clear(&mut self) {
        self.set.clear();
        self.order.clear();
    }

    pub fn extend(&mut self, ids: impl IntoIterator<Item = Uuid>) {
        for id in ids {
            self.insert(id);
        }
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&Uuid) -> bool) {
        self.order.retain(|id| keep(id));
        self.set = self.order.iter().copied().collect();
    }

    /// Empties the selection, handing back what it held in picked order.
    pub fn drain(&mut self) -> std::vec::IntoIter<Uuid> {
        self.set.clear();
        std::mem::take(&mut self.order).into_iter()
    }

    /// The ids in the order they were picked.
    pub fn in_order(&self) -> &[Uuid] {
        &self.order
    }
}

impl<'a> IntoIterator for &'a Selection {
    type Item = &'a Uuid;
    type IntoIter = std::collections::hash_set::Iter<'a, Uuid>;

    fn into_iter(self) -> Self::IntoIter {
        self.set.iter()
    }
}

impl FromIterator<Uuid> for Selection {
    fn from_iter<I: IntoIterator<Item = Uuid>>(ids: I) -> Self {
        let mut selection = Selection::default();
        selection.extend(ids);
        selection
    }
}

fn point_in_rect(p: Vec2D, min: Vec2D, max: Vec2D) -> bool {
    p.x >= min.x && p.x <= max.x && p.y >= min.y && p.y <= max.y
}

/// The curves joined to `start` end to end, `start` included: through
/// the points they share, or ends at one spot, as projected curves meet.
pub(super) fn connected_curves(sketch: &Sketch, start: Uuid) -> Vec<Uuid> {
    let at = |id: Uuid| sketch.point_position(id).map(|p| p.to_glam());
    let curves: Vec<(Uuid, Vec<glam::DVec2>)> = sketch
        .geometry
        .iter()
        .filter_map(|g| {
            let ends = Sketch::curve_ends(g)?;
            Some((g.id(), ends.into_iter().filter_map(at).collect()))
        })
        .collect();
    let meets = |a: &[glam::DVec2], b: &[glam::DVec2]| {
        a.iter()
            .any(|p| b.iter().any(|q| p.distance(*q) < profile::PROJECTED_JOIN))
    };
    let mut found = vec![start];
    let mut frontier = vec![start];
    while let Some(id) = frontier.pop() {
        let Some((_, ends)) = curves.iter().find(|(c, _)| *c == id) else {
            continue;
        };
        for (other, other_ends) in &curves {
            if !found.contains(other) && meets(ends, other_ends) {
                found.push(*other);
                frontier.push(*other);
            }
        }
    }
    found
}

/// Whether `geom` lies FULLY inside the axis-aligned rectangle `min`..`max`
/// (box-selection containment): a point by its position; a line by both
/// endpoints; an arc by its center, endpoints and angular midpoint; a
/// circle by its center±radius bounding box. Unresolvable references are
/// never inside.
/// Whether any of `geom` lies in the box `min`..`max`: a point of it
/// inside, or a stretch of it crossing the box.
pub(super) fn element_touches(
    sketch: &Sketch,
    geom: &GeometryElement,
    min: Vec2D,
    max: Vec2D,
) -> bool {
    let at = |id| sketch.point_position(id);
    let around = |c: Vec2D, r: f64, from: f64, sweep: f64, n: usize| -> Vec<Vec2D> {
        (0..=n)
            .map(|i| {
                let t = from + sweep * i as f64 / n as f64;
                Vec2D::new(c.x + r * t.cos(), c.y + r * t.sin())
            })
            .collect()
    };
    let path: Vec<Vec2D> = match geom {
        GeometryElement::Point(p) => vec![p.position],
        GeometryElement::Line(l) => [at(l.start), at(l.end)].into_iter().flatten().collect(),
        GeometryElement::Circle(c) => at(c.center).map_or_else(Vec::new, |c0| {
            around(c0, c.radius, 0.0, std::f64::consts::TAU, 64)
        }),
        GeometryElement::Arc(a) => match (at(a.center), at(a.start), at(a.end)) {
            (Some(c), Some(s), Some(e)) => {
                let (from, sweep) = snap::arc_angles((s - c).to_glam(), (e - c).to_glam());
                around(c, (s - c).to_glam().length(), from, sweep, 48)
            }
            _ => Vec::new(),
        },
        GeometryElement::Ellipse(e) => e.points(sketch, 64).unwrap_or_default(),
        GeometryElement::Conic(c) => c.points(sketch, 64).unwrap_or_default(),
        GeometryElement::BSpline(b) => b.points(sketch, 64).unwrap_or_default(),
    };
    path.iter().any(|p| point_in_rect(*p, min, max))
        || path
            .windows(2)
            .any(|w| segment_meets_rect(w[0], w[1], min, max))
}

/// Whether the segment `a`..`b` passes through the box `min`..`max`
/// (clipped against each side in turn).
fn segment_meets_rect(a: Vec2D, b: Vec2D, min: Vec2D, max: Vec2D) -> bool {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (p, q) in [
        (-dx, a.x - min.x),
        (dx, max.x - a.x),
        (-dy, a.y - min.y),
        (dy, max.y - a.y),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

pub(super) fn element_fully_inside(
    sketch: &Sketch,
    geom: &GeometryElement,
    min: Vec2D,
    max: Vec2D,
) -> bool {
    let inside = |p: Vec2D| point_in_rect(p, min, max);
    match geom {
        GeometryElement::Point(p) => inside(p.position),
        GeometryElement::Line(l) => {
            match (sketch.point_position(l.start), sketch.point_position(l.end)) {
                (Some(a), Some(b)) => inside(a) && inside(b),
                _ => false,
            }
        }
        GeometryElement::Arc(a) => {
            let (Some(c), Some(s), Some(e)) = (
                sketch.point_position(a.center),
                sketch.point_position(a.start),
                sketch.point_position(a.end),
            ) else {
                return false;
            };
            let sv = (s - c).to_glam();
            let radius = sv.length();
            let (start_angle, sweep) = snap::arc_angles(sv, (e - c).to_glam());
            let mid_angle = start_angle + sweep * 0.5;
            let mid = Vec2D::new(
                c.x + radius * mid_angle.cos(),
                c.y + radius * mid_angle.sin(),
            );
            inside(c) && inside(s) && inside(e) && inside(mid)
        }
        GeometryElement::Circle(circle) => match sketch.point_position(circle.center) {
            Some(c) => {
                inside(Vec2D::new(c.x - circle.radius, c.y - circle.radius))
                    && inside(Vec2D::new(c.x + circle.radius, c.y + circle.radius))
            }
            None => false,
        },
        // Sampled boundary points all inside is exact enough for selection.
        GeometryElement::Ellipse(e) => e
            .points(sketch, 32)
            .is_some_and(|points| points.into_iter().all(inside)),
        GeometryElement::Conic(c) => c
            .points(sketch, 32)
            .is_some_and(|points| points.into_iter().all(inside)),
        // The spline lies in its control polygon's convex hull, so all
        // control points inside implies the curve is inside; the points it
        // is drawn through lie on it.
        GeometryElement::BSpline(b) => b
            .point_ids()
            .iter()
            .map(|id| sketch.point_position(*id))
            .all(|p| p.is_some_and(inside)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_order_follows_picks_removals_and_filters() {
        let ids: Vec<Uuid> = (0..4).map(|_| Uuid::new_v4()).collect();
        let mut s = Selection::default();
        for id in [ids[2], ids[0], ids[3]] {
            assert!(s.insert(id));
        }
        assert!(!s.insert(ids[0]), "already there");
        assert_eq!(s.in_order(), [ids[2], ids[0], ids[3]]);
        assert!(s.remove(&ids[0]));
        s.retain(|id| *id != ids[3]);
        assert_eq!(s.in_order(), [ids[2]]);
        assert!(s.contains(&ids[2]) && s.len() == 1);
    }
}
