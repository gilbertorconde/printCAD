//! Edits that only add to a sketch: elements and constraints after the
//! ones it holds, nothing it holds changed. What they add is known as
//! they add it, so the solve, the patch the store records and the next
//! edit's lookups follow only that, never the whole sketch.
//!
//! [`SketchIndex`] is what such an edit reads besides the sketch: its ids,
//! its points by position and the solver's split of its problem. It
//! describes one sketch exactly, so it is kept beside that sketch as
//! stored and dropped once the stored sketch changes otherwise.

use uuid::Uuid;

use crate::sketch::{ConstraintKind, GeometryElement, IdMap, Point, Sketch, Vec2D};

/// How far apart two points are and still the same point to
/// [`Adding::point_at`].
pub const SAME_POINT: f64 = 1e-5;

/// Cells a millimetre of the points' grid spans: wider than
/// [`SAME_POINT`], so a point that near lies in one of the nine cells
/// round the place looked up.
const CELLS_PER_MM: f64 = 64.0;

/// What an edit that only adds to a sketch reads of the sketch besides the
/// sketch itself, for exactly one sketch.
#[derive(Debug, Clone)]
pub struct SketchIndex {
    elements: usize,
    constraints: usize,
    element_ids: IdMap<()>,
    constraint_ids: IdMap<()>,
    /// Each point's place in the list, by the cell its position is in. A
    /// moved point stays listed in the cell it left too; a lookup checks
    /// the position.
    points: std::collections::HashMap<(i64, i64), Vec<u32>, sketch_solver::problem::IdHash>,
    /// The sketch holds nothing the edit's solve and patch cannot follow
    /// by what was added and what the solve wrote alone ([`plain_element`],
    /// [`plain_constraint`]), and no id twice.
    plain: bool,
    /// The solver's problem of the sketch, where its split is kept.
    pub(crate) base: Option<crate::solver::Base>,
}

/// Whether an added-to solve and patch follow `element` by what was added
/// and what the solve wrote: a point, line, circle or arc with every value
/// equal to itself. An ellipse or conic can turn its axes and place foci,
/// which changes what names them; a spline is refitted.
fn plain_element(element: &GeometryElement) -> bool {
    matches!(
        element,
        GeometryElement::Point(_)
            | GeometryElement::Line(_)
            | GeometryElement::Circle(_)
            | GeometryElement::Arc(_)
    ) && equal_to_itself(element)
}

/// [`plain_element`] for a constraint: no group (its shape is read from its
/// members as they stand) and no internal alignment.
fn plain_constraint(constraint: &crate::sketch::Constraint) -> bool {
    !matches!(
        constraint.kind,
        ConstraintKind::Group { .. } | ConstraintKind::InternalAlignment { .. }
    ) && equal_to_itself(constraint)
}

/// Whether every value in `item` equals itself: none is not a number.
fn equal_to_itself<T: PartialEq>(item: &T) -> bool {
    let same = item;
    item == same
}

fn cell(at: Vec2D) -> (i64, i64) {
    (
        (at.x * CELLS_PER_MM).floor() as i64,
        (at.y * CELLS_PER_MM).floor() as i64,
    )
}

impl SketchIndex {
    /// The index of `sketch`, worked out over all of it.
    pub fn of(sketch: &Sketch) -> Self {
        let mut index = Self {
            elements: 0,
            constraints: 0,
            element_ids: IdMap::default(),
            constraint_ids: IdMap::default(),
            points: Default::default(),
            plain: sketch.texts.is_empty() && sketch.derived.is_empty() && sketch.soft.is_empty(),
            base: None,
        };
        index.element_ids.reserve(sketch.geometry.len());
        index.constraint_ids.reserve(sketch.constraints.len());
        for element in &sketch.geometry {
            index.add_element(element);
        }
        for constraint in &sketch.constraints {
            index.add_constraint(constraint);
        }
        index
    }

    /// Whether this may be `sketch`'s index: it counts as many elements and
    /// constraints.
    pub fn sizes_match(&self, sketch: &Sketch) -> bool {
        self.elements == sketch.geometry.len() && self.constraints == sketch.constraints.len()
    }

    /// Whether the sketch it describes holds only what an added-to solve
    /// and patch can follow.
    pub fn plain(&self) -> bool {
        self.plain
    }

