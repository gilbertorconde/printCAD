//! The Jacobian by blocks. Each residual reads a few variables, so its
//! rows are zero in every other column; and a sketch is mostly clusters
//! of geometry no equation joins, so the Jacobian is block diagonal once
//! its columns are grouped by the equations linking them. A block is the
//! equations and the movable variables one cluster holds: its columns are
//! differenced by evaluating its own equations alone, and the normal
//! equations, the step, the rank and which variables are determined are
//! all worked out block by block. Each block stays a dense matrix: a
//! cluster is small, and a large one is no worse off than the whole
//! system was.
//!
//! The differences are the ones the whole system's would be (the same
//! equation at the same probe), and a block's elimination sees the same
//! pivots: the other blocks' rows are zero in its columns.

use crate::compile::System;

/// A dense matrix, row by row.
pub type Matrix = Vec<Vec<f64>>;

/// The blocks of a system's Jacobian. Columns are positions in
/// `System::free`; a movable variable no equation reads is in none.
#[derive(Debug, Clone, Default)]
pub struct Blocks {
    pub blocks: Vec<Block>,
}

/// One cluster: its columns and rows (ascending, so a block's elimination
/// takes them in the order the whole matrix's would), and its equations.
#[derive(Debug, Clone, Default)]
pub struct Block {
    pub columns: Vec<usize>,
    pub rows: Vec<usize>,
    equations: Vec<Equation>,
}

/// An equation in a block: its spec, its first row in the block, and each
/// movable variable it reads with the block's column for it.
#[derive(Debug, Clone)]
struct Equation {
    spec: usize,
    row: usize,
    reads: Vec<(usize, usize)>,
}

fn root(parent: &mut [usize], mut a: usize) -> usize {
    while parent[a] != a {
        parent[a] = parent[parent[a]];
        a = parent[a];
    }
    a
}

impl Blocks {
    /// The blocks of `sys`'s Jacobian.
    pub fn of(sys: &System) -> Self {
        let mut column = vec![usize::MAX; sys.vars.len()];
        for (at, &j) in sys.free.iter().enumerate() {
            column[j] = at;
        }
        let mut parent: Vec<usize> = (0..sys.free.len()).collect();
        let mut reads = Vec::new();
        let mut first_row = 0;
        // Each spec's first row and the movable variables it reads, once
        // each, in the order it names them.
        let mut specs: Vec<(usize, Vec<usize>)> = Vec::with_capacity(sys.specs.len());
        for spec in &sys.specs {
            reads.clear();
            spec.reads(&mut reads);
            let mut vars: Vec<usize> = Vec::with_capacity(reads.len());
            for &j in &reads {
                if column.get(j).is_some_and(|&c| c != usize::MAX) && !vars.contains(&j) {
                    vars.push(j);
                }
            }
            if let Some((&first, rest)) = vars.split_first() {
                let mut a = root(&mut parent, column[first]);
                for &j in rest {
                    let b = root(&mut parent, column[j]);
                    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                    parent[hi] = lo;
                    a = lo;
                }
            }
            specs.push((first_row, vars));
            first_row += spec.dim();
        }
        // Blocks numbered by their lowest column.
        let mut index = vec![usize::MAX; sys.free.len()];
        let mut blocks: Vec<Block> = Vec::new();
        let mut used = vec![false; sys.free.len()];
        for (_, vars) in &specs {
            for &j in vars {
                used[column[j]] = true;
            }
        }
        for c in 0..sys.free.len() {
            if !used[c] {
                continue;
            }
            let r = root(&mut parent, c);
            if index[r] == usize::MAX {
                index[r] = blocks.len();
                blocks.push(Block::default());
            }
            let block = &mut blocks[index[r]];
            index[c] = index[r];
            block.columns.push(c);
        }
        for (spec, (row, vars)) in specs.into_iter().enumerate() {
            let Some(&first) = vars.first() else {
                continue;
            };
            let block = &mut blocks[index[root(&mut parent, column[first])]];
            let local_row = block.rows.len();
            let dim = sys.specs[spec].dim();
            block.rows.extend(row..row + dim);
            let reads = vars
                .into_iter()
                .map(|j| {
                    let local = block
                        .columns
                        .binary_search(&column[j])
                        .expect("a variable an equation reads is in its block");
                    (j, local)
                })
                .collect();
            block.equations.push(Equation {
                spec,
                row: local_row,
                reads,
            });
        }
        Blocks { blocks }
    }

