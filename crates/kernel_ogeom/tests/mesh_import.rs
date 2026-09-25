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

/// A closed cylinder as an STL has it: `sides` facets round, caps fanned
/// from their centres, wound outward.
fn cylinder(radius: f64, height: f64, sides: usize) -> Triangulation {
    let mut mesh = Triangulation::new();
    let at = |i: usize, z: f64| {
        let t = std::f64::consts::TAU * i as f64 / sides as f64;
        Point::new(radius * t.cos(), radius * t.sin(), z)
    };
    let mut push = |tri: [Point; 3]| {
        let base = mesh.positions.len() as u32;
        let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]);
        let n = n / n.magnitude();
        for p in tri {
            mesh.positions.push(p);
            mesh.normals.push(Vector::new(n.x, n.y, n.z));
            mesh.parameters.push((0.0, 0.0));
        }
        mesh.triangles.push([base, base + 1, base + 2]);
    };
    for i in 0..sides {
        let j = (i + 1) % sides;
        push([at(i, 0.0), at(j, 0.0), at(j, height)]);
        push([at(i, 0.0), at(j, height), at(i, height)]);
        push([Point::new(0.0, 0.0, 0.0), at(j, 0.0), at(i, 0.0)]);
        push([Point::new(0.0, 0.0, height), at(i, height), at(j, height)]);
    }
    mesh
}

/// A converted part takes features: a hole drilled through it cuts, and
/// what is left is measured.
#[test]
fn a_converted_mesh_is_drilled_and_measured() {
    use kernel_api::{BooleanOp, Placement, PrimitiveKind, SolidOp};
    let mut kernel = OgeomKernel::new();
    let detail = TessellationSettings::default();
    for (name, mesh, drill, whole) in [
        ("drilled-cube.stl", cube(false), [5.0, 5.0, -5.0], 1000.0),
        (
            "drilled-rod.stl",
            cylinder(10.0, 20.0, 64),
            [4.0, 0.0, -5.0],
            // The 64-gon's own area, whatever the converter makes of it.
            0.5 * 64.0 * 100.0 * (std::f64::consts::TAU / 64.0).sin() * 20.0,
        ),
    ] {
        let bytes = ogeom::io::stl::write(&mesh, ogeom::io::Encoding::Binary).expect("writes");
        let model = import(&staged(name, &bytes));
        let solid = kernel
            .mesh_to_solid(&model.bodies[0].mesh, &detail)
            .expect("converts");
        assert!(solid.closed, "{name}: {:?}", solid.summary);
        let height = if name == "drilled-cube.stl" {
            10.0
        } else {
            20.0
        };
        let ops = [
            SolidOp::Shape {
                brep: solid.brep_blob.clone(),
            },
            SolidOp::Primitive {
                kind: PrimitiveKind::Cylinder {
                    radius: 2.0,
                    height: height + 10.0,
                    angle_deg: 360.0,
                },
                placement: Placement {
                    origin: drill,
                    ..Placement::default()
                },
                op: BooleanOp::Cut,
            },
        ];
        let drilled = kernel
            .execute_solid_chain(&ops, &detail)
            .unwrap_or_else(|e| panic!("{name}: the drill cuts: {e}"));
        let volume = kernel
            .physical_properties(&drilled.brep_blob)
            .expect("measures")
            .volume_mm3
            .unwrap_or_else(|| panic!("{name}: the drilled part has a volume"));
        let hole = std::f64::consts::PI * 4.0 * height;
        let want = whole - hole;
        assert!(
            (volume - want).abs() < 0.01 * want,
            "{name}: volume {volume}, want about {want}"
        );
    }
}

/// A PLY is read as millimetres, as an STL is; a VRML scene is in metres
/// and comes in scaled, one body per shape, in its colour.
#[test]
fn ply_and_vrml_scenes_import_as_mesh_bodies() {
    let ply = "ply\nformat ascii 1.0\nelement vertex 4\nproperty float x\nproperty float y\nproperty float z\nelement face 4\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n10 0 0\n0 10 0\n0 0 10\n3 0 2 1\n3 0 1 3\n3 0 3 2\n3 1 2 3\n";
    let model = import(&staged("tetra.ply", ply.as_bytes()));
    assert_eq!(model.bodies.len(), 1);
    let (lo, hi) = model.bodies[0].mesh.bounds().unwrap();
    assert_eq!((lo, hi), ([0.0; 3], [10.0; 3]));

    let vrml = "#VRML V2.0 utf8\nShape {\n  appearance Appearance { material Material { diffuseColor 1 0 0 } }\n  geometry Box { size 0.02 0.01 0.03 }\n}\nTransform { translation 0.1 0 0 children [ Shape { geometry Sphere { radius 0.005 } } ] }\n";
    let model = import(&staged("scene.wrl", vrml.as_bytes()));
    assert_eq!(model.bodies.len(), 2, "a body per shape");
    let (lo, hi) = model.bodies[0].mesh.bounds().unwrap();
    for (axis, size) in [20.0, 10.0, 30.0].into_iter().enumerate() {
        assert!((hi[axis] - lo[axis] - size).abs() < 1e-3, "{lo:?} {hi:?}");
    }
    let red = model.bodies[0]
        .mesh
        .colors
        .first()
        .copied()
        .expect("its colour");
    assert!(red[0] > 0.9 && red[1] < 0.05, "{red:?}");
    let (lo, hi) = model.bodies[1].mesh.bounds().unwrap();
    assert!(
        ((lo[0] + hi[0]) / 2.0 - 100.0).abs() < 1e-3,
        "placed 0.1 m along x"
    );
}
