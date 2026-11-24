//! A diagnosis by parts: what [`crate::diagnosis::diagnose_compiled`]
//! finds on the whole problem, read from the parts a solve keeps.
//!
//! - Conflicts: a constraint conflicts when the problem solves without
//!   it. Leaving out a constraint changes only its own part's equations,
//!   so the parts that cannot be solved stay so whatever is left out
//!   elsewhere: with two of them nothing conflicts, and with one only its
//!   constraints are probed. Each probe solves every part together, the
//!   probed one compiled without the constraint, as the whole problem
//!   compiled without it is solved.
//! - Redundancy: the whole rank is the sum of the parts' ranks at the
//!   whole Jacobian's pivot threshold, so a probe ranks only its own part
//!   without the constraint, and the others again only where leaving it
//!   out moves the threshold.
//!
//! Where these do not carry (no part brings an equation, the solve the
//! parts give disagrees with the outcome), it answers `None`, for the
//! whole problem's diagnosis.

use super::Parted;
use crate::compile::{compile_counting, compile_system};
use crate::diagnosis::{Diagnosis, Local, with_rows, without};
use crate::problem::{ConstraintId, IdHash, IdMap, Problem};
use crate::solve::{BlocksOf, Piece, Rest, SolveOutcome, iterate_pieces};
use crate::sparse::{Blocks, Differenced, rank, scale_of};
use crate::trace::EquationSource;

impl Parted {
    /// [`crate::diagnosis::diagnose_measured`] of `problem`, these its
    /// parts, from `solved`, the problem at the values its solve left,
    /// and `solved_parts`, its parts; `None` where only the whole problem
    /// answers it.
    pub fn diagnose(
        &self,
        problem: &Problem,
        outcome: SolveOutcome,
        solved: &Problem,
        solved_parts: &Parted,
        dof: i32,
    ) -> Option<Diagnosis> {
        let mut diagnosis = Diagnosis {
            dof,
            analyzed: true,
            ..Diagnosis::default()
        };
        if problem.constraints.len() > problem.settings.diagnosis_limit {
            diagnosis.analyzed = false;
            return Some(diagnosis);
        }
        match outcome {
            SolveOutcome::NotConverged { .. } => {
                diagnosis.conflicting = self.conflicting(problem)?;
            }
            SolveOutcome::Converged { .. } | SolveOutcome::NothingToSolve => {
                diagnosis.redundant = solved_parts.redundant(problem, solved)?;
            }
        }
        Some(diagnosis)
    }

    /// The constraints of `problem` it solves without, these its parts.
    fn conflicting(&self, problem: &Problem) -> Option<Vec<ConstraintId>> {
        let settings = problem.settings;
        let constrained = self.constrained;
        let pieces: Vec<Piece> = (self.parts.iter())
            .map(|p| p.plain(constrained).piece())
            .collect();
        let run = iterate_pieces(&pieces, settings);
        if !matches!(run.outcome, SolveOutcome::NotConverged { .. }) {
            return None;
        }
        let mut unmet = (run.unmet.iter().enumerate()).filter_map(|(i, &u)| u.then_some(i));
        let f = unmet.next()?;
        if unmet.next().is_some() {
            return Some(Vec::new());
        }
        let part = &self.parts[f];
        let full = &part.plain(constrained).sys;
        let others_constrained =
            (self.parts.iter().enumerate()).any(|(i, p)| i != f && p.constrained);
        let local = Local::counted(problem, self.rows(Some(f), with_rows(full)));

        let mut conflicting = Vec::new();
        for c in &part.problem.constraints {
            let (sys, after) = if local.holds(c) {
                let mut system = without(full, c.id);
                system.point_vars.clone_from(&full.point_vars);
                system.radius_vars.clone_from(&full.radius_vars);
                system.shape_vars.clone_from(&full.shape_vars);
                system.derived_foci.clone_from(&full.derived_foci);
                (system, constrained)
            } else {
                let mut without = part.problem.clone();
                without.constraints.retain(|other| other.id != c.id);
                let counted = compile_counting(&without, None, others_constrained);
                (counted.system, others_constrained || counted.constrained)
            };
            let blocks = std::sync::OnceLock::new();
            let rest = Rest::of(&sys);
            let pieces: Vec<Piece> = (self.parts.iter().enumerate())
                .map(|(i, p)| match i == f {
                    true => Piece {
                        sys: &sys,
                        blocks: BlocksOf::Lazy(&blocks),
                        rest: &rest,
                    },
                    false => p.plain(after).piece(),
                })
                .collect();
            if matches!(
                iterate_pieces(&pieces, settings).outcome,
                SolveOutcome::Converged { .. } | SolveOutcome::NothingToSolve
            ) {
                conflicting.push(c.id);
            }
        }
        Some(conflicting)
    }

