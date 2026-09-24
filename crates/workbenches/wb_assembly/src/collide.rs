//! Collisions while a body is dragged: whether a proposed move makes a
//! body share material with another. The kernel measures what each pair
//! near enough shares; touching faces share nothing, so mated bodies slide
//! freely, and a pair that already shared material when the drag began is
//! held only to not sharing more.

use std::collections::HashMap;

use core_document::{BodyId, BodyPlacement, Document};
use kernel_api::KernelQueries;

/// Material, in mm³, a move may add to a pair before it is a collision:
/// what a joint's rounding can leave between faces meant to touch.
const CLASH_MM3: f64 = 1e-3;

/// What each pair shared where the drag began, asked the first time the
/// pair comes near.
#[derive(Debug, Clone, Default)]
pub struct Baseline {
    start: HashMap<BodyId, BodyPlacement>,
    shared: HashMap<(BodyId, BodyId), f64>,
}

impl Baseline {
    /// Every body where it sat when the drag began.
    pub fn new(start: &[(BodyId, BodyPlacement)]) -> Self {
        Self {
            start: start.iter().copied().collect(),
            shared: HashMap::new(),
        }
    }
}

/// How finely a move from where the bodies sit to `moves` is checked,
/// as fractions of the way: steps short enough that no moved body passes
/// through another between two of them (half its smallest size, at least
/// a millimetre).
pub fn checkpoints(document: &Document, moves: &[(BodyId, BodyPlacement)]) -> Vec<f32> {
    let mut steps = 1usize;
    for (body, to) in moves {
        let Some((mesh, bounds)) = document.local_geometry(*body) else {
            continue;
        };
        let Some((lo, hi)) = bounds.or_else(|| mesh.bounds()) else {
            continue;
        };
        let smallest = (0..3).map(|k| hi[k] - lo[k]).fold(f32::MAX, f32::min);
        let reach = (smallest * 0.5).max(1.0);
        let from = document.body_placement(*body);
        let mut travel = 0.0f32;
        for corner in 0..8 {
            let p = [
                if corner & 1 == 0 { lo[0] } else { hi[0] },
                if corner & 2 == 0 { lo[1] } else { hi[1] },
                if corner & 4 == 0 { lo[2] } else { hi[2] },
            ];
            let (a, b) = (from.point(p), to.point(p));
            let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
            travel = travel.max(d);
        }
        steps = steps.max((travel / reach).ceil() as usize).min(64);
    }
    (1..=steps).map(|i| i as f32 / steps as f32).collect()
}

/// A body's box in the world, sitting at `placement`.
fn placed_bounds(
    document: &Document,
    body: BodyId,
    placement: &BodyPlacement,
) -> Option<([f32; 3], [f32; 3])> {
    let (mesh, bounds) = document.local_geometry(body)?;
    let (lo, hi) = bounds.or_else(|| mesh.bounds())?;
    let mut out = ([f32::MAX; 3], [f32::MIN; 3]);
    for corner in 0..8 {
        let p = [
            if corner & 1 == 0 { lo[0] } else { hi[0] },
            if corner & 2 == 0 { lo[1] } else { hi[1] },
            if corner & 4 == 0 { lo[2] } else { hi[2] },
        ];
        for (k, c) in placement.point(p).into_iter().enumerate() {
            out.0[k] = out.0[k].min(c);
            out.1[k] = out.1[k].max(c);
        }
    }
    Some(out)
}

/// What `a` and `b` share, in mm³, sitting at `at_a` and `at_b`.
fn shared(
    document: &Document,
    kernel: &dyn KernelQueries,
    (a, at_a): (BodyId, &BodyPlacement),
    (b, at_b): (BodyId, &BodyPlacement),
) -> Result<f64, String> {
    let (Some(blob_a), Some(blob_b)) = (
        document.imported_brep_blob(a),
        document.imported_brep_blob(b),
    ) else {
        return Ok(0.0);
    };
    let b_in_a = at_a.inverse().after(at_b).rows();
    Ok(kernel
        .overlap(blob_a, blob_b, &b_in_a)
        .map_err(|e| e.to_string())?
        .map_or(0.0, |o| o.volume_mm3))
}

/// Whether moving bodies to `moves` makes one of them share more material
/// with another visible solid than the two shared at the drag's start.
pub fn collides(
    document: &Document,
    kernel: &dyn KernelQueries,
    moves: &[(BodyId, BodyPlacement)],
    baseline: &mut Baseline,
) -> Result<bool, String> {
    let at = |body: BodyId| {
        moves
            .iter()
            .find(|(b, _)| *b == body)
            .map_or_else(|| document.body_placement(body), |(_, p)| *p)
    };
    let solids: Vec<BodyId> = document
        .bodies()
        .iter()
        .map(|b| b.id)
        .filter(|b| {
            document.imported_body_effective_visible(*b)
                && document.imported_brep_blob(*b).is_some()
        })
        .collect();
    for (moved, placement) in moves {
        if !solids.contains(moved) {
            continue;
        }
        let Some((lo_m, hi_m)) = placed_bounds(document, *moved, placement) else {
            continue;
        };
        for other in &solids {
            let other_moves = moves.iter().any(|(b, _)| b == other);
            // A pair of moved bodies is asked about once.
            if other == moved || (other_moves && other.0 < moved.0) {
                continue;
            }
            let at_other = at(*other);
            let Some((lo_o, hi_o)) = placed_bounds(document, *other, &at_other) else {
                continue;
            };
            if (0..3).any(|k| hi_m[k] < lo_o[k] || hi_o[k] < lo_m[k]) {
                continue;
            }
            let now = shared(document, kernel, (*moved, placement), (*other, &at_other))?;
            if now <= CLASH_MM3 {
                continue;
            }
            let key = if moved.0 < other.0 {
                (*moved, *other)
            } else {
                (*other, *moved)
            };
            let before = match baseline.shared.get(&key) {
                Some(v) => *v,
                None => {
                    let start = |b: BodyId| {
                        baseline
                            .start
                            .get(&b)
                            .copied()
                            .unwrap_or_else(|| document.body_placement(b))
                    };
                    let v = shared(
                        document,
                        kernel,
                        (key.0, &start(key.0)),
                        (key.1, &start(key.1)),
                    )?;
                    baseline.shared.insert(key, v);
                    v
                }
            };
            if now > before * (1.0 + 1e-3) + CLASH_MM3 {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
