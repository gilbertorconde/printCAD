//! Export writes what import reads back: the bundled box as STEP keeps its
//! size, and as STL or 3MF arrives as a closed mesh of the same extent.

use std::collections::HashMap;
use std::path::PathBuf;

use kernel_api::{ImportedModel, Kernel, TessellationSettings, TriMesh};
use kernel_ogeom::OgeomKernel;
use kernel_ogeom::export::{ExportBody, ExportFormat, export};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn import(path: &std::path::Path) -> ImportedModel {
    OgeomKernel::new()
        .import_step(path, &TessellationSettings::default())
        .expect("imports")
}

fn bounds(mesh: &TriMesh) -> ([f32; 3], [f32; 3]) {
    mesh.bounds().expect("a mesh with vertices")
}

fn same_box(a: ([f32; 3], [f32; 3]), b: ([f32; 3], [f32; 3])) {
    for i in 0..3 {
        assert!((a.0[i] - b.0[i]).abs() < 1e-3, "low {i}: {a:?} vs {b:?}");
        assert!((a.1[i] - b.1[i]).abs() < 1e-3, "high {i}: {a:?} vs {b:?}");
    }
}

/// Every edge of a closed mesh borders exactly two triangles, once each way.
fn closed(mesh: &TriMesh) -> bool {
    let key = |p: [f32; 3]| p.map(|c| (c * 1e4).round() as i64);
    let mut edges: HashMap<([i64; 3], [i64; 3]), i32> = HashMap::new();
    for t in mesh.indices.chunks(3) {
        for k in 0..3 {
            let a = key(mesh.positions[t[k] as usize]);
            let b = key(mesh.positions[t[(k + 1) % 3] as usize]);
            *edges.entry((a, b)).or_default() += 1;
        }
    }
    edges
        .iter()
        .all(|((a, b), n)| *n == 1 && edges.get(&(*b, *a)) == Some(&1))
}

