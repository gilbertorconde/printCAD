//! The kernel's answers to a bench's geometry questions: how thin a
//! profile's walls get, and where a tube's centre line runs.

use kernel_api::{
    BooleanOp, ExtrudeTermination, FaceProbe, KernelQueries, Profile, ProfilePlane, ProfileSegment,
    ProfileWire, SolidOp, SweepKind,
};
use kernel_ogeom::QUERIES;

use super::common::chain;

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
        names: Vec::new(),
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
        names: Vec::new(),
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
        names: Vec::new(),
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
    chain(ops).brep_blob
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
                name: 0,
                point: [2.5, 0.0, 0.0],
                normal: [0.0, 0.0, -1.0],
            },
            &FaceProbe {
                name: 0,
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
                names: Vec::new(),
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
        frame: kernel_api::PipeFrame::RotationMinimizing,
        corner: kernel_api::PipeCorner::Transformed,
        sections: Vec::new(),
        op: BooleanOp::NewSolid,
    }]);
    let line = QUERIES
        .centre_line(
            &blob,
            &FaceProbe {
                name: 0,
                point: [0.0, 2.5, 0.0],
                normal: [-1.0, 0.0, 0.0],
            },
            &FaceProbe {
                name: 0,
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

/// Points on a sphere and a cylinder, a little noisy and with a few strays,
/// give back the shapes they were taken from, the strays left out.
#[test]
fn fits_find_the_shape_the_points_were_measured_from() {
    use kernel_api::{FitKind, FitRequest, FittedShape};
    let noise = |i: usize| ((i * 7919 % 101) as f64 / 100.0 - 0.5) * 0.02;
    let mut sphere = Vec::new();
    let mut cylinder = Vec::new();
    for i in 0..400 {
        let (a, b) = (i as f64 * 0.37, (i % 23) as f64 / 23.0 * 2.6 - 1.3);
        let r = 12.0 + noise(i);
        sphere.push([
            1.0 + r * b.cos() * a.cos(),
            2.0 + r * b.cos() * a.sin(),
            3.0 + r * b.sin(),
        ]);
        let r = 5.0 + noise(i);
        cylinder.push([r * a.cos(), r * a.sin(), (i % 40) as f64]);
    }
    for points in [&mut sphere, &mut cylinder] {
        for p in points.iter_mut().step_by(50) {
            p[0] += 3.0;
        }
    }
    let ask = |kind, points: &Vec<[f64; 3]>| {
        QUERIES
            .fit(&FitRequest {
                kind,
                points: points.clone(),
                normals: Vec::new(),
                sigma_cutoff: Some(3.0),
                fixed_radius: None,
            })
            .expect("fit")
    };
    let s = ask(FitKind::Sphere, &sphere);
    let FittedShape::Sphere { centre, radius } = s.shape else {
        panic!("{:?}", s.shape)
    };
    assert!((radius - 12.0).abs() < 0.01, "radius {radius}");
    assert!((centre[0] - 1.0).abs() < 0.01 && (centre[2] - 3.0).abs() < 0.01);
    assert_eq!(s.used.iter().filter(|u| !**u).count(), 8, "strays out");
    assert!(s.sigma < 0.01, "sigma {}", s.sigma);

    let c = ask(FitKind::Cylinder, &cylinder);
    let FittedShape::Cylinder { axis, radius, .. } = c.shape else {
        panic!("{:?}", c.shape)
    };
    assert!((radius - 5.0).abs() < 0.01, "radius {radius}");
    assert!(axis[2].abs() > 0.9999, "axis {axis:?}");
}

/// A closed cylinder of radius 10 and height 30 as facets, its side
/// rippled by `ripple` mm as a scan's is, and how many of its triangles
/// are the side's (the ends follow). Coordinates are worked out in double
/// precision and stored as singles.
fn closed_cylinder(ripple: f64) -> (kernel_api::TriMesh, u32) {
    use kernel_api::TriMesh;
    let (n, h) = (64u32, 24u32);
    let mut mesh = TriMesh::default();
    for j in 0..=h {
        for i in 0..n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            let a = f64::from(a);
            let r = 10.0 + ripple * (a * 7.0 + f64::from(j)).sin();
            mesh.positions.push([
                (r * a.cos()) as f32,
                (r * a.sin()) as f32,
                j as f32 * 30.0 / h as f32,
            ]);
        }
    }
    let ring = |j: u32, i: u32| j * n + i % n;
    for j in 0..h {
        for i in 0..n {
            let (a, b, c, d) = (
                ring(j, i),
                ring(j, i + 1),
                ring(j + 1, i + 1),
                ring(j + 1, i),
            );
            mesh.indices.extend([a, b, c, a, c, d]);
        }
    }
    let side = mesh.indices.len() as u32 / 3;
    for (j, z, up) in [(0, 0.0f32, false), (h, 30.0, true)] {
        // Each end has its own rim vertices, split from the side's at the
        // crease, as an imported mesh is.
        let centre = mesh.positions.len() as u32;
        mesh.positions.push([0.0, 0.0, z]);
        for i in 0..n {
            let p = mesh.positions[ring(j, i) as usize];
            mesh.positions.push(p);
        }
        let rim = |i: u32| centre + 1 + i % n;
        for i in 0..n {
            let (a, b) = (rim(i), rim(i + 1));
            mesh.indices
                .extend(if up { [centre, a, b] } else { [centre, b, a] });
        }
    }
    (mesh, side)
}

/// A click on a scanned cylinder's side grows over the side and not the
/// ends, and fits its radius; a click on an end grows the flat end.
#[test]
fn a_click_grows_over_the_surface_it_lands_on() {
    use kernel_api::{FitKind, FittedShape, GrowRequest};
    let (mesh, side) = closed_cylinder(0.02);
    let n = 64u32;
    let ask = |kind, seed| {
        QUERIES
            .grow(
                &mesh,
                &GrowRequest {
                    kind,
                    seeds: vec![seed],
                    sigma_cutoff: None,
                    normal_limit_deg: None,
                    peel: false,
                    max_triangles: None,
                },
            )
            .expect("grow")
    };
    let wall = ask(FitKind::Cylinder, [10.0, 0.0, 15.0]);
    let FittedShape::Cylinder { radius, .. } = wall.fit.shape else {
        panic!("{:?}", wall.fit.shape)
    };
    // The facets' chords sit inside the true circle: the fit lands between.
    assert!((radius - 10.0).abs() < 0.05, "radius {radius}");
    assert!(wall.triangles.iter().all(|t| *t < side), "no end joins");
    assert!(
        wall.triangles.len() as u32 > side * 9 / 10,
        "{}",
        wall.triangles.len()
    );
    let top = ask(FitKind::Plane, [1.0, 1.0, 30.0]);
    assert!(top.triangles.iter().all(|t| *t >= side + n));
    assert!(matches!(top.fit.shape, FittedShape::Plane { normal, .. } if normal[2].abs() > 0.999));
}

/// A click by a sharp rim traces the rim and fits its circle.
#[test]
fn a_click_by_a_rim_fits_its_circle() {
    use kernel_api::{FitKind, FittedShape, GrowRequest};
    let (mesh, _) = closed_cylinder(0.02);
    let rim = QUERIES
        .grow(
            &mesh,
            &GrowRequest {
                kind: FitKind::Circle,
                seeds: vec![[10.0, 0.0, 29.9]],
                sigma_cutoff: None,
                normal_limit_deg: None,
                peel: false,
                max_triangles: None,
            },
        )
        .expect("grow");
    assert!(
        matches!(rim.fit.shape, FittedShape::Circle { radius, centre, .. }
        if (radius - 10.0).abs() < 0.05 && (centre[2] - 30.0).abs() < 1e-6)
    );
}

/// A mesh with no noise at all (a part exported as triangles) grows as
/// whole as a scanned one.
#[test]
fn a_noise_free_mesh_grows_whole() {
    use kernel_api::{FitKind, GrowRequest};
    let (mesh, side) = closed_cylinder(0.0);
    let request = GrowRequest {
        kind: FitKind::Cylinder,
        seeds: vec![[10.0, 0.0, 15.0]],
        sigma_cutoff: None,
        normal_limit_deg: None,
        peel: false,
        max_triangles: None,
    };
    let wall = QUERIES.grow(&mesh, &request).expect("grow");
    assert!(
        wall.triangles.len() as u32 > side * 95 / 100,
        "{}",
        wall.triangles.len()
    );
}

/// Points half a millimetre outside and inside read ±0.5 against a
/// cylinder as its triangles and against a solid cylinder exactly; one out
/// of reach reads unmeasured.
#[test]
fn points_read_their_signed_distance_to_a_reference() {
    use kernel_api::{DistanceRequest, DistanceTarget, PrimitiveKind};
    let (mesh, _) = closed_cylinder(0.0);
    let request = DistanceRequest {
        points: vec![
            [10.5, 0.0, 15.0],
            [0.0, -9.5, 15.0],
            [0.0, 0.0, 31.0],
            [80.0, 0.0, 0.0],
        ],
        max: Some(5.0),
        ..DistanceRequest::default()
    };
    let solid = super::common::chain(&[super::common::primitive(PrimitiveKind::Cylinder {
        radius: 10.0,
        height: 30.0,
        angle_deg: 360.0,
    })]);
    for (target, slack) in [
        (DistanceTarget::Mesh(&mesh), 0.02),
        (DistanceTarget::Shape(&solid.brep_blob), 1e-3),
    ] {
        let d = QUERIES
            .signed_distance(target, &request)
            .expect("distances");
        for (got, want) in d.iter().zip([0.5, -0.5, 1.0]) {
            assert!((got - want).abs() < slack, "{got} for {want}");
        }
        assert!(d[3].is_nan(), "out of reach");
    }
}
