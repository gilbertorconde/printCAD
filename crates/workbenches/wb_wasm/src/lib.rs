//! Workbenches from packages. A package is a WebAssembly component
//! speaking `printcad:workbench` (`bench_api`), a manifest and its icons;
//! [`load`] turns one into a [`WasmWorkbench`], which the registry takes
//! like any built-in bench. See `docs/PLUGINS.md`.

mod bench;
mod convert;
mod engine;
mod guest;
mod host;
mod jobs;
pub mod package;

pub use bench::{WasmWorkbench, load};
pub use bench_api::{Capabilities, Manifest};
pub use package::{Package, discover, install, pack, uninstall};
