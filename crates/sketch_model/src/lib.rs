//! The sketch as data: the `f64` model a sketch is stored and solved in
//! (points, curves, constraints, texts, its plane), the feature that holds
//! it, solving through `sketch_solver`, the closed profiles and regions
//! other features build from, generated profiles, and the one boundary
//! where values cross to and from `f32`. No tools, input, panels or
//! drawing on screen: those are the sketcher bench's.

/// The feature kind sketches are stored under.
pub const KIND: &str = "wb.sketch";

pub mod boundary;
pub mod change;
pub mod conic;
pub mod curves;
pub mod feature;
pub mod generator;
pub mod geom2d;
pub mod measure;
pub mod operations;
pub mod params;
pub mod profile;
pub mod regions;
pub mod render;
pub mod repair;
pub mod shape;
pub mod sketch;
pub mod solver;
pub mod spline;
pub mod style;
pub mod text;

pub use feature::{AttachedSupport, DatumSupport, FaceSupport, LentFace, SketchFeature};
