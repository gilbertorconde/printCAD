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

#[test]
fn scans_propose_upright_frames_and_mirror_planes() {
    let solid = chain(&[super::common::primitive(kernel_api::PrimitiveKind::Box {
        length: 40.0,
        width: 20.0,
        height: 10.0,
    })]);
    let (cylinder, _) = closed_cylinder(0.0);
    let mut disc = cylinder.clone();
    for p in &mut disc.positions {
        p[2] *= 0.05;
    }
    for (mut mesh, round) in [(solid.mesh, false), (cylinder, true), (disc, true)] {
        for p in &mut mesh.positions {
            let [x, y, z] = *p;
            let (s, c) = 0.7_f32.sin_cos();
            *p = [c * x - s * z + 8.0, s * x + c * z - 3.0, -y + 11.0];
        }
        let proposal = QUERIES.orient(&mesh).unwrap();
        assert_eq!(
            proposal.said,
            if round {
                "axis of revolution"
            } else {
                "largest flat face"
            }
        );
        let vector = |p: [f64; 3]| ogeom::math::Vector::new(p[0], p[1], p[2]);
        let z = vector(proposal.z_axis);
        let x = vector(proposal.x_axis);
        assert!(z.dot(x).abs() < 1e-8);
        let origin = vector(proposal.origin);
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for p in &mesh.positions {
            let d = vector(p.map(f64::from)) - origin;
            for (i, a) in [x, z.cross(x), z].iter().enumerate() {
                lo[i] = lo[i].min(d.dot(*a));
                hi[i] = hi[i].max(d.dot(*a));
            }
        }
        assert!(lo[2].abs() < 1e-6 && (lo[0] + hi[0]).abs() < 1e-6 && (lo[1] + hi[1]).abs() < 1e-6);
        if !round {
            assert!((hi[2] - 10.0).abs() < 1e-4 && (hi[0] - lo[0] - 40.0).abs() < 1e-4);
        }
        let symmetry = QUERIES.symmetry_plane(&mesh).unwrap();
        assert!(symmetry.rms < 1e-4, "{symmetry:?}");
    }
    assert!(QUERIES.orient(&kernel_api::TriMesh::default()).is_err());
}

/// A box's own surface points, turned and moved, fit back onto the box:
/// from where they are for a small motion, and with the search over every
/// axis matching for a turn of 160°.
#[test]
fn points_moved_off_a_reference_fit_back_onto_it() {
    use kernel_api::{DistanceTarget, PrimitiveKind, RegisterRequest};
    let solid = super::common::chain(&[super::common::primitive(PrimitiveKind::Box {
        length: 40.0,
        width: 20.0,
        height: 10.0,
    })]);
    let target = DistanceTarget::Shape(&solid.brep_blob);
    // A grid on each of the six faces, so no face can slide along itself.
    let size = [40.0, 20.0, 10.0];
    let points: Vec<[f64; 3]> = (0..3)
        .flat_map(|axis| {
            let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
            (0..2).flat_map(move |side| {
                (0..64).map(move |k| {
                    let mut p = [0.0; 3];
                    p[axis] = side as f64 * size[axis];
                    p[a] = ((k % 8) as f64 + 0.5) / 8.0 * size[a];
                    p[b] = ((k / 8) as f64 + 0.5) / 8.0 * size[b];
                    p
                })
            })
        })
        .collect();
    for (turn_deg, search) in [(12.0f64, false), (160.0, true)] {
        let (c, s) = (turn_deg.to_radians().cos(), turn_deg.to_radians().sin());
        let moved: Vec<[f64; 3]> = points
            .iter()
            .map(|p| {
                [
                    c * p[0] - s * p[1] + 3.0,
                    s * p[0] + c * p[1] - 2.0,
                    p[2] + 1.0,
                ]
            })
            .collect();
        let fit = QUERIES
            .register(
                target,
                &RegisterRequest {
                    points: moved.clone(),
                    search,
                    ..RegisterRequest::default()
                },
            )
            .expect("register");
        let m = fit.transform;
        let back = |p: [f64; 3]| -> [f64; 3] {
            std::array::from_fn(|r| m[r][0] * p[0] + m[r][1] * p[1] + m[r][2] * p[2] + m[r][3])
        };
        assert!(fit.rms < 1e-3, "{turn_deg}°: rms {}", fit.rms);
        if search {
            // A box fits itself turned half about any axis: any of those
            // poses is an answer, each spanning the box as it stands.
            for (axis, extent) in size.iter().enumerate() {
                let along = moved.iter().map(|q| back(*q)[axis]);
                let (lo, hi) = along.fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(v), b.max(v)));
                assert!(
                    lo.abs() < 1e-3 && (hi - extent).abs() < 1e-3,
                    "{axis}: {lo}..{hi}"
                );
            }
            continue;
        }
        let worst = points
            .iter()
            .zip(&moved)
            .map(|(p, q)| {
                let b = back(*q);
                ((b[0] - p[0]).powi(2) + (b[1] - p[1]).powi(2) + (b[2] - p[2]).powi(2)).sqrt()
            })
            .fold(0.0, f64::max);
        assert!(worst < 1e-3, "{turn_deg}°: off by {worst}");
    }
}

