//! End-to-end smoke test: confirms the OCCT-backed STEP loader can read a
//! real-world STEP file and produce a triangulated mesh.
//!
//! The test looks for the first available STEP file under `/usr/share/kicad`
//! (or honours `PRINTCAD_TEST_STEP_FILE` for full control). When no STEP
//! sample is reachable, the test is skipped with a clear message.

use kernel_api::{Kernel, TessellationSettings};
use kernel_occt::OcctKernel;
use std::path::PathBuf;

fn locate_sample() -> Option<PathBuf> {
    if let Ok(env_path) = std::env::var("PRINTCAD_TEST_STEP_FILE") {
        let candidate = PathBuf::from(env_path);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    // Walk a couple of common system directories looking for any STEP file.
    let roots = [
        "/usr/share/kicad",
        "/usr/share/freecad",
        "/usr/share/opencascade",
    ];
    for root in roots {
        let root = PathBuf::from(root);
        if !root.exists() {
            continue;
        }
        if let Some(found) = walk_for_step(&root, 6) {
            return Some(found);
        }
    }
    None
}

fn walk_for_step(dir: &std::path::Path, depth: usize) -> Option<PathBuf> {
    if depth == 0 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = walk_for_step(&path, depth - 1) {
                return Some(found);
            }
        } else if matches!(
            path.extension().and_then(|s| s.to_str()),
            Some("step") | Some("stp") | Some("STEP") | Some("STP")
        ) {
            return Some(path);
        }
    }
    None
}

#[test]
fn imports_real_world_step_file() {
    let Some(sample) = locate_sample() else {
        eprintln!("No STEP sample found; skipping import smoke test.");
        return;
    };

    let mut kernel = OcctKernel::new();
    kernel.initialize().expect("initialize OCCT kernel");

    let detail = TessellationSettings::default();
    let imported = kernel
        .import_step(&sample, &detail)
        .unwrap_or_else(|err| panic!("import_step failed for {}: {err}", sample.display()));

    assert!(
        !imported.bodies.is_empty(),
        "STEP import produced no bodies for {}",
        sample.display()
    );

    let mut total_triangles = 0usize;
    for body in &imported.bodies {
        assert!(
            !body.mesh.positions.is_empty(),
            "imported body has no vertices"
        );
        assert!(
            !body.mesh.indices.is_empty(),
            "imported body has no triangles"
        );
        assert_eq!(body.mesh.indices.len() % 3, 0, "indices must form triangles");
        assert_eq!(
            body.mesh.positions.len(),
            body.mesh.normals.len(),
            "positions and normals must be aligned"
        );
        total_triangles += body.mesh.indices.len() / 3;
    }
    eprintln!(
        "Imported `{}`: {} bodies, {} triangles",
        sample.display(),
        imported.bodies.len(),
        total_triangles
    );
}
