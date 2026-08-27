//! Find faces whose triangulation escapes their boundary: mesh an assembly's
//! parts and flag any face whose triangle cloud is far larger than the hull
//! of its own vertices. Prints entity ids and surface kinds for an upstream
//! report.
//!
//! ```text
//! cargo run --release -p kernel_ogeom --example face_escape_probe -- <file.step> [name-filter]
//! ```

use ogeom::core::Tolerances;
use ogeom::doc::ProductKind;
use ogeom::topo::{explore, Filter, ShapeType};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: face_escape_probe <file.step> [name-filter]");
    let name_filter = args.next().unwrap_or_default().to_lowercase();

    let bytes = std::fs::read(&path).expect("read STEP");
    let text = String::from_utf8_lossy(&bytes);
    let tol = Tolerances::millimetres();
    let import = ogeom::io::step::read_step(&text, tol).expect("parse STEP");
    let doc = &import.document;
    let model = doc.model();

    for (_, product) in doc.products() {
        let ProductKind::Part { shape } = &product.kind else {
            continue;
        };
        if !name_filter.is_empty() && !product.name.to_lowercase().contains(&name_filter) {
            continue;
        }
        let Ok(faces) = explore(model, shape, Filter::OfType(ShapeType::Face)) else {
            continue;
        };
        for face in &faces {
            // The face's own vertices bound its true extent (undershooting
            // curved bulges by at most the local radius).
            let Ok(vbounds) = ogeom::algo::vertex_bounds(model, face, tol) else {
                continue;
            };
            let (Some(vlo), Some(vhi)) = (vbounds.low(), vbounds.high()) else {
                continue;
            };
            let vdiag = diag([vlo.x, vlo.y, vlo.z], [vhi.x, vhi.y, vhi.z]);

            let Ok(mesh) =
                ogeom::mesh::triangulate_face(model, face, ogeom::mesh::Deflection::default(), tol)
            else {
                continue;
            };
            let mut mlo = [f64::INFINITY; 3];
            let mut mhi = [f64::NEG_INFINITY; 3];
            for p in &mesh.positions {
                for a in 0..3 {
                    let c = [p.x, p.y, p.z][a];
                    mlo[a] = mlo[a].min(c);
                    mhi[a] = mhi[a].max(c);
                }
            }
            if mesh.positions.is_empty() {
                continue;
            }
            let mdiag = diag(mlo, mhi);

            // A curved face legitimately exceeds its vertex hull a little; a
            // wrong-branch sweep exceeds it by the surface's whole extent.
            if vdiag > 1e-6 && mdiag > 2.0 * vdiag + 1.0 {
                let entity = model
                    .identity_of(face)
                    .map(|e| e.get().to_string())
                    .unwrap_or_else(|| "?".into());
                println!(
                    "part `{}`: face (surface entity #{entity}) escapes its boundary: \
                     vertex-hull diag {vdiag:.2} mm, mesh diag {mdiag:.2} mm ({:.1}x), \
                     {} triangles",
                    product.name,
                    mdiag / vdiag,
                    mesh.triangles.len(),
                );
            }
        }
    }
    println!("done");
}

fn diag(lo: [f64; 3], hi: [f64; 3]) -> f64 {
    let dx = hi[0] - lo[0];
    let dy = hi[1] - lo[1];
    let dz = hi[2] - lo[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}
