//! Inertia moved and added: a tensor about a body's centre of mass turned
//! with the body, taken about another point or a line by the parallel axis
//! theorem, and several bodies' added about their shared centre of mass.
//!
//! Tensors are row by row, as `kernel_api::Inertia` keeps them. A "mass"
//! is whatever the tensor was weighed by: a volume in mm³ for a tensor at
//! unit density (mm⁵), grams for one in g·mm².

use glam::{DMat3, DQuat, DVec3};
use kernel_api::Inertia;

/// A tensor kept row by row.
pub type Tensor = [[f64; 3]; 3];

fn matrix(rows: &Tensor) -> DMat3 {
    // glam's columns are the rows transposed.
    DMat3::from_cols_array_2d(rows).transpose()
}

fn rows(m: DMat3) -> Tensor {
    m.transpose().to_cols_array_2d()
}

/// An inertia measured in a body's own frame, on the world's axes once the
/// body is turned by `turn`: the tensor R I Rᵀ, the principal axes turned.
pub fn turned(inertia: &Inertia, turn: DQuat) -> Inertia {
    let r = DMat3::from_quat(turn);
    let world = r * matrix(&inertia.about_centre) * r.transpose();
    Inertia {
        about_centre: rows(world),
        principal: inertia
            .principal
            .map(|(moment, axis)| (moment, (r * DVec3::from_array(axis)).to_array())),
    }
}

/// The tensor about `point` of a body of `mass` centred at `centre` whose
/// tensor about that centre is `about_centre`: I + m (|d|² E − d dᵀ), d
/// the centre less the point.
pub fn about_point(about_centre: &Tensor, mass: f64, centre: [f64; 3], point: [f64; 3]) -> Tensor {
    let d = DVec3::from_array(centre) - DVec3::from_array(point);
    let shift = DMat3::from_diagonal(DVec3::splat(d.length_squared()))
        - DMat3::from_cols(d * d.x, d * d.y, d * d.z);
    rows(matrix(about_centre) + shift * mass)
}

/// The moment about the line through `point` along `direction`: the
/// tensor's along the line, plus the mass times the square of how far the
/// centre stands off it. `None` for a direction of no length.
pub fn about_axis(
    about_centre: &Tensor,
    mass: f64,
    centre: [f64; 3],
    point: [f64; 3],
    direction: [f64; 3],
) -> Option<f64> {
    let u = DVec3::from_array(direction).try_normalize()?;
    let off = DVec3::from_array(centre) - DVec3::from_array(point);
    let square = off - u * off.dot(u);
    Some(u.dot(matrix(about_centre) * u) + mass * square.length_squared())
}

/// The principal moments of a symmetric tensor, smallest first, each with
/// its unit axis, the third the first two crossed (right-handed).
pub fn principal(tensor: &Tensor) -> [(f64, [f64; 3]); 3] {
    // Jacobi rotations: each zeroes the largest product left, until none
    // stands above the rounding of the largest moment.
    let mut a = *tensor;
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let size = (0..3)
        .map(|i| a[i][i].abs())
        .fold(0.0, f64::max)
        .max(1e-300);
    for _ in 0..64 {
        let (p, q) = [(0, 1), (0, 2), (1, 2)]
            .into_iter()
            .max_by(|x, y| a[x.0][x.1].abs().total_cmp(&a[y.0][y.1].abs()))
            .unwrap_or((0, 1));
        if a[p][q].abs() <= size * 1e-15 {
            break;
        }
        let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
        let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
        let c = 1.0 / (t * t + 1.0).sqrt();
        let s = t * c;
        for row in &mut a {
            let (kp, kq) = (row[p], row[q]);
            row[p] = c * kp - s * kq;
            row[q] = s * kp + c * kq;
        }
        let (rp, rq) = (a[p], a[q]);
        a[p] = std::array::from_fn(|k| c * rp[k] - s * rq[k]);
        a[q] = std::array::from_fn(|k| s * rp[k] + c * rq[k]);
        for row in &mut v {
            let (vp, vq) = (row[p], row[q]);
            row[p] = c * vp - s * vq;
            row[q] = s * vp + c * vq;
        }
    }
    let mut order = [0, 1, 2];
    order.sort_by(|&i, &j| a[i][i].total_cmp(&a[j][j]));
    let axis = |i: usize| DVec3::new(v[0][i], v[1][i], v[2][i]).normalize_or_zero();
    let (first, second) = (axis(order[0]), axis(order[1]));
    [
        (a[order[0]][order[0]], first.to_array()),
        (a[order[1]][order[1]], second.to_array()),
        (a[order[2]][order[2]], first.cross(second).to_array()),
    ]
}

/// One body's share of a sum: its mass, its centre and its tensor about
/// that centre, on the axes the sum is on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Share {
    pub mass: f64,
    pub centre: [f64; 3],
    pub about_centre: Tensor,
}

/// Several bodies as one: their mass, its centre and the inertia about it
/// (each body's moved there by the parallel axis theorem). `None` when
/// they weigh nothing.
pub fn combined(shares: &[Share]) -> Option<(f64, [f64; 3], Inertia)> {
    let mass: f64 = shares.iter().map(|s| s.mass).sum();
    if mass <= 0.0 {
        return None;
    }
    let centre = shares.iter().fold(DVec3::ZERO, |sum, s| {
        sum + DVec3::from_array(s.centre) * s.mass
    }) / mass;
    let centre = centre.to_array();
    let tensor = shares.iter().fold(DMat3::ZERO, |sum, s| {
        sum + matrix(&about_point(&s.about_centre, s.mass, s.centre, centre))
    });
    let about_centre = rows(tensor);
    Some((
        mass,
        centre,
        Inertia {
            about_centre,
            principal: principal(&about_centre),
        },
    ))
}