    /// Each block's central-difference Jacobian at `x`: a column is
    /// differenced by evaluating only the equations that read it.
    pub fn jacobian(&self, sys: &System, x: &[f64]) -> Vec<Matrix> {
        let mut probe = x.to_vec();
        (self.blocks.iter())
            .map(|block| block.jacobian(sys, x, &mut probe))
            .collect()
    }

    /// The whole `rows` x `columns` matrix the blocks `jac` make up.
    pub fn dense(&self, jac: &[Matrix], rows: usize, columns: usize) -> Matrix {
        let mut out = vec![vec![0.0; columns]; rows];
        for (block, m) in self.blocks.iter().zip(jac) {
            for (&row, values) in block.rows.iter().zip(m) {
                for (&column, &v) in block.columns.iter().zip(values) {
                    out[row][column] = v;
                }
            }
        }
        out
    }
}

impl Block {
    /// The block's central-difference Jacobian at `x`, its columns
    /// differenced by evaluating only the equations that read them.
    /// `probe` holds `x` on entry and is left so.
    pub(crate) fn jacobian(&self, sys: &System, x: &[f64], probe: &mut [f64]) -> Matrix {
        let (mut plus, mut minus) = (Vec::new(), Vec::new());
        let minimum = sys.settings.minimum_length.0;
        let mut m = vec![vec![0.0; self.columns.len()]; self.rows.len()];
        for eq in &self.equations {
            let spec = &sys.specs[eq.spec];
            for &(j, column) in &eq.reads {
                let eps = sys.settings.finite_difference_step * x[j].abs().max(1.0);
                let original = probe[j];
                probe[j] = original + eps;
                plus.clear();
                spec.eval_with_minimum(probe, &mut plus, minimum);
                probe[j] = original - eps;
                minus.clear();
                spec.eval_with_minimum(probe, &mut minus, minimum);
                probe[j] = original;
                for (k, (rp, rm)) in plus.iter().zip(&minus).enumerate() {
                    m[eq.row + k][column] = (rp - rm) / (2.0 * eps);
                }
            }
        }
        m
    }

    /// The block's residuals at `x`, in its rows' order.
    pub(crate) fn residuals(&self, sys: &System, x: &[f64], out: &mut Vec<f64>) {
        out.clear();
        let minimum = sys.settings.minimum_length.0;
        for eq in &self.equations {
            sys.specs[eq.spec].eval_with_minimum(x, out, minimum);
        }
    }
}

/// A system's Jacobian by blocks at its values (with stacked points held
/// apart parted, see [`crate::solve::part_coincident_ends`]), and its
/// largest magnitude: what its rank and its determined variables are
/// read from, against a threshold that may come from a larger whole.
#[derive(Debug, Clone)]
pub(crate) struct Differenced {
    pub jac: Vec<Matrix>,
    /// The largest magnitude in `jac`, zero for none.
    pub largest: f64,
}

impl Differenced {
    pub(crate) fn of(sys: &System, blocks: &Blocks) -> Self {
        let jac = blocks.jacobian(sys, &crate::solve::part_coincident_ends(sys));
        let largest = jac
            .iter()
            .flatten()
            .flatten()
            .fold(0.0_f64, |acc, v| acc.max(v.abs()));
        Self { jac, largest }
    }

    /// The rank of the blocks, every pivot at or below `tol` taken for
    /// zero.
    pub(crate) fn rank(&self, tol: f64) -> usize {
        (self.jac.iter())
            .map(|m| crate::solve::rank_within(m.clone(), tol))
            .sum()
    }

