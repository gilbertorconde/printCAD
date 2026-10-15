use core_document::{Compression, Document};
use project_tools::document_import::{ImportedObject, import_document};

#[test]
fn mismatched_profile_frames_and_placed_boolean_results_are_reported() {
    let (_, source) = super::fixture("placement_frames");
    let project = import_document(&source, "placements").unwrap();
    assert!(
        project.build_failures.is_empty(),
        "{:?}",
        project.build_failures
    );
    assert!(project.objects.contains_key("Inside"));
    assert!(!project.objects.contains_key("Outside"));
    assert!(!project.objects.contains_key("Moved"));
    assert!(
        project
            .findings
            .iter()
            .any(|f| f.object == "Outside" && f.reason.contains("coordinate"))
    );
    assert!(
        project
            .findings
            .iter()
            .any(|f| f.object == "Moved" && f.reason.contains("placement"))
    );
}

#[test]
fn missing_and_cyclic_dependencies_reject_only_their_consumers_once_in_either_order() {
    for target in ["Missing", "Boss"] {
        for reverse in [false, true] {
            let (_, mut source) = super::fixture("body_features");
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
            let mut project = import_document(&source, "partial").unwrap();
            assert!(
                project.build_failures.is_empty(),
                "{:?}",
                project.build_failures
            );
            for name in ["Pad", "Hole", "Boss", "Notch", "Recess"] {
                assert!(!project.objects.contains_key(name), "{name}");
                let findings: Vec<_> = project
                    .findings
                    .iter()
                    .filter(|f| f.object == name)
                    .collect();
                assert_eq!(findings.len(), 1, "{name}: {findings:?}");
                assert_eq!(findings[0].category, "dependency");
                assert!(findings[0].reason.contains(target), "{findings:?}");
            }
            assert!(!project.objects.contains_key("Missing"));
            let bytes = project.document.save_to_bytes(Compression::None).unwrap();
            let loaded = Document::load_from_bytes(bytes).unwrap();
            assert_eq!(loaded.bodies().len(), 4);
            for name in ["Disc", "PartBox", "PartCylinder", "Cut"] {
                let ImportedObject::Solid(body) = project.objects[name] else {
                    panic!("{name}")
                };
                assert!(loaded.imported_brep_blob(body).is_some(), "{name}");
            }
        }
    }
}
