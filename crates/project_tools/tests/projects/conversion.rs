use core_document::{Compression, Document};
use project_tools::document_import::{BodyKind, import_document};

#[test]
fn what_cannot_be_rebuilt_comes_in_as_its_saved_shape_where_the_file_has_it() {
    let (archive, source) = super::fixture("placement_frames");
    let project = import_document(&source, &archive.shapes, "placements").unwrap();
    assert!(
        project.build_failures.is_empty(),
        "{:?}",
        project.build_failures
    );
    assert!(project.objects.contains_key("Inside"));
    for (name, why) in [("Outside", "coordinate"), ("Moved", "placement")] {
        let body = project.objects[name].body().unwrap();
        let converted = project.bodies.iter().find(|b| b.body == body).unwrap();
        assert_eq!(converted.kind, BodyKind::SavedShape, "{name}");
        assert!(
            project
                .findings
                .iter()
                .any(|f| f.object == name && f.category == "saved shape" && f.reason.contains(why))
        );
        let saved = archive
            .shape(source.object(name).unwrap().shape_file("Shape").unwrap())
            .unwrap();
        let frame = project_tools::document_import::parent_placement(&source, name).unwrap();
        let geometry = project.document.imported_geometry(body).unwrap();
        let parity =
            project_tools::parity::compare_shape(saved, frame, geometry.bounds_mm, 0).unwrap();
        assert!(parity.bounds_match(), "{name}: {parity:?}");
    }
}

#[test]
fn missing_and_cyclic_dependencies_reject_only_their_consumers_once_in_either_order() {
    for target in ["Missing", "Boss"] {
        for reverse in [false, true] {
            let (archive, mut source) = super::fixture("body_features");
            let pad = source.objects.iter_mut().find(|o| o.name == "Pad").unwrap();
            pad.properties.get_mut("BaseFeature").unwrap().value[0]
                .attrs
                .iter_mut()
                .find(|a| a.0 == "value")
                .unwrap()
                .1 = target.into();
            if reverse {
                source.objects.reverse();
            }
            let mut project = import_document(&source, &archive.shapes, "partial").unwrap();
            // A history the kernel cannot build shows the shape it saved.
            for name in project.build_failures.keys() {
                assert!(project.stood_in.contains(name), "{name}");
            }
            // What the broken link reaches is reported once; the rest of
            // the history comes in on the shapes the file saved.
            for name in ["Pad", "Hole", "Boss", "Notch", "Recess"] {
                let findings: Vec<_> = project
                    .findings
                    .iter()
                    .filter(|f| f.object == name && f.category == "dependency")
                    .collect();
                let reported = usize::from(!project.objects.contains_key(name));
                assert_eq!(findings.len(), reported, "{name}: {findings:?}");
                if reported == 1 {
                    assert!(findings[0].reason.contains(target), "{findings:?}");
                }
            }
            assert!(project.objects.contains_key("Recess"));
            assert!(!project.objects.contains_key("Missing"));
            let bytes = project.document.save_to_bytes(Compression::None).unwrap();
            let loaded = Document::load_from_bytes(bytes).unwrap();
            for name in ["Disc", "PartBox", "PartCylinder", "Cut", "Recess"] {
                let body = project.objects[name].body().unwrap();
                assert!(loaded.imported_brep_blob(body).is_some(), "{name}");
            }
        }
    }
}
