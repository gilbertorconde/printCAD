//! Loads on a stepped shaft and what they do to it: the shaft rests on two
//! bearings that hold it up but let it turn and tilt, carries forces
//! square to its axis, each at a place along it and an angle about it,
//! and passes a torque between two places.
//!
//! The analysis is beam theory: reactions from the balance of forces and
//! moments, the bending moment along the shaft in two planes at right
//! angles, bending stress `32 M / π d³` and torsional stress
//! `16 T / π d³` at the surface, combined as von Mises stress, and the
//! deflection from integrating `M / E I` twice with the shaft level at both
//! bearings. Stress concentrations at the steps are not counted.

use serde::{Deserialize, Serialize};

use super::shaft::ShaftSpec;

/// A force square to the axis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShaftForce {
    /// Where along the shaft from its first end, mm.
    pub at: f64,
    /// How hard, N.
    pub force: f64,
    /// Which way about the axis, degrees from the sketch's horizontal.
    pub angle_deg: f64,
}

impl Default for ShaftForce {
    fn default() -> Self {
        Self {
            at: 0.0,
            force: 100.0,
            angle_deg: 90.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShaftLoads {
    /// Where the two bearings stand from the first end, mm.
    pub bearings: [f64; 2],
    pub forces: Vec<ShaftForce>,
    /// The torque the shaft passes, N·m, between `torque_from` and
    /// `torque_to` (mm from the first end).
    pub torque: f64,
    pub torque_from: f64,
    pub torque_to: f64,
    /// Young's modulus of the material, GPa (steel's 210 when made).
    pub modulus: f64,
}

impl Default for ShaftLoads {
    fn default() -> Self {
        Self {
            bearings: [0.0, 0.0],
            forces: Vec::new(),
            torque: 0.0,
            torque_from: 0.0,
            torque_to: 0.0,
            modulus: 210.0,
        }
    }
}

impl ShaftLoads {
    /// Whether anything loads the shaft.
    pub fn any(&self) -> bool {
        self.torque != 0.0 || self.forces.iter().any(|f| f.force != 0.0)
    }
}

/// One place along the shaft, as the loads leave it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Station {
    /// From the first end, mm.
    pub at: f64,
    /// Bending moment, N·mm.
    pub moment: f64,
    /// Von Mises stress at the surface, MPa.
    pub stress: f64,
    /// How far the axis is bent from the line through the bearings, mm.
    pub deflection: f64,
}

/// What the loads do to the shaft.
#[derive(Debug, Clone, PartialEq)]
pub struct ShaftAnalysis {
    /// Each bearing's reaction, N.
    pub reactions: [f64; 2],
    pub stations: Vec<Station>,
}

impl ShaftAnalysis {
    fn peak(&self, of: impl Fn(&Station) -> f64) -> (f64, f64) {
        self.stations
            .iter()
            .map(|s| (of(s).abs(), s.at))
            .fold((0.0, 0.0), |best, s| if s.0 > best.0 { s } else { best })
    }

    /// The largest bending moment, N·mm, and where it is.
    pub fn max_moment(&self) -> (f64, f64) {
        self.peak(|s| s.moment)
    }

    /// The largest von Mises stress, MPa, and where it is.
    pub fn max_stress(&self) -> (f64, f64) {
        self.peak(|s| s.stress)
    }

    /// The largest deflection, mm, and where it is.
    pub fn max_deflection(&self) -> (f64, f64) {
        self.peak(|s| s.deflection)
    }
}

/// How many stations the shaft is measured at.
const STATIONS: usize = 600;

/// Work out what `loads` do to `shaft`.
pub fn analyse(shaft: &ShaftSpec, loads: &ShaftLoads) -> Result<ShaftAnalysis, String> {
    let length: f64 = shaft.sections.iter().map(|s| s.length).sum();
    if length.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return Err("the shaft has no length".into());
    }
    let [a, b] = loads.bearings.map(f64::from);
    let inside = |x: f64| (0.0..=length).contains(&x);
    if !inside(a) || !inside(b) {
        return Err("the bearings must stand on the shaft".into());
    }
    if (b - a).abs() < 1e-6 {
        return Err("the bearings must stand apart".into());
    }
    if loads.forces.iter().any(|f| !inside(f.at)) {
        return Err("every force must act on the shaft".into());
    }
    let modulus = loads.modulus * 1e3;
    if modulus.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return Err("the material's modulus must be more than zero".into());
    }

