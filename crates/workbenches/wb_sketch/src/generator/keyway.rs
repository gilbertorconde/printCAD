//! Parallel keys and their keyways (DIN 6885-1, ISO 773): the key's
//! width and height and the keyway depths in the shaft and in the hub,
//! by the shaft's diameter; a bore keyway cut into a gear's or a
//! sprocket's bore; and the keyway generator, the key's slot drawn to
//! pocket into a shaft.

use serde::{Deserialize, Serialize};

use super::{Edge, Loop, Outline, P2};

/// A parallel key's sizes: its width and height, how deep its keyway
/// goes into the shaft (`shaft_depth`) and into the hub (`hub_depth`), mm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeySize {
    pub width: f64,
    pub height: f64,
    pub shaft_depth: f64,
    pub hub_depth: f64,
}

/// The standard keys: for shafts over the first diameter up to the
/// second, width, height, shaft depth and hub depth.
const KEYS: &[(f64, f64, f64, f64, f64, f64)] = &[
    (6.0, 8.0, 2.0, 2.0, 1.2, 1.0),
    (8.0, 10.0, 3.0, 3.0, 1.8, 1.4),
    (10.0, 12.0, 4.0, 4.0, 2.5, 1.8),
    (12.0, 17.0, 5.0, 5.0, 3.0, 2.3),
    (17.0, 22.0, 6.0, 6.0, 3.5, 2.8),
    (22.0, 30.0, 8.0, 7.0, 4.0, 3.3),
    (30.0, 38.0, 10.0, 8.0, 5.0, 3.3),
    (38.0, 44.0, 12.0, 8.0, 5.0, 3.3),
    (44.0, 50.0, 14.0, 9.0, 5.5, 3.8),
    (50.0, 58.0, 16.0, 10.0, 6.0, 4.3),
    (58.0, 65.0, 18.0, 11.0, 7.0, 4.4),
    (65.0, 75.0, 20.0, 12.0, 7.5, 4.9),
    (75.0, 85.0, 22.0, 14.0, 9.0, 5.4),
    (85.0, 95.0, 25.0, 14.0, 9.0, 5.4),
    (95.0, 110.0, 28.0, 16.0, 10.0, 6.4),
    (110.0, 130.0, 32.0, 18.0, 11.0, 7.4),
    (130.0, 150.0, 36.0, 20.0, 12.0, 8.4),
    (150.0, 170.0, 40.0, 22.0, 13.0, 9.4),
    (170.0, 200.0, 45.0, 25.0, 15.0, 10.4),
    (200.0, 230.0, 50.0, 28.0, 17.0, 11.4),
];

/// The standard key for a shaft of diameter `d`, when the standard has
/// one: shafts from 6 to 230 mm.
pub fn standard(d: f64) -> Option<KeySize> {
    KEYS.iter()
        .find(|(over, to, ..)| d > *over - 1e-9 && d <= *to + 1e-9)
        .map(|&(_, _, width, height, shaft_depth, hub_depth)| KeySize {
            width,
            height,
            shaft_depth,
            hub_depth,
        })
}

/// A keyway in a bore: off, or its width and how far it goes past the
/// bore, each 0 for the standard key's size by the bore.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BoreKeyway {
    pub on: bool,
    /// Width, mm; 0 takes the standard key's.
    pub width: f64,
    /// How far the keyway's floor stands beyond the bore, at its middle,
    /// mm; 0 takes the standard hub depth.
    pub depth: f64,
}

impl BoreKeyway {
    /// The keyway's width and depth in a bore of diameter `bore`.
    pub fn size(&self, bore: f64) -> Result<(f64, f64), String> {
        let key = standard(bore);
        let pick = |given: f64, standard: Option<f64>, what: &str| {
            if given > 0.0 {
                Ok(given)
            } else {
                standard.ok_or_else(|| {
                    format!(
                        "no standard key fits a {bore} mm bore (6 to 230 mm): give the \
                         keyway's {what}"
                    )
                })
            }
        };
        let width = pick(self.width, key.map(|k| k.width), "width")?;
        let depth = pick(self.depth, key.map(|k| k.hub_depth), "depth")?;
        if width >= bore {
            return Err("the keyway is wider than the bore".into());
        }
        Ok((width, depth))
    }

    /// The bore of diameter `bore` with the keyway cut into it, its
    /// floor toward +y: a loop to stand as the hole in the part, which
    /// must reach no further than `limit` from the centre.
    pub fn bore_loop(&self, bore: f64, limit: f64) -> Result<Loop, String> {
        let (width, depth) = self.size(bore)?;
        let r = bore / 2.0;
        let half = width / 2.0;
        let floor = r + depth;
        if floor.hypot(half) >= limit {
            return Err("the keyway runs out through the part: make it shallower".into());
        }
        let side = (r * r - half * half).sqrt();
        let round = || Edge::Arc { center: [0.0, 0.0] };
        // Each joint with the edge that leaves it; the bore's long way
        // round is two arcs, each the short way.
        let lp = ring(vec![
            ([half, side], Edge::Line),
            ([half, floor], Edge::Line),
            ([-half, floor], Edge::Line),
            ([-half, side], round()),
            ([0.0, -r], round()),
        ]);
        Ok(lp)
    }
}

