//! Alignment: moving a scan (its body placement, never the reference) so it
//! sits where it should be measured. Each alignment is kept as a feature
//! saying how it was made and the placement it set; everything measured on
//! the scan is in the scan's own frame, so it comes along.

use core_document::{
    BodyId, BodyPlacement, DocumentError, DocumentResult, FeatureId, WorkbenchFeature, WorkbenchId,
};
use glam::{DMat3, DQuat, DVec3};
use kernel_api::{FittedShape, KernelQueries, RegisterRequest, Registration, TriMesh};
use serde::{Deserialize, Serialize};

use crate::compare::Inputs;

/// The feature kind of every alignment.
pub const KIND: &str = "wb.inspect.alignment";

/// One alignment of a scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alignment {
    pub name: String,
    pub scan: BodyId,
    pub how: How,
    /// Where the scan sat before, and where the alignment put it.
    pub before: BodyPlacement,
    pub after: BodyPlacement,
}

/// How an alignment was made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum How {
    Datums {
        primary: FeatureId,
        secondary: FeatureId,
        zero: FeatureId,
        primary_side: String,
        secondary_along: String,
    },
    Auto {
        said: String,
    },
    Symmetry {
        rms: f64,
    },
    /// Fitted onto a reference body's surface.
    BestFit {
        reference: BodyId,
        /// Whether every principal-axis matching was tried as well.
        search: bool,
        rms: f64,
        inliers: usize,
        sampled: usize,
        /// How many motions the fit could not pin down.
        free: usize,
        /// Whether its rounds settled before their limit.
        #[serde(default = "settled")]
        converged: bool,
    },
    /// Moved and turned by numbers: X, then Y, then Z about the origin,
    /// then the move.
    Numbers {
        translate: [f64; 3],
        rotate_deg: [f64; 3],
    },
    /// Back where the scanner put it.
    Reset,
    /// Points picked on the scan brought onto points picked on a reference.
    Points {
        pairs: usize,
        rms: f64,
    },
}

impl WorkbenchFeature for Alignment {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(KIND)
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    fn from_json(value: &serde_json::Value) -> DocumentResult<Self> {
        serde_json::from_value(value.clone()).map_err(DocumentError::Serialization)
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        Vec::new()
    }

    fn name(&self) -> &str {
        &self.name
    }
}

fn settled() -> bool {
    true
}

impl How {
    pub fn word(&self) -> &'static str {
        match self {
            How::Datums { .. } => "3-2-1 datums",
            How::Auto { .. } => "Auto-align",
            How::Symmetry { .. } => "Use symmetry",
            How::BestFit { .. } => "Best fit",
            How::Numbers { .. } => "Moved by numbers",
            How::Reset => "Reset alignment",
            How::Points { .. } => "Aligned by picked points",
        }
    }

    /// One line saying what it came to.
    pub fn summary(&self) -> String {
        match self {
            How::Datums {
                primary_side,
                secondary_along,
                ..
            } => {
                format!("primary {primary_side}, secondary {secondary_along}")
            }
            How::Auto { said } => said.clone(),
            How::Points { pairs, rms } => format!("{pairs} pairs, RMS {rms:.4} mm"),
            How::Symmetry { rms } => format!("mirror RMS {rms:.4} mm"),
            How::BestFit {
                rms,
                inliers,
                free,
                converged,
                ..
            } => {
                let mut s = format!("RMS {rms:.4} mm over {inliers} points");
                if !converged {
                    s.push_str(" · still moving when its rounds ran out");
                }
                if *free > 0 {
                    s.push_str(&format!(" · {free} motion(s) left free: it may slide"));
                }
                s
            }
            How::Numbers {
                translate,
                rotate_deg,
            } => format!(
                "turned {:.2}°, {:.2}°, {:.2}°, moved {:.3}, {:.3}, {:.3} mm",
                rotate_deg[0],
                rotate_deg[1],
                rotate_deg[2],
                translate[0],
                translate[1],
                translate[2]
            ),
            How::Reset => "the scanner's frame".to_string(),
        }
    }
}

