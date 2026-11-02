//! How a print came out against its model, axis by axis: from each scan
//! point's signed distance to the model along its normal, the stretch of
//! each axis about a centre (`1 + e`, a shrinking print reading `e < 0`)
//! and a leftover shift, by least squares; the points straying past three
//! times the fit's RMS (a support's remains, a defect) left out and the
//! rest fitted again. The factors that compensate are `1 / (1 + e)`.

use crate::Vec3;

/// A print's stretch per axis, and what is left over.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shrink {
    /// Each axis's stretch less one: −0.004 a print 0.4 % small that way.
    pub stretch: Vec3,
    /// The shift the fit took out with it, mm.
    pub shift: Vec3,
    /// The distances' RMS before the fit and after, mm.
    pub rms: [f64; 2],
    /// The points used, and of how many.
    pub used: usize,
    pub points: usize,
}

impl Shrink {
    /// The factors that scale the model so a print comes out as it was
    /// drawn.
    pub fn compensation(&self) -> Vec3 {
        self.stretch.map(|e| 1.0 / (1.0 + e))
    }
}

/// The stretch of each axis about `centre` that best explains `distance`
/// (signed, out of the model positive) at `points` with outward
/// `normals`; with `tie_xy`, one stretch for X and Y together. `None`
/// with too few points or directions to tell the axes apart.
pub fn fit(
    points: &[Vec3],
    normals: &[Vec3],
    distance: &[f64],
    centre: Vec3,
    tie_xy: bool,
) -> Option<Shrink> {
    let n = points.len().min(normals.len()).min(distance.len());
    let usable: Vec<usize> = (0..n).filter(|i| distance[*i].is_finite()).collect();
    if usable.len() < 12 {
        return None;
    }
    let row = |i: usize| -> Vec<f64> {
        let (p, m) = (points[i], normals[i]);
        let a = [
            (p[0] - centre[0]) * m[0],
            (p[1] - centre[1]) * m[1],
            (p[2] - centre[2]) * m[2],
        ];
        if tie_xy {
            vec![a[0] + a[1], a[2], m[0], m[1], m[2]]
        } else {
            vec![a[0], a[1], a[2], m[0], m[1], m[2]]
        }
    };
    let solve = |keep: &[usize]| -> Option<Vec<f64>> {
        let k = if tie_xy { 5 } else { 6 };
        let mut ata = vec![vec![0.0; k]; k];
        let mut atb = vec![0.0; k];
        for &i in keep {
            let r = row(i);
            for a in 0..k {
                atb[a] += r[a] * distance[i];
                for b in 0..k {
                    ata[a][b] += r[a] * r[b];
                }
            }
        }
        gauss(ata, atb)
    };
    let residual = |x: &[f64], i: usize| -> f64 {
        let r = row(i);
        distance[i] - r.iter().zip(x).map(|(a, b)| a * b).sum::<f64>()
    };
    let rms = |values: &mut dyn Iterator<Item = f64>| -> f64 {
        let (s, c) = values.fold((0.0, 0usize), |(s, c), v| (s + v * v, c + 1));
        if c == 0 { 0.0 } else { (s / c as f64).sqrt() }
    };
    let mut keep = usable.clone();
    let mut x = solve(&keep)?;
    for _ in 0..4 {
        let spread = rms(&mut keep.iter().map(|i| residual(&x, *i)));
        let limit = 3.0 * spread.max(1e-9);
        let before = keep.len();
        keep.retain(|i| residual(&x, *i).abs() <= limit);
        if keep.len() < 12 {
            return None;
        }
        x = solve(&keep)?;
        if keep.len() == before {
            break;
        }
    }
    let (stretch, shift) = if tie_xy {
        ([x[0], x[0], x[1]], [x[2], x[3], x[4]])
    } else {
        ([x[0], x[1], x[2]], [x[3], x[4], x[5]])
    };
    Some(Shrink {
        stretch,
        shift,
        rms: [
            rms(&mut keep.iter().map(|i| distance[*i])),
            rms(&mut keep.iter().map(|i| residual(&x, *i))),
        ],
        used: keep.len(),
        points: n,
    })
}

/// `a x = b` by Gaussian elimination with partial pivoting.
fn gauss(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let pivot = (col..n).max_by(|i, j| a[*i][col].abs().total_cmp(&a[*j][col].abs()))?;
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        let (top, rest) = a.split_at_mut(col + 1);
        let pivot_row = &top[col];
        for (offset, row) in rest.iter_mut().enumerate() {
            let f = row[col] / pivot_row[col];
            for (dst, src) in row[col..].iter_mut().zip(&pivot_row[col..]) {
                *dst -= f * src;
            }
            b[col + 1 + offset] -= f * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for col in (0..n).rev() {
        let s: f64 = (col + 1..n).map(|k| a[col][k] * x[k]).sum();
        x[col] = (b[col] - s) / a[col][col];
    }
    Some(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box 40 × 30 × 20 printed 0.5 % small across and 1 % short, and
    /// shifted a little: the fit reads each axis's stretch and the shift,
    /// and the compensation scales it back; a speck far off is left out.
    #[test]
    fn a_shrunk_print_reads_its_axes() {
        let half = [20.0, 15.0, 10.0];
        let stretch = [-0.005, -0.005, -0.01];
        let shift = [0.05, -0.02, 0.0];
        let (mut points, mut normals, mut distance) = (Vec::new(), Vec::new(), Vec::new());
        // Points on each face of the model, with the print's surface's
        // distance from it there along the face's normal.
        for axis in 0..3 {
            for side in [-1.0, 1.0] {
                for a in 0..6 {
                    for b in 0..6 {
                        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                        let mut p = [0.0; 3];
                        p[axis] = side * half[axis];
                        p[u] = (f64::from(a) / 5.0 - 0.5) * 1.6 * half[u];
                        p[v] = (f64::from(b) / 5.0 - 0.5) * 1.6 * half[v];
                        let mut n = [0.0; 3];
                        n[axis] = side;
                        points.push(p);
                        normals.push(n);
                        distance.push(side * (stretch[axis] * p[axis] + shift[axis]));
                    }
                }
            }
        }
        distance[7] += 3.0;
        let free = fit(&points, &normals, &distance, [0.0; 3], false).unwrap();
        for k in 0..3 {
            assert!((free.stretch[k] - stretch[k]).abs() < 1e-9, "{free:?}");
            assert!((free.shift[k] - shift[k]).abs() < 1e-9);
        }
        assert_eq!((free.used, free.points), (215, 216));
        assert!(free.rms[1] < 1e-9 && free.rms[0] > 0.01);
        assert!((free.compensation()[2] - 1.0 / 0.99).abs() < 1e-12);
        let tied = fit(&points, &normals, &distance, [0.0; 3], true).unwrap();
        assert!((tied.stretch[0] + 0.005).abs() < 1e-9 && (tied.stretch[2] + 0.01).abs() < 1e-9);
    }
}
