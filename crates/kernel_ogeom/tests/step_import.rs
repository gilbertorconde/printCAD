//! End-to-end smoke test: confirms the ogeom-backed STEP loader can read a
//! real STEP file and produce a triangulated mesh.
//!
//! Tests run against the committed fixture in `tests/data/box.step` by
//! default; set `PRINTCAD_TEST_STEP_FILE` to exercise a richer model (e.g. a
//! KiCad sample with assemblies and colors).

use kernel_api::{Kernel, TessellationSettings};
use kernel_ogeom::OgeomKernel;
use std::path::PathBuf;

fn locate_sample() -> PathBuf {
    if let Ok(env_path) = std::env::var("PRINTCAD_TEST_STEP_FILE") {
        let candidate = PathBuf::from(&env_path);
        assert!(
            candidate.is_file(),
            "PRINTCAD_TEST_STEP_FILE points to a missing file: {env_path}"
        );
        return candidate;
    }

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/box_native.step");
    assert!(
        fixture.is_file(),
        "bundled STEP fixture missing: {}",
        fixture.display()
    );
    fixture
}

/// Real-world STEP files from OCCT-based exporters (KiCad, etc.) encode edge
/// geometry as `SURFACE_CURVE`/`SEAM_CURVE` wrappers, which the ogeom STEP
/// reader does not unwrap yet — every face refuses and the import fails.
/// Un-ignore when the reader learns those entities (kernel work item G8).
#[test]
fn imports_occt_flavoured_step_file() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/box.step");
    let mut kernel = OgeomKernel::new();
    kernel.initialize().expect("initialize ogeom kernel");
    let imported = kernel
        .import_step(&fixture, &TessellationSettings::default())
        .expect("OCCT-flavoured STEP import");
    assert_eq!(imported.bodies.len(), 1);
}