    fn add_element(&mut self, element: &GeometryElement) {
        self.plain &= plain_element(element);
        self.plain &= self.element_ids.insert(element.id(), ()).is_none();
        if let GeometryElement::Point(p) = element {
            self.list_point(self.elements as u32, p.position);
        }
        self.elements += 1;
    }

    fn add_constraint(&mut self, constraint: &crate::sketch::Constraint) {
        self.plain &= plain_constraint(constraint);
        self.plain &= self.constraint_ids.insert(constraint.id, ()).is_none();
        self.constraints += 1;
    }

    fn list_point(&mut self, at: u32, position: Vec2D) {
        let listed = self.points.entry(cell(position)).or_default();
        if !listed.contains(&at) {
            listed.push(at);
        }
    }

    /// Follow the elements at `written` (places in `sketch`'s list) to
    /// where a solve put them.
    pub(crate) fn follow(&mut self, sketch: &Sketch, written: &[usize]) {
        for &i in written {
            if let Some(GeometryElement::Point(p)) = sketch.geometry.get(i) {
                self.list_point(i as u32, p.position);
            }
        }
    }

    /// The first point of `sketch` (in its list's order) within
    /// [`SAME_POINT`] of `at`.
    fn point_near(&self, sketch: &Sketch, at: Vec2D) -> Option<Uuid> {
        let (x, y) = cell(at);
        let mut first: Option<(u32, Uuid)> = None;
        for cx in x.saturating_sub(1)..=x.saturating_add(1) {
            for cy in y.saturating_sub(1)..=y.saturating_add(1) {
                for &k in self.points.get(&(cx, cy)).into_iter().flatten() {
                    if first.is_some_and(|(f, _)| f <= k) {
                        continue;
                    }
                    if let Some(GeometryElement::Point(p)) = sketch.geometry.get(k as usize)
                        && (p.position - at).to_glam().length() < SAME_POINT
                    {
                        first = Some((k, p.id));
                    }
                }
            }
        }
        first.map(|(_, id)| id)
    }
}

/// A sketch an edit only adds to, with its index kept up with what it
/// adds.
pub struct Adding<'a> {
    sketch: &'a mut Sketch,
    index: &'a mut SketchIndex,
}

impl<'a> Adding<'a> {
    /// `sketch` to add to, `index` its [`SketchIndex`].
    pub fn new(sketch: &'a mut Sketch, index: &'a mut SketchIndex) -> Self {
        debug_assert!(index.sizes_match(sketch));
        Self { sketch, index }
    }

    pub fn sketch(&self) -> &Sketch {
        self.sketch
    }

    /// Add `element` after the sketch's elements.
    pub fn add_geometry(&mut self, element: GeometryElement) -> Uuid {
        self.index.add_element(&element);
        self.sketch.add_geometry(element)
    }

    /// Add a constraint (driving, active, unnamed) after the sketch's
    /// constraints.
    pub fn add_constraint(&mut self, kind: ConstraintKind) -> Uuid {
        let constraint = crate::sketch::Constraint::new(kind);
        let id = constraint.id;
        self.index.add_constraint(&constraint);
        self.sketch.constraints.push(constraint);
        id
    }

