//! Degree conversion in homogeneous coordinates. Knot multiplicities follow the
//! degree difference, so elevation retains continuity as well as the curve.
use super::{
    BSplineCurve, BSplinePiece, SplineBasis, SplineDefinition, SplineDegree, SplineParam,
    SplineShape, solve_dense,
};
use crate::world::{WorldLength, WorldPoint};

impl BSplineCurve {
    /// Clamped representation of the same carrier at a new degree. Elevation
    /// solves an exact change of basis at Greville abscissae. Reduction fits the
    /// homogeneous curve least squares, with both ends pinned. Positive weights
    /// are required: an invalid fit is refused before it can enter the sketch.
    pub fn with_degree(&self, degree: SplineDegree) -> Result<BSplinePiece, &'static str> {
        let piece = self.piece([self.domain().0, self.domain().1], false);
        let p = degree.get() as usize;
        if p == self.degree {
            return Ok(piece);
        }
        let [a, b] = piece.range;
        let mut knots = vec![a.0; p + 1];
        for run in piece.knots.chunk_by(|a, b| a == b) {
            if run[0] > a.0 && run[0] < b.0 {
                let m = (run.len() as isize + p as isize - self.degree as isize).max(1) as usize;
                knots.extend(std::iter::repeat_n(run[0], m.min(p)));
            }
        }
        knots.extend(std::iter::repeat_n(b.0, p + 1));
        self.refit_basis(degree, knots, p > self.degree)
    }

    /// Change of basis in homogeneous coordinates. An exact superspace uses
    /// Greville collocation; a smaller space uses a fit with pinned endpoints.
    pub(super) fn refit_basis(
        &self,
        degree: SplineDegree,
        knots: Vec<f64>,
        exact: bool,
    ) -> Result<BSplinePiece, &'static str> {
        let piece = self.piece([self.domain().0, self.domain().1], false);
        let [a, b] = piece.range;
        let p = degree.get() as usize;
        let n = knots.len() - p - 1;
        let shell = Self {
            degree: p,
            knots: knots.clone(),
            poles: vec![[0.0; 2]; n],
            distinct: n,
            periodic: false,
            weights: None,
        };
        // The clamped carrier has the same homogeneous values even when the
        // original definition used interpolation points or a periodic basis.
        let source = Self::build_with_basis(
            SplineShape {
                definition: SplineDefinition::Poles,
                degree: SplineDegree::new(self.degree as u32),
                periodic: false,
            },
            Some(&SplineBasis {
                knots: piece.knots.clone(),
                weights: piece.weights.clone(),
            }),
            &piece.poles,
        )
        .unwrap();
        let lifted_at = |t| {
            let row = source.basis_row(t, source.poles.len());
            row.iter().enumerate().fold([0.0; 3], |mut out, (i, &v)| {
                let w = piece.weights[i];
                out[0] += v * piece.poles[i].0.x * w;
                out[1] += v * piece.poles[i].0.y * w;
                out[2] += v * w;
                out
            })
        };
        let solve = |rows: Vec<Vec<f64>>, rhs: Vec<[f64; 3]>| {
            let xy = solve_dense(rows.clone(), rhs.iter().map(|q| [q[0], q[1]]).collect());
            let w = solve_dense(rows, rhs.iter().map(|q| [q[2], 0.0]).collect());
            xy.into_iter()
                .zip(w)
                .map(|(q, w)| [q[0], q[1], w[0]])
                .collect::<Vec<_>>()
        };
        let control = if exact {
            let params: Vec<_> = (0..n)
                .map(|i| knots[i + 1..=i + p].iter().sum::<f64>() / p as f64)
                .collect();
            solve(
                params.iter().map(|&t| shell.basis_row(t, n)).collect(),
                params.iter().map(|&t| lifted_at(t)).collect(),
            )
        } else {
            let first = lifted_at(a.0);
            let last = lifted_at(b.0);
            let mut out = vec![first; n];
            out[n - 1] = last;
            if n > 2 {
                let mut normal = vec![vec![0.0; n - 2]; n - 2];
                let mut rhs = vec![[0.0; 3]; n - 2];
                let breaks = source.breaks([a, b]);
                for span in breaks.windows(2) {
                    for k in 0..40 {
                        let t = span[0] + (span[1] - span[0]) * (k as f64 + 0.5) / 40.0;
                        let row = shell.basis_row(t, n);
                        let mut q = lifted_at(t);
                        for d in 0..3 {
                            q[d] -= row[0] * first[d] + row[n - 1] * last[d];
                        }
                        for i in 1..n - 1 {
                            for d in 0..3 {
                                rhs[i - 1][d] += row[i] * q[d];
                            }
                            for j in 1..n - 1 {
                                normal[i - 1][j - 1] += row[i] * row[j];
                            }
                        }
                    }
                }
                out[1..n - 1].copy_from_slice(&solve(normal, rhs));
            }
            out
        };
        if !control
            .iter()
            .all(|q| q.iter().all(|x| x.is_finite()) && q[2] > 0.0)
        {
            return Err(
                "Spline basis edit produced non-positive or non-finite weights; curve unchanged",
            );
        }
        Ok(BSplinePiece {
            degree: p,
            knots,
            weights: control.iter().map(|q| q[2]).collect(),
            poles: control
                .iter()
                .map(|q| WorldPoint::new(q[0] / q[2], q[1] / q[2]))
                .collect(),
            range: [a, b],
        })
    }

    /// Maximum distance at matching parameters, sampled in every source and
    /// target span. This is an explicitly sampled estimate, not a certified bound.
    pub fn sampled_deviation(&self, other: &Self, window: [SplineParam; 2]) -> WorldLength {
        let mut breaks = self.breaks(window);
        breaks.extend(other.breaks(window));
        breaks.sort_by(f64::total_cmp);
        breaks.dedup();
        WorldLength::new(
            breaks
                .windows(2)
                .flat_map(|s| {
                    (0..=128).map(move |i| SplineParam(s[0] + (s[1] - s[0]) * i as f64 / 128.0))
                })
                .map(|t| self.point(t).distance(other.point(t)).0)
                .fold(0.0, f64::max),
        )
    }
}
