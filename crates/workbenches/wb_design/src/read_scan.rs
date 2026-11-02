//! Sizes read off a scan for modelling: a round hole's or boss's diameter
//! and depth from a cylinder grown where it is clicked, and the standard
//! sizes it is nearest (a tap drill or clearance hole for a hole, a
//! thread for a boss); a cylinder that turns only part of the way round
//! is a fillet, read as its radius.

/// A round turning less than this round its axis, degrees, is a fillet.
pub const WHOLE_TURN_DEG: f64 = 300.0;

use core_document::{BodyId, Document};
use kernel_api::{FitKind, FittedShape, GrowRequest, KernelQueries};
use serde_json::{Value, json};

use crate::hole_tables::ThreadStandard;

/// What a round hole or boss on the scan reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Round {
    pub diameter: f64,
    /// How far its points run along its axis.
    pub depth: f64,
    /// A point on the axis where the round starts, and the axis, in the
    /// document's frame.
    pub start: [f64; 3],
    pub axis: [f64; 3],
    pub hole: bool,
    pub sigma: f64,
    /// How far round its axis its points reach, degrees.
    pub sweep_deg: f64,
}

impl Round {
    /// Whether it turns only part of the way round: a fillet.
    pub fn fillet(&self) -> bool {
        self.sweep_deg < WHOLE_TURN_DEG
    }
}

/// How far round an axis directions `angles` (radians) reach: a full
/// turn less the widest gap between them.
pub fn sweep_deg(mut angles: Vec<f64>) -> f64 {
    if angles.len() < 2 {
        return 0.0;
    }
    angles.sort_by(f64::total_cmp);
    let mut gap = angles[0] + std::f64::consts::TAU - angles[angles.len() - 1];
    for w in angles.windows(2) {
        gap = gap.max(w[1] - w[0]);
    }
    (std::f64::consts::TAU - gap).to_degrees()
}

