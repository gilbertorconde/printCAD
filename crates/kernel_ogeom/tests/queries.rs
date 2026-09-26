//! The kernel's answers to a bench's geometry questions: how thin a
//! profile's walls get, and where a tube's centre line runs.

use kernel_api::{
    BooleanOp, ExtrudeTermination, FaceProbe, KernelQueries, Profile, ProfilePlane, ProfileSegment,
    ProfileWire, SolidOp, SweepKind, TessellationSettings,
};
use kernel_ogeom::{OgeomKernel, QUERIES};

fn xy_plane() -> ProfilePlane {
    ProfilePlane {
        origin: [0.0, 0.0, 0.0],
        x_axis: [1.0, 0.0, 0.0],
        y_axis: [0.0, 1.0, 0.0],
        normal: [0.0, 0.0, 1.0],
    }
}

fn polygon(corners: &[[f64; 2]]) -> ProfileWire {
    ProfileWire {
        segments: (0..corners.len())
            .map(|i| ProfileSegment::Line {
                start: corners[i],
                end: corners[(i + 1) % corners.len()],
            })
            .collect(),
    }
}

fn circle(radius: f64) -> ProfileWire {
    ProfileWire {
        segments: vec![ProfileSegment::Circle {
            center: [0.0, 0.0],
            radius,
        }],
    }
}

fn thinnest_wall(wires: Vec<ProfileWire>) -> (f64, [f64; 2]) {
    let regions = QUERIES
        .medial_axis(
            &Profile {
                plane: xy_plane(),
                wires,
            },
            1e-3,
        )
        .expect("a medial axis");
    assert_eq!(regions.len(), 1, "one region");
    let region = &regions[0];
    assert!(!region.paths.is_empty(), "the axis has branches");
    for path in &region.paths {
        assert_eq!(path.points.len(), path.clearance.len());
    }
    let narrowest = region.narrowest.expect("a narrowest place");
    (2.0 * narrowest.clearance, narrowest.at)
}

#[test]
fn a_strip_is_as_thin_as_it_is_wide() {
    let (wall, at) = thinnest_wall(vec![polygon(&[
        [0.0, 0.0],
        [20.0, 0.0],
        [20.0, 2.0],
        [0.0, 2.0],
    ])]);
    assert!((wall - 2.0).abs() < 1e-3, "{wall}");
    assert!((at[1] - 1.0).abs() < 1e-3, "{at:?}");
}

/// A rectangle from the origin with every corner rounded to `r`.
fn rounded_rectangle(w: f64, h: f64, r: f64) -> ProfileWire {
    let s = std::f64::consts::FRAC_1_SQRT_2 * r;
    let arc = |c: [f64; 2], from: [f64; 2], mid: [f64; 2], to: [f64; 2]| ProfileSegment::Arc {
        start: [c[0] + from[0] * r, c[1] + from[1] * r],
        mid: [c[0] + mid[0] * s, c[1] + mid[1] * s],
        end: [c[0] + to[0] * r, c[1] + to[1] * r],
    };
    let line = |start: [f64; 2], end: [f64; 2]| ProfileSegment::Line { start, end };
    ProfileWire {
        segments: vec![
            line([r, 0.0], [w - r, 0.0]),
            arc([w - r, r], [0.0, -1.0], [1.0, -1.0], [1.0, 0.0]),
            line([w, r], [w, h - r]),
            arc([w - r, h - r], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]),
            line([w - r, h], [r, h]),
            arc([r, h - r], [0.0, 1.0], [-1.0, 1.0], [-1.0, 0.0]),
            line([0.0, h - r], [0.0, r]),
            arc([r, r], [-1.0, 0.0], [-1.0, -1.0], [0.0, -1.0]),
        ],
    }
}

#[test]
fn rounded_corners_do_not_make_a_plate_thin() {
    let (wall, _) = thinnest_wall(vec![rounded_rectangle(20.0, 6.0, 0.5)]);
    assert!((wall - 6.0).abs() < 1e-3, "{wall}");
}

#[test]
fn an_l_of_two_thicknesses_is_as_thin_as_its_thinner_arm() {
    // A 3 mm arm along x and a 1.2 mm arm up y.
    let (wall, at) = thinnest_wall(vec![polygon(&[
        [0.0, 0.0],
        [30.0, 0.0],
        [30.0, 3.0],
        [1.2, 3.0],
        [1.2, 25.0],
        [0.0, 25.0],
    ])]);
    assert!((wall - 1.2).abs() < 1e-3, "{wall}");
    assert!(at[1] > 3.0, "on the thin arm: {at:?}");
}

#[test]
fn a_ring_is_as_thin_as_its_width() {
    let (wall, _) = thinnest_wall(vec![circle(10.0), circle(7.0)]);
    assert!((wall - 3.0).abs() < 1e-3, "{wall}");
}

