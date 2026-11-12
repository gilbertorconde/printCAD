//! Reusing lines: a line a drawing click made between two points the
//! sketch already joins by a line of the same kind is that line. The new
//! line goes, its ends join the existing line's points, and what held the
//! new line holds the existing one.
//!
//! A drawn line reuses a drawn line and a construction line a
//! construction line; external geometry is never reused. A line running
//! the other way is reused unless an angle names the new line, since an
//! angle reads a line's direction.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::sketch::{self, ConstraintKind, GeometryElement, Line, Sketch};

/// What [`reuse_lines`] joined: each new line to the line it is, and each
/// new point to the existing point it is.
#[derive(Debug, Default)]
pub(crate) struct Reused {
    pub lines: HashMap<Uuid, Uuid>,
    pub points: HashMap<Uuid, Uuid>,
}

impl Reused {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The ids the joined lines and points are now known by.
    pub fn kept(&self) -> HashSet<Uuid> {
        self.lines
            .values()
            .chain(self.points.values())
            .copied()
            .collect()
    }

    /// `id` as it is after the join.
    pub fn renamed(&self, id: Uuid) -> Uuid {
        self.lines
            .get(&id)
            .or_else(|| self.points.get(&id))
            .copied()
            .unwrap_or(id)
    }
}

/// Every line made since `before` (the geometry ids the sketch held)
/// that repeats one held then, joined into it. A new end within `eps` of
/// the held line's end is that end.
pub(crate) fn reuse_lines(sketch: &mut Sketch, before: &HashSet<Uuid>, eps: f64) -> Reused {
    let external = sketch.external_ids();
    let line = |g: &GeometryElement| match g {
        GeometryElement::Line(l) => Some(l.clone()),
        _ => None,
    };
    let held: Vec<Line> = sketch
        .geometry
        .iter()
        .filter(|g| before.contains(&g.id()) && !external.contains(&g.id()))
        .filter_map(line)
        .collect();
    let made: Vec<Line> = sketch
        .geometry
        .iter()
        .filter(|g| !before.contains(&g.id()))
        .filter_map(line)
        .collect();
    let mut reused = Reused::default();
    if held.is_empty() {
        return reused;
    }
    for new in &made {
        let construction = sketch.is_construction(new.id);
        let angled = sketch.constraints.iter().any(|c| {
            matches!(
                c.kind,
                ConstraintKind::Angle { .. }
                    | ConstraintKind::AngleToAxis { .. }
                    | ConstraintKind::AngleAtPoint { .. }
            ) && sketch::constraint_refs(&c.kind).contains(&new.id)
        });
        // An end is a held point already, or a new one lying on it.
        let is = |end: Uuid, point: Uuid, reused: &Reused| {
            let end = reused.points.get(&end).copied().unwrap_or(end);
            end == point
                || (!before.contains(&end)
                    && sketch
                        .point_position(end)
                        .zip(sketch.point_position(point))
                        .is_some_and(|(a, b)| (a - b).to_glam().length() <= eps))
        };
        let found = held.iter().find_map(|old| {
            if sketch.is_construction(old.id) != construction {
                return None;
            }
            if is(new.start, old.start, &reused) && is(new.end, old.end, &reused) {
                Some((old, [old.start, old.end]))
            } else if !angled && is(new.start, old.end, &reused) && is(new.end, old.start, &reused)
            {
                Some((old, [old.end, old.start]))
            } else {
                None
            }
        });
        let Some((old, [to_start, to_end])) = found else {
            continue;
        };
        reused.lines.insert(new.id, old.id);
        for (end, to) in [(new.start, to_start), (new.end, to_end)] {
            let end = reused.points.get(&end).copied().unwrap_or(end);
            if end != to {
                reused.points.insert(end, to);
            }
        }
    }
    if reused.is_empty() {
        return reused;
    }
    crate::repair::join_points(sketch, &reused.points);
    crate::repair::rename_all(sketch, &reused.lines);
    sketch
        .geometry
        .retain(|g| !reused.lines.contains_key(&g.id()));
    for id in reused.lines.keys() {
        sketch.construction.remove(id);
    }
    reused
}