    // Each force in the two planes, and the bearings' share of each: the
    // moments about the first bearing balance.
    let forces: Vec<(f64, [f64; 2])> = loads
        .forces
        .iter()
        .map(|f| {
            let angle = f.angle_deg.to_radians();
            let force = f.force;
            (f.at, [force * angle.cos(), force * angle.sin()])
        })
        .collect();
    let mut reactions = [[0.0; 2]; 2];
    for plane in 0..2 {
        let total: f64 = forces.iter().map(|(_, f)| f[plane]).sum();
        let moment: f64 = forces.iter().map(|(x, f)| f[plane] * (x - a)).sum();
        let second = -moment / (b - a);
        reactions[1][plane] = second;
        reactions[0][plane] = -total - second;
    }
    let point_loads: Vec<(f64, [f64; 2])> = forces
        .iter()
        .copied()
        .chain([(a, reactions[0]), (b, reactions[1])])
        .collect();

    let diameter_at = |x: f64| {
        let mut end = 0.0;
        for s in &shaft.sections {
            end += s.length;
            if x <= end + 1e-9 {
                return s.diameter;
            }
        }
        shaft.sections.last().map_or(0.0, |s| s.diameter)
    };
    let torque = loads.torque * 1e3;
    let (t0, t1) = {
        let (p, q) = (loads.torque_from, loads.torque_to);
        (p.min(q), p.max(q))
    };

    let xs: Vec<f64> = (0..=STATIONS)
        .map(|i| length * i as f64 / STATIONS as f64)
        .collect();
    // The moment at x from everything before it, in each plane.
    let moments: Vec<[f64; 2]> = xs
        .iter()
        .map(|&x| {
            let mut m = [0.0; 2];
            for (at, f) in &point_loads {
                if *at < x {
                    m[0] += f[0] * (x - at);
                    m[1] += f[1] * (x - at);
                }
            }
            m
        })
        .collect();

    // Slope then deflection from M / E I, fixed so both bearings sit on
    // the line.
    let mut deflection = [vec![0.0; xs.len()], vec![0.0; xs.len()]];
    for (plane, bent) in deflection.iter_mut().enumerate() {
        let curvature: Vec<f64> = xs
            .iter()
            .zip(&moments)
            .map(|(&x, m)| {
                let d = diameter_at(x);
                let i = std::f64::consts::PI * d.powi(4) / 64.0;
                if i > 0.0 {
                    m[plane] / (modulus * i)
                } else {
                    0.0
                }
            })
            .collect();
        let mut slope = 0.0;
        for k in 1..xs.len() {
            let h = xs[k] - xs[k - 1];
            let slope_next = slope + 0.5 * (curvature[k - 1] + curvature[k]) * h;
            bent[k] = bent[k - 1] + 0.5 * (slope + slope_next) * h;
            slope = slope_next;
        }
        let at = |x: f64| {
            let k = ((x / length) * STATIONS as f64).clamp(0.0, STATIONS as f64);
            let (lo, t) = (k.floor() as usize, k.fract());
            let hi = (lo + 1).min(STATIONS);
            bent[lo] * (1.0 - t) + bent[hi] * t
        };
        let (ya, yb) = (at(a), at(b));
        for (k, &x) in xs.iter().enumerate() {
            bent[k] -= ya + (yb - ya) * (x - a) / (b - a);
        }
    }

