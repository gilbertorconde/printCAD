//! A problem by its parts: the geometry and constraints no shared point
//! and no constraint joins to the rest, each compiled on its own and kept
//! in a [`PartCache`], reused while it is equal to the part a problem
//! holds: an edit leaves every part it did not touch compiled, sorted
//! into blocks and analysed.
//!
//! Solved and analysed together, the parts answer what the problem
//! compiled as one system answers, to the bit. What joins the parts is
//! carried across them:
//! - The origin and the axes are pinned, so they join no Jacobian block:
//!   they join no parts either, and each part naming them compiles its
//!   own pinned copy.
//! - Arcs' own equations join only when some constraint anywhere brings
//!   an equation: each part is compiled with them (the system for rank),
//!   and again without them where it has arcs and no part brings one.
//! - Rank and determined variables are held to the whole Jacobian's
//!   largest entry times the rank tolerance: each part keeps its largest
//!   entry, and its rank is read against the largest of all.
//! - The convergence threshold, the step cap and the continuation's
//!   steps come from every part's values together
//!   ([`crate::solve::iterate_pieces`]).
//!
//! A part is reused when its elements, constraints, held points and
//! settings equal the kept one's, compared as values (a zero's sign is
//! not told apart). A problem whose geometry ids or constraint ids
//! repeat, or with a constraint naming no geometry beside a group, is
//! one part.

use std::sync::{Arc, OnceLock};

use crate::compile::{System, compile_counting, compile_system};
use crate::freedom::{Freedom, Locked, locked_into, unsolved_shape};
use crate::problem::{
    ConstraintId, CurveReference, IdHash, IdMap, ItemReference, PointReference, Problem, Relation,
    Settings,
};
use crate::solve::{BlocksOf, Piece, Rest, iterate_pieces};
use crate::sparse::{Blocks, Differenced, scale_of};

/// One part, compiled.
pub struct Part {
    problem: Problem,
    /// Compiled with every arc's own equations: the system for rank, and
    /// the one solved whenever some part brings an equation.
    rank: Compiled,
    /// Its constraints bring an equation.
    constrained: bool,
    /// Some arc brings its own equations.
    arcs: bool,
    /// Compiled without the arcs' own equations, once asked for.
    plain: OnceLock<Compiled>,
}

/// A part's system and what is worked out on it, each once.
struct Compiled {
    sys: System,
    blocks: OnceLock<Blocks>,
    rest: OnceLock<Rest>,
    differenced: OnceLock<Differenced>,
    /// The rank at a pivot threshold (its bits).
    rank: OnceLock<(u64, usize)>,
    /// The determined variables at a pivot threshold (its bits).
    determined: OnceLock<(u64, Vec<bool>)>,
}

impl Compiled {
    fn new(sys: System) -> Self {
        Self {
            sys,
            blocks: OnceLock::new(),
            rest: OnceLock::new(),
            differenced: OnceLock::new(),
            rank: OnceLock::new(),
            determined: OnceLock::new(),
        }
    }

    fn rest(&self) -> &Rest {
        self.rest.get_or_init(|| Rest::of(&self.sys))
    }

    fn blocks(&self) -> &Blocks {
        self.blocks.get_or_init(|| Blocks::of(&self.sys))
    }

    fn differenced(&self) -> &Differenced {
        self.differenced
            .get_or_init(|| Differenced::of(&self.sys, self.blocks()))
    }

    fn rank_at(&self, tol: f64) -> usize {
        match self.rank.get() {
            Some(&(bits, rank)) if bits == tol.to_bits() => rank,
            _ => {
                let rank = self.differenced().rank(tol);
                let _ = self.rank.set((tol.to_bits(), rank));
                rank
            }
        }
    }

