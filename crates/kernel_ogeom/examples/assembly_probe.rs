//! What an assembly's placement structure looks like, versus what the
//! importer currently takes from it.
//!
//! ```text
//! cargo run --release -p kernel_ogeom --example assembly_probe -- <file.step>
//! ```

use ogeom::core::Tolerances;
use ogeom::doc::ProductKind;

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: assembly_probe <file.step>");
        std::process::exit(2);
    };
    let bytes = std::fs::read(&path).expect("read STEP file");
    let text = String::from_utf8_lossy(&bytes);
    let tol = Tolerances::millimetres();
    let import = ogeom::io::step::read_step(&text, tol).expect("parse STEP");
    let doc = &import.document;

    let parts = doc
        .products()
        .filter(|(_, p)| matches!(p.kind, ProductKind::Part { .. }))
        .count();
    let assemblies = doc
        .products()
        .filter(|(_, p)| matches!(p.kind, ProductKind::Assembly { .. }))
        .count();

    let mut occurrences = 0usize;
    let mut placed = 0usize;
    for root in doc.roots() {
        let Ok(occs) = doc.occurrences_of(root) else {
            continue;
        };
        for occ in &occs {
            occurrences += 1;
            if !occ.shape.location().is_identity() {
                placed += 1;
            }
        }
    }

    println!(
        "products: {parts} parts, {assemblies} assemblies, {} roots",
        doc.roots().len()
    );
    println!(
        "solids     (what the importer takes today): {}",
        import.solids.len()
    );
    println!("occurrences (placed parts, world space):    {occurrences}");
    println!("  carrying a non-identity placement:        {placed}");
    if occurrences > import.solids.len() {
        println!(
            "\n=> {} placed instances are never imported at all",
            occurrences - import.solids.len()
        );
    }
    // Would switching to occurrences lose any solid?
    let mut part_shapes = Vec::new();
    for (_, product) in doc.products() {
        if let ProductKind::Part { shape } = &product.kind {
            part_shapes.push(shape.clone());
        }
    }
    let unreachable = import
        .solids
        .iter()
        .filter(|solid| {
            !part_shapes.iter().any(|ps| {
                ps.is_same(solid)
                    || ogeom::topo::explore(doc.model(), ps, ogeom::topo::Filter::All)
                        .map(|subs| subs.iter().any(|sub| sub.is_same(solid)))
                        .unwrap_or(false)
            })
        })
        .count();
    println!("solids not reachable from any part product: {unreachable}");

    if placed > 0 {
        println!(
            "=> {placed} of {occurrences} occurrences are drawn at their part-local\n\
                origin instead of where the assembly puts them"
        );
    }
}
