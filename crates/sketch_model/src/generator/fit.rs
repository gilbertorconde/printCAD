//! A curve as a sketch B-spline: a clamped cubic over uniformly spaced
//! knots (the spline the sketch and the kernel both read from a list of
//! control points), fitted to points on the curve by least squares with
//! its ends held on the curve's ends.

use super::{P2, norm, sub};

/// The clamped uniform knot vector of a cubic over `n` control points.
fn knots(n: usize) -> Vec<f64> {
    let degree = 3.min(n - 1);
    let inner = n - degree - 1;
    let mut k = vec![0.0; degree + 1];
    k.extend((1..=inner).map(|i| i as f64 / (inner + 1) as f64));
    k.extend(std::iter::repeat_n(1.0, degree + 1));
    k
}

/// The `n` basis functions at `t` in [0, 1].
fn basis(n: usize, t: f64) -> Vec<f64> {
    let degree = 3.min(n - 1);
    let k = knots(n);
    // Degree 0: the span holding t (the last one holds t = 1).
    let mut b: Vec<f64> = (0..k.len() - 1)
        .map(|i| {
            let inside = k[i] <= t && t < k[i + 1];
            let last = t >= 1.0 && k[i] < 1.0 && k[i + 1] >= 1.0;
            if inside || last { 1.0 } else { 0.0 }
        })
        .collect();
    for p in 1..=degree {
        for i in 0..k.len() - 1 - p {
            let left = if k[i + p] > k[i] {
                (t - k[i]) / (k[i + p] - k[i]) * b[i]
            } else {
                0.0
            };
            let right = if k[i + p + 1] > k[i + 1] {
                (k[i + p + 1] - t) / (k[i + p + 1] - k[i + 1]) * b[i + 1]
            } else {
                0.0
            };
            b[i] = left + right;
        }
    }
    b.truncate(n);
    b
}

/// The point at `t` in [0, 1] of the spline over `poles`.
pub fn evaluate(poles: &[P2], t: f64) -> P2 {
    let b = basis(poles.len(), t.clamp(0.0, 1.0));
    poles.iter().zip(&b).fold([0.0, 0.0], |acc, (p, w)| {
        [acc[0] + p[0] * w, acc[1] + p[1] * w]
    })
}

/// The control points of an `n`-pole spline through `points`' ends,
/// nearest the rest in the least-squares sense, each point taken at its
/// share of the length along them.
fn fit_poles(points: &[P2], n: usize) -> Vec<P2> {
    let first = points[0];
    let last = *points.last().expect("points");
    if n <= 2 {
        return vec![first, last];
    }
    let mut along = vec![0.0];
    for w in points.windows(2) {
        along.push(along.last().unwrap() + norm(sub(w[1], w[0])));
    }
    let total = *along.last().unwrap();
    let unknowns = n - 2;
    // Normal equations A^T A x = A^T r for the inner poles, one right-hand
    // side per coordinate.
    let mut ata = vec![vec![0.0; unknowns]; unknowns];
    let mut atr = vec![[0.0; 2]; unknowns];
    for (q, s) in points.iter().zip(&along) {
        let b = basis(n, s / total);
        let rest = [
            q[0] - b[0] * first[0] - b[n - 1] * last[0],
            q[1] - b[0] * first[1] - b[n - 1] * last[1],
        ];
        for i in 0..unknowns {
            let bi = b[i + 1];
            if bi == 0.0 {
                continue;
            }
            for j in 0..unknowns {
                ata[i][j] += bi * b[j + 1];
            }
            atr[i][0] += bi * rest[0];
            atr[i][1] += bi * rest[1];
        }
    }
    let inner = solve(ata, atr);
    let mut poles = vec![first];
    poles.extend(inner);
    poles.push(last);
    poles
}

/// Gaussian elimination with partial pivoting, two right-hand sides.
fn solve(mut a: Vec<Vec<f64>>, mut r: Vec<P2>) -> Vec<P2> {
    let n = a.len();
    for col in 0..n {
        let pivot = (col..n)
            .max_by(|&x, &y| a[x][col].abs().total_cmp(&a[y][col].abs()))
            .unwrap_or(col);
        a.swap(col, pivot);
        r.swap(col, pivot);
        let d = a[col][col];
        if d.abs() < 1e-300 {
            continue;
        }
        for row in col + 1..n {
            let f = a[row][col] / d;
            if f == 0.0 {
                continue;
            }
            let (upper, lower) = a.split_at_mut(row);
            for (x, y) in lower[0][col..].iter_mut().zip(&upper[col][col..]) {
                *x -= f * y;
            }
            r[row][0] -= f * r[col][0];
            r[row][1] -= f * r[col][1];
        }
    }
    let mut x = vec![[0.0; 2]; n];
    for row in (0..n).rev() {
        let mut s = r[row];
        for k in row + 1..n {
            s[0] -= a[row][k] * x[k][0];
            s[1] -= a[row][k] * x[k][1];
        }
        let d = a[row][row];
        x[row] = if d.abs() < 1e-300 {
            [0.0, 0.0]
        } else {
            [s[0] / d, s[1] / d]
        };
    }
    x
}

/// The fewest control points (from four up to `max`) whose spline stays
/// within `tolerance` of the curve, sampled densely as `points`; `error`
/// measures how far a point is from the curve. Answers the control points
/// and how far the fit strays.
pub fn fit(points: &[P2], tolerance: f64, max: usize, error: impl Fn(P2) -> f64) -> (Vec<P2>, f64) {
    let mut best: Option<(Vec<P2>, f64)> = None;
    for n in 4..=max.max(4) {
        let poles = fit_poles(points, n);
        let worst = (0..=400)
            .map(|i| error(evaluate(&poles, i as f64 / 400.0)))
            .fold(0.0, f64::max);
        if worst <= tolerance {
            return (poles, worst);
        }
        if best.as_ref().is_none_or(|(_, e)| worst < *e) {
            best = Some((poles, worst));
        }
    }
    best.expect("at least one fit")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spline_runs_from_its_first_pole_to_its_last() {
        let poles = [[0.0, 0.0], [1.0, 2.0], [3.0, 2.0], [4.0, 0.0], [5.0, 1.0]];
        assert_eq!(evaluate(&poles, 0.0), poles[0]);
        let end = evaluate(&poles, 1.0);
        assert!(norm(sub(end, poles[4])) < 1e-12);
    }

    #[test]
    fn the_basis_sums_to_one() {
        for n in 4..9 {
            for i in 0..=20 {
                let s: f64 = basis(n, i as f64 / 20.0).iter().sum();
                assert!((s - 1.0).abs() < 1e-12, "n {n} at {i}: {s}");
            }
        }
    }

    #[test]
    fn a_quarter_circle_fits_within_a_micrometre() {
        let r = 10.0;
        let points: Vec<P2> = (0..=200)
            .map(|i| {
                let t = std::f64::consts::FRAC_PI_2 * i as f64 / 200.0;
                [r * t.cos(), r * t.sin()]
            })
            .collect();
        let (poles, worst) = fit(&points, 1e-3, 30, |p| (norm(p) - r).abs());
        assert!(worst <= 1e-3, "{worst}");
        assert!(poles.len() < 12, "{} poles", poles.len());
    }
}
