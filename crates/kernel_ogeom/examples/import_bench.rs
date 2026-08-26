//! Phase breakdown of a STEP import, so optimization targets the real cost.
//!
//! ```text
//! cargo run --release -p kernel_ogeom --example import_bench -- <file.step>
//! ```
//!
//! Reports the four phases separately: reading the file, parsing it into a
//! model, the per-solid loop (colours, snapshot blobs, bounds), and the
//! deferred per-body tessellation the app runs afterwards.

use std::time::Instant;

use kernel_api::{Kernel, TessellationSettings};
use kernel_ogeom::OgeomKernel;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("printcad.kernel=info")),
        )
        .init();

    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: import_bench <file.step> [thread-count]");
        std::process::exit(2);
    };
    if let Some(threads) = std::env::args()
        .nth(2)
        .and_then(|t| t.parse::<usize>().ok())
    {
        ogeom::core::parallel::set_threads(threads);
    }
    println!(
        "threads: {}  file: {path}",
        ogeom::core::parallel::threads()
    );

    // Phase 1: read the bytes.
    let t = Instant::now();
    let bytes = std::fs::read(&path).expect("read STEP file");
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let read_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!(
        "read file      {read_ms:9.0} ms  ({} MiB)",
        text.len() >> 20
    );

    // Phase 2: parse into a model. Isolated from the rest so we can see how
    // much of the wall clock is the (sequential, kernel-side) reader.
    let t = Instant::now();
    let import = ogeom::io::step::read_step(&text, ogeom::core::Tolerances::millimetres())
        .expect("parse STEP");
    let parse_ms = t.elapsed().as_secs_f64() * 1000.0;
    let solids = import.solids.len();
    println!("parse          {parse_ms:9.0} ms  ({solids} solids)");
    drop(import);
    drop(text);

    // Phase 3: the whole import as the app calls it — parse again, plus the
    // per-solid loop. Subtracting phase 2 gives the loop's own cost.
    let mut kernel = OgeomKernel::new();
    kernel.initialize().expect("initialize kernel");
    let mut detail = TessellationSettings::default();
    if std::env::var("BENCH_NO_BLOBS").is_ok() {
        detail.persist_brep_snapshot = false;
    }

    let t = Instant::now();
    let model = kernel
        .import_step(std::path::Path::new(&path), &detail)
        .expect("import STEP");
    let import_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!(
        "import total   {import_ms:9.0} ms  (per-solid loop ≈ {:.0} ms)",
        import_ms - parse_ms - read_ms
    );

    // The import now meshes from the model it already has in memory, so the
    // bodies come back ready to draw.
    let triangles: usize = model.bodies.iter().map(|b| b.mesh.indices.len() / 3).sum();
    let meshed = model
        .bodies
        .iter()
        .filter(|b| !b.mesh.indices.is_empty())
        .count();
    println!("  meshed inline: {meshed} bodies, {triangles} triangles");

    // For reference: what a deferred pass would have cost, having to parse
    // every snapshot back before it could mesh anything.
    let jobs: Vec<(&[u8], &[[f32; 3]])> = model
        .bodies
        .iter()
        .filter(|b| !b.brep_blob.is_empty())
        .map(|b| (b.brep_blob.as_slice(), b.face_colors.as_slice()))
        .collect();
    if !jobs.is_empty() {
        let t = Instant::now();
        let _ = kernel.tessellate_step_breps(&jobs, &detail);
        println!(
            "  (a deferred re-tessellation from snapshots would add {:.0} ms)",
            t.elapsed().as_secs_f64() * 1000.0
        );
    }

    println!("─────────────────────────");
    // The standalone parse in phase 2 is measurement overhead; the pipeline
    // the app actually runs is read → import → tessellate.
    println!("total          {:9.0} ms", read_ms + import_ms);
}
