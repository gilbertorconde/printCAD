//! Read archived projects, convert their recipes, and check the resulting geometry.

pub mod bench;
pub mod document_import;
pub mod error;
mod numeric;
pub mod parity;
pub mod report;
pub mod sketch_import;

pub use bench::ProjectImportBench;
