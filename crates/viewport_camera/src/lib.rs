//! Camera pose, projection, press-relative navigation and timed transitions.
//! Coordinates are local model units and logical pixels. The host supplies
//! anchors, input ownership, its palette, clip planes and a floating origin.

pub mod camera;
pub mod controller;
pub mod fit;
pub mod focal_view;
pub mod length;
pub mod momentum;
pub mod navigation;
pub mod raycast;
pub mod scale;
pub mod smoothing;
pub mod transition;
pub mod view;

#[cfg(test)]
mod gesture_options_tests;
#[cfg(test)]
mod navigation_tests;
