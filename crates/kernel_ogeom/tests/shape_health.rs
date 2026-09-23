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
