//! The sketcher's integration tests, one program.
//!
//! One program rather than one per file: each would link the whole crate
//! and its dependencies again, which costs far more than running the tests.

mod block_kernel;
mod interaction;
mod precision;
mod recording;
