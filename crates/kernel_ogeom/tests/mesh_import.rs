//! Mesh files import as bodies with no shape snapshot, and a mesh body
//! converts to a B-rep solid on request.

use kernel_api::{Kernel, TessellationSettings};
use kernel_ogeom::OgeomKernel;
use ogeom::math::{Point, Vector};
use ogeom::topo::Triangulation;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// A 10 mm cube, twelve triangles wound outward, each triangle's vertices
/// its own as an STL has them.
fn cube(skip_one: bool) -> Triangulation {
    let c = |x: f64, y: f64, z: f64| Point::new(x * 10.0, y * 10.0, z * 10.0);
    let quads = [
        [c(0., 0., 0.), c(0., 1., 0.), c(1., 1., 0.), c(1., 0., 0.)],
        [c(0., 0., 1.), c(1., 0., 1.), c(1., 1., 1.), c(0., 1., 1.)],
        [c(0., 0., 0.), c(1., 0., 0.), c(1., 0., 1.), c(0., 0., 1.)],
        [c(0., 1., 0.), c(0., 1., 1.), c(1., 1., 1.), c(1., 1., 0.)],
        [c(0., 0., 0.), c(0., 0., 1.), c(0., 1., 1.), c(0., 1., 0.)],
        [c(1., 0., 0.), c(1., 1., 0.), c(1., 1., 1.), c(1., 0., 1.)],
    ];
    let mut mesh = Triangulation::new();
    for quad in quads {
        for tri in [[quad[0], quad[1], quad[2]], [quad[0], quad[2], quad[3]]] {
            if skip_one && mesh.triangles.len() == 11 {
                continue;
            }
            let base = mesh.positions.len() as u32;
            let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]);
            let n = n / n.magnitude();
            for p in tri {
                mesh.positions.push(p);
                mesh.normals.push(Vector::new(n.x, n.y, n.z));
                mesh.parameters.push((0.0, 0.0));
            }
            mesh.triangles.push([base, base + 1, base + 2]);
        }
    }
    mesh
}

fn staged(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("printcad_{}_{name}", std::process::id()));
    std::fs::write(&path, bytes).expect("staged");
    path
}

fn import(path: &PathBuf) -> kernel_api::ImportedModel {
    let mut kernel = OgeomKernel::new();
    let model = kernel
        .import_step(path, &TessellationSettings::default())
        .expect("the mesh file imports");
    let _ = std::fs::remove_file(path);
    model
}

fn assert_is_the_cube(body: &kernel_api::ImportedBody) {
    assert!(
        body.brep_blob.is_empty(),
        "a mesh body has no shape snapshot"
    );
    assert_eq!(body.mesh.indices.len() / 3, 12);
    assert_eq!(
        body.mesh.edges.len() / 2,
        12,
        "the outline is the cube's creases"
    );
    let (lo, hi) = body.bounds_mm.expect("bounds");
    for axis in 0..3 {
        assert!((hi[axis] - lo[axis] - 10.0).abs() < 1e-4);
    }
}

#[test]
fn an_stl_imports_as_one_mesh_body_named_for_its_file() {
    let bytes = ogeom::io::stl::write(&cube(false), ogeom::io::Encoding::Binary).expect("writes");
    let model = import(&staged("cube.stl", &bytes));
    assert_eq!(model.bodies.len(), 1);
    assert!(
        model.bodies[0]
            .name
            .as_deref()
            .is_some_and(|n| n.ends_with("cube"))
    );
    assert_is_the_cube(&model.bodies[0]);
}

#[test]
fn an_obj_imports_as_one_mesh_body() {
    let mesh = cube(false);
    let mut text = String::new();
    for p in &mesh.positions {
        text.push_str(&format!("v {} {} {}\n", p.x, p.y, p.z));
    }
    for t in &mesh.triangles {
        text.push_str(&format!("f {} {} {}\n", t[0] + 1, t[1] + 1, t[2] + 1));
    }
    let model = import(&staged("cube.obj", text.as_bytes()));
    assert_eq!(model.bodies.len(), 1);
    assert_is_the_cube(&model.bodies[0]);
}

#[test]
fn a_3mf_imports_one_body_per_build_item_with_its_name() {
    let mesh = cube(false);
    let bytes = ogeom::io::write_3mf(&[ogeom::io::threemf::Object {
        mesh: &mesh,
        name: Some("Block".into()),
    }]);
    let model = import(&staged("cube.3mf", &bytes));
    assert_eq!(model.bodies.len(), 1);
    assert_eq!(model.bodies[0].name.as_deref(), Some("Block"));
    assert_is_the_cube(&model.bodies[0]);
}

#[test]
fn a_mesh_body_converts_to_a_six_faced_solid_of_the_right_volume() {
    let bytes = ogeom::io::stl::write(&cube(false), ogeom::io::Encoding::Binary).expect("writes");
    let model = import(&staged("convert.stl", &bytes));
    let mut kernel = OgeomKernel::new();
    let solid = kernel
        .mesh_to_solid(&model.bodies[0].mesh, &TessellationSettings::default())
        .expect("converts");
    assert!(solid.closed, "{:?}", solid.summary);
    let faces: BTreeSet<_> = solid.mesh.faces.iter().collect();
    assert_eq!(
        faces.len(),
        6,
        "coplanar triangles merge into one face a side"
    );
    assert_eq!(
        solid.mesh.edge_ids.len(),
        solid.mesh.edges.len() / 2,
        "the solid's outline names its kernel edges"
    );
    assert_eq!(solid.health.broken, 0, "{}", solid.health.describe());
    let volume = kernel
        .physical_properties(&solid.brep_blob)
        .expect("measures")
        .volume_mm3
        .expect("a closed solid has a volume");
    assert!((volume - 1000.0).abs() < 1e-3, "volume {volume}");
}

#[test]
fn an_open_mesh_converts_to_a_shell_and_says_why() {
    let bytes = ogeom::io::stl::write(&cube(true), ogeom::io::Encoding::Binary).expect("writes");
    let model = import(&staged("open.stl", &bytes));
    let mut kernel = OgeomKernel::new();
    let shell = kernel
        .mesh_to_solid(&model.bodies[0].mesh, &TessellationSettings::default())
        .expect("an open mesh still converts");
    assert!(!shell.closed);
    assert!(
        shell.summary.iter().any(|s| s.contains("hole edges")),
        "{:?}",
        shell.summary
    );
}