/// A loop of joints, each with the edge that leaves it for the next.
fn ring(pairs: Vec<(P2, Edge)>) -> Loop {
    let (joints, edges) = pairs.into_iter().unzip();
    Loop { joints, edges }
}

/// The keyway generator: the slot of a parallel key in a shaft, to pocket
/// as deep as the standard's shaft depth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeywaySpec {
    /// The shaft's diameter, mm: it sizes the standard key.
    pub shaft: f64,
    /// The slot's length end to end, mm.
    pub length: f64,
    /// Width, mm; 0 takes the standard key's.
    pub width: f64,
    /// How deep to pocket it, mm; 0 takes the standard shaft depth.
    pub depth: f64,
    /// Round ends (a key of form A); false leaves them square (form B).
    pub rounded: bool,
}

impl Default for KeywaySpec {
    fn default() -> Self {
        Self {
            shaft: 20.0,
            length: 25.0,
            width: 0.0,
            depth: 0.0,
            rounded: true,
        }
    }
}

impl KeywaySpec {
    /// The slot's width and the depth to pocket it.
    pub fn size(&self) -> Result<(f64, f64), String> {
        let d = self.shaft;
        if !(d > 0.0 && d.is_finite()) {
            return Err("the shaft's diameter must be more than zero".into());
        }
        let key = standard(d);
        let pick = |given: f64, standard: Option<f64>, what: &str| {
            if given > 0.0 {
                Ok(given)
            } else {
                standard.ok_or_else(|| {
                    format!(
                        "no standard key fits a {d} mm shaft (6 to 230 mm): give the \
                         keyway's {what}"
                    )
                })
            }
        };
        let width = pick(self.width, key.map(|k| k.width), "width")?;
        let depth = pick(self.depth, key.map(|k| k.shaft_depth), "depth")?;
        if width >= d {
            return Err("the keyway is wider than the shaft".into());
        }
        if depth >= d / 2.0 {
            return Err("the keyway goes past the shaft's middle".into());
        }
        Ok((width, depth))
    }

    /// The slot, centred on the origin and running along x.
    pub fn outline(&self) -> Result<Outline, String> {
        let (width, _) = self.size()?;
        let length = self.length;
        if !(length > width && length.is_finite()) {
            return Err("the keyway must be longer than it is wide".into());
        }
        let (half_l, half_w) = (length / 2.0, width / 2.0);
        let lp = if self.rounded {
            let c = half_l - half_w;
            let end = |x: f64| Edge::Arc { center: [x, 0.0] };
            ring(vec![
                ([-c, -half_w], Edge::Line),
                ([c, -half_w], end(c)),
                ([half_l, 0.0], end(c)),
                ([c, half_w], Edge::Line),
                ([-c, half_w], end(-c)),
                ([-half_l, 0.0], end(-c)),
            ])
        } else {
            ring(
                [
                    [-half_l, -half_w],
                    [half_l, -half_w],
                    [half_l, half_w],
                    [-half_l, half_w],
                ]
                .into_iter()
                .map(|p: P2| (p, Edge::Line))
                .collect(),
            )
        };
        Ok(Outline {
            loops: vec![lp],
            ..Outline::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn the_standard_key_follows_the_shaft_diameter() {
        let key = standard(20.0).unwrap();
        assert_eq!((key.width, key.height), (6.0, 6.0));
        assert_eq!((key.shaft_depth, key.hub_depth), (3.5, 2.8));
        // A range includes its upper end.
        assert_eq!(standard(22.0).unwrap().width, 6.0);
        assert_eq!(standard(22.5).unwrap().width, 8.0);
        assert!(standard(5.0).is_none() && standard(240.0).is_none());
    }

    #[test]
    fn a_keyway_slot_is_as_wide_as_the_key() {
        let rounded = KeywaySpec::default();
        let (width, depth) = rounded.size().unwrap();
        assert_eq!((width, depth), (6.0, 3.5));
        let area = rounded.outline().unwrap().area();
        let want = (25.0 - 6.0) * 6.0 + PI * 9.0;
        assert!((area - want).abs() < 1e-2, "{area} vs {want}");
        let square = KeywaySpec {
            rounded: false,
            ..KeywaySpec::default()
        };
        assert!((square.outline().unwrap().area() - 150.0).abs() < 1e-9);
    }

    #[test]
    fn a_shaft_without_a_standard_key_needs_its_sizes() {
        let small = KeywaySpec {
            shaft: 4.0,
            length: 8.0,
            ..KeywaySpec::default()
        };
        assert!(small.size().unwrap_err().contains("width"));
        let given = KeywaySpec {
            width: 1.5,
            depth: 0.75,
            ..small
        };
        assert_eq!(given.size().unwrap(), (1.5, 0.75));
    }

    #[test]
    fn a_keyway_through_the_part_is_refused() {
        let keyway = BoreKeyway {
            on: true,
            depth: 10.0,
            ..BoreKeyway::default()
        };
        assert!(
            keyway
                .bore_loop(20.0, 15.0)
                .unwrap_err()
                .contains("through")
        );
    }
}
