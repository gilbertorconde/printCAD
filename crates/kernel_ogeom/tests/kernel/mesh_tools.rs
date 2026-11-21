//! Mesh history steps that make a mesh finer or more even or mend where
//! it crosses itself, read through the history a mesh body builds, and
//! the search for where it crosses.

use kernel_api::{KernelQueries, MeshChainAnswer, MeshOp, TriMesh};
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

fn built(mesh: &TriMesh, op: MeshOp) -> MeshChainAnswer {
    let answer = QUERIES.build_mesh_chain(mesh, &[op]).unwrap();
    assert!(answer.failed.is_none(), "{:?}", answer.failed);
    answer
}

/// A 10 mm cube remeshed to 1 mm edges keeps its shape, its sharp edges
/// and its volume, and most edges come out near 1 mm.
#[test]
#[ignore = "kernel: Editable::remesh is not wired in yet (ogeom-rs#1186)"]
fn a_cube_remeshes_to_even_edges() {
    let answer = built(
        &cube([0.0; 3], [10.0; 3]),
        MeshOp::Remesh {
            edge_length: 1.0,
            keep_edges_deg: Some(30.0),
        },
    );
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
}

/// A cube subdivided twice without smoothing is the same cube in sixteen
/// times the triangles.
#[test]
#[ignore = "kernel: Editable::subdivide is not wired in yet (ogeom-rs#1186)"]
fn a_cube_subdivides_flat_into_sixteen_times_the_triangles() {
    let answer = built(
        &cube([0.0; 3], [10.0; 3]),
        MeshOp::Subdivide {
            levels: 2,
            smooth: false,
        },
    );
    assert_eq!(answer.steps[0].faces, 12 * 16);
    assert_eq!(edges(&answer.mesh).1, 0, "closed");
    assert!((volume(&answer.mesh) - 1000.0).abs() < 1e-6);
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
#[ignore = "kernel: mesh::self_intersections is not wired in yet (ogeom-rs#1187)"]
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

/// The overlapping cubes repaired are one closed solid of their union's
/// volume that crosses nothing.
#[test]
#[ignore = "kernel: mesh::resolve_self_intersections is not wired in yet (ogeom-rs#1187)"]
fn overlapping_cubes_repair_into_their_union() {
    let answer = built(&two_cubes(false), MeshOp::RepairIntersections);
    assert_eq!(edges(&answer.mesh).1, 0, "closed");
    assert!((volume(&answer.mesh) - (2000.0 - 125.0)).abs() < 1e-6);
    assert!(QUERIES.self_intersections(&answer.mesh).unwrap().is_empty());
}
