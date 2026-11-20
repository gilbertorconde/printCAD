//! Constraint health from recompilation and rank at a solved configuration.

use crate::compile::compile_system;
use crate::freedom::estimate;
use crate::problem::{ConstraintId, Geometry, Length, Problem, Vector};
use crate::solve::{SolveOutcome, iterate};
use crate::sparse::rank;
use crate::trace::EquationSource;

#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Diagnosis {
    pub dof: i32,
    pub redundant: Vec<ConstraintId>,
    pub conflicting: Vec<ConstraintId>,
    pub analyzed: bool,
}

pub fn analyze(problem: &Problem) -> Result<Diagnosis, crate::input::InputError> {
    crate::input::validate(problem)?;
    Ok(diagnose(problem))
}

/// The standalone path evaluates redundancy on double-precision geometry.
pub fn diagnose(problem: &Problem) -> Diagnosis {
    let system = compile_system(problem, None, false);
    let answer = iterate(&system, problem.settings);
    let mut solved = problem.clone();
    for g in &mut solved.geometry {
        match g {
            Geometry::Point(p) => {
                let k = system.point_vars[&p.id.into()];
                p.position = Vector {
                    x: answer.values[k],
                    y: answer.values[k + 1],
                };
            }
            Geometry::Circle(c) => {
                c.radius = Length(answer.values[system.radius_vars[&c.id.into()]])
            }
            Geometry::Arc(a) => a.radius = Length(answer.values[system.radius_vars[&a.id.into()]]),
            Geometry::Ellipse(e) => {
                if let Some(&k) = system.shape_vars.get(&e.id.into()) {
                    e.major = Vector {
                        x: answer.values[k],
                        y: answer.values[k + 1],
                    };
                    e.minor = Length(answer.values[k + 2]);
                }
            }
            Geometry::Conic(c) => {
                if let Some(&k) = system.shape_vars.get(&c.id.into()) {
                    c.axis = Vector {
                        x: answer.values[k],
                        y: answer.values[k + 1],
                    };
                    c.minor = Length(answer.values[k + 2]);
                }
            }
            Geometry::Line(_) | Geometry::BSpline(_) => {}
        }
    }
    diagnose_at(problem, answer.outcome, &solved)
}

/// Callers with storage rounding or derived geometry may provide the
/// configuration after their application step. An exclusion probe is the
/// system compiled once with the probed constraint's equations left out,
/// or compiled again without it where leaving it out changes more (see
/// `Local`); there are no application callbacks.
pub fn diagnose_at(problem: &Problem, outcome: SolveOutcome, solved: &Problem) -> Diagnosis {
    diagnose_measured(problem, outcome, solved, estimate(problem).degrees)
}

/// [`diagnose_at`] with the problem's degrees of freedom already measured
/// (`freedom::estimate(problem).degrees`), by a caller that keeps them.
pub fn diagnose_measured(
    problem: &Problem,
    outcome: SolveOutcome,
    solved: &Problem,
    dof: i32,
) -> Diagnosis {
    if problem.constraints.len() > problem.settings.diagnosis_limit {
        return Diagnosis {
            dof,
            analyzed: false,
            ..Diagnosis::default()
        };
    }
    let mut diagnosis = Diagnosis {
        dof,
        analyzed: true,
        ..Diagnosis::default()
    };
    match outcome {
        SolveOutcome::NotConverged { .. } => {
            for c in &problem.constraints {
                let mut without = problem.clone();
                without.constraints.retain(|other| other.id != c.id);
                let system = compile_system(&without, None, false);
                if matches!(
                    iterate(&system, problem.settings).outcome,
                    SolveOutcome::Converged { .. } | SolveOutcome::NothingToSolve
                ) {
                    diagnosis.conflicting.push(c.id);
                }
            }
        }
        SolveOutcome::Converged { .. } | SolveOutcome::NothingToSolve => {
            let full = compile_system(solved, None, false);
            if !full.specs.is_empty() && !full.vars.is_empty() {
                let full_rank = rank(&full, problem.settings.rank_tolerance);
                // Fewer rows than a rank as large as the row count cannot
                // keep that rank: no rank to work out for those.
                let row_rank = full_rank == full.residual_len;
                let local = Local::of(solved, &full);
                for c in &problem.constraints {
                    let system = if local.holds(c) {
                        let own = (full.origins.iter())
                            .filter(|o| **o == EquationSource::Constraint(c.id))
                            .count();
                        if own == 0 || row_rank {
                            continue;
                        }
                        without(&full, c.id)
                    } else {
                        compile_system(solved, Some(c.id), false)
                    };
                    if system.residual_len == full.residual_len
                        || (row_rank && system.residual_len < full.residual_len)
                    {
                        continue;
                    }
                    if rank(&system, problem.settings.rank_tolerance) == full_rank {
                        diagnosis.redundant.push(c.id);
                    }
                }
            }
        }
    }
    diagnosis
}

