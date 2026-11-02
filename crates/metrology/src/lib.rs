//! Measurement arithmetic: statistics of residuals, form and orientation
//! tolerances, colour ramps for maps, and design values. See the README.

pub mod design;
pub mod form;
pub mod orientation;
pub mod profile;
pub mod ramp;
pub mod stats;

/// A point or vector in three dimensions.
pub type Vec3 = [f64; 3];

pub(crate) fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(crate) fn norm(a: Vec3) -> f64 {
    dot(a, a).sqrt()
}

/// `a` scaled to length one, or `None` when it has no length.
pub(crate) fn unit(a: Vec3) -> Option<Vec3> {
    let n = norm(a);
    (n > 0.0 && n.is_finite()).then(|| a.map(|c| c / n))
}
