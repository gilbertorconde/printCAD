//! Mesh history steps that make a mesh finer or more even or mend where
//! it crosses itself, read through the history a mesh body builds, and
//! the search for where it crosses.

use kernel_api::{
    KernelQueries, MESH_FACE_LIMIT, MeshChainAnswer, MeshOp, MeshRegion, MeshStepReport,
    REMESH_PASSES, TriMesh,
};
use kernel_ogeom::QUERIES;

/// A closed box from `lo` to `hi`, wound outward, as twelve triangles.
fn cube(lo: [f32; 3], hi: [f32; 3]) -> TriMesh {
    TriMesh {
        positions: (0..8)
            .map(|i| std::array::from_fn(|k| if i >> k & 1 == 1 { hi[k] } else { lo[k] }))
            .collect(),
        indices: vec![
            0, 2, 1, 1, 2, 3, 4, 5, 6, 5, 7, 6, 0, 1, 4, 1, 5, 4, 2, 6, 3, 3, 6, 7, 0, 4, 2, 2, 4,
            6, 1, 3, 5, 3, 7, 5,
        ],
        ..TriMesh::default()
    }
}

/// A sphere of `radius` about the origin in `rings` bands of `2 * rings`
/// quads, each two triangles, fans at the poles: long thin triangles near
/// the poles and wide ones at the equator, as uneven as a scan.
fn uv_sphere(radius: f32, rings: u32) -> TriMesh {
    let segments = rings * 2;
    let mut positions = vec![[0.0, 0.0, radius]];
    for r in 1..rings {
        let polar = std::f32::consts::PI * r as f32 / rings as f32;
        for s in 0..segments {
            let azimuth = std::f32::consts::TAU * s as f32 / segments as f32;
            positions.push([
                radius * polar.sin() * azimuth.cos(),
                radius * polar.sin() * azimuth.sin(),
                radius * polar.cos(),
            ]);
        }
    }
    positions.push([0.0, 0.0, -radius]);
    let south = positions.len() as u32 - 1;
    let at = |r: u32, s: u32| 1 + (r - 1) * segments + s % segments;
    let mut indices = Vec::new();
    for s in 0..segments {
        indices.extend([0, at(1, s), at(1, s + 1)]);
        indices.extend([south, at(rings - 1, s + 1), at(rings - 1, s)]);
    }
    for r in 1..rings - 1 {
        for s in 0..segments {
            let (a, b, c, d) = (at(r, s), at(r, s + 1), at(r + 1, s), at(r + 1, s + 1));
            indices.extend([a, c, b, b, c, d]);
        }
    }
    TriMesh {
        positions,
        indices,
        ..TriMesh::default()
    }
}

/// The regular icosahedron with its corners on the unit sphere, wound
/// outward.
fn icosahedron() -> TriMesh {
    let g = (1.0 + 5.0_f64.sqrt()) / 2.0;
    let n = (1.0 + g * g).sqrt();
    let pairs = [(-1.0, g), (1.0, g), (-1.0, -g), (1.0, -g)];
    let positions: Vec<[f64; 3]> = pairs
        .iter()
        .map(|&(a, b)| [a, b, 0.0])
        .chain(pairs.iter().map(|&(a, b)| [0.0, a, b]))
        .chain(pairs.iter().map(|&(a, b)| [b, 0.0, a]))
        .map(|p| p.map(|v| v / n))
        .collect();
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let close = |a: usize, b: usize| {
        let d = sub(positions[a], positions[b]);
        ((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() - 2.0 / n).abs() < 1e-9
    };
    let mut indices = Vec::new();
    for a in 0..12 {
        for b in a + 1..12 {
            for c in b + 1..12 {
                if close(a, b) && close(b, c) && close(a, c) {
                    let [pa, pb, pc] = [a, b, c].map(|i| positions[i]);
                    let (u, v) = (sub(pb, pa), sub(pc, pa));
                    let normal = [
                        u[1] * v[2] - u[2] * v[1],
                        u[2] * v[0] - u[0] * v[2],
                        u[0] * v[1] - u[1] * v[0],
                    ];
                    let out: f64 = (0..3).map(|k| normal[k] * pa[k]).sum();
                    let t = [a, b, c].map(|i| i as u32);
                    indices.extend(if out > 0.0 { t } else { [t[0], t[2], t[1]] });
                }
            }
        }
    }
    assert_eq!(indices.len(), 60);
    TriMesh {
        positions: positions.iter().map(|p| p.map(|v| v as f32)).collect(),
        indices,
        ..TriMesh::default()
    }
}

fn volume(m: &TriMesh) -> f64 {
    m.indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| m.positions[i as usize].map(f64::from));
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.0
        })
        .sum()
}

