//! Hot spots: where a scan stands out of tolerance, as patches. The
//! vertices beyond the tolerance are grouped with their neighbours across
//! the mesh's edges, those on one side together, and each patch is read for
//! where it lies, how large it is and how far it goes.

use kernel_api::TriMesh;
use serde::Serialize;

/// A patch at most this many times as wide as it is deep is a feature
/// (a boss, a dent); flatter, it is texture (layer lines, a rough skin).
const FEATURE_ASPECT: f64 = 40.0;

/// One patch out of tolerance.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HotSpot {
    /// Outside the reference (+1) or inside it (−1).
    pub side: i8,
    /// The patch's vertices.
    pub count: usize,
    /// Its share of the scan's surface, mm².
    pub area: f64,
    /// The area-weighted centre, in the scan's frame.
    pub centre: [f64; 3],
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub mean: f64,
    /// The farthest it goes, signed.
    pub extreme: f64,
    /// Where the extreme is.
    pub at: [f64; 3],
    /// `feature` or `texture`.
    pub kind: &'static str,
}

/// The patches of `mesh` whose `values` lie beyond ±`tolerance`, largest
/// area first, those of fewer than `least` vertices left out.
pub fn find(mesh: &TriMesh, values: &[f32], tolerance: f64, least: usize) -> Vec<HotSpot> {
    let n = mesh.positions.len().min(values.len());
    let side = |v: usize| -> i8 {
        let d = f64::from(values[v]);
        if d.is_nan() {
            0
        } else if d > tolerance {
            1
        } else if d < -tolerance {
            -1
        } else {
            0
        }
    };
    // Each vertex's share of the surface: a third of every triangle it is a
    // corner of.
    let mut share = vec![0.0f64; n];
    let p = |v: u32| mesh.positions[v as usize].map(f64::from);
    for t in mesh.indices.as_chunks::<3>().0 {
        if t.iter().any(|v| *v as usize >= n) {
            continue;
        }
        let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
        let (u, w) = (sub(b, a), sub(c, a));
        let area = 0.5 * length(cross(u, w));
        for v in t {
            share[*v as usize] += area / 3.0;
        }
    }
    // Union the out-of-tolerance vertices of one side joined by an edge,
    // and vertices at one spot (a mesh split at its creases).
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut v: usize) -> usize {
        while parent[v] != v {
            parent[v] = parent[parent[v]];
            v = parent[v];
        }
        v
    }
    let join = |parent: &mut Vec<usize>, a: usize, b: usize| {
        let (ra, rb) = (root(parent, a), root(parent, b));
        if ra != rb {
            parent[ra] = rb;
        }
    };
    for t in mesh.indices.as_chunks::<3>().0 {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let (a, b) = (a as usize, b as usize);
            if a < n && b < n && side(a) != 0 && side(a) == side(b) {
                join(&mut parent, a, b);
            }
        }
    }
    let mut at_spot: std::collections::HashMap<[u32; 3], usize> = Default::default();
    for v in (0..n).filter(|v| side(*v) != 0) {
        let key = mesh.positions[v].map(f32::to_bits);
        match at_spot.get(&key) {
            Some(&w) if side(w) == side(v) => join(&mut parent, v, w),
            _ => {
                at_spot.insert(key, v);
            }
        }
    }
    let mut groups: std::collections::HashMap<usize, Vec<usize>> = Default::default();
    for v in (0..n).filter(|v| side(*v) != 0) {
        groups.entry(root(&mut parent, v)).or_default().push(v);
    }
    let mut spots: Vec<HotSpot> = groups
        .into_values()
        .filter(|g| g.len() >= least.max(1))
        .map(|g| read(&g, mesh, values, &share, side(g[0])))
        .collect();
    spots.sort_by(|a, b| b.area.total_cmp(&a.area));
    spots
}

fn read(group: &[usize], mesh: &TriMesh, values: &[f32], share: &[f64], side: i8) -> HotSpot {
    let mut area = 0.0;
    let mut centre = [0.0; 3];
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    let mut sum = 0.0;
    let (mut extreme, mut at) = (0.0f64, [0.0; 3]);
    for &v in group {
        let q = mesh.positions[v].map(f64::from);
        let d = f64::from(values[v]);
        let w = share[v].max(1e-12);
        area += share[v];
        for i in 0..3 {
            centre[i] += q[i] * w;
            min[i] = min[i].min(q[i]);
            max[i] = max[i].max(q[i]);
        }
        sum += d;
        if d.abs() > extreme.abs() {
            (extreme, at) = (d, q);
        }
    }
    let weight: f64 = group.iter().map(|v| share[*v].max(1e-12)).sum();
    let centre = centre.map(|c| c / weight);
    let width = length(sub(max, min));
    HotSpot {
        side,
        count: group.len(),
        area,
        centre,
        min,
        max,
        mean: sum / group.len() as f64,
        extreme,
        at,
        kind: if width <= FEATURE_ASPECT * extreme.abs() {
            "feature"
        } else {
            "texture"
        },
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On a 10×10 grid of unit squares, a raised block of four vertices and
    /// a sunk single vertex make two spots, the block first and outside,
    /// each read for its size and extreme; a lone vertex is left out when
    /// spots need two.
    #[test]
    fn spots_group_by_side_and_read_their_extent() {
        let mut mesh = TriMesh::default();
        for y in 0..11 {
            for x in 0..11 {
                mesh.positions.push([x as f32, y as f32, 0.0]);
            }
        }
        let at = |x: u32, y: u32| y * 11 + x;
        for y in 0..10 {
            for x in 0..10 {
                let (a, b, c, d) = (at(x, y), at(x + 1, y), at(x + 1, y + 1), at(x, y + 1));
                mesh.indices.extend([a, b, c, a, c, d]);
            }
        }
        let mut values = vec![0.0f32; 121];
        for (x, y) in [(2, 2), (3, 2), (2, 3), (3, 3)] {
            values[at(x, y) as usize] = 0.3;
        }
        values[at(3, 3) as usize] = 0.5;
        values[at(8, 7) as usize] = -0.4;
        let spots = find(&mesh, &values, 0.1, 1);
        assert_eq!(spots.len(), 2);
        let (block, dent) = (&spots[0], &spots[1]);
        assert_eq!(
            (block.side, block.count, dent.side, dent.count),
            (1, 4, -1, 1)
        );
        assert_eq!((block.extreme, block.at), (0.5f32 as f64, [3.0, 3.0, 0.0]));
        assert_eq!((block.min, block.max), ([2.0, 2.0, 0.0], [3.0, 3.0, 0.0]));
        assert!((block.mean - (0.9 + 0.5) / 4.0).abs() < 1e-6);
        assert!((dent.area - 1.0).abs() < 1e-9, "{}", dent.area);
        assert_eq!(find(&mesh, &values, 0.1, 2).len(), 1);
    }
}
