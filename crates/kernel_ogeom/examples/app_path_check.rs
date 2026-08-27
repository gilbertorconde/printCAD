//! Mesh files exactly as the app does; count vertices escaping body bounds.
use kernel_api::{Kernel, TessellationSettings};
use kernel_ogeom::OgeomKernel;

fn main() {
    for path in std::env::args().skip(1) {
        let mut kernel = OgeomKernel::new();
        kernel.initialize().expect("init");
        let imported = kernel
            .import_step(
                std::path::Path::new(&path),
                &TessellationSettings::default(),
            )
            .expect("import");
        for body in &imported.bodies {
            let Some((lo, hi)) = body.bounds_mm else {
                continue;
            };
            let slack = 1.0_f32;
            let escaped = body
                .mesh
                .positions
                .iter()
                .filter(|p| (0..3).any(|a| p[a] < lo[a] - slack || p[a] > hi[a] + slack))
                .count();
            println!(
                "{}: body `{}`: {} escaped vertices (of {})",
                std::path::Path::new(&path)
                    .file_name()
                    .unwrap()
                    .to_string_lossy(),
                body.name.as_deref().unwrap_or("?"),
                escaped,
                body.mesh.positions.len()
            );
        }
    }
}
