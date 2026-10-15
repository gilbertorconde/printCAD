//! Validate imported numbers at the application's storage and frame boundaries.

use glam::DVec3;
use project_archive::archive::FcstdError;
use project_archive::document_xml::Placement;
use wb_sketch::sketch::SketchPlane;

fn bad(object: &str, field: &str, reason: impl std::fmt::Display) -> FcstdError {
    FcstdError::Format {
        object: object.into(),
        reason: format!("{field}: {reason}"),
    }
}

pub(crate) fn scalar(object: &str, field: &str, value: f64) -> Result<f64, FcstdError> {
    if !value.is_finite() {
        return Err(bad(object, field, format!("{value} is not finite")));
    }
    Ok(value)
}

/// Design feature lengths still cross a single-precision storage boundary.
pub(crate) fn design_length(object: &str, field: &str, value: f64) -> Result<f32, FcstdError> {
    let converted = scalar(object, field, value)? as f32;
    if !converted.is_finite() {
        return Err(bad(object, field, format!("{value} is outside f32 range")));
    }
    Ok(converted)
}

/// Unit lengths, perpendicular axes and X × Y = normal are required; no axis
/// is repaired or reconstructed here. Source arithmetic admits 1e-9 error.
pub(crate) fn validate_frame(
    object: &str,
    axes: [[f64; 3]; 3],
    tolerance: f64,
) -> Result<(), FcstdError> {
    for (field, axis) in ["Placement.x_axis", "Placement.y_axis", "Placement.normal"]
        .into_iter()
        .zip(axes)
    {
        let axis = DVec3::from_array(axis);
        if !axis.is_finite() {
            return Err(bad(object, field, "axis is not finite"));
        }
        let length = axis.length();
        if !length.is_finite() || (length - 1.0).abs() > tolerance {
            return Err(bad(
                object,
                field,
                format!("axis length {length} is not unit"),
            ));
        }
    }
    let [x, y, normal] = axes.map(DVec3::from_array);
    for (field, dot) in [
        ("Placement.x_axis/y_axis", x.dot(y)),
        ("Placement.x_axis/normal", x.dot(normal)),
        ("Placement.y_axis/normal", y.dot(normal)),
    ] {
        if dot.abs() > tolerance {
            return Err(bad(object, field, "axes are not perpendicular"));
        }
    }
    if x.cross(y).distance(normal) > tolerance {
        return Err(bad(object, "Placement.normal", "frame is not right-handed"));
    }
    Ok(())
}

pub(crate) fn frame(object: &str, pose: Placement) -> Result<[[f64; 3]; 3], FcstdError> {
    for (axis, value) in ["x", "y", "z"].into_iter().zip(pose.position.to_array()) {
        if !value.is_finite() {
            return Err(bad(
                object,
                &format!("Placement.origin.{axis}"),
                "is not finite",
            ));
        }
    }
    let axes =
        [DVec3::X, DVec3::Y, DVec3::Z].map(|axis| pose.rotation.transform_vector(axis.to_array()));
    validate_frame(object, axes, 1e-9)?;
    Ok(axes)
}

pub(crate) fn sketch_plane(object: &str, pose: Placement) -> Result<SketchPlane, FcstdError> {
    let axes = frame(object, pose)?;
    Ok(SketchPlane {
        origin: pose.position.to_array(),
        x_axis: axes[0],
        y_axis: axes[1],
        normal: axes[2],
    })
}

#[cfg(test)]
mod tests {
    use super::validate_frame;
    use project_archive::archive::FcstdError;

    #[test]
    fn design_range_is_checked_at_the_actual_f32_rounding_boundary() {
        let boundary = f64::from(f32::MAX) + 2.0_f64.powi(103);
        for sign in [-1.0, 1.0] {
            for value in [boundary, boundary.next_up(), f64::INFINITY, f64::NAN] {
                assert!(super::design_length("Pad", "Length", sign * value).is_err());
            }
            assert_eq!(
                super::design_length("Pad", "Length", sign * boundary.next_down()).unwrap(),
                sign as f32 * f32::MAX
            );
        }
    }

    #[test]
    fn malformed_axes_are_named_errors_instead_of_repaired_frames() {
        let basis = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        for (index, axis, field) in [
            (0, [0.0; 3], "x_axis"),
            (1, [f64::NAN, 0.0, 0.0], "y_axis"),
            (2, [0.0, 0.0, f64::INFINITY], "normal"),
            (0, [2.0, 0.0, 0.0], "x_axis"),
            (1, basis[0], "x_axis/y_axis"),
            (2, [0.0, 0.0, -1.0], "normal"),
        ] {
            let mut axes = basis;
            axes[index] = axis;
            let error = validate_frame("Profile", axes, 1e-9).unwrap_err();
            let FcstdError::Format { object, reason } = error else {
                panic!("expected a format error");
            };
            assert_eq!(object, "Profile");
            assert!(reason.contains(field), "{reason}");
        }
        validate_frame("Profile", basis, 1e-9).unwrap();
    }
}
