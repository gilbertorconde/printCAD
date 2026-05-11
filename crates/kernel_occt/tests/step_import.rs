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
    assert!(
        !imported.nodes.is_empty(),
        "STEP import should provide at least one hierarchy node"
    );
    for node in &imported.nodes {
        if let Some(parent) = node.parent_id {
            assert!(
                imported.nodes.iter().any(|n| n.id == parent),
                "parent node id {parent} is missing from payload"
            );
        }
        if let Some(body_index) = node.body_index {
            assert!(
                body_index < imported.bodies.len(),
                "node body index {body_index} out of bounds"
            );
        }
    }

    let mut total_triangles = 0usize;
    for body in &imported.bodies {
        assert!(
            !body.brep_blob.is_empty() || !body.mesh.positions.is_empty(),
            "expected BRep snapshot or inline mesh from fast import"
        );
        let mesh = if !body.mesh.positions.is_empty() {
            body.mesh.clone()
        } else {
            kernel
                .tessellate_step_brep(&body.brep_blob, &body.face_colors, &detail)
                .unwrap_or_else(|e| panic!("tessellate_step_brep failed: {e}"))
        };
        assert!(
            !mesh.positions.is_empty(),
            "tessellated body has no vertices"
        );
        assert!(
            !mesh.indices.is_empty(),
            "tessellated body has no triangles"
        );
        assert_eq!(mesh.indices.len() % 3, 0, "indices must form triangles");
        assert_eq!(
            mesh.positions.len(),
            mesh.normals.len(),
            "positions and normals must be aligned"
        );
        total_triangles += mesh.indices.len() / 3;
    }
    eprintln!(
        "Imported + tessellated `{}`: {} bodies, {} triangles",
        sample.display(),
        imported.bodies.len(),
        total_triangles
    );
}

#[test]
fn imported_node_body_indices_dont_collide_across_branches() {
    let Some(sample) = locate_sample() else {
        eprintln!("No STEP sample found; skipping branch-uniqueness test.");
        return;
    };

    let mut kernel = OcctKernel::new();
    kernel.initialize().expect("initialize OCCT kernel");
    let detail = TessellationSettings::default();
    let imported = kernel
        .import_step(&sample, &detail)
        .unwrap_or_else(|err| panic!("import_step failed for {}: {err}", sample.display()));

    use std::collections::HashMap;

    let by_id: HashMap<u64, &kernel_api::ImportedNode> =
        imported.nodes.iter().map(|n| (n.id, n)).collect();

    let is_ancestor_of = |ancestor_id: u64, descendant_id: u64| -> bool {
        let mut cursor = by_id.get(&descendant_id).and_then(|n| n.parent_id);
        while let Some(pid) = cursor {
            if pid == ancestor_id {
                return true;
            }
            cursor = by_id.get(&pid).and_then(|n| n.parent_id);
        }
        false
    };

    // Group nodes by body_index. With the previous `TDF_Label::Tag()`
    // collision bug, siblings in unrelated assembly branches could share a
    // body_index, which made the tree visibility toggle the wrong stage
    // element. The only legitimate sharing is between an instance node and
    // the referred prototype it expands into (ancestor relationship).
    let mut by_body_index: HashMap<usize, Vec<&kernel_api::ImportedNode>> = HashMap::new();
    for node in &imported.nodes {
        if let Some(idx) = node.body_index {
            by_body_index.entry(idx).or_default().push(node);
        }
    }

    for (idx, nodes) in &by_body_index {
        if nodes.len() < 2 {
            continue;
        }
        for (i, a) in nodes.iter().enumerate() {
            for b in &nodes[i + 1..] {
                let related = is_ancestor_of(a.id, b.id) || is_ancestor_of(b.id, a.id);
                assert!(
                    related,
                    "body_index {idx} is shared between unrelated tree branches: nodes {} (`{}`) and {} (`{}`)",
                    a.id,
                    a.name.as_deref().unwrap_or("<unnamed>"),
                    b.id,
                    b.name.as_deref().unwrap_or("<unnamed>"),
                );
            }
        }
    }
}

#[test]
fn deferred_pipeline_approximates_monolithic_tessellation() {
    let Some(sample) = locate_sample() else {
        eprintln!("No STEP sample found; skipping deferred vs monolithic test.");
        return;
    };

    let mut kernel = OcctKernel::new();
    kernel.initialize().expect("initialize OCCT kernel");
    // Force BRep snapshot so we exercise read → blob → tessellate (deferred) in this test.
    let mut detail = TessellationSettings::default();
    detail.persist_brep_snapshot = true;

    let fast = kernel.import_step(&sample, &detail).expect("fast import");
    let full = kernel
        .import_step_full_mesh(&sample, &detail)
        .expect("full mesh import");

    assert_eq!(fast.bodies.len(), full.bodies.len(), "body count mismatch");

    for (i, fb) in fast.bodies.iter().enumerate() {
        let defer_mesh = kernel
            .tessellate_step_brep(&fb.brep_blob, &fb.face_colors, &detail)
            .expect("deferred tessellation");
        let mono_mesh = &full.bodies[i].mesh;
        let tri_d = defer_mesh.indices.len() / 3;
        let tri_m = mono_mesh.indices.len() / 3;
        assert!(tri_d > 0 && tri_m > 0, "empty mesh body {i}");
        let rel = (tri_d as f64 - tri_m as f64).abs() / (tri_m.max(tri_d) as f64);
        assert!(
            rel <= 0.12,
            "triangle count drift too large for body {i}: deferred {tri_d} vs monolithic {tri_m} (relative {rel:.3})"
        );
    }
}
