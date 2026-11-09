//! Exact C0 concatenation: degree elevation and homogeneous weight scaling.
use super::{
    BSplineCurve, BSplinePiece, SplineBasis, SplineDefinition, SplineDegree, SplineParam,
    SplineShape,
};

impl BSplineCurve {
    /// Concatenate oriented, clamped pieces. Each source occupies one unit of
    /// the result parameter. Internal knots retain their multiplicities after
    /// elevation; joins have multiplicity p, so sharp corners stay sharp.
    /// The caller establishes topology; coordinates only validate that topology.
    pub fn join_pieces(pieces: &[BSplinePiece]) -> Result<BSplinePiece, &'static str> {
        if pieces.len() < 2 {
            return Err("Joining needs at least two curve pieces");
        }
        let degree = pieces.iter().map(|p| p.degree).max().unwrap();
        let shape = SplineShape {
            definition: SplineDefinition::Poles,
            degree: SplineDegree::new(degree as u32),
            periodic: false,
        };
        let mut joined = BSplinePiece {
            degree,
            poles: Vec::new(),
            weights: Vec::new(),
            knots: vec![0.0; degree + 1],
            range: [SplineParam(0.0), SplineParam(pieces.len() as f64)],
        };
        for (i, piece) in pieces.iter().enumerate() {
            let source = Self::build_with_basis(
                SplineShape {
                    degree: SplineDegree::new(piece.degree as u32),
                    ..shape
                },
                Some(&SplineBasis {
                    knots: piece.knots.clone(),
                    weights: piece.weights.clone(),
                }),
                &piece.poles,
            )
            .map_err(|_| "A source piece has an invalid spline basis")?;
            let elevated = source.with_degree(shape.degree)?;
            let [a, b] = elevated.range.map(|t| t.0);
            if i > 0 && joined.poles.last().unwrap().distance(elevated.poles[0]).0 > 1e-7 {
                return Err(
                    "The chain endpoints have a geometric gap; solve their coincidence first",
                );
            }
            // A homogeneous curve is unchanged by a common weight factor. Make
            // the two copies of the joint identical before dropping one pole.
            let factor = joined.weights.last().copied().unwrap_or(1.0) / elevated.weights[0];
            let skip = usize::from(i > 0);
            joined
                .poles
                .extend(elevated.poles.iter().skip(skip).copied());
            joined
                .weights
                .extend(elevated.weights.iter().skip(skip).map(|w| w * factor));
            joined.knots.extend(
                elevated.knots[degree + 1..elevated.poles.len()]
                    .iter()
                    .map(|t| i as f64 + (*t - a) / (b - a)),
            );
            joined.knots.extend(std::iter::repeat_n(
                (i + 1) as f64,
                degree + usize::from(i + 1 == pieces.len()),
            ));
        }
        SplineBasis {
            knots: joined.knots.clone(),
            weights: joined.weights.clone(),
        }
        .validate(shape, joined.poles.len())
        .map_err(|_| "Joined spline exceeds the finite basis range")?;
        Ok(joined)
    }
}
