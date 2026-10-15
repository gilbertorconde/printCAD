//! Repairing a sketch: what stops an outline from closing or solving, put
//! right. Ends of curves that nearly meet become one point, curves of no
//! length and duplicates go, and constraints left naming nothing, or the
//! same thing twice, go with them. Imported outlines need this most.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::sketch::{self, GeometryElement, Sketch};

/// What a repair did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Repaired {
    /// Pairs of ends joined into one point.
    pub joined: usize,
    /// Curves of no length or size taken away.
    pub degenerate: usize,
    /// Curves another one repeated, taken away.
    pub duplicates: usize,
    /// Constraints left naming nothing, or one thing twice.
    pub constraints: usize,
}

impl Repaired {
    pub fn is_empty(&self) -> bool {
        *self == Repaired::default()
    }

    /// In words, for the log.
    pub fn describe(&self) -> String {
        if self.is_empty() {
            return "nothing to repair".to_string();
        }
        let parts: Vec<String> = [
            (self.joined, "end(s) joined"),
            (self.degenerate, "curve(s) of no size removed"),
            (self.duplicates, "duplicate curve(s) removed"),
            (self.constraints, "stale constraint(s) removed"),
        ]
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, what)| format!("{n} {what}"))
        .collect();
        parts.join(", ")
    }
}

/// The points that end a curve: where outlines join.
fn curve_ends(sketch: &Sketch) -> Vec<Uuid> {
    let mut ends = Vec::new();
    for g in &sketch.geometry {
        ends.extend(Sketch::curve_ends(g).into_iter().flatten());
    }
    let mut seen = HashSet::new();
    ends.retain(|id| seen.insert(*id));
    ends
}