/// Which constraints a probe can leave out of the compiled system by its
/// rows alone: those whose absence changes nothing else the compilation
/// makes. A constraint does when a group's skipped relations depend on
/// it, when it gives an ellipse or conic its shape variables or a focus
/// its place, when a tangency finds its point of contact through it (a
/// point held on a circle), or when its equations are the only ones
/// constraints bring, which leaves the arcs' own equations out.
struct Local {
    /// No probe is local: the problem holds a group.
    grouped: bool,
    /// The problem holds a focus aligned to its curve.
    foci: bool,
    /// The problem holds a tangency.
    tangency: bool,
    /// The constraints the full system has equations from.
    with_rows: usize,
}

impl Local {
    fn of(problem: &Problem, full: &crate::compile::System) -> Self {
        use crate::problem::{InternalRole, Relation as R};
        let constrained: std::collections::HashSet<_> = full
            .origins
            .iter()
            .filter_map(|o| match o {
                EquationSource::Constraint(id) => Some(*id),
                _ => None,
            })
            .collect();
        let any = |f: &dyn Fn(&R) -> bool| problem.constraints.iter().any(|c| f(&c.kind));
        Self {
            grouped: any(&|k| matches!(k, R::Group { .. })),
            foci: any(&|k| {
                matches!(
                    k,
                    R::InternalAlignment {
                        role: InternalRole::Focus1 | InternalRole::Focus2,
                        ..
                    }
                )
            }),
            tangency: any(&|k| matches!(k, R::Tangent { .. })),
            with_rows: constrained.len(),
        }
    }

    fn holds(&self, c: &crate::problem::Constraint) -> bool {
        use crate::problem::{ItemReference, Relation as R};
        let structural = matches!(
            c.kind,
            R::Group { .. }
                | R::InternalAlignment { .. }
                | R::EqualEllipse { .. }
                | R::EllipseRadius { .. }
        ) || (self.tangency && matches!(c.kind, R::PointOnCircle { .. }))
            || (self.foci
                && (c.kind.references().iter()).any(|id| matches!(id, ItemReference::Point(_))));
        !self.grouped && !structural && self.with_rows > 1
    }
}