    /// The point at `at`: the sketch's first within [`SAME_POINT`] of it,
    /// else a new one.
    pub fn point_at(&mut self, at: Vec2D) -> Uuid {
        match self.index.point_near(self.sketch, at) {
            Some(id) => id,
            None => self.add_geometry(GeometryElement::Point(Point::new(at))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The grid finds what a walk over the list finds: the first point in
    /// the list within reach, for places on, near and between points,
    /// across cell edges, and for none.
    #[test]
    fn a_point_is_found_as_a_walk_over_the_list_finds_it() {
        let mut sketch = Sketch::new("s");
        let mut index = SketchIndex::of(&sketch);
        let edge = 1.0 / CELLS_PER_MM;
        let places = [
            Vec2D::new(0.0, 0.0),
            Vec2D::new(edge, edge),
            Vec2D::new(edge - 4e-6, edge),
            Vec2D::new(edge + 4e-6, edge - 4e-6),
            Vec2D::new(-edge, 3.0),
            Vec2D::new(-edge - 9e-6, 3.0),
            Vec2D::new(10.0, -2.5),
        ];
        let mut adding = Adding::new(&mut sketch, &mut index);
        for at in places {
            adding.add_geometry(GeometryElement::Point(Point::new(at)));
        }
        let walk = |sketch: &Sketch, at: Vec2D| {
            sketch.geometry.iter().find_map(|g| match g {
                GeometryElement::Point(p) if (p.position - at).to_glam().length() < SAME_POINT => {
                    Some(p.id)
                }
                _ => None,
            })
        };
        let probes = places.iter().flat_map(|p| {
            [-9e-6, -5e-6, 0.0, 5e-6, 1.1e-5]
                .into_iter()
                .map(move |d| Vec2D::new(p.x + d, p.y - d))
        });
        for at in probes.chain([Vec2D::new(5.0, 5.0)]) {
            assert_eq!(index.point_near(&sketch, at), walk(&sketch, at), "{at:?}");
        }
    }

    /// An index over a sketch with an id twice, or an element an added-to
    /// solve cannot follow, says it is not plain; one over points and
    /// lines says it is.
    #[test]
    fn an_index_says_what_it_can_follow() {
        let mut sketch = Sketch::new("s");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(1.0, 0.0))));
        sketch.add_geometry(GeometryElement::Line(crate::sketch::Line::new(a, b)));
        assert!(SketchIndex::of(&sketch).plain());
        let mut twice = sketch.clone();
        twice.geometry.push(sketch.geometry[0].clone());
        assert!(!SketchIndex::of(&twice).plain());
        let mut odd = sketch.clone();
        odd.geometry
            .push(GeometryElement::Point(Point::new(Vec2D::new(
                f64::NAN,
                0.0,
            ))));
        assert!(!SketchIndex::of(&odd).plain());
    }

    /// A small generator, the same sequence every run.
    struct Draws(u64);

    impl Draws {
        fn below(&mut self, n: u64) -> u64 {
            self.0 = (self.0.wrapping_mul(6364136223846793005)).wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }

        /// A place on a coarse lattice, now and then a hair off one, so
        /// ends land on points the sketch has.
        fn place(&mut self) -> Vec2D {
            let off = [0.0, 3e-6, 2e-5][self.below(3) as usize];
            Vec2D::new(self.below(5) as f64 * 2.0 + off, self.below(5) as f64 - off)
        }

        fn pick(&mut self, sketch: &Sketch, point: bool) -> Option<Uuid> {
            let ids: Vec<Uuid> = (sketch.geometry.iter())
                .filter(|g| matches!(g, GeometryElement::Point(_)) == point)
                .map(GeometryElement::id)
                .collect();
            (!ids.is_empty()).then(|| ids[self.below(ids.len() as u64) as usize])
        }
    }

    /// Add one random thing: a point, line, circle or arc through
    /// [`Adding::point_at`], or a constraint that moves what it names.
    fn add_one(adding: &mut Adding, draws: &mut Draws) {
        use crate::sketch::{Arc, Circle, Line};
        match draws.below(8) {
            0 => {
                adding.point_at(draws.place());
            }
            1 | 2 => {
                let (a, b) = (draws.place(), draws.place());
                if (b - a).to_glam().length() >= SAME_POINT {
                    let (a, b) = (adding.point_at(a), adding.point_at(b));
                    adding.add_geometry(GeometryElement::Line(Line::new(a, b)));
                }
            }
            3 => {
                let c = adding.point_at(draws.place());
                let r = 0.5 + draws.below(3) as f64;
                adding.add_geometry(GeometryElement::Circle(Circle::new(c, r)));
            }
            4 => {
                let c = adding.point_at(draws.place());
                let s = adding.point_at(draws.place());
                let e = adding.point_at(draws.place());
                if c != s && c != e && s != e {
                    adding.add_geometry(GeometryElement::Arc(Arc::new(c, s, e, 1.5)));
                }
            }
            5 => {
                if let Some(line) = (draws.pick(adding.sketch(), false)).filter(|id| {
                    matches!(
                        adding.sketch().get_geometry(*id),
                        Some(GeometryElement::Line(_))
                    )
                }) {
                    adding.add_constraint(ConstraintKind::Horizontal { element: line });
                }
            }
            6 => {
                let a = draws.pick(adding.sketch(), true);
                let b = draws.pick(adding.sketch(), true);
                if let (Some(point1), Some(point2)) = (a, b)
                    && point1 != point2
                {
                    adding.add_constraint(ConstraintKind::Distance {
                        point1,
                        point2,
                        distance: 1.0 + draws.below(4) as f64,
                    });
                }
            }
            _ => {
                if let Some(point) = draws.pick(adding.sketch(), true) {
                    let position = draws.place();
                    adding.add_constraint(ConstraintKind::FixedPoint { point, position });
                }
            }
        }
    }

