//! Export writes what import reads back: the bundled box as STEP keeps its
//! size, and as STL or 3MF arrives as a closed mesh of the same extent.

use std::collections::HashMap;
use std::path::PathBuf;

use kernel_api::{ImportedModel, Kernel, TessellationSettings, TriMesh};
use kernel_ogeom::OgeomKernel;
use kernel_ogeom::export::{ExportBody, ExportFormat, export};

use super::common::{chain, volume};

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
            scale: None,
            rounds: None,
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
fn a_mesh_body_is_left_out_of_step_and_written_to_the_mesh_formats_scaled_when_asked() {
    let source = import(&fixture("box_native.step"));
    let mesh = &source.bodies[0].mesh;
    let bodies = [ExportBody {
        name: "scan".into(),
        brep: None,
        transform: None,
        mesh,
        finish: None,
        scale: None,
        rounds: None,
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
    // A shrinkage made up for: the mesh written scaled per axis about its
    // middle.
    let extent = |bytes: &[u8]| {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for t in bytes[84..].chunks(50) {
            for v in 0..3 {
                for k in 0..3 {
                    let at = 12 + v * 12 + k * 4;
                    let c = f32::from_le_bytes(t[at..at + 4].try_into().unwrap());
                    lo[k] = lo[k].min(c);
                    hi[k] = hi[k].max(c);
                }
            }
        }
        (lo, hi)
    };
    let scaled = [ExportBody {
        scale: Some([1.1, 1.0, 0.9]),
        rounds: None,
        ..ExportBody {
            name: "scan".into(),
            brep: None,
            transform: None,
            mesh,
            finish: None,
            scale: None,
            rounds: None,
        }
    }];
    let grown = export(&scaled, ExportFormat::Stl, &TessellationSettings::default()).unwrap();
    let (a, b) = (extent(&stl.bytes), extent(&grown.bytes));
    for (k, f) in [1.1f32, 1.0, 0.9].into_iter().enumerate() {
        assert!(
            ((b.1[k] - b.0[k]) - f * (a.1[k] - a.0[k])).abs() < 1e-3,
            "axis {k}"
        );
        assert!(
            ((b.1[k] + b.0[k]) - (a.1[k] + a.0[k])).abs() < 1e-3,
            "about its middle"
        );
    }
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
            scale: None,
            rounds: None,
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
        scale: None,
        rounds: None,
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
    let built = chain(&[SolidOp::Primitive {
        kind: PrimitiveKind::Cylinder {
            radius: 5.0,
            height: 10.0,
            angle_deg: 360.0,
        },
        placement: Placement::default(),
        op: BooleanOp::NewSolid,
    }]);
    let body = ExportBody {
        name: "drum".into(),
        brep: Some(&built.brep_blob),
        transform: None,
        mesh: &built.mesh,
        finish: None,
        scale: None,
        rounds: None,
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
    let volume = volume(&back.bodies[0].brep_blob);
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
            scale: None,
            rounds: None,
        },
        ExportBody {
            name: "right".into(),
            brep: Some(&body.brep_blob),
            transform: Some(at(100.0)),
            mesh: &body.mesh,
            finish: None,
            scale: None,
            rounds: None,
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

#[test]
fn semantic_pmi_round_trips_on_faces_of_placed_parts() {
    use kernel_api::{PmiDatum, PmiDimension, PmiTolerance, PrimitiveKind, SemanticPmi};
    use kernel_ogeom::export::{ExportPmi, export_with_annotations};
    let box_shape = chain(&[super::common::primitive(PrimitiveKind::Box {
        length: 10.0,
        width: 10.0,
        height: 10.0,
    })]);
    let cylinder = chain(&[super::common::primitive(PrimitiveKind::Cylinder {
        radius: 3.0,
        height: 10.0,
        angle_deg: 360.0,
    })]);
    let top = vec![[5.0, 5.0, 10.0], [6.0, 5.0, 10.0]];
    let side = vec![[0.0, 5.0, 5.0]];
    let bottom = vec![[5.0, 5.0, 0.0]];
    let annotations = SemanticPmi {
        tolerances: vec![
            PmiTolerance {
                kind: "flatness".into(),
                magnitude: 0.05,
                at: top.clone(),
                ..Default::default()
            },
            PmiTolerance {
                kind: "perpendicularity".into(),
                magnitude: 0.1,
                datums: vec!["A".into()],
                at: side.clone(),
                ..Default::default()
            },
        ],
        datums: vec![PmiDatum {
            label: "A".into(),
            at: bottom.clone(),
        }],
        dimensions: vec![
            PmiDimension {
                name: "linear distance".into(),
                nominal: 10.0,
                plus: Some(0.2),
                minus: Some(-0.1),
                features: vec![bottom, top.clone()],
                ..Default::default()
            },
            PmiDimension {
                name: "angle".into(),
                angular: true,
                nominal: std::f64::consts::FRAC_PI_2,
                plus: Some(0.5_f64.to_radians()),
                minus: Some(-0.5_f64.to_radians()),
                features: vec![top, side],
            },
        ],
    };
    for (count, format) in [
        (1, ExportFormat::Step),
        (3, ExportFormat::Step),
        (1, ExportFormat::StepNurbs),
    ] {
        let bodies: Vec<_> = (0..count)
            .map(|i| {
                let shape = if i == 2 { &cylinder } else { &box_shape };
                ExportBody {
                    name: format!("part {i}"),
                    brep: Some(&shape.brep_blob),
                    transform: Some([
                        [0.0, -1.0, 0.0, 100.0 * (i + 1) as f64],
                        [1.0, 0.0, 0.0, 20.0],
                        [0.0, 0.0, 1.0, 30.0],
                        [0.0, 0.0, 0.0, 1.0],
                    ]),
                    mesh: &shape.mesh,
                    finish: None,
                    scale: None,
                    rounds: None,
                }
            })
            .collect();
        let pmi: Vec<_> = (0..count)
            .map(|body| ExportPmi {
                body,
                annotations: if body < 2 {
                    annotations.clone()
                } else {
                    SemanticPmi {
                        dimensions: vec![PmiDimension {
                            name: "diameter".into(),
                            nominal: 6.0,
                            plus: Some(0.1),
                            minus: Some(-0.1),
                            features: vec![vec![[3.0, 0.0, 5.0]]],
                            ..Default::default()
                        }],
                        ..Default::default()
                    }
                },
            })
            .collect();
        let out =
            export_with_annotations(&bodies, &[], &pmi, format, &TessellationSettings::default())
                .unwrap();
        let text = String::from_utf8(out.bytes.clone()).unwrap();
        let raw =
            ogeom::io::step::read_step(&text, ogeom::core::Tolerances::millimetres()).unwrap();
        let pmi = raw.document.pmi();
        for items in pmi
            .tolerances
            .iter()
            .map(|t| t.items.as_slice())
            .chain(pmi.datums.iter().map(|d| d.items.as_slice()))
            .chain(
                pmi.dimensions
                    .iter()
                    .flat_map(|d| d.features.iter().map(Vec::as_slice)),
            )
        {
            assert_eq!(items.len(), 1, "one distinct face per feature");
            assert_eq!(
                raw.document.model().node_by_id(items[0]).unwrap().kind(),
                ogeom::topo::ShapeType::Face
            );
        }
        assert_ne!(pmi.tolerances[0].items, pmi.tolerances[1].items);
        assert_ne!(pmi.tolerances[0].items, pmi.datums[0].items);
        if count == 3 {
            assert_eq!(
                pmi.datums
                    .iter()
                    .map(|d| d.label.as_str())
                    .collect::<Vec<_>>(),
                ["A", "A2"]
            );
            assert_eq!(pmi.tolerances[3].datums[0].label, "A2");
            assert_ne!(pmi.datums[0].items, pmi.datums[1].items);
        }
        let path =
            std::env::temp_dir().join(format!("printcad-semantic-{}.step", std::process::id()));
        std::fs::write(&path, out.bytes).unwrap();
        let back = import(&path);
        std::fs::remove_file(path).unwrap();
        for body in 0..count.min(2) {
            let texts: Vec<_> = back
                .annotations
                .iter()
                .filter(|a| a.body_index == Some(body))
                .map(|a| a.text.as_str())
                .collect();
            for expected in [
                "Flatness 0.05",
                if body == 0 {
                    "Perpendicularity 0.1 | A"
                } else {
                    "Perpendicularity 0.1 | A2"
                },
                "10 +0.2/-0.1",
                "90° ±0.5°",
            ] {
                assert!(texts.contains(&expected), "{texts:?} missing {expected}");
            }
        }
        if count == 3 {
            assert!(
                back.annotations
                    .iter()
                    .any(|a| a.body_index == Some(2) && a.text == "Ø 6 ±0.1")
            );
        }
        let mut missing = annotations.clone();
        missing.tolerances[0].at = vec![[1e6; 3]];
        assert!(
            export_with_annotations(
                &bodies,
                &[],
                &[ExportPmi {
                    body: 0,
                    annotations: missing
                }],
                format,
                &TessellationSettings::default()
            )
            .is_err()
        );
    }
}

/// A placed box with a note written as STEP, and read back: once, for
/// every test that reads it, since the file it passes through is named by
/// the process alone.
fn noted_box_round_trip() -> &'static ImportedModel {
    static BACK: std::sync::OnceLock<ImportedModel> = std::sync::OnceLock::new();
    BACK.get_or_init(write_and_read_noted_box)
}

fn write_and_read_noted_box() -> ImportedModel {
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
        scale: None,
        rounds: None,
    }];
    let notes = [ExportNote {
        body: 0,
        text: "Deburr this edge".into(),
        at: [1.0, 2.0, 3.0],
        label: [1.0, 2.0, 13.0],
        faces: vec![[0.0, 2.0, 3.0]],
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
            scale: None,
            rounds: None,
        },
        ExportBody {
            name: "threaded".into(),
            brep: Some(&b.brep_blob),
            transform: None,
            mesh: &b.mesh,
            finish: None,
            scale: None,
            rounds: None,
        },
    ];
    let notes = [ExportNote {
        body: 1,
        text: "M6 thread".into(),
        at: [1.0, 2.0, 3.0],
        label: [1.0, 2.0, 13.0],
        faces: vec![[5.0, 5.0, 10.0]],
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

/// A mesh export making up for a printer's round faces draws a plate's
/// hole 0.2 mm larger in radius and its pin 0.1 mm smaller; the solid is
/// left as it was drawn.
#[test]
fn a_mesh_export_offsets_round_holes_and_bosses() {
    use kernel_api::{BooleanOp, Placement, PrimitiveKind, SolidOp};
    let cylinder = |x: f64, z: f64, radius: f64, height: f64, op| SolidOp::Primitive {
        kind: PrimitiveKind::Cylinder {
            radius,
            height,
            angle_deg: 360.0,
        },
        placement: Placement {
            origin: [x, 10.0, z],
            ..Placement::default()
        },
        op,
    };
    let built = chain(&[
        SolidOp::Primitive {
            kind: PrimitiveKind::Box {
                length: 30.0,
                width: 20.0,
                height: 5.0,
            },
            placement: Placement::default(),
            op: BooleanOp::NewSolid,
        },
        cylinder(8.0, -1.0, 4.0, 7.0, BooleanOp::Cut),
        cylinder(22.0, 5.0, 3.0, 5.0, BooleanOp::Fuse),
    ]);
    let body = ExportBody {
        name: "plate".into(),
        brep: Some(&built.brep_blob),
        transform: None,
        mesh: &built.mesh,
        finish: None,
        scale: None,
        rounds: Some([0.2, -0.1]),
    };
    let stl = export(&[body], ExportFormat::Stl, &TessellationSettings::default()).unwrap();
    // The radii of the vertices round each wall, at its rims.
    let radii = |x: f32, z: [f32; 2]| -> Vec<f32> {
        stl.bytes[84..]
            .chunks(50)
            .flat_map(|t| {
                (0..3).map(move |v| {
                    let c = |k: usize| {
                        let at = 12 + v * 12 + k * 4;
                        f32::from_le_bytes(t[at..at + 4].try_into().unwrap())
                    };
                    [c(0), c(1), c(2)]
                })
            })
            .filter(|p| p[2] > z[0] && p[2] < z[1])
            .map(|p| ((p[0] - x).powi(2) + (p[1] - 10.0).powi(2)).sqrt())
            .filter(|r| *r > 1.0 && *r < 6.0)
            .collect()
    };
    let (hole, pin) = (radii(8.0, [-0.1, 5.1]), radii(22.0, [4.9, 10.1]));
    assert!(!hole.is_empty() && !pin.is_empty());
    assert!(hole.iter().all(|r| (r - 4.2).abs() < 1e-3), "{hole:?}");
    assert!(pin.iter().all(|r| (r - 2.9).abs() < 1e-3), "{pin:?}");
    let drawn = 3000.0 + std::f64::consts::PI * 5.0 * (9.0 - 16.0);
    assert!(
        (volume(&built.brep_blob) - drawn).abs() < 1e-6,
        "the solid as drawn"
    );
}