/// A plate reads its thickness through its face and its length across
/// it, as a solid and as its triangles; past the reach, unmeasured.
#[test]
fn a_wall_reads_its_thickness() {
    use kernel_api::{DistanceTarget, PrimitiveKind, ThicknessRequest};
    let plate = super::common::chain(&[super::common::primitive(PrimitiveKind::Box {
        length: 40.0,
        width: 20.0,
        height: 4.0,
    })]);
    let request = ThicknessRequest {
        points: vec![[20.0, 10.0, 4.0], [0.0, 10.0, 2.0]],
        normals: vec![[0.0, 0.0, 1.0], [-1.0, 0.0, 0.0]],
        max: Some(30.0),
        ..ThicknessRequest::default()
    };
    for target in [
        DistanceTarget::Shape(&plate.brep_blob),
        DistanceTarget::Mesh(&plate.mesh),
    ] {
        let t = QUERIES.thickness(target, &request).expect("thickness");
        assert!((t[0] - 4.0).abs() < 1e-6 && t[1].is_nan(), "{t:?}");
    }
}

/// A tube's points on its rims, each with its wall's normal, read the
/// wall as its faces' other points do.
#[test]
fn a_solid_reads_its_wall_at_its_edges() {
    use kernel_api::{
        BooleanOp, DistanceTarget, Placement, PrimitiveKind, SolidOp, ThicknessRequest,
    };
    let cylinder = |radius| PrimitiveKind::Cylinder {
        radius,
        height: 30.0,
        angle_deg: 360.0,
    };
    let tube = super::common::chain(&[
        super::common::primitive(cylinder(10.0)),
        SolidOp::Primitive {
            kind: cylinder(8.0),
            placement: Placement::default(),
            op: BooleanOp::Cut,
        },
    ]);
    let m = &tube.mesh;
    let request = ThicknessRequest {
        points: m.positions.iter().map(|p| p.map(f64::from)).collect(),
        normals: m.normals.iter().map(|n| n.map(f64::from)).collect(),
        facing_limit_deg: 60.0,
        max: Some(7.0),
        ..ThicknessRequest::default()
    };
    let t = QUERIES
        .thickness(DistanceTarget::Shape(&tube.brep_blob), &request)
        .expect("thickness");
    for (i, v) in t.iter().enumerate() {
        let axial = m.normals[i][2].abs() > 0.9;
        assert!(
            axial && v.is_nan() || (v - 2.0).abs() < 1e-6,
            "{:?}: {v}",
            m.positions[i]
        );
    }
}

