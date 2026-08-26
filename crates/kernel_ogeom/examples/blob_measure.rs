//! How large is a one-solid snapshot, versus the whole model?

use ogeom::core::Tolerances;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: blob_measure <file.step>");
    let text = std::fs::read_to_string(&path).expect("read STEP");
    let import = ogeom::io::step::read_step(&text, Tolerances::millimetres()).expect("parse");
    let model = import.document.model();
    let options = ogeom::io::native::WriteOptions {
        triangulations: false,
    };

    let one = ogeom::io::native::write(model, std::slice::from_ref(&import.solids[0]), options)
        .expect("write one");
    let all = ogeom::io::native::write(model, &import.solids, options).expect("write all");

    println!("STEP file:                {:>12} bytes", text.len());
    println!("solids:                   {:>12}", import.solids.len());
    println!("blob of solids[0]:        {:>12} bytes", one.len());
    println!("blob of ALL solids:       {:>12} bytes", all.len());
    println!(
        "sum if written per-solid: {:>12} bytes (≈ {} × the one-root blob)",
        one.len() * import.solids.len(),
        import.solids.len()
    );
}