/// Each edge's length, every edge once, corners at one point joined; and
/// how many edges have only one triangle.
fn edges(m: &TriMesh) -> (Vec<f64>, usize) {
    let key = |i: u32| m.positions[i as usize].map(f32::to_bits);
    let mut count: std::collections::HashMap<_, (u32, f64)> = Default::default();
    for t in m.indices.as_chunks::<3>().0 {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let length = (0..3)
                .map(|k| {
                    (f64::from(m.positions[a as usize][k]) - f64::from(m.positions[b as usize][k]))
                        .powi(2)
                })
                .sum::<f64>()
                .sqrt();
            let (a, b) = (key(a), key(b));
            count
                .entry(if a <= b { (a, b) } else { (b, a) })
                .or_insert((0, length))
                .0 += 1;
        }
    }
    let open = count.values().filter(|(n, _)| *n == 1).count();
    (count.into_values().map(|(_, l)| l).collect(), open)
}

/// The largest over the smallest distance of a point from the origin.
fn spread(m: &TriMesh) -> f64 {
    let (lo, hi) = m
        .positions
        .iter()
        .map(|p| p.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>().sqrt())
        .fold((f64::INFINITY, 0.0_f64), |(lo, hi), r| {
            (lo.min(r), hi.max(r))
        });
    hi / lo
}

fn remesh(edge_length: f64, keep_edges_deg: Option<f64>) -> MeshOp {
    MeshOp::Remesh {
        edge_length,
        keep_edges_deg,
        passes: REMESH_PASSES,
        region: MeshRegion::All,
        face_limit: MESH_FACE_LIMIT,
    }
}

fn subdivide(levels: u32, smooth: bool, keep_edges_deg: Option<f64>) -> MeshOp {
    MeshOp::Subdivide {
        levels,
        smooth,
        keep_edges_deg,
        face_limit: MESH_FACE_LIMIT,
    }
}

fn built(mesh: &TriMesh, op: MeshOp) -> MeshChainAnswer {
    let answer = QUERIES.build_mesh_chain(mesh, &[op]).unwrap();
    assert!(answer.failed.is_none(), "{:?}", answer.failed);
    answer
}

/// The step's refusal, the mesh it was handed kept.
fn refused(mesh: &TriMesh, op: MeshOp) -> String {
    let answer = QUERIES.build_mesh_chain(mesh, &[op]).unwrap();
    let (step, why) = answer.failed.expect("refused");
    assert_eq!(step, 0);
    assert_eq!(answer.mesh.indices.len(), mesh.indices.len());
    why
}

/// A 10 mm cube remeshed to 1 mm edges keeps its shape, its sharp edges
/// and its volume, and most edges come out near 1 mm.
#[test]
fn a_cube_remeshes_to_even_edges() {
    let answer = built(&cube([0.0; 3], [10.0; 3]), remesh(1.0, Some(30.0)));
    let (lengths, open) = edges(&answer.mesh);
    assert_eq!(open, 0, "closed");
    let near = lengths.iter().filter(|l| (0.5..=1.5).contains(*l)).count();
    assert!(
        near * 100 >= lengths.len() * 95,
        "{near} of {}",
        lengths.len()
    );
    assert!((volume(&answer.mesh) - 1000.0).abs() < 1e-3);
    for p in &answer.mesh.positions {
        let on_a_side = (0..3).any(|k| p[k].abs() < 1e-4 || (p[k] - 10.0).abs() < 1e-4);
        assert!(on_a_side, "{p:?} left the surface");
    }
    let Some(MeshStepReport::Remesh { max, .. }) = answer.steps[0].report else {
        panic!("{:?}", answer.steps[0].report);
    };
    assert!(max < 1e-4, "{max} off the cube");
}

/// A sphere of slivers at its poles and wide triangles at its equator
/// remeshed to 1 mm: the edges come out within 30 % of 1 mm, their mean a
/// little under it as the split and collapse lengths (4/3 and 4/5 of it)
/// settle, and the volume and the surface stay where they were.
#[test]
fn an_uneven_sphere_remeshes_to_its_edge_length_keeping_its_volume() {
    let sphere = uv_sphere(8.0, 24);
    let answer = built(&sphere, remesh(1.0, None));
    let (lengths, open) = edges(&answer.mesh);
    assert_eq!(open, 0, "closed");
    let mean = lengths.iter().sum::<f64>() / lengths.len() as f64;
    assert!((0.8..=1.1).contains(&mean), "mean edge {mean}");
    let near = lengths.iter().filter(|l| (0.7..=1.3).contains(*l)).count();
    assert!(
        near * 100 >= lengths.len() * 95,
        "{near} of {}",
        lengths.len()
    );
    let (before, after) = (volume(&sphere), volume(&answer.mesh));
    assert!(
        (after - before).abs() < before * 5e-3,
        "{before} to {after}"
    );
    let Some(MeshStepReport::Remesh { max, rms }) = answer.steps[0].report else {
        panic!("{:?}", answer.steps[0].report);
    };
    assert!(max < 0.05 && rms <= max, "{max} {rms}");
}