/// A stack of sections across a closed cylinder gives one closed chain
/// each, round at the cylinder's radius, pointing out of it and running
/// counter-clockwise seen from above.
#[test]
fn a_stack_of_sections_rings_a_cylinder() {
    use kernel_api::KernelQueries;
    let (mesh, _) = closed_cylinder(0.0);
    let stack = kernel_ogeom::QUERIES
        .mesh_sections(&mesh, [0.0, 0.0, 10.0], [0.0, 0.0, 1.0], &[0.0, 5.0, 12.5])
        .expect("sections");
    assert_eq!(stack.len(), 3);
    for (chains, z) in stack.iter().zip([10.0, 15.0, 22.5]) {
        assert_eq!(chains.len(), 1, "{z}");
        let c = &chains[0];
        assert!(c.closed && c.points.len() >= 64);
        let mut turn = 0.0;
        for (i, (p, o)) in c.points.iter().zip(&c.outward).enumerate() {
            let r = p[0].hypot(p[1]);
            assert!(
                (p[2] - z).abs() < 1e-6 && r > 9.9 && r < 10.0 + 1e-6,
                "{p:?}"
            );
            assert!((p[0] * o[0] + p[1] * o[1]) / r > 0.99, "{o:?}");
            let q = c.points[(i + 1) % c.points.len()];
            turn += p[0] * q[1] - p[1] * q[0];
        }
        assert!(turn > 0.0, "clockwise at {z}");
    }
}

/// A cylinder's shadow along its axis is its round end, and the outline 1
/// mm clear of it within 0.1 is one circle; seen from the side it is a
/// 20 × 30 rectangle whose outline keeps its sharp corners.
#[test]
fn a_shadow_is_outlined_clear_of_it() {
    use kernel_api::{KernelQueries, ProfilePlane, ProfileSegment, ShadowRequest};
    let (mesh, _) = closed_cylinder(0.0);
    let ask = |plane: ProfilePlane| {
        kernel_ogeom::QUERIES
            .mesh_shadow(
                &mesh,
                &ShadowRequest {
                    plane,
                    play: 1.0,
                    tolerance: 0.1,
                    round_below_deg: 30.0,
                    fill_below: 0.0,
                },
            )
            .expect("shadow")
    };
    let end = ask(ProfilePlane::default());
    assert_eq!((end.silhouette.len(), end.outline.len()), (1, 1));
    match end.outline[0].segments.as_slice() {
        [ProfileSegment::Circle { center, radius }] => {
            assert!(center[0].hypot(center[1]) < 0.05 && (11.0..=11.2).contains(radius));
        }
        other => panic!("{other:?}"),
    }
    let side = ask(ProfilePlane {
        origin: [0.0; 3],
        x_axis: [1.0, 0.0, 0.0],
        y_axis: [0.0, 0.0, 1.0],
        normal: [0.0, -1.0, 0.0],
    });
    // Every point of the outline lies in the band about the rectangle,
    // so its corners are rounds of about the clearance.
    let off = |[x, y]: [f64; 2]| {
        let (dx, dy) = (
            (x.abs() - 10.0).max(0.0),
            ((y - 15.0).abs() - 15.0).max(0.0),
        );
        dx.hypot(dy)
    };
    let wire = &side.outline[0];
    let mut arcs = 0;
    for s in &wire.segments {
        let points = match *s {
            ProfileSegment::Line { start, end } => vec![start, end],
            ProfileSegment::Arc { start, mid, end } => {
                arcs += 1;
                vec![start, mid, end]
            }
            ref other => panic!("{other:?}"),
        };
        for p in points {
            assert!((1.0..=1.2).contains(&off(p)), "{p:?} {}", off(p));
        }
    }
    assert_eq!((wire.segments.len(), arcs), (8, 4), "{wire:?}");
}

fn fit_chain(points: &[[f64; 2]], closed: bool) -> kernel_api::DrawingShape {
    use kernel_api::KernelQueries;
    kernel_ogeom::QUERIES
        .fit_chain_2d(
            points,
            &kernel_api::ChainFit {
                tolerance: 0.05,
                corners: true,
                axis_snap_deg: Some(3.0),
                min_bow: 0.1,
                closed,
            },
        )
        .expect("fit")
}