    let stations = xs
        .iter()
        .enumerate()
        .map(|(k, &x)| {
            let moment = moments[k][0].hypot(moments[k][1]);
            let d = diameter_at(x);
            let pi = std::f64::consts::PI;
            let (bending, twisting) = if d > 0.0 {
                let t = if (t0..=t1).contains(&x) { torque } else { 0.0 };
                (
                    32.0 * moment / (pi * d.powi(3)),
                    16.0 * t / (pi * d.powi(3)),
                )
            } else {
                (0.0, 0.0)
            };
            Station {
                at: x,
                moment,
                stress: (bending * bending + 3.0 * twisting * twisting).sqrt(),
                deflection: deflection[0][k].hypot(deflection[1][k]),
            }
        })
        .collect();
    Ok(ShaftAnalysis {
        reactions: reactions.map(|r| r[0].hypot(r[1])),
        stations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::ShaftSection;
    use std::f64::consts::PI;

    fn plain(length: f64, diameter: f64) -> ShaftSpec {
        ShaftSpec {
            sections: vec![ShaftSection {
                length,
                diameter,
                ..ShaftSection::default()
            }],
            start_chamfer: 0.0,
            loads: ShaftLoads::default(),
        }
    }

    /// A force in the middle of a shaft on bearings at its ends: half on
    /// each, `F L / 4` under it, `F L³ / 48 E I` down.
    #[test]
    fn a_centre_load_matches_the_textbook_beam() {
        let shaft = plain(200.0, 20.0);
        let loads = ShaftLoads {
            bearings: [0.0, 200.0],
            forces: vec![ShaftForce {
                at: 100.0,
                force: 1000.0,
                angle_deg: 90.0,
            }],
            ..ShaftLoads::default()
        };
        let got = analyse(&shaft, &loads).unwrap();
        assert!((got.reactions[0] - 500.0).abs() < 1e-6);
        assert!((got.reactions[1] - 500.0).abs() < 1e-6);
        let (moment, at) = got.max_moment();
        assert!((moment - 1000.0 * 200.0 / 4.0).abs() < 1.0, "{moment}");
        assert!((at - 100.0).abs() < 1.0);
        let (stress, _) = got.max_stress();
        let want = 32.0 * 50_000.0 / (PI * 20f64.powi(3));
        assert!((stress - want).abs() < 0.05, "{stress} vs {want}");
        let i = PI * 20f64.powi(4) / 64.0;
        let want = 1000.0 * 200f64.powi(3) / (48.0 * 210e3 * i);
        let (sag, at) = got.max_deflection();
        assert!((sag - want).abs() < 1e-3 * want, "{sag} vs {want}");
        assert!((at - 100.0).abs() < 1.0);
    }

    /// An overhung load: the far bearing pulls the other way, and the
    /// moment peaks over the near one.
    #[test]
    fn an_overhung_load_levers_on_the_bearings() {
        let shaft = plain(150.0, 20.0);
        let loads = ShaftLoads {
            bearings: [0.0, 100.0],
            forces: vec![ShaftForce {
                at: 150.0,
                force: 200.0,
                angle_deg: 0.0,
            }],
            ..ShaftLoads::default()
        };
        let got = analyse(&shaft, &loads).unwrap();
        assert!((got.reactions[0] - 100.0).abs() < 1e-6);
        assert!((got.reactions[1] - 300.0).abs() < 1e-6);
        let (moment, at) = got.max_moment();
        assert!((moment - 200.0 * 50.0).abs() < 1.0);
        assert!((at - 100.0).abs() < 1.0);
    }

    /// Torque alone twists the thinner section hardest.
    #[test]
    fn torque_stresses_the_thin_section_most() {
        let mut shaft = plain(50.0, 20.0);
        shaft.sections.push(ShaftSection {
            length: 50.0,
            diameter: 10.0,
            ..ShaftSection::default()
        });
        let loads = ShaftLoads {
            bearings: [0.0, 100.0],
            torque: 10.0,
            torque_from: 0.0,
            torque_to: 100.0,
            ..ShaftLoads::default()
        };
        let got = analyse(&shaft, &loads).unwrap();
        let (stress, at) = got.max_stress();
        let tau = 16.0 * 10_000.0 / (PI * 1000.0);
        assert!((stress - 3f64.sqrt() * tau).abs() < 1e-6, "{stress}");
        assert!(at > 50.0);
        assert_eq!(got.max_deflection().0, 0.0);
    }

    #[test]
    fn bearings_off_the_shaft_are_refused() {
        let loads = ShaftLoads {
            bearings: [0.0, 300.0],
            ..ShaftLoads::default()
        };
        assert!(analyse(&plain(100.0, 10.0), &loads).is_err());
    }
}