/// The scan fitted onto its reference: where the scan goes, and the fit.
pub fn best_fit(
    kernel: &dyn KernelQueries,
    inputs: &Inputs,
    reference: BodyPlacement,
    search: bool,
) -> Result<(BodyPlacement, Registration), String> {
    let (points, normals) = inputs.points();
    let request = RegisterRequest {
        points,
        normals,
        search,
        max_distance: None,
        facing_limit_deg: None,
    };
    let fit = kernel
        .register(inputs.target(), &request)
        .map_err(|e| e.to_string())?;
    let motion = placement_of(&fit.transform);
    // The scan's frame goes to the reference's, through the fit, and back
    // out where the reference sits.
    let after = reference.after(&motion.after(&inputs.scan_to_reference()));
    Ok((after, fit))
}

/// The placement turned X, then Y, then Z degrees about the origin, then
/// moved.
pub fn by_numbers(translate: [f64; 3], rotate_deg: [f64; 3]) -> BodyPlacement {
    let [x, y, z] = rotate_deg.map(f64::to_radians);
    let turn = DQuat::from_rotation_z(z) * DQuat::from_rotation_y(y) * DQuat::from_rotation_x(x);
    BodyPlacement::new(turn.as_quat(), DVec3::from_array(translate).as_vec3())
}

/// A rigid motion given as three row-major rows.
fn placement_of(rows: &[[f64; 4]; 3]) -> BodyPlacement {
    let m = DMat3::from_cols_array_2d(&[
        [rows[0][0], rows[1][0], rows[2][0]],
        [rows[0][1], rows[1][1], rows[2][1]],
        [rows[0][2], rows[1][2], rows[2][2]],
    ]);
    BodyPlacement::new(
        DQuat::from_mat3(&m).as_quat(),
        DVec3::new(rows[0][3], rows[1][3], rows[2][3]).as_vec3(),
    )
}

/// A signed document axis.
pub fn axis(named: &str) -> Result<DVec3, String> {
    match named {
        "+x" => Ok(DVec3::X),
        "-x" => Ok(-DVec3::X),
        "+y" => Ok(DVec3::Y),
        "-y" => Ok(-DVec3::Y),
        "+z" => Ok(DVec3::Z),
        "-z" => Ok(-DVec3::Z),
        _ => Err("an axis must be +x, -x, +y, -y, +z or -z".into()),
    }
}

fn unit(v: [f64; 3]) -> Result<DVec3, String> {
    DVec3::from_array(v)
        .try_normalize()
        .filter(|v| v.is_finite())
        .ok_or_else(|| "a datum direction is degenerate".into())
}

fn direction(shape: &FittedShape) -> Result<([f64; 3], DVec3), String> {
    let (p, n) = match *shape {
        FittedShape::Plane { origin, normal } => (origin, normal),
        FittedShape::Cylinder { point, axis, .. } => (point, axis),
        FittedShape::Cone { apex, axis, .. } => (apex, axis),
        FittedShape::Line { point, direction } => (point, direction),
        _ => return Err("a direction datum must be a plane, cylinder, cone or line".into()),
    };
    Ok((p, unit(n)?))
}

pub struct Datums<'a> {
    pub shapes: [&'a FittedShape; 3],
    pub primary_side: &'a str,
    pub secondary_along: &'a str,
    pub zero_at: [f64; 3],
    pub centre: bool,
}

