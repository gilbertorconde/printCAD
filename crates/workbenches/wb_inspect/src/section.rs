//! Sections of a scan by a plane: one cut, a stack of parallel cuts, or a
//! half-section through an axis. The plane is kept in the scan's own frame,
//! fixed when the section is made, so aligning the scan carries it and
//! refitting the element it was taken from does not move it. The chains
//! are worked out from the scan's mesh whenever it or the section changes,
//! never stored.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use core_document::{
    BodyId, DocumentError, DocumentResult, FeatureId, WorkbenchFeature, WorkbenchId,
};
use kernel_api::{KernelQueries, SectionChain, TriMesh};
use serde::{Deserialize, Serialize};

use crate::element::{add, cross, dot, scale, sub};

/// The feature kind of every section.
pub const KIND: &str = "wb.inspect.section";

/// Chains shorter than this, in mm, are dropped as noise.
pub const SHORTEST: f64 = 1.0;

/// One section of a scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub name: String,
    pub scan: BodyId,
    /// The plane at offset zero, in the scan's frame.
    pub origin: [f64; 3],
    pub normal: [f64; 3],
    /// Where the first cut is along the normal from `origin`, mm.
    #[serde(default)]
    pub offset: f64,
    /// How many cuts, `step` apart.
    #[serde(default = "one")]
    pub count: u32,
    #[serde(default = "five")]
    pub step: f64,
    /// For a half-section through an axis, the direction in the plane
    /// from the axis the half lies along; the chains are kept on its side.
    #[serde(default)]
    pub half: Option<[f64; 3]>,
}

fn one() -> u32 {
    1
}

fn five() -> f64 {
    5.0
}

impl WorkbenchFeature for Section {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(KIND)
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    fn from_json(value: &serde_json::Value) -> DocumentResult<Self> {
        serde_json::from_value(value.clone()).map_err(DocumentError::Serialization)
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        Vec::new()
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl Section {
    /// Each cut's distance from `origin` along the normal.
    pub fn offsets(&self) -> Vec<f64> {
        (0..self.count.clamp(1, 200))
            .map(|i| self.offset + f64::from(i) * self.step)
            .collect()
    }
}

/// One cut of a section and what it reads.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cut {
    pub offset: f64,
    pub chains: Vec<SectionChain>,
}

impl Cut {
    pub fn closed(&self) -> usize {
        self.chains.iter().filter(|c| c.closed).count()
    }

    /// The chains' length together, mm.
    pub fn length(&self) -> f64 {
        self.chains.iter().map(length).sum()
    }