#[test]
fn imports_real_world_step_file() {
    let sample = locate_sample();

    let mut kernel = OgeomKernel::new();
    kernel.initialize().expect("initialize ogeom kernel");

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
            "expected shape snapshot or inline mesh from fast import"
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
    let sample = locate_sample();

    let mut kernel = OgeomKernel::new();
    kernel.initialize().expect("initialize ogeom kernel");
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

    // The only legitimate body_index sharing is between an instance node and
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
    let sample = locate_sample();

    let mut kernel = OgeomKernel::new();
    kernel.initialize().expect("initialize ogeom kernel");
    // Force snapshot persistence so we exercise read → blob → tessellate
    // (deferred) in this test.
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

#[test]
fn snapshot_blob_round_trips_through_native_format() {
    let sample = locate_sample();

    let mut kernel = OgeomKernel::new();
    kernel.initialize().expect("initialize ogeom kernel");
    let detail = TessellationSettings {
        persist_brep_snapshot: true,
        ..TessellationSettings::default()
    };

    let imported = kernel.import_step(&sample, &detail).expect("import");
    for (i, body) in imported.bodies.iter().enumerate() {
        assert!(!body.brep_blob.is_empty(), "body {i} missing snapshot");
        let mesh = kernel
            .tessellate_step_brep(&body.brep_blob, &body.face_colors, &detail)
            .expect("tessellate snapshot");
        let (blob_min, blob_max) = mesh.bounds().expect("mesh bounds");
        let (imp_min, imp_max) = body.bounds_mm.expect("import bounds");
        for axis in 0..3 {
            assert!(
                (blob_min[axis] - imp_min[axis]).abs() < 0.5
                    && (blob_max[axis] - imp_max[axis]).abs() < 0.5,
                "body {i} bounds drifted through the snapshot round-trip: \
                 mesh {blob_min:?}..{blob_max:?} vs import {imp_min:?}..{imp_max:?}"
            );
        }
    }
}

/// The mesh must not depend on how many threads produced it.
///
/// Faces are triangulated in parallel and bodies are prepared in parallel;
/// both use an order-preserving map, so the result is the caller's business
/// and the thread count is not.
#[test]
fn the_import_is_identical_at_any_thread_count() {
    let sample = locate_sample();
    let detail = TessellationSettings::default();

    let import_with = |threads: usize| {
        ogeom::core::parallel::set_threads(threads);
        let mut kernel = OgeomKernel::new();
        kernel.initialize().expect("initialize ogeom kernel");
        kernel.import_step(&sample, &detail).expect("import")
    };

    let single = import_with(1);
    let many = import_with(8);
    // Leave the process-wide setting as we found it.
    ogeom::core::parallel::set_threads(0);

    assert_eq!(single.bodies.len(), many.bodies.len(), "body count");
    for (i, (a, b)) in single.bodies.iter().zip(&many.bodies).enumerate() {
        assert_eq!(a.name, b.name, "body {i} name");
        assert_eq!(a.brep_blob, b.brep_blob, "body {i} snapshot bytes");
        assert_eq!(a.face_colors, b.face_colors, "body {i} colours");
        assert_eq!(a.bounds_mm, b.bounds_mm, "body {i} bounds");
        assert_eq!(
            a.mesh.positions, b.mesh.positions,
            "body {i} vertex positions must not depend on the thread count"
        );
        assert_eq!(a.mesh.indices, b.mesh.indices, "body {i} triangles");
        assert_eq!(a.mesh.normals, b.mesh.normals, "body {i} normals");
        assert_eq!(a.mesh.edges, b.mesh.edges, "body {i} outline edges");
    }
}

/// An assembly's parts must arrive where the assembly puts them.
///
/// The raw `MANIFOLD_SOLID_BREP` shapes are in part-local coordinates; only
/// the product tree says where each part sits. Importing the former gives
/// parts that look right alone and are scattered in relation to each other,
/// so this pins the world-space extent against the placements.
#[test]
fn assembly_parts_arrive_in_world_space() {
    let sample = match std::env::var("PRINTCAD_TEST_ASSEMBLY_STEP") {
        Ok(p) => PathBuf::from(p),
        // The bundled fixture is a single body; without an assembly file
        // there is nothing to place, so the check is vacuous.
        Err(_) => return,
    };
    if !sample.is_file() {
        return;
    }

    let mut kernel = OgeomKernel::new();
    kernel.initialize().expect("initialize ogeom kernel");
    let imported = kernel
        .import_step(&sample, &TessellationSettings::default())
        .expect("assembly import");

    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    let mut distinct_origins = 0usize;
    for body in &imported.bodies {
        let Some((bmin, bmax)) = body.bounds_mm else {
            continue;
        };
        for axis in 0..3 {
            lo[axis] = lo[axis].min(bmin[axis]);
            hi[axis] = hi[axis].max(bmax[axis]);
        }
        // A body whose own box starts at the origin on every axis is the
        // signature of an unplaced, part-local shape.
        if bmin.iter().all(|v| v.abs() < 1e-3) {
            distinct_origins += 1;
        }
    }

    assert!(
        imported.bodies.len() > 1,
        "expected an assembly with several bodies"
    );
    assert!(
        distinct_origins < imported.bodies.len() / 2,
        "{distinct_origins} of {} bodies sit at the origin — parts are being \
         imported in part-local coordinates instead of placed",
        imported.bodies.len()
    );
    eprintln!(
        "assembly envelope: x {:.0}..{:.0}  y {:.0}..{:.0}  z {:.0}..{:.0} ({} bodies)",
        lo[0],
        hi[0],
        lo[1],
        hi[1],
        lo[2],
        hi[2],
        imported.bodies.len()
    );
}

/// A face whose boundary hovers off its surface is named by the reader and
/// healed by the import at its wider cap — the body draws whole instead of
/// with a hole. The fixture is the kernel's own hovering-face acceptance
/// file (a boundary 3 mm off a planar B-spline surface; the reader's own
/// healing stops at 1 mm).
#[test]
fn an_untrimmed_face_is_healed_at_import_and_draws() {
    let step = r#"ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('hover','2026-08-26',(''),(''),'','','');
FILE_SCHEMA(('AUTOMOTIVE_DESIGN'));
ENDSEC;
DATA;
#1=CARTESIAN_POINT('',(0.,0.,0.));
#2=CARTESIAN_POINT('',(10.,0.,0.));
#3=CARTESIAN_POINT('',(0.,10.,0.));
#4=CARTESIAN_POINT('',(10.,10.,0.));
#5=B_SPLINE_SURFACE_WITH_KNOTS('',1,1,((#1,#3),(#2,#4)),.UNSPECIFIED.,.F.,.F.,.F.,(2,2),(2,2),(0.,10.),(0.,10.),.UNSPECIFIED.);
#10=CARTESIAN_POINT('',(0.,0.,3.));
#11=CARTESIAN_POINT('',(10.,0.,3.));
#12=CARTESIAN_POINT('',(10.,10.,3.));
#13=CARTESIAN_POINT('',(0.,10.,3.));
#14=VERTEX_POINT('',#10);
#15=VERTEX_POINT('',#11);
#16=VERTEX_POINT('',#12);
#17=VERTEX_POINT('',#13);
#20=DIRECTION('',(1.,0.,0.));
#21=DIRECTION('',(0.,1.,0.));
#22=DIRECTION('',(-1.,0.,0.));
#23=DIRECTION('',(0.,-1.,0.));
#24=VECTOR('',#20,1.);
#25=VECTOR('',#21,1.);
#26=VECTOR('',#22,1.);
#27=VECTOR('',#23,1.);
#30=LINE('',#10,#24);
#31=LINE('',#11,#25);
#32=LINE('',#12,#26);
#33=LINE('',#13,#27);
#40=EDGE_CURVE('',#14,#15,#30,.T.);
#41=EDGE_CURVE('',#15,#16,#31,.T.);
#42=EDGE_CURVE('',#16,#17,#32,.T.);
#43=EDGE_CURVE('',#17,#14,#33,.T.);
#50=ORIENTED_EDGE('',*,*,#40,.T.);
#51=ORIENTED_EDGE('',*,*,#41,.T.);
#52=ORIENTED_EDGE('',*,*,#42,.T.);
#53=ORIENTED_EDGE('',*,*,#43,.T.);
#54=EDGE_LOOP('',(#50,#51,#52,#53));
#55=FACE_OUTER_BOUND('',#54,.T.);
#56=ADVANCED_FACE('',(#55),#5,.T.);
#57=CLOSED_SHELL('',(#56));
#58=MANIFOLD_SOLID_BREP('',#57);
ENDSEC;
END-ISO-10303-21;
"#;
    let dir = std::env::temp_dir();
    let path = dir.join(format!("printcad_heal_{}.step", std::process::id()));
    std::fs::write(&path, step).expect("write fixture");

    let mut kernel = OgeomKernel::new();
    kernel.initialize().expect("initialize");
    let imported = kernel
        .import_step(&path, &TessellationSettings::default())
        .expect("import with healing");
    let _ = std::fs::remove_file(&path);

    assert_eq!(imported.bodies.len(), 1);
    assert!(
        !imported.bodies[0].mesh.positions.is_empty(),
        "the healed face must triangulate — an empty mesh means the gap survived"
    );
}