    fn determined_at(&self, tol: f64, rank_tolerance: f64) -> std::borrow::Cow<'_, [bool]> {
        let work = || {
            (self.differenced()).determined(self.blocks(), self.sys.free.len(), tol, rank_tolerance)
        };
        if self.determined.get().is_none() {
            let _ = self.determined.set((tol.to_bits(), work()));
        }
        match self.determined.get() {
            Some((bits, held)) if *bits == tol.to_bits() => held.into(),
            _ => work().into(),
        }
    }

    fn piece(&self) -> Piece<'_> {
        Piece {
            sys: &self.sys,
            blocks: BlocksOf::Lazy(&self.blocks),
            rest: self.rest(),
        }
    }
}

impl Part {
    fn compile(problem: Problem) -> Self {
        let counted = compile_counting(&problem, None, true);
        Self {
            problem,
            rank: Compiled::new(counted.system),
            constrained: counted.constrained,
            arcs: counted.arcs,
            plain: OnceLock::new(),
        }
    }

    /// The system a solve iterates, given whether any part brings an
    /// equation.
    fn plain(&self, constrained: bool) -> &Compiled {
        if constrained || !self.arcs {
            &self.rank
        } else {
            (self.plain).get_or_init(|| Compiled::new(compile_system(&self.problem, None, false)))
        }
    }
}

/// A problem's parts, in the order of their first element.
#[derive(Clone)]
pub struct Parted {
    parts: Vec<Arc<Part>>,
    /// Some part's constraints bring an equation.
    constrained: bool,
    settings: Settings,
}

impl std::fmt::Debug for Parted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Parted")
            .field("parts", &self.parts.len())
            .finish_non_exhaustive()
    }
}

/// What [`Parted::iterate`] answers: each part's values, `None` where they
/// are the part's own, with the outcome of all of them together.
#[derive(Debug, Clone, PartialEq)]
pub struct Solved {
    pub values: Vec<Option<Vec<f64>>>,
    pub outcome: crate::solve::SolveOutcome,
    pub iterations: usize,
    pub residual: f64,
    pub threshold: f64,
}

impl Parted {
    /// `problem` as one part, compiled now and kept nowhere.
    pub fn whole(problem: Problem) -> Self {
        let part = Part::compile(problem);
        Self {
            constrained: part.constrained,
            settings: part.problem.settings,
            parts: vec![Arc::new(part)],
        }
    }

    /// `problem` by its parts, each taken from `cache` where it holds an
    /// equal one and compiled (and kept there) where it does not. The
    /// split of a problem solved lately is updated where `problem` is that
    /// problem with values changed in place and elements and constraints
    /// added after, and worked out afresh otherwise.
    pub fn of(problem: &Arc<Problem>, cache: &mut PartCache) -> Self {
        cache.turn(problem.geometry.len());
        let parts = match cache.take_kept(problem) {
            Some(kept) => match kept.grow(problem, &mut cache.parts) {
                Some((kept, parts)) => {
                    cache.keep_split(kept);
                    parts
                }
                None => cache.split(problem),
            },
            None => cache.split(problem),
        };
        Self {
            constrained: parts.iter().any(|p| p.constrained),
            parts,
            settings: problem.settings,
        }
    }

