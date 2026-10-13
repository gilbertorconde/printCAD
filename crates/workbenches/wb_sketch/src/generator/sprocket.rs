//! A sprocket for roller chain, its tooth form after ISO 606: each roller
//! seats in an arc a little larger than itself, and the tooth flanks are
//! arcs tangent to the seats, up to the tip circle.
//!
//! Where the standard gives a range (seating radius and angle, flank
//! radius, tip diameter), the sprocket takes its middle.

use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

use super::{Edge, Loop, Outline, P2, add, norm, polar, rotate, scale, sub};

/// Chains a sprocket is made for: (name, pitch, roller diameter), mm.
pub const SPROCKET_CHAINS: &[(&str, f64, f64)] = &[
    ("ISO 05B", 8.0, 5.0),
    ("ISO 06B", 9.525, 6.35),
    ("ISO 08B", 12.7, 8.51),
    ("ISO 10B", 15.875, 10.16),
    ("ANSI 25", 6.35, 3.3),
    ("ANSI 35", 9.525, 5.08),
    ("ANSI 40", 12.7, 7.92),
    ("ANSI 50", 15.875, 10.16),
    ("ANSI 60", 19.05, 11.91),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SprocketSpec {
    /// Chain pitch, mm.
    pub pitch: f64,
    /// Roller diameter (the bush's, for a chain without rollers), mm.
    pub roller: f64,
    #[serde(deserialize_with = "super::count")]
    pub teeth: u32,
    /// Bore diameter, mm; 0 leaves it solid.
    pub bore: f64,
    /// A keyway in the bore.
    pub keyway: super::BoreKeyway,
}

impl Default for SprocketSpec {
    fn default() -> Self {
        Self {
            pitch: 12.7,
            roller: 8.51,
            teeth: 18,
            bore: 8.0,
            keyway: super::BoreKeyway::default(),
        }
    }
}

/// The measured circles and arcs of a sprocket, mm and radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SprocketGeometry {
    pub pitch_diameter: f64,
    pub tip_diameter: f64,
    pub root_diameter: f64,
    pub seating_radius: f64,
    pub seating_angle: f64,
    pub flank_radius: f64,
}

impl SprocketSpec {
    /// The chain preset these numbers are, if any.
    pub fn chain(&self) -> Option<&'static str> {
        SPROCKET_CHAINS
            .iter()
            .find(|(_, p, d)| (p - self.pitch).abs() < 1e-4 && (d - self.roller).abs() < 1e-4)
            .map(|(name, _, _)| *name)
    }

    pub fn geometry(&self) -> Result<SprocketGeometry, String> {
        let p = self.pitch;
        let d1 = self.roller;
        if !(p > 0.0 && p.is_finite()) {
            return Err("the chain pitch must be more than zero".into());
        }
        if !(d1 > 0.0 && d1 < p) {
            return Err("the roller must be more than zero and less than the pitch".into());
        }
        if self.teeth < 5 {
            return Err("a sprocket needs at least 5 teeth".into());
        }
        let z = f64::from(self.teeth);
        let d = p / (PI / z).sin();
        let seating_radius = 0.505 * d1 + 0.069 * d1.cbrt() / 2.0;
        let seating_angle = (130.0 - 90.0 / z).to_radians();
        let flank_radius = (0.008 * d1 * (z * z + 180.0) + 0.12 * d1 * (z + 2.0)) / 2.0;
        let tip_diameter = d + (1.25 * p + p * (1.0 - 1.6 / z)) / 2.0 - d1;
        let g = SprocketGeometry {
            pitch_diameter: d,
            tip_diameter,
            root_diameter: d - 2.0 * seating_radius,
            seating_radius,
            seating_angle,
            flank_radius,
        };
        let bore = self.bore;
        if bore < 0.0 || bore >= g.root_diameter {
            return Err("the bore must fit inside the root circle".into());
        }
        if self.keyway.on && bore <= 0.0 {
            return Err("a keyway needs a bore".into());
        }
        Ok(g)
    }

    pub fn outline(&self) -> Result<Outline, String> {
        let g = self.geometry()?;
        let z = self.teeth as usize;
        let pitch = 2.0 * PI / z as f64;
        let half_tooth = pitch / 2.0;

        // The seat of the roller on angle 0 and the flank that rises from
        // its upper end, towards the tooth centred on half a pitch.
        let roller = [g.pitch_diameter / 2.0, 0.0];
        let up = polar(1.0, PI - g.seating_angle / 2.0);
        let seat_end = add(roller, scale(up, g.seating_radius));
        let flank_center = add(roller, scale(up, g.seating_radius + g.flank_radius));
        let mut tip_radius = g.tip_diameter / 2.0;
        // Flanks that meet before the tip circle make a pointed tooth: the
        // tip comes down to just below where they meet.
        if let Some(meet) = crossing(flank_center, g.flank_radius, half_tooth)
            && meet < tip_radius + 1e-9
        {
            tip_radius = meet - 0.02 * self.pitch;
        }
        let tip = circle_meet(flank_center, g.flank_radius, tip_radius, seat_end)
            .ok_or("the tooth flank never reaches the tip circle")?;
        let tip_angle = tip[1].atan2(tip[0]);
        if !(tip_angle > 0.0 && tip_angle < half_tooth) {
            return Err("the tooth flank misses the tooth: check the roller and the pitch".into());
        }

        let mirror = |p: P2| [p[0], -p[1]];
        let mut lp = Loop::default();
        for k in 0..z {
            let turn = |p: P2| rotate(p, pitch * k as f64);
            // Down the flank below the roller, round the seat, up the
            // flank above it, and over the tip to the next seat.
            let chain = [
                (
                    turn(mirror(tip)),
                    Edge::Arc {
                        center: turn(mirror(flank_center)),
                    },
                ),
                (
                    turn(mirror(seat_end)),
                    Edge::Arc {
                        center: turn(roller),
                    },
                ),
                (
                    turn(seat_end),
                    Edge::Arc {
                        center: turn(flank_center),
                    },
                ),
                (turn(tip), Edge::Arc { center: [0.0, 0.0] }),
            ];
            for (joint, edge) in chain {
                lp.joints.push(joint);
                lp.edges.push(edge);
            }
        }
        let mut outline = Outline {
            loops: vec![lp],
            guides: vec![([0.0, 0.0], g.pitch_diameter / 2.0)],
            ..Outline::default()
        };
        let bore = self.bore;
        if self.keyway.on {
            outline
                .holes
                .push(self.keyway.bore_loop(bore, g.root_diameter / 2.0)?);
        } else if bore > 0.0 {
            outline.circles.push(([0.0, 0.0], bore / 2.0));
        }
        Ok(outline)
    }
}