#[test]
fn a_square_ring_is_as_thin_as_its_width_beside_the_hole() {
    let (wall, at) = thinnest_wall(vec![
        polygon(&[[-10.0, -10.0], [10.0, -10.0], [10.0, 10.0], [-10.0, 10.0]]),
        circle(7.0),
    ]);
    assert!((wall - 3.0).abs() < 1e-3, "{wall}");
    // Midway between the hole and a side.
    let r = at[0].hypot(at[1]);
    assert!((r - 8.5).abs() < 1e-2, "{at:?}");
}

#[test]
fn a_disc_is_as_thin_as_its_diameter() {
    let regions = QUERIES
        .medial_axis(
            &Profile {
                plane: xy_plane(),
                wires: vec![circle(4.0)],
            },
            1e-3,
        )
        .expect("a medial axis");
    let narrowest = regions[0].narrowest.expect("its centre");
    assert!((narrowest.clearance - 4.0).abs() < 1e-6);
    assert!(narrowest.at[0].abs() < 1e-6 && narrowest.at[1].abs() < 1e-6);
}

fn ring_plane_at_origin_facing_x() -> ProfilePlane {
    ProfilePlane {
        origin: [0.0, 0.0, 0.0],
        x_axis: [0.0, 0.0, 1.0],
        y_axis: [0.0, 1.0, 0.0],
        normal: [1.0, 0.0, 0.0],
    }
}

fn solid(ops: &[SolidOp]) -> Vec<u8> {
    OgeomKernel::new()
        .execute_solid_chain(ops, &TessellationSettings::default())
        .expect("a solid")
        .brep_blob
}

#[test]
fn a_straight_tube_has_a_straight_centre_line_its_length() {
    let blob = solid(&[SolidOp::Sweep {
        profile: Profile {
            plane: xy_plane(),
            wires: vec![circle(3.0), circle(2.0)],
        },
        kind: SweepKind::Extrude {
            termination: ExtrudeTermination::Blind { distance: 20.0 },
            second_side: None,
            symmetric: false,
            reversed: false,
            taper_deg: 0.0,
            direction: None,
        },
        op: BooleanOp::NewSolid,
    }]);
    let line = QUERIES
        .centre_line(
            &blob,
            &FaceProbe {
                point: [2.5, 0.0, 0.0],
                normal: [0.0, 0.0, -1.0],
            },
            &FaceProbe {
                point: [0.0, 2.5, 20.0],
                normal: [0.0, 0.0, 1.0],
            },
            0.01,
        )
        .expect("a centre line");
    assert!((line.length - 20.0).abs() < 1e-3, "{}", line.length);
    assert!(line.straight);
    let first = line.points.first().unwrap();
    let last = line.points.last().unwrap();
    for (p, z) in [(first, 0.0), (last, 20.0)] {
        assert!(p[0].abs() < 1e-3 && p[1].abs() < 1e-3, "{p:?}");
        assert!((p[2] - z).abs() < 1e-3, "{p:?}");
    }
}

#[test]
fn a_bent_tube_has_a_centre_line_along_its_bend() {
    // A ring swept a quarter turn round (0, 20): the spine is 10π long.
    let blob = solid(&[SolidOp::Pipe {
        profile: Profile {
            plane: ring_plane_at_origin_facing_x(),
            wires: vec![circle(3.0), circle(2.0)],
        },
        spine: Profile {
            plane: xy_plane(),
            wires: vec![ProfileWire {
                segments: vec![ProfileSegment::Arc {
                    start: [0.0, 0.0],
                    mid: [
                        20.0 * std::f64::consts::FRAC_1_SQRT_2,
                        20.0 - 20.0 * std::f64::consts::FRAC_1_SQRT_2,
                    ],
                    end: [20.0, 20.0],
                }],
            }],
        },
        frenet: false,
        op: BooleanOp::NewSolid,
    }]);
    let line = QUERIES
        .centre_line(
            &blob,
            &FaceProbe {
                point: [0.0, 2.5, 0.0],
                normal: [-1.0, 0.0, 0.0],
            },
            &FaceProbe {
                point: [22.5, 20.0, 0.0],
                normal: [0.0, 1.0, 0.0],
            },
            0.01,
        )
        .expect("a centre line");
    let want = 10.0 * std::f64::consts::PI;
    assert!(
        (line.length - want).abs() < 0.05,
        "{} against {want}",
        line.length
    );
    assert!(!line.straight);
    assert!(line.deviation <= 0.01, "{}", line.deviation);
    // Every point sits on the spine's circle.
    for p in &line.points {
        let r = (p[0].powi(2) + (p[1] - 20.0).powi(2)).sqrt();
        assert!((r - 20.0).abs() < 0.05 && p[2].abs() < 0.05, "{p:?}");
    }
    let first = line.points.first().unwrap();
    assert!(first[0].abs() < 1e-2 && first[1].abs() < 1e-2, "{first:?}");
}
