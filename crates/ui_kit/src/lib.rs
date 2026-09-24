//! The design system: tokens, bundled fonts, the egui theme, the widget
//! vocabulary the mockups use, and the line-icon set. Every crate that
//! draws UI — the app shell and the workbenches — builds on this one, so it
//! knows nothing about documents.

pub mod completion;
pub mod icon;
mod icon_table;
pub mod theme;
pub mod tokens;
pub mod widgets;

pub use theme::{apply_theme, mono, mono_medium, sans, sans_medium, sans_semibold};
