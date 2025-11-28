use core_document::{DocumentResult, DocumentService, Workbench};
use wb_part::PartDesignWorkbench;
use wb_sketch::SketchWorkbench;

// Use the core_document macro to define a helper that registers all built-in
// workbenches and records their descriptors for the UI.
core_document::define_workbenches!(SketchWorkbench, PartDesignWorkbench);

pub use core_document::registration::REGISTERED_WORKBENCHES;


