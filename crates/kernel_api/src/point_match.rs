//! When two mesh points count as one: the key every weld and every match
//! of mesh corners hashes a point by, so each caller says only how close.

/// When two mesh points are one point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointMatch {
    /// The very same coordinates, `-0.0` and `0.0` alike.
    Exact,
    /// Rounding to the same cell of a grid of `per_mm` cells to the
    /// millimetre on each axis.
    Grid { per_mm: f64 },
}

impl PointMatch {
    /// The key `p` hashes by: two points are one when their keys are equal.
    /// A grid's cells have edges, so two points a hair apart either side of
    /// one stay two.
    pub fn key(self, p: [f32; 3]) -> [i64; 3] {
        match self {
            // Adding zero turns `-0.0` into `0.0` and leaves every other
            // value as it is.
            Self::Exact => p.map(|c| i64::from((c + 0.0).to_bits())),
            Self::Grid { per_mm } => p.map(|c| (f64::from(c) * per_mm).round() as i64),
        }
    }
}
