//! Sizes read off a scan for modelling: a round hole's or boss's diameter
//! and depth from a cylinder grown where it is clicked, and the standard
//! sizes it is nearest (a tap drill or clearance hole for a hole, a
//! thread for a boss).

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
    for v in &grown.vertices {
        let Some(p) = mesh.positions.get(*v as usize) else {
            continue;
        };
        let d = std::array::from_fn::<f64, 3, _>(|k| f64::from(p[k]) - point[k]);
        let t = d[0] * axis[0] + d[1] * axis[1] + d[2] * axis[2];
        lo = lo.min(t);
        hi = hi.max(t);
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
