//! Application-shell internals split out of `main.rs`.

pub(crate) mod commands;
pub(crate) mod doc_io;
pub(crate) mod frame;
pub(crate) mod gfx;
pub(crate) mod input;
pub(crate) mod step_import;
pub(crate) mod undo_host;
pub(crate) mod workbench_host;

pub(crate) use gfx::Gfx;