/// How far out along the ray at `angle` the circle about `center` of
/// radius `r` crosses it last: where a flank rising from the seat meets
/// the tooth's middle.
fn crossing(center: P2, r: f64, angle: f64) -> Option<f64> {
    let b = polar(1.0, angle);
    let along = b[0] * center[0] + b[1] * center[1];
    let disc = along * along - (norm(center).powi(2) - r * r);
    if disc < 0.0 {
        return None;
    }
    let s = along + disc.sqrt();
    (s > 0.0).then_some(s)
}

/// Where the circle about `center` of radius `r` meets the circle about
/// the origin of radius `r0`, the meeting nearest `near`.
fn circle_meet(center: P2, r: f64, r0: f64, near: P2) -> Option<P2> {
    let d = norm(center);
    let a = (r0 * r0 - r * r + d * d) / (2.0 * d);
    let h2 = r0 * r0 - a * a;
    if h2 < 0.0 {
        return None;
    }
    let u = scale(center, 1.0 / d);
    let base = scale(u, a);
    let across = scale([-u[1], u[0]], h2.sqrt());
    let (p, q) = (add(base, across), sub(base, across));
    Some(if norm(sub(p, near)) < norm(sub(q, near)) {
        p
    } else {
        q
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pitch_diameter_is_pitch_over_the_sine_of_half_a_tooth() {
        let spec = SprocketSpec {
            teeth: 17,
            ..SprocketSpec::default()
        };
        let g = spec.geometry().unwrap();
        let want = 12.7f64 / (180f64 / 17.0).to_radians().sin();
        assert!((g.pitch_diameter - want).abs() < 1e-9);
        assert_eq!(spec.chain(), Some("ISO 08B"));
    }

    #[test]
    fn each_roller_seats_on_the_pitch_circle_and_the_teeth_stop_at_the_tip() {
        let spec = SprocketSpec::default();
        let g = spec.geometry().unwrap();
        let outline = spec.outline().unwrap();
        let points = outline.loops[0].polyline(32);
        let radii: Vec<f64> = points.iter().map(|p| norm(*p)).collect();
        let lo = radii.iter().copied().fold(f64::MAX, f64::min);
        let hi = radii.iter().copied().fold(0.0, f64::max);
        assert!((lo - g.root_diameter / 2.0).abs() < 1e-6, "{lo}");
        assert!(hi <= g.tip_diameter / 2.0 + 1e-9, "{hi}");
        // A roller on the pitch circle sits in its seat with room to spare.
        let roller = [g.pitch_diameter / 2.0, 0.0];
        let clear = points
            .iter()
            .map(|p| norm(sub(*p, roller)))
            .fold(f64::MAX, f64::min);
        assert!(clear >= spec.roller / 2.0, "{clear}");
        assert!((clear - g.seating_radius).abs() < 1e-6);
        assert_eq!(outline.loops[0].edges.len(), 4 * 18);
    }

    #[test]
    fn every_chain_makes_sprockets_from_nine_to_sixty_teeth() {
        for (name, pitch, roller) in SPROCKET_CHAINS {
            for teeth in [9, 13, 25, 60] {
                let spec = SprocketSpec {
                    pitch: *pitch,
                    roller: *roller,
                    teeth,
                    bore: 0.0,
                    ..SprocketSpec::default()
                };
                spec.outline()
                    .unwrap_or_else(|e| panic!("{name} with {teeth}: {e}"));
            }
        }
    }
}