    pub fn len(&self) -> usize {
        self.parts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// Each part's problem and the system its solve iterates, in the order
    /// [`Parted::iterate`] answers their values.
    pub fn systems(&self) -> impl Iterator<Item = (&Problem, &System)> {
        (self.parts.iter()).map(|p| (&p.problem, &p.plain(self.constrained).sys))
    }

    /// [`crate::solve::iterate`] of the problem's system.
    pub fn iterate(&self, settings: Settings) -> Solved {
        let pieces: Vec<Piece> = (self.parts.iter())
            .map(|p| p.plain(self.constrained).piece())
            .collect();
        let solved = iterate_pieces(&pieces, settings);
        Solved {
            values: solved.values,
            outcome: solved.outcome,
            iterations: solved.iterations,
            residual: solved.residual,
            threshold: solved.threshold,
        }
    }

    /// Whether the values already meet the equations, by the solve's own
    /// convergence test, without solving.
    pub fn holds(&self, tolerance: f64) -> bool {
        let systems = || self.parts.iter().map(|p| p.plain(self.constrained));
        if systems().all(|c| c.sys.specs.is_empty()) || systems().all(|c| c.sys.vars.is_empty()) {
            return true;
        }
        let (mut residual, mut size) = (0.0_f64, 0.0_f64);
        for c in systems() {
            let rest = c.rest();
            residual = residual.max(match rest.start {
                None => rest.residual,
                Some(_) => {
                    crate::solve::inf_norm(&crate::solve::eval_residuals(&c.sys, &c.sys.vars))
                }
            });
            size = size.max(rest.size);
        }
        residual < tolerance * size.max(1.0)
    }

    /// The pivot threshold the parts' rank is held to, `None` where there
    /// is no rank to read; and whether any equation reads a movable
    /// variable at all.
    fn pivots(&self) -> (bool, Option<f64>) {
        let ranked = (self.parts.iter()).any(|p| !p.rank.sys.specs.is_empty())
            && (self.parts.iter()).any(|p| !p.rank.sys.free.is_empty());
        if !ranked {
            return (false, None);
        }
        let largest = (self.parts.iter()).fold(0.0_f64, |m, p| m.max(p.rank.differenced().largest));
        (
            true,
            scale_of(largest).map(|m| m * self.settings.rank_tolerance),
        )
    }

    /// [`crate::freedom::estimate`] of the problem.
    pub fn estimate(&self) -> Freedom {
        let free_variables: usize = self.parts.iter().map(|p| p.rank.sys.free.len()).sum();
        let unsolved_shape: i32 = (self.parts.iter())
            .map(|p| unsolved_shape(&p.problem, &p.rank.sys))
            .sum();
        let rank = match self.pivots() {
            (true, Some(tol)) => self.parts.iter().map(|p| p.rank.rank_at(tol)).sum(),
            _ => 0,
        };
        Freedom {
            free_variables,
            rank,
            unsolved_shape,
            degrees: free_variables as i32 - rank as i32 + unsolved_shape,
        }
    }

    /// [`crate::freedom::locked`] of the problem.
    pub fn locked(&self) -> Locked {
        let (ranked, tol) = self.pivots();
        let mut out = Locked::default();
        for part in &self.parts {
            let sys = &part.rank.sys;
            let columns = ranked.then(|| match tol {
                Some(tol) => part.rank.determined_at(tol, self.settings.rank_tolerance),
                None => vec![false; sys.free.len()].into(),
            });
            locked_into(&part.problem, sys, columns.as_deref(), &mut out);
        }
        out
    }
}

/// Where each element and constraint of a problem goes: the parts by
/// their first element, a constraint naming no element in a part after
/// them.
struct Split {
    count: usize,
    /// Each part's geometry indices, then its constraint indices, ascending.
    order: Vec<u32>,
    /// Where each part's run starts in `order`: geometry, then constraints.
    starts: Vec<(usize, usize, usize)>,
    /// What [`KeptSplit`] updates this split from, where it can be.
    kept: Option<Joins>,
}

/// How a problem's elements and constraints join into parts.
struct Joins {
    places: IdMap<ItemReference, u32>,
    constraint_at: IdMap<ConstraintId, u32>,
    of_geometry: Vec<u32>,
    of_constraint: Vec<u32>,
    /// The part of the constraints naming no element.
    loose: Option<u32>,
    grouped: bool,
    /// Some element or constraint names an element the problem lacks.
    dangling: bool,
}

fn root(parent: &mut [u32], mut a: u32) -> u32 {
    while parent[a as usize] != a {
        parent[a as usize] = parent[parent[a as usize] as usize];
        a = parent[a as usize];
    }
    a
}

fn join(parent: &mut [u32], a: u32, b: u32) {
    let (a, b) = (root(parent, a), root(parent, b));
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    parent[hi as usize] = lo;
}

/// What a problem holds still in one part: its external geometry, held
/// points and application-held points there.
type Held = (Vec<ItemReference>, Vec<PointReference>, Vec<PointReference>);

/// What `problem` holds still, by the part `part` puts each element in;
/// what names no element holds nothing.
fn held_by_part(
    problem: &Problem,
    part: impl Fn(&ItemReference) -> Option<u32>,
) -> std::collections::BTreeMap<u32, Held> {
    let mut out: std::collections::BTreeMap<u32, Held> = Default::default();
    for r in &problem.external {
        if let Some(k) = part(r) {
            out.entry(k).or_default().0.push(*r);
        }
    }
    for p in &problem.held_points {
        if let Some(k) = part(&(*p).into()) {
            out.entry(k).or_default().1.push(*p);
        }
    }
    for p in &problem.application_held_points {
        if let Some(k) = part(&(*p).into()) {
            out.entry(k).or_default().2.push(*p);
        }
    }
    out
}

/// Whether the problem holds anything still in an element.
fn holds_any(problem: &Problem) -> bool {
    !problem.external.is_empty()
        || !problem.held_points.is_empty()
        || !problem.application_held_points.is_empty()
}

impl Split {
    fn of(problem: &Problem) -> Self {
        Self::parted(problem).unwrap_or_else(|| {
            let (n, m) = (problem.geometry.len(), problem.constraints.len());
            Self {
                count: 1,
                order: (0..n as u32).chain(0..m as u32).collect(),
                starts: vec![(0, n, n + m)],
                kept: None,
            }
        })
    }

