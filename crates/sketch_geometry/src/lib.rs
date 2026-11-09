pub mod analytic_spline;
pub mod arc;
pub mod bbox;
pub mod bspline;
pub mod bspline_window;
pub mod conic;
pub mod curves;
pub mod intersections;
pub mod primitives;
pub mod profile_curve;
pub mod profile_geometry;
pub mod profile_spline_contacts;
pub mod roundoff;
pub mod sketch_plane;
pub mod trim_curve;
pub mod types;
pub mod world;

#[cfg(test)]
mod bbox_tests;
#[cfg(test)]
mod bspline_tests;
#[cfg(test)]
mod curve_projection_tests;
#[cfg(test)]
mod primitives_tests;
#[cfg(test)]
mod roundoff_tests;