/// Every id `map` names replaced with the one it maps to, wherever it is
/// held in `value`.
fn rename(value: &mut serde_json::Value, map: &HashMap<Uuid, Uuid>) {
    match value {
        serde_json::Value::String(text) => {
            if let Ok(id) = Uuid::parse_str(text)
                && let Some(to) = map.get(&id)
            {
                *text = to.to_string();
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(|v| rename(v, map)),
        serde_json::Value::Object(fields) => fields.values_mut().for_each(|v| rename(v, map)),
        _ => {}
    }
}

/// The sketch's references renamed through `map`: its curves' points and
/// its constraints'.
fn rename_all(sketch: &mut Sketch, map: &HashMap<Uuid, Uuid>) {
    if map.is_empty() {
        return;
    }
    for g in &mut sketch.geometry {
        if let GeometryElement::Point(_) = g {
            continue;
        }
        if let Ok(mut value) = serde_json::to_value(&*g) {
            rename(&mut value, map);
            // The element keeps its own id: only what it refers to moves.
            if let Ok(renamed) = serde_json::from_value::<GeometryElement>(value)
                && renamed.id() == g.id()
            {
                *g = renamed;
            }
        }
    }
    for c in &mut sketch.constraints {
        if let Ok(mut value) = serde_json::to_value(&c.kind) {
            rename(&mut value, map);
            if let Ok(kind) = serde_json::from_value(value) {
                c.kind = kind;
            }
        }
    }
}

/// Join each point in `joined` into the point it maps to, carrying every
/// reference over. The kept point is construction only when every point
/// joined into it was: a drawn end stays drawn.
pub(crate) fn join_points(sketch: &mut Sketch, joined: &HashMap<Uuid, Uuid>) {
    let mut drawn: HashSet<Uuid> = HashSet::new();
    for (alias, kept) in joined {
        for id in [alias, kept] {
            if !sketch.construction.contains(id) {
                drawn.insert(*kept);
            }
        }
    }
    rename_all(sketch, joined);
    sketch
        .geometry
        .retain(|g| !matches!(g, GeometryElement::Point(p) if joined.contains_key(&p.id)));
    sketch
        .construction
        .retain(|id| !joined.contains_key(id) && !drawn.contains(id));
}

/// Repair `sketch`, ends within `tolerance` of each other joined.
pub fn repair(sketch: &mut Sketch, tolerance: f64) -> Repaired {
    let mut done = Repaired::default();

    // Ends that nearly meet: each joined into the first of them.
    let ends = curve_ends(sketch);
    let mut joined: HashMap<Uuid, Uuid> = HashMap::new();
    for (i, a) in ends.iter().enumerate() {
        if joined.contains_key(a) {
            continue;
        }
        let Some(pa) = sketch.point_position(*a) else {
            continue;
        };
        for b in &ends[i + 1..] {
            if joined.contains_key(b) {
                continue;
            }
            let Some(pb) = sketch.point_position(*b) else {
                continue;
            };
            if (pa - pb).to_glam().length() <= tolerance {
                joined.insert(*b, *a);
            }
        }
    }
    done.joined = joined.len();
    join_points(sketch, &joined);

    // Curves of no size.
    let length = |sketch: &Sketch, a: Uuid, b: Uuid| match (
        sketch.point_position(a),
        sketch.point_position(b),
    ) {
        (Some(pa), Some(pb)) => (pa - pb).to_glam().length(),
        _ => 0.0,
    };
    let degenerate: HashSet<Uuid> = sketch
        .geometry
        .iter()
        .filter_map(|g| {
            let none = match g {
                GeometryElement::Line(l) => {
                    l.start == l.end || length(sketch, l.start, l.end) <= tolerance
                }
                GeometryElement::Arc(a) => {
                    a.radius <= tolerance
                        || a.start == a.end
                        || length(sketch, a.start, a.end) <= tolerance * 0.01
                }
                GeometryElement::Circle(c) => c.radius <= tolerance,
                _ => false,
            };
            none.then(|| g.id())
        })
        .collect();
    done.degenerate = degenerate.len();

    // A curve another repeats: the same ends, or the same centre and size.
    let mut kept: Vec<(String, Uuid)> = Vec::new();
    let mut duplicate: HashMap<Uuid, Uuid> = HashMap::new();
    for g in &sketch.geometry {
        if degenerate.contains(&g.id()) {
            continue;
        }
        let key = match g {
            GeometryElement::Line(l) => {
                let (a, b) = if l.start < l.end {
                    (l.start, l.end)
                } else {
                    (l.end, l.start)
                };
                format!("line {a} {b}")
            }
            GeometryElement::Arc(a) => format!(
                "arc {} {} {} {:.0}",
                a.center,
                a.start,
                a.end,
                a.radius / tolerance.max(1e-6)
            ),
            GeometryElement::Circle(c) => {
                format!("circle {} {:.0}", c.center, c.radius / tolerance.max(1e-6))
            }
            _ => continue,
        };
        match kept.iter().find(|(k, _)| *k == key) {
            Some((_, first)) => {
                duplicate.insert(g.id(), *first);
            }
            None => kept.push((key, g.id())),
        }
    }
    done.duplicates = duplicate.len();
    // What held the duplicate holds the one kept.
    rename_all(sketch, &duplicate);
    sketch
        .geometry
        .retain(|g| !degenerate.contains(&g.id()) && !duplicate.contains_key(&g.id()));

    // Constraints naming nothing, or one thing twice.
    let ids: HashSet<Uuid> = sketch
        .geometry
        .iter()
        .map(GeometryElement::id)
        .chain([sketch::ORIGIN_ID, sketch::X_AXIS_ID, sketch::Y_AXIS_ID])
        .collect();
    let before = sketch.constraints.len();
    sketch.constraints.retain(|c| {
        let refs = sketch::constraint_refs(&c.kind);
        let unique: HashSet<&Uuid> = refs.iter().collect();
        refs.iter().all(|r| ids.contains(r)) && unique.len() == refs.len()
    });
    done.constraints = before - sketch.constraints.len();
    done
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Circle, ConstraintKind, Line, Point, Vec2D};

    fn point(sketch: &mut Sketch, x: f64, y: f64) -> Uuid {
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
    }

    #[test]
    fn a_triangle_that_nearly_closes_is_closed_and_its_litter_cleared() {
        let mut sketch = Sketch::new("t");
        // Three sides whose ends miss each other by a hair, as an import
        // leaves them.
        let (a, b) = (point(&mut sketch, 0.0, 0.0), point(&mut sketch, 10.0, 0.0));
        let (b2, c) = (
            point(&mut sketch, 10.0005, 0.0003),
            point(&mut sketch, 5.0, 8.0),
        );
        let (c2, a2) = (
            point(&mut sketch, 5.0002, 7.9998),
            point(&mut sketch, -0.0004, 0.0),
        );
        let ab = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        sketch.add_geometry(GeometryElement::Line(Line::new(b2, c)));
        sketch.add_geometry(GeometryElement::Line(Line::new(c2, a2)));
        // A second copy of the first side, a zero-length line, and a
        // constraint on the copy.
        let copy = sketch.add_geometry(GeometryElement::Line(Line::new(b, a)));
        let dot = point(&mut sketch, 3.0, 3.0);
        sketch.add_geometry(GeometryElement::Line(Line::new(dot, dot)));
        sketch.add_constraint(ConstraintKind::Horizontal { element: copy });
        let center = point(&mut sketch, 20.0, 0.0);
        sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 0.0)));
        assert!(
            crate::profile::extract_wires(&sketch).is_err(),
            "open to start with"
        );

        let done = repair(&mut sketch, 0.01);
        assert_eq!(done.joined, 3, "{done:?}");
        assert_eq!(done.degenerate, 2, "the dot and the circle of no size");
        assert_eq!(done.duplicates, 1);
        let wires = crate::profile::extract_wires(&sketch).expect("closed now");
        assert_eq!(wires.len(), 1);
        assert!(
            sketch
                .constraints
                .iter()
                .any(|c| matches!(c.kind, ConstraintKind::Horizontal { element } if element == ab)),
            "the copy's constraint holds the side kept"
        );
        assert!(
            repair(&mut sketch, 0.01).is_empty(),
            "a second repair finds nothing"
        );
    }

    #[test]
    fn repair_keeps_construction_only_where_every_joined_point_had_it() {
        let mut sketch = Sketch::new("outlines");
        let mut removed = Vec::new();
        let mut guide_points = Vec::new();
        let mut drawn_points = Vec::new();
        let mut guides = Vec::new();
        for (offset, construction) in [(0.0, false), (20.0, true)] {
            let a = point(&mut sketch, offset, 0.0);
            let b = point(&mut sketch, offset + 10.0, 0.0);
            let b2 = point(&mut sketch, offset + 10.0005, 0.0);
            let c = point(&mut sketch, offset + 5.0, 8.0);
            let c2 = point(&mut sketch, offset + 5.0005, 8.0);
            let a2 = point(&mut sketch, offset + 0.0005, 0.0);
            // Only the removed end is a guide at a; both are at b; only the
            // kept one is at c.
            sketch.set_construction(a2, true);
            sketch.set_construction(b, true);
            sketch.set_construction(b2, true);
            sketch.set_construction(c, true);
            removed.extend([a2, b2, c2]);
            guide_points.push(b);
            drawn_points.extend([a, c]);
            for (start, end) in [(a, b), (b2, c), (c2, a2)] {
                let line = sketch.add_geometry(GeometryElement::Line(Line::new(start, end)));
                sketch.set_construction(line, construction);
                if construction {
                    guides.push(line);
                }
            }
        }
        let done = repair(&mut sketch, 0.01);
        assert_eq!(done.joined, 6);
        assert_eq!(done.degenerate + done.duplicates + done.constraints, 0);
        for id in removed {
            assert!(sketch.get_geometry(id).is_none());
            assert!(!sketch.construction.contains(&id), "no stale membership");
        }
        for id in guide_points.into_iter().chain(guides) {
            assert!(
                sketch.is_construction(id),
                "a guide joined to a guide stays one"
            );
        }
        for id in drawn_points {
            assert!(!sketch.is_construction(id), "a drawn end stays drawn");
        }
        let restored: Sketch =
            serde_json::from_value(serde_json::to_value(&sketch).unwrap()).unwrap();
        assert_eq!(restored.construction, sketch.construction);
        let wires = crate::profile::extract_wires(&restored).unwrap();
        assert_eq!(wires.len(), 1, "only the drawn triangle defines a profile");
        assert!(repair(&mut sketch, 0.01).is_empty());
    }
}
