//! Workbenches from packages. A package is a WebAssembly component
//! speaking `printcad:workbench` (`bench_api`), a manifest and its icons;
//! [`load`] turns one into a [`WasmWorkbench`], which the registry takes
//! like any built-in bench. See `docs/PLUGINS.md`.

// Packages run under wasmtime on a desktop (`runtime`) and on the page's
// own engine in a browser (`web`); both share the host side and the bench.
#[cfg(any(feature = "runtime", target_arch = "wasm32"))]
mod bench;
#[cfg(any(feature = "runtime", target_arch = "wasm32"))]
mod convert;
#[cfg(feature = "runtime")]
mod engine;
#[cfg(any(feature = "runtime", target_arch = "wasm32"))]
mod exports;
#[cfg(any(feature = "runtime", target_arch = "wasm32"))]
mod geometry;
#[cfg(feature = "runtime")]
mod guest;
#[cfg(any(feature = "runtime", target_arch = "wasm32"))]
mod host;
#[cfg(feature = "runtime")]
mod jobs;
#[cfg(all(target_arch = "wasm32", not(feature = "runtime")))]
mod web;
#[cfg(all(target_arch = "wasm32", not(feature = "runtime")))]
use web::{self as guest, jobs};
pub mod package;
pub mod remote;
pub mod store;

#[cfg(any(feature = "runtime", target_arch = "wasm32"))]
pub use bench::WasmWorkbench;
#[cfg(feature = "runtime")]
pub use bench::load;
pub use bench_api::{Capabilities, Manifest};
#[cfg(any(feature = "runtime", target_arch = "wasm32"))]
pub use host::{ProfileSource, set_profile_source};
pub use package::{Package, discover, install, pack, uninstall};
#[cfg(all(target_arch = "wasm32", not(feature = "runtime")))]
pub use web::load;