fn round_trip(format: ExportFormat) -> (ImportedModel, ImportedModel) {
    let source = import(&fixture("box_native.step"));
    let bodies: Vec<ExportBody<'_>> = source
        .bodies
        .iter()
        .map(|b| ExportBody {
            name: "box".into(),
            brep: Some(&b.brep_blob),
            transform: None,
            mesh: &b.mesh,
        })
        .collect();
    let out = export(&bodies, format, &TessellationSettings::default()).expect("exports");
    assert_eq!(out.written, 1);
    assert!(out.skipped.is_empty());
    let dir = std::env::temp_dir().join(format!("printcad-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("box.{}", format.extension()));
    std::fs::write(&path, &out.bytes).unwrap();
    let back = import(&path);
    let _ = std::fs::remove_file(&path);
    (source, back)
}

#[test]
fn a_box_written_as_step_reads_back_the_same_size() {
    let (source, back) = round_trip(ExportFormat::Step);
    assert_eq!(back.bodies.len(), 1);
    same_box(
        source.bodies[0].bounds_mm.expect("bounds"),
        back.bodies[0].bounds_mm.expect("bounds"),
    );
}

#[test]
fn a_box_written_as_stl_or_3mf_reads_back_closed_and_the_same_size() {
    for format in [ExportFormat::Stl, ExportFormat::ThreeMf] {
        let (source, back) = round_trip(format);
        assert_eq!(back.bodies.len(), 1, "{}", format.label());
        let mesh = &back.bodies[0].mesh;
        assert!(closed(mesh), "{} comes back open", format.label());
        same_box(bounds(&source.bodies[0].mesh), bounds(mesh));
    }
}

#[test]
fn a_mesh_body_is_left_out_of_step_and_written_to_the_mesh_formats() {
    let source = import(&fixture("box_native.step"));
    let mesh = &source.bodies[0].mesh;
    let bodies = [ExportBody {
        name: "scan".into(),
        brep: None,
        transform: None,
        mesh,
    }];
    assert!(
        export(
            &bodies,
            ExportFormat::Step,
            &TessellationSettings::default()
        )
        .is_err()
    );
    let stl = export(&bodies, ExportFormat::Stl, &TessellationSettings::default()).unwrap();
    assert_eq!(stl.triangles, mesh.indices.len() / 3);
}

#[test]
fn the_format_follows_the_file_name() {
    let of = |name: &str| ExportFormat::of_path(std::path::Path::new(name));
    assert_eq!(of("a.STP"), Some(ExportFormat::Step));
    assert_eq!(of("a.step"), Some(ExportFormat::Step));
    assert_eq!(of("a.stl"), Some(ExportFormat::Stl));
    assert_eq!(of("a.3mf"), Some(ExportFormat::ThreeMf));
    assert_eq!(of("a.obj"), None);
}

#[test]
fn a_real_part_written_as_3mf_is_closed() {
    let source = import(&fixture("drive_frame_upper.step"));
    let bodies: Vec<ExportBody<'_>> = source
        .bodies
        .iter()
        .map(|b| ExportBody {
            name: "part".into(),
            brep: Some(&b.brep_blob),
            transform: None,
            mesh: &b.mesh,
        })
        .collect();
    let out = export(
        &bodies,
        ExportFormat::ThreeMf,
        &TessellationSettings::default(),
    )
    .unwrap();
    let path =
        std::env::temp_dir().join(format!("printcad-export-real-{}.3mf", std::process::id()));
    std::fs::write(&path, &out.bytes).unwrap();
    let back = import(&path);
    let _ = std::fs::remove_file(&path);
    for body in &back.bodies {
        assert!(closed(&body.mesh), "an exported body comes back open");
    }
}

/// A placed body is written where it sits, shape and mesh alike.
#[test]
fn a_placed_body_is_written_where_it_sits() {
    let source = import(&fixture("box_native.step"));
    let body = &source.bodies[0];
    let mut shift = [[0.0; 4]; 4];
    for (i, row) in shift.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    shift[0][3] = 100.0;
    let bodies = [ExportBody {
        name: "box".into(),
        brep: Some(&body.brep_blob),
        transform: Some(shift),
        mesh: &body.mesh,
    }];
    let (lo, hi) = body.bounds_mm.unwrap();
    for format in [ExportFormat::Step, ExportFormat::ThreeMf] {
        let out = export(&bodies, format, &TessellationSettings::default()).unwrap();
        let path = std::env::temp_dir().join(format!(
            "printcad-export-placed-{}.{}",
            std::process::id(),
            format.extension()
        ));
        std::fs::write(&path, &out.bytes).unwrap();
        let back = import(&path);
        let _ = std::fs::remove_file(&path);
        let (blo, bhi) = bounds(&back.bodies[0].mesh);
        assert!(
            (blo[0] - (lo[0] + 100.0)).abs() < 1e-2,
            "{}",
            format.label()
        );
        assert!(
            (bhi[0] - (hi[0] + 100.0)).abs() < 1e-2,
            "{}",
            format.label()
        );
        assert!((blo[1] - lo[1]).abs() < 1e-2, "{}", format.label());
    }
}

/// A NURBS-only STEP writes a cylinder's walls and caps as splines, and it
/// reads back as the same solid.
#[test]
fn a_cylinder_written_as_nurbs_only_step_is_all_splines_and_the_same_solid() {
    use kernel_api::{BooleanOp, Placement, PrimitiveKind, SolidOp};
    let mut kernel = OgeomKernel::new();
    let built = kernel
        .execute_solid_chain(
            &[SolidOp::Primitive {
                kind: PrimitiveKind::Cylinder {
                    radius: 5.0,
                    height: 10.0,
                    angle_deg: 360.0,
                },
                placement: Placement::default(),
                op: BooleanOp::NewSolid,
            }],
            &TessellationSettings::default(),
        )
        .expect("a cylinder");
    let body = ExportBody {
        name: "drum".into(),
        brep: Some(&built.brep_blob),
        transform: None,
        mesh: &built.mesh,
    };
    let out = export(
        &[body],
        ExportFormat::StepNurbs,
        &TessellationSettings::default(),
    )
    .expect("exports");
    let text = String::from_utf8(out.bytes.clone()).unwrap();
    assert!(text.contains("B_SPLINE_SURFACE"), "splines written");
    assert!(
        !text.contains("CYLINDRICAL_SURFACE") && !text.contains("PLANE("),
        "no analytic surface left"
    );
    let dir = std::env::temp_dir().join(format!("printcad-nurbs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("drum.step");
    std::fs::write(&path, &out.bytes).unwrap();
    let back = import(&path);
    let _ = std::fs::remove_file(&path);
    let volume = kernel
        .physical_properties(&back.bodies[0].brep_blob)
        .unwrap()
        .volume_mm3
        .unwrap();
    let want = std::f64::consts::PI * 25.0 * 10.0;
    assert!(
        (volume - want).abs() < want * 1e-4,
        "{volume} against {want}"
    );
}
