//! The eigenvalues and eigenvectors of a small symmetric matrix, by
//! Jacobi rotations: the principal axes of an inertia tensor, and the
//! motions an assembly's joints leave a body.

/// The eigenvalues of the symmetric `matrix` (rows by index), unsorted,
/// each with its unit eigenvector at the same index.
///
/// Each rotation zeroes the largest off-diagonal entry left (the last of
/// equal ones, row by row), until none stands above 1e-15 of the largest
/// diagonal entry. A repeated eigenvalue's vectors are some orthonormal
/// basis of its space; a diagonal matrix keeps the coordinate axes.
pub fn symmetric<const N: usize>(matrix: &[[f64; N]; N]) -> ([f64; N], [[f64; N]; N]) {
    let mut a = *matrix;
    // The rotations gathered, one eigenvector a column.
    let mut v = [[0.0f64; N]; N];
    for (i, row) in v.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    let size = (0..N)
        .map(|i| a[i][i].abs())
        .fold(0.0, f64::max)
        .max(1e-300);
    for _ in 0..8 * N * N {
        let mut pivot = (0, 1.min(N - 1));
        for p in 0..N {
            for q in p + 1..N {
                if a[p][q].abs() >= a[pivot.0][pivot.1].abs() {
                    pivot = (p, q);
                }
            }
        }
        let (p, q) = pivot;
        if p == q || a[p][q].abs() <= size * 1e-15 {
            break;
        }
        let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
        let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
        let c = 1.0 / (t * t + 1.0).sqrt();
        let s = t * c;
        for row in a.iter_mut().chain(v.iter_mut()) {
            let (kp, kq) = (row[p], row[q]);
            row[p] = c * kp - s * kq;
            row[q] = s * kp + c * kq;
        }
        let (rp, rq) = (a[p], a[q]);
        a[p] = std::array::from_fn(|k| c * rp[k] - s * rq[k]);
        a[q] = std::array::from_fn(|k| s * rp[k] + c * rq[k]);
    }
    (
        std::array::from_fn(|i| a[i][i]),
        std::array::from_fn(|i| std::array::from_fn(|k| v[k][i])),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every pair answers A v = λ v and the vectors are orthonormal.
    fn holds<const N: usize>(m: &[[f64; N]; N]) -> [f64; N] {
        let (values, vectors) = symmetric(m);
        let scale = m.iter().flatten().fold(1.0f64, |s, x| s.max(x.abs()));
        for (i, v) in vectors.iter().enumerate() {
            for (k, row) in m.iter().enumerate() {
                let av: f64 = row.iter().zip(v).map(|(a, b)| a * b).sum();
                assert!((av - values[i] * v[k]).abs() < 1e-12 * scale, "{m:?}");
            }
            for (j, w) in vectors.iter().enumerate() {
                let dot: f64 = v.iter().zip(w).map(|(a, b)| a * b).sum();
                assert!((dot - f64::from(i == j)).abs() < 1e-12, "{vectors:?}");
            }
        }
        let mut sorted = values;
        sorted.sort_by(f64::total_cmp);
        sorted
    }

    /// `diag` turned by a rotation in the (0, 1) and (1, 2) planes.
    fn rotated<const N: usize>(diag: [f64; N]) -> [[f64; N]; N] {
        let mut q = [[0.0f64; N]; N];
        for (i, row) in q.iter_mut().enumerate() {
            row[i] = 1.0;
        }
        for (p, angle) in [(0, 0.7f64), (1, -1.1)] {
            let (s, c) = angle.sin_cos();
            for row in &mut q {
                let (a, b) = (row[p], row[p + 1]);
                row[p] = c * a - s * b;
                row[p + 1] = s * a + c * b;
            }
        }
        std::array::from_fn(|a| {
            std::array::from_fn(|b| (0..N).map(|i| q[a][i] * diag[i] * q[b][i]).sum())
        })
    }

    #[test]
    fn a_diagonal_matrix_keeps_its_axes() {
        let d = [[3.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 2.0]];
        assert_eq!(
            symmetric(&d),
            (
                [3.0, -1.0, 2.0],
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
            )
        );
    }

    #[test]
    fn a_rotated_matrix_gives_back_its_diagonal() {
        let found = holds(&rotated([4.0, 1.0, 9.0]));
        for (f, want) in found.iter().zip([1.0, 4.0, 9.0]) {
            assert!((f - want).abs() < 1e-12, "{found:?}");
        }
    }

    /// Repeated values, zeros among them as a body's free motions are:
    /// each comes back as often as it stands, with orthonormal vectors.
    #[test]
    fn repeated_eigenvalues_come_back_with_a_basis() {
        let found = holds(&rotated([0.0, 0.0, 5.0, 5.0, 80.0, 2.0]));
        for (f, want) in found.iter().zip([0.0, 0.0, 2.0, 5.0, 5.0, 80.0]) {
            assert!((f - want).abs() < 1e-12, "{found:?}");
        }
        assert_eq!(holds(&[[0.0; 6]; 6]), [0.0; 6]);
    }
}