    /// Over random runs of edits that only add, each solved and stored by
    /// what it added: the solve leaves the sketch the whole solve leaves,
    /// to the bit, and lists exactly the elements it changed; the patch
    /// written from those lists is the one comparing the whole sketch
    /// with the one before writes; and the index kept up with them finds
    /// every point the index worked out afresh finds.
    #[test]
    fn what_an_edit_added_is_solved_and_stored_as_the_whole_sketch_is() {
        use crate::feature::SketchFeature;
        use core_document::WorkbenchFeature;
        let (mut moved, mut based) = (0, 0);
        for seed in 0..8 {
            let mut draws = Draws(seed);
            let mut feature = SketchFeature::from_sketch(Sketch::new("s"));
            let mut index = SketchIndex::of(&feature.sketch);
            for _ in 0..30 {
                let before = feature.clone();
                let stored = before.to_json();
                let sizes = (
                    before.sketch.geometry.len(),
                    before.sketch.constraints.len(),
                );
                let mut adding = Adding::new(&mut feature.sketch, &mut index);
                for _ in 0..=draws.below(3) {
                    add_one(&mut adding, &mut draws);
                }
                assert!(index.plain());
                let unsolved = feature.clone();
                let mut whole = feature.clone();
                let outcome = crate::solver::solve(&mut whole.sketch);
                let added = crate::solver::solve_added(&mut feature.sketch, &mut index, sizes);
                assert_eq!(added.outcome, outcome);
                assert_eq!(feature.to_json(), whole.to_json());
                assert_eq!(feature.sketch, whole.sketch);
                let differ: Vec<usize> = (0..feature.sketch.geometry.len())
                    .filter(|&k| unsolved.sketch.geometry[k] != feature.sketch.geometry[k])
                    .collect();
                assert_eq!(added.written, differ);
                // The problem kept for the next edit is the sketch's, split
                // into the parts a split afresh finds.
                let now = (
                    feature.sketch.geometry.len(),
                    feature.sketch.constraints.len(),
                );
                if let Some(base) = index.base {
                    let kept = crate::solver::Prepared::grown(&feature.sketch, base, now, &[])
                        .expect("the problem is kept");
                    let afresh = crate::solver::Prepared::of(&feature.sketch);
                    assert_eq!(kept.problem(), afresh.problem());
                    let alone = sketch_solver::parts::Parted::of(
                        &std::sync::Arc::new(afresh.problem().clone()),
                        &mut Default::default(),
                    );
                    let mut alone: Vec<_> = alone.systems().map(|(p, _)| p.clone()).collect();
                    alone.sort_by_key(|p| p.geometry.first().map(|g| g.id()));
                    assert_eq!(kept.part_problems(), alone);
                    index.base = kept.base();
                    based += 1;
                }
                let kept = added.written.partition_point(|&k| k < sizes.0);
                moved += kept;
                let patch =
                    feature
                        .clone()
                        .patch_added(&stored, sizes.0, sizes.1, &added.written[..kept]);
                let full = feature.clone().patch_over(&before, &stored);
                assert!(full.is_some());
                assert_eq!(patch, full);

                let afresh = SketchIndex::of(&feature.sketch);
                assert!(afresh.plain() && index.sizes_match(&feature.sketch));
                for _ in 0..20 {
                    let at = draws.place();
                    assert_eq!(
                        index.point_near(&feature.sketch, at),
                        afresh.point_near(&feature.sketch, at)
                    );
                }
            }
        }
        assert!(
            moved > 20,
            "the solves moved what was there before: {moved}"
        );
        assert!(based > 60, "the problems were kept: {based}");
    }
}
