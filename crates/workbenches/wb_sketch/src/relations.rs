//! How a sketch's elements and constraints relate, as the task panel's
//! lists show it and the view lights it: each constraint's operands, the
//! constraints on each element, and what counts as related to an item in
//! hand.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::sketch::{self, Sketch};

/// One sketch's relations, worked out once a frame from its lists.
#[derive(Debug, Default)]
pub(crate) struct Relations {
    /// Each constraint's elements, each once, in the order it names them.
    operands: HashMap<Uuid, Vec<Uuid>>,
    /// Each element's constraints, in list order: those naming it or one
    /// of its own points (a line's ends, a circle's centre).
    on_element: HashMap<Uuid, Vec<Uuid>>,
    /// Each point's curves: those it ends, centres or shapes.
    owners: HashMap<Uuid, Vec<Uuid>>,
    /// Each curve's own points.
    points: HashMap<Uuid, Vec<Uuid>>,
}

/// The rows related to what is in hand.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Related {
    pub elements: HashSet<Uuid>,
    pub constraints: HashSet<Uuid>,
}

impl Relations {
    pub(crate) fn of(sketch: &Sketch) -> Self {
        let mut out = Relations::default();
        for element in &sketch.geometry {
            let points = Sketch::curve_point_ids(element);
            for point in &points {
                out.owners.entry(*point).or_default().push(element.id());
            }
            out.points.insert(element.id(), points);
        }
        for c in &sketch.constraints {
            let mut named: Vec<Uuid> = Vec::new();
            for id in sketch::constraint_refs(&c.kind) {
                if !named.contains(&id) {
                    named.push(id);
                }
            }
            let mut reached: Vec<Uuid> = named.clone();
            for id in &named {
                for curve in out.owners.get(id).into_iter().flatten() {
                    if !reached.contains(curve) {
                        reached.push(*curve);
                    }
                }
            }
            for id in reached {
                out.on_element.entry(id).or_default().push(c.id);
            }
            out.operands.insert(c.id, named);
        }
        out
    }

    /// The elements `constraint` names, each once.
    pub(crate) fn operands(&self, constraint: Uuid) -> &[Uuid] {
        self.operands.get(&constraint).map_or(&[], Vec::as_slice)
    }

    /// The constraints on `element` or on one of its own points.
    pub(crate) fn constraints_on(&self, element: Uuid) -> &[Uuid] {
        self.on_element.get(&element).map_or(&[], Vec::as_slice)
    }

    /// What `constraint` holds: the elements it names and the curves
    /// owning a point it names.
    pub(crate) fn held_by(&self, constraint: Uuid) -> impl Iterator<Item = Uuid> + '_ {
        self.operands(constraint).iter().flat_map(|id| {
            std::iter::once(*id).chain(self.owners.get(id).into_iter().flatten().copied())
        })
    }

    /// The rows related to `elements` and `constraints`: a constraint's
    /// held elements; an element's constraints, its own points and the
    /// curves sharing a point with it. What is in hand is left out.
    pub(crate) fn related(
        &self,
        elements: impl IntoIterator<Item = Uuid>,
        constraints: impl IntoIterator<Item = Uuid>,
    ) -> Related {
        let mut out = Related::default();
        let elements: HashSet<Uuid> = elements.into_iter().collect();
        let constraints: HashSet<Uuid> = constraints.into_iter().collect();
        for c in &constraints {
            out.elements.extend(self.held_by(*c));
        }
        for e in &elements {
            out.constraints.extend(self.constraints_on(*e));
            let own = self.points.get(e).into_iter().flatten();
            for point in own.chain(std::iter::once(e)) {
                out.elements.insert(*point);
                out.elements
                    .extend(self.owners.get(point).into_iter().flatten());
            }
        }
        out.elements.retain(|id| !elements.contains(id));
        out.constraints.retain(|id| !constraints.contains(id));
        out
    }
}
