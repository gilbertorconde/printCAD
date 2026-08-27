//! Extract one part from an assembly STEP into a standalone STEP file, so a
//! misbehaving part can be tested in isolation.
//!
//! ```text
//! cargo run --release -p kernel_ogeom --example extract_part -- \
//!     <assembly.step> <name-filter> <out.step>
//! ```

use ogeom::core::Tolerances;
use ogeom::doc::ProductKind;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: extract_part <assembly.step> <name-filter> <out.step>");
    let name_filter = args.next().expect("name filter").to_lowercase();
    let out = args.next().expect("output path");

    let bytes = std::fs::read(&path).expect("read STEP");
    let text = String::from_utf8_lossy(&bytes);
    let tol = Tolerances::millimetres();
    let import = ogeom::io::step::read_step(&text, tol).expect("parse STEP");

    let mut matches = Vec::new();
    for (_, product) in import.document.products() {
        if let ProductKind::Part { shape } = &product.kind {
            if product.name.to_lowercase().contains(&name_filter) {
                matches.push((product.name.clone(), shape.clone()));
            }
        }
    }
    assert!(
        !matches.is_empty(),
        "no part matches `{name_filter}`; products: {:?}",
        import
            .document
            .products()
            .map(|(_, p)| p.name.clone())
            .collect::<Vec<_>>()
    );
    println!(
        "matched {} part(s): {:?}",
        matches.len(),
        matches.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );

    // Move each matched solid into a fresh document via the native subset
    // round-trip (write carries the closure of its roots), then export.
    let mut doc = ogeom::doc::Document::new();
    let options = ogeom::io::native::WriteOptions {
        triangulations: false,
    };
    for (name, shape) in &matches {
        let subset = ogeom::io::native::write(
            import.document.model(),
            std::slice::from_ref(shape),
            options,
        )
        .expect("subset write");
        let absorbed =
            ogeom::io::native::read_into(doc.model_mut(), &subset).expect("absorb subset");
        let root = absorbed.shapes.first().expect("subset has a root").clone();
        doc.add_part(name.clone(), root);
    }

    let step = ogeom::io::step::write_step(&doc, tol).expect("write STEP");
    std::fs::write(&out, &step).expect("write output");
    println!("wrote {} ({} bytes)", out, step.len());
}