    /// For each movable variable (in `free`'s order), whether it is
    /// determined, pivots held to `tol`; a variable no equation reads
    /// never is.
    pub(crate) fn determined(
        &self,
        blocks: &Blocks,
        free: usize,
        tol: f64,
        rank_tolerance: f64,
    ) -> Vec<bool> {
        let mut out = vec![false; free];
        for (block, m) in blocks.blocks.iter().zip(&self.jac) {
            let held = crate::freedom::determined_within(m.clone(), tol, rank_tolerance);
            for (&column, held) in block.columns.iter().zip(held) {
                out[column] = held;
            }
        }
        out
    }
}

/// The pivot scale of a Jacobian whose largest magnitude is `largest`,
/// or `None` when it is zero or not finite (no rank to read).
pub(crate) fn scale_of(largest: f64) -> Option<f64> {
    (largest != 0.0 && largest.is_finite()).then_some(largest)
}

/// The numerical rank of `sys`'s Jacobian at its values (with stacked
/// points held apart parted, see [`crate::solve::part_coincident_ends`]):
/// the sum of its blocks' ranks, every pivot held to the whole matrix's
/// threshold.
pub fn rank(sys: &System, rank_tolerance: f64) -> usize {
    let differenced = Differenced::of(sys, &Blocks::of(sys));
    let Some(max_abs) = scale_of(differenced.largest) else {
        return 0;
    };
    differenced.rank(max_abs * rank_tolerance)
}

/// For each movable variable of `sys` (in `free`'s order), whether no
/// direction its equations leave free moves it, block by block; a
/// variable no equation reads never is.
pub fn determined(sys: &System, rank_tolerance: f64) -> Vec<bool> {
    let blocks = Blocks::of(sys);
    let differenced = Differenced::of(sys, &blocks);
    let Some(max_abs) = scale_of(differenced.largest) else {
        return vec![false; sys.free.len()];
    };
    differenced.determined(
        &blocks,
        sys.free.len(),
        max_abs * rank_tolerance,
        rank_tolerance,
    )
}

#[cfg(test)]
mod tests {
    use super::{Blocks, Matrix, determined, rank};
    use crate::compile::System;
    use crate::curves::CurveVars;
    use crate::problem::Settings;
    use crate::residual::{CurveShape, Offset, ResidualSpec as R, Tangent};
    use crate::solve::{eval_residuals, jacobian, jacobian_rank, part_coincident_ends};

    /// A fixed-seed generator, so every run draws the same systems.
    struct Rng(u64);

    impl Rng {
        fn unit(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
        fn within(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (hi - lo) * self.unit()
        }
        fn below(&mut self, n: usize) -> usize {
            (self.unit() * n as f64) as usize % n
        }
        /// `n` distinct points' x variables out of `points` from `first`.
        fn points(&mut self, first: usize, points: usize, n: usize) -> Vec<usize> {
            let mut out: Vec<usize> = Vec::new();
            while out.len() < n {
                let p = first + 2 * self.below(points);
                if !out.contains(&p) {
                    out.push(p);
                }
            }
            out
        }
    }

    /// The whole system differenced one column at a time over every
    /// equation, as a dense Jacobian is by definition.
    fn reference(sys: &System, x: &[f64]) -> Matrix {
        let mut jac = vec![vec![0.0; sys.free.len()]; sys.residual_len];
        let mut probe = x.to_vec();
        for (column, &j) in sys.free.iter().enumerate() {
            let eps = sys.settings.finite_difference_step * x[j].abs().max(1.0);
            probe[j] = x[j] + eps;
            let plus = eval_residuals(sys, &probe);
            probe[j] = x[j] - eps;
            let minus = eval_residuals(sys, &probe);
            probe[j] = x[j];
            for (row, (p, m)) in jac.iter_mut().zip(plus.iter().zip(&minus)) {
                row[column] = (p - m) / (2.0 * eps);
            }
        }
        jac
    }

    fn system(vars: Vec<f64>, free: Vec<usize>, specs: Vec<R>) -> System {
        System {
            origins: Vec::new(),
            settings: Settings::default(),
            residual_len: specs.iter().map(R::dim).sum(),
            vars,
            free,
            point_vars: Default::default(),
            radius_vars: Default::default(),
            shape_vars: Default::default(),
            derived_foci: Default::default(),
            specs,
        }
    }

