//! An axis-aligned box in single precision: what a mesh spans, moved by a
//! placement or a matrix as a box around its moved corners, so every
//! caller that moves a box gets the same answer.

/// An axis-aligned box from `min` to `max`, both corners included.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Aabb {
    pub const fn new(min: [f32; 3], max: [f32; 3]) -> Self {
        Self { min, max }
    }

    /// The box around `points`; `None` when there are none.
    pub fn around(points: impl IntoIterator<Item = [f32; 3]>) -> Option<Self> {
        let mut points = points.into_iter();
        let first = points.next()?;
        Some(points.fold(Self::new(first, first), |b, p| b.with(p)))
    }

    /// The box grown to hold `p`.
    pub fn with(self, p: [f32; 3]) -> Self {
        Self {
            min: std::array::from_fn(|k| self.min[k].min(p[k])),
            max: std::array::from_fn(|k| self.max[k].max(p[k])),
        }
    }

    /// The eight corners, corner `i` taking `max` along axis `k` where
    /// bit `k` of `i` is set.
    pub fn corners(&self) -> [[f32; 3]; 8] {
        std::array::from_fn(|i| {
            std::array::from_fn(|k| {
                if i >> k & 1 == 0 {
                    self.min[k]
                } else {
                    self.max[k]
                }
            })
        })
    }

    /// The box around the corners once each is moved by `f`, which for a
    /// rigid motion or any affine map holds everything the box held.
    pub fn moved(&self, f: impl Fn([f32; 3]) -> [f32; 3]) -> Self {
        let [first, rest @ ..] = self.corners().map(f);
        rest.into_iter().fold(Self::new(first, first), Self::with)
    }

    /// The box moved by the affine column-major matrix `m` (`m[3]` the
    /// translation; its last row is not read).
    pub fn transformed(&self, m: &[[f32; 4]; 4]) -> Self {
        self.moved(|p| {
            std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r])
        })
    }

    /// The smallest box holding both.
    pub fn union(&self, other: &Self) -> Self {
        self.with(other.min).with(other.max)
    }

    /// Whether `p` lies in the box, its faces included.
    pub fn contains(&self, p: [f32; 3]) -> bool {
        (0..3).all(|k| self.min[k] <= p[k] && p[k] <= self.max[k])
    }
}

impl From<([f32; 3], [f32; 3])> for Aabb {
    fn from((min, max): ([f32; 3], [f32; 3])) -> Self {
        Self::new(min, max)
    }
}

impl From<Aabb> for ([f32; 3], [f32; 3]) {
    fn from(b: Aabb) -> Self {
        (b.min, b.max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-5)
    }

    /// A box turned a quarter about Z and moved spans what its turned
    /// corners do, by matrix or by function alike.
    #[test]
    fn a_turned_box_spans_its_turned_corners() {
        let b = Aabb::new([1.0, 2.0, 3.0], [4.0, 6.0, 5.0]);
        let (s, c) = (0.3f32.sin(), 0.3f32.cos());
        // Column-major: a turn of 0.3 rad about Z, then (10, -1, 2).
        let m = [
            [c, s, 0.0, 0.0],
            [-s, c, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [10.0, -1.0, 2.0, 1.0],
        ];
        let turn = |p: [f32; 3]| {
            [
                c * p[0] - s * p[1] + 10.0,
                s * p[0] + c * p[1] - 1.0,
                p[2] + 2.0,
            ]
        };
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for x in [1.0, 4.0] {
            for y in [2.0, 6.0] {
                for z in [3.0, 5.0] {
                    let p = turn([x, y, z]);
                    lo = std::array::from_fn(|k| lo[k].min(p[k]));
                    hi = std::array::from_fn(|k| hi[k].max(p[k]));
                }
            }
        }
        for moved in [b.transformed(&m), b.moved(turn)] {
            assert!(close(moved.min, lo) && close(moved.max, hi), "{moved:?}");
        }
    }

    #[test]
    fn union_contains_and_empty() {
        assert_eq!(Aabb::around([]), None);
        let a = Aabb::around([[0.0, 0.0, 0.0], [1.0, 2.0, -1.0]]).unwrap();
        assert_eq!(a, Aabb::new([0.0, 0.0, -1.0], [1.0, 2.0, 0.0]));
        let u = a.union(&Aabb::new([-3.0, 1.0, 0.5], [0.5, 1.5, 4.0]));
        assert_eq!(u, Aabb::new([-3.0, 0.0, -1.0], [1.0, 2.0, 4.0]));
        assert!(u.contains([1.0, 2.0, 4.0]) && u.contains([0.0, 1.0, 0.0]));
        assert!(!u.contains([1.01, 1.0, 0.0]));
    }
}
