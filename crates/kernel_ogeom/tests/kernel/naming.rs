//! Faces keep their names through a rebuild: references that kept a name
//! find their face again after dimensions change, where the point they
//! were picked at no longer lies on it.

use kernel_api::{
    BooleanOp, ChainProbe, ChainProjection, ExtrudeTermination, ProbeAnswer, Profile, ProfilePlane,
    ProfileSegment, ProfileWire, ShapeProbe, SolidOp, SweepKind, TessellationSettings, TopoName,
    naming,
};
use kernel_ogeom::OgeomKernel;

fn plane(z: f64) -> ProfilePlane {
    ProfilePlane {
        origin: [0.0, 0.0, z],
        x_axis: [1.0, 0.0, 0.0],
        y_axis: [0.0, 1.0, 0.0],
        normal: [0.0, 0.0, 1.0],
    }
}

/// A square of side `w` at (`x`, `y`), its sides named 1 to 4.
fn square(x: f64, y: f64, w: f64) -> ProfileWire {
    let p = [[x, y], [x + w, y], [x + w, y + w], [x, y + w]];
    ProfileWire {
        segments: (0..4)
            .map(|i| ProfileSegment::Line {
                start: p[i],
                end: p[(i + 1) % 4],
            })
            .collect(),
        names: (1..=4).map(|i| naming::name_of(&[i])).collect(),
    }
}

fn extrude(termination: ExtrudeTermination) -> SweepKind {
    SweepKind::Extrude {
        termination,
        second_side: None,
        symmetric: false,
        reversed: false,
        taper_deg: 0.0,
        direction: None,
    }
}

/// A 20 mm square block `height` tall.
fn block(height: f64) -> SolidOp {
    SolidOp::Sweep {
        profile: Profile {
            plane: plane(0.0),
            wires: vec![square(0.0, 0.0, 20.0)],
        },
        kind: extrude(ExtrudeTermination::Blind { distance: height }),
        op: BooleanOp::NewSolid,
    }
}

const TAGS: [TopoName; 2] = [101, 202];

fn build(ops: &[SolidOp], probes: &[ChainProbe]) -> kernel_api::SolidBuildResult {
    OgeomKernel::new()
        .execute_solid_chain(
            kernel_ogeom::ChainRequest::new(ops, &TessellationSettings::default())
                .tags(&TAGS)
                .probes(probes),
        )
        .unwrap()
}

/// The name of the face of `mesh` whose triangles all satisfy `on`.
fn face_name(mesh: &kernel_api::TriMesh, on: impl Fn([f32; 3]) -> bool) -> TopoName {
    for (face, name) in mesh.face_names.iter().enumerate() {
        let points: Vec<[f32; 3]> = mesh
            .indices
            .chunks(3)
            .zip(&mesh.faces)
            .filter(|(_, f)| **f as usize == face)
            .flat_map(|(t, _)| t.iter().map(|i| mesh.positions[*i as usize]))
            .collect();
        if !points.is_empty() && points.iter().all(|p| on(*p)) {
            return *name;
        }
    }
    panic!("no such face")
}

#[test]
fn every_face_is_named_and_the_names_hold_across_builds() {
    let a = build(&[block(10.0)], &[]);
    let b = build(&[block(10.0)], &[]);
    assert!(a.mesh.face_names.iter().all(|n| *n != 0));
    assert!(
        a.mesh.colors.is_empty(),
        "a built solid has no colours of its own: it draws in the body's"
    );
    assert_eq!(a.mesh.face_names, b.mesh.face_names, "the same every time");
    let mut unique = a.mesh.face_names.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 6, "six faces, six names");
    // A taller block names its faces alike: the walls by their sketch
    // segments, the top and bottom by where they stand.
    let tall = build(&[block(25.0)], &[]);
    let mut a_sorted = a.mesh.face_names.clone();
    a_sorted.sort_unstable();
    let mut tall_sorted = tall.mesh.face_names.clone();
    tall_sorted.sort_unstable();
    assert_eq!(a_sorted, tall_sorted);
    assert!(
        a.mesh
            .edge_faces
            .iter()
            .all(|pair| pair.iter().all(|n| *n != 0))
    );
}

