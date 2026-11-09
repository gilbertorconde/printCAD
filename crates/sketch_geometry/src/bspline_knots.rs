//! Exact knot refinement and homogeneous fitting when knots are removed.
use glam::DVec3;

use super::{BSplineCurve, BSplinePiece, SplineDegree, SplineMultiplicity, SplineParam};
use crate::world::WorldPoint;

impl BSplineCurve {
    /// Distinct interior knots on one carrier period. End knots are clamped
    /// boundaries, not editable interior knots, including on a closed carrier.
    pub fn interior_knots(&self) -> Vec<(SplineParam, SplineMultiplicity)> {
        let (a, b) = self.domain();
        self.knots
            .chunk_by(|a, b| a == b)
            .filter(|run| run[0] > a.0 && run[0] < b.0)
            .map(|run| {
                (
                    SplineParam(run[0]),
                    SplineMultiplicity::new(run.len() as u32),
                )
            })
            .collect()
    }

    /// Set a knot's total multiplicity, or insert it when absent. Increasing
    /// uses Boehm insertion on weighted poles and preserves the curve exactly.
    /// Decreasing fits in the smaller basis; callers must report its deviation.
    pub fn with_knot_multiplicity(
        &self,
        at: SplineParam,
        multiplicity: SplineMultiplicity,
    ) -> Result<BSplinePiece, &'static str> {
        let (a, b) = self.domain();
        if !at.0.is_finite() || at.0 <= a.0 || at.0 >= b.0 {
            return Err("A knot parameter must be finite and strictly inside the carrier domain");
        }
        let count = multiplicity.get() as usize;
        if count > self.degree {
            return Err("Interior knot multiplicity cannot exceed the spline degree");
        }
        let mut piece = self.piece([a, b], false);
        let now = piece.knots.iter().filter(|&&k| k == at.0).count();
        if now == count {
            return Err("The knot already has this multiplicity");
        }
        if count < now {
            let mut left = count;
            piece.knots.retain(|&k| {
                if k != at.0 {
                    true
                } else if left > 0 {
                    left -= 1;
                    true
                } else {
                    false
                }
            });
            return self.refit_basis(SplineDegree::new(self.degree as u32), piece.knots, false);
        }
        let mut poles: Vec<_> = piece
            .poles
            .iter()
            .zip(&piece.weights)
            .map(|(p, &w)| DVec3::new(p.0.x * w, p.0.y * w, w))
            .collect();
        for _ in now..count {
            super::measure::insert_knot(&mut piece.knots, &mut poles, self.degree, at.0);
        }
        piece.weights = poles.iter().map(|p| p.z).collect();
        piece.poles = poles
            .iter()
            .map(|p| WorldPoint::from_dvec2(p.truncate() / p.z))
            .collect();
        Ok(piece)
    }
}