/// `full` without the equations constraint `id` brings, for its rank: the
/// same variables (any `id` alone read stay, read by nothing), and no
/// maps of where each element's variables are.
fn without(
    full: &crate::compile::System,
    id: crate::problem::ConstraintId,
) -> crate::compile::System {
    let mut specs = Vec::with_capacity(full.specs.len());
    let mut origins = Vec::with_capacity(full.origins.len());
    let mut row = 0;
    for spec in &full.specs {
        let dim = spec.dim();
        if full.origins[row] != EquationSource::Constraint(id) {
            specs.push(spec.clone());
            origins.extend_from_slice(&full.origins[row..row + dim]);
        }
        row += dim;
    }
    crate::compile::System {
        residual_len: origins.len(),
        origins,
        settings: full.settings,
        vars: full.vars.clone(),
        free: full.free.clone(),
        point_vars: Default::default(),
        radius_vars: Default::default(),
        shape_vars: Default::default(),
        derived_foci: Default::default(),
        specs,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::problem::{
        Circle, Constraint, ConstraintId, CurveId, CurveReference, Line, Point, PointId,
        PointReference, Relation as R,
    };

    /// A fixed-seed generator, so every run draws the same sketches.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn chance(&mut self, percent: u64) -> bool {
            self.next() % 100 < percent
        }
    }

    /// Cells of a rectangle and a circle each, with relations drawn at
    /// random from ones that hold as drawn, so the sketch solves where it
    /// stands and often says a thing twice: a rectangle's sides held
    /// horizontal, vertical, parallel, of equal length and to their
    /// length, a corner on the origin, the circle sized, tangent to a side
    /// at a corner it holds, or a corner's distance to another.
    fn drawn(seed: u64) -> Problem {
        let mut rng = Rng(seed);
        let mut problem = Problem::default();
        let mut next = 1u128;
        let mut id = || {
            next += 1;
            next
        };
        let mut relations = Vec::new();
        for cell in 0..3 {
            let (x, y) = (cell as f64 * 10.0, 0.0);
            let (w, h) = (4.0, 3.0);
            let corners: Vec<PointReference> = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)]
                .into_iter()
                .map(|(px, py)| {
                    let p = PointId(id());
                    problem.geometry.push(Geometry::Point(Point {
                        id: p,
                        position: Vector { x: px, y: py },
                    }));
                    p.into()
                })
                .collect();
            let sides: Vec<CurveReference> = (0..4)
                .map(|k| {
                    let l = CurveId(id());
                    problem.geometry.push(Geometry::Line(Line {
                        id: l,
                        start: corners[k],
                        end: corners[(k + 1) % 4],
                    }));
                    l.into()
                })
                .collect();
            let centre = PointId(id());
            problem.geometry.push(Geometry::Point(Point {
                id: centre,
                position: Vector {
                    x: x + w + 2.0,
                    y: y + h,
                },
            }));
            let circle = CurveId(id());
            problem.geometry.push(Geometry::Circle(Circle {
                id: circle,
                center: centre.into(),
                radius: Length(2.0),
            }));
            let circle: CurveReference = circle.into();
            let pool = [
                R::Horizontal { element: sides[0] },
                R::Horizontal { element: sides[2] },
                R::Vertical { element: sides[1] },
                R::Vertical { element: sides[3] },
                R::Parallel {
                    line1: sides[0],
                    line2: sides[2],
                },
                R::Perpendicular {
                    line1: sides[0],
                    line2: sides[1],
                },
                R::EqualLength {
                    line1: sides[0],
                    line2: sides[2],
                },
                R::Length {
                    line: sides[0],
                    length: Length(w),
                },
                R::Length {
                    line: sides[2],
                    length: Length(w),
                },
                R::Radius {
                    circle,
                    radius: Length(2.0),
                },
                R::Distance {
                    point1: corners[0],
                    point2: corners[1],
                    distance: Length(w),
                },
                R::FixedPoint {
                    point: corners[0],
                    position: Vector { x, y },
                },
                R::PointOnCircle {
                    point: corners[2],
                    circle,
                },
                R::Tangent {
                    line_or_circle1: sides[1],
                    item2: circle,
                },
                R::Coincident {
                    point1: PointReference::Origin,
                    point2: corners[0],
                },
                R::PointOnLine {
                    point: corners[1],
                    line: CurveReference::XAxis,
                },
            ];
            for relation in pool {
                if rng.chance(45) {
                    relations.push(relation);
                }
            }
        }
        for kind in relations {
            problem.constraints.push(Constraint {
                id: ConstraintId(id()),
                kind,
            });
        }
        problem
    }

    /// The redundant constraints found by compiling the sketch again
    /// without each one.
    fn recompiled(problem: &Problem, solved: &Problem) -> Vec<ConstraintId> {
        let full = compile_system(solved, None, false);
        let full_rank = rank(&full, problem.settings.rank_tolerance);
        let row_rank = full_rank == full.residual_len;
        let mut redundant = Vec::new();
        for c in &problem.constraints {
            let system = compile_system(solved, Some(c.id), false);
            if system.residual_len == full.residual_len
                || (row_rank && system.residual_len < full.residual_len)
            {
                continue;
            }
            if rank(&system, problem.settings.rank_tolerance) == full_rank {
                redundant.push(c.id);
            }
        }
        redundant
    }

    /// Probes that leave a constraint's rows out of the one compiled
    /// system find what compiling without each constraint finds, on
    /// sketches that say things twice, reach the origin and the axes, and
    /// hold tangencies through a point on the circle.
    #[test]
    fn probes_by_rows_find_what_recompiling_finds() {
        let mut redundant_sketches = 0;
        for seed in 1..=120 {
            let problem = drawn(seed * 7919);
            let outcome =
                crate::solve::iterate(&compile_system(&problem, None, false), problem.settings)
                    .outcome;
            if matches!(outcome, SolveOutcome::NotConverged { .. }) {
                continue;
            }
            let diagnosis = diagnose_at(&problem, outcome, &problem);
            let expected = recompiled(&problem, &problem);
            assert_eq!(diagnosis.redundant, expected, "seed {seed}");
            redundant_sketches += usize::from(!expected.is_empty());
        }
        assert!(redundant_sketches > 60, "{redundant_sketches}");
    }
}
