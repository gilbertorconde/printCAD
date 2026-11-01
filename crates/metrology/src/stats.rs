//! What a set of values says: its summary, percentiles, histogram and the
//! share within a tolerance. A `NaN` is a value that was not measured
//! (beyond a search distance, say) and is left out of all of them.

/// A set of values in a few numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    /// How many values were measured.
    pub count: usize,
    /// How many were not (`NaN`).
    pub unmeasured: usize,
    pub mean: f64,
    /// Root mean square: the typical size of a value, its sign aside.
    pub rms: f64,
    /// Standard deviation about the mean.
    pub sigma: f64,
    pub min: f64,
    pub max: f64,
}

/// The summary of `values`, or `None` when none was measured.
pub fn summary(values: &[f64]) -> Option<Summary> {
    let measured = values.iter().copied().filter(|v| !v.is_nan());
    let (mut count, mut sum, mut squares) = (0usize, 0.0, 0.0);
    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    for v in measured {
        count += 1;
        sum += v;
        squares += v * v;
        min = min.min(v);
        max = max.max(v);
    }
    if count == 0 {
        return None;
    }
    let n = count as f64;
    let mean = sum / n;
    // Taken about the mean, not as squares/n − mean², which loses digits
    // when the values sit far from zero.
    let spread: f64 = values
        .iter()
        .filter(|v| !v.is_nan())
        .map(|v| (v - mean).powi(2))
        .sum();
    Some(Summary {
        count,
        unmeasured: values.len() - count,
        mean,
        rms: (squares / n).sqrt(),
        sigma: (spread / n).sqrt(),
        min,
        max,
    })
}

/// The value below which `p` (0 to 1) of the measured values lie, by
/// linear interpolation between neighbours; `None` with nothing measured.
pub fn percentile(values: &[f64], p: f64) -> Option<f64> {
    let mut sorted: Vec<f64> = values.iter().copied().filter(|v| !v.is_nan()).collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(f64::total_cmp);
    let at = p.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let (below, above) = (at.floor() as usize, at.ceil() as usize);
    let t = at - below as f64;
    Some(sorted[below] * (1.0 - t) + sorted[above] * t)
}

/// The share (0 to 1) of measured values within `low..=high`; `None` with
/// nothing measured.
pub fn share_within(values: &[f64], low: f64, high: f64) -> Option<f64> {
    let s = summary(values)?;
    let within = values.iter().filter(|v| (low..=high).contains(*v)).count();
    Some(within as f64 / s.count as f64)
}

/// Counts of measured values in equal bins across `low..high`; values
/// beyond either end count in the end bins, which a legend draws open.
#[derive(Debug, Clone, PartialEq)]
pub struct Histogram {
    pub low: f64,
    pub high: f64,
    pub counts: Vec<usize>,
}

impl Histogram {
    pub fn of(values: &[f64], low: f64, high: f64, bins: usize) -> Self {
        let bins = bins.max(1);
        let mut counts = vec![0; bins];
        let width = (high - low) / bins as f64;
        for v in values.iter().filter(|v| !v.is_nan()) {
            let at = if width > 0.0 {
                ((v - low) / width).floor()
            } else {
                0.0
            };
            counts[(at.max(0.0) as usize).min(bins - 1)] += 1;
        }
        Self { low, high, counts }
    }

    /// Bin `i`'s range.
    pub fn bin(&self, i: usize) -> (f64, f64) {
        let width = (self.high - self.low) / self.counts.len() as f64;
        (
            self.low + width * i as f64,
            self.low + width * (i + 1) as f64,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Closed forms: a symmetric set about 10 has mean 10, sigma its half
    /// width; unmeasured values are left out of everything.
    #[test]
    fn a_set_reads_its_closed_forms() {
        let values = [9.0, 11.0, f64::NAN, 9.0, 11.0];
        let s = summary(&values).unwrap();
        assert_eq!((s.count, s.unmeasured), (4, 1));
        assert_eq!((s.mean, s.sigma, s.min, s.max), (10.0, 1.0, 9.0, 11.0));
        assert!((s.rms - (101.0f64).sqrt()).abs() < 1e-12);
        assert_eq!(summary(&[f64::NAN]), None);
        let ramp: Vec<f64> = (0..=100).map(f64::from).collect();
        assert_eq!(percentile(&ramp, 0.95), Some(95.0));
        assert_eq!(percentile(&[1.0, 2.0], 0.5), Some(1.5));
        assert_eq!(share_within(&ramp, 10.0, 19.5), Some(10.0 / 101.0));
        let h = Histogram::of(&[-5.0, 0.1, 0.9, 1.0, 99.0, f64::NAN], 0.0, 2.0, 2);
        assert_eq!((h.counts.clone(), h.bin(1)), (vec![3, 2], (1.0, 2.0)));
    }
}