    fn parted(problem: &Problem) -> Option<Self> {
        let n = problem.geometry.len();
        let mut places: IdMap<ItemReference, u32> = IdMap::with_capacity_and_hasher(n, IdHash);
        for (i, g) in problem.geometry.iter().enumerate() {
            if places.insert(g.id(), i as u32).is_some() {
                return None;
            }
        }
        let mut dangling = false;
        let mut parent: Vec<u32> = (0..n as u32).collect();
        for (i, g) in problem.geometry.iter().enumerate() {
            for p in g.point_references() {
                match places.get(&p.into()) {
                    Some(&j) => join(&mut parent, i as u32, j),
                    None => dangling |= p != PointReference::Origin,
                }
            }
        }
        let mut constraint_at: IdMap<ConstraintId, u32> =
            IdMap::with_capacity_and_hasher(problem.constraints.len(), IdHash);
        let mut anchors = Vec::with_capacity(problem.constraints.len());
        let mut loose = false;
        let mut grouped = false;
        for (k, c) in problem.constraints.iter().enumerate() {
            if constraint_at.insert(c.id, k as u32).is_some() {
                return None;
            }
            grouped |= matches!(c.kind, Relation::Group { .. });
            let mut anchor = None;
            for r in c.kind.references() {
                match places.get(&r) {
                    Some(&j) => match anchor {
                        None => anchor = Some(j),
                        Some(a) => join(&mut parent, a, j),
                    },
                    None => dangling |= !is_reference(r),
                }
            }
            loose |= anchor.is_none();
            anchors.push(anchor);
        }
        if loose && grouped {
            return None;
        }

        // Parts numbered by their first element; the loose constraints'
        // part last.
        let mut numbered = vec![u32::MAX; n];
        let mut of_geometry = Vec::with_capacity(n);
        let mut count = 0u32;
        for i in 0..n as u32 {
            let r = root(&mut parent, i) as usize;
            if numbered[r] == u32::MAX {
                numbered[r] = count;
                count += 1;
            }
            of_geometry.push(numbered[r]);
        }
        let loose_part = loose.then_some(count);
        if loose {
            count += 1;
        }
        let of_constraint: Vec<u32> = (anchors.iter())
            .map(|a| match a {
                Some(a) => of_geometry[*a as usize],
                None => count - 1,
            })
            .collect();
        let count = count as usize;

        // Each part's run: its geometry, then its constraints.
        let mut sizes = vec![(0usize, 0usize); count];
        for &k in &of_geometry {
            sizes[k as usize].0 += 1;
        }
        for &k in &of_constraint {
            sizes[k as usize].1 += 1;
        }
        let mut starts = Vec::with_capacity(count);
        let mut at = 0;
        for &(g, c) in &sizes {
            starts.push((at, at + g, at + g + c));
            at += g + c;
        }
        let mut fill: Vec<(usize, usize)> = starts.iter().map(|&(g, c, _)| (g, c)).collect();
        let mut order = vec![0u32; at];
        for (i, &k) in of_geometry.iter().enumerate() {
            let slot = &mut fill[k as usize].0;
            order[*slot] = i as u32;
            *slot += 1;
        }
        for (i, &k) in of_constraint.iter().enumerate() {
            let slot = &mut fill[k as usize].1;
            order[*slot] = i as u32;
            *slot += 1;
        }
        Some(Self {
            count,
            order,
            starts,
            kept: Some(Joins {
                places,
                constraint_at,
                of_geometry,
                of_constraint,
                loose: loose_part,
                grouped,
                dangling,
            }),
        })
    }