/// A pad stopping on the block's top keeps stopping on it when the block
/// grows: the face is found by its name, where the point it was picked at
/// is now 10 mm below it.
#[test]
fn an_up_to_face_follows_its_face_by_name() {
    let first = build(&[block(10.0)], &[]);
    let top = face_name(&first.mesh, |p| (p[2] - 10.0).abs() < 1e-3);
    // A 5 mm square from below the block, up to its top.
    let post = |name: TopoName| SolidOp::Sweep {
        profile: Profile {
            plane: plane(-10.0),
            wires: vec![square(30.0, 0.0, 5.0)],
        },
        kind: extrude(ExtrudeTermination::UpToFace {
            point: [10.0, 10.0, 10.0],
            normal: [0.0, 0.0, 1.0],
            offset: 0.0,
            name,
        }),
        op: BooleanOp::Fuse,
    };
    let height = |result: &kernel_api::SolidBuildResult| {
        result
            .mesh
            .positions
            .iter()
            .filter(|p| p[0] > 29.0)
            .map(|p| p[2])
            .fold(f32::MIN, f32::max)
    };
    let short = build(&[block(10.0), post(top)], &[]);
    assert!((height(&short) - 10.0).abs() < 1e-3, "{}", height(&short));
    let tall = build(&[block(20.0), post(top)], &[]);
    assert!(
        (height(&tall) - 20.0).abs() < 1e-3,
        "by name: {}",
        height(&tall)
    );
    // The point alone stops on the plane it was picked on.
    let unnamed = build(&[block(20.0), post(0)], &[]);
    assert!(
        (height(&unnamed) - 10.0).abs() < 1e-3,
        "{}",
        height(&unnamed)
    );
}

/// A reference standing on the block's top (a datum on it) is answered
/// where the top is now.
#[test]
fn a_face_probe_finds_its_face_by_name() {
    let first = build(&[block(10.0)], &[]);
    let top = face_name(&first.mesh, |p| (p[2] - 10.0).abs() < 1e-3);
    let probe = |name| ChainProbe {
        after_op: 1,
        probe: ShapeProbe::Face {
            point: [5.0, 5.0, 10.0],
            normal: [0.0, 0.0, 1.0],
            name,
        },
    };
    let answered = build(&[block(30.0)], &[probe(top)]);
    let Ok(ProbeAnswer::Face { point, normal, .. }) = &answered.probes[0] else {
        panic!("{:?}", answered.probes[0]);
    };
    assert!((point[2] - 30.0).abs() < 1e-6 && (normal[2] - 1.0).abs() < 1e-9);
}

/// A shell opened at the block's top stays open at the top when the block
/// grows: the open face is found by its name.
#[test]
fn a_thickness_opens_its_face_by_name() {
    let first = build(&[block(10.0)], &[]);
    let top = face_name(&first.mesh, |p| (p[2] - 10.0).abs() < 1e-3);
    let shell = |name: TopoName| SolidOp::Thickness {
        both_sides: false,
        value: 1.0,
        open_faces: vec![[10.0, 10.0, 10.0]],
        open_face_names: vec![name],
        inward: true,
        join: kernel_api::ThicknessJoin::Intersection,
    };
    let volume = |ops: &[SolidOp]| super::common::volume(&build(ops, &[]).brep_blob);
    // Open at the top: the walls and the floor, 1 mm thick.
    let open_top = |h: f64| 400.0 * h - 18.0 * 18.0 * (h - 1.0);
    assert!((volume(&[block(10.0), shell(top)]) - open_top(10.0)).abs() < 0.5);
    assert!((volume(&[block(30.0), shell(top)]) - open_top(30.0)).abs() < 0.5);
}

