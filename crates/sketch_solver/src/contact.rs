//! Pure branch predicates shared by equation compilation and measurements.

/// Circles whose centres are `distance` apart are nested when one lies
/// inside the other; equal boundary distances select the outside branch.
pub fn circles_nest(distance: f64, radius1: f64, radius2: f64) -> bool {
    distance < (radius1 - radius2).abs()
}

/// A point `distance` from a centre is strictly inside its circle.
pub fn point_inside(distance: f64, radius: f64) -> bool {
    distance < radius
}

/// The side of a line a point stands on, from its signed distance to the
/// line: `-1` on the negative side, `1` on the positive side or on the
/// line itself.
pub fn side_of_line(signed_distance: f64) -> f64 {
    if signed_distance < 0.0 { -1.0 } else { 1.0 }
}
