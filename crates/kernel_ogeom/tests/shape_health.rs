//! Every imported body carries the kernel checker's verdict, and a body's
//! snapshot can be run through the kernel's repair.

use kernel_api::{ImportedModel, Kernel, TessellationSettings};
use kernel_ogeom::OgeomKernel;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn import(name: &str) -> (OgeomKernel, ImportedModel) {
    let mut kernel = OgeomKernel::new();
    let model = kernel
        .import_step(&fixture(name), &TessellationSettings::default())
        .expect("fixture imports");
    (kernel, model)
}

#[test]
fn a_clean_file_checks_clean() {
    let (_, model) = import("box_native.step");
    assert!(!model.bodies.is_empty());
    for body in &model.bodies {
        let health = body.health.as_ref().expect("every body is checked");
        assert_eq!(health.broken, 0, "{}", health.describe());
        assert!(!health.repaired);
    }
}

#[test]
fn a_repair_hands_back_a_mesh_and_the_checker_s_verdict() {
    let (mut kernel, model) = import("drive_frame_upper.step");
    for body in &model.bodies {
        let result = kernel
            .repair_brep(
                &body.brep_blob,
                &body.face_colors,
                &TessellationSettings::default(),
            )
            .expect("the repair runs");
        assert!(result.health.repaired);
        assert!(!result.mesh.indices.is_empty(), "the mended shape draws");
        assert!(!result.brep_blob.is_empty());
        assert_eq!(
            result.face_colors.len(),
            body.face_colors.len(),
            "no faces were sewn, so every face keeps its colour"
        );
        let before = body.health.as_ref().expect("checked on import");
        assert!(
            result.health.broken <= before.broken,
            "a repair never adds defects"
        );
    }
}

#[test]
#[ignore = "kernel: the STEP reader widens edge tolerances past their vertices', breaking the containment rule it documents, so the checker calls well-formed exports broken (ogeom-rs#45)"]
fn a_well_formed_export_checks_clean() {
    let (_, model) = import("drive_frame_upper.step");
    for body in &model.bodies {
        let health = body.health.as_ref().expect("every body is checked");
        assert_eq!(health.broken, 0, "{}", health.describe());
    }
}

#[test]
#[ignore = "kernel: fix_shape only tightens tolerances and never widens a vertex to cover its edges, so the containment findings survive the repair (ogeom-rs#45)"]
fn a_repair_clears_tolerance_containment_findings() {
    let (mut kernel, model) = import("drive_frame_upper.step");
    for body in &model.bodies {
        let result = kernel
            .repair_brep(
                &body.brep_blob,
                &body.face_colors,
                &TessellationSettings::default(),
            )
            .expect("the repair runs");
        assert_eq!(result.health.broken, 0, "{}", result.health.describe());
    }
}

/// A box is a measure its own bounds can check.
#[test]
fn a_box_measures_its_volume_area_and_centre() {
    let (mut kernel, model) = import("box_native.step");
    let body = model.bodies.first().expect("one body");
    let props = kernel
        .physical_properties(&body.brep_blob)
        .expect("a closed box measures");
    let (lo, hi) = body.bounds_mm.expect("bounds");
    let size: Vec<f64> = (0..3).map(|i| f64::from(hi[i] - lo[i])).collect();
    let volume = size[0] * size[1] * size[2];
    let area = 2.0 * (size[0] * size[1] + size[1] * size[2] + size[0] * size[2]);
    let got = props.volume_mm3.expect("a closed box has a volume");
    assert!(
        (got - volume).abs() < 1e-3 * volume,
        "volume {got} vs {volume}"
    );
    assert!(
        (props.area_mm2 - area).abs() < 1e-3 * area,
        "area {} vs {area}",
        props.area_mm2
    );
    for i in 0..3 {
        let mid = f64::from(lo[i] + hi[i]) / 2.0;
        assert!((props.centre_mm[i] - mid).abs() < 1e-3, "centre axis {i}");
    }
}

/// An IGES file imports through the same path as STEP: the bundled box,
/// written out as IGES by the kernel, comes back as one body of the same
/// size, checked like any other.
#[test]
fn an_iges_file_imports_like_a_step_file() {
    let step_text = std::fs::read_to_string(fixture("box_native.step")).expect("fixture");
    let tol = ogeom::core::Tolerances::millimetres();
    let read = ogeom::io::step::read_step(&step_text, tol).expect("fixture reads");
    let iges_text = ogeom::io::write_iges(&read.document, tol).expect("the box writes as IGES");
    let path = std::env::temp_dir().join(format!("printcad_box_{}.igs", std::process::id()));
    std::fs::write(&path, iges_text).expect("staged");

    let mut kernel = OgeomKernel::new();
    let from_iges = kernel
        .import_step(&path, &TessellationSettings::default())
        .expect("the IGES file imports");
    let _ = std::fs::remove_file(&path);
    let (_, from_step) = import("box_native.step");

    assert_eq!(from_iges.bodies.len(), from_step.bodies.len());
    let (a, b) = (&from_iges.bodies[0], &from_step.bodies[0]);
    let (ia, ib) = (a.bounds_mm.expect("bounds"), b.bounds_mm.expect("bounds"));
    for axis in 0..3 {
        assert!((ia.0[axis] - ib.0[axis]).abs() < 1e-3, "min {axis}");
        assert!((ia.1[axis] - ib.1[axis]).abs() < 1e-3, "max {axis}");
    }
    assert!(!a.mesh.indices.is_empty(), "the IGES body draws");
    assert!(a.health.is_some(), "the IGES body is checked");
}

/// The fixture written as IGES by the kernel, staged to a temp file.
fn fixture_as_iges(name: &str) -> PathBuf {
    let tol = ogeom::core::Tolerances::millimetres();
    let text = std::fs::read_to_string(fixture(name)).expect("fixture");
    let read = ogeom::io::step::read_step(&text, tol).expect("fixture reads");
    let iges = ogeom::io::write_iges(&read.document, tol).expect("fixture writes as IGES");
    let path = std::env::temp_dir().join(format!(
        "printcad_{}_{}.igs",
        name.replace('.', "_"),
        std::process::id()
    ));
    std::fs::write(&path, iges).expect("staged");
    path
}

#[test]
#[ignore = "kernel: write_iges prints a tiny real in positional notation past the 80-column record, and read_iges refuses the file (ogeom-rs#46)"]
fn a_real_part_written_as_iges_keeps_the_record_width() {
    let path = fixture_as_iges("drive_frame_upper.step");
    let text = std::fs::read_to_string(&path).expect("staged file");
    let _ = std::fs::remove_file(&path);
    let long = text.lines().filter(|l| l.len() != 80).count();
    assert_eq!(long, 0, "{long} records are not 80 columns");
}

#[test]
#[ignore = "kernel: the written IGES overflows its records (ogeom-rs#46), and read_iges refuses solids whose curves miss their vertices by nanometres (ogeom-rs#47)"]
fn a_real_part_imports_back_from_iges() {
    let path = fixture_as_iges("drive_frame_upper.step");
    let mut kernel = OgeomKernel::new();
    let back = kernel.import_step(&path, &TessellationSettings::default());
    let _ = std::fs::remove_file(&path);
    let (_, from_step) = import("drive_frame_upper.step");
    let back = back.expect("the IGES file imports");
    assert_eq!(back.bodies.len(), from_step.bodies.len());
}
