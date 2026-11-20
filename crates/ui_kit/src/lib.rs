//! The design system: tokens, bundled fonts, the egui theme, the widget
//! vocabulary the mockups use, and the line-icon set. Every crate that
//! draws UI (the app shell and the workbenches) builds on this one, so it
//! knows nothing about documents.

pub mod completion;
pub mod icon;
mod icon_table;
pub mod lazy;
pub mod markdown;
pub mod search;
pub mod theme;
pub mod tokens;
pub mod widgets;

pub use theme::{apply_theme, mono, mono_medium, sans, sans_medium, sans_semibold};

/// The layer of what floats on the view (its toolbar, cards and cube):
/// above the panels, below dialogs, menus and their pop-ups, which share
/// egui's foreground layer, so a dialog is never drawn under the view's
/// furniture whichever of them was last brought forward.
pub const VIEW_LAYER: egui::Order = egui::Order::Middle;