/// A cylinder grown on `scan` from `at` (in the document's frame).
pub fn read_round(
    document: &Document,
    kernel: &dyn KernelQueries,
    scan: BodyId,
    at: [f64; 3],
) -> Result<Round, String> {
    if document.body_scan(scan).is_none() {
        return Err("that body is not a scan".into());
    }
    let (mesh, _) = document
        .local_geometry(scan)
        .ok_or("the scan has no mesh")?;
    let placement = document.body_placement(scan);
    let local = placement
        .inverse()
        .point(at.map(|c| c as f32))
        .map(f64::from);
    let grown = kernel
        .grow(
            &mesh,
            &GrowRequest {
                kind: FitKind::Cylinder,
                seeds: vec![local],
                sigma_cutoff: None,
                normal_limit_deg: None,
                peel: false,
                max_triangles: None,
            },
        )
        .map_err(|e| e.to_string())?;
    if grown.seeds_in_region.first() == Some(&false) {
        return Err("the surface there is not round".into());
    }
    let FittedShape::Cylinder {
        point,
        axis,
        radius,
    } = grown.fit.shape
    else {
        return Err("the surface there is not round".into());
    };
    let (mut lo, mut hi, mut facing) = (f64::INFINITY, f64::NEG_INFINITY, 0.0);
    let u = {
        let other = if axis[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let c = [
            axis[1] * other[2] - axis[2] * other[1],
            axis[2] * other[0] - axis[0] * other[2],
            axis[0] * other[1] - axis[1] * other[0],
        ];
        let len = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
        c.map(|x| x / len)
    };
    let w = [
        axis[1] * u[2] - axis[2] * u[1],
        axis[2] * u[0] - axis[0] * u[2],
        axis[0] * u[1] - axis[1] * u[0],
    ];
    let mut angles = Vec::with_capacity(grown.vertices.len());
    for v in &grown.vertices {
        let Some(p) = mesh.positions.get(*v as usize) else {
            continue;
        };
        let d = std::array::from_fn::<f64, 3, _>(|k| f64::from(p[k]) - point[k]);
        let t = d[0] * axis[0] + d[1] * axis[1] + d[2] * axis[2];
        lo = lo.min(t);
        hi = hi.max(t);
        let (x, y) = (
            (0..3).map(|k| d[k] * u[k]).sum::<f64>(),
            (0..3).map(|k| d[k] * w[k]).sum::<f64>(),
        );
        angles.push(y.atan2(x));
        if let Some(n) = mesh.normals.get(*v as usize) {
            let out: [f64; 3] = std::array::from_fn(|k| d[k] - t * axis[k]);
            facing += (0..3).map(|k| f64::from(n[k]) * out[k]).sum::<f64>();
        }
    }
    if lo > hi {
        return Err("no scan points on the surface".into());
    }
    let start: [f64; 3] = std::array::from_fn(|k| point[k] + lo * axis[k]);
    Ok(Round {
        diameter: 2.0 * radius,
        depth: hi - lo,
        start: placement.point(start.map(|c| c as f32)).map(f64::from),
        axis: placement.direction(axis.map(|c| c as f32)).map(f64::from),
        hole: facing < 0.0,
        sigma: grown.fit.sigma,
        sweep_deg: sweep_deg(angles),
    })
}

/// The standard sizes nearest `diameter`, nearest first: for a hole the
/// tap drills and clearance holes of every thread standard, for a boss
/// the threads' major diameters.
pub fn nearest_sizes(diameter: f64, hole: bool, count: usize) -> Vec<Value> {
    let mut found: Vec<(f64, Value)> = Vec::new();
    for standard in ThreadStandard::ALL {
        for size in standard.sizes() {
            let mut offer = |what: &str, value: f64| {
                found.push((
                    (diameter - value).abs(),
                    json!({
                        "standard": standard.label(),
                        "size": size.name,
                        "as": what,
                        "diameter": value,
                        "off": diameter - value,
                    }),
                ));
            };
            if hole {
                offer("tap drill", size.tap_drill);
                if let Some([close, normal, loose]) = size.clearance {
                    offer("close clearance", close);
                    offer("normal clearance", normal);
                    offer("loose clearance", loose);
                }
            } else {
                offer("thread", size.major);
            }
        }
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.into_iter().take(count).map(|(_, v)| v).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Directions all round reach a full turn, a quarter's spread a
    /// quarter, across the ±180° seam as well.
    #[test]
    fn a_sweep_is_a_turn_less_its_widest_gap() {
        let all: Vec<f64> = (0..36).map(|k| f64::from(k) * 10f64.to_radians()).collect();
        assert!((sweep_deg(all) - 350.0).abs() < 1e-9);
        let seam = vec![
            170f64.to_radians(),
            (-170f64).to_radians(),
            (-100f64).to_radians(),
        ];
        assert!((sweep_deg(seam) - 90.0).abs() < 1e-9);
    }

    /// A 6.6 mm hole is an M6's normal clearance, a 5.0 mm one an M6's tap
    /// drill, and a 7.94 mm boss is nearest a 5/16 thread.
    #[test]
    fn a_reading_finds_the_standard_it_was_drawn_at() {
        let first = |d, hole| nearest_sizes(d, hole, 1).remove(0);
        let m6 = first(6.6, true);
        assert_eq!(
            (m6["size"].as_str(), m6["as"].as_str()),
            (Some("M6"), Some("normal clearance"))
        );
        let tap = first(5.0, true);
        assert_eq!(tap["as"], "tap drill");
        let boss = first(7.94, false);
        assert_eq!(boss["size"], "5/16-18", "{boss}");
    }
}

/// A thread read off a scan.
#[derive(Debug, Clone, PartialEq)]
pub struct Thread {
    pub pitch: f64,
    /// Twice the radius most points reach out to, and in to.
    pub major: f64,
    pub minor: f64,
    pub right_handed: bool,
    /// Inside a hole (the wall faces the axis) rather than on a bolt.
    pub internal: bool,
    /// How clearly the points follow the helix: the helix's share of the
    /// radii's spread, from 0 to 1.
    pub clarity: f64,
    pub start: [f64; 3],
    pub axis: [f64; 3],
    pub length: f64,
}

/// The pitch and hand of the helix points `(along, angle, radius)` about
/// an axis follow best, and how clearly: for each pitch tried, the
/// strength of the radii's variation with `2π·along/pitch ∓ angle`.
pub fn helix_of(points: &[(f64, f64, f64)], pitches: &[f64]) -> Option<(f64, bool, f64)> {
    if points.len() < 32 {
        return None;
    }
    let mean = points.iter().map(|p| p.2).sum::<f64>() / points.len() as f64;
    let spread =
        (points.iter().map(|p| (p.2 - mean).powi(2)).sum::<f64>() / points.len() as f64).sqrt();
    if spread <= 0.0 {
        return None;
    }
    let mut best: Option<(f64, bool, f64)> = None;
    for &pitch in pitches {
        for right in [true, false] {
            let hand = if right { 1.0 } else { -1.0 };
            let (mut re, mut im) = (0.0, 0.0);
            for (t, angle, r) in points {
                let phase = std::f64::consts::TAU * t / pitch - hand * angle;
                re += (r - mean) * phase.cos();
                im += (r - mean) * phase.sin();
            }
            // A pure sine of the spread's size scores 1.
            let score = (re * re + im * im).sqrt() * 2f64.sqrt() / (points.len() as f64 * spread);
            if best.is_none_or(|b| score > b.2) {
                best = Some((pitch, right, score));
            }
        }
    }
    best
}

/// Pitches to try first: every 0.02 mm from 0.2 to 6.5 mm.
fn pitches() -> Vec<f64> {
    (10..=325).map(|k| f64::from(k) * 0.02).collect()
}

/// The helix `points` follow: the best of `pitches()`, then refined to a
/// thousandth of a millimetre about it, on at most 6000 points.
pub fn find_helix(points: &[(f64, f64, f64)]) -> Option<(f64, bool, f64)> {
    let step = points.len().div_ceil(6000).max(1);
    let few: Vec<_> = points.iter().step_by(step).copied().collect();
    let (coarse, ..) = helix_of(&few, &pitches())?;
    let fine: Vec<f64> = (-20..=20)
        .map(|k| coarse + f64::from(k) * 0.001)
        .filter(|p| *p > 0.0)
        .collect();
    helix_of(&few, &fine)
}

/// A thread on `scan` at `at` (in the document's frame): its axis from a
/// cylinder grown over its crests and flanks, then every scan point near
/// that axis read for the helix it follows.
pub fn read_thread(
    document: &Document,
    kernel: &dyn KernelQueries,
    scan: BodyId,
    at: [f64; 3],
) -> Result<Thread, String> {
    if document.body_scan(scan).is_none() {
        return Err("that body is not a scan".into());
    }
    let (mesh, _) = document
        .local_geometry(scan)
        .ok_or("the scan has no mesh")?;
    let placement = document.body_placement(scan);
    let local = placement
        .inverse()
        .point(at.map(|c| c as f32))
        .map(f64::from);
    let grown = kernel
        .grow(
            &mesh,
            &GrowRequest {
                kind: FitKind::Cylinder,
                seeds: vec![local],
                sigma_cutoff: Some(6.0),
                normal_limit_deg: Some(80.0),
                peel: false,
                max_triangles: None,
            },
        )
        .map_err(|e| e.to_string())?;
    let FittedShape::Cylinder {
        point,
        axis,
        radius,
    } = grown.fit.shape
    else {
        return Err("no thread there: the surface is not round".into());
    };
    // The grown region's extent along the axis bounds the thread.
    let along = |p: [f64; 3]| (0..3).map(|k| (p[k] - point[k]) * axis[k]).sum::<f64>();
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for v in &grown.vertices {
        if let Some(p) = mesh.positions.get(*v as usize) {
            let t = along(p.map(f64::from));
            lo = lo.min(t);
            hi = hi.max(t);
        }
    }
    let other = if axis[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let u = {
        let c = cross(axis, other);
        let len = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
        c.map(|x| x / len)
    };
    let w = cross(axis, u);
    // Every scan point within a third of the radius of the fitted
    // cylinder, along the grown stretch.
    let mut samples = Vec::new();
    let mut facing = 0.0;
    for (i, p) in mesh.positions.iter().enumerate() {
        let p = p.map(f64::from);
        let t = along(p);
        if t < lo || t > hi {
            continue;
        }
        let d: [f64; 3] = std::array::from_fn(|k| p[k] - point[k] - t * axis[k]);
        let r = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if (r - radius).abs() > radius / 3.0 {
            continue;
        }
        let x = (0..3).map(|k| d[k] * u[k]).sum::<f64>();
        let y = (0..3).map(|k| d[k] * w[k]).sum::<f64>();
        samples.push((t, y.atan2(x), r));
        if let Some(n) = mesh.normals.get(i) {
            facing += (0..3).map(|k| f64::from(n[k]) * d[k]).sum::<f64>();
        }
    }
    let (pitch, right_handed, clarity) =
        find_helix(&samples).ok_or("too few scan points on the thread")?;
    let radii: Vec<f64> = samples.iter().map(|s| s.2).collect();
    let pct = |q| metrology::stats::percentile(&radii, q).unwrap_or(radius);
    let start: [f64; 3] = std::array::from_fn(|k| point[k] + lo * axis[k]);
    Ok(Thread {
        pitch,
        major: 2.0 * pct(0.97),
        minor: 2.0 * pct(0.03),
        right_handed,
        internal: facing < 0.0,
        clarity,
        start: placement.point(start.map(|c| c as f32)).map(f64::from),
        axis: placement.direction(axis.map(|c| c as f32)).map(f64::from),
        length: hi - lo,
    })
}

/// The standard threads nearest a reading, nearest first: by major
/// diameter and pitch, each off by its share of the size.
pub fn nearest_threads(major: f64, pitch: f64, count: usize) -> Vec<Value> {
    let mut found: Vec<(f64, Value)> = Vec::new();
    for standard in ThreadStandard::ALL {
        for size in standard.sizes() {
            let off = ((major - size.major) / size.major).abs()
                + ((pitch - size.pitch) / size.pitch).abs();
            found.push((
                off,
                json!({
                    "standard": standard.label(),
                    "size": size.name,
                    "major": size.major,
                    "pitch": size.pitch,
                }),
            ));
        }
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.into_iter().take(count).map(|(_, v)| v).collect()
}

#[cfg(test)]
mod thread_tests {
    use super::*;

    /// Points on a right-hand thread of pitch 1.25 read that pitch and
    /// hand, not half or twice it, and a left-hand one reads left.
    #[test]
    fn a_helix_reads_its_pitch_and_hand() {
        for (pitch, right) in [(1.25, true), (1.5, false)] {
            let hand = if right { 1.0 } else { -1.0 };
            let mut points = Vec::new();
            for i in 0..4000 {
                let t = f64::from(i) * 0.003;
                let angle =
                    (f64::from(i) * 2.399).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
                // A triangular profile, 0.6 mm deep.
                let phase = (t / pitch - hand * angle / std::f64::consts::TAU).rem_euclid(1.0);
                let r = 4.0 - 0.6 * (2.0 * phase - 1.0).abs();
                points.push((t, angle, r));
            }
            let (p, r, clarity) = find_helix(&points).unwrap();
            assert!((p - pitch).abs() < 0.002 && r == right, "{p} {r}");
            assert!(clarity > 0.8, "{clarity}");
        }
        let m8 = nearest_threads(7.9, 1.26, 1).remove(0);
        assert_eq!(m8["size"], "M8");
    }
}