/// A half circle r 10 from (10, 0) to (-10, 0), then back along the
/// diameter to (9, 0).
fn d_shape() -> Vec<[f64; 2]> {
    let mut d: Vec<[f64; 2]> = (0..=40)
        .map(|k| f64::from(k) * std::f64::consts::PI / 40.0)
        .map(|t| [10.0 * t.cos(), 10.0 * t.sin()])
        .collect();
    d.extend((1..20).map(|k| [-10.0 + f64::from(k), 0.0]));
    d
}

/// The two bulges of a fitted D, sorted: a line's 0 and a half circle's 1.
fn d_bulges(shape: kernel_api::DrawingShape, ends: usize) -> Vec<f64> {
    match shape {
        kernel_api::DrawingShape::Polyline {
            points, mut bulges, ..
        } => {
            assert_eq!(points.len(), ends, "{points:?}");
            bulges.truncate(2);
            bulges.sort_by(f64::total_cmp);
            assert!(
                bulges[0].abs() < 1e-9 && (bulges[1] - 1.0).abs() < 0.01,
                "{bulges:?}"
            );
            bulges
        }
        other => panic!("{other:?}"),
    }
}

/// A half circle and its diameter, as an open chain of points, fits as
/// one arc and one line; a whole ring as one circle.
#[test]
fn a_chain_of_points_fits_as_lines_and_arcs() {
    d_bulges(fit_chain(&d_shape(), false), 3);
    let ring: Vec<[f64; 2]> = (0..60)
        .map(|k| f64::from(k) * std::f64::consts::TAU / 60.0)
        .map(|t| [10.0 * t.cos(), 10.0 * t.sin()])
        .collect();
    match fit_chain(&ring, true) {
        kernel_api::DrawingShape::Circle { radius, .. } => assert!((radius - 10.0).abs() < 0.01),
        other => panic!("{other:?}"),
    }
}

/// The same D closed fits as one arc and one line meeting at corners.
#[test]
fn a_closed_chain_of_points_fits_as_lines_and_arcs() {
    d_bulges(fit_chain(&d_shape(), true), 2);
}

/// Every kind of construction item goes into a STEP part of its own, each
/// by its name, with no solid.
#[test]
fn construction_geometry_writes_as_a_step_part() {
    use kernel_api::{ConstructionItem, ConstructionShape as S, KernelQueries};
    let items = [
        ("Point 1", S::Point([1.0, 2.0, 3.0])),
        (
            "Cylinder 1 axis",
            S::Axis {
                origin: [0.0; 3],
                z: [0.0, 0.0, 1.0],
                x: [1.0, 0.0, 0.0],
            },
        ),
        (
            "Section 1 +0.0",
            S::Polyline {
                points: vec![[0.0; 3], [10.0, 0.0, 0.0], [10.0, 5.0, 0.0]],
                closed: true,
            },
        ),
        (
            "Circle 1",
            S::Circle {
                centre: [0.0, 0.0, 5.0],
                normal: [0.0, 0.0, 1.0],
                radius: 4.0,
            },
        ),
        (
            "Plane 1",
            S::Plane {
                origin: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
                x_axis: [1.0, 0.0, 0.0],
                u: [-5.0, 5.0],
                v: [-2.0, 2.0],
            },
        ),
    ]
    .map(|(name, shape)| ConstructionItem {
        name: name.into(),
        shape,
    });
    let text = String::from_utf8(
        kernel_ogeom::QUERIES
            .write_construction_step("Scan", &items)
            .expect("written"),
    )
    .unwrap();
    for item in &items {
        assert!(text.contains(&format!("'{}'", item.name)), "{}", item.name);
    }
    assert!(text.contains("PRODUCT('Scan'") && !text.contains("MANIFOLD_SOLID_BREP"));
}

