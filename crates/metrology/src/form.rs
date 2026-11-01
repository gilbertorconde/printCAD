//! Form error from the residuals of a fit: signed distances of the used
//! points from the ideal shape, positive outside the material.

/// Peak-to-peak form error: the width of the band the residuals fill.
/// About a least-squares fit this is no smaller than the minimum zone.
pub fn peak_to_peak(residuals: &[f64]) -> Option<f64> {
    let s = crate::stats::summary(residuals)?;
    Some(s.max - s.min)
}

/// The smallest and largest diameter a round surface reaches: its fitted
/// radius moved in to the most negative residual and out to the most
/// positive, doubled.
pub fn diameter_extremes(radius: f64, residuals: &[f64]) -> Option<(f64, f64)> {
    let s = crate::stats::summary(residuals)?;
    Some((2.0 * (radius + s.min), 2.0 * (radius + s.max)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bore_reads_its_band_and_its_diameters() {
        let residuals = [-0.02, 0.01, 0.03, f64::NAN];
        assert!((peak_to_peak(&residuals).unwrap() - 0.05).abs() < 1e-12);
        let (low, high) = diameter_extremes(5.0, &residuals).unwrap();
        assert!((low - 9.96).abs() < 1e-12 && (high - 10.06).abs() < 1e-12);
        assert_eq!(peak_to_peak(&[]), None);
    }
}
