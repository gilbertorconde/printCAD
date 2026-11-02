//! An outline read as radius against angle about a centre, and the peaks
//! along it counted by prominence: a gear's teeth, a knurl's ridges, a
//! star knob's lobes.

/// An outline's radius every `step` degrees round `centre`: the farthest
/// crossing of the outline within each sector (the outline a closed
/// polygon).
pub fn radial(outline: &[[f64; 2]], centre: [f64; 2], sectors: usize) -> Vec<f64> {
    let sectors = sectors.max(8);
    let mut out = vec![f64::NAN; sectors];
    let n = outline.len();
    if n < 3 {
        return out;
    }
    let tau = std::f64::consts::TAU;
    let angle = |p: [f64; 2]| (p[1] - centre[1]).atan2(p[0] - centre[0]).rem_euclid(tau);
    let radius = |p: [f64; 2]| (p[0] - centre[0]).hypot(p[1] - centre[1]);
    // Each edge, sampled finely enough to land in every sector it spans.
    for i in 0..n {
        let (a, b) = (outline[i], outline[(i + 1) % n]);
        let span = {
            let d = (angle(b) - angle(a)).rem_euclid(tau);
            d.min(tau - d)
        };
        let steps = ((span / tau * sectors as f64 * 4.0).ceil() as usize).max(1);
        for k in 0..=steps {
            let t = k as f64 / steps as f64;
            let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            let s = ((angle(p) / tau * sectors as f64) as usize).min(sectors - 1);
            let r = radius(p);
            if out[s].is_nan() || r > out[s] {
                out[s] = r;
            }
        }
    }
    // A sector nothing crossed takes its neighbours' mean.
    for s in 0..sectors {
        if out[s].is_nan() {
            let (l, r) = (out[(s + sectors - 1) % sectors], out[(s + 1) % sectors]);
            out[s] = if l.is_nan() {
                r
            } else if r.is_nan() {
                l
            } else {
                (l + r) / 2.0
            };
        }
    }
    out
}

/// One peak of a closed profile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Peak {
    /// Where, as an index into the profile.
    pub at: usize,
    pub value: f64,
    /// How far it stands above the higher of the lowest points either
    /// side of it before a higher peak.
    pub prominence: f64,
}

/// The peaks of a closed (wrapping) profile standing at least
/// `prominence` above their surroundings, in order round it.
pub fn peaks(values: &[f64], prominence: f64) -> Vec<Peak> {
    let n = values.len();
    if n < 3 {
        return Vec::new();
    }
    let v = |i: isize| values[i.rem_euclid(n as isize) as usize];
    let mut out = Vec::new();
    for i in 0..n as isize {
        let here = v(i);
        // A local maximum; on a flat top only its first sample counts.
        if !(here > v(i - 1) && here >= v(i + 1)) {
            continue;
        }
        // Walk each way to a higher sample or round the whole profile,
        // keeping the lowest point passed.
        let side = |dir: isize| {
            let mut low = here;
            for k in 1..n as isize {
                let x = v(i + dir * k);
                if x > here {
                    break;
                }
                low = low.min(x);
            }
            low
        };
        let p = here - side(-1).max(side(1));
        if p >= prominence {
            out.push(Peak {
                at: i as usize,
                value: here,
                prominence: p,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A twelve-toothed outline (radius 20 with teeth 2 tall) reads twelve
    /// peaks 30° apart, its radius between 20 and 22, and ripple well
    /// under the prominence asked for reads none.
    #[test]
    fn a_toothed_outline_counts_its_teeth() {
        let outline: Vec<[f64; 2]> = (0..720)
            .map(|k| {
                let t = f64::from(k) * std::f64::consts::TAU / 720.0;
                let r = 20.0 + (1.0 + (12.0 * t).cos());
                [5.0 + r * t.cos(), -3.0 + r * t.sin()]
            })
            .collect();
        let profile = radial(&outline, [5.0, -3.0], 360);
        let (lo, hi) = profile
            .iter()
            .fold((f64::MAX, f64::MIN), |(a, b), r| (a.min(*r), b.max(*r)));
        assert!(
            (lo - 20.0).abs() < 0.02 && (hi - 22.0).abs() < 0.02,
            "{lo} {hi}"
        );
        let found = peaks(&profile, 0.5);
        assert_eq!(found.len(), 12);
        for w in found.windows(2) {
            assert!(((w[1].at - w[0].at) as f64 - 30.0).abs() <= 1.0);
        }
        let ripple: Vec<f64> = (0..360).map(|k| 10.0 + 0.01 * f64::from(k % 7)).collect();
        assert!(peaks(&ripple, 0.5).is_empty());
    }
}