/// A mesh history on a closed cylinder with a speck beside it: the speck
/// goes as an island, a cut at half height capped flat leaves a closed
/// half of the volume, a ball deleted from its side opens a hole, and a
/// flip turns every face over; a step that cannot run is reported with
/// the mesh before it kept.
#[test]
fn a_mesh_history_edits_a_scan_in_steps() {
    use kernel_api::{KernelQueries, MeshOp, MeshRegion};
    let (mut mesh, _) = closed_cylinder(0.0);
    let base = mesh.positions.len() as u32;
    mesh.positions.extend([
        [40.0, 0.0, 0.0],
        [40.5, 0.0, 0.0],
        [40.0, 0.5, 0.0],
        [40.0, 0.0, 0.5],
    ]);
    mesh.indices
        .extend([0, 2, 1, 0, 1, 3, 1, 2, 3, 0, 3, 2].map(|i| base + i));
    let volume = |m: &kernel_api::TriMesh| {
        m.indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| m.positions[i as usize].map(f64::from));
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.0
            })
            .sum::<f64>()
    };
    let open_edges = |m: &kernel_api::TriMesh| {
        let key = |i: u32| m.positions[i as usize].map(f32::to_bits);
        let mut count: std::collections::HashMap<_, i32> = Default::default();
        for t in m.indices.as_chunks::<3>().0 {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let (a, b) = (key(a), key(b));
                *count
                    .entry(if a <= b { (a, b) } else { (b, a) })
                    .or_default() += 1;
            }
        }
        count.values().filter(|n| **n == 1).count()
    };
    let half = 0.5 * 64.0 * (std::f64::consts::TAU / 64.0).sin() * 100.0 * 15.0;
    let ops = vec![
        MeshOp::RemoveIslands { smaller_than: 5.0 },
        MeshOp::Cut {
            origin: [0.0, 0.0, 15.0],
            normal: [0.0, 0.0, 1.0],
            keep_above: false,
            close: true,
            bridge: 1.0,
        },
    ];
    let cut = kernel_ogeom::QUERIES.build_mesh_chain(&mesh, &ops).unwrap();
    assert!(cut.failed.is_none(), "{:?}", cut.failed);
    assert!(
        cut.steps[0].note.starts_with("1 of 2 pieces"),
        "{}",
        cut.steps[0].note
    );
    assert!(
        (volume(&cut.mesh) - half).abs() < 1e-3 * half,
        "{}",
        volume(&cut.mesh)
    );
    assert_eq!(open_edges(&cut.mesh), 0, "capped closed");
    let mut more = ops.clone();
    more.push(MeshOp::Delete {
        region: MeshRegion::Ball {
            centre: [10.0, 0.0, 7.5],
            radius: 3.0,
        },
    });
    more.push(MeshOp::Flip {
        region: MeshRegion::All,
    });
    let holed = kernel_ogeom::QUERIES
        .build_mesh_chain(&mesh, &more)
        .unwrap();
    assert!(holed.failed.is_none() && open_edges(&holed.mesh) > 0);
    assert!(volume(&holed.mesh) < 0.0, "turned inside out");
    more.insert(
        2,
        MeshOp::Delete {
            region: MeshRegion::Ball {
                centre: [100.0; 3],
                radius: 1.0,
            },
        },
    );
    let refused = kernel_ogeom::QUERIES
        .build_mesh_chain(&mesh, &more)
        .unwrap();
    assert_eq!(refused.failed.as_ref().map(|f| f.0), Some(2));
    assert!(
        (volume(&refused.mesh) - half).abs() < 1e-3 * half,
        "the mesh before it"
    );
}

/// A scan missing a block's top layer, and one missing the boss on it,
/// read the material each lacks with its volume and depth.
#[test]
fn a_scan_reads_the_material_it_lacks() {
    use super::common::{chain, primitive};
    use kernel_api::{BooleanOp, Placement, PrimitiveKind, SolidOp};
    let block = |height| {
        primitive(PrimitiveKind::Box {
            length: 20.0,
            width: 20.0,
            height,
        })
    };
    let boss = SolidOp::Primitive {
        kind: PrimitiveKind::Cylinder {
            radius: 3.0,
            height: 4.0,
            angle_deg: 360.0,
        },
        placement: Placement {
            origin: [10.0, 10.0, 10.0],
            ..Placement::default()
        },
        op: BooleanOp::Fuse,
    };
    let plain = chain(&[block(10.0)]);
    let bossed = chain(&[block(10.0), boss]);
    let thin = chain(&[block(8.0)]);
    let boss_volume = std::f64::consts::PI * 9.0 * 4.0;
    for (scan, solid, volume, depth, layer) in [
        (&thin.mesh, &plain.brep_blob, 800.0, 2.0, true),
        (&plain.mesh, &bossed.brep_blob, boss_volume, 4.0, false),
    ] {
        let regions = QUERIES
            .volume_difference(scan, solid, 0.5)
            .expect("compared");
        let r = &regions[0];
        assert!(r.missing && r.layer == layer, "{regions:?}");
        assert!((r.volume - volume).abs() < volume * 0.1, "{r:?}");
        assert!((r.depth - depth).abs() < 0.3, "{r:?}");
        assert!(
            regions[1..].iter().all(|o| o.volume < volume * 0.05),
            "{regions:?}"
        );
    }
}

