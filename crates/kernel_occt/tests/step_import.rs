//! End-to-end smoke test: confirms the OCCT-backed STEP loader can read a
//! real STEP file and produce a triangulated mesh.
//!
//! Tests run against the committed fixture in `tests/data/box.step` by
//! default; set `PRINTCAD_TEST_STEP_FILE` to exercise a richer model (e.g. a
//! KiCad sample with assemblies and colors).

use kernel_api::{Kernel, TessellationSettings};
use kernel_occt::OcctKernel;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

/// OCCT's STEP machinery relies on process-global state (Interface statics,
/// XCAF sessions) and crashes when two kernels import concurrently, so the
/// tests in this binary serialize on this mutex. The production app is safe
/// because all OCCT work funnels through the single kernel-worker thread.
static OCCT_SERIAL: Mutex<()> = Mutex::new(());

fn occt_guard() -> MutexGuard<'static, ()> {
    OCCT_SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn locate_sample() -> PathBuf {
    if let Ok(env_path) = std::env::var("PRINTCAD_TEST_STEP_FILE") {
        let candidate = PathBuf::from(&env_path);
        assert!(
            candidate.is_file(),
            "PRINTCAD_TEST_STEP_FILE points to a missing file: {env_path}"
        );
        return candidate;
    }

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/box.step");
    assert!(
        fixture.is_file(),
        "bundled STEP fixture missing: {}",
        fixture.display()
    );
    fixture
}

#[test]
fn imports_real_world_step_file() {
    let _serial = occt_guard();
    let sample = locate_sample();

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
    let _serial = occt_guard();
    let sample = locate_sample();

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
    let _serial = occt_guard();
    let sample = locate_sample();

    let mut kernel = OcctKernel::new();
    kernel.initialize().expect("initialize OCCT kernel");
    // Force BRep snapshot so we exercise read → blob → tessellate (deferred) in this test.
    let detail = TessellationSettings {
        persist_brep_snapshot: true,
        ..TessellationSettings::default()
    };

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
