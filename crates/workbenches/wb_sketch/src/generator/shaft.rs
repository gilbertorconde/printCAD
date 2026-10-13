//! A stepped shaft: sections of a length and a diameter, one after the
//! other along the sketch's vertical axis, drawn as the half section a
//! Revolution turns about that axis (the Revolution's default).
//!
//! Each step can break its outer edge with a chamfer and round its inner
//! corner with a fillet; the far end takes the last section's chamfer (or
//! its fillet), the near end the shaft's own chamfer.

use serde::{Deserialize, Serialize};

use super::{Edge, Loop, Outline, P2, add, norm, scale, sub};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShaftSection {
    /// Along the axis, mm.
    pub length: f64,
    pub diameter: f64,
    /// The chamfer on the outer edge of the step after this section (the
    /// last section: on the shaft's end), mm.
    pub chamfer: f64,
    /// The fillet in the inner corner of the step after this section (the
    /// last section: on the shaft's end when it has no chamfer), mm.
    pub fillet: f64,
}

impl Default for ShaftSection {
    fn default() -> Self {
        Self {
            length: 20.0,
            diameter: 10.0,
            chamfer: 0.0,
            fillet: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShaftSpec {
    pub sections: Vec<ShaftSection>,
    /// The chamfer on the outer edge of the shaft's first end, mm.
    pub start_chamfer: f64,
    /// What the shaft carries, for its stresses and deflection; nothing
    /// in its shape follows from them.
    pub loads: super::loads::ShaftLoads,
}

impl Default for ShaftSpec {
    fn default() -> Self {
        let section = |length, diameter, chamfer, fillet| ShaftSection {
            length,
            diameter,
            chamfer,
            fillet,
        };
        Self {
            sections: vec![
                section(15.0, 8.0, 0.0, 0.5),
                section(30.0, 12.0, 0.5, 1.0),
                section(20.0, 10.0, 0.5, 0.0),
            ],
            start_chamfer: 0.5,
            loads: Default::default(),
        }
    }
}

/// What a corner of the half section is given.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Corner {
    Sharp,
    Chamfer(f64),
    Fillet(f64),
}

impl ShaftSpec {
    /// The shaft's volume, as its sections' cylinders make it with every
    /// corner sharp.
    pub fn plain_volume(&self) -> f64 {
        self.sections
            .iter()
            .map(|s| {
                let r = s.diameter / 2.0;
                std::f64::consts::PI * r * r * s.length
            })
            .sum()
    }

    /// The half section's corners, from the axis at the near end round to
    /// the axis at the far end, each with what it is given.
    fn corners(&self) -> Result<Vec<(P2, Corner)>, String> {
        if self.sections.is_empty() {
            return Err("a shaft needs at least one section".into());
        }
        for (i, s) in self.sections.iter().enumerate() {
            if !(s.length > 0.0 && s.diameter > 0.0) {
                return Err(format!(
                    "section {} needs a length and a diameter more than zero",
                    i + 1
                ));
            }
            if s.chamfer < 0.0 || s.fillet < 0.0 {
                return Err(format!(
                    "section {}'s chamfer and fillet cannot be negative",
                    i + 1
                ));
            }
        }
        let size = |v: f64| v;
        let given = |v: f64, make: fn(f64) -> Corner| {
            if v > 0.0 {
                make(size(v))
            } else {
                Corner::Sharp
            }
        };
        let mut out = vec![([0.0, 0.0], Corner::Sharp)];
        let mut y = 0.0;
        let first = &self.sections[0];
        out.push((
            [size(first.diameter) / 2.0, 0.0],
            given(self.start_chamfer, Corner::Chamfer),
        ));
        for (i, s) in self.sections.iter().enumerate() {
            let r = size(s.diameter) / 2.0;
            y += size(s.length);
            match self.sections.get(i + 1) {
                Some(next) => {
                    let r2 = size(next.diameter) / 2.0;
                    if (r2 - r).abs() < 1e-9 {
                        // No step: the sections run on as one.
                        continue;
                    }
                    let outer = given(s.chamfer, Corner::Chamfer);
                    let inner = given(s.fillet, Corner::Fillet);
                    // Stepping down, the outer edge comes first; stepping
                    // up, the inner corner does.
                    if r2 < r {
                        out.push(([r, y], outer));
                        out.push(([r2, y], inner));
                    } else {
                        out.push(([r, y], inner));
                        out.push(([r2, y], outer));
                    }
                }
                None => {
                    let end = if s.chamfer > 0.0 {
                        Corner::Chamfer(size(s.chamfer))
                    } else {
                        given(s.fillet, Corner::Fillet)
                    };
                    out.push(([r, y], end));
                    out.push(([0.0, y], Corner::Sharp));
                }
            }
        }
        Ok(out)
    }

    pub fn outline(&self) -> Result<Outline, String> {
        let corners = self.corners()?;
        let n = corners.len();
        let unit = |a: P2| scale(a, 1.0 / norm(a));
        let mut lp = Loop::default();
        for i in 0..n {
            let (v, corner) = corners[i];
            let prev = corners[(i + n - 1) % n].0;
            let next = corners[(i + 1) % n].0;
            let size = match corner {
                Corner::Sharp => 0.0,
                Corner::Chamfer(s) | Corner::Fillet(s) => s,
            };
            // A corner takes at most half of either side, so the corners
            // at both ends of a side never overlap.
            let room = norm(sub(v, prev)).min(norm(sub(next, v))) / 2.0;
            let t = size.min(room);
            if t < 1e-6 {
                lp.joints.push(v);
                lp.edges.push(Edge::Line);
                continue;
            }
            let into = add(v, scale(unit(sub(prev, v)), t));
            let out = add(v, scale(unit(sub(next, v)), t));
            lp.joints.push(into);
            lp.edges.push(match corner {
                // Every corner is square: the fillet's centre is the
                // square's fourth corner.
                Corner::Fillet(_) => Edge::Arc {
                    center: sub(add(into, out), v),
                },
                _ => Edge::Line,
            });
            lp.joints.push(out);
            lp.edges.push(Edge::Line);
        }
        // Two corners that each took half of the side between them meet:
        // the side is gone.
        let mut i = 0;
        while i < lp.joints.len() {
            let next = lp.joints[(i + 1) % lp.joints.len()];
            if lp.edges[i] == Edge::Line && norm(sub(next, lp.joints[i])) < 1e-9 {
                lp.joints.remove(i);
                lp.edges.remove(i);
            } else {
                i += 1;
            }
        }
        Ok(Outline {
            loops: vec![lp],
            ..Outline::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::shoelace;

    #[test]
    fn a_plain_shaft_is_its_sections_side_by_side() {
        let spec = ShaftSpec {
            loads: Default::default(),
            sections: vec![
                ShaftSection {
                    length: 10.0,
                    diameter: 6.0,
                    ..ShaftSection::default()
                },
                ShaftSection {
                    length: 5.0,
                    diameter: 10.0,
                    ..ShaftSection::default()
                },
            ],
            start_chamfer: 0.0,
        };
        let outline = spec.outline().unwrap();
        let lp = &outline.loops[0];
        assert_eq!(
            lp.joints,
            vec![
                [0.0, 0.0],
                [3.0, 0.0],
                [3.0, 10.0],
                [5.0, 10.0],
                [5.0, 15.0],
                [0.0, 15.0]
            ]
        );
        assert!((shoelace(&lp.polyline(1)).abs() - (3.0 * 10.0 + 5.0 * 5.0)).abs() < 1e-9);
    }

    #[test]
    fn a_chamfer_takes_its_triangle_and_a_fillet_adds_its_corner() {
        let one = |chamfer, fillet| ShaftSpec {
            loads: Default::default(),
            sections: vec![
                ShaftSection {
                    length: 10.0,
                    diameter: 10.0,
                    chamfer,
                    fillet,
                },
                ShaftSection {
                    length: 10.0,
                    diameter: 6.0,
                    ..ShaftSection::default()
                },
            ],
            start_chamfer: 0.0,
        };
        let area =
            |spec: ShaftSpec| shoelace(&spec.outline().unwrap().loops[0].polyline(256)).abs();
        let plain = area(one(0.0, 0.0));
        assert!((plain - area(one(1.0, 0.0)) - 0.5).abs() < 1e-9);
        // The inner fillet fills the corner outside a quarter circle.
        let filled = area(one(0.0, 1.0)) - plain;
        assert!(
            (filled - (1.0 - std::f64::consts::PI / 4.0)).abs() < 1e-4,
            "{filled}"
        );
    }

    #[test]
    fn corners_bigger_than_their_sides_are_cut_to_fit() {
        let spec = ShaftSpec {
            loads: Default::default(),
            sections: vec![ShaftSection {
                length: 4.0,
                diameter: 20.0,
                chamfer: 50.0,
                fillet: 0.0,
            }],
            start_chamfer: 50.0,
        };
        let lp = &spec.outline().unwrap().loops[0];
        let points = lp.polyline(1);
        assert!(shoelace(&points).abs() > 0.0);
        assert!(points.iter().all(|p| p[1] >= 0.0 && p[1] <= 4.0));
    }

    #[test]
    fn corners_that_meet_leave_no_side_of_no_length() {
        // The default's last step is 1 mm high, its chamfer and fillet
        // each take half of it.
        let lp = &ShaftSpec::default().outline().unwrap().loops[0];
        let n = lp.joints.len();
        for i in 0..n {
            let gap = norm(sub(lp.joints[(i + 1) % n], lp.joints[i]));
            assert!(gap > 1e-6, "side {i} has no length");
        }
    }

    #[test]
    fn a_section_without_a_length_is_refused() {
        let mut spec = ShaftSpec::default();
        spec.sections[1].length = 0.0;
        assert!(spec.outline().unwrap_err().contains("section 2"));
    }
}