    const POINTS: usize = 12;
    const RADII: usize = 2 * POINTS;
    const SHAPES: usize = RADII + 3;
    const FRAME: usize = SHAPES + 6;
    const PARAMS: usize = FRAME + 3;
    const VARS: usize = PARAMS + 2;

    fn curve(rng: &mut Rng, which: usize) -> CurveVars {
        let p = rng.points(0, POINTS, 4);
        let shape = CurveShape::Var(SHAPES + 3 * rng.below(2));
        match which {
            0 => CurveVars::Line { s: p[0], e: p[1] },
            1 => CurveVars::Circle {
                c: p[0],
                r: RADII + rng.below(3),
            },
            2 => CurveVars::Ellipse { c: p[0], shape },
            3 => CurveVars::Hyperbola { c: p[0], shape },
            4 => CurveVars::Parabola {
                c: p[0],
                shape: CurveShape::Fixed {
                    x: 1.5,
                    y: 0.5,
                    minor: 0.0,
                },
            },
            _ => CurveVars::Spline {
                basis: crate::spline::Basis::new(2, 4, &[], false).expect("a basis"),
                control: p,
            },
        }
    }

    fn tangent(rng: &mut Rng) -> Tangent {
        let p = rng.points(0, POINTS, 2);
        if rng.below(2) == 0 {
            Tangent::Line { s: p[0], e: p[1] }
        } else {
            Tangent::Circle { c: p[0] }
        }
    }