/// The names of the two faces the 10 mm block's top edge along X at y = 0
/// runs between, as a pick keeps them.
fn top_front_edge() -> [TopoName; 2] {
    let first = build(&[block(10.0)], &[]);
    super::common::edge_names(&first.mesh, |p| {
        (p[2] - 10.0).abs() < 1e-3 && p[1].abs() < 1e-3
    })
}

/// A reference whose names are lost is found by where it was picked only
/// near there, and the op that took it is warned; one that never had
/// names is found by position as it always was, unwarned. The block's
/// diagonal is 30 mm, so a lost reference may have moved 3 mm.
#[test]
fn a_reference_whose_name_is_lost_is_warned_of_or_refused() {
    let named = top_front_edge();
    let lost: [TopoName; 2] = [7, 8];
    let fillet = |point: [f64; 3], faces| SolidOp::Fillet {
        radius: 2.0,
        edges: kernel_api::EdgeSelection::Picked(vec![kernel_api::EdgeProbe {
            point,
            direction: [1.0, 0.0, 0.0],
            faces,
        }]),
        follow_tangent: false,
    };
    let chain = |ops: &[SolidOp], probes: &[ChainProbe], projections: &[ChainProjection]| {
        OgeomKernel::new().execute_solid_chain(
            kernel_ogeom::ChainRequest::new(ops, &TessellationSettings::default())
                .tags(&TAGS)
                .probes(probes)
                .projections(projections),
        )
    };
    // (what, the pick, its names, the warning or refusal it gets)
    type Case<'a> = (&'a str, [f64; 3], [TopoName; 2], Option<&'a str>);
    let edges: [Case; 4] = [
        ("named", [10.0, 0.0, 10.0], named, None),
        ("never named", [10.0, -1.0, 10.0], [0, 0], None),
        (
            "lost, near",
            [10.0, -1.0, 10.0],
            lost,
            Some("not found by its name"),
        ),
        ("lost, far", [10.0, -5.0, 10.0], lost, Some("is gone")),
    ];
    for (what, point, faces, said) in edges {
        let built = chain(&[block(10.0), fillet(point, faces)], &[], &[]);
        match (said, built) {
            (Some("is gone"), Err(e)) => {
                assert_eq!(e.op_index, 1, "{what}");
                assert!(e.message.contains("is gone"), "{what}: {e}");
            }
            (said, Ok(built)) => {
                let warned: Vec<_> = built.warnings.iter().map(|w| w.op_index).collect();
                match said {
                    Some(text) => {
                        assert_eq!(warned, [1], "{what}");
                        assert!(built.warnings[0].message.contains(text), "{what}");
                    }
                    None => assert!(warned.is_empty(), "{what}: {:?}", built.warnings),
                }
            }
            (_, Err(e)) => panic!("{what}: {e}"),
        }
    }
    // A datum's face follows however far it moved while it keeps its
    // name or never had one; one whose name is lost does not cross the
    // block to stand on whatever face is nearest, and is warned of when it
    // finds one near. A sketch's projected edge is warned of the same way.
    let probe = |z, name| ChainProbe {
        after_op: 1,
        probe: ShapeProbe::Face {
            point: [5.0, 5.0, z],
            normal: [0.0, 0.0, 1.0],
            name,
        },
    };
    let projection = |faces| ChainProjection {
        after_op: 1,
        probe: kernel_api::ProjectionProbe {
            point: [5.0, 0.0, 10.0],
            faces,
            section: false,
            plane: plane(0.0),
        },
    };
    let answered = chain(
        &[block(10.0)],
        &[probe(30.0, 0), probe(30.0, 9), probe(11.0, 9)],
        &[projection(named), projection(lost)],
    )
    .unwrap();
    let Err(refused) = &answered.probes[1] else {
        panic!("{:?}", answered.probes[1]);
    };
    assert!(refused.contains("is gone"), "{refused}");
    assert!(answered.probes[0].is_ok() && answered.probes[2].is_ok());
    assert!(answered.projections.iter().all(Result::is_ok));
    let warned = |w: &[kernel_api::OpWarning]| w.iter().map(|w| w.op_index).collect::<Vec<_>>();
    assert_eq!(warned(&answered.probe_warnings), [2]);
    assert_eq!(warned(&answered.projection_warnings), [1]);
    assert!(answered.warnings.is_empty());
}

