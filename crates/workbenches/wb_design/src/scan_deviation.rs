//! A body against the scans it is being modelled from: each scan point's
//! signed distance to the body's solid, shown while modelling. Points on
//! the surface within the range ramp from blue (inside) through green to
//! red (outside), points deeper inside the body draw violet (material
//! the scan does not have: a hole or pocket still to cut), and points
//! farther outside stay plain.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use core_document::{BodyId, Document};
use kernel_api::{DistanceRequest, DistanceTarget, KernelQueries};
use serde_json::{Value, json};

/// The range drawn on the ramp, mm, when none is given.
pub const RANGE: f64 = 0.1;

/// What a scan is measured against: its points and normals in the
/// body's frame, the body's solid, and a key that changes with any of
/// them.
pub struct Inputs {
    points: Vec<[f64; 3]>,
    normals: Vec<[f64; 3]>,
    solid: Arc<Vec<u8>>,
    pub key: u64,
}

/// What `scan` is measured from against `body` in `document` now.
pub fn inputs(document: &Document, scan: BodyId, body: BodyId) -> Result<Inputs, String> {
    let (mesh, _) = document
        .local_geometry(scan)
        .ok_or("the scan has no mesh")?;
    let solid = document
        .imported_brep_blob_arc(body)
        .ok_or("the body has no solid yet")?;
    let to = document
        .body_placement(body)
        .inverse()
        .after(&document.body_placement(scan));
    let points = mesh
        .positions
        .iter()
        .map(|p| to.point(*p).map(f64::from))
        .collect();
    let normals = if mesh.normals.len() == mesh.positions.len() {
        mesh.normals
            .iter()
            .map(|n| to.direction(*n).map(f64::from))
            .collect()
    } else {
        Vec::new()
    };
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let revision = |b| document.imported_geometry(b).map(|g| g.revision);
        (scan, body, revision(scan), revision(body)).hash(&mut h);
        format!("{:?}", to.rows()).hash(&mut h);
        h.finish()
    };
    Ok(Inputs {
        points,
        normals,
        solid,
        key,
    })
}

/// Each scan point's signed distance to the body, outside positive.
pub fn measure(kernel: &dyn KernelQueries, inputs: &Inputs) -> Result<Arc<[f32]>, String> {
    let request = DistanceRequest {
        points: inputs.points.clone(),
        normals: inputs.normals.clone(),
        max: None,
        facing_limit_deg: None,
    };
    kernel
        .signed_distance(DistanceTarget::Shape(&inputs.solid), &request)
        .map(|d| d.into_iter().map(|v| v as f32).collect())
        .map_err(|e| e.to_string())
}

/// The shares of the measured points on the surface (within `range`),
/// deeper inside the body and farther outside, and the RMS of those on
/// the surface.
pub fn reading(values: &[f32], range: f64) -> Value {
    let measured: Vec<f64> = values
        .iter()
        .map(|v| f64::from(*v))
        .filter(|v| v.is_finite())
        .collect();
    let n = measured.len().max(1) as f64;
    let on: Vec<f64> = measured
        .iter()
        .copied()
        .filter(|v| v.abs() <= range)
        .collect();
    let rms = if on.is_empty() {
        0.0
    } else {
        (on.iter().map(|v| v * v).sum::<f64>() / on.len() as f64).sqrt()
    };
    json!({
        "within": on.len() as f64 / n,
        "inside": measured.iter().filter(|v| **v < -range).count() as f64 / n,
        "outside": measured.iter().filter(|v| **v > range).count() as f64 / n,
        "rms": rms,
        "measured": measured.len(),
    })
}

/// The values a map draws: farther outside than `range` left unmeasured,
/// so plain.
pub fn drawn(values: &[f32], range: f64) -> Arc<[f32]> {
    values
        .iter()
        .map(|v| if f64::from(*v) > range { f32::NAN } else { *v })
        .collect()
}

/// Each scan's distances by what they were measured from, and those
/// being measured.
#[derive(Default)]
pub struct Maps {
    made: HashMap<BodyId, (u64, Arc<[f32]>)>,
    running: HashMap<BodyId, u64>,
}

/// The distances of `scan` for `key`, or `None` while they are measured
/// on a thread of their own, started here when they are not.
pub fn values(
    shared: &Arc<Mutex<Maps>>,
    kernel: &'static dyn KernelQueries,
    scan: BodyId,
    inputs: Inputs,
) -> Option<Arc<[f32]>> {
    let key = inputs.key;
    let mut maps = shared.lock().ok()?;
    if let Some((k, v)) = maps.made.get(&scan)
        && *k == key
    {
        return Some(v.clone());
    }
    if maps.running.get(&scan) != Some(&key) {
        maps.running.insert(scan, key);
        let shared = Arc::clone(shared);
        std::thread::spawn(move || {
            let values = measure(kernel, &inputs).unwrap_or_else(|_| Arc::from([]));
            if let Ok(mut maps) = shared.lock() {
                if maps.running.get(&scan) == Some(&key) {
                    maps.running.remove(&scan);
                }
                maps.made.insert(scan, (key, values));
            }
        });
    }
    None
}

/// Whether a measuring runs.
pub fn busy(shared: &Arc<Mutex<Maps>>) -> bool {
    shared.lock().is_ok_and(|m| !m.running.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Points on the surface count within the range, deeper inside and
    /// farther outside apart; the map leaves the outside plain.
    #[test]
    fn a_reading_splits_the_surface_from_what_is_missing() {
        let values = [0.05, -0.05, -0.5, 0.4, f32::NAN];
        let r = reading(&values, 0.1);
        assert_eq!(r["measured"], 4);
        assert!((r["within"].as_f64().unwrap() - 0.5).abs() < 1e-12);
        assert!((r["inside"].as_f64().unwrap() - 0.25).abs() < 1e-12);
        assert!((r["outside"].as_f64().unwrap() - 0.25).abs() < 1e-12);
        let d = drawn(&values, 0.1);
        assert!(d[3].is_nan() && d[2] == -0.5);
    }
}
