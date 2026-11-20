//! Import a STEP file, list every body the checker calls broken with its
//! findings, run the repair on each distinct shape and say what is left.
//!
//! ```text
//! cargo run --release -p kernel_ogeom --example health_probe -- <file.step> [out-dir]
//! ```
//! With `out-dir`, each broken shape's snapshot is written there as
//! `<n>.ogeom` for a kernel repro.

use std::collections::HashMap;

use kernel_api::TessellationSettings;
use kernel_ogeom::OgeomKernel;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: health_probe <file.step> [out-dir]");
    let out = args.next();
    let detail = TessellationSettings::default();
    let started = std::time::Instant::now();
    let model = OgeomKernel::new()
        .import_step(std::path::Path::new(&path), &detail)
        .expect("imports");
    println!(
        "{} bodies in {:.1}s",
        model.bodies.len(),
        started.elapsed().as_secs_f32()
    );
    // One entry per distinct shape: its names and how often it appears.
    let mut shapes: HashMap<&[u8], (Vec<String>, usize, usize)> = HashMap::new();
    for (i, body) in model.bodies.iter().enumerate() {
        if !body.health.as_ref().is_some_and(|h| h.is_broken()) {
            continue;
        }
        let entry = shapes.entry(&body.brep_blob).or_insert((Vec::new(), 0, i));
        let name = body.name.clone().unwrap_or_else(|| format!("#{i}"));
        if !entry.0.contains(&name) {
            entry.0.push(name);
        }
        entry.1 += 1;
    }
    let mut shapes: Vec<_> = shapes.into_iter().collect();
    shapes.sort_by_key(|(_, (_, _, first))| *first);
    println!("{} distinct broken shapes", shapes.len());
    let mut kinds_before: HashMap<String, usize> = HashMap::new();
    let mut kinds_after: HashMap<String, usize> = HashMap::new();
    let kind = |f: &str| f.split(':').next().unwrap_or(f).trim().to_string();
    for (n, (blob, (names, count, first))) in shapes.iter().enumerate() {
        let body = &model.bodies[*first];
        let health = body.health.as_ref().unwrap();
        println!(
            "\n[{n}] {} (x{count}), {} broken, {} suspect",
            names.join(" / "),
            health.broken,
            health.suspect
        );
        for f in &health.findings {
            println!("    before: {f}");
            *kinds_before.entry(kind(f)).or_default() += 1;
        }
        if let Some(dir) = &out {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(format!("{dir}/{n}.ogeom"), blob).unwrap();
        }
        let t = std::time::Instant::now();
        match OgeomKernel::new().repair_brep(blob, &body.face_colors, &detail) {
            Ok(r) => {
                println!(
                    "    repair ({:.0} ms): {}; now {} broken, {} suspect",
                    t.elapsed().as_secs_f64() * 1000.0,
                    if r.mended.is_empty() {
                        "nothing mended".to_string()
                    } else {
                        r.mended.join(", ")
                    },
                    r.health.broken,
                    r.health.suspect
                );
                for f in r.health.findings.iter().filter(|f| f.contains("broken")) {
                    println!("    after: {f}");
                    *kinds_after.entry(kind(f)).or_default() += 1;
                }
                if let Some(dir) = &out
                    && r.health.broken > 0
                {
                    std::fs::write(format!("{dir}/{n}.repaired.ogeom"), &r.brep_blob).unwrap();
                }
            }
            Err(e) => println!("    repair failed: {e}"),
        }
    }
    println!("\nfinding kinds before: {kinds_before:#?}");
    println!("finding kinds left after repair: {kinds_after:#?}");
}