    /// The material's area within the closed chains, mm²: outer loops
    /// count, holes take away.
    pub fn area(&self, normal: [f64; 3]) -> f64 {
        self.chains
            .iter()
            .filter(|c| c.closed)
            .map(|c| {
                let n = c.points.len();
                (0..n)
                    .map(|i| dot(cross(c.points[i], c.points[(i + 1) % n]), normal))
                    .sum::<f64>()
                    / 2.0
            })
            .sum::<f64>()
            .abs()
    }
}

fn length(c: &SectionChain) -> f64 {
    let n = c.points.len();
    let spans = if c.closed { n } else { n.saturating_sub(1) };
    (0..spans)
        .map(|i| {
            let d = sub(c.points[(i + 1) % n], c.points[i]);
            dot(d, d).sqrt()
        })
        .sum()
}

/// `chains` kept on the side of the half-plane through `origin` that
/// `along` points to: a chain crossing the axis is split there.
fn kept_on_side(chains: Vec<SectionChain>, origin: [f64; 3], along: [f64; 3]) -> Vec<SectionChain> {
    let side = |p: [f64; 3]| dot(sub(p, origin), along);
    let mut out = Vec::new();
    for c in chains {
        let n = c.points.len();
        if c.points.iter().all(|p| side(*p) >= 0.0) {
            out.push(c);
            continue;
        }
        // Walk from a point outside so every run inside starts fresh.
        let Some(start) = c.points.iter().position(|p| side(*p) < 0.0) else {
            continue;
        };
        let steps = if c.closed { n } else { n - 1 - start };
        let mut run = SectionChain::default();
        let mut flush = |run: &mut SectionChain| {
            if run.points.len() > 1 {
                out.push(std::mem::take(run));
            }
            run.points.clear();
            run.outward.clear();
        };
        if !c.closed {
            // An open chain's start, inside, before the first point outside.
            let mut head = SectionChain::default();
            for i in 0..start {
                head.points.push(c.points[i]);
                head.outward.push(c.outward[i]);
            }
            if start > 0 {
                let (a, b) = (c.points[start - 1], c.points[start]);
                let t = side(a) / (side(a) - side(b));
                head.points.push(add(a, scale(sub(b, a), t)));
                head.outward.push(c.outward[start - 1]);
            }
            flush(&mut head);
        }
        for k in 0..steps {
            let (i, j) = ((start + k) % n, (start + k + 1) % n);
            let (a, b) = (c.points[i], c.points[j]);
            let (sa, sb) = (side(a), side(b));
            if (sa < 0.0) != (sb < 0.0) {
                let t = sa / (sa - sb);
                run.points.push(add(a, scale(sub(b, a), t)));
                run.outward.push(c.outward[i]);
                if sb < 0.0 {
                    flush(&mut run);
                }
            }
            if sb >= 0.0 {
                run.points.push(b);
                run.outward.push(c.outward[j]);
            }
        }
        flush(&mut run);
    }
    out
}

/// The cuts of `scan` by `s`, chains shorter than [`SHORTEST`] dropped.
pub fn cut(kernel: &dyn KernelQueries, scan: &TriMesh, s: &Section) -> Result<Vec<Cut>, String> {
    let offsets = s.offsets();
    let stack = kernel
        .mesh_sections(scan, s.origin, s.normal, &offsets)
        .map_err(|e| e.to_string())?;
    Ok(offsets
        .into_iter()
        .zip(stack)
        .map(|(offset, chains)| {
            let chains = match s.half {
                Some(along) => kept_on_side(chains, s.origin, along),
                None => chains,
            };
            Cut {
                offset,
                chains: chains
                    .into_iter()
                    .filter(|c| length(c) >= SHORTEST)
                    .collect(),
            }
        })
        .collect())
}

/// The cuts of every section, kept by what they were cut from.
#[derive(Default)]
pub struct Cuts(Mutex<HashMap<FeatureId, Kept>>);

/// A section's cuts and the key they were cut at.
type Kept = (u64, Arc<Vec<Cut>>);

impl Cuts {
    pub fn get(&self, id: FeatureId, key: u64) -> Option<Arc<Vec<Cut>>> {
        let cuts = self.0.lock().ok()?;
        cuts.get(&id)
            .filter(|(k, _)| *k == key)
            .map(|(_, c)| c.clone())
    }

    pub fn store(&self, id: FeatureId, key: u64, cuts: Vec<Cut>) -> Arc<Vec<Cut>> {
        let cuts = Arc::new(cuts);
        if let Ok(mut all) = self.0.lock() {
            all.insert(id, (key, cuts.clone()));
        }
        cuts
    }
}

/// The section's chains as lines to draw, in the scan's frame.
pub fn lines(cuts: &[Cut]) -> TriMesh {
    let mut mesh = TriMesh::default();
    for c in cuts.iter().flat_map(|c| &c.chains) {
        let base = mesh.positions.len() as u32;
        let n = c.points.len() as u32;
        for p in &c.points {
            mesh.positions.push(p.map(|v| v as f32));
            mesh.normals.push([0.0, 0.0, 1.0]);
        }
        for i in 0..n.saturating_sub(1) {
            mesh.edges.extend([base + i, base + i + 1]);
        }
        if c.closed && n > 2 {
            mesh.edges.extend([base + n - 1, base]);
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square loop cut by the half-plane x ≥ 0 keeps one open run from
    /// where it crosses the axis on one side to where it crosses back.
    #[test]
    fn a_half_section_keeps_one_side_of_its_axis() {
        let square = SectionChain {
            points: vec![
                [-1.0, -1.0, 0.0],
                [1.0, -1.0, 0.0],
                [1.0, 1.0, 0.0],
                [-1.0, 1.0, 0.0],
            ],
            outward: vec![[0.0; 3]; 4],
            closed: true,
        };
        let kept = kept_on_side(vec![square], [0.0; 3], [1.0, 0.0, 0.0]);
        assert_eq!(kept.len(), 1);
        let run = &kept[0];
        assert!(!run.closed);
        assert_eq!(
            run.points,
            vec![
                [0.0, -1.0, 0.0],
                [1.0, -1.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0]
            ]
        );
        assert!((length(run) - 4.0).abs() < 1e-12);
        let cut = Cut {
            offset: 0.0,
            chains: kept,
        };
        assert_eq!(cut.closed(), 0);
    }
}
