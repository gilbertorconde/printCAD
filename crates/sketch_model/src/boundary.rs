//! Where the sketch's model space meets the host's single precision.
//!
//! A sketch keeps its geometry, its dimensions and its plane in `f64`. The
//! host hands the bench pointer positions, rays, picks and placements in
//! `f32`, and takes screen overlays, marks and meshes back in `f32`. These
//! helpers are the only crossings.
//!
//! A value coming in is widened through its shortest decimal: an `f32`
//! stands for the number it prints as, so a click stored as `1.99997`
//! is `1.99997` in double precision too, and a recorded script that
//! writes it in those digits replays to the same bits.

/// An incoming single-precision value as the double its digits name.
pub fn widen(v: f32) -> f64 {
    if !v.is_finite() {
        return f64::from(v);
    }
    let mut buf = String::with_capacity(16);
    use std::fmt::Write;
    let _ = write!(buf, "{v}");
    buf.parse().unwrap_or(f64::from(v))
}

/// [`widen`] of each coordinate.
pub fn widen3(v: [f32; 3]) -> [f64; 3] {
    v.map(widen)
}

/// A model value going out to the screen, the GPU or the package contract.
pub fn narrow(v: f64) -> f32 {
    v as f32
}

/// [`narrow`] of each coordinate.
pub fn narrow3(v: [f64; 3]) -> [f32; 3] {
    v.map(narrow)
}

/// Where the cursor's ray meets a plane, worked out in double precision
/// from the host's ray. `None` when the ray runs along the plane or meets
/// it behind the eye.
pub fn cursor_on_plane(
    view_proj: [[f32; 4]; 4],
    viewport: (u32, u32, u32, u32),
    cursor: (f32, f32),
    origin: [f64; 3],
    normal: [f64; 3],
) -> Option<[f64; 3]> {
    let (from, along) = core_document::runtime::viewport_to_ray(view_proj, viewport, cursor)?;
    let from = glam::DVec3::from_array(widen3(from));
    let along = glam::DVec3::from_array(widen3(along));
    let normal = glam::DVec3::from_array(normal).normalize();
    let denom = along.dot(normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (glam::DVec3::from_array(origin) - from).dot(normal) / denom;
    (t >= 0.0).then(|| (from + along * t).to_array())
}

/// [`cursor_on_plane`] through the context's view.
pub fn ctx_cursor_on_plane(
    ctx: &core_document::WorkbenchRuntimeContext,
    cursor: (f32, f32),
    origin: [f64; 3],
    normal: [f64; 3],
) -> Option<[f64; 3]> {
    cursor_on_plane(ctx.view_proj?, ctx.viewport, cursor, origin, normal)
}

/// A body's placement turning and moving a point of its frame, in double
/// precision.
pub fn place_point(placement: &core_document::BodyPlacement, p: [f64; 3]) -> [f64; 3] {
    let (turn, offset) = placement_parts(placement);
    (turn * glam::DVec3::from_array(p) + offset).to_array()
}

/// A body's placement turning a direction of its frame.
pub fn place_direction(placement: &core_document::BodyPlacement, d: [f64; 3]) -> [f64; 3] {
    (placement_parts(placement).0 * glam::DVec3::from_array(d)).to_array()
}

fn placement_parts(placement: &core_document::BodyPlacement) -> (glam::DQuat, glam::DVec3) {
    let turn = glam::DQuat::from_array(placement.rotation.map(widen)).normalize();
    (turn, glam::DVec3::from_array(widen3(placement.translation)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_widened_value_is_the_double_of_its_digits() {
        assert_eq!(widen(1.99997), 1.99997_f64);
        assert_eq!(widen(0.1), 0.1_f64);
        assert_eq!(widen(-3.0), -3.0);
        assert_eq!(narrow(widen(1.99997)), 1.99997_f32);
        assert!(widen(f32::NAN).is_nan());
        assert_eq!(widen(f32::INFINITY), f64::INFINITY);
    }
}