    fn geometry(&self, k: usize) -> &[u32] {
        let (g, c, _) = self.starts[k];
        &self.order[g..c]
    }

    fn constraints(&self, k: usize) -> &[u32] {
        let (_, c, end) = self.starts[k];
        &self.order[c..end]
    }
}

/// Whether `r` is the origin or an axis, which no element is.
fn is_reference(r: ItemReference) -> bool {
    matches!(
        r,
        ItemReference::Point(PointReference::Origin)
            | ItemReference::Curve(CurveReference::XAxis | CurveReference::YAxis)
    )
}

/// The part of `problem` holding the elements `geometry` and the
/// constraints `constraints` (indices, ascending): the kept one equal to
/// it, or compiled now and kept.
fn part_of(
    problem: &Problem,
    geometry: &[u32],
    constraints: &[u32],
    held: Held,
    parts: &mut Generations,
) -> Arc<Part> {
    let (external, held_points, application_held_points) = held;
    let key = geometry
        .first()
        .map_or(ItemReference::Point(PointReference::Origin), |&i| {
            problem.geometry[i as usize].id()
        });
    let same = |part: &Part| {
        let kept = &part.problem;
        kept.settings == problem.settings
            && kept.geometry.len() == geometry.len()
            && kept.constraints.len() == constraints.len()
            && kept.external == external
            && kept.held_points == held_points
            && kept.application_held_points == application_held_points
            && (kept.geometry.iter().zip(geometry))
                .all(|(g, &i)| *g == problem.geometry[i as usize])
            && (kept.constraints.iter().zip(constraints))
                .all(|(c, &i)| *c == problem.constraints[i as usize])
    };
    if let Some(part) = parts.find(key, same) {
        return part;
    }
    let part = Arc::new(Part::compile(Problem {
        geometry: (geometry.iter())
            .map(|&i| problem.geometry[i as usize].clone())
            .collect(),
        constraints: (constraints.iter())
            .map(|&i| problem.constraints[i as usize].clone())
            .collect(),
        external,
        held_points,
        application_held_points,
        settings: problem.settings,
    }));
    parts.keep(key, Arc::clone(&part));
    part
}

/// A problem solved lately with its parts and how its elements joined
/// into them, so the next version of it is split by what changed.
struct KeptSplit {
    problem: Arc<Problem>,
    joins: Joins,
    /// The parts, by number; `None` for a number no longer in use.
    parts: Vec<Option<Arc<Part>>>,
}

impl KeptSplit {
    /// Whether `problem` may be this one's next version: the same first
    /// element, and no fewer elements or constraints.
    fn precedes(&self, problem: &Problem) -> bool {
        let (before, after) = (&self.problem, problem);
        before.geometry.len() <= after.geometry.len()
            && before.constraints.len() <= after.constraints.len()
            && before.settings == after.settings
            && before.geometry.first().map(|g| g.id()) == after.geometry.first().map(|g| g.id())
    }