    /// How many constraints bring equations to the whole problem, counted
    /// as far as two, which is all [`Local`] reads: `given` of them in the
    /// part `skip`, the others counted.
    fn rows(&self, skip: Option<usize>, given: usize) -> usize {
        let mut rows = given;
        for (i, part) in self.parts.iter().enumerate() {
            if rows > 1 {
                break;
            }
            if Some(i) != skip && part.constrained {
                rows += with_rows(&part.rank.sys);
            }
        }
        rows
    }

    /// The constraints of `problem` whose equations add nothing to the
    /// rank of `solved`, these its parts.
    fn redundant(&self, problem: &Problem, solved: &Problem) -> Option<Vec<ConstraintId>> {
        // Some part brings an equation, so each part's system for rank is
        // the one solved, and the whole system's.
        if !self.constrained {
            return None;
        }
        let systems = || self.parts.iter().map(|p| &p.rank);
        if systems().all(|c| c.sys.specs.is_empty()) || systems().all(|c| c.sys.vars.is_empty()) {
            return Some(Vec::new());
        }
        let rank_tolerance = problem.settings.rank_tolerance;
        // The two largest entries, so the largest without any one part is
        // at hand.
        let (mut first, mut second) = ((0.0_f64, usize::MAX), 0.0_f64);
        for (i, c) in systems().enumerate() {
            let largest = c.differenced().largest;
            if largest > first.0 {
                second = first.0;
                first = (largest, i);
            } else {
                second = second.max(largest);
            }
        }
        let tol = scale_of(first.0).map(|m| m * rank_tolerance);
        let full_rank: usize = tol.map_or(0, |tol| systems().map(|c| c.rank_at(tol)).sum());
        let residual_len: usize = systems().map(|c| c.sys.residual_len).sum();
        let row_rank = full_rank == residual_len;
        let local = Local::counted(solved, self.rows(None, 0));
        let constrained_parts = self.parts.iter().filter(|p| p.constrained).count();

        let mut at: Option<IdMap<ConstraintId, usize>> = None;
        let mut redundant = Vec::new();
        for c in &problem.constraints {
            let holds = local.holds(c);
            if holds && row_rank {
                continue;
            }
            let at = at.get_or_insert_with(|| self.constraint_parts());
            let &k = at.get(&c.id)?;
            let part = &self.parts[k];
            let own = &part.rank.sys;
            let sys = if holds {
                let rows = (own.origins.iter())
                    .filter(|o| **o == EquationSource::Constraint(c.id))
                    .count();
                if rows == 0 {
                    continue;
                }
                without(own, c.id)
            } else {
                let others_constrained = constrained_parts > usize::from(part.constrained);
                let counted = compile_counting(&part.problem, Some(c.id), others_constrained);
                if !others_constrained && !counted.constrained {
                    // Leaving it out takes every arc's own equations out
                    // of every part: the whole problem answers.
                    let system = compile_system(solved, Some(c.id), false);
                    if system.residual_len != residual_len
                        && !(row_rank && system.residual_len < residual_len)
                        && rank(&system, rank_tolerance) == full_rank
                    {
                        redundant.push(c.id);
                    }
                    continue;
                }
                counted.system
            };
            if sys.residual_len == own.residual_len
                || (row_rank && sys.residual_len < own.residual_len)
            {
                continue;
            }
            let differenced = Differenced::of(&sys, &Blocks::of(&sys));
            let others = if first.1 == k { second } else { first.0 };
            let after = match scale_of(others.max(differenced.largest)) {
                None => 0,
                Some(m) => {
                    let at = m * rank_tolerance;
                    let others = match tol {
                        Some(tol) if tol.to_bits() == at.to_bits() => {
                            full_rank - part.rank.rank_at(tol)
                        }
                        _ => (systems().enumerate())
                            .filter(|&(i, _)| i != k)
                            .map(|(_, c)| c.rank_at(at))
                            .sum(),
                    };
                    others + differenced.rank(at)
                }
            };
            if after == full_rank {
                redundant.push(c.id);
            }
        }
        Some(redundant)
    }

    /// Each constraint's part.
    fn constraint_parts(&self) -> IdMap<ConstraintId, usize> {
        let count = self.parts.iter().map(|p| p.problem.constraints.len()).sum();
        let mut at = IdMap::with_capacity_and_hasher(count, IdHash);
        for (k, part) in self.parts.iter().enumerate() {
            for c in &part.problem.constraints {
                at.insert(c.id, k);
            }
        }
        at
    }
}