/// Fix rotation in primary/secondary order, and translation without moving
/// any coordinate a preceding datum fixed.
pub fn datums(
    mesh: &TriMesh,
    before: BodyPlacement,
    request: &Datums<'_>,
) -> Result<BodyPlacement, String> {
    let [primary, secondary, zero] = request.shapes;
    let (p, n) = direction(primary)?;
    let (s, d) = direction(secondary)?;
    let target_n = axis(request.primary_side)?;
    let target_d = axis(request.secondary_along)?;
    if target_n.dot(target_d).abs() > 1e-8 {
        return Err("primary and secondary world axes must be perpendicular".into());
    }
    let d = (d - n * d.dot(n))
        .try_normalize()
        .filter(|_| n.cross(d).length() > 1e-8)
        .ok_or("the datum directions are parallel")?;
    let source = DMat3::from_cols(d, n.cross(d), n);
    let target = DMat3::from_cols(target_d, target_n.cross(target_d), target_n);
    let turn = DQuat::from_mat3(&(target * source.transpose())).normalize();
    let at = DVec3::from_array(request.zero_at);
    if !at.is_finite() {
        return Err("zero_at must be finite".into());
    }
    let mut translation = before.offset().as_dvec3();
    let mut fixed: Vec<DVec3> = Vec::new();
    let mut constrain = |normal: DVec3, point: DVec3| {
        let free = fixed.iter().fold(normal, |v, n| v - *n * v.dot(*n));
        if free.length_squared() > 1e-16 {
            translation += free * (normal.dot(at - point - translation) / free.length_squared());
            fixed.push(free.normalize());
        }
    };
    let p = turn * DVec3::from_array(p);
    if matches!(primary, FittedShape::Plane { .. }) {
        constrain(target_n, p);
    } else {
        constrain(target_d, p);
        constrain(target_n.cross(target_d), p);
    }
    if matches!(secondary, FittedShape::Plane { .. }) {
        constrain(turn * direction(secondary)?.1, turn * DVec3::from_array(s));
    }
    match *zero {
        FittedShape::Plane { origin, normal } => {
            let normal = unit(normal)?;
            let nearest = normal * normal.dot(DVec3::from_array(origin));
            constrain(turn * normal, turn * nearest);
        }
        FittedShape::Sphere { centre, .. } | FittedShape::Circle { centre, .. } => {
            for normal in [DVec3::X, DVec3::Y, DVec3::Z] {
                constrain(normal, turn * DVec3::from_array(centre));
            }
        }
        _ => return Err("zero must be a point, sphere, circle or plane".into()),
    }
    if request.centre && fixed.len() < 3 {
        let (lo, hi) = bounds(mesh, turn)?;
        let remaining = at - (lo + hi) * 0.5 - translation;
        translation += fixed.iter().fold(remaining, |v, n| v - *n * v.dot(*n));
    }
    if !translation.is_finite() {
        return Err("datum points must be finite".into());
    }
    Ok(BodyPlacement::new(turn.as_quat(), translation.as_vec3()))
}

fn bounds(mesh: &TriMesh, turn: DQuat) -> Result<(DVec3, DVec3), String> {
    if mesh.positions.is_empty() {
        return Err("the scan has no points".into());
    }
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = -lo;
    for p in &mesh.positions {
        let p = turn * DVec3::from_array(p.map(f64::from));
        if !p.is_finite() {
            return Err("scan points must be finite".into());
        }
        lo = lo.min(p);
        hi = hi.max(p);
    }
    Ok((lo, hi))
}

pub fn automatic(proposal: &kernel_api::Orientation) -> BodyPlacement {
    let x = DVec3::from_array(proposal.x_axis);
    let z = DVec3::from_array(proposal.z_axis);
    let turn = DQuat::from_mat3(&DMat3::from_cols(x, z.cross(x), z).transpose()).normalize();
    BodyPlacement::new(
        turn.as_quat(),
        (-(turn * DVec3::from_array(proposal.origin))).as_vec3(),
    )
}