    /// The parts of `problem`, this one's next version, and the split
    /// kept for it: each part whose elements and constraints are all as
    /// they were and which nothing added joins kept as it is. `None` when
    /// an element or constraint kept its place but not its id or what it
    /// names, or what was added cannot be split by what was kept.
    fn grow(
        mut self,
        problem: &Arc<Problem>,
        parts: &mut Generations,
    ) -> Option<(Self, Vec<Arc<Part>>)> {
        let before = Arc::clone(&self.problem);
        let (n0, m0) = (before.geometry.len(), before.constraints.len());
        let (n1, m1) = (problem.geometry.len(), problem.constraints.len());
        let joins = &mut self.joins;
        let slots = self.parts.len();
        let mut changed = vec![false; slots];
        for (i, (now, was)) in problem.geometry.iter().zip(&before.geometry).enumerate() {
            if now != was {
                if now.id() != was.id() || now.point_references() != was.point_references() {
                    return None;
                }
                changed[joins.of_geometry[i] as usize] = true;
            }
        }
        for (k, (now, was)) in (problem.constraints.iter().zip(&before.constraints)).enumerate() {
            if now != was {
                if now.id != was.id || now.kind.references() != was.kind.references() {
                    return None;
                }
                changed[joins.of_constraint[k] as usize] = true;
            }
        }
        if n1 > n0 && joins.dangling {
            return None;
        }

        // What was added, joined to the kept parts: nodes are the parts
        // by number, then the added elements.
        let node_of = |joins: &Joins, i: u32| {
            if (i as usize) < n0 {
                joins.of_geometry[i as usize]
            } else {
                (slots + i as usize - n0) as u32
            }
        };
        for (i, g) in problem.geometry.iter().enumerate().skip(n0) {
            if joins.places.insert(g.id(), i as u32).is_some() {
                return None;
            }
        }
        let mut parent: Vec<u32> = (0..(slots + n1 - n0) as u32).collect();
        for (i, g) in problem.geometry.iter().enumerate().skip(n0) {
            for p in g.point_references() {
                match joins.places.get(&p.into()) {
                    Some(&j) => join(&mut parent, node_of(joins, i as u32), node_of(joins, j)),
                    None => joins.dangling |= p != PointReference::Origin,
                }
            }
        }
        let mut anchors = Vec::with_capacity(m1 - m0);
        for (k, c) in problem.constraints.iter().enumerate().skip(m0) {
            if joins.constraint_at.insert(c.id, k as u32).is_some() {
                return None;
            }
            joins.grouped |= matches!(c.kind, Relation::Group { .. });
            let mut anchor = None;
            for r in c.kind.references() {
                match joins.places.get(&r) {
                    Some(&j) => {
                        let node = node_of(joins, j);
                        match anchor {
                            None => anchor = Some(node),
                            Some(a) => join(&mut parent, a, node),
                        }
                    }
                    None => joins.dangling |= !is_reference(r),
                }
            }
            anchors.push(anchor);
        }
        let loose = joins.loose.is_some() || anchors.iter().any(Option::is_none);
        if loose && joins.grouped {
            return None;
        }

        // Each node's part after the additions: a kept number for a part
        // nothing joined, a new one for a part made of several or of
        // added elements alone.
        let nodes = parent.len();
        let mut members = vec![0u32; nodes];
        for a in 0..nodes as u32 {
            if (a as usize) < slots && self.parts[a as usize].is_none() {
                continue;
            }
            members[root(&mut parent, a) as usize] += 1;
        }
        let mut renumbered = vec![u32::MAX; nodes];
        let mut joined: Vec<u32> = Vec::new();
        for a in 0..nodes as u32 {
            if (a as usize) < slots && self.parts[a as usize].is_none() {
                continue;
            }
            let r = root(&mut parent, a) as usize;
            if renumbered[r] != u32::MAX {
                continue;
            }
            let alone = (a as usize) < slots && members[r] == 1;
            renumbered[r] = if alone {
                a
            } else {
                self.parts.push(None);
                joined.push((self.parts.len() - 1) as u32);
                (self.parts.len() - 1) as u32
            };
        }
        let number = |parent: &mut Vec<u32>, node: u32| renumbered[root(parent, node) as usize];
        // The constraints added naming no element join the loose part.
        let loose_number = match joins.loose {
            Some(loose) if anchors.iter().all(Option::is_some) => Some(loose),
            Some(loose) => {
                changed[loose as usize] = true;
                Some(loose)
            }
            None if loose => {
                self.parts.push(None);
                joined.push((self.parts.len() - 1) as u32);
                Some((self.parts.len() - 1) as u32)
            }
            None => None,
        };

        // Move the elements and constraints of joined parts to their new
        // numbers, and add what was added.
        let mut relabelled: Vec<(u32, u32)> = Vec::new();
        for s in 0..slots as u32 {
            if self.parts[s as usize].is_none() {
                continue;
            }
            let to = number(&mut parent, s);
            if to != s {
                relabelled.push((s, to));
            }
        }
        let mut merged: std::collections::BTreeMap<u32, Vec<Arc<Part>>> = Default::default();
        for &(from, to) in &relabelled {
            let part = self.parts[from as usize].take().expect("a part in use");
            for g in &part.problem.geometry {
                let i = joins.places[&g.id()];
                joins.of_geometry[i as usize] = to;
            }
            for c in &part.problem.constraints {
                let k = joins.constraint_at[&c.id];
                joins.of_constraint[k as usize] = to;
            }
            merged.entry(to).or_default().push(part);
        }
        for i in n0..n1 {
            let to = number(&mut parent, (slots + i - n0) as u32);
            joins.of_geometry.push(to);
        }
        for anchor in &anchors {
            let to = match anchor {
                Some(node) => number(&mut parent, *node),
                None => loose_number.expect("a loose part"),
            };
            // A constraint added to a part nothing else joined changes it.
            if let Some(changed) = changed.get_mut(to as usize) {
                *changed = true;
            }
            joins.of_constraint.push(to);
        }
        joins.loose = loose_number;

        // Every part made or changed, compiled or taken from what is kept;
        // a kept part whose held points changed too.
        let mut held = if holds_any(problem) || holds_any(&before) {
            held_by_part(problem, |r| {
                (joins.places.get(r)).map(|&i| joins.of_geometry[i as usize])
            })
        } else {
            Default::default()
        };
        let empty = Held::default();
        let mut rebuild: Vec<u32> = joined;
        for (s, part) in self.parts.iter().enumerate().take(slots) {
            let Some(part) = part else { continue };
            let kept = &part.problem;
            let now = held.get(&(s as u32)).unwrap_or(&empty);
            if changed[s]
                || (
                    &kept.external,
                    &kept.held_points,
                    &kept.application_held_points,
                ) != (&now.0, &now.1, &now.2)
            {
                rebuild.push(s as u32);
            }
        }
        rebuild.sort_unstable();
        rebuild.dedup();
        // A part rebuilt holds what the parts it is made of held, and what
        // was added to it.
        let mut added: std::collections::BTreeMap<u32, (Vec<u32>, Vec<u32>)> = Default::default();
        for i in n0..n1 {
            added
                .entry(joins.of_geometry[i])
                .or_default()
                .0
                .push(i as u32);
        }
        for k in m0..m1 {
            added
                .entry(joins.of_constraint[k])
                .or_default()
                .1
                .push(k as u32);
        }
        for &s in &rebuild {
            let (mut geometry, mut constraints) = added.remove(&s).unwrap_or_default();
            let olds = merged.remove(&s).unwrap_or_default();
            for part in olds.iter().chain(self.parts[s as usize].as_ref()) {
                geometry.extend(part.problem.geometry.iter().map(|g| joins.places[&g.id()]));
                constraints
                    .extend((part.problem.constraints.iter()).map(|c| joins.constraint_at[&c.id]));
            }
            geometry.sort_unstable();
            constraints.sort_unstable();
            let held = held.remove(&s).unwrap_or_default();
            self.parts[s as usize] = Some(part_of(problem, &geometry, &constraints, held, parts));
        }
        self.problem = Arc::clone(problem);
        let list: Vec<Arc<Part>> = self.parts.iter().flatten().cloned().collect();
        Some((self, list))
    }
}

/// The parts and splits of the problems solved lately: each part by its
/// first element, so the next problem takes those it holds unchanged, and
/// the last few problems' splits, so their next versions are split by
/// what changed.
#[derive(Default)]
pub struct PartCache {
    parts: Generations,
    splits: Vec<KeptSplit>,
}

/// Compiled parts in two generations: the parts kept or taken since the
/// last turn, and the ones before, dropped at the next turn.
#[derive(Default)]
struct Generations {
    now: IdMap<ItemReference, Vec<Arc<Part>>>,
    before: IdMap<ItemReference, Vec<Arc<Part>>>,
    /// The elements the parts in `now` hold.
    held: usize,
}

/// The elements a generation of parts holds before it turns, unless one
/// problem holds more.
const GENERATION: usize = 1 << 16;

/// The versions of one part kept: a dragged part solved holding its
/// points and measured without them, the version before an edit.
const VERSIONS: usize = 4;

/// The splits kept: the problems solved lately, each its own sketch or
/// a draft of one.
const SPLITS: usize = 4;

impl PartCache {
    fn turn(&mut self, elements: usize) {
        self.parts.turn(elements);
    }

