//! Colour ramps for maps, and where a value falls on one. Colours are sRGB
//! components from 0 to 1. Viridis, magma, inferno and plasma are the
//! public-domain perceptually uniform ramps, kept here as ten even samples
//! each and blended between them.

/// A colour ramp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ramp {
    /// Blue through cyan, green and yellow to red: zero reads green at the
    /// middle of a signed map.
    #[default]
    Jet,
    Viridis,
    Magma,
    Inferno,
    Plasma,
}

impl Ramp {
    pub const ALL: [Ramp; 5] = [
        Ramp::Jet,
        Ramp::Viridis,
        Ramp::Magma,
        Ramp::Inferno,
        Ramp::Plasma,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Ramp::Jet => "jet",
            Ramp::Viridis => "viridis",
            Ramp::Magma => "magma",
            Ramp::Inferno => "inferno",
            Ramp::Plasma => "plasma",
        }
    }

    /// The colour at `t`, from 0 (the low end) to 1 (the high end).
    pub fn at(self, t: f64) -> [f32; 3] {
        let t = if t.is_nan() { 0.5 } else { t.clamp(0.0, 1.0) };
        match self {
            Ramp::Jet => jet(t),
            Ramp::Viridis => sampled(&VIRIDIS, t),
            Ramp::Magma => sampled(&MAGMA, t),
            Ramp::Inferno => sampled(&INFERNO, t),
            Ramp::Plasma => sampled(&PLASMA, t),
        }
    }

    /// The colours beyond the low and the high end: darker than anything on
    /// the ramp, so an out-of-range value never reads as an in-range one.
    pub fn caps(self) -> ([f32; 3], [f32; 3]) {
        let darken = |c: [f32; 3]| c.map(|v| v * 0.35);
        (darken(self.at(0.0)), darken(self.at(1.0)))
    }
}

/// Where `value` falls on a signed scale of half-width `range` centred on
/// zero (0 at −range, 0.5 at zero, 1 at +range), split into `bands` steps
/// when there are any; `None` beyond either end, where a cap colour goes.
pub fn signed(value: f64, range: f64, bands: u32) -> Option<f64> {
    span(value, -range, range, bands)
}

/// Where `value` falls between `low` and `high` (0 to 1), in `bands`
/// steps when there are any; `None` outside, or when not measured.
pub fn span(value: f64, low: f64, high: f64, bands: u32) -> Option<f64> {
    if value.is_nan() || high <= low || value < low || value > high {
        return None;
    }
    let t = (value - low) / (high - low);
    Some(match bands {
        0 => t,
        n => ((t * f64::from(n)).floor().min(f64::from(n) - 1.0) + 0.5) / f64::from(n),
    })
}

fn jet(t: f64) -> [f32; 3] {
    let channel = |centre: f64| (1.5 - (4.0 * t - centre).abs()).clamp(0.0, 1.0) as f32;
    [channel(3.0), channel(2.0), channel(1.0)]
}

fn sampled(stops: &[u32; 10], t: f64) -> [f32; 3] {
    let at = t * 9.0;
    let i = (at.floor() as usize).min(8);
    let f = (at - i as f64) as f32;
    let (a, b) = (rgb(stops[i]), rgb(stops[i + 1]));
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * f)
}

fn rgb(hex: u32) -> [f32; 3] {
    [16, 8, 0].map(|shift| ((hex >> shift) & 0xff) as f32 / 255.0)
}

const VIRIDIS: [u32; 10] = [
    0x440154, 0x482878, 0x3e4989, 0x31688e, 0x26828e, 0x1f9e89, 0x35b779, 0x6dcd59, 0xb4de2c,
    0xfde725,
];
const MAGMA: [u32; 10] = [
    0x000004, 0x180f3d, 0x440f76, 0x721f81, 0x9e2f7f, 0xcd4071, 0xf1605d, 0xfd9668, 0xfec98d,
    0xfcfdbf,
];
const INFERNO: [u32; 10] = [
    0x000004, 0x1b0c41, 0x4a0c6b, 0x781c6d, 0xa52c60, 0xcf4446, 0xed6925, 0xfb9b06, 0xf7d13d,
    0xfcffa4,
];
const PLASMA: [u32; 10] = [
    0x0d0887, 0x46039f, 0x7201a8, 0x9c179e, 0xbd3786, 0xd8576b, 0xed7953, 0xfb9f3a, 0xfdca26,
    0xf0f921,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramps_run_end_to_end_and_values_land_on_them() {
        // Jet: blue, green at the middle, red; viridis from its first
        // sample to its last; caps darker than either end.
        assert_eq!(Ramp::Jet.at(0.0), [0.0, 0.0, 0.5]);
        assert_eq!(Ramp::Jet.at(0.5), [0.5, 1.0, 0.5]);
        assert_eq!(Ramp::Jet.at(1.0), [0.5, 0.0, 0.0]);
        assert_eq!(Ramp::Viridis.at(0.0), rgb(0x440154));
        assert_eq!(Ramp::Viridis.at(1.0), rgb(0xfde725));
        for ramp in Ramp::ALL {
            let (low, high) = ramp.caps();
            let brightness = |c: [f32; 3]| c.iter().sum::<f32>();
            assert!(
                brightness(low) <= brightness(ramp.at(0.0)),
                "{}",
                ramp.name()
            );
            assert!(
                brightness(high) < brightness(ramp.at(1.0)),
                "{}",
                ramp.name()
            );
        }
        assert_eq!(signed(0.0, 1.0, 0), Some(0.5));
        assert_eq!(signed(1.5, 1.0, 0), None);
        assert_eq!(signed(f64::NAN, 1.0, 0), None);
        // Four bands: a value just above zero lands in the third band's middle.
        assert_eq!(signed(0.01, 1.0, 4), Some(0.625));
        assert_eq!(span(10.0, 0.0, 10.0, 4), Some(0.875));
    }
}
