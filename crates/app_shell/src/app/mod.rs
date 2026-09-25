//! Application-shell internals split out of `main.rs`.

pub(crate) mod animation;
pub(crate) mod annotations;
pub(crate) mod chat_store;
pub(crate) mod chats;
pub(crate) mod commands;
pub(crate) mod doc_io;
pub(crate) mod edges;
pub(crate) mod export;
pub(crate) mod frame;
pub(crate) mod gfx;
pub(crate) mod import_report;
pub(crate) mod input;
pub(crate) mod mcp;
pub(crate) mod packages;
pub(crate) mod recompute;
pub(crate) mod scripts;
#[cfg(test)]
mod seam_lint;
pub(crate) mod session;
pub(crate) mod sixdof;
pub(crate) mod step_import;
pub(crate) mod tabs;
pub(crate) mod undo_host;
pub(crate) mod workbench_host;

pub(crate) use gfx::Gfx;