    /// The kept split `problem` may be the next version of, taken out.
    fn take_kept(&mut self, problem: &Problem) -> Option<KeptSplit> {
        let at = self.splits.iter().position(|k| k.precedes(problem))?;
        Some(self.splits.remove(at))
    }

    /// Keep `split` as the latest, unless numbers no longer in use
    /// outgrow the parts, when the next version is split afresh.
    fn keep_split(&mut self, split: KeptSplit) {
        let unused = split.parts.iter().filter(|p| p.is_none()).count();
        if unused > 64 && unused > split.parts.len() / 2 {
            return;
        }
        self.splits.insert(0, split);
        self.splits.truncate(SPLITS);
    }

    /// `problem`'s parts, split afresh, and its split kept.
    fn split(&mut self, problem: &Arc<Problem>) -> Vec<Arc<Part>> {
        let split = Split::of(problem);
        let mut held = match &split.kept {
            Some(joins) => held_by_part(problem, |r| {
                (joins.places.get(r)).map(|&i| joins.of_geometry[i as usize])
            }),
            None => std::iter::once((
                0,
                (
                    problem.external.clone(),
                    problem.held_points.clone(),
                    problem.application_held_points.clone(),
                ),
            ))
            .collect(),
        };
        let parts: Vec<Arc<Part>> = (0..split.count)
            .map(|k| {
                let held = held.remove(&(k as u32)).unwrap_or_default();
                part_of(
                    problem,
                    split.geometry(k),
                    split.constraints(k),
                    held,
                    &mut self.parts,
                )
            })
            .collect();
        if let Some(joins) = split.kept {
            self.keep_split(KeptSplit {
                problem: Arc::clone(problem),
                joins,
                parts: parts.iter().cloned().map(Some).collect(),
            });
        }
        parts
    }
}

impl Generations {
    /// The kept part `key` names that `same` takes, made one of this
    /// generation's.
    fn find(&mut self, key: ItemReference, same: impl Fn(&Part) -> bool) -> Option<Arc<Part>> {
        if let Some(part) = self.now.get(&key).and_then(|v| v.iter().find(|p| same(p))) {
            return Some(Arc::clone(part));
        }
        let versions = self.before.get_mut(&key)?;
        let at = versions.iter().position(|p| same(p))?;
        let part = versions.swap_remove(at);
        self.keep(key, Arc::clone(&part));
        Some(part)
    }

    fn keep(&mut self, key: ItemReference, part: Arc<Part>) {
        self.held += part.problem.geometry.len().max(1);
        let versions = self.now.entry(key).or_default();
        versions.insert(0, part);
        if versions.len() > VERSIONS
            && let Some(dropped) = versions.pop()
        {
            self.held -= dropped.problem.geometry.len().max(1);
        }
    }

    /// Start a new generation once this one holds more than a problem of
    /// `elements` needs.
    fn turn(&mut self, elements: usize) {
        if self.held > GENERATION.max(2 * elements) {
            self.before = std::mem::take(&mut self.now);
            self.held = 0;
        }
    }
}

#[path = "parts_diagnosis.rs"]
mod diagnosis;

#[cfg(test)]
#[path = "parts_tests.rs"]
mod tests;
