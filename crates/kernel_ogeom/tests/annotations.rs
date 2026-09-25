//! The annotations and layers a file carries come through an import.
//!
//! `nist_ctc_01_asme1_ap242-e1.stp` is NIST's CTC 1 test part from the MBE
//! PMI Validation and Conformance Testing project, AP242 with semantic and
//! drawn PMI; NIST states the files "can be used without any restrictions".
//! `bracket_annotated.igs` is authored for this project: a 40 × 20 × 10 box
//! on level 7, a model-space linear dimension reading `40.00` with its
//! leaders and witness lines, and a note.

use kernel_api::{AnnotationKind, ImportedModel, Kernel, TessellationSettings};
use kernel_ogeom::OgeomKernel;
use std::path::PathBuf;

fn import(name: &str) -> ImportedModel {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    let mut kernel = OgeomKernel::new();
    kernel.initialize().expect("initialize ogeom kernel");
    kernel
        .import_step(&path, &TessellationSettings::default())
        .unwrap_or_else(|err| panic!("import of {name} failed: {err}"))
}

#[test]
fn a_step_part_brings_its_drawn_and_semantic_annotations() {
    let model = import("nist_ctc_01_asme1_ap242-e1.stp");
    assert_eq!(model.bodies.len(), 1);
    let drawn: Vec<_> = model
        .annotations
        .iter()
        .filter(|a| !a.polylines.is_empty())
        .collect();
    assert_eq!(drawn.len(), 23, "every callout the part draws");
    for annotation in &drawn {
        assert_eq!(annotation.body_index, Some(0), "{}", annotation.name);
        assert!(
            annotation.anchor.is_some(),
            "{} has a label",
            annotation.name
        );
        assert!(annotation.polylines.iter().all(|line| line.len() >= 2));
    }

    // A dimension shows its value and bounds, a tolerance its kind, zone
    // and datums.
    let by_name = |name: &str| {
        model
            .annotations
            .iter()
            .find(|a| a.name == name)
            .unwrap_or_else(|| panic!("no annotation named {name}"))
    };
    let size = by_name("Linear Size.1");
    assert_eq!(size.kind, AnnotationKind::Dimension);
    assert_eq!(size.text, "Ø 35 0/-0.2");
    let flatness = by_name("Flatness.1");
    assert_eq!(flatness.kind, AnnotationKind::Tolerance);
    assert_eq!(flatness.text, "Flatness 0.2");
    let position = by_name("Position.1");
    assert_eq!(position.text, "Position 0.75 | A");
    let angle = by_name("Angular Size.1");
    assert_eq!(angle.text, "60° ±0.5°");
    assert_eq!(by_name("Text.1").kind, AnnotationKind::Note);
    assert_eq!(by_name("Simple Datum.1").kind, AnnotationKind::Datum);

    // The datums no callout is linked to are listed, drawing nothing.
    let datums: Vec<&str> = model
        .annotations
        .iter()
        .filter(|a| a.kind == AnnotationKind::Datum && a.polylines.is_empty())
        .map(|a| a.text.as_str())
        .collect();
    assert_eq!(datums, ["A", "B", "C"]);

    // What is drawn sits on the part, not somewhere far off.
    let (lo, hi) = model.bodies[0].bounds_mm.expect("the part has bounds");
    let size = (0..3).map(|i| hi[i] - lo[i]).fold(0.0_f32, f32::max);
    for annotation in &drawn {
        let [x, y, z] = annotation.anchor.unwrap();
        let near = |v: f32, i: usize| v > lo[i] - size && v < hi[i] + size;
        assert!(
            near(x, 0) && near(y, 1) && near(z, 2),
            "{} is drawn far from the part",
            annotation.name
        );
    }
}

#[test]
fn an_iges_part_brings_its_level_dimension_and_note() {
    let model = import("bracket_annotated.igs");
    assert_eq!(model.bodies.len(), 1);
    assert_eq!(model.bodies[0].layers, ["level 7"]);

    let dimension = model
        .annotations
        .iter()
        .find(|a| a.kind == AnnotationKind::Dimension)
        .expect("the dimension comes through");
    assert_eq!(dimension.text, "40.00", "the text the file draws");
    assert_eq!(
        dimension.polylines.len(),
        4,
        "two leaders, two witness lines"
    );
    assert_eq!(dimension.body_index, Some(0));
    let [x, y, _] = dimension.anchor.expect("the dimension has a label");
    assert!((x - 20.0).abs() < 1e-3, "centred over the length: {x}");
    assert!(y < 0.0, "beside the part, where it is drawn: {y}");

    let note = model
        .annotations
        .iter()
        .find(|a| a.kind == AnnotationKind::Note)
        .expect("the note comes through");
    assert_eq!(note.text, "PRINT IN PLA");
}

#[test]
fn a_file_without_annotations_brings_none() {
    let model = import("box_native.step");
    assert!(model.annotations.is_empty());
    assert!(model.bodies.iter().all(|b| b.layers.is_empty()));
}