/// A remesh kept to a box leaves the points below it where they were.
#[test]
fn a_remesh_in_a_region_leaves_the_rest() {
    let sphere = uv_sphere(8.0, 24);
    let answer = built(
        &sphere,
        MeshOp::Remesh {
            region: MeshRegion::Box {
                min: [-10.0, -10.0, 2.0],
                max: [10.0, 10.0, 10.0],
            },
            edge_length: 1.0,
            keep_edges_deg: None,
            passes: REMESH_PASSES,
            face_limit: MESH_FACE_LIMIT,
        },
    );
    let key = |p: &[f32; 3]| p.map(f32::to_bits);
    let kept: std::collections::HashSet<_> = answer.mesh.positions.iter().map(key).collect();
    for p in sphere.positions.iter().filter(|p| p[2] < 1.0) {
        assert!(kept.contains(&key(p)), "{p:?} moved");
    }
    assert_eq!(edges(&answer.mesh).1, 0, "closed");
}

/// A remesh to edges finer than the face limit allows is refused by name.
#[test]
fn a_remesh_past_its_face_limit_is_refused() {
    let why = refused(
        &cube([0.0; 3], [10.0; 3]),
        MeshOp::Remesh {
            edge_length: 0.1,
            keep_edges_deg: Some(30.0),
            passes: REMESH_PASSES,
            region: MeshRegion::All,
            face_limit: 1000,
        },
    );
    assert!(why.contains("1000 allowed"), "{why}");
}

/// A cube subdivided twice without smoothing is the same cube in sixteen
/// times the triangles.
#[test]
fn a_cube_subdivides_flat_into_sixteen_times_the_triangles() {
    let answer = built(&cube([0.0; 3], [10.0; 3]), subdivide(2, false, None));
    assert_eq!(answer.steps[0].faces, 12 * 16);
    assert_eq!(edges(&answer.mesh).1, 0, "closed");
    assert!((volume(&answer.mesh) - 1000.0).abs() < 1e-6);
    assert_eq!(
        answer.steps[0].report,
        Some(MeshStepReport::Subdivide { moved: 0.0 })
    );
}

/// Smooth subdivision with every cube edge kept sharp leaves the cube as
/// it is: its corners stay, its edges' points slide along them and its
/// faces' points stay in their planes.
#[test]
fn a_cube_with_sharp_edges_kept_subdivides_smooth_into_itself() {
    let answer = built(&cube([0.0; 3], [10.0; 3]), subdivide(2, true, Some(30.0)));
    assert_eq!(answer.steps[0].faces, 12 * 16);
    assert!((volume(&answer.mesh) - 1000.0).abs() < 1e-3);
}

/// An icosahedron subdivided smooth tends to a limit surface: each level
/// changes the volume by less than half what the one before did, and the
/// points come to lie at nearly one distance from the centre, where flat
/// subdivision leaves its new points well inside the corners' sphere.
#[test]
fn an_icosahedron_subdivided_smooth_converges_toward_a_round_limit() {
    let ico = icosahedron();
    let volumes: Vec<f64> = (1..=4)
        .map(|levels| {
            let answer = built(&ico, subdivide(levels, true, None));
            assert_eq!(answer.steps[0].faces, 20 * 4_usize.pow(levels));
            assert_eq!(edges(&answer.mesh).1, 0, "closed");
            volume(&answer.mesh)
        })
        .collect();
    let changes: Vec<f64> = volumes.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
    for w in changes.windows(2) {
        assert!(w[1] < w[0] * 0.5, "{volumes:?}");
    }
    let smooth = spread(&built(&ico, subdivide(3, true, None)).mesh);
    let flat = spread(&built(&ico, subdivide(3, false, None)).mesh);
    assert!(smooth < 1.03, "{smooth}");
    assert!(flat > 1.15, "{flat}");
}

/// A subdivision past its face limit is refused before any work.
#[test]
fn a_subdivision_past_its_face_limit_is_refused() {
    let why = refused(
        &cube([0.0; 3], [10.0; 3]),
        MeshOp::Subdivide {
            levels: 2,
            smooth: false,
            keep_edges_deg: None,
            face_limit: 100,
        },
    );
    assert!(why.contains("192 faces"), "{why}");
}

/// Two 10 mm cubes in one mesh: the second moved 5 mm along every axis
/// into the first, or standing apart.
fn two_cubes(apart: bool) -> TriMesh {
    let at = if apart { 20.0 } else { 5.0 };
    let mut both = cube([0.0; 3], [10.0; 3]);
    let other = cube([at; 3], [at + 10.0; 3]);
    let base = both.positions.len() as u32;
    both.indices.extend(other.indices.iter().map(|i| i + base));
    both.positions.extend(other.positions);
    both
}