/// Scaled by `factor`: a tensor at unit density times a density.
pub fn scaled(tensor: &Tensor, factor: f64) -> Tensor {
    tensor.map(|row| row.map(|v| v * factor))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
    }

    /// A block 20 × 10 × 4 at unit density: its tensor about its centre.
    fn block() -> (f64, Tensor) {
        let (x, y, z) = (20.0f64, 10.0f64, 4.0f64);
        let m = x * y * z;
        let t = [
            [m * (y * y + z * z) / 12.0, 0.0, 0.0],
            [0.0, m * (x * x + z * z) / 12.0, 0.0],
            [0.0, 0.0, m * (x * x + y * y) / 12.0],
        ];
        (m, t)
    }

    /// A body stood on its side shows its inertia on the world's axes: a
    /// long bar along X turned a quarter about Z lies along Y.
    #[test]
    fn a_turned_body_turns_its_inertia() {
        let bar = Inertia {
            about_centre: [[1.0, 0.0, 0.0], [0.0, 9.0, 0.5], [0.0, 0.5, 9.0]],
            principal: [
                (1.0, [1.0, 0.0, 0.0]),
                (8.5, [0.0, 0.7, -0.7]),
                (9.5, [0.0, 0.7, 0.7]),
            ],
        };
        let t = turned(&bar, DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2));
        let m = t.about_centre;
        assert!(close(m[0][0], 9.0) && close(m[1][1], 1.0), "{m:?}");
        assert!(close(m[0][2], -0.5) && close(m[2][0], -0.5), "{m:?}");
        assert!(close(t.principal[0].1[1], 1.0), "{:?}", t.principal);
        assert_eq!(t.principal[0].0, 1.0);
    }

    /// The block's corner at the origin and its centre at (10, 5, 2): about
    /// the origin each moment gains the mass times the square of the
    /// centre's distance from that axis, and the products turn negative.
    #[test]
    fn a_point_away_from_the_centre_adds_the_mass_times_the_square() {
        let (m, t) = block();
        let o = about_point(&t, m, [10.0, 5.0, 2.0], [0.0; 3]);
        assert!(close(o[0][0], t[0][0] + m * (25.0 + 4.0)), "{o:?}");
        assert!(close(o[2][2], t[2][2] + m * (100.0 + 25.0)), "{o:?}");
        assert!(close(o[0][1], -m * 50.0) && close(o[1][0], -m * 50.0));
        assert!(close(o[1][2], -m * 10.0));
        // About its own centre it is unchanged.
        assert_eq!(about_point(&t, m, [1.0, 2.0, 3.0], [1.0, 2.0, 3.0]), t);
    }

    /// About an edge of the block along Z: m (x² + y²) / 3, as a textbook
    /// has it; about a line through the centre, the tensor's moment.
    #[test]
    fn an_axis_takes_the_moment_along_it_and_the_offset_square() {
        let (m, t) = block();
        let edge = about_axis(&t, m, [10.0, 5.0, 2.0], [0.0, 0.0, 7.0], [0.0, 0.0, 3.0]);
        assert!(close(edge.unwrap(), m * (400.0 + 100.0) / 3.0), "{edge:?}");
        let through = about_axis(&t, m, [10.0, 5.0, 2.0], [10.0, 0.0, 2.0], [0.0, 1.0, 0.0]);
        assert!(close(through.unwrap(), t[1][1]));
        assert_eq!(about_axis(&t, m, [0.0; 3], [0.0; 3], [0.0; 3]), None);
    }

    /// The block turned an eighth about Z: its principal moments are the
    /// block's own, with its long side along the turned X.
    #[test]
    fn the_principal_moments_come_out_of_a_turned_tensor() {
        let (_, t) = block();
        let inertia = Inertia {
            about_centre: t,
            principal: [(0.0, [0.0; 3]); 3],
        };
        let turn = DQuat::from_rotation_z(std::f64::consts::FRAC_PI_4);
        let world = turned(&inertia, turn).about_centre;
        let found = principal(&world);
        assert!(close(found[0].0, t[0][0]), "{found:?}");
        assert!(close(found[1].0, t[1][1]), "{found:?}");
        assert!(close(found[2].0, t[2][2]), "{found:?}");
        let long = turn * DVec3::X;
        assert!(
            DVec3::from_array(found[0].1).dot(long).abs() > 1.0 - 1e-9,
            "{found:?}"
        );
        let [a, b, c] = found.map(|(_, axis)| DVec3::from_array(axis));
        assert!(a.cross(b).dot(c) > 1.0 - 1e-9, "right-handed");
    }

    /// Two halves of the block side by side add up to the whole block.
    #[test]
    fn two_halves_add_up_to_the_whole() {
        let (m, whole) = block();
        let half = |cx: f64| {
            let (x, y, z) = (10.0f64, 10.0f64, 4.0f64);
            let hm = m / 2.0;
            Share {
                mass: hm,
                centre: [cx, 5.0, 2.0],
                about_centre: [
                    [hm * (y * y + z * z) / 12.0, 0.0, 0.0],
                    [0.0, hm * (x * x + z * z) / 12.0, 0.0],
                    [0.0, 0.0, hm * (x * x + y * y) / 12.0],
                ],
            }
        };
        let (mass, centre, inertia) = combined(&[half(5.0), half(15.0)]).unwrap();
        assert!(close(mass, m));
        assert_eq!(centre, [10.0, 5.0, 2.0]);
        for (i, row) in inertia.about_centre.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                assert!(close(*v, whole[i][j]), "{:?}", inertia.about_centre);
            }
        }
        assert!(close(inertia.principal[0].0, whole[0][0]));
        assert_eq!(combined(&[]), None);
    }
}
