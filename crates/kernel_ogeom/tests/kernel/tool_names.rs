//! The names a feature's tool gives its faces are what a document keeps
//! in its references (a pick keeps a face's name, an edge pick the names
//! of its two faces), so a document saved by an earlier release finds the
//! same faces only while every tool names its faces as that release did.
//! `tests/data/tool_names.txt` holds the names each kind of tool gave,
//! face by face (each face by where its middle is); every build names
//! them alike. `PRINTCAD_WRITE_TOOL_NAMES=1` writes the file afresh.

use kernel_api::{
    BooleanOp, ExtrudeTermination, Profile, ProfilePlane, ProfileSegment, ProfileWire,
    RevolveTermination, SolidOp, SweepKind, TessellationSettings, TopoName, naming,
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

/// A closed polygon through `points`, its sides named after their place.
fn polygon(points: &[[f64; 2]]) -> ProfileWire {
    let n = points.len();
    ProfileWire {
        segments: (0..n)
            .map(|i| ProfileSegment::Line {
                start: points[i],
                end: points[(i + 1) % n],
            })
            .collect(),
        names: (1..=n as u8).map(|i| naming::name_of(&[i])).collect(),
    }
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> ProfileWire {
    polygon(&[[x, y], [x + w, y], [x + w, y + h], [x, y + h]])
}

fn circle(c: [f64; 2], r: f64) -> ProfileWire {
    ProfileWire {
        segments: vec![ProfileSegment::Circle {
            center: c,
            radius: r,
        }],
        names: vec![naming::name_of(b"circle")],
    }
}

fn profile(wires: Vec<ProfileWire>) -> Profile {
    Profile {
        plane: plane(0.0),
        wires,
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

fn blind(distance: f64) -> SweepKind {
    extrude(ExtrudeTermination::Blind { distance })
}

fn revolve(angle_deg: f64, midplane: bool) -> SweepKind {
    SweepKind::Revolve {
        axis_origin: [0.0, 0.0],
        axis_dir: [0.0, 1.0],
        angle_deg,
        second_angle_deg: None,
        midplane,
        reversed: false,
        termination: RevolveTermination::Angle,
    }
}

fn sweep(profile: Profile, kind: SweepKind, op: BooleanOp) -> SolidOp {
    SolidOp::Sweep { profile, kind, op }
}

fn new_solid(profile: Profile, kind: SweepKind) -> Vec<SolidOp> {
    vec![sweep(profile, kind, BooleanOp::NewSolid)]
}

/// A 40 by 30 block 10 tall, and `then` on it.
fn on_block(then: SolidOp) -> Vec<SolidOp> {
    vec![
        sweep(
            profile(vec![rect(-20.0, -15.0, 40.0, 30.0)]),
            blind(10.0),
            BooleanOp::NewSolid,
        ),
        then,
    ]
}

/// Every case: a name and its ops.
fn cases() -> Vec<(&'static str, Vec<SolidOp>)> {
    let square = || profile(vec![rect(0.0, 0.0, 20.0, 20.0)]);
    let off_axis = || profile(vec![rect(5.0, 0.0, 10.0, 20.0)]);
    let on_axis = || profile(vec![rect(0.0, 0.0, 10.0, 20.0)]);
    let mut kinds: Vec<(&'static str, Vec<SolidOp>)> = vec![
        ("pad", new_solid(square(), blind(10.0))),
        (
            "pad reversed",
            new_solid(
                square(),
                SweepKind::Extrude {
                    termination: ExtrudeTermination::Blind { distance: 10.0 },
                    second_side: None,
                    symmetric: false,
                    reversed: true,
                    taper_deg: 0.0,
                    direction: None,
                },
            ),
        ),
        (
            "pad symmetric",
            new_solid(
                square(),
                SweepKind::Extrude {
                    termination: ExtrudeTermination::Blind { distance: 10.0 },
                    second_side: None,
                    symmetric: true,
                    reversed: false,
                    taper_deg: 0.0,
                    direction: None,
                },
            ),
        ),
        (
            "pad two sides",
            new_solid(
                square(),
                SweepKind::Extrude {
                    termination: ExtrudeTermination::Blind { distance: 10.0 },
                    second_side: Some(ExtrudeTermination::Blind { distance: 4.0 }),
                    symmetric: false,
                    reversed: false,
                    taper_deg: 0.0,
                    direction: None,
                },
            ),
        ),
        (
            "pad tapered",
            new_solid(
                square(),
                SweepKind::Extrude {
                    termination: ExtrudeTermination::Blind { distance: 10.0 },
                    second_side: None,
                    symmetric: false,
                    reversed: false,
                    taper_deg: 5.0,
                    direction: None,
                },
            ),
        ),
        (
            "pad tapered two sides",
            new_solid(
                square(),
                SweepKind::Extrude {
                    termination: ExtrudeTermination::Blind { distance: 10.0 },
                    second_side: Some(ExtrudeTermination::Blind { distance: 4.0 }),
                    symmetric: false,
                    reversed: false,
                    taper_deg: 5.0,
                    direction: None,
                },
            ),
        ),
        (
            "pad slanted",
            new_solid(
                square(),
                SweepKind::Extrude {
                    termination: ExtrudeTermination::Blind { distance: 10.0 },
                    second_side: None,
                    symmetric: false,
                    reversed: false,
                    taper_deg: 0.0,
                    direction: Some([0.3, 0.2, 1.0]),
                },
            ),
        ),
        (
            "pad with a hole",
            new_solid(
                profile(vec![rect(0.0, 0.0, 20.0, 20.0), circle([10.0, 10.0], 4.0)]),
                blind(10.0),
            ),
        ),
        (
            "pad of two regions",
            new_solid(
                profile(vec![
                    rect(0.0, 0.0, 10.0, 10.0),
                    rect(20.0, 0.0, 10.0, 10.0),
                ]),
                blind(10.0),
            ),
        ),
        (
            "pad of a hexagon",
            new_solid(
                profile(vec![polygon(&[
                    [10.0, 0.0],
                    [5.0, 8.66],
                    [-5.0, 8.66],
                    [-10.0, 0.0],
                    [-5.0, -8.66],
                    [5.0, -8.66],
                ])]),
                blind(6.0),
            ),
        ),
        (
            "pad of unnamed sides",
            new_solid(
                profile(vec![ProfileWire {
                    names: Vec::new(),
                    ..rect(0.0, 0.0, 20.0, 10.0)
                }]),
                blind(5.0),
            ),
        ),
        (
            "pad of a spline",
            new_solid(
                profile(vec![ProfileWire {
                    segments: vec![
                        ProfileSegment::Line {
                            start: [0.0, 0.0],
                            end: [20.0, 0.0],
                        },
                        ProfileSegment::BSpline {
                            control_points: vec![
                                [20.0, 0.0],
                                [24.0, 8.0],
                                [10.0, 16.0],
                                [0.0, 10.0],
                            ],
                            periodic: false,
                        },
                        ProfileSegment::Line {
                            start: [0.0, 10.0],
                            end: [0.0, 0.0],
                        },
                    ],
                    names: (1..=3u8).map(|i| naming::name_of(&[i])).collect(),
                }]),
                blind(5.0),
            ),
        ),
        (
            "pad up to a plane",
            new_solid(
                square(),
                extrude(ExtrudeTermination::UpToPlane {
                    point: [0.0, 0.0, 12.0],
                    normal: [0.0, 0.0, 1.0],
                    offset: 0.0,
                }),
            ),
        ),
        (
            "pad up to a slanted plane",
            new_solid(
                square(),
                extrude(ExtrudeTermination::UpToPlane {
                    point: [0.0, 0.0, 12.0],
                    normal: [0.3, 0.0, 1.0],
                    offset: 0.0,
                }),
            ),
        ),
        (
            "pocket through all",
            on_block(sweep(
                Profile {
                    plane: plane(10.0),
                    wires: vec![rect(-5.0, -5.0, 10.0, 10.0)],
                },
                SweepKind::Extrude {
                    termination: ExtrudeTermination::ThroughAll,
                    second_side: None,
                    symmetric: false,
                    reversed: true,
                    taper_deg: 0.0,
                    direction: None,
                },
                BooleanOp::Cut,
            )),
        ),
        (
            "pad to the first face",
            vec![
                sweep(
                    Profile {
                        plane: plane(20.0),
                        wires: vec![rect(-20.0, -15.0, 40.0, 30.0)],
                    },
                    blind(5.0),
                    BooleanOp::NewSolid,
                ),
                sweep(
                    profile(vec![circle([0.0, 0.0], 5.0)]),
                    extrude(ExtrudeTermination::ToFirst),
                    BooleanOp::Fuse,
                ),
            ],
        ),
        (
            "revolve a quarter",
            new_solid(off_axis(), revolve(90.0, false)),
        ),
        (
            "revolve a half",
            new_solid(off_axis(), revolve(180.0, false)),
        ),
        (
            "revolve whole",
            new_solid(off_axis(), revolve(360.0, false)),
        ),
        (
            "revolve on the axis",
            new_solid(on_axis(), revolve(270.0, false)),
        ),
        (
            "revolve whole on the axis",
            new_solid(on_axis(), revolve(360.0, false)),
        ),
        (
            "revolve about the middle",
            new_solid(off_axis(), revolve(120.0, true)),
        ),
        (
            "revolve two ways",
            new_solid(
                off_axis(),
                SweepKind::Revolve {
                    axis_origin: [0.0, 0.0],
                    axis_dir: [0.0, 1.0],
                    angle_deg: 60.0,
                    second_angle_deg: Some(30.0),
                    midplane: false,
                    reversed: false,
                    termination: RevolveTermination::Angle,
                },
            ),
        ),
        (
            "revolve a circle",
            new_solid(
                profile(vec![circle([15.0, 0.0], 4.0)]),
                revolve(360.0, false),
            ),
        ),
        (
            "helix",
            new_solid(
                profile(vec![rect(10.0, 0.0, 2.0, 2.0)]),
                SweepKind::Helix {
                    axis_origin: [0.0, 0.0],
                    axis_dir: [0.0, 1.0],
                    pitch: 5.0,
                    height: 10.0,
                    left_handed: false,
                    cone_angle_deg: 0.0,
                    reversed: false,
                    turns: None,
                    growth: None,
                    through_all: false,
                },
            ),
        ),
    ];
    let section = |z: f64, w: f64| Profile {
        plane: plane(z),
        wires: vec![rect(-w / 2.0, -w / 2.0, w, w)],
    };
    kinds.push((
        "loft",
        vec![SolidOp::Loft {
            sections: vec![section(0.0, 20.0), section(10.0, 10.0)],
            ruled: true,
            closed: false,
            op: BooleanOp::NewSolid,
        }],
    ));
    kinds.push((
        "loft of three",
        vec![SolidOp::Loft {
            sections: vec![section(0.0, 20.0), section(10.0, 20.0), section(20.0, 20.0)],
            ruled: true,
            closed: false,
            op: BooleanOp::NewSolid,
        }],
    ));
    kinds
}

/// Each face of the build's solid: where its middle is, in tenths of a
/// millimetre, and its name; in that order.
fn faces(ops: &[SolidOp]) -> Vec<([i64; 3], TopoName)> {
    let tags: Vec<TopoName> = (1..=ops.len() as u64).map(|i| i * 1000 + 7).collect();
    let built = OgeomKernel::new()
        .execute_solid_chain(
            kernel_ogeom::ChainRequest::new(ops, &TessellationSettings::default()).tags(&tags),
        )
        .unwrap();
    let mesh = &built.mesh;
    let mut out = Vec::new();
    for (face, name) in mesh.face_names.iter().enumerate() {
        let (mut sum, mut area) = ([0.0f64; 3], 0.0f64);
        for (t, _) in mesh
            .indices
            .chunks(3)
            .zip(&mesh.faces)
            .filter(|(_, f)| **f as usize == face)
        {
            let p = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize].map(f64::from));
            let u = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
            let v = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let a = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            for (k, s) in sum.iter_mut().enumerate() {
                *s += a * (p[0][k] + p[1][k] + p[2][k]) / 3.0;
            }
            area += a;
        }
        let at = sum.map(|s| (s / area.max(1e-300) * 10.0).round() as i64);
        out.push((at, *name));
    }
    out.sort_unstable();
    out
}

#[test]
fn every_tool_names_its_faces_as_earlier_releases_did() {
    let mut text = String::new();
    for (case, ops) in cases() {
        for (at, name) in faces(&ops) {
            text.push_str(&format!(
                "{case}: {} {} {} {name:016x}\n",
                at[0], at[1], at[2]
            ));
        }
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tool_names.txt");
    if std::env::var_os("PRINTCAD_WRITE_TOOL_NAMES").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let want = std::fs::read_to_string(&path).unwrap();
    let wrong: Vec<String> = want
        .lines()
        .zip(text.lines())
        .filter(|(w, g)| w != g)
        .map(|(w, g)| format!("want {w}\n got {g}"))
        .collect();
    assert!(
        wrong.is_empty() && want.lines().count() == text.lines().count(),
        "faces named otherwise than before:\n{}",
        wrong.join("\n")
    );
}