/// The shortest turn to a signed world axis, followed by a shift normal
/// to the plane; tangential placement is preserved.
pub fn symmetry(
    before: BodyPlacement,
    plane: &kernel_api::SymmetryPlane,
) -> Result<BodyPlacement, String> {
    if !plane.rms.is_finite() || plane.rms > 0.5 {
        return Err(format!("mirror RMS {:.4} mm exceeds 0.5 mm", plane.rms));
    }
    let n = before.quat().as_dquat() * unit(plane.normal)?;
    let target = [DVec3::X, DVec3::Y, DVec3::Z]
        .into_iter()
        .max_by(|a, b| a.dot(n).abs().total_cmp(&b.dot(n).abs()))
        .unwrap();
    let target = target * target.dot(n).signum();
    let turn = DQuat::from_rotation_arc(n, target);
    let point =
        before.quat().as_dquat() * DVec3::from_array(plane.point) + before.offset().as_dvec3();
    if !point.is_finite() {
        return Err("symmetry plane must be finite".into());
    }
    let motion = BodyPlacement::new(
        turn.as_quat(),
        (-target * target.dot(turn * point)).as_vec3(),
    );
    Ok(motion.after(&before))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datum_planes_put_a_box_corner_at_zero_and_preserve_primary() {
        let turn = DQuat::from_euler(glam::EulerRot::XYZ, 0.3, -0.6, 0.8);
        let corner = DVec3::new(12.0, 7.0, -4.0);
        let plane = |n: DVec3| FittedShape::Plane {
            origin: corner.to_array(),
            normal: (turn * n).to_array(),
        };
        let primary = plane(-DVec3::Z);
        let secondary = plane(DVec3::X);
        let zero = plane(DVec3::Y);
        let mesh = TriMesh::default();
        let mut request = Datums {
            shapes: [&primary, &secondary, &zero],
            primary_side: "-z",
            secondary_along: "+x",
            zero_at: [0.0; 3],
            centre: true,
        };
        let placed = datums(&mesh, BodyPlacement::IDENTITY, &request).unwrap();
        assert!(
            DVec3::from_array(placed.point(corner.as_vec3().to_array()).map(f64::from)).length()
                < 1e-5
        );
        for n in [DVec3::X, DVec3::Y, DVec3::Z] {
            assert!((placed.quat().as_dquat() * turn * n - n).length() < 1e-6);
        }
        request.secondary_along = "+z";
        assert!(datums(&mesh, placed, &request).is_err());
        request.secondary_along = "+x";
        request.shapes[1] = &primary;
        assert!(datums(&mesh, placed, &request).is_err());
        request.shapes = [&primary, &secondary, &primary];
        let mesh = TriMesh {
            positions: [DVec3::ZERO, DVec3::new(4.0, 6.0, 2.0)]
                .map(|p| (corner + turn * p).as_vec3().to_array())
                .to_vec(),
            ..Default::default()
        };
        let centred = datums(&mesh, BodyPlacement::IDENTITY, &request).unwrap();
        let middle = corner + turn * DVec3::new(2.0, 3.0, 1.0);
        assert!(centred.point(middle.as_vec3().to_array())[1].abs() < 1e-5);
        request.centre = false;
        let kept = datums(&mesh, by_numbers([0.0, 8.0, 0.0], [0.0; 3]), &request).unwrap();
        assert!((kept.translation[1] - 8.0).abs() < 1e-5);
    }

    #[test]
    fn symmetry_uses_the_shortest_turn_and_rejects_large_residual() {
        let before = by_numbers([7.0, 3.0, 2.0], [0.0, 0.0, 20.0]);
        let mut plane = kernel_api::SymmetryPlane {
            point: [1.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            rms: 0.1,
        };
        let after = symmetry(before, &plane).unwrap();
        assert!(after.point([1.0, 0.0, 0.0])[0].abs() < 1e-5);
        assert!((after.direction([1.0, 0.0, 0.0])[0] - 1.0).abs() < 1e-6);
        assert!((after.point([0.0; 3])[2] - 2.0).abs() < 1e-6);
        plane.rms = 0.501;
        assert!(symmetry(before, &plane).is_err());
    }

    /// A motion read from rows is the one written there, and turning by
    /// numbers goes X, then Y, then Z.
    #[test]
    fn placements_read_and_turn_as_written() {
        let p = by_numbers([1.0, 2.0, 3.0], [0.0, 0.0, 90.0]);
        let moved = p.point([1.0, 0.0, 0.0]);
        assert!((moved[0] - 1.0).abs() < 1e-6 && (moved[1] - 3.0).abs() < 1e-6);
        let r = p.rows();
        let back = placement_of(&[r[0], r[1], r[2]]);
        let q = back.point([0.3, -0.7, 2.0]);
        let w = p.point([0.3, -0.7, 2.0]);
        assert!((0..3).all(|i| (q[i] - w[i]).abs() < 1e-5));
        let xy = by_numbers([0.0; 3], [90.0, 0.0, 90.0]).point([0.0, 1.0, 0.0]);
        assert!(
            (xy[0] + 0.0).abs() < 1e-6 && (xy[1] - 0.0).abs() < 1e-6 && (xy[2] - 1.0).abs() < 1e-6,
            "{xy:?}"
        );
    }
}