/// A three-lobed bore, 0.1 mm out of round about Ø20, reads its zone and
/// its mating sizes: circularity and cylindricity 0.1, inscribed Ø19.9,
/// circumscribed Ø20.1.
#[test]
fn zone_fits_read_a_lobed_bore() {
    use kernel_api::{FitKind, FitMethod, FittedShape};
    let ring = |z: f64| {
        (0..360).map(move |i| {
            let a = f64::from(i).to_radians();
            let r = 10.0 + 0.05 * (3.0 * a).cos();
            [r * a.cos(), r * a.sin(), z]
        })
    };
    let circle: Vec<[f64; 3]> = ring(0.0).collect();
    let bore: Vec<[f64; 3]> = (0..5).flat_map(|k| ring(f64::from(k) * 5.0)).collect();
    let radius = |shape: &FittedShape| match *shape {
        FittedShape::Circle { radius, .. } | FittedShape::Cylinder { radius, .. } => radius,
        _ => panic!("{shape:?}"),
    };
    for (kind, points) in [(FitKind::Circle, &circle), (FitKind::Cylinder, &bore)] {
        let zone = QUERIES
            .zone_fit(kind, FitMethod::MinimumZone, points)
            .expect("zone");
        assert!(
            (zone.form.1 - zone.form.0 - 0.1).abs() < 1e-6,
            "{kind:?} {:?}",
            zone.form
        );
        for (method, want) in [
            (FitMethod::Inscribed, 9.95),
            (FitMethod::Circumscribed, 10.05),
        ] {
            let mate = QUERIES.zone_fit(kind, method, points).expect("mating fit");
            assert!(
                (radius(&mate.shape) - want).abs() < 1e-6,
                "{kind:?} {method:?} {mate:?}"
            );
        }
    }
    let least = QUERIES
        .zone_fit(FitKind::Plane, FitMethod::Inscribed, &circle)
        .unwrap_err();
    assert!(least.to_string().contains("inscribed"), "{least}");
}

/// A plate's 0.8 mm hole stays a hole in its outline, and is filled once
/// holes up to a millimetre are.
#[test]
fn a_shadow_fills_holes_narrower_than_asked() {
    use kernel_api::{BooleanOp, KernelQueries, Placement, PrimitiveKind, ShadowRequest, SolidOp};
    let plate = super::common::chain(&[
        super::common::primitive(PrimitiveKind::Box {
            length: 20.0,
            width: 20.0,
            height: 2.0,
        }),
        SolidOp::Primitive {
            kind: PrimitiveKind::Cylinder {
                radius: 0.4,
                height: 2.0,
                angle_deg: 360.0,
            },
            placement: Placement {
                origin: [10.0, 10.0, 0.0],
                ..Placement::default()
            },
            op: BooleanOp::Cut,
        },
    ]);
    let loops = |fill_below: f64| {
        kernel_ogeom::QUERIES
            .mesh_shadow(
                &plate.mesh,
                &ShadowRequest {
                    plane: kernel_api::ProfilePlane::default(),
                    play: 0.0,
                    tolerance: 0.02,
                    round_below_deg: 30.0,
                    fill_below,
                },
            )
            .expect("shadow")
            .outline
            .len()
    };
    assert_eq!((loops(0.0), loops(1.0)), (2, 1));
}
