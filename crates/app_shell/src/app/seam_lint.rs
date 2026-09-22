//! The host names no workbench. Every bench-dependent behaviour reaches
//! the host through the `Workbench` trait and the registry, so a line in
//! this crate that names a bench crate, a bench id or a bench feature type
//! is a bypass, and this test refuses it.

use std::path::Path;

/// What a bypass looks like: a bench crate, a bench id string, a bench
/// feature type, or the datum kind Part Design claims.
const BYPASSES: &[&str] = &[
    "wb_part",
    "wb_sketch",
    "\"wb.part\"",
    "\"wb.sketch\"",
    "PartFeature",
    "SketchFeature",
    "core.datum",
];

fn offending_lines(dir: &Path, hits: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("source directory reads") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            offending_lines(&path, hits);
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "rs") || path.ends_with("seam_lint.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("source file reads");
        for (number, line) in text.lines().enumerate() {
            if BYPASSES.iter().any(|needle| line.contains(needle)) {
                hits.push(format!(
                    "{}:{}: {}",
                    path.display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }
}

#[test]
fn the_host_names_no_workbench() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    offending_lines(&src, &mut hits);
    assert!(
        hits.is_empty(),
        "the host reaches past the workbench trait:\n{}",
        hits.join("\n")
    );
}
