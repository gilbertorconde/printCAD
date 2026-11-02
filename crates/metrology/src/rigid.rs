//! The rigid motion that best brings one set of points onto another, pair
//! by pair (Horn's closed form: the rotation is the quaternion of the
//! largest eigenvalue of a 4 × 4 matrix built from the pairs'
//! cross-covariance).

use crate::Vec3;

/// A rotation (row-major) and a translation: `p ↦ rotation · p +
/// translation`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rigid {
    pub rotation: [[f64; 3]; 3],
    pub translation: Vec3,
    /// The root mean square distance between the moved points and their
    /// partners.
    pub rms: f64,
}

impl Rigid {
    pub fn apply(&self, p: Vec3) -> Vec3 {
        let r = &self.rotation;
        std::array::from_fn(|i| {
            r[i][0] * p[0] + r[i][1] * p[1] + r[i][2] * p[2] + self.translation[i]
        })
    }
}

/// The rigid motion taking `from` best onto `to`, pair by pair in the
/// least-squares sense; `None` with fewer than three pairs, the lists
/// unequal, or the points all on one line.
pub fn rigid(from: &[Vec3], to: &[Vec3]) -> Option<Rigid> {
    let n = from.len();
    if n < 3 || to.len() != n {
        return None;
    }
    let mean = |pts: &[Vec3]| -> Vec3 {
        let mut m = [0.0; 3];
        for p in pts {
            for k in 0..3 {
                m[k] += p[k] / n as f64;
            }
        }
        m
    };
    let (ca, cb) = (mean(from), mean(to));
    // Cross-covariance of the centred pairs.
    let mut s = [[0.0; 3]; 3];
    for (a, b) in from.iter().zip(to) {
        let (a, b) = (crate::sub(*a, ca), crate::sub(*b, cb));
        for i in 0..3 {
            for j in 0..3 {
                s[i][j] += a[i] * b[j];
            }
        }
    }
    // Points on one line leave a turn about it free.
    let spread = |pts: &[Vec3], c: Vec3| {
        let mut m = [[0.0; 3]; 3];
        for p in pts {
            let d = crate::sub(*p, c);
            for i in 0..3 {
                for j in 0..3 {
                    m[i][j] += d[i] * d[j];
                }
            }
        }
        let (_, values) = jacobi3(m);
        let mut v = values;
        v.sort_by(f64::total_cmp);
        v[1] > 1e-12 * v[2].max(1e-300)
    };
    if !spread(from, ca) {
        return None;
    }
    let [[sxx, sxy, sxz], [syx, syy, syz], [szx, szy, szz]] = s;
    let k = [
        [sxx + syy + szz, syz - szy, szx - sxz, sxy - syx],
        [syz - szy, sxx - syy - szz, sxy + syx, szx + sxz],
        [szx - sxz, sxy + syx, -sxx + syy - szz, syz + szy],
        [sxy - syx, szx + sxz, syz + szy, -sxx - syy + szz],
    ];
    let (vectors, values) = jacobi4(k);
    let best = (0..4).max_by(|a, b| values[*a].total_cmp(&values[*b]))?;
    let q: [f64; 4] = std::array::from_fn(|i| vectors[i][best]);
    let [w, x, y, z] = q;
    let rotation = [
        [
            w * w + x * x - y * y - z * z,
            2.0 * (x * y - w * z),
            2.0 * (x * z + w * y),
        ],
        [
            2.0 * (x * y + w * z),
            w * w - x * x + y * y - z * z,
            2.0 * (y * z - w * x),
        ],
        [
            2.0 * (x * z - w * y),
            2.0 * (y * z + w * x),
            w * w - x * x - y * y + z * z,
        ],
    ];
    let turned: Vec3 = std::array::from_fn(|i| {
        rotation[i][0] * ca[0] + rotation[i][1] * ca[1] + rotation[i][2] * ca[2]
    });
    let mut out = Rigid {
        rotation,
        translation: crate::sub(cb, turned),
        rms: 0.0,
    };
    let sum: f64 = from
        .iter()
        .zip(to)
        .map(|(a, b)| {
            let d = crate::sub(out.apply(*a), *b);
            crate::dot(d, d)
        })
        .sum();
    out.rms = (sum / n as f64).sqrt();
    Some(out)
}

/// Eigenvectors (as columns) and eigenvalues of a symmetric matrix, by
/// Jacobi rotations.
fn jacobi<const N: usize>(mut a: [[f64; N]; N]) -> ([[f64; N]; N], [f64; N]) {
    let mut v = [[0.0; N]; N];
    for (i, row) in v.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for _ in 0..100 {
        let off: f64 = (0..N)
            .flat_map(|i| (0..N).filter(move |j| *j != i).map(move |j| (i, j)))
            .map(|(i, j)| a[i][j] * a[i][j])
            .sum();
        if off < 1e-30 {
            break;
        }
        for p in 0..N {
            for q in p + 1..N {
                if a[p][q].abs() < 1e-300 {
                    continue;
                }
                let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for row in a.iter_mut() {
                    let (akp, akq) = (row[p], row[q]);
                    row[p] = c * akp - s * akq;
                    row[q] = s * akp + c * akq;
                }
                let (rp, rq) = (a[p], a[q]);
                for k in 0..N {
                    a[p][k] = c * rp[k] - s * rq[k];
                    a[q][k] = s * rp[k] + c * rq[k];
                }
                for row in v.iter_mut() {
                    let (vkp, vkq) = (row[p], row[q]);
                    row[p] = c * vkp - s * vkq;
                    row[q] = s * vkp + c * vkq;
                }
            }
        }
    }
    (v, std::array::from_fn(|i| a[i][i]))
}

fn jacobi3(a: [[f64; 3]; 3]) -> ([[f64; 3]; 3], [f64; 3]) {
    jacobi(a)
}

fn jacobi4(a: [[f64; 4]; 4]) -> ([[f64; 4]; 4], [f64; 4]) {
    jacobi(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Four points turned 70° about a slanted axis and moved come back
    /// exactly, and with a little error the motion is still found within
    /// it; three points on a line pin nothing down.
    #[test]
    fn a_rigid_motion_is_found_from_its_pairs() {
        let axis: Vec3 = {
            let a = [1.0, 2.0, 2.0];
            let n = crate::norm(a);
            a.map(|c| c / n)
        };
        let (s, c) = 70f64.to_radians().sin_cos();
        let turn = |p: Vec3| -> Vec3 {
            let cross = crate::cross(axis, p);
            let along = crate::dot(axis, p);
            std::array::from_fn(|i| {
                p[i] * c + cross[i] * s + axis[i] * along * (1.0 - c) + [5.0, -3.0, 12.0][i]
            })
        };
        let from = [
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [0.0, 7.0, 0.0],
            [3.0, 4.0, 9.0],
        ];
        let to: Vec<Vec3> = from.iter().map(|p| turn(*p)).collect();
        let found = rigid(&from, &to).unwrap();
        assert!(found.rms < 1e-9, "{}", found.rms);
        for p in [[1.0, 1.0, 1.0], [-4.0, 2.0, 7.0]] {
            let (a, b) = (found.apply(p), turn(p));
            assert!(crate::norm(crate::sub(a, b)) < 1e-9);
        }
        let mut rough = to.clone();
        rough[1][0] += 0.05;
        let close = rigid(&from, &rough).unwrap();
        assert!(close.rms < 0.05 && close.rms > 0.0);
        let line = [[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        assert!(rigid(&line, &line).is_none());
    }
}
