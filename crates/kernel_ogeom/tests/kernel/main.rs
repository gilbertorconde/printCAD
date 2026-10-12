//! The kernel adapter's integration tests, one program: each module a part of what the adapter does, run against the real kernel.
//!
//! One program rather than one per file: each would link the whole crate
//! and its dependencies again, which costs far more than running the tests.

mod annotations;
mod boolean_tolerance;
mod borrowed_geometry;
mod chain_cache;
mod datum_attachments;
mod design_stack;
mod dxf_import;
mod export;
mod extrude;
mod generated_profiles;
mod imported_base;
mod kernel_fillet_closed_edges;
mod kernel_orientation;
mod kernel_revolution_booleans;
mod kernel_scaled_revolution;
mod mesh_import;
mod naming;
mod print_checks;
mod probes;
mod progress;
mod queries;
mod scripted_design;
mod scripted_part;
mod shape_health;
mod solid_ops;
mod step_import;
mod surface_ops;
mod surface_stack;

/// The bundled real part (`tests/data/drive_frame_upper.step`) at the
/// default detail, imported once for every test that reads it.
pub fn real_part() -> &'static kernel_api::ImportedModel {
    use kernel_api::Kernel;
    static PART: std::sync::OnceLock<kernel_api::ImportedModel> = std::sync::OnceLock::new();
    PART.get_or_init(|| {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/drive_frame_upper.step");
        kernel_ogeom::OgeomKernel::new()
            .import_step(&path, &kernel_api::TessellationSettings::default())
            .expect("the real part imports")
    })
}
