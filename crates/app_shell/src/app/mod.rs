//! Application-shell internals: the app's per-concern modules beside
//! `main.rs`.

pub(crate) mod agent_context;
pub(crate) mod animation;
pub(crate) mod annotations;
pub(crate) mod array_handle;
pub(crate) mod array_paths;
pub(crate) mod box_select;
pub(crate) mod chat_store;
pub(crate) mod chats;
pub(crate) mod commands;
pub(crate) mod discovery;
pub(crate) mod doc_io;
pub(crate) mod edges;
pub(crate) mod export;
pub(crate) mod file_safety;
pub(crate) mod frame;
pub(crate) mod gfx;
pub(crate) mod import_report;
pub(crate) mod input;
pub(crate) mod links;
pub(crate) mod mcp;
pub(crate) mod measure;
pub(crate) mod packages;
#[cfg(target_arch = "wasm32")]
pub(crate) mod packages_web;
pub(crate) mod print_layout;
pub(crate) mod recompute;
pub(crate) mod recovery;
pub(crate) mod scene_guides;
pub(crate) mod scripts;
#[cfg(test)]
mod seam_lint;
pub(crate) mod server;
pub(crate) mod session;
pub(crate) mod session_store;
pub(crate) mod sixdof;
pub(crate) mod step_import;
pub(crate) mod tabs;
pub(crate) mod textures;
pub(crate) mod undo_host;
pub(crate) mod unsaved;
pub(crate) mod updates;
pub(crate) mod workbench_host;

pub(crate) use gfx::Gfx;
