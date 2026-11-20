//! Every mesh a release kept as an entry of its own reads back, whatever
//! layout that release wrote it in.

use core_document::Document;

/// Each `.prtcad` in the app's corpus, by release, its meshes present after
/// loading: a mesh entry that does not read leaves the empty mesh the
/// document's JSON carries in its place.
#[test]
fn every_corpus_mesh_entry_reads() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../app_shell/tests/corpus");
    let (mut checked, mut missing) = (0, Vec::new());
    for release in std::fs::read_dir(&root).expect("corpus").flatten() {
        for file in std::fs::read_dir(release.path())
            .expect("release")
            .flatten()
        {
            let path = file.path();
            if path.extension().is_none_or(|e| e != "prtcad") {
                continue;
            }
            let bytes = std::fs::read(&path).expect("read");
            let doc = Document::load_from_bytes(bytes).expect("load");
            for body in doc.bodies() {
                for geom in [
                    doc.derived().imported_geometry(body.id),
                    doc.mesh_start(body.id),
                ]
                .into_iter()
                .flatten()
                {
                    let Some(entry) = &geom.mesh_path else {
                        continue;
                    };
                    checked += 1;
                    if geom.mesh.positions.is_empty() || geom.mesh.indices.is_empty() {
                        missing.push(format!("{}: {entry}", path.display()));
                    }
                }
            }
        }
    }
    assert!(checked > 0, "the corpus keeps meshes apart");
    assert!(
        missing.is_empty(),
        "meshes that did not read:\n{}",
        missing.join("\n")
    );
}
