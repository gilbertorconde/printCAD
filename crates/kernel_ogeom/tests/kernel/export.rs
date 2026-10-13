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
            finish: None,
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
        finish: None,
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
#[ignore = "kernel: the real part's quarter-cone corners (an apex vertex, two rulings and a quarter circle) and the B-spline face beside them mesh with triangles folded over at the apex, so the export comes back open (ogeom-rs#210)"]
fn a_real_part_written_as_3mf_is_closed() {
    let source = crate::real_part();
    let bodies: Vec<ExportBody<'_>> = source
        .bodies
        .iter()
        .map(|b| ExportBody {
            name: "part".into(),
            brep: Some(&b.brep_blob),
            transform: None,
            mesh: &b.mesh,
            finish: None,
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
        finish: None,
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
        finish: None,
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

/// Two bodies of one shape written together are one part placed twice in
/// an assembly: they read back as two bodies where they stood, from one
/// shape.
#[test]
fn bodies_of_one_shape_are_written_as_one_part_placed_twice() {
    let source = import(&fixture("box_native.step"));
    let body = &source.bodies[0];
    let at = |x: f64| {
        let mut m = [[0.0; 4]; 4];
        for (i, row) in m.iter_mut().enumerate() {
            row[i] = 1.0;
        }
        m[0][3] = x;
        m
    };
    let bodies = [
        ExportBody {
            name: "left".into(),
            brep: Some(&body.brep_blob),
            transform: None,
            mesh: &body.mesh,
            finish: None,
        },
        ExportBody {
            name: "right".into(),
            brep: Some(&body.brep_blob),
            transform: Some(at(100.0)),
            mesh: &body.mesh,
            finish: None,
        },
    ];
    let out = export(
        &bodies,
        ExportFormat::Step,
        &TessellationSettings::default(),
    )
    .unwrap();
    assert_eq!(out.written, 2);
    let text = String::from_utf8(out.bytes.clone()).unwrap();
    assert_eq!(
        text.matches("MANIFOLD_SOLID_BREP").count(),
        1,
        "one shape, written once"
    );
    let path =
        std::env::temp_dir().join(format!("printcad-export-asm-{}.step", std::process::id()));
    std::fs::write(&path, &out.bytes).unwrap();
    let back = import(&path);
    let _ = std::fs::remove_file(&path);
    assert_eq!(back.bodies.len(), 2);
    let (lo, _) = body.bounds_mm.unwrap();
    let mut lows: Vec<f32> = back.bodies.iter().map(|b| bounds(&b.mesh).0[0]).collect();
    lows.sort_by(f32::total_cmp);
    assert!(
        (lows[0] - lo[0]).abs() < 1e-2 && (lows[1] - (lo[0] + 100.0)).abs() < 1e-2,
        "{lows:?}"
    );
}

/// A placed box with a note written as STEP, and read back.
fn noted_box_round_trip() -> ImportedModel {
    use kernel_ogeom::export::{ExportNote, export_with_notes};
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
        finish: None,
    }];
    let notes = [ExportNote {
        body: 0,
        text: "Deburr this edge".into(),
        at: [1.0, 2.0, 3.0],
        label: [1.0, 2.0, 13.0],
    }];
    let out = export_with_notes(
        &bodies,
        &notes,
        ExportFormat::Step,
        &TessellationSettings::default(),
    )
    .unwrap();
    let text = String::from_utf8(out.bytes.clone()).unwrap();
    assert!(text.contains("DRAUGHTING_CALLOUT('Deburr this edge'"));
    let path =
        std::env::temp_dir().join(format!("printcad-export-note-{}.step", std::process::id()));
    std::fs::write(&path, &out.bytes).unwrap();
    let back = import(&path);
    let _ = std::fs::remove_file(&path);
    back
}

/// A note goes out as a drawn callout: its leader where the body sits,
/// its words read back as the annotation's text.
#[test]
fn a_note_is_written_to_step_as_a_callout_where_its_body_sits() {
    let back = noted_box_round_trip();
    let note = back
        .annotations
        .iter()
        .find(|a| a.text == "Deburr this edge")
        .expect("the note reads back");
    let leader = &note.polylines[0];
    assert_eq!(leader.len(), 2);
    for (got, want) in leader.iter().zip([[101.0, 2.0, 3.0], [101.0, 2.0, 13.0]]) {
        for i in 0..3 {
            assert!((got[i] - want[i]).abs() < 1e-4, "{got:?} against {want:?}");
        }
    }
}

#[test]
#[ignore = "kernel: a callout carries no text of its own (no TEXT_LITERAL written or read), so a note's words ride in its name and read back as an unclassified annotation"]
fn a_note_written_to_step_reads_back_as_a_note() {
    let back = noted_box_round_trip();
    let note = back
        .annotations
        .iter()
        .find(|a| a.text == "Deburr this edge")
        .expect("the note reads back");
    assert_eq!(note.kind, kernel_api::AnnotationKind::Note);
}

/// A thread's callout written with the second of two parts reads back as
/// that part's: the callout names the bore it threads, so a reader puts
/// the designation on the right part of an assembly.
#[test]
#[ignore = "kernel: a callout names no topology unless it draws a measured annotation, and write_step anchors every semantic annotation to the first part's product definition, so a thread's designation cannot be tied to the bore of the part it threads (ogeom-rs#227)"]
fn a_thread_callout_reads_back_on_the_part_it_threads() {
    use kernel_ogeom::export::{ExportNote, export_with_notes};
    let first = import(&fixture("box_native.step"));
    let second = import(&fixture("box.step"));
    let (a, b) = (&first.bodies[0], &second.bodies[0]);
    let bodies = [
        ExportBody {
            name: "plain".into(),
            brep: Some(&a.brep_blob),
            transform: None,
            mesh: &a.mesh,
            finish: None,
        },
        ExportBody {
            name: "threaded".into(),
            brep: Some(&b.brep_blob),
            transform: None,
            mesh: &b.mesh,
            finish: None,
        },
    ];
    let notes = [ExportNote {
        body: 1,
        text: "M6 thread".into(),
        at: [1.0, 2.0, 3.0],
        label: [1.0, 2.0, 13.0],
    }];
    let out = export_with_notes(
        &bodies,
        &notes,
        ExportFormat::Step,
        &TessellationSettings::default(),
    )
    .unwrap();
    let path = std::env::temp_dir().join(format!(
        "printcad-export-thread-{}.step",
        std::process::id()
    ));
    std::fs::write(&path, &out.bytes).unwrap();
    let back = import(&path);
    let _ = std::fs::remove_file(&path);
    let thread = back
        .annotations
        .iter()
        .find(|a| a.text == "M6 thread")
        .expect("the thread reads back");
    assert_eq!(thread.body_index, Some(1));
}