/// The round a fillet makes is named after the edge it rounds, so it keeps
/// its name when the block grows (a reference on the round finds it).
#[test]
fn a_fillet_s_round_is_named_after_its_edge() {
    let faces = top_front_edge();
    let round = |height: f64| {
        let fillet = SolidOp::Fillet {
            radius: 2.0,
            edges: kernel_api::EdgeSelection::Picked(vec![kernel_api::EdgeProbe {
                point: [10.0, 0.0, height],
                direction: [1.0, 0.0, 0.0],
                faces,
            }]),
            follow_tangent: false,
        };
        let built = build(&[block(height), fillet], &[]);
        let blend = built
            .mesh
            .face_surfaces
            .iter()
            .position(|s| matches!(s, kernel_api::FaceSurface::Cylinder { .. }))
            .expect("a round");
        built.mesh.face_names[blend]
    };
    let (short, tall) = (round(10.0), round(20.0));
    // Named after the edge it came from: its two faces' names under the
    // fillet's own.
    let mut bytes = faces[0].to_le_bytes().to_vec();
    bytes.extend_from_slice(&faces[1].to_le_bytes());
    assert_eq!(
        short,
        naming::child(TAGS[1], &bytes),
        "named after its edge"
    );
    assert_eq!(short, tall, "the round keeps its name");
}

/// Each face's maker is the op that made it, kept by a face a later op
/// trims: a block's top trimmed by a pocket and its sides stay the
/// block's, the pocket's walls and floor are the pocket's, and the round
/// of an edge the fillet's.
#[test]
fn every_face_keeps_the_op_that_made_it() {
    let pocket = SolidOp::Sweep {
        profile: Profile {
            plane: plane(6.0),
            wires: vec![square(5.0, 5.0, 10.0)],
        },
        kind: extrude(ExtrudeTermination::Blind { distance: 4.0 }),
        op: BooleanOp::Cut,
    };
    let round = SolidOp::Fillet {
        radius: 2.0,
        edges: kernel_api::EdgeSelection::Near(vec![[10.0, 0.0, 10.0]]),
        follow_tangent: false,
    };
    let tags: [TopoName; 3] = [101, 202, 303];
    let mesh = OgeomKernel::new()
        .execute_solid_chain(
            kernel_ogeom::ChainRequest::new(
                &[block(10.0), pocket, round],
                &TessellationSettings::default(),
            )
            .tags(&tags),
        )
        .unwrap()
        .mesh;
    let maker = |on: &dyn Fn([f32; 3]) -> bool| {
        let name = face_name(&mesh, on);
        let face = mesh.face_names.iter().position(|n| *n == name).unwrap();
        mesh.face_makers[face]
    };
    let e = 1e-4;
    let near = |a: f32, b: f32| (a - b).abs() < e;
    type On<'a> = &'a dyn Fn([f32; 3]) -> bool;
    let cases: [(&str, On, TopoName); 5] = [
        (
            "the trimmed top",
            &|p| near(p[2], 10.0) && p[1] > 2.0 - e,
            101,
        ),
        ("a side", &|p| near(p[0], 20.0), 101),
        ("the pocket's floor", &|p| near(p[2], 6.0), 202),
        ("a pocket wall", &|p| near(p[0], 5.0) && p[2] > 6.0 - e, 202),
        (
            "the round",
            &|p| {
                p[1] < 2.0 + e && p[2] > 8.0 - e && !near(p[1], 0.0)
                    || near(p[1], 0.0) && near(p[2], 8.0)
            },
            303,
        ),
    ];
    for (what, on, made_by) in cases {
        assert_eq!(maker(on), made_by, "{what}");
    }
}