    /// One equation of every kind over twelve points, three radii, two
    /// curve shapes, a group's frame and two curve parameters.
    fn every_kind(rng: &mut Rng) -> Vec<R> {
        let mut specs = Vec::new();
        for which in 0..6 {
            let p = rng.points(0, POINTS, 1)[0];
            specs.push(R::PointOnCurve {
                p,
                curve: curve(rng, which),
                t: PARAMS,
            });
            specs.push(R::CurveLength {
                curve: curve(rng, which),
                ends: None,
                length: 3.0,
            });
            specs.push(R::CurvesMeet {
                a: curve(rng, which),
                ta: PARAMS,
                b: curve(rng, (which + 1) % 6),
                tb: PARAMS + 1,
                square: which % 2 == 0,
            });
        }
        let ends = rng.points(0, POINTS, 2);
        specs.push(R::CurveLength {
            curve: curve(rng, 3),
            ends: Some((ends[0], ends[1])),
            length: 2.0,
        });
        let p = rng.points(0, POINTS, 5);
        let (a, b, c, d, e) = (p[0], p[1], p[2], p[3], p[4]);
        let (s1, s2) = (SHAPES, SHAPES + 3);
        let mut r = || RADII + rng.below(3);
        let radii: Vec<usize> = (0..8).map(|_| r()).collect();
        let shapes: Vec<CurveShape> = (0..5)
            .map(|_| CurveShape::Var(SHAPES + 3 * rng.below(2)))
            .collect();
        let flips: Vec<bool> = (0..5).map(|_| rng.below(2) == 0).collect();
        let (t1, t2, t3) = (tangent(rng), tangent(rng), tangent(rng));
        let at = [
            Offset::Major(1.0),
            Offset::Minor(-1.0),
            Offset::HyperbolaFocus,
        ][rng.below(3)];
        specs.extend([
            R::EllipseRadius {
                k: s1,
                major: true,
                radius: 2.0,
            },
            R::EllipseRadius {
                k: s2,
                major: false,
                radius: 1.0,
            },
            R::FixedPoint {
                p: a,
                x: 1.0,
                y: 2.0,
            },
            R::Coincident { p1: a, p2: b },
            R::SplineFitPoint {
                p: a,
                control: vec![(b, 0.25), (c, 0.5), (d, 0.25)],
            },
            R::Horizontal { s: a, e: b },
            R::Vertical { s: c, e: d },
            R::Length {
                s: a,
                e: b,
                len: 2.0,
            },
            R::Distance {
                p1: c,
                p2: d,
                d: 3.0,
            },
            R::Radius {
                r: radii[0],
                radius: 1.0,
            },
            R::Diameter {
                r: radii[1],
                diameter: 2.0,
            },
            R::CoordDistance {
                a,
                b: Some(b + 1),
                value: 1.5,
            },
            R::CoordDistance {
                a: c,
                b: None,
                value: -0.5,
            },
            R::EqualRadius {
                r1: RADII,
                r2: RADII + 1,
            },
            R::EqualLength {
                s1: a,
                e1: b,
                s2: c,
                e2: d,
            },
            R::Parallel {
                s1: a,
                e1: b,
                s2: c,
                e2: d,
            },
            R::Perpendicular {
                s1: a,
                e1: c,
                s2: b,
                e2: d,
            },
            R::PointOnLine { p: a, s: b, e: c },
            R::PointOnCircle {
                p: a,
                c: b,
                r: radii[2],
            },
            R::Angle {
                s1: a,
                e1: b,
                s2: c,
                e2: d,
                angle: 0.7,
            },
            R::AngleToTarget {
                s: a,
                e: b,
                target: 0.3,
            },
            R::PointOnEllipse {
                p: a,
                c: b,
                shape: shapes[0],
            },
            R::ArcEndpoint {
                p: c,
                c: d,
                r: radii[3],
            },
            R::TangentAtEnd {
                p: a,
                c: b,
                s: c,
                e: d,
                side: 1.0,
            },
            R::TangentArcsAtEnd { p: a, c1: b, c2: c },
            R::TangentLineCircle {
                s: a,
                e: b,
                c,
                r: radii[4],
                side: -1.0,
            },
            R::TangentCircles {
                c1: a,
                r1: RADII,
                c2: b,
                r2: RADII + 2,
                internal: flips[0],
            },
            R::Symmetric {
                p1: a,
                p2: b,
                s: c,
                e: d,
            },
            R::Midpoint { p: a, s: b, e: c },
            R::ArcLength {
                c: a,
                s: b,
                e: c,
                r: radii[5],
                len: 4.0,
            },
            R::EqualEllipse { a: s1, b: s2 },
            R::ArcAngle {
                c: a,
                s: b,
                e: c,
                angle: 1.0,
            },
            R::AngleThreePoints {
                a,
                v: b,
                b: c,
                angle: 2.0,
            },
            R::GapPointLine {
                p: a,
                s: b,
                e: c,
                side: 1.0,
                d: 0.5,
            },
            R::GapPointCircle {
                p: a,
                c: b,
                r: radii[6],
                inside: flips[1],
                d: 0.2,
            },
            R::GapLines {
                s1: a,
                e1: b,
                s2: c,
                e2: d,
                d: 1.0,
            },
            R::Step {
                a0: a,
                a1: b,
                dx: 1.0,
                dy: -1.0,
            },
            R::SameStep {
                a0: a,
                a1: b,
                b0: c,
                b1: d,
            },
            R::GapLineCircle {
                s: a,
                e: b,
                c,
                r: radii[7],
                side: 1.0,
                d: 0.3,
            },
            R::GapCircles {
                c1: a,
                r1: RADII,
                c2: b,
                r2: RADII + 1,
                nested: flips[2],
                d: 0.1,
            },
            R::AngleAtPoint {
                t1,
                t2,
                p: e,
                angle: 0.4,
            },
            R::Refraction {
                near1: a,
                far1: b,
                near2: c,
                far2: d,
                interface: t3,
                p: e,
                ratio: 1.3,
            },
            R::OnConic {
                p: a,
                c: b,
                shape: shapes[1],
                hyperbola: flips[3],
            },
            R::Internal {
                p: a,
                c: b,
                shape: shapes[2],
                at,
            },
            R::Focus {
                p: a,
                c: b,
                shape: shapes[3],
            },
            R::Rigid {
                p: e,
                frame: FRAME,
                x: 0.5,
                y: -0.25,
            },
            R::RigidAxes {
                k: s1,
                frame: FRAME,
                x: 2.0,
                y: 0.5,
                minor: flips[4].then_some(0.8),
            },
        ]);
        specs
    }

