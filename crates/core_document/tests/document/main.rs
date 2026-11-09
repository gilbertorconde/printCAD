//! The document's integration tests, one program: each module a part of the document model.
//!
//! One program rather than one per file: each would link the whole crate
//! and its dependencies again, which costs far more than running the tests.

mod arrays;
mod base_solid;
mod body_properties;
mod file_stamp;
mod history_position;
mod notes;
mod op_replay;
mod placement;
mod seam;
mod step_persistence;
mod support;
mod textures;
