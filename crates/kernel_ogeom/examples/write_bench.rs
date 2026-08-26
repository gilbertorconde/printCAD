//! Time ONLY the per-solid snapshot writes: parse once, then serialize each
//! solid's blob. No derived numbers, no cross-phase subtraction.

use std::time::Instant;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: write_bench <file.step>");
    let bytes = std::fs::read(&path).expect("read STEP");
    let text = String::from_utf8_lossy(&bytes);
    let import = ogeom::io::step::read_step(&text, ogeom::core::Tolerances::millimetres())
        .expect("parse STEP");
    let model = import.document.model();
    let options = ogeom::io::native::WriteOptions {
        triangulations: false,
    };

    let t = Instant::now();
    let mut total_bytes = 0usize;
    for solid in &import.solids {
        let blob = ogeom::io::native::write(model, std::slice::from_ref(solid), options)
            .expect("write solid");
        total_bytes += blob.len();
    }
    let elapsed = t.elapsed().as_secs_f64();
    println!(
        "wrote {} solids, {} bytes, in {:.2} s ({:.1} ms/solid)",
        import.solids.len(),
        total_bytes,
        elapsed,
        elapsed * 1000.0 / import.solids.len() as f64
    );
}