/// The overlapping cubes cross each other, never themselves; cubes apart
/// cross nothing.
#[test]
fn overlapping_cubes_cross_and_cubes_apart_do_not() {
    let pairs = QUERIES.self_intersections(&two_cubes(false)).unwrap();
    assert!(!pairs.is_empty());
    for [a, b] in pairs {
        assert!(a < 12 && b >= 12, "{a} and {b} are of one cube");
    }
    assert!(
        QUERIES
            .self_intersections(&two_cubes(true))
            .unwrap()
            .is_empty()
    );
}

/// A drawn mesh splits its corners at creases for shading: the copies of
/// one corner are one corner, so neighbours across a crease do not cross,
/// and the pairs come back by the mesh's own triangle numbers.
#[test]
fn split_corners_are_one_corner_and_pairs_keep_the_mesh_numbers() {
    let welded = two_cubes(false);
    let split = TriMesh {
        positions: welded
            .indices
            .iter()
            .map(|i| welded.positions[*i as usize])
            .collect(),
        indices: (0..welded.indices.len() as u32).collect(),
        ..TriMesh::default()
    };
    assert_eq!(
        QUERIES.self_intersections(&split).unwrap(),
        QUERIES.self_intersections(&welded).unwrap()
    );
}

/// The overlapping cubes repaired are one closed solid of their union's
/// volume that crosses nothing.
#[test]
fn overlapping_cubes_repair_into_their_union() {
    let answer = built(&two_cubes(false), MeshOp::RepairIntersections);
    assert_eq!(edges(&answer.mesh).1, 0, "closed");
    assert!((volume(&answer.mesh) - (2000.0 - 125.0)).abs() < 1e-6);
    assert!(QUERIES.self_intersections(&answer.mesh).unwrap().is_empty());
    let Some(MeshStepReport::Repair { crossings, volume }) = answer.steps[0].report else {
        panic!("{:?}", answer.steps[0].report);
    };
    assert!(crossings > 0);
    assert!((volume - 1875.0).abs() < 1e-6);
}

/// A mesh that does not cross itself is left as it is by a repair, and an
/// open one that does is refused with what to do first.
#[test]
fn a_repair_leaves_a_whole_mesh_and_refuses_an_open_one() {
    let apart = two_cubes(true);
    let answer = built(&apart, MeshOp::RepairIntersections);
    assert_eq!(answer.mesh.indices.len(), apart.indices.len());
    assert!((volume(&answer.mesh) - 2000.0).abs() < 1e-6);

    let mut open = two_cubes(false);
    open.indices.truncate(open.indices.len() - 3);
    let why = refused(&open, MeshOp::RepairIntersections);
    assert!(why.contains("fill holes first"), "{why}");
}

#[cfg(feature = "heavy-tests")]
mod heavy {
    use super::*;

    /// Two spheres of thousands of triangles each, overlapping: their
    /// crossings found, and repaired into a closed union that crosses
    /// nothing, of a volume between the larger sphere's and their sum.
    #[test]
    fn overlapping_fine_spheres_repair_into_their_union() {
        let first = uv_sphere(10.0, 60);
        let second = uv_sphere(8.0, 50);
        let mut both = first.clone();
        let base = both.positions.len() as u32;
        both.indices.extend(second.indices.iter().map(|i| i + base));
        both.positions.extend(
            second
                .positions
                .iter()
                .map(|p| [p[0] + 9.0, p[1] + 0.3, p[2] + 0.2]),
        );
        assert!(!QUERIES.self_intersections(&both).unwrap().is_empty());
        let answer = built(&both, MeshOp::RepairIntersections);
        assert_eq!(edges(&answer.mesh).1, 0, "closed");
        assert!(QUERIES.self_intersections(&answer.mesh).unwrap().is_empty());
        let v = volume(&answer.mesh);
        assert!(
            v > volume(&first) && v < volume(&first) + volume(&second),
            "{v}"
        );
    }

    /// A sphere of tens of thousands of uneven triangles remeshed to
    /// 0.5 mm edges keeps its volume within a thousandth.
    #[test]
    fn a_fine_sphere_remeshes_to_its_edge_length() {
        let sphere = uv_sphere(20.0, 150);
        let answer = built(&sphere, remesh(0.5, None));
        let (lengths, open) = edges(&answer.mesh);
        assert_eq!(open, 0, "closed");
        let mean = lengths.iter().sum::<f64>() / lengths.len() as f64;
        assert!((0.4..=0.55).contains(&mean), "mean edge {mean}");
        let (before, after) = (volume(&sphere), volume(&answer.mesh));
        assert!(
            (after - before).abs() < before * 1e-3,
            "{before} to {after}"
        );
    }
}
