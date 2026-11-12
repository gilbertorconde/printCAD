//! A closed polygon in a plane, its corners in order and the last joined
//! back to the first: the area it encloses and whether it holds a point.
//! One implementation over plain coordinates, in single or double
//! precision.

use crate::nearest::Real;

/// The area `polygon` encloses, signed by the way it is walked: positive
/// counter-clockwise (x toward y), negative clockwise; zero with fewer
/// than three corners. A caller wanting the size alone takes `abs`.
pub fn signed_area<R: Real>(polygon: &[[R; 2]]) -> R {
    let n = polygon.len();
    if n < 3 {
        return R::ZERO;
    }
    let mut twice = R::ZERO;
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        twice = twice + (a[0] * b[1] - b[0] * a[1]);
    }
    twice / (R::ONE + R::ONE)
}

/// Whether `p` lies inside `polygons` taken together, by the even-odd
/// rule: a point inside an odd number of them is in, so a polygon inside
/// another is a hole whichever way either is walked.
///
/// A point on an edge is decided by a half-open rule rather than taken
/// as in or out: an edge counts from its lower end up to, not including,
/// its upper end, and a crossing exactly at `p` does not count. Two
/// polygons sharing an edge then never both hold a point on it.
pub fn contains<R: Real>(polygons: &[&[[R; 2]]], p: [R; 2]) -> bool {
    let mut odd = false;
    for polygon in polygons {
        let n = polygon.len();
        for i in 0..n {
            let (a, b) = (polygon[i], polygon[(i + 1) % n]);
            if (a[1] > p[1]) != (b[1] > p[1]) {
                let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if x > p[0] {
                    odd = !odd;
                }
            }
        }
    }
    odd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_is_signed_by_the_walk_and_containment_is_even_odd() {
        let square = [[0.0, 0.0], [2.0, 0.0], [2.0, 3.0], [0.0, 3.0]];
        let mut clockwise = square;
        clockwise.reverse();
        assert_eq!(signed_area(&square), 6.0);
        assert_eq!(signed_area(&clockwise), -6.0);
        assert_eq!(signed_area(&square[..2]), 0.0);
        assert_eq!(signed_area(&[[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0]]), 0.5);

        let hole = [[0.5, 0.5], [1.5, 0.5], [1.5, 1.5], [0.5, 1.5]];
        assert!(contains(&[&square], [1.0, 1.0]));
        assert!(!contains(&[&square, &hole], [1.0, 1.0]));
        assert!(contains(&[&square, &hole], [1.8, 2.5]));
        assert!(!contains(&[&square], [3.0, 1.0]));
        assert!(contains(
            &[&[[0.0f32, 0.0], [4.0, 0.0], [0.0, 4.0]][..]],
            [1.0, 1.0]
        ));
        // Two squares sharing the edge x = 2: a point on it is in one only.
        let right = [[2.0, 0.0], [4.0, 0.0], [4.0, 3.0], [2.0, 3.0]];
        let on_edge = [2.0, 1.0];
        assert_ne!(contains(&[&square], on_edge), contains(&[&right], on_edge));
    }
}
