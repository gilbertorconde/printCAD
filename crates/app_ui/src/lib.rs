//! The window's UI and what it exchanges with the application. The UI
//! (`ui`, one module per region, drawn with egui) is private: the
//! application drives it through [`UiLayer`] alone, handing it each
//! frame's [`contract::UiFrameInputs`] and applying the
//! [`contract::UiCommand`]s it answers with. Beside it, what both sides
//! use: the keymap, the script console's lines, the scripts folder, and
//! the scan and array path helpers.

pub mod array_paths;
pub mod build;
pub mod console;
pub mod contract;
pub mod keymap;
pub mod scans;
pub mod script_library;
#[cfg(test)]
mod test_support;
mod ui;

pub use ui::UiLayer;
