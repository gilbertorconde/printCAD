//! Workbenches from packages. A package is a WebAssembly component
//! speaking `printcad:workbench` (`bench_api`), a manifest and its icons;
//! [`load`] turns one into a [`WasmWorkbench`], which the registry takes
//! like any built-in bench. See `docs/PLUGINS.md`.

mod bench;
mod convert;
mod engine;
mod exports;
mod geometry;
mod guest;
mod host;
mod jobs;
mod kept;
pub mod package;
mod region;
pub mod remote;
mod sessions;
pub mod store;

pub use bench::WasmWorkbench;
pub use bench::load;
pub use bench_api::{Capabilities, Manifest};
pub use host::{ProfileSource, set_profile_source};
pub use package::{Package, discover, install, pack, uninstall};

/// Moves the clock that stops package calls `ticks` at once, as it moves
/// when it catches up after a sleep the system stretched; for tests.
#[doc(hidden)]
pub fn advance_clock(ticks: u64) {
    for _ in 0..ticks {
        engine::ENGINE.increment_epoch();
    }
}
