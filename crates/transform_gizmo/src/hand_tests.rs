use glam::DVec3;

use super::*;

fn unit(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z).normalize()
}

#[test]
fn slide_delta_is_the_projection_on_the_axis() {
    let axis = unit(1.0, 1.0, 0.0);
    let delta = slide_delta(axis, DVec3::new(3.0, 1.0, 50.0));
    assert!((delta - 4.0 / 2.0_f64.sqrt()).abs() < 1e-14, "{delta:?}");
}

/// The part off the plane is dropped: the result is the orthogonal projection.
#[test]
fn plane_split_drops_the_normal_component() {
    let first = unit(1.0, 0.0, 0.0);
    let second = unit(1.0, 1.0, 0.0);
    let plane = SlidePlane::new(first, second).unwrap();
    let (along_first, along_second) = plane.split(DVec3::new(4.0, 3.0, 100.0));
    let in_plane = first * along_first + second * along_second;
    assert!(
        (in_plane - DVec3::new(4.0, 3.0, 0.0)).length() < 1e-13,
        "{in_plane:?}"
    );
}

#[test]
fn collinear_pair_has_no_plane() {
    let axis = unit(1.0, 2.0, 3.0);
    assert!(SlidePlane::new(axis, axis).is_none());
    assert!(SlidePlane::new(axis, -axis).is_none());
    // The threshold: a sine of 1e-7 is below 1e-6, 1e-5 above.
    let near = |sin: f64| DVec3::new((1.0 - sin * sin).sqrt(), sin, 0.0);
    assert!(SlidePlane::new(unit(1.0, 0.0, 0.0), near(1e-7)).is_none());
    assert!(SlidePlane::new(unit(1.0, 0.0, 0.0), near(1e-5)).is_some());
}

/// Unit directions spread evenly over the sphere (a Fibonacci lattice).
fn sphere(count: usize) -> Vec<DVec3> {
    let golden = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
    (0..count)
        .map(|i| {
            let z = 1.0 - 2.0 * (i as f64 + 0.5) / count as f64;
            let ring = (1.0 - z * z).sqrt();
            let turn = golden * i as f64;
            DVec3::new(ring * turn.cos(), ring * turn.sin(), z)
        })
        .collect()
}

/// Gram: every pair of skewed slides over the sphere recovers a delta in its
/// plane, whatever the amounts.
#[test]
fn slide_pair_recovers_an_in_plane_delta() {
    let directions = sphere(48);
    let mut checked = 0;
    for (index, &first) in directions.iter().enumerate() {
        for &second in &directions[index + 1..] {
            let sine = first.cross(second).length();
            if sine <= 0.05 {
                continue;
            }
            let plane = SlidePlane::new(first, second).expect("non-collinear pair");
            for amounts in [[-1e3, 7.5], [3.25, 999.0], [-0.001, -512.0]] {
                let (along_first, along_second) =
                    plane.split(amounts[0] * first + amounts[1] * second);
                // The Gram matrix's condition number goes as 1/sin²: so does the tolerance.
                let tolerance = 1e-12 * 1e3 / (sine * sine);
                assert!(
                    (along_first - amounts[0]).abs() < tolerance,
                    "{along_first} vs {}",
                    amounts[0]
                );
                assert!(
                    (along_second - amounts[1]).abs() < tolerance,
                    "{along_second} vs {}",
                    amounts[1]
                );
                checked += 1;
            }
        }
    }
    // 48 directions give 1128 pairs, three deltas each, minus the near-collinear.
    assert!(checked > 3000, "{checked}");
}

#[test]
#[should_panic(expected = "slide_delta: direction must be a finite unit vector")]
fn slide_delta_rejects_a_non_unit_axis() {
    let _ = slide_delta(DVec3::new(2.0, 0.0, 0.0), DVec3::new(1.0, 0.0, 0.0));
}