    /// Points within a box, radii and minor radii positive, axes of some
    /// length, a frame's angle and curve parameters inside their range.
    fn values(rng: &mut Rng) -> Vec<f64> {
        (0..VARS)
            .map(|i| match i {
                _ if i < RADII => rng.within(-5.0, 5.0),
                _ if i < SHAPES => rng.within(0.5, 3.0),
                _ if i < FRAME => match (i - SHAPES) % 3 {
                    0 => rng.within(1.5, 3.0),
                    1 => rng.within(-1.0, 1.0),
                    _ => rng.within(0.4, 1.2),
                },
                _ if i < PARAMS => rng.within(-1.0, 1.0),
                _ => rng.within(0.2, 0.8),
            })
            .collect()
    }

    #[test]
    fn every_kind_differences_as_the_whole_system_does() {
        let mut rng = Rng(0x5eed_0813);
        for _ in 0..40 {
            let vars = values(&mut rng);
            // Some variables held, as references and external geometry are.
            let free: Vec<usize> = (0..VARS).filter(|_| rng.below(5) != 0).collect();
            let specs = every_kind(&mut rng);
            let all = system(vars.clone(), free.clone(), specs.clone());
            let x = part_coincident_ends(&all);
            assert_eq!(jacobian(&all, &x), reference(&all, &x));
            for spec in specs {
                let one = system(vars.clone(), free.clone(), vec![spec]);
                assert_eq!(
                    jacobian(&one, &x),
                    reference(&one, &x),
                    "{:?}",
                    one.specs[0]
                );
            }
        }
    }

    /// Clusters of a few equations each over points of their own, some
    /// repeated (a redundant row), some joined to the one before.
    fn clusters(rng: &mut Rng) -> System {
        let clusters = 2 + rng.below(5);
        let mut vars = Vec::new();
        let mut specs = Vec::new();
        for k in 0..clusters {
            let first = vars.len();
            for _ in 0..4 {
                vars.extend([rng.within(-5.0, 5.0), rng.within(-5.0, 5.0)]);
            }
            vars.push(rng.within(0.5, 2.0));
            let radius = first + 8;
            for _ in 0..1 + rng.below(4) {
                let p = rng.points(first, 4, 3);
                let spec = match rng.below(6) {
                    0 => R::Horizontal { s: p[0], e: p[1] },
                    1 => R::Distance {
                        p1: p[0],
                        p2: p[1],
                        d: 2.0,
                    },
                    2 => R::PointOnLine {
                        p: p[0],
                        s: p[1],
                        e: p[2],
                    },
                    3 => R::PointOnCircle {
                        p: p[0],
                        c: p[1],
                        r: radius,
                    },
                    4 => R::FixedPoint {
                        p: p[0],
                        x: 0.0,
                        y: 1.0,
                    },
                    _ => R::Parallel {
                        s1: p[0],
                        e1: p[1],
                        s2: p[1],
                        e2: p[2],
                    },
                };
                if rng.below(4) == 0 {
                    specs.push(spec.clone());
                }
                specs.push(spec);
            }
            if k > 0 && rng.below(3) == 0 {
                specs.push(R::Coincident {
                    p1: first,
                    p2: first - 9,
                });
            }
        }
        let free = (0..vars.len()).filter(|_| rng.below(6) != 0).collect();
        system(vars, free, specs)
    }

    #[test]
    fn blocks_rank_and_hold_as_the_whole_matrix_does() {
        let mut rng = Rng(0x0b10c5);
        let tolerance = Settings::default().rank_tolerance;
        for _ in 0..200 {
            let sys = clusters(&mut rng);
            let whole = reference(&sys, &part_coincident_ends(&sys));
            assert_eq!(
                rank(&sys, tolerance),
                jacobian_rank(whole.clone(), tolerance)
            );
            let max_abs = whole.iter().flatten().fold(0.0_f64, |m, v| m.max(v.abs()));
            let held = if max_abs > 0.0 {
                crate::freedom::determined_within(whole, max_abs * tolerance, tolerance)
            } else {
                vec![false; sys.free.len()]
            };
            assert_eq!(determined(&sys, tolerance), held);
            assert!(Blocks::of(&sys).blocks.len() <= sys.free.len());
        }
    }
}
